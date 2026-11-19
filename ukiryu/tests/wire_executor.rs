//! The `executor` wire op against the pinned Ruby `Ukiryu::Executor.execute`
//! (ukiryu/ukiryu ab6df353). Each row of `executor_data/executor_wire.json`
//! holds the request and what Ruby returned or raised for it, captured by
//! `executor_data/capture/capture_executor_wire.rb` on Linux with bash.
#![cfg(target_os = "linux")]
use serde_json::{json, Value};

fn wire(request: &Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

fn rows() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/executor_data/executor_wire.json"
    );
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        data["oracle"]["sha"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    data["rows"].as_array().unwrap().clone()
}

/// The wire request for a captured case. `cwd: true` runs in a fresh
/// directory holding `in.txt`, as the capture did.
fn request(case: &Value, dir: &std::path::Path) -> Value {
    let mut request = json!({"op": "executor", "executable": case["executable"]});
    for key in [
        "args",
        "env",
        "shell",
        "stdin",
        "tool_name",
        "command_name",
        "allow_failure",
    ] {
        if let Some(value) = case.get(key) {
            request[key] = value.clone();
        }
    }
    if let Some(timeout) = case.get("timeout") {
        request["timeout"] = match timeout.as_f64() {
            Some(f) if timeout.is_f64() => json!({"$float": f.to_string()}),
            _ => timeout.clone(),
        };
    }
    if case["cwd"] == true {
        std::fs::write(dir.join("in.txt"), "file content").unwrap();
        request["cwd"] = json!(dir.to_str().unwrap());
    }
    request
}

fn timeout_value(value: &Value) -> Value {
    match value.get("$float") {
        Some(text) => json!(text.as_str().unwrap().parse::<f64>().unwrap()),
        None => value.clone(),
    }
}

#[test]
fn executor_matches_the_ruby_executor() {
    let rows = rows();
    assert_eq!(rows.len(), 14);
    for row in rows {
        let case = &row["case"];
        let ruby = &row["ruby"];
        let id = case["id"].as_str().unwrap();
        let dir = std::env::temp_dir().join(format!("ukiryu-wire-executor-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let reply = wire(&request(case, &dir));
        std::fs::remove_dir_all(&dir).unwrap();
        if ruby["outcome"] == "raise" {
            assert_eq!(reply["ok"], false, "{id}: {reply}");
            assert_eq!(reply["error"]["source_class"], ruby["class"], "{id}");
            assert_eq!(reply["error"]["message"], ruby["message"], "{id}");
            continue;
        }
        assert_eq!(reply["ok"], true, "{id}: {reply}");
        let value = &reply["value"];
        for key in ["stdout", "stderr", "status", "success"] {
            assert_eq!(value[key], ruby[key], "{id}: {key}");
        }
        for key in [
            "executable",
            "arguments",
            "full_command",
            "shell",
            "tool_name",
            "command_name",
        ] {
            assert_eq!(value["command"][key], ruby[key], "{id}: {key}");
        }
        assert_eq!(timeout_value(&value["timeout"]), ruby["timeout"], "{id}");
    }
}

/// D1: the environment is required on the wire; the Ruby binding passes the
/// classic default (`{}`, applied over the inherited environment).
#[test]
fn the_environment_is_required() {
    let reply =
        wire(&json!({"op": "executor", "executable": "echo", "shell": "bash", "timeout": 5}));
    assert_eq!(reply["error"]["code"], "INVALID_REQUEST", "{reply}");
}

/// D2: only concrete shell names; the source also accepts platform groups.
#[test]
fn a_platform_group_is_not_a_shell() {
    let reply = wire(&json!({"op": "executor", "executable": "echo", "env": {},
        "shell": "unix", "timeout": 5}));
    assert_eq!(
        reply["error"]["source_class"],
        "Ukiryu::Errors::UnknownShellError"
    );
}
