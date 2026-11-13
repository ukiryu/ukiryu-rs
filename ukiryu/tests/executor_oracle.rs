//! `Ukiryu::Executor.execute` against frozen Ruby observations.
//!
//! Golden data: tests/executor_data/observations.json, captured by running
//! the pinned oracle ukiryu/ukiryu@ab6df353 (lib/ukiryu/executor.rb,
//! lib/ukiryu/shell/{unix_base,tcsh}.rb, lib/ukiryu/errors.rb) with Ruby
//! 4.0.1 and GNU bash 5.1 on Linux. Rows marked `parity` must match the
//! observation. Rows marked `migrated` keep the Ruby observation and check
//! the target behaviour named in `target` (traced in `trace`).
//!
//! These rows run real processes through bash, so they are Unix-only. The
//! Windows and PowerShell checks live in executor_process.rs.
#![cfg(unix)]

use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use ukiryu::environment::Environment;
use ukiryu::errors::ErrorKind;
use ukiryu::execution::Seconds;
use ukiryu::execution_context::{ExecutionContext, MissingSetting};
use ukiryu::executor::{
    execute, extract_status, ExecuteError, ExecuteRequest, Execution, ProcessStatus,
};
use ukiryu::runtime::ShellName;

fn golden() -> &'static Value {
    static GOLDEN: OnceLock<Value> = OnceLock::new();
    GOLDEN.get_or_init(|| {
        serde_json::from_str(include_str!("executor_data/observations.json")).expect("golden json")
    })
}

fn row(id: &str) -> &'static Value {
    golden()["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no golden row {id}"))
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn bash(executable: &str, args: &[&str], timeout: Seconds) -> ExecuteRequest {
    ExecuteRequest::new(
        executable,
        strings(args),
        Environment::new(),
        ShellName::Bash,
        timeout,
    )
}

fn seconds(v: &Value) -> Seconds {
    match v.as_i64() {
        Some(i) => Seconds::Int(i),
        None => Seconds::Float(v.as_f64().expect("numeric timeout")),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares a returned result with a `result` row.
fn assert_result(id: &str, execution: &Execution) {
    let row = row(id);
    assert_eq!(row["outcome"], "result", "{id}");
    let result = &execution.result;
    if let Some(stdout) = row["stdout"].as_str() {
        assert_eq!(result.stdout(), stdout.as_bytes(), "{id} stdout");
    }
    assert_eq!(
        hex(&execution.stdout),
        row["stdout_bytes"]["hex"].as_str().expect("hex"),
        "{id} stdout bytes"
    );
    assert_eq!(
        result.stderr(),
        row["stderr"].as_str().expect("stderr").as_bytes(),
        "{id}"
    );
    assert_eq!(
        result.exit_status(),
        row["exit_status"].as_i64().unwrap(),
        "{id}"
    );
    assert_eq!(
        result.is_success(),
        row["success"].as_bool().unwrap(),
        "{id}"
    );
    let info = &result.command_info;
    assert_eq!(info.executable, row["executable"].as_str().unwrap(), "{id}");
    let args: Vec<&str> = row["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(info.arguments, args, "{id}");
    assert_eq!(
        info.full_command,
        row["full_command"].as_str().unwrap(),
        "{id}"
    );
    let shell = row["shell"].as_str().unwrap().trim_start_matches(':');
    assert_eq!(info.shell.as_deref(), Some(shell), "{id}");
    assert_eq!(
        result.metadata.timeout,
        Some(seconds(&row["timeout"])),
        "{id}"
    );
    assert!(result.started_at().is_some() && result.finished_at().is_some());
    assert!(result.duration().unwrap() >= 0.0, "{id} duration");
}

/// Compares an `ExecutionError` with a `raise` row; the target keeps the run.
fn assert_failed(id: &str, outcome: Result<Execution, ExecuteError>) -> Execution {
    let row = row(id);
    assert_eq!(row["outcome"], "raise", "{id}");
    match outcome {
        Err(ExecuteError::Failed { error, execution }) => {
            assert_eq!(error.ruby_class(), row["class"].as_str().unwrap(), "{id}");
            assert_eq!(error.message(), row["message"].as_str().unwrap(), "{id}");
            assert!(execution.result.is_failure(), "{id}");
            *execution
        }
        other => panic!("{id}: expected ExecutionError, got {other:?}"),
    }
}

fn run(request: ExecuteRequest) -> Execution {
    execute(request).unwrap_or_else(|e| panic!("execute failed: {e}"))
}

#[test]
fn golden_provenance() {
    let oracle = &golden()["oracle"];
    assert_eq!(oracle["sha"], "ab6df353c6dc2d7faf7817a86aafcafa82579896");
    for r in golden()["rows"].as_array().unwrap() {
        let contract = r["contract"].as_str().unwrap();
        assert!(contract == "parity" || contract == "migrated", "{r}");
        if contract == "migrated" {
            assert!(r["trace"].is_string() && r["target"].is_string(), "{r}");
        }
    }
}

// --- Required settings (D1, migrated) ---

#[test]
fn missing_shell_and_timeout_keep_source_messages() {
    let context = ExecutionContext::new();
    let err = context.run_settings(Environment::new()).unwrap_err();
    assert_eq!(err, MissingSetting::Shell);
    assert_eq!(
        err.to_string(),
        row("missing_shell")["message"].as_str().unwrap()
    );

    let context = context.with_shell(ShellName::Bash);
    let err = context.run_settings(Environment::new()).unwrap_err();
    assert_eq!(err, MissingSetting::Timeout);
    assert_eq!(
        err.to_string(),
        row("missing_timeout")["message"].as_str().unwrap()
    );
}

#[test]
fn explicit_empty_environment_matches_source_default() {
    let settings = ExecutionContext::new()
        .with_shell(ShellName::Bash)
        .with_timeout_secs(30)
        .run_settings(Environment::new())
        .unwrap();
    let execution = run(ExecuteRequest::from_settings(
        "echo",
        strings(&["x"]),
        settings,
    ));
    assert_result("missing_env_defaults", &execution);
}

#[test]
fn non_positive_or_non_finite_timeouts_are_rejected() {
    assert_eq!(row("timeout_zero_runs_unbounded")["outcome"], "result");
    assert_eq!(
        row("timeout_negative")["message"],
        "Timeout sec must be a non-negative number"
    );
    for timeout in [
        Seconds::Int(0),
        Seconds::Int(-1),
        Seconds::Float(0.0),
        Seconds::Float(-0.5),
        Seconds::Float(f64::NAN),
        Seconds::Float(f64::INFINITY),
    ] {
        match execute(bash("echo", &["x"], timeout)) {
            Err(ExecuteError::InvalidTimeout(t)) => {
                assert_eq!(format!("{t:?}"), format!("{timeout:?}"))
            }
            other => panic!("{timeout:?}: {other:?}"),
        }
    }
}

// --- Success and capture ---

#[test]
fn echo_rows() {
    assert_result("echo_test", &run(bash("echo", &["test"], Seconds::Int(30))));
    assert_result(
        "echo_hello_world",
        &run(bash("echo", &["hello world"], Seconds::Int(30))),
    );
    assert_result(
        "special_chars",
        &run(bash("echo", &["hello \"quoted\" test's"], Seconds::Int(90)).allow_failure(true)),
    );
    assert_result(
        "timeout_float_success",
        &run(bash("sleep", &["0.01"], Seconds::Int(5)).allow_failure(true)),
    );
}

#[test]
fn command_info_records_tool_and_command_names() {
    let execution = run(bash("echo", &["test"], Seconds::Int(90))
        .tool_name("echo-tool")
        .command_name("print"));
    let info = &execution.result.command_info;
    assert_eq!(info.tool_name.as_deref(), Some("echo-tool"));
    assert_eq!(info.command_name.as_deref(), Some("print"));
}

#[test]
fn repeated_executions() {
    for _ in 0..5 {
        assert_result("echo_test", &run(bash("echo", &["test"], Seconds::Int(30))));
    }
}

// --- Exit statuses and ExecutionError ---

#[test]
fn exit_status_rows() {
    assert_result(
        "exit_42_allow_failure",
        &run(bash("sh", &["-c", "exit 42"], Seconds::Int(30)).allow_failure(true)),
    );
    assert_failed(
        "exit_1_raises",
        execute(bash("sh", &["-c", "exit 1"], Seconds::Int(30))),
    );
    let failed = assert_failed(
        "stderr_in_error",
        execute(bash(
            "sh",
            &["-c", "echo error >&2; exit 1"],
            Seconds::Int(30),
        )),
    );
    assert_eq!(failed.result.stderr(), b"error\n");
    assert_failed(
        "false_raises",
        execute(bash("false", &[], Seconds::Int(30))),
    );
    assert_failed(
        "exit_42_raises",
        execute(bash("sh", &["-c", "exit 42"], Seconds::Int(90))),
    );
    assert_failed(
        "stdout_and_stderr_stripped",
        execute(bash(
            "sh",
            &[
                "-c",
                "printf \"  out \\n\"; printf \"\\n err  \\n\" >&2; exit 3",
            ],
            Seconds::Int(30),
        )),
    );
}

#[test]
fn failure_kind_is_execution_error() {
    let err = execute(bash("sh", &["-c", "exit 1"], Seconds::Int(30))).unwrap_err();
    assert_eq!(err.kind(), Some(ErrorKind::Execution));
}

#[test]
fn signalled_child_reports_128_plus_signal() {
    assert_result(
        "signal_term",
        &run(bash("sh", &["-c", "kill -TERM $$"], Seconds::Int(30)).allow_failure(true)),
    );
}

#[test]
fn missing_executable_is_status_127() {
    let row = row("missing_executable");
    let execution = run(bash("ukiryu_no_such_exe_xyz", &[], Seconds::Int(30)).allow_failure(true));
    assert_eq!(execution.result.exit_status(), 127);
    assert_eq!(
        execution.result.exit_status(),
        row["exit_status"].as_i64().unwrap()
    );
    assert!(std::str::from_utf8(execution.result.stderr())
        .expect("UTF-8 stderr")
        .contains("ukiryu_no_such_exe_xyz: command not found"));
    assert!(row["stderr"]
        .as_str()
        .unwrap()
        .contains("ukiryu_no_such_exe_xyz: command not found"));
}

#[test]
fn extract_status_table() {
    for r in golden()["extract_status"].as_array().unwrap() {
        let status = match r["id"].as_str().unwrap() {
            "exited_42" => ProcessStatus::Exited(42),
            "signaled_15" => ProcessStatus::Signaled(15),
            "stopped_19" => ProcessStatus::Stopped(19),
            "unknown" => ProcessStatus::Unknown,
            other => panic!("unknown status row {other}"),
        };
        assert_eq!(extract_status(status), r["status"].as_i64().unwrap(), "{r}");
    }
}

// --- Timeout (the source 10-second case; migrated kill semantics) ---

fn assert_timeout(id: &str, request: ExecuteRequest, timeout: Seconds) {
    let row = row(id);
    let start = Instant::now();
    let err = execute(request).unwrap_err();
    let elapsed = start.elapsed();
    match &err {
        ExecuteError::TimedOut(error) => {
            assert_eq!(error.ruby_class(), row["class"].as_str().unwrap());
            assert_eq!(error.message(), row["message"].as_str().unwrap());
            assert!(row["timeout_attr"].is_null(), "source leaves timeout nil");
            assert_eq!(error.timeout(), Some(timeout));
        }
        other => panic!("{id}: expected timeout, got {other:?}"),
    }
    assert_eq!(err.kind(), Some(ErrorKind::Timeout));
    assert!(
        elapsed < Duration::from_secs(5),
        "{id}: killed at the deadline, took {elapsed:?}"
    );
}

#[test]
fn sleep_10_times_out_and_is_killed() {
    let timeout = Seconds::Float(0.01);
    assert_timeout("timeout_sleep_10", bash("sleep", &["10"], timeout), timeout);
}

#[test]
fn integer_timeout_message() {
    let timeout = Seconds::Int(1);
    assert_timeout(
        "timeout_int_message",
        bash("sleep", &["10.0372"], timeout),
        timeout,
    );
}

#[test]
fn timeout_kills_descendants() {
    let dir = temp_dir("descendants");
    let marker = dir.join("late");
    let script = format!("(sleep 2; echo late > '{}') & sleep 10", marker.display());
    let err = execute(bash("sh", &["-c", &script], Seconds::Float(0.5))).unwrap_err();
    assert!(matches!(err, ExecuteError::TimedOut(_)), "{err:?}");
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        !marker.exists(),
        "a descendant survived the timeout and wrote {marker:?}"
    );
}

/// Whether a process is gone: no such pid, or a zombie awaiting its reaper.
fn terminated(pid: &str) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .expect("run ps");
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    stat.is_empty() || stat.starts_with('Z')
}

#[test]
fn timeout_kills_child_and_grandchild_that_hold_the_pipes() {
    let dir = temp_dir("tree-ids");
    let ids = dir.join("ids");
    // The shell records its own pid and a grandchild that inherits stdout
    // and stderr, then waits for it.
    let script = format!(
        "echo $$ > '{0}'; sleep 30 & echo $! >> '{0}'; wait",
        ids.display()
    );
    let start = Instant::now();
    let err = execute(bash("sh", &["-c", &script], Seconds::Int(2))).unwrap_err();
    let elapsed = start.elapsed();
    assert!(matches!(err, ExecuteError::TimedOut(_)), "{err:?}");
    assert!(
        elapsed < Duration::from_secs(6),
        "returned after {elapsed:?}"
    );
    let recorded = std::fs::read_to_string(&ids).expect("pid file");
    let pids: Vec<&str> = recorded.split_whitespace().collect();
    assert_eq!(pids.len(), 2, "{recorded:?}");
    let mut gone = false;
    for _ in 0..50 {
        gone = pids.iter().all(|pid| terminated(pid));
        if gone {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!(
        "EVIDENCE executor tree timeout: child={} grandchild={} returned_after_ms={} all_terminated={gone}",
        pids[0],
        pids[1],
        elapsed.as_millis()
    );
    assert!(gone, "processes {pids:?} survived the timeout");
}

#[test]
fn descendant_holding_pipes_waits_then_times_out_at_deadline() {
    // Parity: the run lasts until inherited pipes close.
    let start = Instant::now();
    let execution = run(bash(
        "sh",
        &["-c", "sleep 1.5 & echo done"],
        Seconds::Int(30),
    ));
    assert!(start.elapsed() >= Duration::from_millis(1400));
    assert_result("descendant_holds_pipe", &execution);
    assert!(row("descendant_holds_pipe")["elapsed_ge_1_4s"]
        .as_bool()
        .unwrap());

    let start = Instant::now();
    let execution = run(bash(
        "sh",
        &["-c", "sleep 1.5 >/dev/null 2>&1 & echo done"],
        Seconds::Int(30),
    ));
    assert!(start.elapsed() < Duration::from_millis(1400));
    assert_result("descendant_detached_pipes", &execution);

    // Target: the deadline also covers pipe closure.
    let start = Instant::now();
    let err = execute(bash("sh", &["-c", "sleep 10 & echo done"], Seconds::Int(1))).unwrap_err();
    assert!(matches!(err, ExecuteError::TimedOut(_)), "{err:?}");
    assert!(start.elapsed() < Duration::from_secs(5));
}

// --- Environment (D1) ---

#[test]
fn environment_rows() {
    let with = |pairs: &[(&str, &str)]| Environment::from_pairs(pairs.iter().copied());
    let echo = |script: &str, env: Environment| {
        ExecuteRequest::new(
            "sh",
            strings(&["-c", script]),
            env,
            ShellName::Bash,
            Seconds::Int(30),
        )
        .allow_failure(true)
    };
    assert_result(
        "env_hash_var",
        &run(echo(
            "echo \"$TEST_VAR\"",
            with(&[("TEST_VAR", "test_value")]),
        )),
    );
    assert_result(
        "env_hash_spaces",
        &run(echo(
            "echo \"$TEST_VAR\"",
            with(&[("TEST_VAR", "value with spaces")]),
        )),
    );
    assert_result(
        "env_empty_hash_inherits_home",
        &run(echo("echo \"${HOME:+home-set}\"", Environment::new())),
    );
    assert_result(
        "env_object_without_home_still_inherits",
        &run(echo(
            "echo \"${HOME:+home-set}|$ONLY\"",
            with(&[("ONLY", "1")]),
        )),
    );
}

// The DISPLAY and time-zone rows change the process environment, so they
// run in their own test binary (executor_host_env.rs).

// --- Working directory ---

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ukiryu-executor-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cwd_row() {
    let dir = temp_dir("cwd");
    std::fs::write(dir.join("test.txt"), "test content").unwrap();
    let execution = run(bash("cat", &["test.txt"], Seconds::Int(30))
        .cwd(dir)
        .allow_failure(true));
    assert_result("cwd_cat", &execution);
}

// --- stdin ---

#[test]
fn stdin_rows() {
    let cat = |timeout: i64| bash("/bin/cat", &[], Seconds::Int(timeout));
    assert_result("stdin_string", &run(cat(90).stdin_bytes("test data")));
    assert_result(
        "stdin_multiline",
        &run(cat(90).stdin_bytes("line1\nline2\nline3")),
    );
    assert_result(
        "stdin_binary",
        &run(cat(90).stdin_bytes(vec![0u8, 1, 2, 3])),
    );
    assert_result("stdin_empty_string", &run(cat(30).stdin_bytes("")));
    assert_result("stdin_absent_is_closed", &run(cat(5)));
    assert_result(
        "stdin_head_epipe",
        &run(bash("/usr/bin/head", &["-n", "1"], Seconds::Int(90))
            .stdin_bytes("line1\nline2\nline3\n")),
    );
    assert_result(
        "stdin_head_epipe_large",
        &run(bash("/usr/bin/head", &["-n", "1"], Seconds::Int(30))
            .stdin_bytes("line\n".repeat(200_000))),
    );
}

#[test]
fn stdin_from_reader() {
    let dir = temp_dir("stdin-file");
    let path = dir.join("input");
    std::fs::write(&path, "file content").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    assert_result(
        "stdin_io_file",
        &run(bash("/bin/cat", &[], Seconds::Int(90)).stdin_reader(file)),
    );
    let pipe_like = std::io::Cursor::new(b"pipe data".to_vec());
    let execution = run(bash("/bin/cat", &[], Seconds::Int(90)).stdin_reader(pipe_like));
    assert_eq!(execution.result.stdout(), b"pipe data");
}

#[test]
fn large_stdin_does_not_deadlock() {
    let row = row("stdin_large_cat_deadlock");
    assert_eq!(
        row["class"], "Ukiryu::Errors::TimeoutError",
        "frozen source deadlock"
    );
    let input = vec![b'x'; row["input_bytes"].as_u64().unwrap() as usize];
    let execution = run(bash("/bin/cat", &[], Seconds::Int(30)).stdin_bytes(input.clone()));
    assert_eq!(execution.stdout, input);
    assert!(execution.result.is_success());
}

// --- Output bytes ---

#[test]
fn utf8_and_invalid_output() {
    assert_result(
        "stdout_utf8",
        &run(bash("printf", &["h\\303\\251"], Seconds::Int(30))),
    );
    let execution = run(bash("printf", &["a\\377b"], Seconds::Int(30)));
    assert_result("stdout_invalid_utf8", &execution);
    // Ruby keeps the invalid byte (row stdout_bytes 61ff62); no U+FFFD.
    assert_eq!(execution.result.stdout(), b"a\xffb");
}

#[test]
fn large_stdout_and_stderr_together() {
    let row = row("large_both_streams");
    let execution = run(bash(
        "sh",
        &[
            "-c",
            "head -c 1000000 /dev/zero | tr \"\\0\" o; head -c 1000000 /dev/zero | tr \"\\0\" e >&2",
        ],
        Seconds::Int(30),
    ));
    assert_eq!(
        execution.result.command_info.full_command,
        row["full_command"].as_str().unwrap()
    );
    assert_eq!(
        execution.stdout.len() as u64,
        row["stdout_len"].as_u64().unwrap()
    );
    assert_eq!(
        execution.stderr.len() as u64,
        row["stderr_len"].as_u64().unwrap()
    );
    assert!(execution.stdout.iter().all(|&b| b == b'o'));
    assert!(execution.stderr.iter().all(|&b| b == b'e'));
}

// --- Shell lookup and spawn failure ---

#[test]
fn shell_not_on_search_path() {
    let empty = temp_dir("empty-path");
    for (id, shell) in [
        ("shell_not_found", ShellName::Zsh),
        ("tcsh_not_found", ShellName::Tcsh),
    ] {
        let request = ExecuteRequest::new(
            "echo",
            strings(&["x"]),
            Environment::new(),
            shell,
            Seconds::Int(30),
        )
        .shell_search_path(empty.as_os_str());
        match execute(request) {
            Err(ExecuteError::ShellNotFound { shell: s, message }) => {
                assert_eq!(s, shell);
                assert_eq!(message, row(id)["message"].as_str().unwrap());
                assert_eq!(row(id)["class"], "RuntimeError");
            }
            other => panic!("{id}: {other:?}"),
        }
    }
}

#[test]
fn spawn_failure_is_reported() {
    // There is no `cmd` program on Unix hosts.
    let request = ExecuteRequest::new(
        "echo",
        strings(&["x"]),
        Environment::new(),
        ShellName::Cmd,
        Seconds::Int(30),
    );
    match execute(request) {
        Err(ExecuteError::Spawn { program, source }) => {
            assert_eq!(program, "cmd");
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("{other:?}"),
    }
}
