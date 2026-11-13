//! The `schema.definition_validate` rows of the conformance corpus
//! (`corpus/conformance/cases/schema.jsonl`) through the core
//! `definition_validator::validate_string_report`.
//!
//! Each row is `DefinitionValidator.validate_string(File.read(file),
//! schema_path: schema).to_h` captured from the pinned oracle. The row's
//! inputs go to the core API unchanged; the expected value is only compared,
//! never used to produce the result.

use serde_json::Value;
use std::path::PathBuf;
use ukiryu::definition::definition_validator::{self, SchemaStage, ValidationReport};
use ukiryu::definition::ValidatorEnv;
use ukiryu::schema::SchemaError;

const OP: &str = "schema.definition_validate";
const ROWS: usize = 58;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace")
        .join("corpus/conformance")
}

fn rows() -> Vec<Value> {
    let text = std::fs::read_to_string(corpus_dir().join("cases/schema.jsonl")).expect("cases");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("row"))
        .filter(|r| r["op"] == OP)
        .collect()
}

fn input_path(rel: &str) -> String {
    corpus_dir()
        .join("inputs")
        .join(rel)
        .to_string_lossy()
        .into_owned()
}

fn sym(name: &str) -> Value {
    serde_json::json!({ "$sym": name })
}

/// `ValidationResult#to_h` in the corpus encoding (Symbol keys, in order).
fn to_h(r: &ValidationReport) -> Value {
    let res = &r.result;
    let pairs = vec![
        (sym("valid"), Value::Bool(res.is_valid())),
        (sym("errors"), serde_json::json!(res.errors)),
        (sym("warnings"), serde_json::json!(res.warnings)),
        (sym("schema_path"), serde_json::json!(res.schema_path)),
        (sym("error_count"), serde_json::json!(res.error_count())),
        (sym("warning_count"), serde_json::json!(res.warning_count())),
    ];
    serde_json::json!({
        "$map": pairs.into_iter().map(|(k, v)| Value::Array(vec![k, v])).collect::<Vec<_>>()
    })
}

/// The row's outcome in the corpus `expect` shape.
fn outcome(got: &Result<ValidationReport, SchemaError>) -> Result<Value, String> {
    match got {
        Ok(r) => Ok(serde_json::json!({ "ok": to_h(r) })),
        Err(e) => match e.ruby_class() {
            Some(class) => Ok(serde_json::json!({
                "error": { "class": class, "message": e.message() }
            })),
            None => Err(format!("core reports unsupported: {e}")),
        },
    }
}

fn run(
    row: &Value,
    schema_override: Option<Option<&str>>,
) -> Result<ValidationReport, SchemaError> {
    let input = &row["input"];
    let file = input["file"].as_str().expect("file");
    let text = std::fs::read_to_string(input_path(file)).expect("input file");
    let schema = match schema_override {
        Some(s) => s.map(str::to_owned),
        None => input["schema"].as_str().map(input_path),
    };
    definition_validator::validate_string_report(&text, schema.as_deref(), &ValidatorEnv::default())
}

#[test]
fn every_definition_validate_row_matches_through_the_core() {
    let rows = rows();
    assert_eq!(rows.len(), ROWS, "row count of {OP} changed");
    let mut failures = Vec::new();
    let mut schema_stage_rows = 0;
    for row in &rows {
        let id = row["id"].as_str().expect("id");
        assert_eq!(
            row["api"], "Ukiryu::Definition::DefinitionValidator.validate_string",
            "{id}"
        );
        let got = run(row, None);
        match outcome(&got) {
            Ok(have) if have == row["expect"] => {}
            Ok(have) => failures.push(format!("{id}:\n got  {have}\n want {}", row["expect"])),
            Err(why) => failures.push(format!("{id}: {why}")),
        }
        // Stages: the result is the structural issues followed by the schema
        // stage's, and the schema stage runs exactly when the schema loads.
        if let Ok(report) = &got {
            let structure = report.structure.as_ref().expect("structure stage");
            let mut errors = structure.errors.clone();
            let mut warnings = structure.warnings.clone();
            match (&report.schema_stage, &report.schema) {
                (Some(SchemaStage::Given { .. }), Some(stage)) => {
                    schema_stage_rows += 1;
                    errors.extend(stage.errors.iter().cloned());
                    warnings.extend(stage.warnings.iter().cloned());
                }
                (Some(SchemaStage::GivenNotLoaded { .. }), None) => {}
                other => failures.push(format!("{id}: unexpected schema stage {other:?}")),
            }
            if errors != report.result.errors || warnings != report.result.warnings {
                failures.push(format!("{id}: result is not structure + schema stage"));
            }
            let schema = row["input"]["schema"].as_str().expect("schema");
            // The fixture schema loads; the original v1 tool schema is not
            // valid YAML (Psych::SyntaxError, rescued by load_schema), so the
            // oracle skips the schema stage for it.
            let want_ran = schema.starts_with("fixture-schema/");
            if report.schema_stage.as_ref().map(SchemaStage::ran) != Some(want_ran) {
                failures.push(format!("{id}: schema stage ran != {want_ran}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {ROWS} rows differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(schema_stage_rows, 29, "rows that ran the schema stage");
}

/// Control: the comparison rejects every row when its expected value is
/// altered (validity flipped, or one more error), so a matching run is not
/// an artefact of a lenient comparison.
#[test]
fn altered_expectations_are_rejected() {
    for row in rows() {
        let id = row["id"].as_str().expect("id");
        let have = outcome(&run(&row, None)).expect("outcome");
        let pairs = row["expect"]["ok"]["$map"].as_array().expect("map");
        let mut flipped = pairs.clone();
        let valid = flipped[0][1].as_bool().expect("valid");
        flipped[0][1] = Value::Bool(!valid);
        let mut extra = pairs.clone();
        extra[1][1]
            .as_array_mut()
            .expect("errors")
            .push(Value::String("extra".into()));
        for altered in [flipped, extra] {
            let want = serde_json::json!({ "ok": { "$map": altered } });
            assert_ne!(have, want, "{id}: altered expectation accepted");
        }
    }
}

/// Control: the rows depend on the schema stage. Without the schema the
/// rows that fail only on schema errors become valid; against a stricter
/// draft-04 schema (`additionalProperties: false`) every row becomes invalid.
#[test]
fn rows_depend_on_the_schema_stage() {
    let strict = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/schema_golden/inputs/synthetic/draft4_basic.yaml")
        .to_string_lossy()
        .into_owned();
    let mut changed_without_schema = 0;
    for row in rows() {
        let id = row["id"].as_str().expect("id");
        let expected = outcome(&run(&row, None)).expect("outcome");
        let expected_invalid = row["expect"]["ok"]["$map"][0][1] == Value::Bool(false);
        let bare = run(&row, Some(None)).expect("no schema");
        assert_eq!(
            bare.schema_stage,
            Some(SchemaStage::RemoteFallbackNotPerformed),
            "{id}"
        );
        if expected_invalid {
            assert!(bare.result.is_valid(), "{id}: invalid without a schema");
            assert_ne!(
                outcome(&Ok(bare)).expect("outcome"),
                expected,
                "{id}: same result without the schema stage"
            );
            changed_without_schema += 1;
        }
        let tightened = run(&row, Some(Some(&strict))).expect("strict schema");
        assert!(
            tightened.result.is_invalid(),
            "{id}: valid against additionalProperties: false"
        );
        assert!(
            tightened.schema.is_some(),
            "{id}: strict schema stage did not run"
        );
    }
    assert_eq!(
        changed_without_schema, 27,
        "rows invalid only through the schema"
    );
}
