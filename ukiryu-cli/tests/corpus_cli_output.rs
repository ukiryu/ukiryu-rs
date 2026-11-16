//! The `cli-output` corpus rows through the compiled `ukiryu` binary.
//!
//! Each row of `corpus/conformance/cases/cli-output.jsonl` runs
//! `CARGO_BIN_EXE_ukiryu` as a real process, set up the way the Ruby oracle's
//! `op_cli_run` (corpus/oracle-ruby/lib/ops.rb) runs `exe/ukiryu`:
//!
//! - the test process environment, without the variables `Sandbox.setup!`
//!   deletes, plus the oracle's fixed values (`HOME`, `TZ`, `LANG`, `LC_ALL`,
//!   `SHELL`, `UKIRYU_REGISTER`, `COLUMNS`, `THOR_COLUMNS`, `TERM`,
//!   `GIT_REDIRECT_STDERR`) and the row's own `env`;
//! - a fresh empty home directory as `HOME` and as the working directory;
//! - `${INPUTS}` in `argv` replaced by `<corpus>/inputs`;
//! - stdin is the row's `stdin` (empty when absent), then closed.
//!
//! stdout, stderr and the exit status are compared exactly, after the oracle's
//! own scrub (`<corpus>/inputs` -> `${INPUTS}`, then the home -> `${HOME}`).
//! Nothing else is normalized.
//!
//! Rows that differ today are listed in [`EXPECTED_FAILURES`] with the reason.
//! A listed row must still differ: when it starts matching, this test fails so
//! the entry is removed. Every other applicable row must match.
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_ukiryu");

/// Variables `Sandbox.setup!` deletes from the oracle's environment.
const DELETED: [&str; 14] = [
    "UKIRYU_SCHEMA_PATH",
    "UKIRYU_TEST_SHELL",
    "UKIRYU_DEBUG",
    "UKIRYU_DEBUG_EXECUTABLE",
    "UKIRYU_CONFIG",
    "UKIRYU_TIMEOUT",
    "UKIRYU_SHELL",
    "UKIRYU_FORMAT",
    "UKIRYU_LOG_LEVEL",
    "NO_COLOR",
    "PSModulePath",
    "MSYSTEM",
    "MINGW_PREFIX",
    "WSL_DISTRO",
];

/// Rows the binary does not match yet, with the reason. Each must still
/// differ; remove the entry when the row starts matching.
const EXPECTED_FAILURES: &[(&str, &str)] = &[
    (
        "cli-output.commands-jq",
        "'commands' is gated in dispatch: its handler (PR #73) is not merged",
    ),
    (
        "cli-output.commands-unknown",
        "'commands' is gated in dispatch: its handler (PR #73) is not merged",
    ),
    (
        "cli-output.info-absent",
        "'info' is gated in dispatch: no handler is merged",
    ),
    (
        "cli-output.info-unknown",
        "'info' is gated in dispatch: no handler is merged",
    ),
    (
        "cli-output.opts-jq",
        "'opts' is gated in dispatch: no handler is merged",
    ),
    (
        "cli-output.opts-jq-process",
        "'opts' is gated in dispatch: no handler is merged",
    ),
    (
        "cli-output.resolve-jq",
        "'resolve' is gated in dispatch: its handler (PR #73) is not merged",
    ),
    (
        "cli-output.resolve-jq-1-0",
        "'resolve' is gated in dispatch: its handler (PR #73) is not merged",
    ),
    (
        "cli-output.describe-absent-text",
        "'describe' is gated in dispatch: its handler (PR #74) is not merged",
    ),
    (
        "cli-output.describe-absent-json",
        "'describe' is gated in dispatch: its handler (PR #74) is not merged",
    ),
    (
        "cli-output.describe-absent-yaml",
        "'describe' is gated in dispatch: its handler (PR #74) is not merged",
    ),
    (
        "cli-output.describe-absent-process",
        "'describe' is gated in dispatch: its handler (PR #74) is not merged",
    ),
];

fn platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "other"
    }
}

fn corpus_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance");
    std::fs::canonicalize(&dir).expect("corpus/conformance exists")
}

/// A fresh empty directory for one row's `HOME`.
fn sandbox_home(index: usize) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("ukiryu-cli-corpus-{}-{index}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

struct Observed {
    stdout: String,
    stderr: String,
    exit: Option<i32>,
}

fn run_row(input: &Value, inputs: &str, home: &Path) -> Result<Observed, String> {
    let argv: Vec<String> = input["argv"]
        .as_array()
        .ok_or("input.argv is not an array")?
        .iter()
        .map(|a| a.as_str().map(|s| s.replace("${INPUTS}", inputs)))
        .collect::<Option<_>>()
        .ok_or("input.argv has a non-string entry")?;
    let home_text = home.to_str().ok_or("home path is not UTF-8")?;
    let mut command = Command::new(BIN);
    command.args(&argv);
    for key in DELETED {
        command.env_remove(key);
    }
    command
        .env("HOME", home_text)
        .env("TZ", "UTC")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("SHELL", "/bin/bash")
        .env("UKIRYU_REGISTER", format!("{inputs}/register"))
        .env("COLUMNS", "100")
        .env("THOR_COLUMNS", "100")
        .env("TERM", "dumb")
        .env("GIT_REDIRECT_STDERR", "/dev/null");
    if let Some(env) = input.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            let value = value.as_str().ok_or("input.env has a non-string value")?;
            command.env(key, value);
        }
    }
    let mut child = command
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {BIN}: {e}"))?;
    {
        let mut pipe = child.stdin.take().unwrap();
        let stdin = input.get("stdin").and_then(Value::as_str).unwrap_or("");
        // A binary that does not read stdin may close it first.
        let _ = pipe.write_all(stdin.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    let text = |bytes: Vec<u8>, name: &str| {
        String::from_utf8(bytes).map_err(|e| format!("{name} is not UTF-8: {e}"))
    };
    Ok(Observed {
        stdout: text(out.stdout, "stdout")?,
        stderr: text(out.stderr, "stderr")?,
        exit: out.status.code(),
    })
}

/// The oracle's `scrub`: `<corpus>/inputs` first, then the home.
fn scrub(text: &str, inputs: &str, home: &str) -> String {
    text.replace(inputs, "${INPUTS}").replace(home, "${HOME}")
}

/// The first differing field, or `None` when the row matches.
fn difference(expected: &Value, observed: &Observed, inputs: &str, home: &str) -> Option<String> {
    let stdout = scrub(&observed.stdout, inputs, home);
    let stderr = scrub(&observed.stderr, inputs, home);
    if expected["stdout"].as_str() != Some(stdout.as_str()) {
        return Some(format!(
            "stdout: expected {:?}, got {stdout:?}",
            expected["stdout"]
        ));
    }
    if expected["stderr"].as_str() != Some(stderr.as_str()) {
        return Some(format!(
            "stderr: expected {:?}, got {stderr:?}",
            expected["stderr"]
        ));
    }
    let exit = observed.exit.map(i64::from);
    if expected["exit"].as_i64() != exit {
        return Some(format!("exit: expected {}, got {exit:?}", expected["exit"]));
    }
    None
}

#[test]
fn cli_output_rows_through_the_binary() {
    let corpus = corpus_dir();
    let inputs = corpus.join("inputs");
    let inputs = inputs.to_str().expect("UTF-8 corpus path").to_owned();
    let cases = std::fs::read_to_string(corpus.join("cases/cli-output.jsonl"))
        .expect("read cli-output.jsonl");
    let platform = platform();

    let mut seen: Vec<String> = Vec::new();
    let mut unexpected: Vec<String> = Vec::new();
    let (mut passed, mut expected_failed, mut skipped) = (0, 0, 0);
    for (index, line) in cases.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line).expect("row is JSON");
        let id = row["id"].as_str().expect("row id").to_owned();
        seen.push(id.clone());
        let applicable = row["applicability"]["platforms"]
            .as_array()
            .is_none_or(|ps| ps.iter().any(|p| p.as_str() == Some(platform)));
        if !applicable {
            skipped += 1;
            println!("SKIP  {id} (not captured on {platform})");
            continue;
        }
        let expected = &row["expect"]["ok"];
        assert!(
            expected.is_object(),
            "{id}: expect.ok is missing; this harness compares process output only"
        );
        let home = sandbox_home(index);
        let home_text = home.to_str().expect("UTF-8 home").to_owned();
        let diff = match run_row(&row["input"], &inputs, &home) {
            Ok(observed) => difference(expected, &observed, &inputs, &home_text),
            Err(error) => Some(format!("harness: {error}")),
        };
        let _ = std::fs::remove_dir_all(&home);
        let ledger = EXPECTED_FAILURES.iter().find(|(row_id, _)| *row_id == id);
        match (diff, ledger) {
            (None, None) => {
                passed += 1;
                println!("PASS  {id}");
            }
            (Some(diff), Some((_, reason))) => {
                expected_failed += 1;
                println!("XFAIL {id} ({reason})\n      {diff}");
            }
            (Some(diff), None) => {
                println!("FAIL  {id}\n      {diff}");
                unexpected.push(format!("{id} differs: {diff}"));
            }
            (None, Some((_, reason))) => {
                println!("XPASS {id} (listed: {reason})");
                unexpected.push(format!(
                    "{id} now matches; remove it from EXPECTED_FAILURES (listed: {reason})"
                ));
            }
        }
    }
    for (id, _) in EXPECTED_FAILURES {
        if !seen.iter().any(|s| s == id) {
            unexpected.push(format!(
                "{id} is in EXPECTED_FAILURES but not in the corpus"
            ));
        }
    }
    println!(
        "cli-output: {} rows, {passed} pass, {expected_failed} expected failures, \
         {skipped} skipped, {} unexpected",
        seen.len(),
        unexpected.len()
    );
    assert!(
        unexpected.is_empty(),
        "unexpected cli-output results:\n{}",
        unexpected.join("\n")
    );
}
