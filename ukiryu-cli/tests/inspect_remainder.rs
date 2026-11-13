//! `config list`, `list`, `commands`, `register` and `resolve` against Ruby CLI
//! goldens, plus the gates of `info`, `opts` and `which`.
//!
//! `inspect_remainder_goldens.json` holds stdout, stderr, exit status and the
//! files left under `$HOME` for each case, captured from the pinned Ruby CLI
//! with a controlled environment (see its `source.context`). The register is
//! the fixture tree under `inspect_data/register`; each case gets its own copy.
//! Each case is replayed here: the arguments go through the real parser, the
//! handler runs over an explicit context with the same variables, and the
//! writes are applied.
//!
//! Recorded deviations:
//! - shell detection is strict (DECISIONS.md D2): a row with a `note` expects
//!   the shell error after the output printed before the first tool lookup;
//! - the YAML load warning of a malformed `config.yml` carries the engine's
//!   message, so only its prefix is compared;
//! - the capture loads `yaml` before the gem (the pinned `resolve` skips every
//!   definition with `uninitialized constant Psych` otherwise).
//!
//! The rows need executables on `PATH`, so they run on Unix hosts only.
#![cfg(unix)]

use serde_json::Value as Json;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use ukiryu::environment::Environment;
use ukiryu::executable::Platform;
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{Host, ShellError};
use ukiryu_cli::commands::inspect::{
    self, CommandsInput, ConfigAction, ConfigInput, HandlerContext, InfoInput, InspectError,
    OptsInput, RegisterInput, ResolveInput, WhichInput, FIND_BY_NEEDS,
};
use ukiryu_cli::commands::CommandOutcome;

fn goldens() -> Json {
    let text = include_str!("inspect_remainder_goldens.json");
    serde_json::from_str(text).expect("inspect_remainder_goldens.json parses")
}

fn text(value: &Json) -> &str {
    value.as_str().expect("string")
}

fn fixture_register() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/inspect_data/register")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Sandbox {
    root: PathBuf,
    home: String,
    bin: String,
    xdg: String,
    sysdata: String,
    register: String,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!(
            "ukiryu-cli-inspect-remainder-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dir = |leaf: &str| {
            let path = root.join(leaf);
            fs::create_dir_all(&path).unwrap();
            path.to_str().unwrap().to_owned()
        };
        Sandbox {
            home: dir("home"),
            bin: dir("bin"),
            xdg: dir("xdg"),
            sysdata: dir("sysdata"),
            register: root.join("register").to_str().unwrap().to_owned(),
            root,
        }
    }

    fn substitute(&self, golden: &str) -> String {
        golden
            .replace("{HOME}", &self.home)
            .replace("{BIN}", &self.bin)
            .replace("{XDG}", &self.xdg)
            .replace("{SYSDATA}", &self.sysdata)
            .replace("{REGISTER}", &self.register)
    }

    fn install_executable(&self, name: &str) {
        let path = Path::new(&self.bin).join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn host(&self, extra: &[(String, String)]) -> Host {
        let mut pairs = vec![
            ("HOME".to_owned(), self.home.clone()),
            ("PATH".to_owned(), self.bin.clone()),
            ("TZ".to_owned(), "UTC".to_owned()),
            ("XDG_DATA_HOME".to_owned(), self.xdg.clone()),
            ("XDG_DATA_DIRS".to_owned(), self.sysdata.clone()),
        ];
        pairs.extend(extra.iter().cloned());
        Host {
            platform: Platform::Linux,
            env: Environment::from_pairs(pairs),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn arg(matches: &clap::ArgMatches, id: &str) -> Option<String> {
    matches
        .get_one::<OsString>(id)
        .map(|v| v.to_str().expect("UTF-8 argument").to_owned())
}

/// The parser-to-handler mapping documented for root's dispatcher.
fn dispatch(args: &[String], host: Host) -> Result<CommandOutcome, InspectError> {
    let matches = ukiryu_cli::arguments::command()
        .try_get_matches_from(std::iter::once("ukiryu".to_owned()).chain(args.iter().cloned()))
        .expect("arguments parse");
    let (verb, sub) = matches.subcommand().expect("subcommand");
    let has_register = matches!(verb, "config" | "list" | "commands");
    let register = if has_register {
        arg(sub, "register")
    } else {
        None
    };
    let ctx = HandlerContext { host, register };
    match verb {
        "config" => inspect::config(
            ConfigInput::from_arguments(
                arg(sub, "action").as_deref(),
                arg(sub, "key").as_deref(),
                arg(sub, "value").as_deref(),
            )
            .expect("every config action has a handler"),
            &ctx,
        ),
        "list" => inspect::list(&ctx),
        "commands" => inspect::commands(
            CommandsInput {
                tool: arg(sub, "tool").unwrap(),
            },
            &ctx,
        ),
        "register" => inspect::register(
            RegisterInput {
                subcommand: arg(sub, "subcommand"),
                force: sub.get_one::<bool>("force").copied().unwrap_or(false),
            },
            &ctx,
        ),
        "resolve" => inspect::resolve(
            ResolveInput {
                tool: arg(sub, "tool").unwrap(),
                constraint: arg(sub, "version-constraint"),
            },
            &ctx,
        ),
        other => panic!("unexpected verb {other}"),
    }
}

struct Applied {
    stdout: String,
    stderr: String,
}

fn apply(writes: &[OutputWrite]) -> Applied {
    let mut applied = Applied {
        stdout: String::new(),
        stderr: String::new(),
    };
    for write in writes {
        match write {
            OutputWrite::Stdout(s) => applied.stdout.push_str(s),
            OutputWrite::Stderr(s) => applied.stderr.push_str(s),
            OutputWrite::File { path, contents } => fs::write(path, contents).unwrap(),
        }
    }
    applied
}

fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                if !rel.starts_with(".ukiryu/register") {
                    found.push(rel);
                }
            }
        }
    }
    found.sort();
    found
}

/// Compare stdout line by line, tolerating only the engine-specific YAML load
/// message of a malformed `config.yml`.
fn assert_stdout(name: &str, actual: &str, expected: &str) {
    const WARNING: &str = "\x1b[31mWarning: Failed to load config file: ";
    let actual_lines: Vec<&str> = actual.split('\n').collect();
    let expected_lines: Vec<&str> = expected.split('\n').collect();
    assert_eq!(
        actual_lines.len(),
        expected_lines.len(),
        "{name}: {actual:?}"
    );
    for (a, e) in actual_lines.iter().zip(&expected_lines) {
        if e.starts_with(WARNING) {
            assert!(
                a.starts_with(WARNING) && a.ends_with("\x1b[0m"),
                "{name}: {a:?}"
            );
        } else {
            assert_eq!(a, e, "{name}");
        }
    }
}

fn setup_case(case: &Json, sandbox: &Sandbox) {
    for bin in ["bash", "sh", "which"] {
        sandbox.install_executable(bin);
    }
    for bin in case["bins"].as_array().unwrap() {
        sandbox.install_executable(text(bin));
    }
    for (rel, body) in case["home_before"].as_object().unwrap() {
        let path = Path::new(&sandbox.home).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text(body)).unwrap();
    }
    let mtime = UNIX_EPOCH + Duration::from_secs(goldens()["fixed_mtime_unix"].as_u64().unwrap());
    for (rel, body) in case["defs"].as_object().unwrap() {
        let (base, rest) = rel.split_once('/').unwrap();
        let root = if base == "user" {
            Path::new(&sandbox.xdg)
        } else {
            Path::new(&sandbox.sysdata)
        };
        let path = root.join("ukiryu/definitions").join(rest);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text(body)).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }
    match text(&case["register"]) {
        "env" => copy_tree(&fixture_register(), Path::new(&sandbox.register)),
        "user" => copy_tree(
            &fixture_register(),
            &Path::new(&sandbox.home).join(".ukiryu/register"),
        ),
        "empty" => {
            let dir = Path::new(&sandbox.register).join("tools/x");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("a.yaml"), "---\nname: x\n").unwrap();
        }
        other => panic!("register mode {other}"),
    }
}

#[test]
fn goldens_replay() {
    let goldens = goldens();
    let cases = goldens["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    let mut replayed = 0;
    for case in cases {
        let name = text(&case["name"]);
        let mut sandbox = Sandbox::new(name);
        // The user register of the capture lives under HOME.
        let user_register = text(&case["register"]) == "user";
        if user_register {
            sandbox.register = format!("{}/.ukiryu/register", sandbox.home);
        }
        setup_case(case, &sandbox);
        let mut extra: Vec<(String, String)> = Vec::new();
        if text(&case["register"]) != "user" {
            extra.push(("UKIRYU_REGISTER".to_owned(), sandbox.register.clone()));
        }
        for (k, v) in case["env"].as_object().unwrap() {
            extra.retain(|(name, _)| name != k);
            extra.push((k.clone(), text(v).replace("{REGISTER}", &sandbox.register)));
        }
        let args: Vec<String> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| text(a).replace("{REGISTER}", &sandbox.register))
            .collect();
        let expected_stdout = sandbox.substitute(text(&case["stdout"]));
        let expected_stderr = sandbox.substitute(text(&case["stderr"]));
        let expected_exit = case["exit"].as_i64().unwrap() as i32;

        match dispatch(&args, sandbox.host(&extra)) {
            Ok(outcome) => {
                assert!(
                    case["note"].is_null(),
                    "{name}: expected the D2 shell error"
                );
                let applied = apply(&outcome.writes);
                assert_stdout(name, &applied.stdout, &expected_stdout);
                assert_eq!(applied.stderr, expected_stderr, "{name}");
                assert_eq!(outcome.exit_code, expected_exit, "{name}");
            }
            Err(InspectError::Shell { partial, error }) => {
                assert!(!case["note"].is_null(), "{name}: unexpected {error}");
                let applied = apply(&partial);
                let first_line = expected_stdout.split_inclusive('\n').next().unwrap();
                assert_eq!(applied.stdout, first_line, "{name}");
                assert!(
                    matches!(error, ShellError::Unknown { ref value, .. } if value == "/opt/bin/xonsh"),
                    "{name}: {error:?}"
                );
            }
            Err(error) => {
                let applied = apply(error.partial());
                assert_eq!(applied.stdout, expected_stdout, "{name}");
                assert_eq!(
                    format!("{}\n", error.friendly_message()),
                    expected_stderr,
                    "{name}"
                );
                assert_eq!(expected_exit, 1, "{name}");
            }
        }

        let expected_files = case["home_after"].as_object().unwrap();
        let home = Path::new(&sandbox.home);
        let wanted: Vec<String> = expected_files.keys().cloned().collect();
        assert_eq!(files_under(home), wanted, "{name}: files under HOME");
        for (rel, body) in expected_files {
            assert_eq!(
                fs::read_to_string(home.join(rel)).unwrap(),
                text(body),
                "{name}: {rel}"
            );
        }
        replayed += 1;
    }
    assert_eq!(replayed, cases.len());
}

#[test]
fn every_config_action_has_a_handler() {
    for action in [None, Some("list")] {
        assert_eq!(
            ConfigInput::from_arguments(action, None, None),
            Some(ConfigInput {
                action: ConfigAction::List
            })
        );
    }
}

fn context(sandbox: &Sandbox) -> HandlerContext {
    HandlerContext {
        host: sandbox.host(&[("UKIRYU_REGISTER".to_owned(), sandbox.register.clone())]),
        register: None,
    }
}

#[test]
fn info_opts_and_which_are_gated_with_the_missing_apis() {
    let sandbox = Sandbox::new("gated");
    copy_tree(&fixture_register(), Path::new(&sandbox.register));
    let ctx = context(&sandbox);
    let errors = [
        inspect::info(
            InfoInput {
                tool: "beta".to_owned(),
                all: false,
            },
            &ctx,
        ),
        inspect::opts(
            OptsInput {
                tool: "beta".to_owned(),
                command: None,
            },
            &ctx,
        ),
        inspect::which(
            WhichInput {
                identifier: "beta".to_owned(),
                platform: None,
                shell: None,
            },
            &ctx,
        ),
    ];
    for result in errors {
        let error = result.expect_err("a gated verb never succeeds");
        let message = error.friendly_message();
        assert!(
            message.contains("is not available in this build yet"),
            "{message}"
        );
        for api in [
            "register scanner",
            "Tool.find_by",
            "load_tool_metadata",
            "compatible_profile",
        ] {
            assert!(message.contains(api), "{api}: {message}");
        }
        assert!(error.partial().is_empty());
    }
    assert!(FIND_BY_NEEDS.contains("Tool.find_by"));
}

#[test]
fn list_gates_a_tool_without_a_version_file() {
    let sandbox = Sandbox::new("orphan");
    copy_tree(&fixture_register(), Path::new(&sandbox.register));
    // No version spec: the source would fall back to interface discovery.
    fs::write(
        Path::new(&sandbox.register).join("tools/zeta/index.yaml"),
        "---\nukiryu_schema: '1.0'\nname: zeta\ninterface: zeta\nimplementations: []\n",
    )
    .unwrap();
    let error = inspect::list(&context(&sandbox)).expect_err("gated, not a success");
    let InspectError::Gated {
        what,
        needs,
        partial,
    } = &error
    else {
        panic!("{error:?}")
    };
    assert_eq!(what, "list (interface lookup for zeta)");
    assert_eq!(needs, FIND_BY_NEEDS);
    // The header was printed before the lookup.
    assert!(apply(partial).stdout.contains("Available tools (2):"));
}

#[test]
fn register_update_needs_a_git_work_tree() {
    let sandbox = Sandbox::new("update");
    copy_tree(&fixture_register(), Path::new(&sandbox.register));
    let error = inspect::register(
        RegisterInput {
            subcommand: Some("update".to_owned()),
            force: false,
        },
        &context(&sandbox),
    )
    .expect_err("not a git work tree");
    assert_eq!(
        error.friendly_message(),
        format!(
            "[ArgumentError] '{}' is not in a git working tree",
            sandbox.register
        )
    );
    assert!(Path::new(&sandbox.register).join("tools/zeta").is_dir());
}

#[test]
fn register_info_without_git_ignores_the_zone() {
    // A register without `.git` has no last update to render, so a non-UTC
    // zone does not matter.
    let sandbox = Sandbox::new("zone");
    copy_tree(&fixture_register(), Path::new(&sandbox.register));
    let ctx = HandlerContext {
        host: sandbox.host(&[
            ("UKIRYU_REGISTER".to_owned(), sandbox.register.clone()),
            ("TZ".to_owned(), "Asia/Karachi".to_owned()),
        ]),
        register: None,
    };
    let outcome = inspect::register(RegisterInput::default(), &ctx).unwrap();
    assert_eq!(outcome.exit_code, 0);
}

#[test]
fn resolve_gates_a_non_utc_zone() {
    let sandbox = Sandbox::new("resolve-zone");
    let path = Path::new(&sandbox.xdg).join("ukiryu/definitions/gamma/1.0.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "---\nname: gamma\nversion: '1.0'\n").unwrap();
    let ctx = HandlerContext {
        host: sandbox.host(&[("TZ".to_owned(), "Asia/Karachi".to_owned())]),
        register: None,
    };
    let error = inspect::resolve(
        ResolveInput {
            tool: "gamma".to_owned(),
            constraint: None,
        },
        &ctx,
    )
    .expect_err("local zone is not rendered");
    assert!(matches!(error, InspectError::Gated { .. }), "{error:?}");
}
