//! The tool runtime oracle (`tests/tool_runtime_data/oracle.json`, captured
//! from the pinned Ruby `Tool.new`/`#execute` with authored definitions)
//! dispatched through the wire's `resolve` (`method: tool`), `build` and
//! `results` operations with the definition YAML as the source.
//!
//! The capture ran `Tool.new` with a discovery directory holding fake
//! executables; the wire discovers executables on the real `PATH`, so the
//! executable and availability fields are not compared here. Everything the
//! definition alone decides is: load errors (class and message), name,
//! version, chosen profile, routing, command names, built arguments,
//! environment, stdin, command name, build-stage errors and exit-code
//! meanings.
use serde_json::{json, Value};

fn oracle() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/tool_runtime_data/oracle.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("oracle")).expect("json")
}

fn wire(request: &Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

/// The capture's `{t, v}` value form in the wire's typed encoding.
fn typed(j: &Value) -> Value {
    let v = &j["v"];
    match j["t"].as_str().expect("tag") {
        "nil" => Value::Null,
        "bool" | "int" | "str" => v.clone(),
        "float" => json!({ "$float": v }),
        "sym" => json!({ "$sym": v }),
        "arr" => Value::Array(v.as_array().expect("arr").iter().map(typed).collect()),
        "hash" => json!({ "$map": v.as_array().expect("hash").iter()
            .map(|p| json!([typed(&p[0]), typed(&p[1])])).collect::<Vec<_>>() }),
        other => panic!("unexpected tag {other}"),
    }
}

fn tool_request(row: &Value, yaml: &str) -> Value {
    json!({
        "source": {"kind": "definition", "yaml": yaml},
        "platform": row["platform"],
        "shell": row["shell"],
        "version": row["version_option"],
    })
}

#[test]
fn synthetic_tools_match_ruby_through_the_wire() {
    let oracle = oracle();
    let definitions = &oracle["definitions"];
    let rows = oracle["synthetic"].as_array().expect("rows");
    assert_eq!(rows.len(), 25, "pinned synthetic rows");
    let mut failures = Vec::new();
    let (mut load_errors, mut loaded, mut built, mut build_errors, mut meanings) = (0, 0, 0, 0, 0);
    for row in rows {
        let id = format!(
            "{} {}/{} ({})",
            row["file"], row["platform"], row["shell"], row["path"]
        );
        let yaml = definitions[row["input_yaml"].as_str().expect("input_yaml")]
            .as_str()
            .expect("definition yaml");
        let tool = tool_request(row, yaml);
        let mut request = tool.clone();
        request["op"] = json!("resolve");
        request["method"] = json!("tool");
        let reply = wire(&request);
        if let Some(error) = row.get("load_error") {
            load_errors += 1;
            let have = json!({"class": reply["error"]["source_class"], "message": reply["error"]["message"]});
            if reply["ok"] != false || &have != error {
                failures.push(format!("{id}: load error {reply} != {error}"));
            }
            continue;
        }
        if reply["ok"] != true {
            failures.push(format!("{id}: load failed: {reply}"));
            continue;
        }
        loaded += 1;
        let value = &reply["value"];
        for (field, want) in [
            ("name", &row["name"]),
            ("version", &row["version"]),
            ("profile_index", &row["profile_index"]),
            ("profile_name", &row["profile_name"]),
            ("routing", &row["routing"]),
            ("commands", &row["command_names"]),
        ] {
            if &value[field] != want {
                failures.push(format!("{id}: {field} {} != {want}", value[field]));
            }
        }
        for command in row["commands"].as_array().into_iter().flatten() {
            let path = &command["path"];
            if let Some(expected) = command.get("exit_meanings") {
                for pair in expected.as_array().expect("meanings") {
                    meanings += 1;
                    let reply = wire(&json!({
                        "op": "results", "method": "exit_code_meaning",
                        "tool": tool, "command": path, "status": pair[0],
                    }));
                    if reply["value"] != pair[1] || reply["ok"] != true {
                        failures.push(format!(
                            "{id} {path} exit {}: {reply} != {}",
                            pair[0], pair[1]
                        ));
                    }
                }
            }
            for case in command["cases"].as_array().expect("cases") {
                let result = &case["result"];
                // A missing executable or an unnamed environment variable
                // fails only when the original runs the command (TypeError);
                // the build stage ends before that. The execute operation
                // keeps those failures (tests/wire_execute.rs).
                if row["path"] == "empty" || result["err"]["class"] == "TypeError" {
                    continue;
                }
                let reply = wire(&json!({
                    "op": "build", "tool": tool, "command": path, "params": typed(&case["params"]),
                }));
                if let Some(ok) = result.get("ok") {
                    built += 1;
                    let value = &reply["value"];
                    let have = json!({
                        "args": value["args"], "env": value["env"],
                        "stdin": value["stdin"], "command_name": value["command_name"],
                    });
                    let want = json!({
                        "args": ok["args"], "env": ok["env"],
                        "stdin": ok["stdin"], "command_name": ok["command_name"],
                    });
                    if reply["ok"] != true || have != want {
                        failures.push(format!("{id} {path} {}: {reply} != {want}", case["set"]));
                    }
                } else {
                    build_errors += 1;
                    let err = &result["err"];
                    let have = json!({"class": reply["error"]["source_class"], "message": reply["error"]["message"]});
                    if reply["ok"] != false || &have != err {
                        failures.push(format!("{id} {path} {}: {reply} != {err}", case["set"]));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(load_errors, 3, "load error rows");
    assert_eq!(loaded, 22, "loaded rows");
    assert!(built > 0 && build_errors > 0 && meanings > 0);
    eprintln!("wire tool oracle: {loaded} loaded, {load_errors} load errors, {built} builds, {build_errors} build errors, {meanings} exit meanings");
}

/// Request shape errors are typed and never reach the domain.
#[test]
fn malformed_requests_are_rejected_with_stable_codes() {
    for request in [
        json!({"op": "build"}),
        json!({"op": "resolve", "method": "nope"}),
        json!({"op": "validate", "method": "type", "value": 1.5, "type": {"$sym": "float"}}),
        json!({"op": "validate", "method": "type", "value": {"$float": "1.50"}, "type": {"$sym": "float"}}),
        json!({"op": "validate", "method": "type", "value": {"$sym": "a", "x": 1}, "type": {"$sym": "symbol"}}),
        json!({"op": "validate", "method": "type", "value": {"$unknown": 1}, "type": {"$sym": "string"}}),
        json!({"op": "validate", "method": "type", "value": {"$time": "2024-02-30T00:00:00.000000000Z"}, "type": {"$sym": "datetime"}}),
        json!({"op": "validate", "method": "type", "value": {"$bytes": "ff"}, "type": {"$sym": "string"}}),
        json!({"op": "validate", "method": "type", "value": {"a": 1, "a ": 2}, "type": {"$sym": "hash"}, "extra": true}),
    ] {
        let reply = wire(&request);
        assert_eq!(reply["ok"], false, "{request}: {reply}");
        assert_eq!(
            reply["error"]["code"], "INVALID_REQUEST",
            "{request}: {reply}"
        );
    }
    let reply = wire(&json!({"op": "build", "tool": {
        "source": {"kind": "definition", "yaml": "name: x\nprofiles:\n- name: d\n  commands: []\n"},
        "platform": "plan9", "shell": "bash"}, "command": "c"}));
    assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT", "{reply}");
    let reply = wire(&json!({"op": "build", "tool": {
        "source": {"kind": "definition", "yaml": "name: x\nprofiles:\n- name: d\n  commands: []\n"},
        "platform": "linux", "shell": "ksh"}, "command": "c"}));
    assert_eq!(reply["error"]["code"], "UNKNOWN_SHELL", "{reply}");
    assert_eq!(
        reply["error"]["source_class"],
        "Ukiryu::Errors::UnknownShellError"
    );
}

/// Typed values cross the wire unchanged: exact floats, Symbols, ordered
/// Hashes with non-String keys, and Times (UTC and with an offset, to the
/// nanosecond) through `Type.validate`, which returns these values as given.
#[test]
fn typed_values_round_trip_exactly() {
    let cases = [
        (json!({"$float": "0.1"}), json!({"$sym": "float"})),
        (json!({"$float": "1.0e+20"}), json!({"$sym": "float"})),
        (
            json!({"$time": "2024-01-17T10:20:30.123456789Z"}),
            json!({"$sym": "datetime"}),
        ),
        (
            json!({"$time": "1969-12-31T23:59:59.999999999Z"}),
            json!({"$sym": "datetime"}),
        ),
        (
            json!({"$time": {"seconds": "1705486830", "nanos": 1, "utc_offset": 19800, "utc": false}}),
            json!({"$sym": "datetime"}),
        ),
        (
            json!({"$map": [[{"$sym": "b"}, 1], ["a", [true, null]], [2, {"$sym": "x"}]]}),
            json!({"$sym": "hash"}),
        ),
        (
            json!({"$int": "123456789012345678901234567890"}),
            json!({"$sym": "integer"}),
        ),
    ];
    for (value, r#type) in cases {
        let reply =
            wire(&json!({"op": "validate", "method": "type", "value": value, "type": r#type}));
        assert_eq!(reply["ok"], true, "{value}: {reply}");
        let want = if value.get("$int").is_some() {
            serde_json::from_str::<Value>("123456789012345678901234567890").unwrap()
        } else {
            value.clone()
        };
        assert_eq!(reply["value"], want, "{value}");
    }
}
