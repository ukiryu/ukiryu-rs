//! Golden tests for the Tool-dependent authoring handlers
//! (`ukiryu_cli::commands::author::{describe, validate_with_tools}`).
//!
//! `author_data/tools_cases.json` is captured from the pinned Ruby oracle
//! (ukiryu/ukiryu ab6df353) through `exe/ukiryu` by
//! `scripts/capture-author-reference.py`, against a copy
//! of `author_data/tree` (the `regx` register, the fake tools in `tbin` and
//! the definitions under `smoke` and `fall`) with every file mtime set to
//! 2026-01-02 03:04:05 UTC and an isolated environment. Each case is replayed
//! here in a fresh copy of the same tree, with the process working directory
//! set to it (one test function, so the cases run sequentially).
//!
//! The fake tools are POSIX shell scripts and the captures use Linux paths,
//! so the replay is Linux only. Every row must match the oracle exactly;
//! `describe` resolves names through `Tool.find_by` (exact name, interface,
//! alias and register-wide lookups), so no row is gated.
#![cfg(target_os = "linux")]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ukiryu::definition::DiscoveryEnv;
use ukiryu::environment::Environment;
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{native_platform, Host};
use ukiryu_cli::commands::author::{
    self, AuthorContext, AuthorError, DescribeInput, ToolEnv, ValidateInput, ValidateOptions,
    ValidateTarget,
};
use ukiryu_cli::commands::CommandOutcome;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/author_data")
}

fn copy_tree(from: &Path, to: &Path, mtime: SystemTime) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target, mtime);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&target)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
        }
    }
}

/// The value of `--name`/`-n` after the positionals, if given.
fn option(args: &[String], names: &[&str]) -> Option<String> {
    args.iter()
        .position(|a| names.contains(&a.as_str()))
        .and_then(|i| args.get(i + 1).cloned())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Positionals: arguments that are neither options nor option values.
fn positionals(args: &[String], value_options: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if value_options.contains(&a.as_str()) {
            i += 2;
        } else if a.starts_with('-') && a.len() > 1 {
            i += 1;
        } else {
            out.push(a.clone());
            i += 1;
        }
    }
    out
}

fn perform(writes: &[OutputWrite]) -> (String, String) {
    let (mut stdout, mut stderr) = (String::new(), String::new());
    for w in writes {
        match w {
            OutputWrite::Stdout(s) => stdout.push_str(s),
            OutputWrite::Stderr(s) => stderr.push_str(s),
            OutputWrite::File { .. } => panic!("no case writes a file"),
        }
    }
    (stdout, stderr)
}

/// The case's environment variable with `{root}` replaced, over the capture
/// defaults.
fn case_env(case: &Value, root: &str, key: &str, default: Option<String>) -> Option<String> {
    match case["env"].get(key) {
        Some(Value::String(v)) => Some(v.replace("{root}", root)),
        Some(Value::Null) => None,
        _ => default,
    }
}

fn contexts(case: &Value, root: &str) -> (AuthorContext, ToolEnv) {
    let path = case_env(
        case,
        root,
        "PATH",
        Some(format!("{root}/tbin:/usr/bin:/bin")),
    )
    .unwrap();
    let register = case_env(case, root, "UKIRYU_REGISTER", Some(format!("{root}/regx")));
    let colors_disabled = case_env(case, root, "NO_COLOR", None).is_some_and(|v| !v.is_empty());
    let ctx = AuthorContext {
        thor_colors: case["pty"].as_bool().unwrap_or(false),
        discovery: DiscoveryEnv {
            home: Some(format!("{root}/home")),
            xdg_data_home: Some(format!("{root}/xdg")),
            xdg_data_dirs: Some(format!("{root}/sysdata")),
            path: Some(path.clone()),
            opt_root: format!("{root}/opt"),
        },
        schema_path_env: None,
        default_register: register,
        config_colors_disabled: colors_disabled,
    };
    let host = Host {
        platform: native_platform().unwrap(),
        env: Environment::from_pairs([
            ("PATH".to_owned(), path),
            ("SHELL".to_owned(), "/bin/bash".to_owned()),
            ("HOME".to_owned(), format!("{root}/home")),
        ]),
    };
    (
        ctx,
        ToolEnv {
            host,
            configured_shell: None,
        },
    )
}

fn run_case(
    case: &Value,
    root: &str,
    ctx: &AuthorContext,
    tools: &ToolEnv,
) -> Result<CommandOutcome, AuthorError> {
    let args: Vec<String> = case["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().replace("{root}", root))
        .collect();
    let rest = &args[1..];
    match args[0].as_str() {
        "describe" => {
            let words = positionals(rest, &["-r", "--register", "--format", "-f"]);
            author::describe(
                DescribeInput {
                    tool: words[0].clone(),
                    command: words.get(1).cloned(),
                    register: option(rest, &["-r", "--register"]),
                },
                ctx,
                tools,
            )
        }
        "validate" => {
            let words = positionals(rest, &["--format", "--schema", "--register"]);
            let target = match words[0].as_str() {
                "file" => ValidateTarget::File {
                    path: words[1].clone(),
                },
                "string" => ValidateTarget::String {
                    yaml: words[1].clone(),
                },
                _ => ValidateTarget::All,
            };
            author::validate_with_tools(
                ValidateInput {
                    target,
                    options: ValidateOptions {
                        verbose: flag(rest, "--verbose"),
                        format: option(rest, &["--format"]),
                        schema: option(rest, &["--schema"]),
                        strict: flag(rest, "--strict"),
                        executable: flag(rest, "--executable"),
                        all: flag(rest, "--all"),
                        register: option(rest, &["--register"]),
                    },
                },
                ctx,
                tools,
            )
        }
        other => panic!("unexpected verb {other}"),
    }
}

#[test]
fn tool_handlers_match_the_oracle_captures() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(data_dir().join("tools_cases.json")).unwrap(),
    )
    .unwrap();
    let mtime = UNIX_EPOCH + Duration::from_secs(fixture["fixed_mtime_unix"].as_u64().unwrap());
    let base = std::env::temp_dir().join(format!("ukiryu-author-tools-{}", std::process::id()));
    let original_dir = std::env::current_dir().unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (n, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let id = case["id"].as_str().unwrap();
        let mode = case["mode"].as_str().unwrap();
        let root_path = base.join(n.to_string());
        copy_tree(&data_dir().join("tree"), &root_path, mtime);
        std::fs::create_dir_all(root_path.join("home")).unwrap();
        std::fs::create_dir_all(root_path.join("bin")).unwrap();
        let root_path = std::fs::canonicalize(&root_path).unwrap();
        let root = root_path.to_string_lossy().into_owned();
        std::env::set_current_dir(&root_path).unwrap();
        let (ctx, tools) = contexts(case, &root);
        let run = run_case(case, &root, &ctx, &tools);
        checked += 1;
        let mut fail = |what: String| failures.push(format!("{id}: {what}"));
        match (mode, run) {
            (other, _) if other != "exact" && other != "tty" => {
                fail(format!("unexpected mode {other}"))
            }
            (_, Err(e)) => fail(format!("handler error {e:?}")),
            (_, Ok(outcome)) => {
                let (stdout, stderr) = perform(&outcome.writes);
                let expect = &case["expect"];
                if mode == "tty" || case["pty"].as_bool() == Some(true) {
                    let tty = case["tty_output"]
                        .as_str()
                        .unwrap()
                        .replace("{root}", &root)
                        .replace("\r\n", "\n");
                    let both = format!("{stdout}{stderr}");
                    if both != tty {
                        fail(format!("tty output\n{both:?}\nexpected\n{tty:?}"));
                    }
                } else {
                    let es = expect["stdout"].as_str().unwrap().replace("{root}", &root);
                    let ee = expect["stderr"].as_str().unwrap().replace("{root}", &root);
                    if stdout != es {
                        fail(format!("stdout\n{stdout:?}\nexpected\n{es:?}"));
                    }
                    if stderr != ee {
                        fail(format!("stderr\n{stderr:?}\nexpected\n{ee:?}"));
                    }
                }
                let exit = expect["exit"].as_i64().unwrap() as i32;
                if outcome.exit_code != exit {
                    fail(format!("exit {} expected {exit}", outcome.exit_code));
                }
            }
        }
    }
    std::env::set_current_dir(original_dir).unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(checked, 51, "{checked} cases checked");
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
