//! `execute` through the wire runs real processes (Unix hosts, bash): the
//! result payload carries the exact stream bytes, exact times and the
//! original's failure classes. `jq`, `printf`, `cat` and `false` are on
//! every GitHub-hosted Unix runner; elsewhere a missing tool is reported as
//! unavailable and the test returns without claiming a pass.
#![cfg(unix)]
use serde_json::{json, Value};

fn wire(request: &Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn definition(name: &str, command: &str) -> String {
    format!(
        "name: '{name}'\nversion: '1.0'\nprofiles:\n- name: default\n  platforms: [linux, macos]\n  shells: [bash]\n  executable_name: '{name}'\n  commands:\n{command}"
    )
}

fn tool(yaml: &str) -> Value {
    json!({"source": {"kind": "definition", "yaml": yaml}, "platform": platform(), "shell": "bash"})
}

/// The tool, when the host has it (always on CI).
fn available(yaml: &str) -> Option<Value> {
    let tool = tool(yaml);
    let reply = wire(
        &json!({"op": "resolve", "method": "tool", "source": tool["source"],
        "platform": tool["platform"], "shell": tool["shell"]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    if reply["value"]["available"] == true {
        return Some(tool);
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "expected on CI: {}",
        reply["value"]["unavailability_reason"]
    );
    eprintln!(
        "UNAVAILABLE (not a pass): {}",
        reply["value"]["unavailability_reason"]
    );
    None
}

fn execute(tool: &Value, command: &str, params: Value, stdin: Option<Value>) -> Value {
    let mut request = json!({
        "op": "execute", "tool": tool, "command": command, "params": params,
        "env": {}, "timeout": 60,
    });
    if let Some(stdin) = stdin {
        request["stdin"] = stdin;
    }
    wire(&request)
}

fn seconds(time: &Value) -> i128 {
    let t = &time["$time"];
    match t {
        Value::Object(_) => {
            let s: i128 = t["seconds"].as_str().unwrap().parse().unwrap();
            s * 1_000_000_000 + t["nanos"].as_i64().unwrap() as i128
        }
        other => panic!("time without an offset object: {other}"),
    }
}

const JQ: &str = "  - name: process\n    arguments:\n    - name: filter\n      type: string\n      position: first\n";

#[test]
fn jq_reads_stdin_and_reports_an_exact_result() {
    let Some(jq) = available(&definition("jq", JQ)) else {
        return;
    };
    let reply = execute(
        &jq,
        "process",
        json!({"filter": ".foo", "stdin": "{\"foo\": \"bar\"}"}),
        None,
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let result = &reply["value"];
    assert_eq!(result["stdout"], "\"bar\"\n");
    assert_eq!(result["stderr"], "");
    assert_eq!(result["status"], 0);
    assert_eq!(result["success"], true);
    assert_eq!(result["command"]["arguments"], json!([".foo"]));
    assert_eq!(result["command"]["tool_name"], "jq");
    assert_eq!(result["command"]["command_name"], "process");
    assert_eq!(result["timeout"], 60);
    assert_eq!(result["timed_out"], false);
    let (start, finish) = (
        seconds(&result["started_at"]),
        seconds(&result["finished_at"]),
    );
    assert!(finish >= start, "{result}");
    assert!(result["duration"]["$float"].is_string(), "{result}");
}

#[test]
fn stream_bytes_that_are_not_utf8_cross_unchanged() {
    let printf = "  - name: emit\n    arguments:\n    - name: format\n      type: string\n      position: first\n";
    if let Some(printf) = available(&definition("printf", printf)) {
        let reply = execute(&printf, "emit", json!({"format": "\\377A\\000"}), None);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["value"]["stdout"], json!({"$bytes": "ff4100"}));
    }
    let cat = "  - name: copy\n";
    if let Some(cat) = available(&definition("cat", cat)) {
        let reply = execute(&cat, "copy", json!({}), Some(json!({"$bytes": "ff00fe0a"})));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["value"]["stdout"], json!({"$bytes": "ff00fe0a"}));
        let reply = execute(&cat, "copy", json!({"stdin": "x"}), Some(json!("y")));
        assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT", "{reply}");
    }
}

#[test]
fn a_failed_command_is_an_execution_error_with_its_result() {
    let Some(tool) = available(&definition("false", "  - name: fail\n")) else {
        return;
    };
    let reply = execute(&tool, "fail", json!({}), None);
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["code"], "EXECUTION_FAILED");
    assert_eq!(
        reply["error"]["source_class"],
        "Ukiryu::Errors::ExecutionError"
    );
    assert_eq!(reply["error"]["result"]["status"], 1);
    assert_eq!(reply["error"]["result"]["success"], false);
}

/// Without a discovered executable the original runs `nil` for a command
/// with a subcommand (the shell reports exit 127) and raises `TypeError`
/// from `File.dirname(nil)` otherwise.
#[test]
fn a_missing_executable_behaves_as_the_original() {
    let name = "ukiryu-wire-missing-tool";
    let sub = tool(&definition(name, "  - name: run\n    subcommand: run\n"));
    let reply = execute(&sub, "run", json!({}), None);
    assert_eq!(
        reply["error"]["source_class"], "Ukiryu::Errors::ExecutionError",
        "{reply}"
    );
    assert_eq!(reply["error"]["result"]["status"], 127, "{reply}");
    assert_eq!(reply["error"]["result"]["command"]["executable"], "");

    let legacy = tool(&definition(name, "  - name: run\n"));
    let reply = execute(&legacy, "run", json!({}), None);
    assert_eq!(
        reply["error"],
        json!({"code": "RUBY_EXCEPTION", "source_class": "TypeError",
               "message": "no implicit conversion of nil into String"})
    );
    let built = wire(&json!({"op": "build", "tool": legacy, "command": "run"}));
    assert_eq!(built["value"]["executable"], Value::Null, "{built}");
    assert_eq!(built["value"]["args"], json!([]), "{built}");
}

#[test]
fn execution_settings_are_required() {
    let yaml = definition("cat", "  - name: copy\n");
    for missing in ["env", "timeout"] {
        let mut request = json!({"op": "execute", "tool": tool(&yaml), "command": "copy",
            "env": {}, "timeout": 5});
        request.as_object_mut().unwrap().remove(missing);
        assert_eq!(
            wire(&request)["error"]["code"],
            "INVALID_REQUEST",
            "{missing}"
        );
    }
    let request = json!({"op": "execute", "tool": tool(&yaml), "command": "copy",
        "env": {}, "timeout": "5"});
    assert_eq!(wire(&request)["error"]["code"], "INVALID_ARGUMENT");
}
