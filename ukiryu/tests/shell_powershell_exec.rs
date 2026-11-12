//! Real-shell execution of the PowerShell formatting.
//!
//! Every `echo` case captured from the pinned Ruby oracle (fixture
//! `src/shell/powershell/oracle_golden.json`) is run through a real
//! PowerShell twice: once as the executor's `-Command` argv
//! (`PowerShell::command_argv`) and once as the `join` command line. Before
//! running, the Rust bytes are compared with the oracle bytes. After running,
//! the text PowerShell prints is compared with the original arguments.
//!
//! Some arguments are left unquoted by the source rules even though
//! PowerShell parses them (for example `it's`, `a;b` on Windows, `#hash` in
//! `join`). The port keeps those quirks. Such cases are run and their outcome
//! printed as `UNASSERTED`, but they are not asserted. Cases where every
//! argument is either quoted or made only of plain characters are asserted
//! exactly.
//!
//! Shells are found by PATH: `pwsh` everywhere, and also `powershell` (what
//! the source runs) on Windows. With `UKIRYU_REQUIRE_SHELL_TESTS=1`, a missing
//! required shell fails the test. Without it, a missing shell prints
//! `UNVERIFIED` and the test does nothing. On Linux `pwsh` is never required.
//! Run with `--nocapture` to see the per-case evidence lines.

use serde_json::Value;
use std::process::Command;
use ukiryu::shell::powershell::{InvocationHost, PowerShell};
use ukiryu::shell::ShellFormatting;

/// Harness-only prefix on Windows so that stdout is UTF-8 regardless of the
/// console code page. The oracle bytes are compared before it is added.
const WINDOWS_UTF8_PREFIX: &str = "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; ";

fn golden() -> Value {
    serde_json::from_str(include_str!("../src/shell/powershell/oracle_golden.json"))
        .expect("fixture is valid JSON")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|e| e.as_str().expect("string").to_owned())
        .collect()
}

fn host() -> InvocationHost {
    if cfg!(windows) {
        InvocationHost::Windows
    } else {
        InvocationHost::Unix
    }
}

fn runnable(program: &str) -> bool {
    Command::new(program)
        .args(["-NoLogo", "-NoProfile", "-Command", "exit 0"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The PowerShell programs to run, honouring `UKIRYU_REQUIRE_SHELL_TESTS`.
fn programs() -> Vec<&'static str> {
    let required = std::env::var("UKIRYU_REQUIRE_SHELL_TESTS").as_deref() == Ok("1");
    let candidates: &[&'static str] = if cfg!(windows) {
        &["powershell", "pwsh"]
    } else {
        &["pwsh"]
    };
    let mut found = Vec::new();
    for &program in candidates {
        if runnable(program) {
            found.push(program);
        } else {
            assert!(
                !(required && cfg!(windows)),
                "UKIRYU_REQUIRE_SHELL_TESTS=1 but `{program}` is not runnable"
            );
            eprintln!("UNVERIFIED shell_powershell_exec: `{program}` not found on PATH");
        }
    }
    found
}

/// Smart quotes act as quote characters in PowerShell, and the source does
/// not escape them.
fn has_smart_quote(value: &str) -> bool {
    value.chars().any(|c| {
        matches!(
            c,
            '\u{2018}'
                | '\u{2019}'
                | '\u{201a}'
                | '\u{201b}'
                | '\u{201c}'
                | '\u{201d}'
                | '\u{201e}'
        )
    })
}

/// An unquoted argument that PowerShell passes through as one literal token.
fn plain_unquoted(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['-', '\u{2013}', '\u{2014}', '\u{2015}'])
        && value.chars().all(|c| {
            (c.is_alphanumeric() && !c.is_whitespace())
                || matches!(
                    c,
                    '.' | '_' | '/' | '\\' | ':' | '=' | '+' | '%' | '~' | '!' | '^'
                )
                || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
        })
        && !has_smart_quote(value)
}

/// Whether the Ruby executor single-quotes `value` on `host`.
fn executor_quotes(value: &str, host: InvocationHost) -> bool {
    match host {
        InvocationHost::Windows => {
            value.starts_with('-')
                || value.contains('$')
                || value.contains('`')
                || value.contains(' ')
                || value.contains('"')
        }
        InvocationHost::Unix => {
            value.contains(' ')
                || value.starts_with('-')
                || value.contains('$')
                || value.contains('`')
                || value.contains(';')
        }
    }
}

struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(program: &str, script: &str, envs: &[(&str, &str)]) -> Outcome {
    let script = if cfg!(windows) {
        format!("{WINDOWS_UTF8_PREFIX}{script}")
    } else {
        script.to_owned()
    };
    let output = Command::new(program)
        .args(["-NoLogo", "-Command", &script])
        .envs(envs.iter().copied())
        .output()
        .expect("spawn PowerShell");
    Outcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Write-Output prints each argument on its own line.
fn expected_stdout(args: &[String]) -> String {
    args.iter().map(|a| format!("{a}\n")).collect()
}

/// `echo` (Write-Output) with no arguments prompts for its mandatory
/// InputObject parameter, so its output says nothing about the formatting.
/// The bytes of these cases are still compared with the oracle.
fn not_run_without_arguments(label: &str, args: &[String]) -> bool {
    if args.is_empty() {
        eprintln!(
            "NOT-RUN {label} input=[]: Write-Output with no arguments prompts for InputObject"
        );
    }
    args.is_empty()
}

#[derive(Default)]
struct Tally {
    asserted: usize,
    unasserted: usize,
    failures: Vec<String>,
}

impl Tally {
    fn record(&mut self, label: &str, assert: bool, args: &[String], out: &Outcome) {
        let want = expected_stdout(args);
        let matched = out.code == Some(0) && out.stdout == want;
        let status = match (assert, matched) {
            (true, true) => "PASS",
            (true, false) => "FAIL",
            (false, true) => "UNASSERTED-MATCH",
            (false, false) => "UNASSERTED-DIFFERS",
        };
        eprintln!(
            "{status} {label} input={args:?} exit={:?} stdout={:?} stderr={:?}",
            out.code, out.stdout, out.stderr
        );
        if assert {
            self.asserted += 1;
            if !matched {
                self.failures.push(format!(
                    "{label} input={args:?}: got {:?}, want {want:?}",
                    out.stdout
                ));
            }
        } else {
            self.unasserted += 1;
        }
    }
}

#[test]
fn echo_cases_run_through_real_powershell() {
    let programs = programs();
    if programs.is_empty() {
        return;
    }
    let g = golden();
    let sh = PowerShell;
    let host = host();
    let windows = host == InvocationHost::Windows;
    let mut tally = Tally::default();

    for r in g["invocations"].as_array().expect("invocations") {
        if r["executable"] != "echo" || r["windows"] != windows || !r["stdin_path"].is_null() {
            continue;
        }
        let args = strings(&r["args"]);
        let argv = sh.command_argv("echo", &args, host);
        assert_eq!(argv, strings(&r["argv"]), "oracle bytes for {args:?}");
        if not_run_without_arguments("command_argv", &args) {
            continue;
        }
        let assert = args
            .iter()
            .all(|a| !has_smart_quote(a) && (executor_quotes(a, host) || plain_unquoted(a)));
        for program in &programs {
            let out = run(program, &argv[3], &[]);
            tally.record(&format!("command_argv[{program}]"), assert, &args, &out);
        }
    }

    for r in g["joins"].as_array().expect("joins") {
        if r["executable"] != "echo" {
            continue;
        }
        let args = strings(&r["args"]);
        let line = sh.join("echo", &args);
        assert_eq!(
            line,
            r["out"].as_str().expect("out"),
            "oracle bytes for {args:?}"
        );
        if not_run_without_arguments("join", &args) {
            continue;
        }
        let assert = !args.iter().any(|a| a == "-Command" || a == "-File")
            && args
                .iter()
                .all(|a| !has_smart_quote(a) && (sh.needs_quoting(a) || plain_unquoted(a)));
        for program in &programs {
            let out = run(program, &line, &[]);
            tally.record(&format!("join[{program}]"), assert, &args, &out);
        }
    }

    // env_var: the reference must expand to the child environment value.
    let reference = sh.env_var("MY_VAR_1");
    let oracle = g["strings"]
        .as_array()
        .expect("strings")
        .iter()
        .find(|r| r["input"] == "MY_VAR_1")
        .expect("MY_VAR_1 captured");
    assert_eq!(reference, oracle["env_var"].as_str().expect("env_var"));
    let value = "a b 'q' \"d\" $x `t` é 日本";
    for program in &programs {
        let out = run(
            program,
            &format!("echo {reference}"),
            &[("MY_VAR_1", value)],
        );
        tally.record(
            &format!("env_var[{program}]"),
            true,
            &[value.to_owned()],
            &out,
        );
    }

    eprintln!(
        "shell_powershell_exec: programs={programs:?} asserted={} unasserted={} failures={}",
        tally.asserted,
        tally.unasserted,
        tally.failures.len()
    );
    assert!(
        tally.asserted >= 20,
        "too few asserted cases: {}",
        tally.asserted
    );
    assert!(
        tally.failures.is_empty(),
        "{} failures:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
}
