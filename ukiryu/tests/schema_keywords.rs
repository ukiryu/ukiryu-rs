//! Exact keyword/dialect observations from clean ab6df353, json-schema 6.2.1.
use serde_json::{json, Value};
use ukiryu::schema::{self, Data, Permitted};

fn data(value: &Value) -> Data {
    schema::safe_load(&serde_json::to_string(value).unwrap(), Permitted::DOCUMENT).unwrap()
}

#[test]
fn keyword_dialect_goldens_match_ruby() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/schema_golden/keywords")
        .to_string_lossy()
        .replace('\\', "/");
    let file_uri = format!(
        "file://{}{}",
        if fixture_path.starts_with('/') {
            ""
        } else {
            "/"
        },
        fixture_path
    );
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let text = include_str!("schema_golden/keywords/cases.json")
        .replace("${CWD}", &cwd)
        .replace("${FIXTURE_FILE_URI}", &file_uri)
        .replace("${FIXTURE}", &fixture_path);
    let fixture: Value = serde_json::from_str(&text).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 4500, "keyword fixture shrank");
    assert!(cases.iter().any(|case| case["expect"]["ok"] == json!([])));
    assert!(cases.iter().any(|case| case["expect"]["error"].is_object()));
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in cases {
        let id = case["id"].as_str().unwrap();
        checked += 1;
        let document = match case["data_yaml"].as_str() {
            Some(yaml) => schema::safe_load(yaml, Permitted::DOCUMENT).unwrap(),
            None => data(&case["data"]),
        };
        let result = schema::fully_validate(
            &data(&case["schema"]),
            &document,
            case["strict"].as_bool().unwrap(),
        );
        let actual = match result {
            Ok(messages) => json!({"ok":messages}),
            Err(e) => json!({"error":{"class":e.ruby_class(),"message":e.message()}}),
        };
        if actual != case["expect"] {
            failures.push(json!({"id":id,"schema":case["schema"],"data":case["data"],"strict":case["strict"],"want":case["expect"],"got":actual}));
        }
    }
    assert!(checked > 0);
    eprintln!(
        "keyword cases checked={checked}, failures={}",
        failures.len()
    );
    for failure in &failures {
        eprintln!("KEYWORD_FAILURE {failure}");
    }
    assert!(
        failures.is_empty(),
        "{} keyword/dialect mismatches",
        failures.len()
    );
}
