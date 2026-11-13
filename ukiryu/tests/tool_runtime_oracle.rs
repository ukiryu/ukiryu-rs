//! Replays the pinned Ruby `Ukiryu::Tool` (ukiryu/ukiryu ab6df353,
//! `lib/ukiryu/tool.rb` and `lib/ukiryu/tool/*.rb`) against the Rust tool
//! runtime.
//!
//! `tool_runtime_data/oracle.json` was captured by running the Ruby code:
//! register fixture tools loaded by `Tool.get` and synthetic definitions
//! built with `Tool.new(ToolDefinition.from_yaml(...))`, for several
//! platform/shell pairs, with the `Executor.execute` call that
//! `Tool#execute` makes recorded instead of run. Executable discovery was
//! limited to the PATH scan over a directory of fake executables named in
//! `fake_names`; this test recreates that directory.
//!
//! Rows the Ruby register loader could not load (`ToolNotFoundError`) belong
//! to the loader and are skipped. Documented differences (see
//! `ukiryu::tool`) are asserted as such.
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::PathBuf;

use ukiryu::executable::{DiscoveryContext, Platform};
use ukiryu::models::{Model, Text, ToolDefinition, VersionDetection};
use ukiryu::runtime::{native_platform, ShellName};
use ukiryu::tool::{profile_is_modern, Tool, ToolError, ToolOptions};
use ukiryu::types::{ErrorClass, Value};

fn oracle() -> Json {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tool_runtime_data/oracle.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("oracle")).expect("json")
}

fn host() -> Platform {
    native_platform().expect("supported host")
}

struct Bins {
    root: PathBuf,
    bin: String,
    empty: String,
}

impl Drop for Bins {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn make_bins(names: &[Json]) -> Bins {
    let root = std::env::temp_dir().join(format!(
        "ukiryu-tool-runtime-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let bin = root.join("bin");
    let empty = root.join("empty");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    for n in names {
        let n = n.as_str().unwrap();
        for f in [n.to_string(), format!("{n}.exe")] {
            let p = bin.join(f);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }
    Bins {
        bin: bin.to_str().unwrap().to_owned(),
        empty: empty.to_str().unwrap().to_owned(),
        root,
    }
}

/// The oracle's `<BIN>`-relative form of a discovered path on this host.
fn rel(path: &str, bins: &Bins) -> String {
    let p = path.replace('\\', "/");
    let b = bins.bin.replace('\\', "/");
    let mut out = match p.strip_prefix(&b) {
        Some(rest) => format!("<BIN>{rest}"),
        None => p,
    };
    if cfg!(windows) {
        if let Some(stem) = out.strip_suffix(".exe") {
            out = stem.to_owned();
        }
    }
    out
}

fn decode(j: &Json) -> Value {
    let v = &j["v"];
    match j["t"].as_str().unwrap() {
        "nil" => Value::Nil,
        "bool" => Value::Bool(v.as_bool().unwrap()),
        "int" => Value::int(v.as_i64().unwrap()),
        "float" => Value::Float(v.as_str().unwrap().parse().unwrap()),
        "sym" => Value::sym(v.as_str().unwrap()),
        "str" => Value::str(v.as_str().unwrap()),
        "arr" => Value::Array(v.as_array().unwrap().iter().map(decode).collect()),
        "hash" => Value::Hash(
            v.as_array()
                .unwrap()
                .iter()
                .map(|p| (decode(&p[0]), decode(&p[1])))
                .collect(),
        ),
        other => panic!("unexpected tag {other}"),
    }
}

fn shell(name: &str) -> ShellName {
    ShellName::lookup(name).expect("shell")
}

fn options(row: &Json, bins: &Bins) -> ToolOptions {
    let platform = Platform::from_name(row["platform"].as_str().unwrap()).unwrap();
    let sh = shell(row["shell"].as_str().unwrap());
    let dir = if row["path"].as_str() == Some("empty") {
        &bins.empty
    } else {
        &bins.bin
    };
    let pathext = if cfg!(windows) { Some(".exe") } else { None };
    let mut o = ToolOptions::new(platform, sh).discovery(DiscoveryContext::new(
        host(),
        sh.as_str(),
        Some(dir.as_str()),
        pathext,
    ));
    if let Some(v) = row["version_option"].as_str() {
        o = o.version(v);
    }
    o
}

#[derive(Default)]
struct Tally {
    failures: Vec<String>,
    checked: usize,
    skipped: BTreeMap<&'static str, usize>,
}

impl Tally {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.checked += 1;
        if !ok {
            self.failures.push(what());
        }
    }
    fn skip(&mut self, why: &'static str) {
        *self.skipped.entry(why).or_default() += 1;
    }
}

fn expect_error(t: &mut Tally, id: &str, oracle: &Json, got: &ToolError) {
    let class = oracle["class"].as_str().unwrap();
    let message = oracle["message"].as_str().unwrap();
    let ok = match (class, got) {
        ("Ukiryu::Errors::ValidationError", ToolError::Build(e)) => {
            e.class == ErrorClass::Validation && e.message == message
        }
        ("ArgumentError", ToolError::Argument { message: m }) => m == message,
        ("Ukiryu::Errors::ProfileNotFoundError", ToolError::ProfileNotFound { message: m }) => {
            m == message
        }
        // Documented difference: the source cannot resolve the constant.
        ("NameError", ToolError::ProfileNotFound { message: m }) => {
            message.contains("ProfileNotFoundError") && m.starts_with("No compatible profile for ")
        }
        // Documented difference: File.dirname(nil) on a missing executable.
        ("TypeError", ToolError::ExecutableNotFound { .. }) => {
            message == "no implicit conversion of nil into String"
        }
        _ => false,
    };
    t.check(ok, || {
        format!("{id}: oracle {class}: {message}; got {got:?}")
    });
}

fn check_call(
    t: &mut Tally,
    id: &str,
    tool: &Tool,
    path: &str,
    params: &Value,
    oracle: &Json,
    bins: &Bins,
) {
    let built = tool.build(path, params);
    if let Some(err) = oracle.get("err") {
        match built {
            Err(e) => expect_error(t, id, err, &e),
            Ok(b) => t.check(false, || format!("{id}: oracle error {err}, got {b:?}")),
        }
        return;
    }
    let ok = &oracle["ok"];
    if ok["executable"].is_null() {
        // Documented difference: the source hands nil to the executor.
        t.check(
            matches!(built, Err(ToolError::ExecutableNotFound { .. })),
            || format!("{id}: nil executable, got {built:?}"),
        );
        return;
    }
    let b = match built {
        Ok(b) => b,
        Err(e) => {
            t.check(false, || format!("{id}: oracle ok, got error {e:?}"));
            return;
        }
    };
    let exe = rel(&b.invocation.executable, bins);
    let want_exe = ok["executable"].as_str().unwrap();
    if cfg!(windows)
        && exe != want_exe
        && tool.executable().map(|e| rel(e, bins)).as_deref() == Some(exe.as_str())
    {
        // A standalone executable without an extension is not executable on
        // a Windows host; the capture ran on Linux.
        t.skip("standalone executable on a Windows host");
    } else {
        t.check(exe == want_exe, || {
            format!("{id}: executable {exe} != {want_exe}")
        });
    }
    let want_args: Vec<String> = ok["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    t.check(b.invocation.args == want_args, || {
        format!("{id}: args {:?} != {:?}", b.invocation.args, want_args)
    });
    let want_env: Vec<(Option<String>, String)> = ok["env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p[0].as_str().map(str::to_owned),
                p[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    t.check(b.invocation.env.0 == want_env, || {
        format!("{id}: env {:?} != {:?}", b.invocation.env.0, want_env)
    });
    let want_stdin = ok["stdin"].as_str().map(|s| s.as_bytes().to_vec());
    t.check(b.stdin == want_stdin, || {
        format!("{id}: stdin {:?}", b.stdin)
    });
    t.check(tool.name().as_deref() == ok["tool_name"].as_str(), || {
        format!("{id}: tool name")
    });
    t.check(
        b.command_name.as_deref() == ok["command_name"].as_str(),
        || {
            format!(
                "{id}: command name {:?} != {}",
                b.command_name, ok["command_name"]
            )
        },
    );
    t.check(
        ok["timeout"].as_i64() == Some(42) && ok["shell"].as_str() == Some(tool.shell().as_str()),
        || format!("{id}: shell/timeout {} {}", ok["shell"], ok["timeout"]),
    );
}

fn check_row(t: &mut Tally, id: &str, row: &Json, yaml: &str, bins: &Bins) {
    let definition = ToolDefinition::from_yaml(yaml).unwrap_or_else(|e| panic!("{id}: {e}"));
    let opts = options(row, bins);
    let tool = Tool::new(definition, &opts);
    if let Some(err) = row.get("load_error") {
        match tool {
            Err(e) => expect_error(t, id, err, &e),
            Ok(_) => t.check(false, || {
                format!("{id}: oracle load error {err}, got a tool")
            }),
        }
        return;
    }
    let tool = match tool {
        Ok(tool) => tool,
        Err(e) => {
            t.check(false, || format!("{id}: oracle loaded, got {e:?}"));
            return;
        }
    };
    t.check(
        Some(tool.profile_index() as i64) == row["profile_index"].as_i64(),
        || {
            format!(
                "{id}: profile {} != {}",
                tool.profile_index(),
                row["profile_index"]
            )
        },
    );
    let exe = tool.executable().map(|e| rel(e, bins));
    t.check(exe.as_deref() == row["executable"].as_str(), || {
        format!("{id}: executable {exe:?} != {}", row["executable"])
    });
    t.check(
        tool.is_available() == row["available"].as_bool().unwrap(),
        || format!("{id}: available"),
    );
    t.check(
        tool.unavailability_reason().as_deref() == row["unavailability_reason"].as_str(),
        || format!("{id}: reason {:?}", tool.unavailability_reason()),
    );
    t.check(tool.version().as_deref() == row["version"].as_str(), || {
        format!("{id}: version {:?} != {}", tool.version(), row["version"])
    });
    let names: Vec<Option<String>> = tool
        .commands()
        .iter()
        .map(|c| c.name.as_ref().map(Text::to_ruby_s))
        .collect();
    let want: Vec<Option<String>> = row["command_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().map(str::to_owned))
        .collect();
    t.check(names == want, || {
        format!("{id}: commands {names:?} != {want:?}")
    });
    t.check(
        tool.has_routing() == row["routing"].as_bool().unwrap(),
        || format!("{id}: routing?"),
    );

    for cmd in row["commands"].as_array().unwrap() {
        let path = cmd["path"].as_str().unwrap();
        if let Some(meanings) = cmd.get("exit_meanings") {
            let name = if path.contains(':') {
                let parts: Vec<String> = path.split(':').map(str::to_owned).collect();
                tool.resolve_action_path(&parts)
                    .and_then(|r| r.action)
                    .and_then(|a| a.name.as_ref().map(Text::to_ruby_s))
                    .unwrap_or_default()
            } else {
                path.to_owned()
            };
            for m in meanings.as_array().unwrap() {
                let status = m[0].as_i64().unwrap();
                let got = tool.exit_code_meaning(&name, status);
                t.check(got.as_deref() == m[1].as_str(), || {
                    format!("{id} {path}: exit {status} meaning {got:?} != {}", m[1])
                });
            }
        }
        for case in cmd["cases"].as_array().unwrap() {
            let cid = format!("{id} {path} [{}]", case["set"].as_str().unwrap());
            let params = decode(&case["params"]);
            check_call(t, &cid, &tool, path, &params, &case["result"], bins);
        }
    }

    if let Some(resolve) = row.get("resolve") {
        for r in resolve.as_array().unwrap() {
            let path: Vec<String> = r[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_owned())
                .collect();
            let got = tool.resolve_action_path(&path);
            let ok = match (&got, r[1].is_null()) {
                (None, true) => true,
                (Some(g), false) => {
                    g.executable == r[1]["executable"].as_str().unwrap()
                        && g.action
                            .and_then(|a| a.name.as_ref().map(Text::to_ruby_s))
                            .as_deref()
                            == r[1]["action"].as_str()
                        && g.path == path
                }
                _ => false,
            };
            t.check(ok, || {
                format!("{id}: resolve {path:?} -> {got:?} != {}", r[1])
            });
        }
    }
}

fn finish(name: &str, t: Tally) {
    eprintln!("{name}: {} checks, skipped {:?}", t.checked, t.skipped);
    assert!(t.checked > 0, "{name}: nothing checked");
    if !t.failures.is_empty() {
        for f in t.failures.iter().take(60) {
            eprintln!("FAIL {f}");
        }
        panic!(
            "{name}: {} of {} checks failed",
            t.failures.len(),
            t.checked
        );
    }
}

#[test]
fn register_tools_match_ruby_tool() {
    let o = oracle();
    let bins = make_bins(o["fake_names"].as_array().unwrap());
    let mut t = Tally::default();
    for row in o["register"].as_array().unwrap() {
        let id = format!(
            "{} {}/{}",
            row["tool"].as_str().unwrap(),
            row["platform"].as_str().unwrap(),
            row["shell"].as_str().unwrap()
        );
        if row.get("load_error").is_some() {
            t.skip("register loader found no tool (loader-owned)");
            continue;
        }
        let yaml = o["definitions"][row["definition_yaml"].as_str().unwrap()]
            .as_str()
            .unwrap();
        check_row(&mut t, &id, row, yaml, &bins);
    }
    finish("register", t);
}

#[test]
fn synthetic_definitions_match_ruby_tool() {
    let o = oracle();
    let bins = make_bins(o["fake_names"].as_array().unwrap());
    let mut t = Tally::default();
    for row in o["synthetic"].as_array().unwrap() {
        let id = format!(
            "{} {}/{} ({})",
            row["file"].as_str().unwrap(),
            row["platform"].as_str().unwrap(),
            row["shell"].as_str().unwrap(),
            row["path"].as_str().unwrap()
        );
        let yaml = o["definitions"][row["input_yaml"].as_str().unwrap()]
            .as_str()
            .unwrap();
        check_row(&mut t, &id, row, yaml, &bins);
    }
    finish("synthetic", t);
}

#[test]
fn profile_is_modern_matches_ruby() {
    let o = oracle();
    let mut t = Tally::default();
    for row in o["modern"].as_array().unwrap() {
        let version = Text::new(row["version"].as_str().unwrap());
        let vd = row["threshold"].as_str().map(|th| VersionDetection {
            modern_threshold: Some(Text::new(th)),
            ..VersionDetection::default()
        });
        let got = profile_is_modern(Some(&version), vd.as_ref());
        let want = row["result"]["ok"].as_bool();
        t.check(matches!(&got, Ok(g) if *g == want), || {
            format!("{row}: got {got:?}")
        });
    }
    finish("modern", t);
}

#[test]
fn routed_root_paths_split_like_ruby() {
    // `'remote:add:'.split(':')` drops the trailing empty field.
    let o = oracle();
    let bins = make_bins(o["fake_names"].as_array().unwrap());
    let row = o["synthetic"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["file"] == "routed.yaml" && r["platform"] == "linux")
        .unwrap();
    let yaml = o["definitions"][row["input_yaml"].as_str().unwrap()]
        .as_str()
        .unwrap();
    let tool = Tool::new(
        ToolDefinition::from_yaml(yaml).unwrap(),
        &options(row, &bins),
    )
    .unwrap();
    let params = Value::Hash(vec![(Value::sym("name"), Value::str("origin"))]);
    let a = tool.build("remote:add", &params).unwrap();
    let b = tool.build(" remote : add :", &params).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.invocation.executable, "git-remote");
    // String keys are symbolized when no key is a Symbol.
    let c = tool
        .build(
            "remote:add",
            &Value::Hash(vec![(Value::str("name"), Value::str("origin"))]),
        )
        .unwrap();
    assert_eq!(a, c);
}
