//! Executor rows that depend on the parent process environment: inherited
//! `DISPLAY` and the local time zone of result timestamps.
//!
//! This binary holds a single test, so changing the process environment
//! cannot race with another test thread. Golden data:
//! tests/executor_data/observations.json (rows `display_*` and the
//! `time_zone` list), captured from the pinned Ruby executor
//! (ukiryu/ukiryu@ab6df353) with Ruby 4.0.1 on Linux.

use serde_json::Value;
use ukiryu::environment::Environment;
use ukiryu::execution::Seconds;
use ukiryu::executor::{execute, native_utc_offset, ExecuteRequest};
use ukiryu::runtime::{Clock, ShellName};
use ukiryu::types::ruby::time_to_s;

fn golden() -> Value {
    serde_json::from_str(include_str!("executor_data/observations.json")).expect("golden json")
}

fn find<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array()
        .expect("list")
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no golden row {id}"))
}

fn echo(executable: &str, args: &[&str], env: Environment) -> ExecuteRequest {
    ExecuteRequest::new(
        executable,
        args.iter().map(|a| a.to_string()).collect(),
        env,
        ShellName::Bash,
        Seconds::Int(30),
    )
}

#[test]
fn host_environment_rows() {
    let golden = golden();
    if cfg!(unix) {
        display_rows(&golden);
        time_zone_rows(&golden);
    } else {
        windows_offset();
    }
    explicit_clock_offset();
}

/// `DISPLAY` is dropped from the request and inherited from the parent.
fn display_rows(golden: &Value) {
    let rows = &golden["rows"];
    let pairs = |p: &[(&str, &str)]| Environment::from_pairs(p.iter().copied());
    let cases: [(&str, Option<&str>, Environment); 6] = [
        ("display_parent_set", Some(":99"), Environment::new()),
        ("display_user_set", Some(":99"), pairs(&[("DISPLAY", ":5")])),
        (
            "display_empty_explicit_parent_set",
            Some(":99"),
            pairs(&[("DISPLAY", "")]),
        ),
        (
            "display_env_object_parent_set",
            Some(":99"),
            pairs(&[("DISPLAY", ":7")]),
        ),
        ("display_absent_parent_unset", None, Environment::new()),
        (
            "display_user_set_parent_unset",
            None,
            pairs(&[("DISPLAY", ":5")]),
        ),
    ];
    for (id, parent, env) in cases {
        match parent {
            Some(value) => std::env::set_var("DISPLAY", value),
            None => std::env::remove_var("DISPLAY"),
        }
        let row = find(rows, id);
        assert_eq!(row["contract"], "parity", "{id}");
        let execution = execute(echo("sh", &["-c", "echo \"${DISPLAY-unset}\""], env))
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(
            execution.result.stdout(),
            row["stdout"].as_str().unwrap().as_bytes(),
            "{id}"
        );
        assert_eq!(
            execution.result.command_info.full_command,
            row["full_command"].as_str().unwrap(),
            "{id}"
        );
        println!(
            "EVIDENCE display {id}: parent={parent:?} stdout={:?}",
            std::str::from_utf8(execution.result.stdout())
        );
    }
    std::env::remove_var("DISPLAY");
}

/// Timestamps carry the local offset of `TZ`, as Ruby's `Time.now` does.
fn time_zone_rows(golden: &Value) {
    let saved = std::env::var_os("TZ");
    for id in ["tz_karachi", "tz_utc", "tz_st_johns"] {
        let row = find(&golden["time_zone"], id);
        std::env::set_var("TZ", row["tz"].as_str().unwrap());
        let execution = execute(echo("echo", &["x"], Environment::new())).expect("echo");
        let started = execution.result.started_at().unwrap().clone();
        let finished = execution.result.finished_at().unwrap().clone();
        // The source rows were captured in November; America/St_Johns
        // observes daylight saving time, so its offset is compared with the
        // host rules for the same instant rather than with the capture.
        let expected = if id == "tz_st_johns" {
            native_utc_offset(started.seconds).expect("offset")
        } else {
            i32::try_from(row["utc_offset"].as_i64().unwrap()).unwrap()
        };
        assert_eq!(started.utc_offset, expected, "{id}");
        assert_eq!(finished.utc_offset, expected, "{id}");
        assert_eq!(started.utc, row["utc"].as_bool().unwrap(), "{id}");
        assert!(started.nanos < 1_000_000_000);
        let text = time_to_s(&started);
        let shape: String = text
            .chars()
            .map(|c| if c.is_ascii_digit() { 'D' } else { c })
            .collect();
        let row_shape = row["to_s_shape"].as_str().unwrap();
        // Same layout; the sign depends on the offset.
        assert_eq!(shape.replace('-', "+"), row_shape.replace('-', "+"), "{id}");
        if id != "tz_st_johns" {
            assert!(text.ends_with(row["to_s_zone"].as_str().unwrap()), "{id}");
            let hash = execution.result.to_h().expect("valid UTF-8 streams");
            let started_at = hash.metadata.started_at.expect("started_at");
            assert!(
                started_at.ends_with(row["to_h_started_at_zone"].as_str().unwrap()),
                "{id}: {started_at}"
            );
        }
        println!(
            "EVIDENCE time zone {id}: TZ={} started_at={text} utc_offset={} nanos={}",
            row["tz"].as_str().unwrap(),
            started.utc_offset,
            started.nanos
        );
    }
    match saved {
        Some(value) => std::env::set_var("TZ", value),
        None => std::env::remove_var("TZ"),
    }
}

/// On Windows the offset comes from the system time zone; compare it with
/// what .NET reports for the same moment.
fn windows_offset() {
    let execution = execute(ExecuteRequest::new(
        "echo",
        vec!["x".to_string()],
        Environment::new(),
        ShellName::Cmd,
        Seconds::Int(60),
    ))
    .expect("cmd echo");
    let started = execution.result.started_at().unwrap().clone();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "[int][System.TimeZoneInfo]::Local.GetUtcOffset([DateTime]::UtcNow).TotalSeconds",
        ])
        .output()
        .expect("powershell");
    let reported: i32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("offset seconds");
    assert_eq!(started.utc_offset, reported);
    assert!(!started.utc);
    println!(
        "EVIDENCE time zone windows: started_at={} utc_offset={} reported={reported}",
        time_to_s(&started),
        started.utc_offset
    );
}

/// An explicit clock offset replaces the host offset.
fn explicit_clock_offset() {
    let (executable, args, shell): (&str, &[&str], ShellName) = if cfg!(windows) {
        ("echo", &["x"], ShellName::Cmd)
    } else {
        ("echo", &["x"], ShellName::Bash)
    };
    let request = ExecuteRequest::new(
        executable,
        args.iter().map(|a| a.to_string()).collect(),
        Environment::new(),
        shell,
        Seconds::Int(60),
    )
    .clock(Clock::System {
        utc_offset_secs: -12_600,
    });
    let execution = execute(request).expect("echo");
    let started = execution.result.started_at().unwrap().clone();
    assert_eq!(started.utc_offset, -12_600);
    let text = time_to_s(&started);
    assert!(text.ends_with(" -0330"), "{text}");
    let hash = execution.result.to_h().expect("valid UTF-8 streams");
    assert!(hash.metadata.started_at.unwrap().ends_with(" -0330"));
    println!("EVIDENCE explicit clock: started_at={text}");
}
