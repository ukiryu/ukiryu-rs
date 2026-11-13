//! `lint` and `schema` kinds: `lint.string` through
//! `DefinitionLinter::lint_string`, `schema.validate_profile` through
//! `schema::safe_load` + `schema::validate_profile`, and
//! `schema.definition_validate`, which has no core entrypoint yet.
use std::path::PathBuf;

use ukiryu::definition::linter::{DefinitionLinter, LintError, LintResult};
use ukiryu::definition::LintIssueHash;
use ukiryu::schema::{self, safe_load, Permitted, SchemaError, ValidateOptions};

use super::harness::{Ctx, Outcome, RowError};
use super::json::Oj;

/// The oracle's `read_input`: inline `yaml` when given, else the `file`.
fn read_text(input: &Oj, ctx: &Ctx) -> Result<String, RowError> {
    match input.get("yaml") {
        Some(Oj::Str(s)) => Ok(s.clone()),
        Some(other) => Err(RowError::harness(format!(
            "yaml must be a string, got {other:?}"
        ))),
        None => ctx.read_input(input.field_str("file").map_err(RowError::harness)?),
    }
}

/// A Ruby Hash with Symbol keys, in insertion order.
fn sym_map(pairs: Vec<(&str, Oj)>) -> Oj {
    Oj::tagged(
        "$map",
        Oj::Arr(
            pairs
                .into_iter()
                .map(|(k, v)| Oj::Arr(vec![Oj::sym(k), v]))
                .collect(),
        ),
    )
}

fn opt_str(s: &Option<String>) -> Oj {
    match s {
        Some(s) => Oj::s(s.as_str()),
        None => Oj::Null,
    }
}

fn int(n: usize) -> Oj {
    Oj::Num(n.to_string())
}

fn issue_hash(h: &LintIssueHash) -> Oj {
    sym_map(vec![
        ("severity", Oj::sym(h.severity.as_str())),
        ("severity_string", Oj::s(h.severity_string.as_str())),
        ("message", Oj::s(h.message.as_str())),
        ("location", opt_str(&h.location)),
        ("suggestion", opt_str(&h.suggestion)),
        ("rule_id", opt_str(&h.rule_id)),
    ])
}

/// `LintResult#to_h`.
fn lint_result_hash(result: &LintResult) -> Oj {
    let h = result.to_h();
    sym_map(vec![
        ("issues", Oj::Arr(h.issues.iter().map(issue_hash).collect())),
        ("total_count", int(h.total_count)),
        ("error_count", int(h.error_count)),
        ("warning_count", int(h.warning_count)),
        ("info_count", int(h.info_count)),
        ("style_count", int(h.style_count)),
    ])
}

fn lint_error(e: LintError) -> RowError {
    match e.ruby_class() {
        Some(class) => RowError::new(class, e.message()),
        None => RowError::harness(e.message()),
    }
}

fn schema_error(e: SchemaError) -> RowError {
    match e.ruby_class() {
        Some(class) => RowError::new(class, e.message()),
        None => RowError::harness(e.message()),
    }
}

/// `DefinitionLinter.lint_string(text).to_h`.
pub fn lint_string(input: &Oj, ctx: &Ctx) -> Outcome {
    let text = read_text(input, ctx)?;
    let result = DefinitionLinter::lint_string(&text, None).map_err(lint_error)?;
    Ok(lint_result_hash(&result))
}

/// `SchemaValidator.validate_profile(YAML.safe_load(text, permitted_classes:
/// [Symbol, Date, Time]), schema_path: i['schema'] && input_path(...))`.
pub fn validate_profile(input: &Oj, ctx: &Ctx) -> Outcome {
    let text = read_text(input, ctx)?;
    let schema_path = match input.get("schema") {
        None | Some(Oj::Null) => None,
        Some(Oj::Str(rel)) => Some(PathBuf::from(ctx.input_path(rel))),
        Some(other) => {
            return Err(RowError::harness(format!(
                "schema must be a string or null, got {other:?}"
            )))
        }
    };
    let profile = safe_load(&text, Permitted::DOCUMENT).map_err(schema_error)?;
    let messages = schema::validate_profile(
        &profile,
        &ValidateOptions {
            schema_path,
            strict: false,
        },
    )
    .map_err(schema_error)?;
    Ok(Oj::Arr(messages.into_iter().map(Oj::Str).collect()))
}

/// `DefinitionValidator.validate_string(text, schema_path:).to_h`.
///
/// The core ports only the JSON Schema stage of `DefinitionValidator`
/// (`schema::load_definition_schema` + `schema::definition_schema_report`);
/// `validate_string`/`validate` (YAML rescue, `validate_structure`, result
/// assembly) has no core API, so the row fails visibly.
pub fn definition_validate(_input: &Oj, _ctx: &Ctx) -> Outcome {
    Err(RowError::harness(
        "no core API: Ukiryu::Definition::DefinitionValidator.validate_string \
         (only schema::load_definition_schema/definition_schema_report are ported; \
         validate_structure and result assembly are not)",
    ))
}
