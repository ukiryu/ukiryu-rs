//! Compares request loading with the pinned Ruby reference captured by
//! `fixtures/request/capture-request-reference.rb` (Ruby 4.0.1, Psych 5.3.1,
//! corpus oracle Gemfile.lock fc3b12f2, x86_64-linux).
//!
//! Every captured case must match exactly unless `fixtures/request/pending.json`
//! lists it. `detail` entries match the Ruby class and the public message
//! prefix while the parser diagnostic after it differs. `pending` entries are
//! known divergences that need a change outside this module; they must still
//! diverge, so a fix cannot leave a stale entry behind. The Ruby expectation
//! of a pending case is never changed to the port's result.
//!
//! The `directory` case depends on the host: Ruby reports `io_fread` where
//! the directory opens (Linux, macOS) and `rb_sysopen` where it does not
//! (Windows). On Windows it is compared with the native Windows capture in
//! `reference-windows-ruby3.4.json` (Ruby 3.4.8, Psych 5.3.1, x64-mingw-ucrt).
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use ukiryu::models::ruby::float_to_s;
use ukiryu::request::{load_execution_request, ExecutionRequest, Value};

fn root() -> String {
    format!("{}/../fixtures/request", env!("CARGO_MANIFEST_DIR"))
}

fn read_json(name: &str) -> Json {
    let path = format!("{}/{name}", root());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// The capture's typed value profile for a Ruby value.
fn typed(value: &Value) -> Json {
    match value {
        Value::Nil => json!({"type": "nil"}),
        Value::Bool(flag) => json!({"type": "boolean", "value": flag}),
        Value::String(text) => json!({"type": "string", "value": text}),
        Value::Symbol(name) => json!({"type": "symbol", "value": name}),
        Value::Integer(n) => json!({"type": "integer", "value": n.to_string()}),
        Value::Float(n) => json!({"type": "float", "value": float_text(*n)}),
        Value::Array(items) => {
            json!({"type": "array", "items": items.iter().map(typed).collect::<Vec<_>>()})
        }
        Value::Hash(pairs) => json!({"type": "hash", "entries":
            pairs.iter().map(|(k, v)| json!([typed(k), typed(v)])).collect::<Vec<_>>()}),
        other => json!({"type": "unsupported", "class": other.class_name()}),
    }
}

fn float_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        float_to_s(n)
    }
}

fn outcome(file: &str) -> (Json, Option<Json>) {
    let cases = format!("{}/cases", root());
    let path = if file.is_empty() {
        cases.clone()
    } else {
        format!("{cases}/{file}")
    };
    match load_execution_request(&path) {
        Ok(request) => {
            let keys = match request.arguments().stringify_keys() {
                Ok(hash) => json!({"ok": typed(&hash)}),
                Err(error) => {
                    json!({"error": {"class": error.ruby_class(), "message": error.message}})
                }
            };
            (json!({"ok": typed(&request.to_value())}), Some(keys))
        }
        Err(error) => {
            let message = error.to_string().replace(&cases, "$CASES");
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
    let provenance = &reference["provenance"];
    assert_eq!(
        provenance["revision"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    assert_eq!(provenance["ruby"], "4.0.1");
    assert_eq!(provenance["psych"], "5.3.1");
    assert_eq!(
        provenance["gemfile_lock_sha256"],
        "fc3b12f2ede259028b638d2f26c65ddf9506d142c20a8bf0c1b151741078b65e"
    );
    let windows = read_json("reference-windows-ruby3.4.json");
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
    let on_disk = std::fs::read_dir(format!("{}/cases", root()))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".yaml")
        })
        .count();
    // Every case file plus the missing-file and directory inputs.
    assert_eq!(cases.len(), on_disk + 2, "reference.json is stale");

    let mut failures = Vec::new();
    let (mut exact, mut detail, mut pending_count) = (0, 0, 0);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let mut expected = case.clone();
        if cfg!(windows) && id == "directory" {
            expected = windows["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == "directory")
                .unwrap()
                .clone();
        }
        let (load, keys) = outcome(case["path"].as_str().unwrap());
        let matches = load == expected["load"]
            && keys
                .as_ref()
                .map_or(expected.get("stringify_keys").is_none(), |k| {
                    Some(k) == expected.get("stringify_keys")
                });
        let report = || {
            format!(
                "{id}:\n   expected {} / {:?}\n   actual   {load} / {keys:?}",
                expected["load"],
                expected.get("stringify_keys")
            )
        };
        match status.get(id).map(String::as_str) {
            None => {
                exact += 1;
                if !matches {
                    failures.push(report());
                }
            }
            Some("detail") => {
                detail += 1;
                let (want, got) = (&expected["load"]["error"], &load["error"]);
                let same = want["class"] == got["class"]
                    && got["message"].is_string()
                    && prefix(want["message"].as_str().unwrap())
                        == prefix(got["message"].as_str().unwrap());
                eprintln!("detail {}", report());
                if !same {
                    failures.push(format!("detail case changed: {}", report()));
                }
            }
            Some("pending") => {
                pending_count += 1;
                eprintln!("pending {}", report());
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
        Some(&Value::str("spec/fixtures/output.jpg"))
    );

    let missing = load_execution_request(format!("{cases}/spec-missing-fields.yaml")).unwrap_err();
    assert_eq!(missing.source_class(), Some("RuntimeError"));
    assert!(missing.to_string().contains("tool"));

    let sequence =
        load_execution_request(format!("{cases}/spec-arguments-sequence.yaml")).unwrap_err();
    assert_eq!(sequence.source_class(), Some("RuntimeError"));
    assert!(sequence.to_string().contains("arguments"));

    let absent = load_execution_request("/nonexistent/file.yaml").unwrap_err();
    assert_eq!(absent.source_class(), Some("Thor::Error"));

    // Round trip: a loaded request serializes through Yeptris and reloads
    // unchanged (this request holds no Symbols and no YAML 1.1 lookalikes).
    let tree = yeptris::to_value(&valid).unwrap();
    let yaml = yeptris::to_string(&tree).unwrap();
    assert_eq!(ExecutionRequest::from_yaml(&yaml).unwrap(), valid);
    assert_eq!(
        ExecutionRequest::from_value(valid.to_value()).unwrap(),
        valid
    );
}
