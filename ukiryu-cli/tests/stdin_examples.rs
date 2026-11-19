//! One end-to-end check per original cli_stdin_spec example (ukiryu ab6df353).
//! Linux is the qualified oracle host; jq is required rather than skipped.
//! Compare the source examples' semantic assertions, excluding run timestamps.
#![cfg(target_os = "linux")]

use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

fn golden() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("stdin_data/source_examples.json")).unwrap()
    })
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(id: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("ukiryu-cli-stdin-{id}-{}", std::process::id()));
        std::fs::create_dir(&path).expect("fresh scenario fixture");
        std::fs::write(
            path.join("input.json"),
            golden()["file_content"].as_str().unwrap(),
        )
        .unwrap();
        std::fs::write(
            path.join("direct.json"),
            golden()["direct_content"].as_str().unwrap(),
        )
        .unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cli() -> Command {
    assert!(Command::new("jq")
        .arg("--version")
        .output()
        .expect("jq is required for the original CLI examples")
        .status
        .success());
    let mut command = Command::new(env!("CARGO_BIN_EXE_ukiryu"));
    command
        .args(["exec", "jq", "process"])
        .env(
            "UKIRYU_REGISTER",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stdin_data/register"),
        )
        .env_remove("UKIRYU_SCHEMA_PATH")
        .env("SHELL", "/bin/bash")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn execute(row: &Value, fixture: &Fixture, flag: bool) -> Output {
    let mut command = cli();
    command.env("HOME", &fixture.0);
    for arg in row["args"].as_array().unwrap() {
        let arg = arg.as_str().unwrap();
        command.arg(if flag && arg == "stdin=-" {
            "--stdin".to_owned()
        } else {
            arg.replace("{dir}", fixture.0.to_str().unwrap())
        });
    }
    if row["kind"] == "pipeline" {
        // Actual process composition at both ends, not a buffered imitation.
        let mut producer = Command::new("printf")
            .args(["%s", row["stdin"].as_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        command.stdin(producer.stdout.take().unwrap());
        let mut child = command.spawn().unwrap();
        let consumer = Command::new("head")
            .arg("-10")
            .stdin(child.stdout.take().unwrap())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = consumer.wait_with_output().unwrap();
        let execution = child.wait_with_output().unwrap();
        assert!(producer.wait().unwrap().success());
        assert!(execution.status.success(), "{:?}", execution.stderr);
        return output;
    }
    let mut child = command.spawn().unwrap();
    if let Some(mut input) = child.stdin.take() {
        if let Some(data) = row["stdin"].as_str() {
            input.write_all(data.as_bytes()).unwrap();
        }
    }
    child.wait_with_output().unwrap()
}

fn project(row: &Value, output: &Output) -> Value {
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    let combined = format!("{stdout}{stderr}");
    let mut projection = match row["kind"].as_str().unwrap() {
        "json" => json!({"command_stdout": serde_json::from_str::<Value>(stdout)
            .unwrap_or_else(|e| panic!("{}: {e}: {stdout} {stderr}", row["id"]))["output"]["stdout"]}),
        "raw" => json!({"stdout": stdout, "stderr": stderr}),
        "missing" => json!({"file_not_found": combined.contains("File not found")}),
        "preview" => ["DRY RUN", "stdin:", "test", "value"]
            .into_iter()
            .map(|s| (s.to_owned(), json!(combined.contains(s))))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        "long_preview" => {
            json!({"ellipsis": stdout.contains("..."), "full_input": stdout.contains(row["stdin"].as_str().unwrap())})
        }
        "yaml" => {
            json!({"stdout_label": stdout.contains("stdout:"), "value": stdout.contains("value")})
        }
        "pipeline" => json!({"array": stdout.contains('[')}),
        "error" => {
            json!({"stderr_nonempty": !stderr.is_empty(), "error": combined.contains("error"), "syntax_error": combined.contains("syntax error")})
        }
        other => panic!("unmapped projection {other}"),
    };
    projection["exit_status"] = json!(output.status.code().unwrap());
    projection
}

fn check(id: &str) {
    let rows = golden()["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 15);
    let row = rows
        .iter()
        .find(|r| r["id"] == id)
        .expect("captured scenario");
    let fixture = Fixture::new(id);
    let observed = project(row, &execute(row, &fixture, false));
    assert_eq!(observed, row["expected"], "{id}");
    if id == "flag_equivalence" {
        assert_eq!(observed, project(row, &execute(row, &fixture, true)));
    }
}

macro_rules! examples {
    ($($id:ident),+ $(,)?) => { $(#[test] fn $id() { check(stringify!($id)); })+ };
}
examples!(
    flag_pipe,
    multiline,
    dash_pipe,
    flag_equivalence,
    file,
    missing_file,
    direct_data,
    preview,
    long_preview,
    json,
    yaml,
    pipeline,
    curl_output,
    raw,
    raw_error
);
