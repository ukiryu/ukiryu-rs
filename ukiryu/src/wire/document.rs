//! Definition documents: `lint` (`DefinitionLinter`) and `yaml` (the core's
//! Ruby-semantics YAML readers and the model loaders and emitter).
//!
//! Unlike `parse_yaml`, which is a JSON diagnostic profile, these replies
//! carry Ruby values in the typed [`value`](super::value) encoding: Symbols,
//! Floats and non-String Hash keys survive.
use num_bigint::BigInt;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use super::error::{model_error, ruby, schema_error, transport};
use super::value::encode;
use crate::definition::linter::{DefinitionLinter, LintResult};
use crate::definition::LintIssueHash;
use crate::models::{Model, ToolDefinition};
use crate::schema::{self, Data, Permitted, SchemaError};
use crate::types::Value;

// ------------------------------------------------------------------- lint

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum LintRequest {
    /// `DefinitionLinter.lint_string(yaml).to_h`.
    String { yaml: String },
}

pub fn lint(request: LintRequest) -> Json {
    match request {
        LintRequest::String { yaml } => match DefinitionLinter::lint_string(&yaml, None) {
            Ok(result) => json!({"ok": true, "value": encode(&lint_hash(&result))}),
            Err(error) => match error.ruby_class() {
                Some(class) => ruby(class, error.message()),
                None => transport("NOT_PORTED", error.message()),
            },
        },
    }
}

fn sym(name: &str) -> Value {
    Value::Symbol(name.to_owned())
}

fn sym_hash(pairs: Vec<(&str, Value)>) -> Value {
    Value::Hash(pairs.into_iter().map(|(k, v)| (sym(k), v)).collect())
}

fn opt_str(s: &Option<String>) -> Value {
    s.as_ref().map_or(Value::Nil, |s| Value::String(s.clone()))
}

fn count(n: usize) -> Value {
    Value::Integer(BigInt::from(n))
}

fn issue_hash(h: &LintIssueHash) -> Value {
    sym_hash(vec![
        ("severity", sym(&h.severity)),
        ("severity_string", Value::String(h.severity_string.clone())),
        ("message", Value::String(h.message.clone())),
        ("location", opt_str(&h.location)),
        ("suggestion", opt_str(&h.suggestion)),
        ("rule_id", opt_str(&h.rule_id)),
    ])
}

/// `LintResult#to_h`: a Symbol-keyed Hash in the source's key order.
fn lint_hash(result: &LintResult) -> Value {
    let h = result.to_h();
    sym_hash(vec![
        (
            "issues",
            Value::Array(h.issues.iter().map(issue_hash).collect()),
        ),
        ("total_count", count(h.total_count)),
        ("error_count", count(h.error_count)),
        ("warning_count", count(h.warning_count)),
        ("info_count", count(h.info_count)),
        ("style_count", count(h.style_count)),
    ])
}

// ------------------------------------------------------------------- yaml

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum YamlRequest {
    /// `YAML.safe_load(yaml, permitted_classes: [Symbol, Date, Time])`.
    SafeLoad { yaml: String },
    /// `Ukiryu::Models::<model>.from_yaml(path)` for the plain register
    /// models (`Interface`, `ImplementationVersion`, `ImplementationIndex`),
    /// which read the file at `path`. The value is the object's class name
    /// and its instance variables sorted by name (`instance_variables.sort`).
    #[cfg(not(target_arch = "wasm32"))]
    ModelLoad { model: String, path: String },
    /// `Ukiryu::Models::<model>.from_yaml(yaml).to_yaml` for a lutaml model
    /// (`ToolDefinition`), emitted by the core's Yeptris engine.
    ModelToYaml { model: String, yaml: String },
}

pub fn yaml(request: YamlRequest) -> Json {
    let value = match request {
        YamlRequest::SafeLoad { yaml } => schema::safe_load(&yaml, Permitted::DOCUMENT)
            .map_err(|e| reader_error(&e, "document"))
            .and_then(|data| data_value(&data))
            .map(|v| encode(&v)),
        #[cfg(not(target_arch = "wasm32"))]
        YamlRequest::ModelLoad { model, path } => snapshot::load(&model, &path),
        YamlRequest::ModelToYaml { model, yaml } => match model.as_str() {
            "ToolDefinition" => ToolDefinition::from_yaml(&yaml)
                .and_then(|m| m.to_yaml())
                .map(Json::String)
                .map_err(|e| model_error(&e)),
            other => Err(unknown_model(other)),
        },
    };
    match value {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => error,
    }
}

/// A core reader failure with `detail`: the failure category and the
/// stage that failed. The message is the core reader's own diagnostic, so
/// callers compare these facts rather than the original's Psych wording.
fn reader_error(error: &SchemaError, stage: &str) -> Json {
    let mut reply = schema_error(error);
    let category = match error {
        SchemaError::YamlSyntax { .. } => "YamlSyntax",
        _ => "Other",
    };
    reply["error"]["detail"] = json!({"category": category, "stage": stage});
    reply
}

fn unknown_model(name: &str) -> Json {
    transport(
        "INVALID_ARGUMENT",
        format!("no model operation for Ukiryu::Models::{name}"),
    )
}

/// A `safe_load` result as a Ruby value. The core keeps Date and Time as
/// their YAML spelling, which has no wire value, so those fail loudly.
fn data_value(d: &Data) -> Result<Value, Json> {
    Ok(match d {
        Data::Null => Value::Nil,
        Data::Bool(b) => Value::Bool(*b),
        Data::Integer(i) => Value::Integer(i.clone()),
        Data::Float(f) => Value::Float(*f),
        Data::String(s) => Value::String(s.clone()),
        Data::Symbol(s) => Value::Symbol(s.clone()),
        Data::Date { .. } => {
            return Err(transport("NOT_PORTED", "a YAML Date has no wire value"));
        }
        Data::Time(raw) => {
            return Err(transport(
                "NOT_PORTED",
                format!("a YAML Time ({raw:?}) has no wire value"),
            ));
        }
        Data::Array(items) => Value::Array(items.iter().map(data_value).collect::<Result<_, _>>()?),
        Data::Hash(pairs) => Value::Hash(
            pairs
                .iter()
                .map(|(k, v)| Ok((data_value(k)?, data_value(v)?)))
                .collect::<Result<_, Json>>()?,
        ),
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod snapshot {
    use std::path::Path;

    use serde_json::{json, Value as Json};

    use super::super::error::model_error;
    use super::super::value::encode;
    use super::unknown_model;
    use crate::models::{
        ImplementationIndex, ImplementationVersion, Interface, Text, VersionDetection,
        VersionDetectionMethod,
    };

    pub fn load(model: &str, path: &str) -> Result<Json, Json> {
        let path = Path::new(path);
        let loaded = match model {
            "Interface" => Interface::from_yaml(path).map(|i| interface(&i)),
            "ImplementationVersion" => ImplementationVersion::from_yaml(path).map(|v| version(&v)),
            "ImplementationIndex" => ImplementationIndex::from_yaml(path).map(|i| index(&i)),
            other => return Err(unknown_model(other)),
        };
        loaded.map_err(|e| model_error(&e))
    }

    /// `{"class": name, "ivars": {...}}`; `ivars` are given sorted by name.
    fn object(class: &str, ivars: Vec<(&str, Json)>) -> Json {
        let ivars: serde_json::Map<String, Json> =
            ivars.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        json!({"class": class, "ivars": ivars})
    }

    fn interface(i: &Interface) -> Json {
        object(
            "Ukiryu::Models::Interface",
            vec![
                ("@actions", encode(&i.actions)),
                (
                    "@aliases",
                    Json::Array(i.aliases.iter().map(|a| json!({"$sym": a})).collect()),
                ),
                ("@display_name", encode(&i.display_name)),
                ("@name", encode(&i.name)),
                ("@version", encode(&i.version)),
            ],
        )
    }

    fn version(v: &ImplementationVersion) -> Json {
        object(
            "Ukiryu::Models::ImplementationVersion",
            vec![
                ("@aliases", json!(v.aliases)),
                ("@display_name", encode(&v.display_name)),
                ("@execution_profiles", encode(&v.execution_profiles)),
                ("@implements", encode(&v.implements)),
                ("@version", encode(&v.version)),
                (
                    "@version_detection",
                    v.version_detection.as_ref().map_or(Json::Null, detection),
                ),
            ],
        )
    }

    fn index(i: &ImplementationIndex) -> Json {
        object(
            "Ukiryu::Models::ImplementationIndex",
            vec![
                (
                    "@implementations",
                    Json::Array(i.implementations.iter().map(encode).collect()),
                ),
                ("@interface", encode(&i.interface)),
                ("@interface_version", encode(&i.interface_version)),
                ("@name", encode(&i.name)),
            ],
        )
    }

    fn text(t: Option<&Text>) -> Json {
        t.map_or(Json::Null, |t| encode(&t.to_ruby_value()))
    }

    /// The model's declared attributes as the core holds them; lutaml's own
    /// bookkeeping instance variables have no counterpart in the core.
    fn detection(vd: &VersionDetection) -> Json {
        object(
            "Ukiryu::Models::VersionDetection",
            vec![
                (
                    "@command",
                    vd.command.as_ref().map_or(Json::Null, |items| {
                        Json::Array(items.iter().map(|t| text(Some(t))).collect())
                    }),
                ),
                (
                    "@detection_methods",
                    vd.detection_methods.as_ref().map_or(Json::Null, |ms| {
                        Json::Array(ms.iter().map(method).collect())
                    }),
                ),
                ("@modern_threshold", text(vd.modern_threshold.as_ref())),
                ("@pattern", text(vd.pattern.as_ref())),
                ("@source", text(vd.source.get())),
            ],
        )
    }

    fn method(m: &VersionDetectionMethod) -> Json {
        object(
            "Ukiryu::Models::VersionDetectionMethod",
            vec![
                ("@command", text(m.command.as_ref())),
                ("@paths", m.paths.as_ref().map_or(Json::Null, encode)),
                ("@pattern", text(m.pattern.as_ref())),
                ("@type", text(m.r#type.as_ref())),
            ],
        )
    }
}
