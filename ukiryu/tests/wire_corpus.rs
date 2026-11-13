//! Conformance corpus rows dispatched through the JSON wire
//! (`ukiryu::wire::process`): `build.args`, `type.validate`,
//! `schema.validate_profile`, `schema.definition_validate`,
//! `resolve.extract_version`, `resolve.range` and `resolve.version_spec`.
//!
//! Each row's input becomes one wire request; the reply is compared with the
//! row's expectation (captured from the pinned Ruby oracle) and never used to
//! produce it. Files named by a row are read from `corpus/conformance/inputs`
//! and passed as the request's document text or path. Error rows compare the
//! reply's `source_class` and message with the row's class and message.
//!
//! Not dispatched here: `resolve.shell_detect` (owner decision D2 is open and
//! the wire has no host shell detection), and the kinds other cards own
//! (`escape` in `wire_escape.rs`, `lint`, `yaml`, `cli-output`).
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace")
        .join("corpus/conformance")
}

fn inputs() -> PathBuf {
    corpus().join("inputs")
}

fn input_path(rel: &str) -> String {
    inputs().join(rel).to_string_lossy().into_owned()
}

fn read_input(rel: &str) -> String {
    std::fs::read_to_string(inputs().join(rel)).expect("corpus input")
}

/// The oracle's `read_input`: the row's `yaml` text, else its `file`.
fn document(i: &Value) -> String {
    match i["yaml"].as_str() {
        Some(yaml) => yaml.to_owned(),
        None => read_input(i["file"].as_str().expect("file")),
    }
}

fn rows(kind: &str) -> Vec<Value> {
    std::fs::read_to_string(corpus().join(format!("cases/{kind}.jsonl")))
        .expect("cases")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("row"))
        .collect()
}

fn host() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn wire(request: &Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

/// The corpus host substitution: the absolute `inputs/` path is `${INPUTS}`.
fn scrub(value: &Value) -> Value {
    let base = inputs().to_string_lossy().into_owned();
    match value {
        Value::String(s) => Value::String(s.replace(&base, "${INPUTS}")),
        Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), scrub(v))).collect())
        }
        other => other.clone(),
    }
}

/// The request for a row, or `None` for an op this test does not dispatch.
fn request(row: &Value) -> Option<Value> {
    let i = &row["input"];
    Some(match row["op"].as_str().expect("op") {
        "build.args" => json!({
            "op": "build",
            "tool": {
                "source": {
                    "kind": "version_file",
                    "register": input_path("register"),
                    "tool": i["tool"],
                    "implementation": i["implementation"],
                    "file": i["file"],
                },
                "platform": i["platform"],
                "shell": i["shell"],
            },
            "command": i["command"],
            "params": i["params"],
        }),
        "type.validate" => json!({
            "op": "validate", "method": "type",
            "value": i["value"], "type": i["type"],
            "options": if i["options"].is_null() { json!({}) } else { i["options"].clone() },
        }),
        "schema.validate_profile" => json!({
            "op": "schema", "method": "validate_profile",
            "profile": document(i),
            "schema_path": i["schema"].as_str().map(input_path),
        }),
        "schema.definition_validate" => json!({
            "op": "validate", "method": "definition",
            "yaml": document(i),
            "schema_path": i["schema"].as_str().map(input_path),
        }),
        "resolve.extract_version" => json!({
            "op": "resolve", "method": "extract_version",
            "output": i["output"], "pattern": i["pattern"],
        }),
        "resolve.range" => json!({
            "op": "resolve", "method": "range",
            "scheme": i["scheme"], "range": i["range"], "bounds": i["bounds"], "version": i["version"],
        }),
        "resolve.version_spec" => json!({
            "op": "resolve", "method": "version_spec",
            "index": read_input(&format!("register/tools/{}/index.yaml", i["tool"].as_str().expect("tool"))),
            "implementation": i["implementation"], "version": i["version"],
        }),
        _ => return None,
    })
}

fn sym(name: &str) -> Value {
    json!({ "$sym": name })
}

/// The reply in the row's `expect` shape.
fn outcome(op: &str, reply: &Value) -> Value {
    if reply["ok"] != true {
        let error = &reply["error"];
        return match error.get("source_class") {
            Some(class) => json!({"error": {"class": class, "message": scrub(&error["message"])}}),
            None => json!({"wire_error": error}),
        };
    }
    let value = &reply["value"];
    let ok = match op {
        "build.args" => value["args"].clone(),
        "schema.definition_validate" => json!({"$map": [
            [sym("valid"), value["valid"]],
            [sym("errors"), value["errors"]],
            [sym("warnings"), value["warnings"]],
            [sym("schema_path"), value["schema_path"]],
            [sym("error_count"), value["error_count"]],
            [sym("warning_count"), value["warning_count"]],
        ]}),
        _ => value.clone(),
    };
    json!({ "ok": scrub(&ok) })
}

/// Rows each op must dispatch; a change to the corpus changes these.
const PINNED: [(&str, usize); 7] = [
    ("build.args", 485),
    ("type.validate", 60),
    ("schema.validate_profile", 135),
    ("schema.definition_validate", 58),
    ("resolve.extract_version", 372),
    ("resolve.range", 68),
    ("resolve.version_spec", 130),
];

#[test]
fn corpus_rows_match_through_the_wire() {
    let mut dispatched = BTreeMap::<String, usize>::new();
    let mut not_applicable = 0;
    let mut failures = Vec::new();
    for kind in ["build", "validate", "schema", "resolve"] {
        for row in rows(kind) {
            let id = row["id"].as_str().expect("id");
            let op = row["op"].as_str().expect("op");
            let Some(request) = request(&row) else {
                assert_eq!(op, "resolve.shell_detect", "{id}: undispatched op");
                continue;
            };
            let platforms = row["applicability"]["platforms"]
                .as_array()
                .expect("platforms");
            if !platforms.iter().any(|p| p == host()) {
                not_applicable += 1;
                continue;
            }
            *dispatched.entry(op.to_owned()).or_default() += 1;
            let reply = wire(&request);
            let have = outcome(op, &reply);
            if have != row["expect"] {
                failures.push(format!("{id}:\n  got  {have}\n  want {}", row["expect"]));
            }
        }
    }
    assert_eq!(
        not_applicable, 0,
        "every dispatched row applies on every platform"
    );
    for (op, count) in PINNED {
        assert_eq!(
            dispatched.get(op).copied(),
            Some(count),
            "{op} rows dispatched"
        );
    }
    let total: usize = dispatched.values().sum();
    assert!(
        failures.is_empty(),
        "{} of {total} rows differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Control: an altered input changes the reply, so a match is not an echo.
#[test]
fn altered_inputs_change_the_reply() {
    let row = rows("build")
        .into_iter()
        .find(|r| r["id"] == "build.bzip2.default.1-0.compress.all-arguments")
        .expect("row");
    let mut altered = row.clone();
    altered["input"]["params"]["inputs"] = json!(["other.txt"]);
    let have = outcome("build.args", &wire(&request(&altered).expect("request")));
    assert_eq!(have, json!({"ok": ["other.txt"]}));
    assert_ne!(have, row["expect"]);

    let row = rows("validate")
        .into_iter()
        .find(|r| r["id"] == "validate.type.integer-max-bad")
        .expect("row");
    let mut altered = row.clone();
    altered["input"]["value"] = json!(8);
    let have = outcome("type.validate", &wire(&request(&altered).expect("request")));
    assert_eq!(have, json!({"ok": 8}));
}
