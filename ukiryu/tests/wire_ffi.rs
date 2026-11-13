//! The C ABI over the typed operations: every reply is an owned string
//! released exactly once, and repeated calls of each operation return the
//! same reply (no state leaks between calls).
#![cfg(feature = "c")]
use serde_json::{json, Value};
use std::ffi::CStr;
use ukiryu::ffi::{ukiryu_free, ukiryu_json};

fn call(request: &str) -> Value {
    // SAFETY: the span is a live &str; the reply is freed exactly once below.
    let reply = unsafe { ukiryu_json(request.as_ptr(), request.len()) };
    assert!(!reply.is_null());
    let bytes = unsafe { CStr::from_ptr(reply) }.to_bytes().to_vec();
    unsafe { ukiryu_free(reply) };
    serde_json::from_slice(&bytes).expect("reply is JSON")
}

const DEFINITION: &str = "name: tool\nprofiles:\n- name: default\n  commands:\n  - name: run\n    arguments:\n    - name: input\n      type: string\n      position: first\n    exit_codes:\n      standard:\n        '0': success\n";

#[test]
fn repeated_calls_of_each_operation_are_stable_and_freed() {
    let tool = json!({"source": {"kind": "definition", "yaml": DEFINITION}, "platform": "linux", "shell": "bash"});
    let requests = [
        json!({"op": "build", "tool": tool, "command": "run", "params": {"input": "a b"}}),
        json!({"op": "build", "tool": tool, "command": "missing"}),
        json!({"op": "resolve", "method": "range", "scheme": "semantic", "range": "between",
               "bounds": {"from": "1.0.0", "to": "2.0.0"}, "version": "1.5.0"}),
        json!({"op": "resolve", "method": "extract_version", "output": "tool 1.2.3", "pattern": "(\\d+\\.\\d+\\.\\d+)"}),
        json!({"op": "validate", "method": "type", "value": "3", "type": {"$sym": "integer"}}),
        json!({"op": "validate", "method": "definition", "yaml": DEFINITION}),
        json!({"op": "schema", "method": "fully_validate", "profile": "name: 1\n",
               "schema": "type: object\nproperties:\n  name:\n    type: string\n"}),
        json!({"op": "results", "method": "exit_code_meaning", "tool": tool, "command": "run", "status": 0}),
        json!({"op": "escape", "shell": "bash", "operation": {"method": "quote", "string": "it's"}}),
    ];
    for request in requests {
        let text = request.to_string();
        let first = call(&text);
        assert!(first.get("ok").is_some(), "{text}: {first}");
        for _ in 0..300 {
            assert_eq!(call(&text), first, "{text}");
        }
    }
    let built = call(
        &json!({"op": "build", "tool": tool, "command": "run", "params": {"input": "a b"}})
            .to_string(),
    );
    assert_eq!(built["value"]["args"], json!(["a b"]), "{built}");
    let missing = call(&json!({"op": "build", "tool": tool, "command": "missing"}).to_string());
    assert_eq!(missing["error"]["source_class"], "ArgumentError");
    assert_eq!(missing["error"]["message"], "Unknown command: missing");
    let meaning = call(&json!({"op": "results", "method": "exit_code_meaning", "tool": tool, "command": "run", "status": 0}).to_string());
    assert_eq!(meaning["value"], "success", "{meaning}");
}
