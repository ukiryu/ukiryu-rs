//! Replays the pinned Ruby `Tool#check_version_compatibility` and
//! `Tool#probe_flag` (ukiryu/ukiryu ab6df353,
//! `lib/ukiryu/tool/version_detection.rb`,
//! `lib/ukiryu/models/version_compatibility.rb`) against the Rust tool.
//!
//! `tool_version_compat_data/oracle.json` was captured by running the Ruby
//! code with `Tool.new(ToolDefinition.from_yaml(...))`:
//!
//! * `compat` rows: every mode against several requirements and version
//!   sources (option, definition, command detection, man-page detection).
//!   `Executor.execute` and `ManPageParser.parse_date` were scripted fakes;
//!   the rows hold their responses and every call they received. Here the
//!   same responses come from an injected `CommandRunner`/`ManPageDates`.
//! * `probe` rows: `probe_flag` over real `/bin/sh` scripts through the real
//!   Ruby executor, with each executor call and its raw output recorded.
//!   Here an injected `FlagProbe` returns that output (or the error Ruby
//!   raised), so the decision and the request are checked on every host;
//!   `tool_version_compat_real.rs` runs the scripts themselves.
use serde_json::Value as Json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ukiryu::errors::{Error, ErrorKind};
use ukiryu::executable::{DiscoveryContext, Platform};
use ukiryu::execution::{
    CommandInfo, ExecutionMetadata, ExecutionResult, InvalidByteSequence, Output, Seconds,
};
use ukiryu::executor::{ExecuteError, Execution};
use ukiryu::extractors::man::ManPageError;
use ukiryu::models::{Model, ToolDefinition, VersionCompatibility};
use ukiryu::runtime::{native_platform, ShellName};
use ukiryu::tool::{
    CompatibilityMode, FlagProbe, FlagProbeRequest, Tool, ToolError, ToolOptions,
    PROBE_FLAG_TIMEOUT_SECS,
};
use ukiryu::version_detection::{
    CommandOutput, CommandRequest, CommandRunner, ManPageDates, RunnerError,
};

const ORACLE_COMMIT: &str = "ab6df353c6dc2d7faf7817a86aafcafa82579896";

fn oracle() -> Json {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/tool_version_compat_data/oracle.json");
    let data: Json =
        serde_json::from_str(&std::fs::read_to_string(path).expect("oracle")).expect("json");
    assert_eq!(data["source"]["commit"], ORACLE_COMMIT);
    data
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

/// A directory of fake executables (`name` and `name.exe`) and an empty one.
fn make_bins(names: &[&str]) -> Bins {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "ukiryu-tool-compat-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let bin = root.join("bin");
    let empty = root.join("empty");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    for n in names {
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

fn tool_for(row: &Json, yaml: &str, bins: &Bins) -> Tool {
    let platform = Platform::from_name(row["platform"].as_str().unwrap()).unwrap();
    let shell = ShellName::lookup(row["shell"].as_str().unwrap()).expect("shell");
    let dir = if row["path"].as_str().unwrap().starts_with("empty") {
        &bins.empty
    } else {
        &bins.bin
    };
    let pathext = if cfg!(windows) { Some(".exe") } else { None };
    let mut options = ToolOptions::new(platform, shell).discovery(DiscoveryContext::new(
        host(),
        shell.as_str(),
        Some(dir.as_str()),
        pathext,
    ));
    if let Some(v) = row["version_option"].as_str() {
        options = options.version(v);
    }
    let definition = ToolDefinition::from_yaml(yaml).expect("definition");
    Tool::new(definition, &options).expect("tool")
}

/// The `Executor.execute` fake of the capture: scripted responses in order,
/// every request recorded. An exhausted script fails as the Ruby fake did.
struct ScriptedRunner {
    responses: Vec<Json>,
    requests: Vec<CommandRequest>,
}

impl CommandRunner for ScriptedRunner {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, RunnerError> {
        self.requests.push(request.clone());
        if self.responses.is_empty() {
            return Err(RunnerError::new("no scripted response"));
        }
        let r = self.responses.remove(0);
        if let Some(message) = r["raise"].as_str() {
            return Err(RunnerError::new(message));
        }
        Ok(CommandOutput {
            stdout: r["stdout"].as_str().unwrap_or("").as_bytes().to_vec(),
            stderr: r["stderr"].as_str().unwrap_or("").as_bytes().to_vec(),
            exit_status: r["status"].as_i64().unwrap_or(0) as i32,
        })
    }
}

/// The `ManPageParser.parse_date` fake of the capture.
struct ScriptedMan {
    dates: Vec<Json>,
    paths: Vec<String>,
}

impl ManPageDates for ScriptedMan {
    fn parse_date(&mut self, path: &str) -> Result<Option<String>, ManPageError> {
        self.paths.push(path.to_owned());
        if self.dates.is_empty() {
            return Ok(None);
        }
        Ok(self.dates.remove(0).as_str().map(str::to_owned))
    }
}

#[derive(Default)]
struct Tally {
    failures: Vec<String>,
    checked: usize,
}

impl Tally {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.checked += 1;
        if !ok {
            self.failures.push(what());
        }
    }

    fn finish(self, name: &str) {
        eprintln!("{name}: {} checks", self.checked);
        assert!(
            self.failures.is_empty(),
            "{name}: {} of {} checks failed:\n{}",
            self.failures.len(),
            self.checked,
            self.failures.join("\n")
        );
    }
}

fn check_result(t: &mut Tally, id: &str, expected: &Json, got: &VersionCompatibility) {
    t.check(expected["class"] == "Ukiryu::VersionCompatibility", || {
        format!("{id}: class {}", expected["class"])
    });
    t.check(
        expected["installed_version"].as_str() == Some(got.installed_version.as_str()),
        || {
            format!(
                "{id}: installed {} vs {:?}",
                expected["installed_version"], got.installed_version
            )
        },
    );
    t.check(
        expected["required_version"].as_str() == got.required_version.as_deref(),
        || {
            format!(
                "{id}: required {} vs {:?}",
                expected["required_version"], got.required_version
            )
        },
    );
    t.check(
        expected["compatible"].as_bool() == Some(got.compatible)
            && expected["compatible?"].as_bool() == Some(got.is_compatible())
            && expected["incompatible?"].as_bool() == Some(got.is_incompatible()),
        || {
            format!(
                "{id}: compatible {} vs {}",
                expected["compatible"], got.compatible
            )
        },
    );
    t.check(expected["reason"].as_str() == got.reason.as_deref(), || {
        format!("{id}: reason {} vs {:?}", expected["reason"], got.reason)
    });
    t.check(
        expected["status_message"].as_str() == Some(got.status_message().as_str()),
        || {
            format!(
                "{id}: status_message {} vs {:?}",
                expected["status_message"],
                got.status_message()
            )
        },
    );
}

fn check_compat_row(t: &mut Tally, row: &Json, yaml: &str, bins: &Bins) {
    let id = row["id"].as_str().unwrap();
    let tool = tool_for(row, yaml, bins);
    t.check(
        row["executable"].as_str().map(str::to_owned) == tool.executable().map(|e| rel(e, bins)),
        || {
            format!(
                "{id}: executable {} vs {:?}",
                row["executable"],
                tool.executable()
            )
        },
    );
    t.check(
        row["requirement"].as_str().map(str::to_owned) == tool.version_requirement(),
        || {
            format!(
                "{id}: requirement {} vs {:?}",
                row["requirement"],
                tool.version_requirement()
            )
        },
    );

    let mut runner = ScriptedRunner {
        responses: row["responses"].as_array().unwrap().clone(),
        requests: Vec::new(),
    };
    let mut man = ScriptedMan {
        dates: row["man"].as_array().unwrap().clone(),
        paths: Vec::new(),
    };
    let mode = CompatibilityMode::from_name(row["mode"].as_str().unwrap());
    match tool.check_version_compatibility(mode, &mut runner, &mut man) {
        Ok(got) => match row.get("result") {
            Some(expected) => check_result(t, id, expected, &got),
            None => t.check(false, || format!("{id}: Ruby raised {}", row["error"])),
        },
        Err(e) => t.check(false, || format!("{id}: unexpected error {e}")),
    }

    let calls = row["calls"].as_array().unwrap();
    t.check(calls.len() == runner.requests.len(), || {
        format!(
            "{id}: {} executor calls vs {}",
            calls.len(),
            runner.requests.len()
        )
    });
    for (call, got) in calls.iter().zip(&runner.requests) {
        let args: Vec<String> = call["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_owned())
            .collect();
        let opts = &call["opts"];
        let ok = call["executable"].as_str() == Some(rel(&got.executable, bins).as_str())
            && args == got.args
            && opts["shell"].as_str() == got.shell.as_deref()
            && opts["timeout"].as_u64() == Some(got.timeout_secs)
            && opts["allow_failure"].as_bool() == Some(got.allow_failure);
        t.check(ok, || format!("{id}: call {call} vs {got:?}"));
    }
    let man_calls: Vec<&str> = row["man_calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    t.check(man_calls == man.paths, || {
        format!("{id}: man pages {man_calls:?} vs {:?}", man.paths)
    });
}

#[test]
fn check_version_compatibility_matches_ruby() {
    let data = oracle();
    let definitions = data["definitions"].as_array().unwrap();
    let bins = make_bins(&["synth"]);
    let mut t = Tally::default();
    let rows = data["compat"].as_array().unwrap();
    assert_eq!(rows.len(), 728);
    for row in rows {
        let yaml = definitions[row["def"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap();
        check_compat_row(&mut t, row, yaml, &bins);
    }
    t.finish("check_version_compatibility");
}

/// The process outcome Ruby observed for a probe row, as the injected probe
/// returns it.
struct ReplayProbe {
    row: Json,
    requests: Vec<FlagProbeRequest>,
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn execution(
    request: &FlagProbeRequest,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: i64,
) -> Execution {
    let info = CommandInfo {
        executable: request.executable.clone(),
        arguments: request.arguments.clone(),
        full_command: String::new(),
        shell: Some(request.shell.as_str().to_owned()),
        tool_name: None,
        command_name: None,
    };
    Execution {
        result: ExecutionResult::new(
            info,
            Output::new(stdout.clone(), stderr.clone(), status),
            ExecutionMetadata::new(None, None, Some(request.timeout)),
        ),
        stdout,
        stderr,
    }
}

impl FlagProbe for ReplayProbe {
    fn run(&mut self, request: FlagProbeRequest) -> Result<Execution, ExecuteError> {
        self.requests.push(request.clone());
        let call = &self.row["calls"][self.requests.len() - 1];
        if let Some(raw) = call.get("raw") {
            return Ok(execution(
                &request,
                hex(raw["stdout_hex"].as_str().unwrap()),
                hex(raw["stderr_hex"].as_str().unwrap()),
                raw["status"].as_i64().unwrap(),
            ));
        }
        let error = &self.row["error"];
        let message = error["message"].as_str().unwrap();
        let failed = Box::new(execution(&request, Vec::new(), Vec::new(), 1));
        Err(match error["class"].as_str().unwrap() {
            "Ukiryu::Errors::ExecutionError" => ExecuteError::Failed {
                error: Error::new(ErrorKind::Execution, Some(message)),
                execution: failed,
            },
            "Ukiryu::Errors::TimeoutError" => {
                ExecuteError::TimedOut(Error::timeout_error(Some(message), Some(request.timeout)))
            }
            "ArgumentError" => ExecuteError::InvalidByteSequence {
                error: InvalidByteSequence::argument(),
                execution: failed,
            },
            other => panic!("no executor error for {other}"),
        })
    }
}

/// The Ruby class and message of a `probe_flag` error.
fn error_class_message(e: &ToolError) -> (String, String) {
    match e {
        ToolError::Execute(inner) => match inner.as_ref() {
            ExecuteError::InvalidByteSequence { error, .. } => {
                (error.ruby_class().to_owned(), error.message().to_owned())
            }
            other => (
                other
                    .kind()
                    .map(|k| k.ruby_class().to_owned())
                    .unwrap_or_else(|| format!("{other:?}")),
                other.to_string(),
            ),
        },
        other => (format!("{other:?}"), other.to_string()),
    }
}

#[test]
fn probe_flag_matches_ruby_with_injected_process() {
    let data = oracle();
    let rows = data["probe"].as_array().unwrap();
    assert_eq!(rows.len(), 15);
    let names: Vec<&str> = data["probe_scripts"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let bins = make_bins(&names);
    let mut t = Tally::default();
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let tool = tool_for(row, row["yaml"].as_str().unwrap(), &bins);
        let mut probe = ReplayProbe {
            row: row.clone(),
            requests: Vec::new(),
        };
        let got = tool.probe_flag_with(row["flag"].as_str().unwrap(), &mut probe);
        match (&got, row.get("result")) {
            (Ok(b), Some(expected)) => t.check(expected.as_bool() == Some(*b), || {
                format!("{id}: {expected} vs {b}")
            }),
            (Err(e), None) => {
                let (class, message) = error_class_message(e);
                t.check(
                    row["error"]["class"] == class.as_str()
                        && row["error"]["message"] == message.as_str(),
                    || format!("{id}: error {} vs {class}: {message}", row["error"]),
                );
            }
            _ => t.check(false, || {
                format!(
                    "{id}: {got:?} vs {:?} {:?}",
                    row.get("result"),
                    row.get("error")
                )
            }),
        }

        let calls = row["calls"].as_array().unwrap();
        t.check(calls.len() == probe.requests.len(), || {
            format!("{id}: {} calls vs {}", calls.len(), probe.requests.len())
        });
        for (call, request) in calls.iter().zip(&probe.requests) {
            let args: Vec<String> = call["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_owned())
                .collect();
            let opts = call["opts"].as_object().unwrap();
            let mut keys: Vec<&str> = opts.keys().map(String::as_str).collect();
            keys.sort_unstable();
            let ok = call["executable"].as_str() == Some(rel(&request.executable, &bins).as_str())
                && args == request.arguments
                && opts["shell"].as_str() == Some(request.shell.as_str())
                && opts["timeout"].as_i64() == Some(PROBE_FLAG_TIMEOUT_SECS)
                && request.timeout == Seconds::Int(PROBE_FLAG_TIMEOUT_SECS)
                // The source passes no env:, no allow_failure: and no tool name.
                && keys == ["shell", "timeout"]
                && request.environment.is_empty();
            t.check(ok, || format!("{id}: call {call} vs {request:?}"));
        }
    }
    t.finish("probe_flag (injected)");
}

#[test]
fn compatibility_mode_names() {
    assert_eq!(CompatibilityMode::default(), CompatibilityMode::Strict);
    for (name, mode) in [
        ("strict", CompatibilityMode::Strict),
        ("lenient", CompatibilityMode::Lenient),
        ("probe", CompatibilityMode::Probe),
        ("other", CompatibilityMode::Other),
    ] {
        assert_eq!(CompatibilityMode::from_name(name), mode);
    }
}
