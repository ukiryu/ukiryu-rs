//! Validation of tool definitions: structural checks plus the JSON Schema
//! stage.
//!
//! Source: `lib/ukiryu/definition/definition_validator.rb`
//! (`Ukiryu::Definition::DefinitionValidator`) at the pinned Ruby oracle
//! (ukiryu/ukiryu ab6df353, json-schema 6.2.1).
//!
//! The stages run in the original order:
//!
//! 1. `validate_string`/`validate_file` load the YAML with
//!    `YAML.safe_load(permitted_classes: [Symbol, Date, Time])`. A
//!    `Psych::SyntaxError` becomes the single error `"Invalid YAML: ..."`;
//!    every other load error is raised.
//! 2. `validate_structure`: the definition must be a Hash, plus the tool name
//!    and `ukiryu_schema` format checks.
//! 3. The JSON Schema stage (`validate_against_schema`), when a schema loads:
//!    the given `schema_path:` only (no fallback), or else `find_schema`
//!    (`UKIRYU_SCHEMA_PATH` when that file exists).
//! 4. The result: valid when there are no errors, with the warnings of both
//!    stages.
//!
//! What this port does not do, and says so explicitly:
//!
//! - The remote schema fallback (`download_and_load_schema` of
//!   [`REMOTE_SCHEMA_URL`]) is not performed. When Ruby would download, the
//!   report says [`SchemaStage::RemoteFallbackNotPerformed`] and no schema
//!   stage runs, which is what Ruby does when that download fails.
//! - Schema dialects and features the schema engine does not reproduce
//!   surface as [`SchemaError::Unsupported`]; they are not skipped.
//! - `schema_validation_available?` is always true here (json-schema is
//!   part of the port), so the "json-schema gem not available" warning is
//!   never produced.
use std::path::Path;

use super::ValidationResult;
use crate::schema::{self, Data, Permitted, SchemaError};

/// `DefinitionValidator::DEFAULT_SCHEMA_VERSION`.
pub const DEFAULT_SCHEMA_VERSION: &str = "1.0";

/// `DefinitionValidator::REMOTE_SCHEMA_URL`: the schema Ruby downloads when
/// no local schema is found. Never fetched by this port.
pub const REMOTE_SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/ukiryu/schemas/refs/heads/main/v1/tool.schema.yaml";

/// Process state `DefinitionValidator` reads, chosen by the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidatorEnv {
    /// `ENV['UKIRYU_SCHEMA_PATH']` (`find_schema`).
    pub schema_path: Option<String>,
}

impl ValidatorEnv {
    /// Reads `UKIRYU_SCHEMA_PATH` from the process environment.
    pub fn from_process() -> Self {
        ValidatorEnv {
            schema_path: std::env::var_os(schema::SCHEMA_PATH_ENV)
                .map(|v| v.to_string_lossy().into_owned()),
        }
    }
}

/// Errors and warnings of one stage (`{ errors:, warnings: }` in Ruby).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageIssues {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Where the JSON Schema stage got its schema, or why it did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaStage {
    /// `schema_path:` was given and `load_schema` returned a schema.
    Given { path: String },
    /// `schema_path:` was given and `load_schema` returned `nil` (missing
    /// file, unknown extension, parse error) or a falsy document. Ruby
    /// skips the stage; it does not fall back.
    GivenNotLoaded { path: String },
    /// No `schema_path:`; `find_schema` returned `UKIRYU_SCHEMA_PATH` and it
    /// loaded.
    Env { path: String },
    /// No `schema_path:` and no loadable `UKIRYU_SCHEMA_PATH`: Ruby calls
    /// `download_and_load_schema` ([`REMOTE_SCHEMA_URL`]). Not performed by
    /// this port; the stage is skipped, as in Ruby when the download fails.
    RemoteFallbackNotPerformed,
}

impl SchemaStage {
    /// Whether the JSON Schema stage ran.
    pub fn ran(&self) -> bool {
        matches!(self, SchemaStage::Given { .. } | SchemaStage::Env { .. })
    }
}

/// The result plus what each stage contributed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationReport {
    /// What `DefinitionValidator` returns.
    pub result: ValidationResult,
    /// `validate_structure`; `None` when the YAML did not parse.
    pub structure: Option<StageIssues>,
    /// How the schema was resolved; `None` when the YAML did not parse.
    pub schema_stage: Option<SchemaStage>,
    /// `validate_against_schema`, when the schema stage ran.
    pub schema: Option<StageIssues>,
}

/// `DefinitionValidator.find_schema`: `UKIRYU_SCHEMA_PATH` when that file
/// exists.
pub fn find_schema(env: &ValidatorEnv) -> Option<String> {
    env.schema_path
        .as_deref()
        .filter(|p| Path::new(p).exists())
        .map(str::to_owned)
}

/// `DefinitionValidator.load_schema(path)`; see
/// [`schema::load_definition_schema`].
pub fn load_schema(path: &str) -> Result<Option<Data>, SchemaError> {
    schema::load_definition_schema(Path::new(path))
}

/// `definition[:key] || definition['key']` (a truthy value or `None`).
fn hash_get<'a>(pairs: &'a [(Data, Data)], key: &str) -> Option<&'a Data> {
    fn last<'a>(pairs: &'a [(Data, Data)], key: &str, symbol: bool) -> Option<&'a Data> {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| match k {
                Data::Symbol(s) => symbol && s == key,
                Data::String(s) => !symbol && s == key,
                _ => false,
            })
            .map(|(_, v)| v)
            .filter(|v| v.truthy())
    }
    last(pairs, key, true).or_else(|| last(pairs, key, false))
}

/// `definition.key?(:key) || definition.key?('key')`.
fn has_key(pairs: &[(Data, Data)], key: &str) -> bool {
    pairs
        .iter()
        .any(|(k, _)| matches!(k, Data::Symbol(s) | Data::String(s) if s == key))
}

/// Ruby regexp `\s`.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// Ruby `^...$` (multi-line anchors): whether any line satisfies `line_ok`.
fn any_line(s: &str, line_ok: impl Fn(&str) -> bool) -> bool {
    s.split('\n').any(line_ok)
}

/// `DefinitionValidator.validate_structure(definition)`.
pub fn validate_structure(definition: &Data) -> Result<StageIssues, SchemaError> {
    let mut issues = StageIssues::default();
    let Data::Hash(pairs) = definition else {
        issues
            .errors
            .push("Definition must be a hash/object".to_owned());
        return Ok(issues);
    };
    if let Some(name) = hash_get(pairs, "name") {
        let name = name.ruby_to_s()?;
        if name.chars().any(is_ruby_space) {
            issues
                .errors
                .push("Tool name must not contain whitespace".to_owned());
        }
        if any_line(&name, |l| l.starts_with(|c: char| c.is_ascii_digit())) {
            issues
                .warnings
                .push("Tool name starting with number may cause issues".to_owned());
        }
        let lower_ok = |l: &str| {
            !l.is_empty()
                && l.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        };
        if !any_line(&name, lower_ok) {
            issues.warnings.push(
                "Tool name should contain only lowercase letters, numbers, hyphens, and underscores"
                    .to_owned(),
            );
        }
    }
    if has_key(pairs, "ukiryu_schema") {
        let value = hash_get(pairs, "ukiryu_schema")
            .cloned()
            .unwrap_or_else(|| {
                // `nil || definition['ukiryu_schema']` keeps a falsy String-key value.
                pairs
                    .iter()
                    .rev()
                    .find(|(k, _)| matches!(k, Data::String(s) if s == "ukiryu_schema"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Data::Null)
            });
        let text = value.ruby_to_s()?;
        let format_ok = |l: &str| {
            let mut parts = l.splitn(2, '.');
            let a = parts.next().unwrap_or("");
            let b = parts.next().unwrap_or("");
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        };
        if !any_line(&text, format_ok) {
            issues.warnings.push(format!(
                "Invalid schema version format: {text} (expected format: '1.0')"
            ));
        }
    }
    Ok(issues)
}

/// `DefinitionValidator.validate_against_schema(definition, schema)`; see
/// [`schema::definition_schema_report`].
pub fn validate_against_schema(
    definition: &Data,
    schema: &Data,
) -> Result<StageIssues, SchemaError> {
    let report = schema::definition_schema_report(definition, schema)?;
    Ok(StageIssues {
        errors: report.errors,
        warnings: report.warnings,
    })
}

/// The schema of `validate`: `schema_path ? load_schema(schema_path) :
/// find_and_load_schema`, keeping only a truthy document.
fn resolve_schema(
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<(SchemaStage, Option<Data>), SchemaError> {
    if let Some(path) = schema_path {
        return Ok(match load_schema(path)?.filter(Data::truthy) {
            Some(schema) => (
                SchemaStage::Given {
                    path: path.to_owned(),
                },
                Some(schema),
            ),
            None => (
                SchemaStage::GivenNotLoaded {
                    path: path.to_owned(),
                },
                None,
            ),
        });
    }
    if let Some(path) = find_schema(env) {
        if let Some(schema) = load_schema(&path)?.filter(Data::truthy) {
            return Ok((SchemaStage::Env { path }, Some(schema)));
        }
    }
    Ok((SchemaStage::RemoteFallbackNotPerformed, None))
}

fn assemble(errors: Vec<String>, warnings: Vec<String>) -> ValidationResult {
    if errors.is_empty() {
        if warnings.is_empty() {
            ValidationResult::success()
        } else {
            ValidationResult::with_warnings(warnings)
        }
    } else {
        ValidationResult::failure(errors, warnings)
    }
}

/// `DefinitionValidator.validate(definition, schema_path:)` with the
/// contribution of each stage.
pub fn validate_report(
    definition: &Data,
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<ValidationReport, SchemaError> {
    let structure = validate_structure(definition)?;
    let (stage, schema) = resolve_schema(schema_path, env)?;
    let schema_issues = match &schema {
        Some(schema) => Some(validate_against_schema(definition, schema)?),
        None => None,
    };
    let mut errors = structure.errors.clone();
    let mut warnings = structure.warnings.clone();
    if let Some(issues) = &schema_issues {
        errors.extend(issues.errors.iter().cloned());
        warnings.extend(issues.warnings.iter().cloned());
    }
    Ok(ValidationReport {
        result: assemble(errors, warnings),
        structure: Some(structure),
        schema_stage: Some(stage),
        schema: schema_issues,
    })
}

/// `DefinitionValidator.validate(definition, schema_path:)`.
pub fn validate(
    definition: &Data,
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<ValidationResult, SchemaError> {
    validate_report(definition, schema_path, env).map(|r| r.result)
}

fn invalid_yaml(message: String) -> ValidationReport {
    ValidationReport {
        result: ValidationResult::failure(vec![format!("Invalid YAML: {message}")], Vec::new()),
        structure: None,
        schema_stage: None,
        schema: None,
    }
}

/// `DefinitionValidator.validate_string(yaml_string, schema_path:)` with
/// the contribution of each stage.
pub fn validate_string_report(
    yaml: &str,
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<ValidationReport, SchemaError> {
    match schema::safe_load(yaml, Permitted::DOCUMENT) {
        Ok(definition) => validate_report(&definition, schema_path, env),
        Err(SchemaError::YamlSyntax { message }) => Ok(invalid_yaml(message)),
        Err(e) => Err(e),
    }
}

/// `DefinitionValidator.validate_string(yaml_string, schema_path:)`.
pub fn validate_string(
    yaml: &str,
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<ValidationResult, SchemaError> {
    validate_string_report(yaml, schema_path, env).map(|r| r.result)
}

/// `File.read(file_path)` errors the original raises (`Errno::ENOENT` is
/// rescued by the caller).
fn read_error(e: &std::io::Error, path: &str) -> SchemaError {
    let io = |errno: &str, message: String| SchemaError::Io {
        errno: errno.to_owned(),
        message,
    };
    match e.kind() {
        std::io::ErrorKind::IsADirectory => {
            io("EISDIR", format!("Is a directory @ io_fread - {path}"))
        }
        std::io::ErrorKind::NotADirectory => {
            io("ENOTDIR", format!("Not a directory @ rb_sysopen - {path}"))
        }
        std::io::ErrorKind::PermissionDenied => {
            io("EACCES", format!("Permission denied @ rb_sysopen - {path}"))
        }
        _ => SchemaError::unsupported("definition file read", format!("reading {path}: {e}")),
    }
}

/// `DefinitionValidator.validate_file(file_path, schema_path:)` with the
/// contribution of each stage.
pub fn validate_file_report(
    path: &str,
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<ValidationReport, SchemaError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ValidationReport {
                result: ValidationResult::failure(
                    vec![format!("File not found: {path}")],
                    Vec::new(),
                ),
                structure: None,
                schema_stage: None,
                schema: None,
            })
        }
        Err(e) => return Err(read_error(&e, path)),
    };
    let text = String::from_utf8(bytes).map_err(|_| {
        SchemaError::unsupported("definition file encoding", format!("{path} is not UTF-8"))
    })?;
    validate_string_report(&text, schema_path, env)
}

/// `DefinitionValidator.validate_file(file_path, schema_path:)`.
pub fn validate_file(
    path: &str,
    schema_path: Option<&str>,
    env: &ValidatorEnv,
) -> Result<ValidationResult, SchemaError> {
    validate_file_report(path, schema_path, env).map(|r| r.result)
}
