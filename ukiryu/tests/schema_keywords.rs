//! Exact keyword/dialect observations from clean ab6df353, json-schema 6.2.1.
use serde_json::{json, Value};
use ukiryu::schema::{self, Data, Permitted};

fn data(value: &Value) -> Data {
    schema::safe_load(&serde_json::to_string(value).unwrap(), Permitted::DOCUMENT).unwrap()
}

// The oracle capture used a POSIX path. Under a URN base, replacing that
// input with a drive path changes URI resolution (urn:/x versus C:/x).
// Keep that input's URI shape; file-based rows still use the real host path.
fn keyword_fixture(fixture_path: &str, cwd: &str) -> Value {
    let fixture_path = fixture_path.replace('\\', "/");
    let posix_path = if fixture_path
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && fixture_path.as_bytes().get(1) == Some(&b':')
        && fixture_path.as_bytes().get(2) == Some(&b'/')
    {
        &fixture_path[2..]
    } else {
        &fixture_path
    };
    let file_uri = format!(
        "file://{}{}",
        if fixture_path.starts_with('/') {
            ""
        } else {
            "/"
        },
        fixture_path
    );
    let mut fixture: Value =
        serde_json::from_str(include_str!("schema_golden/keywords/cases.json")).unwrap();
    for case in fixture["cases"].as_array_mut().unwrap() {
        let path = if case["schema"]["id"] == "urn:ukiryu:keyword-root"
            && case["schema"]["extends"] == "${FIXTURE}/resources/target.json"
        {
            posix_path
        } else {
            &fixture_path
        };
        *case = serde_json::from_str(
            &case
                .to_string()
                .replace("${CWD}", cwd)
                .replace("${FIXTURE_FILE_URI}", &file_uri)
                .replace("${FIXTURE}", path),
        )
        .unwrap();
    }
    fixture
}

#[test]
fn windows_fixture_paths_preserve_posix_uri_inputs() {
    for path in ["C:/x.json", r"C:\x.json"] {
        let fixture = keyword_fixture(path, "/cwd");
        let cases = fixture["cases"].as_array().unwrap();
        let urn_cases: Vec<_> = cases
            .iter()
            .filter(|case| {
                case["id"]
                    .as_str()
                    .unwrap()
                    .starts_with("extends.resource.0.")
            })
            .collect();
        assert_eq!(urn_cases.len(), 4);
        for case in urn_cases {
            assert_eq!(case["schema"]["extends"], "/x.json/resources/target.json");
            assert_eq!(case["expect"]["error"]["class"], "Errno::ENOENT");
            assert_eq!(
                case["expect"]["error"]["message"],
                "No such file or directory @ rb_sysopen - urn:/x.json/resources/target.json"
            );
            let error = schema::fully_validate(&data(&case["schema"]), &data(&case["data"]), false)
                .unwrap_err();
            assert!(matches!(
                &error,
                schema::SchemaError::Io { errno, .. } if errno == "ENOENT"
            ));
            assert_eq!(
                json!({"class":error.ruby_class(), "message":error.message()}),
                case["expect"]["error"],
                "{} with {path}",
                case["id"]
            );
        }
        let file_case = cases
            .iter()
            .find(|case| case["id"] == "extends.file.0.0")
            .unwrap();
        assert_eq!(
            file_case["schema"]["extends"],
            "C:/x.json/resources/target.json"
        );
        assert_eq!(file_case["schema"]["id"], "file:///C:/x.json/root.json");
    }
}

#[test]
fn keyword_dialect_goldens_match_ruby() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/schema_golden/keywords")
        .to_string_lossy()
        .replace('\\', "/");
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let fixture = keyword_fixture(&fixture_path, &cwd);
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
