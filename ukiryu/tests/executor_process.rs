//! Real-process checks of the executor on every shell the host offers.
//!
//! - Unix shells (bash, zsh, sh, dash, fish, tcsh): an argument with quotes
//!   and an environment variable reach the child unchanged through
//!   `<shell> -c <join>`.
//! - cmd (Windows only): exit status, stdout, stdin and timeout.
//! - PowerShell (`powershell` on Windows, `pwsh` elsewhere): exit status,
//!   stdout and the `Get-Content` stdin path.
//!
//! These rows have no Ruby capture on the host (the oracle was captured on
//! Linux only); they check the target contract. Shells are found on PATH.
//! With `UKIRYU_REQUIRE_SHELL_TESTS=1`, a missing Unix shell fails on Linux
//! and a missing cmd or PowerShell fails on Windows; anything else missing
//! is reported as UNVERIFIED and not counted as a pass.

use std::time::{Duration, Instant};
use ukiryu::environment::Environment;
use ukiryu::execution::Seconds;
use ukiryu::executor::{execute, ExecuteError, ExecuteRequest};
use ukiryu::runtime::ShellName;

fn required() -> bool {
    std::env::var("UKIRYU_REQUIRE_SHELL_TESTS").as_deref() == Ok("1")
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn request(shell: ShellName, executable: &str, args: &[&str]) -> ExecuteRequest {
    ExecuteRequest::new(
        executable,
        strings(args),
        Environment::new(),
        shell,
        Seconds::Int(120),
    )
}

#[cfg(unix)]
#[test]
fn unix_shells_pass_arguments_and_environment() {
    let path = std::env::var_os("PATH");
    let value = "hello \"quoted\" test's $HOME `x` ; & | *";
    let mut verified = Vec::new();
    for shell in [
        ShellName::Bash,
        ShellName::Zsh,
        ShellName::Sh,
        ShellName::Dash,
        ShellName::Fish,
        ShellName::Tcsh,
    ] {
        if ukiryu::executor::find_executable_in(shell.as_str(), path.as_deref(), None, false)
            .is_none()
        {
            assert!(
                !(required() && cfg!(target_os = "linux")),
                "{shell} is not on PATH but UKIRYU_REQUIRE_SHELL_TESTS=1"
            );
            eprintln!("UNVERIFIED executor_process: {shell} is not on PATH; not passed");
            continue;
        }
        let execution = execute(request(shell, "printf", &["%s", value]))
            .unwrap_or_else(|e| panic!("{shell}: {e}"));
        assert_eq!(execution.result.stdout(), value.as_bytes(), "{shell}");
        assert_eq!(
            execution.result.command_info.shell.as_deref(),
            Some(shell.as_str())
        );

        let env = Environment::from_pairs([("UKIRYU_EXEC_VALUE", "value-\u{e9} 1")]);
        let execution = execute(ExecuteRequest::new(
            "printenv",
            strings(&["UKIRYU_EXEC_VALUE"]),
            env,
            shell,
            Seconds::Int(120),
        ))
        .unwrap_or_else(|e| panic!("{shell}: {e}"));
        assert_eq!(
            execution.result.stdout(),
            "value-\u{e9} 1\n".as_bytes(),
            "{shell}"
        );

        let execution = execute(request(shell, "sh", &["-c", "exit 7"]).allow_failure(true))
            .unwrap_or_else(|e| panic!("{shell}: {e}"));
        assert_eq!(execution.result.exit_status(), 7, "{shell}");
        verified.push(shell);
    }
    eprintln!("executor_process verified unix shells: {verified:?}");
}

#[cfg(windows)]
#[test]
fn cmd_status_output_stdin_and_timeout() {
    let execution = execute(request(ShellName::Cmd, "cmd", &["/c", "exit 42"]).allow_failure(true))
        .expect("cmd exit");
    assert_eq!(execution.result.exit_status(), 42);
    assert_eq!(execution.result.command_info.full_command, "cmd /c exit 42");

    let err = execute(request(ShellName::Cmd, "cmd", &["/c", "exit 1"])).unwrap_err();
    match err {
        ExecuteError::Failed { error, .. } => {
            assert!(error.message().starts_with("Command failed: cmd"));
            assert!(error.message().contains("Exit status: 1"));
        }
        other => panic!("{other:?}"),
    }

    let execution = execute(request(ShellName::Cmd, "echo", &["hello"])).expect("cmd echo");
    assert_eq!(utf8(execution.result.stdout()).trim(), "hello");

    let execution =
        execute(request(ShellName::Cmd, "more", &[]).stdin_bytes("test data")).expect("cmd stdin");
    assert_eq!(utf8(execution.result.stdout()).trim(), "test data");

    let start = Instant::now();
    let err = execute(ExecuteRequest::new(
        "ping",
        strings(&["-n", "30", "127.0.0.1"]),
        Environment::new(),
        ShellName::Cmd,
        Seconds::Int(2),
    ))
    .unwrap_err();
    assert!(matches!(err, ExecuteError::TimedOut(_)), "{err:?}");
    assert_eq!(err.to_string(), "Command timed out after 2 seconds: ping");
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
}

fn powershell_available() -> bool {
    let program = if cfg!(windows) { "powershell" } else { "pwsh" };
    let found = std::process::Command::new(program)
        .args(["-NoLogo", "-NoProfile", "-Command", "exit 0"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found {
        assert!(
            !(required() && cfg!(windows)),
            "UKIRYU_REQUIRE_SHELL_TESTS=1 but {program} is not runnable"
        );
        eprintln!("UNVERIFIED executor_process: {program} not runnable; not passed");
    }
    found
}

/// The pinned Ruby executor's exact output bytes for this host, captured
/// by .github/workflows/executor-reference.yml
/// (tests/executor_data/capture/capture_native_shells.rb).
fn native_golden() -> Option<(String, serde_json::Value)> {
    let name = if cfg!(windows) {
        "native_shells_windows.json"
    } else if cfg!(target_os = "linux") {
        "native_shells_linux.json"
    } else {
        return None;
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/executor_data")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    Some((
        name.to_owned(),
        serde_json::from_str(&text).expect("golden json"),
    ))
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A stream that must be valid UTF-8 for the assertion to apply.
#[allow(dead_code)]
fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("UTF-8 stream")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// PowerShell (and on Windows cmd) rows: stdout and stderr bytes, status
/// and command line equal the Ruby capture exactly, byte order marks
/// included.
#[test]
fn native_shell_rows_match_ruby_bytes() {
    let Some((name, golden)) = native_golden() else {
        eprintln!("UNVERIFIED executor_process: no Ruby capture for this host; not passed");
        return;
    };
    if !powershell_available() {
        return;
    }
    let rows = golden["rows"].as_array().expect("rows");
    assert!(!rows.is_empty(), "{name}");
    for row in rows {
        let id = row["id"].as_str().unwrap();
        assert_eq!(row["outcome"], "result", "{name} {id}: {row}");
        let shell = match row["shell"].as_str().unwrap() {
            "powershell" => ShellName::PowerShell,
            "cmd" => ShellName::Cmd,
            other => panic!("{id}: shell {other}"),
        };
        let args: Vec<&str> = row["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let mut req =
            request(shell, row["executable"].as_str().unwrap(), &args).allow_failure(true);
        if let Some(input) = row["stdin_hex"].as_str() {
            req = req.stdin_bytes(unhex(input));
        }
        let execution = execute(req).unwrap_or_else(|e| panic!("{id}: {e}"));
        let result = &execution.result;
        println!(
            "EVIDENCE {name} {id}: raw_stdout={} raw_stderr={} stdout={} stderr={} status={}",
            hex(&execution.stdout),
            hex(&execution.stderr),
            hex(result.stdout()),
            hex(result.stderr()),
            result.exit_status()
        );
        assert_eq!(
            hex(result.stdout()),
            row["stdout_hex"].as_str().unwrap(),
            "{id} stdout"
        );
        assert_eq!(
            hex(result.stderr()),
            row["stderr_hex"].as_str().unwrap(),
            "{id} stderr"
        );
        assert_eq!(
            result.exit_status(),
            row["exit_status"].as_i64().unwrap(),
            "{id}"
        );
        assert_eq!(
            result.command_info.full_command,
            row["full_command"].as_str().unwrap(),
            "{id}"
        );
    }
}

/// The bytes a child receives on stdin and the text read back from it,
/// against the Ruby capture's `child_rows` (a Ruby child hex-dumps its
/// binary stdin). On Windows this checks that cmd passes stdin unchanged,
/// that the PowerShell stdin file is written in text mode (LF as CR LF), and
/// that CR LF output reads as LF. Needs `ruby` on PATH (the executor
/// reference workflow provides it).
#[test]
fn child_stdin_bytes_match_ruby() {
    let Some((name, golden)) = native_golden() else {
        eprintln!("UNVERIFIED executor_process: no Ruby capture for this host; not passed");
        return;
    };
    let ruby_runs = std::process::Command::new("ruby")
        .args(["-e", "exit 0"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ruby_runs {
        eprintln!("UNVERIFIED executor_process: ruby not on PATH; child stdin rows not passed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("ukiryu-stdin-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("ukiryu_stdin_probe.rb");
    std::fs::write(
        &script,
        "STDOUT.binmode\nSTDIN.binmode\nprint STDIN.read.unpack1('H*')\nprint 13.chr, 10.chr\n",
    )
    .unwrap();
    let script = script.to_string_lossy().replace('\\', "/");
    for row in golden["child_rows"].as_array().expect("child_rows") {
        let id = row["id"].as_str().unwrap();
        let shell = match row["shell"].as_str().unwrap() {
            "powershell" => ShellName::PowerShell,
            "cmd" => ShellName::Cmd,
            "bash" => ShellName::Bash,
            other => panic!("{id}: shell {other}"),
        };
        if shell == ShellName::PowerShell && !powershell_available() {
            continue;
        }
        let execution = execute(
            request(shell, "ruby", &[script.as_str()])
                .stdin_bytes(unhex(row["stdin_hex"].as_str().unwrap())),
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        let stdout = hex(execution.result.stdout());
        println!(
            "EVIDENCE {name} {id}: raw_stdout={} stdout={stdout}",
            hex(&execution.stdout)
        );
        assert_eq!(stdout, row["stdout_hex"].as_str().unwrap(), "{id}");
        assert_eq!(
            execution.result.exit_status(),
            row["exit_status"].as_i64().unwrap(),
            "{id}"
        );
    }
}

/// Windows: a child and a grandchild that inherits the output pipes are
/// both in the run's job object and both end at the deadline.
#[cfg(windows)]
#[test]
fn windows_timeout_terminates_the_process_tree() {
    fn pings() -> Vec<String> {
        let out = std::process::Command::new("powershell")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='PING.EXE'\" | \
                 Where-Object { $_.CommandLine -match ' -n 5[78] ' } | \
                 ForEach-Object { \"$($_.ProcessId):$($_.ParentProcessId)\" }",
            ])
            .output()
            .expect("powershell");
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    let runner = std::thread::spawn(|| {
        let start = Instant::now();
        let result = execute(ExecuteRequest::new(
            "cmd",
            strings(&["/c", "start /b ping -n 57 127.0.0.1 & ping -n 58 127.0.0.1"]),
            Environment::new(),
            ShellName::Cmd,
            Seconds::Int(8),
        ));
        (result, start.elapsed())
    });
    std::thread::sleep(Duration::from_secs(4));
    let during = pings();
    let (result, elapsed) = runner.join().expect("runner");
    let err = result.unwrap_err();
    assert!(matches!(err, ExecuteError::TimedOut(_)), "{err:?}");
    let mut after = pings();
    for _ in 0..20 {
        if after.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
        after = pings();
    }
    println!(
        "EVIDENCE windows tree timeout: during(pid:parent)={during:?} after={after:?} returned_after_ms={}",
        elapsed.as_millis()
    );
    assert_eq!(
        during.len(),
        2,
        "child and grandchild pings while running: {during:?}"
    );
    assert!(after.is_empty(), "survived the timeout: {after:?}");
    assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
}

#[test]
fn powershell_timeout_is_killed() {
    if !powershell_available() {
        return;
    }
    let (exe, args): (&str, &[&str]) = if cfg!(windows) {
        ("ping", &["-n", "30", "127.0.0.1"])
    } else {
        ("sleep", &["30"])
    };
    let start = Instant::now();
    let err = execute(ExecuteRequest::new(
        exe,
        strings(args),
        Environment::new(),
        ShellName::PowerShell,
        Seconds::Int(5),
    ))
    .unwrap_err();
    assert!(matches!(err, ExecuteError::TimedOut(_)), "{err:?}");
    assert!(
        start.elapsed() < Duration::from_secs(25),
        "{:?}",
        start.elapsed()
    );
}
