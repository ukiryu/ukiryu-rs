//! Real-process extraction through [`ProcessHost`].
//!
//! Fixture rows run the shell-script tools in `native_extractor_data/bin`
//! and compare against `native_extractor_data/oracle.json`, captured by
//! running the pinned Ruby source (ukiryu/ukiryu ab6df353,
//! lib/ukiryu/extractors/*.rb) on the same scripts with real processes; only
//! `Shell.detect` was pinned (bash, or cmd for the ENOENT rows). See
//! `provenance` in the data file.
//!
//! Probe rows run tools found on the runner's PATH. They have no Ruby
//! capture (help text differs per runner); they check the extraction
//! contract. A tool that is not on PATH is reported UNAVAILABLE and is not
//! counted as a pass.
//!
//! The fixtures are POSIX `sh` scripts, so this file runs on Unix only.
#![cfg(unix)]

use std::path::PathBuf;

use serde_json::{json, Value as Json};
use ukiryu::environment::Environment;
use ukiryu::extractors::base::ProcessHost;
use ukiryu::extractors::help::{
    self, CommandHost, CommandOutput, ExtractionMethod, ExtractionResult, HostError,
    EXTRACTION_TIMEOUT_SECS,
};
use ukiryu::runtime::ShellName;
use ukiryu::version_detection::{CommandRequest, CommandRunner};
use yeptris::Value;

fn oracle() -> Json {
    serde_json::from_str(include_str!("native_extractor_data/oracle.json")).unwrap()
}

fn fixture_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/native_extractor_data/bin")
}

/// The process environment with the fixture tools first on PATH, as the
/// capture ran (`Environment.from_env` after prepending the fixture dir).
fn fixture_environment() -> Environment {
    let bin = fixture_bin();
    Environment::from_process()
        .expect("process environment is UTF-8")
        .prepend_path(&[bin.to_str().unwrap()])
}

fn host(shell: ShellName) -> ProcessHost {
    ProcessHost::new(fixture_environment(), shell)
}

/// Records each call's argv and timeout, then runs it for real.
struct Recording {
    inner: ProcessHost,
    calls: Vec<Json>,
}

impl CommandHost for Recording {
    fn execute(
        &mut self,
        executable: &str,
        args: &[&str],
        timeout_secs: u64,
    ) -> Result<CommandOutput, HostError> {
        let argv: Vec<&str> = std::iter::once(executable)
            .chain(args.iter().copied())
            .collect();
        self.calls
            .push(json!({"argv": argv, "timeout": timeout_secs}));
        self.inner.execute(executable, args, timeout_secs)
    }
}

/// The ordered, tagged view the capture script writes for Ruby values.
fn ordered(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::String(s) => json!(s),
        Value::Sequence(items) => Json::Array(items.iter().map(ordered).collect()),
        Value::Mapping(map) => {
            json!({"map": map.0.iter().map(|(k, v)| json!([ordered(k), ordered(v)])).collect::<Vec<_>>()})
        }
        other => panic!("unexpected YAML value {other:?}"),
    }
}

fn method(label: &str) -> Option<ExtractionMethod> {
    match label {
        "auto" => None,
        "native" => Some(ExtractionMethod::Native),
        "help" => Some(ExtractionMethod::Help),
        other => Some(ExtractionMethod::Other(other.to_owned())),
    }
}

/// Our result in the capture's shape. Help YAML is compared as data: our
/// emitter is yeptris, not Psych, so only the content must agree.
fn result_view(result: &ExtractionResult) -> Json {
    let yaml = match (&result.yaml, result.method) {
        (None, _) => Json::Null,
        (Some(text), Some(help::ResultMethod::Help)) => {
            let parsed: Value = yeptris::from_str(text)
                .unwrap_or_else(|e| panic!("our help YAML does not parse: {e}\n{text}"));
            json!({"parsed": ordered(&parsed)})
        }
        (Some(text), _) => json!({"raw": text}),
    };
    json!({
        "success": result.success,
        "method": result.method.map(|m| m.as_str()),
        "error": result.error,
        "yaml": yaml,
    })
}

fn run_flow(flow: &Json, shell: ShellName) {
    let id = flow["id"].as_str().unwrap();
    let mut recording = Recording {
        inner: host(shell),
        calls: Vec::new(),
    };
    let outcome = help::extract(
        flow["tool"].as_str().unwrap(),
        method(flow["method"].as_str().unwrap()),
        &mut recording,
    );
    let ours = match outcome {
        Ok(result) => json!({"ok": result_view(&result)}),
        Err(e) => json!({"error": {"class": e.class_name(), "message": e.message()}}),
    };
    assert_eq!(ours, flow["result"], "{id}: result");
    assert_eq!(
        Json::Array(recording.calls),
        flow["calls"],
        "{id}: commands run"
    );
}

#[test]
fn extraction_flows_match_the_source() {
    let data = oracle();
    let flows = data["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 32);
    for flow in flows {
        run_flow(flow, ShellName::Bash);
    }
}

/// The process cannot be started (no `cmd` on a Unix host): the host
/// reports `NotFound` and the extractors use the source's 127 result.
#[test]
fn unstartable_process_follows_the_enoent_policy() {
    let data = oracle();
    for flow in data["enoent_flows"].as_array().unwrap() {
        run_flow(flow, ShellName::Cmd);
    }
    let mut cmd = host(ShellName::Cmd);
    assert_eq!(
        cmd.execute("ukx-help", &["--help"], EXTRACTION_TIMEOUT_SECS),
        Err(HostError::NotFound)
    );
    let expected = &data["enoent_command"]["ok"];
    let fallback = CommandOutput::command_not_found();
    assert_eq!(
        json!({
            "stdout": String::from_utf8(fallback.stdout).unwrap(),
            "stderr": String::from_utf8(fallback.stderr).unwrap(),
            "exit_status": fallback.exit_status,
        }),
        *expected
    );
}

#[test]
fn single_commands_match_the_source() {
    let data = oracle();
    let mut bash = host(ShellName::Bash);
    for command in data["commands"].as_array().unwrap() {
        let id = command["id"].as_str().unwrap();
        let argv: Vec<&str> = command["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let timeout = command["timeout"].as_u64().unwrap();
        let ours = match bash.execute(argv[0], &argv[1..], timeout) {
            Ok(out) => json!({"ok": {
                "stdout": String::from_utf8(out.stdout).unwrap(),
                "stderr": String::from_utf8(out.stderr).unwrap(),
                "exit_status": out.exit_status,
            }}),
            Err(HostError::Raised { class, message }) => {
                json!({"error": {"class": class, "message": message}})
            }
            Err(HostError::NotFound) => panic!("{id}: unexpected NotFound"),
        };
        if id == "missing" {
            // The shell's own message names its path and differs between
            // bash versions; the source keeps it unchanged too.
            assert_eq!(ours["ok"]["exit_status"], 127, "{id}");
            assert_eq!(ours["ok"]["stdout"], "", "{id}");
            let stderr = ours["ok"]["stderr"].as_str().unwrap();
            assert!(
                stderr.contains("ukx-missing") && stderr.contains("not found"),
                "{id}: {stderr}"
            );
        } else {
            assert_eq!(ours, command["result"], "{id}");
        }
    }
}

/// Version detection runs through the same host (target contract; no Ruby
/// capture for the runner trait itself).
#[test]
fn command_runner_runs_detection_commands() {
    let mut bash = host(ShellName::Bash);
    let request = |executable: &str, args: &[&str], shell: Option<&str>| CommandRequest {
        executable: executable.to_owned(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        shell: shell.map(str::to_owned),
        timeout_secs: 30,
        allow_failure: true,
    };
    let out = bash
        .run(&request("ukx-help", &["--version"], None))
        .unwrap();
    assert_eq!(out.stdout, b"ukx-help 2.3.4 (fixture)\n");
    assert_eq!(out.exit_status, 0);

    let out = bash
        .run(&request("ukx-help-fails", &["--nope"], Some("bash")))
        .unwrap();
    assert_eq!(out.exit_status, 1);
    assert_eq!(out.stderr, b"unknown option: --nope\n");

    let err = bash
        .run(&request("ukx-help", &["--version"], Some("nosuchshell")))
        .unwrap_err();
    assert_eq!(err.message, "Unknown shell: nosuchshell");
}

/// Real tools on the runner. git is on every GitHub-hosted runner; with
/// `UKIRYU_REQUIRE_SHELL_TESTS=1` its absence fails.
#[test]
fn real_tool_probes() {
    let required = std::env::var("UKIRYU_REQUIRE_SHELL_TESTS").as_deref() == Ok("1");
    let path = std::env::var_os("PATH");
    let mut verified = Vec::new();
    let mut unavailable = Vec::new();
    for tool in ["git", "tar", "curl", "grep", "ls", "gzip"] {
        if ukiryu::executor::find_executable_in(tool, path.as_deref(), None, false).is_none() {
            eprintln!("UNAVAILABLE {tool}: not on PATH");
            unavailable.push(tool);
            continue;
        }
        let mut real = ProcessHost::new(Environment::new(), ShellName::Bash);
        let result = match help::extract(tool, None, &mut real) {
            Ok(result) => result,
            // The source raises for help text that is not valid UTF-8; that
            // is its behaviour, not a probe failure, except for git.
            Err(e) => {
                assert_ne!(tool, "git", "git: extraction raised {e}");
                eprintln!("PROBE {tool}: raised {e}");
                verified.push(tool);
                continue;
            }
        };
        eprintln!(
            "PROBE {tool}: success={} method={:?} error={:?}",
            result.success,
            result.method.map(|m| m.as_str()),
            result.error
        );
        if result.success {
            let yaml = result.yaml.as_deref().unwrap();
            let parsed: Value = yeptris::from_str(yaml)
                .unwrap_or_else(|e| panic!("{tool}: YAML does not parse: {e}"));
            let view = ordered(&parsed);
            let name = view["map"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p[0] == "name")
                .map(|p| p[1].clone());
            assert_eq!(name, Some(json!(tool)), "{tool}: definition name");
        } else {
            assert!(result.yaml.is_none(), "{tool}: failed result has YAML");
            assert!(result.error.is_some(), "{tool}: failed result has no error");
        }
        if tool == "git" {
            assert!(result.success, "git: {:?}", result.error);
            assert_eq!(result.method, Some(help::ResultMethod::Help));
        }
        verified.push(tool);
    }
    eprintln!("probes verified={verified:?} unavailable={unavailable:?}");
    if required {
        assert!(
            verified.contains(&"git"),
            "git is not on PATH but UKIRYU_REQUIRE_SHELL_TESTS=1"
        );
    }
}
