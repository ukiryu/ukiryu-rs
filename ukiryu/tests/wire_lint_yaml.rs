//! Conformance corpus `lint` and `yaml` rows dispatched through the JSON
//! wire (`ukiryu::wire::process`): `lint.string`, `yaml.load`,
//! `yaml.model_load` and `yaml.lutaml_roundtrip`.
//!
//! Each row's input becomes wire requests; the reply is compared with the
//! row's expectation (captured from the pinned Ruby oracle) and never used to
//! produce it. Files named by a row are read from `corpus/conformance/inputs`
//! and passed as document text, or as a path where the original reads the
//! file (`Ukiryu::Models::<model>.from_yaml(path)`).
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

/// A reply in the row's `expect` shape.
fn outcome(reply: &Value) -> Value {
    if reply["ok"] != true {
        let error = &reply["error"];
        return match error.get("source_class") {
            Some(class) => json!({"error": {"class": class, "message": scrub(&error["message"])}}),
            None => json!({"wire_error": error}),
        };
    }
    json!({ "ok": scrub(&reply["value"]) })
}

fn to_yaml(model: &Value, yaml: &str) -> Value {
    wire(&json!({"op": "yaml", "method": "model_to_yaml", "model": model, "yaml": yaml}))
}

/// The row's outcome through the wire.
fn run(row: &Value) -> Value {
    let i = &row["input"];
    match row["op"].as_str().expect("op") {
        "lint.string" => outcome(&wire(&json!({
            "op": "lint", "method": "string", "yaml": document(i),
        }))),
        "yaml.load" => outcome(&wire(&json!({
            "op": "yaml", "method": "safe_load", "yaml": document(i),
        }))),
        "yaml.model_load" => outcome(&wire(&json!({
            "op": "yaml", "method": "model_load", "model": i["model"],
            "path": input_path(i["file"].as_str().expect("file")),
        }))),
        // `first = from_yaml(text).to_yaml; from_yaml(first).to_yaml == first`.
        "yaml.lutaml_roundtrip" => {
            let first = to_yaml(&i["model"], &document(i));
            let Some(emitted) = first["value"].as_str() else {
                return outcome(&first);
            };
            let again = to_yaml(&i["model"], emitted);
            json!({"ok": {
                "emitted": emitted,
                "reparsed_equal": again["value"].as_str() == Some(emitted),
            }})
        }
        op => panic!("{}: undispatched op {op}", row["id"]),
    }
}

/// Rows each op must dispatch; a change to the corpus changes these.
const PINNED: [(&str, usize); 4] = [
    ("lint.string", 70),
    ("yaml.load", 81),
    ("yaml.model_load", 62),
    ("yaml.lutaml_roundtrip", 2),
];

#[test]
fn corpus_rows_match_through_the_wire() {
    let mut dispatched = BTreeMap::<String, usize>::new();
    let mut failures = Vec::new();
    for kind in ["lint", "yaml"] {
        for row in rows(kind) {
            let id = row["id"].as_str().expect("id");
            let op = row["op"].as_str().expect("op");
            let platforms = row["applicability"]["platforms"]
                .as_array()
                .expect("platforms");
            assert!(
                platforms.iter().any(|p| p == host()),
                "{id}: does not apply on {}",
                host()
            );
            *dispatched.entry(op.to_owned()).or_default() += 1;
            let have = run(&row);
            if have != row["expect"] {
                failures.push(format!("{id}:\n  got  {have}\n  want {}", row["expect"]));
            }
        }
    }
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

/// The replies keep Ruby types the JSON profile of `parse_yaml` refuses.
#[test]
fn safe_load_keeps_ruby_types() {
    let reply = wire(&json!({
        "op": "yaml", "method": "safe_load",
        "yaml": ":sym: 1.5\n2: [.nan, ~]\nplain: text\n",
    }));
    assert_eq!(
        reply,
        json!({"ok": true, "value": {"$map": [
            [{"$sym": "sym"}, {"$float": "1.5"}],
            [2, [{"$float": "NaN"}, null]],
            ["plain", "text"],
        ]}}),
    );
    let reply = wire(&json!({"op": "yaml", "method": "safe_load", "yaml": "a: [1"}));
    assert_eq!(
        reply["error"]["source_class"], "Psych::SyntaxError",
        "{reply}"
    );
}

/// Control: an altered input changes the reply, so a match is not an echo.
#[test]
fn altered_inputs_change_the_reply() {
    let lint = |yaml: &str| wire(&json!({"op": "lint", "method": "string", "yaml": yaml}));
    let row = rows("lint")
        .into_iter()
        .find(|r| r["input"]["yaml"].is_string())
        .expect("inline lint row");
    let altered = lint("name: other\nversion: '1.0'\n");
    assert_eq!(altered["ok"], true, "{altered}");
    assert_ne!(outcome(&altered), row["expect"]);
    assert_ne!(
        outcome(&altered),
        outcome(&lint(row["input"]["yaml"].as_str().expect("yaml")))
    );
}

#[test]
fn unknown_models_are_invalid_arguments() {
    let reply = wire(&json!({
        "op": "yaml", "method": "model_to_yaml", "model": "Nope", "yaml": "a: 1",
    }));
    assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT", "{reply}");
}
