//! Compares request loading with the pinned Ruby reference captured by
//! `fixtures/request/capture-request-reference.rb`.
//!
//! Every captured case must match exactly unless `fixtures/request/pending.json`
//! lists it. `detail` entries match the Ruby class and the public message
//! prefix while the parser/OS diagnostic after it differs. `pending` entries
//! are known divergences owned elsewhere; they must still diverge, so a fix
//! cannot leave a stale entry behind.
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use ukiryu::request::{load_execution_request, ArgumentValue, ExecutionRequest};
use yeptris::Number;

fn root() -> String {
    format!("{}/../fixtures/request", env!("CARGO_MANIFEST_DIR"))
}

fn read_json(name: &str) -> Json {
    let path = format!("{}/{name}", root());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// The capture's typed value profile for a Yeptris value.
fn typed(value: &ArgumentValue) -> Json {
    match value {
        ArgumentValue::Null => json!({"type": "nil"}),
        ArgumentValue::Bool(flag) => json!({"type": "boolean", "value": flag}),
        ArgumentValue::String(text) => json!({"type": "string", "value": text}),
        ArgumentValue::Number(Number::I64(n)) => json!({"type": "integer", "value": n.to_string()}),
        ArgumentValue::Number(Number::U64(n)) => json!({"type": "integer", "value": n.to_string()}),
        ArgumentValue::Number(Number::F64(n)) => json!({"type": "float", "value": float_text(*n)}),
        ArgumentValue::Sequence(items) => {
            json!({"type": "array", "items": items.iter().map(typed).collect::<Vec<_>>()})
        }
        ArgumentValue::Mapping(mapping) => json!({"type": "hash", "entries":
            mapping.0.iter().map(|(k, v)| json!([typed(k), typed(v)])).collect::<Vec<_>>()}),
        ArgumentValue::Tagged(tagged) => {
            json!({"type": "tagged", "tag": tagged.tag, "value": typed(&tagged.value)})
        }
    }
}

/// Ruby `Float#to_s` for the values used in the fixtures.
fn float_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n.fract() == 0.0 && n.abs() < 1e16 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    }
}

fn outcome(id: &str, file: &str) -> (Json, Option<Json>) {
    let cases = format!("{}/cases", root());
    let path = if file.is_empty() {
        cases.clone()
    } else {
        format!("{cases}/{file}")
    };
    match load_execution_request(&path) {
        Ok(request) => {
            let keys = match request.arguments().stringify_keys() {
                Ok(entries) => json!({"ok": {"type": "hash", "entries": entries.iter().map(|(k, v)|
                    json!([{"type": "symbol", "value": k}, typed(v)])).collect::<Vec<_>>()}}),
                Err(error) => {
                    json!({"error": {"class": error.source_class(), "message": error.to_string()}})
                }
            };
            (json!({"ok": typed(&request.to_value())}), Some(keys))
        }
        Err(error) => {
            let message = error.to_string().replace(&cases, "$CASES");
            let _ = id;
            (
                json!({"error": {"class": error.source_class(), "message": message}}),
                None,
            )
        }
    }
}

fn prefix(message: &str) -> &str {
    message.split_once(": ").map_or(message, |(head, _)| head)
}

#[test]
fn request_loading_matches_pinned_ruby_reference() {
    let reference = read_json("reference.json");
    assert_eq!(
        reference["provenance"]["revision"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    let pending = read_json("pending.json");
    let status: BTreeMap<String, String> = pending["cases"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, entry)| (id.clone(), entry["status"].as_str().unwrap().to_owned()))
        .collect();
    let cases = reference["cases"].as_array().unwrap();
    let ids: Vec<&str> = cases.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for id in status.keys() {
        assert!(
            ids.contains(&id.as_str()),
            "pending.json names unknown case {id}"
        );
    }

    let mut failures = Vec::new();
    let (mut exact, mut detail, mut pending_count) = (0, 0, 0);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let (load, keys) = outcome(id, case["path"].as_str().unwrap());
        let matches = load == case["load"]
            && keys
                .as_ref()
                .map_or(case.get("stringify_keys").is_none(), |k| {
                    Some(k) == case.get("stringify_keys")
                });
        match status.get(id).map(String::as_str) {
            None => {
                exact += 1;
                if !matches {
                    failures.push(format!(
                        "{id}: expected {} / {:?}\n   actual {load} / {keys:?}",
                        case["load"],
                        case.get("stringify_keys")
                    ));
                }
            }
            Some("detail") => {
                detail += 1;
                let (expected, actual) = (&case["load"]["error"], &load["error"]);
                let same = expected["class"] == actual["class"]
                    && actual["message"].is_string()
                    && prefix(expected["message"].as_str().unwrap())
                        == prefix(actual["message"].as_str().unwrap());
                if !same {
                    failures.push(format!("{id}: detail case changed: {expected} vs {actual}"));
                }
            }
            Some("pending") => {
                pending_count += 1;
                if matches {
                    failures.push(format!(
                        "{id}: now matches Ruby; remove it from pending.json"
                    ));
                }
            }
            Some(other) => failures.push(format!("{id}: unknown status {other}")),
        }
    }
    eprintln!("request reference: exact={exact} detail={detail} pending={pending_count}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn authored_spec_requests_validate_as_in_ruby_spec() {
    // spec/ukiryu/cli_commands/run_file_command_spec.rb at the pinned revision.
    let cases = format!("{}/cases", root());
    let valid = load_execution_request(format!("{cases}/spec-valid.yaml")).unwrap();
    assert_eq!(valid.tool_name(), Some("imagemagick"));
    assert_eq!(valid.command_name(), Some("convert"));
    assert_eq!(
        valid.arguments().get("output"),
        Some(&ArgumentValue::String("spec/fixtures/output.jpg".into()))
    );

    let missing = load_execution_request(format!("{cases}/spec-missing-fields.yaml")).unwrap_err();
    assert_eq!(missing.source_class(), "RuntimeError");
    assert!(missing.to_string().contains("tool"));

    let sequence =
        load_execution_request(format!("{cases}/spec-arguments-sequence.yaml")).unwrap_err();
    assert_eq!(sequence.source_class(), "RuntimeError");
    assert!(sequence.to_string().contains("arguments"));

    let absent = load_execution_request("/nonexistent/file.yaml").unwrap_err();
    assert_eq!(absent.source_class(), "Thor::Error");

    // Round trip: a loaded request serializes and reloads unchanged.
    let tree = yeptris::to_value(&valid).unwrap();
    let yaml = yeptris::to_string(&tree).unwrap();
    assert_eq!(ExecutionRequest::from_yaml(&yaml).unwrap(), valid);
}
