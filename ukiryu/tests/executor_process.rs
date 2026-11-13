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
        assert_eq!(execution.result.stdout(), value, "{shell}");
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
        assert_eq!(execution.result.stdout(), "value-\u{e9} 1\n", "{shell}");

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
    assert_eq!(execution.result.stdout().trim(), "hello");

    let execution =
        execute(request(ShellName::Cmd, "more", &[]).stdin_bytes("test data")).expect("cmd stdin");
    assert_eq!(execution.result.stdout().trim(), "test data");

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

#[test]
fn powershell_status_output_and_stdin() {
    if !powershell_available() {
        return;
    }
    let (exit_exe, exit_args): (&str, &[&str]) = if cfg!(windows) {
        ("cmd", &["/c", "exit", "42"])
    } else {
        ("sh", &["-c", "exit 42"])
    };
    let execution =
        execute(request(ShellName::PowerShell, exit_exe, exit_args).allow_failure(true))
            .expect("powershell exit");
    assert_eq!(execution.result.exit_status(), 42);

    let (echo_exe, echo_args): (&str, &[&str]) = if cfg!(windows) {
        ("cmd", &["/c", "echo", "hello"])
    } else {
        ("echo", &["hello"])
    };
    let execution =
        execute(request(ShellName::PowerShell, echo_exe, echo_args)).expect("powershell echo");
    assert_eq!(execution.result.stdout().trim(), "hello");

    let cat = if cfg!(windows) { "more" } else { "cat" };
    let execution = execute(request(ShellName::PowerShell, cat, &[]).stdin_bytes("test data"))
        .expect("powershell stdin");
    assert_eq!(execution.result.stdout().trim(), "test data");
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
