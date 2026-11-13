//! Runs `Tool::probe_flag` on real processes through the executor.
//!
//! * Unix: the `/bin/sh` scripts of `tool_version_compat_data/oracle.json`,
//!   run with bash. Each outcome (the boolean, or the error class and
//!   message) must equal what the pinned Ruby `Tool#probe_flag` (ukiryu/ukiryu
//!   ab6df353) produced on the same scripts.
//! * Windows: `.cmd` scripts with the same behaviour as the `p_ok`, `p_unk`
//!   and `p_fail` scripts, run with cmd; each must give the same outcome as
//!   that oracle row (boolean, or error class and exit status).
//! * Every host: `jq` and `git`, which GitHub-hosted runners provide. The
//!   pinned Ruby code on Linux (jq 1.6, git 2.34.1) answered `true` for
//!   `jq --tab --help` and raised `ExecutionError` for `jq --bogus --help`
//!   (status 2) and `git --bogus --help` (status 129). On CI (`CI` set) a
//!   missing tool fails the test; elsewhere it is reported as unavailable
//!   and the test returns without claiming a pass.
use serde_json::Value as Json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ukiryu::executable::DiscoveryContext;
use ukiryu::executor::ExecuteError;
use ukiryu::models::{Model, ToolDefinition};
use ukiryu::runtime::{native_platform, ShellName};
use ukiryu::tool::{Tool, ToolError, ToolOptions};

fn oracle() -> Json {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/tool_version_compat_data/oracle.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("oracle")).expect("json")
}

fn temp_root() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "ukiryu-tool-compat-real-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
}

/// The error class, message and (for a failed run) exit status.
fn error_parts(e: &ToolError) -> (String, String, Option<i64>) {
    match e {
        ToolError::Execute(inner) => match inner.as_ref() {
            ExecuteError::InvalidByteSequence { error, .. } => (
                error.ruby_class().to_owned(),
                error.message().to_owned(),
                None,
            ),
            ExecuteError::Failed { error, execution } => (
                error.ruby_class().to_owned(),
                error.message().to_owned(),
                Some(execution.result.status()),
            ),
            other => (
                other
                    .kind()
                    .map(|k| k.ruby_class().to_owned())
                    .unwrap_or_else(|| format!("{other:?}")),
                other.to_string(),
                None,
            ),
        },
        other => (format!("{other:?}"), other.to_string(), None),
    }
}

fn script_tool(yaml: &str, dir: &str, shell: ShellName, pathext: Option<&str>) -> Tool {
    let platform = native_platform().expect("supported host");
    let options = ToolOptions::new(platform, shell).discovery(DiscoveryContext::new(
        platform,
        shell.as_str(),
        Some(dir),
        pathext,
    ));
    Tool::new(
        ToolDefinition::from_yaml(yaml).expect("definition"),
        &options,
    )
    .expect("tool")
}

#[cfg(unix)]
#[test]
fn shell_scripts_match_ruby_processes() {
    use std::os::unix::fs::PermissionsExt;

    let data = oracle();
    let root = temp_root();
    let bin = root.join("bin");
    let empty = root.join("empty");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    for (name, body) in data["probe_scripts"].as_object().unwrap() {
        let p = bin.join(name);
        std::fs::write(&p, body.as_str().unwrap()).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let bin_s = bin.to_str().unwrap().to_owned();
    let mut failures = Vec::new();
    let rows = data["probe"].as_array().unwrap();
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let dir = if row["path"].as_str().unwrap().starts_with("empty") {
            empty.to_str().unwrap()
        } else {
            bin_s.as_str()
        };
        let tool = script_tool(row["yaml"].as_str().unwrap(), dir, ShellName::Bash, None);
        let got = tool.probe_flag(row["flag"].as_str().unwrap());
        let ok = match (&got, row.get("result")) {
            (Ok(b), Some(expected)) => expected.as_bool() == Some(*b),
            (Err(e), None) => {
                let (class, message, _) = error_parts(e);
                row["error"]["class"] == class.as_str()
                    && row["error"]["message"] == message.replace(&bin_s, "<BIN>").as_str()
            }
            _ => false,
        };
        if !ok {
            failures.push(format!(
                "{id}: {got:?} vs {:?} {:?}",
                row.get("result"),
                row.get("error")
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    eprintln!("probe_flag scripts: {} rows", rows.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[cfg(windows)]
#[test]
fn cmd_scripts_match_ruby_outcomes() {
    let data = oracle();
    let rows = data["probe"].as_array().unwrap();
    let expected = |id: &str| {
        rows.iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("oracle row {id}"))
            .clone()
    };
    let root = temp_root();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let scripts = [
        (
            "w_ok",
            "@echo usage: %*\r\n@exit /b 0\r\n",
            "probe/p_ok/\"--worktree\"",
        ),
        (
            "w_unk",
            "@echo error: unknown option %1 1>&2\r\n@exit /b 0\r\n",
            "probe/p_unk/\"--worktree\"",
        ),
        (
            "w_fail",
            "@echo out\r\n@echo unknown flag %1 1>&2\r\n@exit /b 2\r\n",
            "probe/p_fail/\"--worktree\"",
        ),
    ];
    let mut failures = Vec::new();
    for (name, body, oracle_id) in scripts {
        std::fs::write(bin.join(format!("{name}.cmd")), body).unwrap();
        let yaml = format!(
            "name: synth\nprofiles:\n- name: default\n  executable_name: {name}\n  commands:\n  - name: run\n"
        );
        let tool = script_tool(&yaml, bin.to_str().unwrap(), ShellName::Cmd, Some(".CMD"));
        assert!(tool.is_available(), "{name}: script not found");
        let row = expected(oracle_id);
        let got = tool.probe_flag("--worktree");
        let ok = match (&got, row.get("result")) {
            (Ok(b), Some(e)) => e.as_bool() == Some(*b),
            (Err(e), None) => {
                let (class, _, status) = error_parts(e);
                row["error"]["class"] == class.as_str() && status == Some(2)
            }
            _ => false,
        };
        if !ok {
            failures.push(format!(
                "{name}: {got:?} vs {:?} {:?}",
                row.get("result"),
                row.get("error")
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn native_tool(name: &str, shell: ShellName) -> Option<Tool> {
    let platform = native_platform().expect("supported host");
    let yaml = format!(
        "name: {name}\nprofiles:\n- name: default\n  executable_name: {name}\n  commands:\n  - name: run\n"
    );
    let tool = Tool::new(
        ToolDefinition::from_yaml(&yaml).expect("definition"),
        &ToolOptions::new(platform, shell),
    )
    .expect("tool");
    if tool.is_available() {
        return Some(tool);
    }
    let reason = tool.unavailability_reason().unwrap_or_default();
    assert!(
        std::env::var_os("CI").is_none(),
        "tool expected on CI runners: {reason}"
    );
    eprintln!("UNAVAILABLE (not a pass): {reason}");
    None
}

fn default_shell() -> ShellName {
    if cfg!(windows) {
        ShellName::Cmd
    } else {
        ShellName::Bash
    }
}

#[test]
fn jq_supported_flag_is_true() {
    let Some(jq) = native_tool("jq", default_shell()) else {
        return;
    };
    assert!(jq.probe_flag("--tab").expect("jq --tab --help runs"));
}

#[test]
fn jq_unknown_flag_fails_the_run() {
    let Some(jq) = native_tool("jq", default_shell()) else {
        return;
    };
    let err = jq.probe_flag("--bogus").expect_err("non-zero exit raises");
    let (class, message, status) = error_parts(&err);
    assert_eq!(class, "Ukiryu::Errors::ExecutionError", "{message}");
    assert_eq!(status, Some(2), "{message}");
}

#[test]
fn git_unknown_flag_fails_the_run() {
    // git lives under "C:\Program Files" on Windows; run it through
    // PowerShell there (see tool_runtime_real.rs). jq covers cmd.
    let shell = if cfg!(windows) {
        ShellName::PowerShell
    } else {
        ShellName::Bash
    };
    let Some(git) = native_tool("git", shell) else {
        return;
    };
    let err = git.probe_flag("--bogus").expect_err("non-zero exit raises");
    let (class, message, status) = error_parts(&err);
    assert_eq!(class, "Ukiryu::Errors::ExecutionError", "{message}");
    assert_eq!(status, Some(129), "{message}");
}
