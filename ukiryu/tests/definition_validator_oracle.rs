//! Golden parity for `ukiryu::definition::definition_validator` against
//! `Ukiryu::Definition::DefinitionValidator` (ukiryu/ukiryu ab6df353,
//! json-schema 6.2.1).
//!
//! `tests/definition_validator_data/cases.json` was captured by running the
//! pinned oracle (see its `provenance` block). Each case records the public
//! result (`to_h`, or the raised class and message) and what each stage
//! produced: `validate_structure`, how the schema resolved (including an
//! observed call of the remote fallback, stubbed as a failed download) and
//! `validate_against_schema`. Every case must match exactly, including YAML
//! syntax messages and the timestamp-name outcome captured under TZ=UTC.

use serde_json::Value;
use std::path::PathBuf;
use ukiryu::definition::definition_validator::{
    self, SchemaStage, StageIssues, ValidationReport, ValidatorEnv,
};
use ukiryu::schema::SchemaError;

/// Cases whose oracle behaviour is platform specific (captured on Linux).
const UNIX_ONLY: &[&str] = &["file.directory"];

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn root() -> String {
    tests_dir().to_string_lossy().into_owned()
}

fn abs(rel: &str) -> String {
    format!("{}/{rel}", root())
}

fn golden() -> Value {
    let path = tests_dir().join("definition_validator_data/cases.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden file")).expect("json")
}

fn scrub(s: &str) -> String {
    s.replace(&root(), "{root}")
}

fn strings(v: &[String]) -> Value {
    Value::Array(v.iter().map(|s| Value::String(scrub(s))).collect())
}

fn issues(i: &StageIssues) -> Value {
    serde_json::json!({ "errors": strings(&i.errors), "warnings": strings(&i.warnings) })
}

fn stage_label(s: &SchemaStage) -> &'static str {
    match s {
        SchemaStage::Given { .. } => "given",
        SchemaStage::GivenNotLoaded { .. } => "given_not_loaded",
        SchemaStage::Env { .. } => "env",
        SchemaStage::RemoteFallbackNotPerformed => "remote_fallback",
        SchemaStage::RemoteFallback { .. } => "remote_fallback",
        SchemaStage::Supplied => "supplied",
        SchemaStage::SuppliedNotLoaded => "supplied_not_loaded",
    }
}

/// The capture's shape of a report.
fn report_value(r: &ValidationReport) -> Value {
    let res = &r.result;
    serde_json::json!({
        "ok": {
            "valid": res.is_valid(),
            "errors": strings(&res.errors),
            "warnings": strings(&res.warnings),
            "schema_path": res.schema_path,
            "error_count": res.error_count(),
            "warning_count": res.warning_count(),
        },
        "stages": {
            "structure": r.structure.as_ref().map(issues),
            "schema_stage": r.schema_stage.as_ref().map(stage_label),
            "schema": r.schema.as_ref().map(issues),
        }
    })
}

fn run(input: &Value) -> Result<ValidationReport, SchemaError> {
    let env = ValidatorEnv {
        remote_schema: definition_validator::RemoteSchemaSource::Unavailable,
        schema_path: input["env"]
            .as_str()
            .map(abs)
            .or_else(|| input["env_literal"].as_str().map(str::to_owned)),
    };
    let schema = input["schema"]
        .as_str()
        .map(|s| if s.is_empty() { String::new() } else { abs(s) });
    match (input["file"].as_str(), input["yaml"].as_str()) {
        (Some(file), _) => {
            definition_validator::validate_file_report(&abs(file), schema.as_deref(), &env)
        }
        (None, Some(yaml)) => {
            definition_validator::validate_string_report(yaml, schema.as_deref(), &env)
        }
        _ => panic!("case without yaml or file: {input}"),
    }
}

/// Compare a captured and an actual case; `None` when they agree.
fn compare(want: &Value, got: &Result<ValidationReport, SchemaError>) -> Option<String> {
    match got {
        Ok(r) => {
            if want.get("error").is_some() {
                return Some(format!(
                    "expected raise {}, got {}",
                    want["error"],
                    report_value(r)
                ));
            }
            let have = report_value(r);
            let exp = serde_json::json!({ "ok": want["ok"], "stages": want["stages"] });
            (have != exp).then(|| format!("got  {have}\nwant {exp}"))
        }
        Err(e) => {
            let Some(class) = e.ruby_class() else {
                return Some(format!("unsupported: {e}"));
            };
            let have = serde_json::json!({ "class": class, "message": scrub(&e.message()) });
            (have != want["error"]).then(|| format!("raised {have}, want {}", want["error"]))
        }
    }
}

#[test]
fn definition_validator_matches_the_oracle() {
    // Match the capture's timezone without changing the environment read by
    // other tests running in this process.
    if std::env::var_os("UKIRYU_VALIDATOR_ORACLE_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "definition_validator_matches_the_oracle",
                "--nocapture",
            ])
            .env("TZ", "UTC")
            .env("UKIRYU_VALIDATOR_ORACLE_CHILD", "1")
            .status()
            .expect("UTC oracle subprocess");
        assert!(status.success(), "UTC oracle subprocess: {status}");
        return;
    }
    let golden = golden();
    assert_eq!(
        golden["provenance"]["oracle"],
        "ukiryu/ukiryu ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    let cases = golden["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 70, "golden case count changed");
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in cases {
        let id = case["id"].as_str().expect("id");
        if UNIX_ONLY.contains(&id) && !cfg!(unix) {
            continue;
        }
        checked += 1;
        if let Some(why) = compare(case, &run(&case["input"])) {
            failures.push(format!("{id}: {why}"));
        }
    }
    assert!(checked >= 69, "only {checked} cases checked");
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// The comparison must reject a wrong result, a wrong stage and a wrong
/// raise, so a broken validator cannot pass by accident.
#[test]
fn oracle_comparison_rejects_altered_expectations() {
    let golden = golden();
    let cases = golden["cases"].as_array().expect("cases");
    let mut rejected = 0;
    for case in cases {
        let id = case["id"].as_str().expect("id");
        if divergence(id).is_some() || (UNIX_ONLY.contains(&id) && !cfg!(unix)) {
            continue;
        }
        let got = run(&case["input"]);
        let mut altered = case.clone();
        if altered.get("error").is_some() {
            altered["error"]["message"] = Value::String("altered".into());
        } else {
            let valid = altered["ok"]["valid"].as_bool().expect("valid");
            altered["ok"]["valid"] = Value::Bool(!valid);
            let mut stage = case.clone();
            stage["stages"]["schema_stage"] = Value::String("altered".into());
            assert!(
                compare(id, &stage, &got).is_some(),
                "{id}: altered stage accepted"
            );
        }
        assert!(
            compare(id, &altered, &got).is_some(),
            "{id}: altered result accepted"
        );
        rejected += 1;
    }
    assert!(rejected >= 60, "only {rejected} controls ran");
}

/// An explicitly host-supplied unavailable remote response: with neither a schema
/// path nor a loadable `UKIRYU_SCHEMA_PATH` the schema stage does not run.
#[test]
fn remote_fallback_is_explicit() {
    let report = definition_validator::validate_string_report(
        "name: tool\n",
        None,
        &ValidatorEnv {
            remote_schema: definition_validator::RemoteSchemaSource::Unavailable,
            ..ValidatorEnv::default()
        },
    )
    .expect("report");
    assert_eq!(
        report.schema_stage,
        Some(SchemaStage::RemoteFallbackNotPerformed)
    );
    assert!(report.schema.is_none());
    assert!(!SchemaStage::RemoteFallbackNotPerformed.ran());
    assert_eq!(
        definition_validator::REMOTE_SCHEMA_URL,
        "https://raw.githubusercontent.com/ukiryu/schemas/refs/heads/main/v1/tool.schema.yaml"
    );
}
