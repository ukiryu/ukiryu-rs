//! Every register/profile fixture measured against the original Ruby linter
//! and DefinitionValidator. No Unsupported outcomes or skipped rows.
use serde_json::{json, Value};
use std::path::Path;
use ukiryu::definition::linter::{DefinitionLinter, LintError};
use ukiryu::definition::{definition_validator, ValidatorEnv};

#[test]
fn all_register_and_profile_fixtures_match_ruby_issues() {
    let fixture: Value =
        serde_json::from_str(include_str!("author_data/fixture_cases.json")).unwrap();
    let inputs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance/inputs");
    let inputs = inputs.canonicalize().unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 64);
    for row in rows {
        let relative = row["path"].as_str().unwrap();
        let path = inputs.join(relative);
        let lint = match DefinitionLinter::lint_file(path.to_str().unwrap(), None) {
            Ok(result) => {
                json!({"kind": "result", "to_h": result.to_h(), "to_s": result.to_string()})
            }
            Err(LintError::Raised { class, message }) => {
                json!({"kind": "raised", "class": class, "message": message})
            }
            Err(error) => panic!("{relative}: {error:?}"),
        };
        let expected = row["lint"]
            .to_string()
            .replace("{root}", &inputs.to_string_lossy());
        assert_eq!(
            lint,
            serde_json::from_str::<Value>(&expected).unwrap(),
            "{relative}: lint"
        );
        let result = definition_validator::validate_file(
            path.to_str().unwrap(),
            None,
            &ValidatorEnv::default(),
        )
        .unwrap();
        let got: Value = serde_json::from_str(&result.to_json()).unwrap();
        assert_eq!(got, row["validation"], "{relative}: validate");
    }
}
