//! Runs real tools end to end through `Tool::execute`: definition, profile
//! choice, PATH discovery, argument building, shell execution and exit-code
//! handling. Mirrors the pinned Ruby `spec/ukiryu/tool_stdin_spec.rb`
//! (jq with stdin) and a `git` subcommand.
//!
//! GitHub-hosted runners provide `jq` and `git`. On CI (`CI` set) a missing
//! tool fails the test; elsewhere it is reported as unavailable and the test
//! returns without claiming a pass for that tool.
use ukiryu::execution::Seconds;
use ukiryu::executor::ExecuteError;
use ukiryu::models::{Model, ToolDefinition};
use ukiryu::runtime::{native_platform, ShellName};
use ukiryu::tool::{ExecuteSettings, Tool, ToolError, ToolOptions};
use ukiryu::types::Value;

const JQ: &str = r#"
name: jq
version: '1.0'
profiles:
- name: default
  platforms: [macos, linux, windows]
  shells: [bash, zsh, sh, fish, powershell, cmd]
  executable_name: jq
  commands:
  - name: process
    arguments:
    - name: filter
      type: string
      required: true
      position: first
    - name: inputs
      type: file
      position: last
      variadic: true
    exit_codes:
      standard:
        '0': success
      custom:
        '3': compile_error
"#;

const GIT: &str = r#"
name: git
version: '2.0'
profiles:
- name: default
  executable_name: git
  commands:
  - name: version
    subcommand: version
"#;

fn shell() -> ShellName {
    if cfg!(windows) {
        ShellName::Cmd
    } else {
        ShellName::Bash
    }
}

fn tool(yaml: &str) -> Option<Tool> {
    let platform = native_platform().expect("supported host");
    let tool = Tool::new(
        ToolDefinition::from_yaml(yaml).expect("definition"),
        &ToolOptions::new(platform, shell()),
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

fn params(pairs: &[(&str, &str)]) -> Value {
    Value::Hash(
        pairs
            .iter()
            .map(|(k, v)| (Value::sym(*k), Value::str(*v)))
            .collect(),
    )
}

fn settings() -> ExecuteSettings {
    ExecuteSettings::new(Seconds::Int(90))
}

#[test]
fn jq_reads_stdin_and_filters() {
    let Some(jq) = tool(JQ) else { return };
    let run = jq
        .execute(
            "process",
            &params(&[
                ("stdin", "{\"foo\": \"bar\", \"baz\": \"qux\"}"),
                ("filter", ".foo"),
            ]),
            &settings(),
        )
        .expect("jq runs");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "\"bar\"");
    assert_eq!(run.result.status(), 0);
}

#[test]
fn jq_stdin_is_not_an_argument() {
    let Some(jq) = tool(JQ) else { return };
    let built = jq
        .build("process", &params(&[("stdin", "{}"), ("filter", ".")]))
        .unwrap();
    assert_eq!(built.invocation.args, vec!["."]);
    assert_eq!(built.stdin.as_deref(), Some(&b"{}"[..]));
}

#[test]
fn jq_failure_maps_through_the_exit_code_table() {
    let Some(jq) = tool(JQ) else { return };
    let err = jq
        .execute(
            "process",
            &params(&[("stdin", "{}"), ("filter", ".[")]),
            &settings(),
        )
        .expect_err("invalid filter fails");
    let execution = match err {
        ToolError::Execute(e) => match *e {
            ExecuteError::Failed { execution, .. } => execution,
            other => panic!("expected a failed execution, got {other:?}"),
        },
        other => panic!("expected a failed execution, got {other:?}"),
    };
    let status = execution.result.status();
    assert_ne!(status, 0);
    if status == 3 {
        assert_eq!(
            jq.exit_code_meaning("process", status).as_deref(),
            Some("compile_error")
        );
    }
    assert_eq!(
        jq.exit_code_meaning("process", 0).as_deref(),
        Some("success")
    );
    assert_eq!(jq.exit_code_meaning("process", 42), None);
}

#[test]
fn git_runs_its_subcommand() {
    let Some(git) = tool(GIT) else { return };
    let run = git
        .execute("version", &Value::Hash(Vec::new()), &settings())
        .expect("git version runs");
    assert!(
        String::from_utf8_lossy(&run.stdout).starts_with("git version "),
        "{:?}",
        String::from_utf8_lossy(&run.stdout)
    );
}

#[test]
fn unknown_command_is_an_argument_error() {
    let Some(git) = tool(GIT) else { return };
    let err = git
        .execute("nope", &Value::Hash(Vec::new()), &settings())
        .unwrap_err();
    assert_eq!(err.to_string(), "Unknown command: nope");
}
