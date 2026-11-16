use serde_json::{json, Value};
#[path = "version_detection_bytes/transport.rs"]
mod transport;

#[test]
fn frozen_encoding_regex_and_error_stages() {
    let rows: Vec<Value> =
        serde_json::from_str(include_str!("version_detection_bytes/cases.json")).unwrap();
    assert_eq!(rows.len(), 663);
    let mut failures = Vec::new();
    for row in &rows {
        let actual = transport::request(&row["input"]);
        if actual != row["expect"] {
            failures.push(format!("{}: {actual} != {}", row["id"], row["expect"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures; first12 {:#?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
}

#[test]
fn every_shared_extract_version_row_including_31_binary_strings() {
    let mut count = 0;
    let mut binary = 0;
    let mut failures = Vec::new();
    for line in include_str!("../../corpus/conformance/cases/resolve.jsonl")
        .split('\n')
        .filter(|s| !s.is_empty())
    {
        let row: Value = serde_json::from_str(line).unwrap();
        if row["op"] != "resolve.extract_version" {
            continue;
        }
        assert_eq!(row["applicability"]["requires"], json!([]));
        let input = &row["input"];
        let output = if input["output"].is_null() {
            Value::Null
        } else if let Some(text) = input["output"].as_str() {
            json!({"hex":transport::hex(text.as_bytes()),"encoding":"UTF-8"})
        } else {
            binary += 1;
            json!({"hex":input["output"]["$bytes"],"encoding":"ASCII-8BIT"})
        };
        let actual = transport::request(&json!({"output":output,"pattern":input["pattern"]}));
        let expected = &row["expect"]["ok"];
        let expected = if expected.is_null() {
            json!({"ok":null})
        } else if let Some(text) = expected.as_str() {
            json!({"ok":{"hex":transport::hex(text.as_bytes()),"encoding":"UTF-8"}})
        } else {
            json!({"ok":{"hex":expected["$bytes"],"encoding":"ASCII-8BIT"}})
        };
        if actual != expected {
            failures.push(format!("{}: {actual} != {expected}", row["id"]));
        }
        count += 1;
    }
    assert_eq!((count, binary), (372, 31));
    assert!(
        failures.is_empty(),
        "{} failures; first12 {:#?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
}
