//! `yaml` kind: YAML loading through the core's own readers.
//!
//! * `yaml.load` is `YAML.safe_load(text, permitted_classes: [Symbol, Date,
//!   Time])`, the core's `ukiryu::schema::safe_load` with
//!   `Permitted::DOCUMENT` (Yeptris parse, Psych scalar resolution).
//! * `yaml.model_load` is `Ukiryu::Models::<model>.from_yaml(path)` (the
//!   plain register models read the file at `path`), snapshotted as the
//!   oracle's `snapshot`: class name plus sorted instance variables.
//! * `yaml.lutaml_roundtrip` is the lutaml `from_yaml`/`to_yaml` pair of a
//!   `Model`, emitted by the core's Yeptris engine.
use std::path::Path;

use ukiryu::models::{
    ImplementationIndex, ImplementationVersion, Interface, Model, ModelError, Text, ToolDefinition,
    VersionDetection, VersionDetectionMethod,
};
use ukiryu::schema::{safe_load, Data, Permitted, SchemaError};
use ukiryu::types::ruby::float_to_s;

use super::codec::encode;
use super::harness::{input_str, Ctx, Outcome, RowError};
use super::json::Oj;

pub fn load(input: &Oj, ctx: &Ctx) -> Outcome {
    let text = ctx.read_input(input_str(input, "file")?)?;
    let data = safe_load(&text, Permitted::DOCUMENT).map_err(schema_error)?;
    data_to_oj(&data)
}

pub fn model_load(input: &Oj, ctx: &Ctx) -> Outcome {
    let path = ctx.input_path(input_str(input, "file")?);
    let path = Path::new(&path);
    match input_str(input, "model")? {
        "Interface" => Interface::from_yaml(path)
            .map(|i| interface_snapshot(&i))
            .map_err(model_error),
        "ImplementationVersion" => ImplementationVersion::from_yaml(path)
            .map(|v| version_snapshot(&v))
            .map_err(model_error),
        "ImplementationIndex" => ImplementationIndex::from_yaml(path)
            .map(|i| index_snapshot(&i))
            .map_err(model_error),
        other => Err(RowError::harness(format!(
            "no core API: Ukiryu::Models::{other}.from_yaml(path) is not mapped"
        ))),
    }
}

pub fn lutaml_roundtrip(input: &Oj, ctx: &Ctx) -> Outcome {
    let text = ctx.read_input(input_str(input, "file")?)?;
    match input_str(input, "model")? {
        "ToolDefinition" => roundtrip::<ToolDefinition>(&text),
        other => Err(RowError::harness(format!(
            "no core API: Ukiryu::Models::{other} lutaml roundtrip is not mapped"
        ))),
    }
}

/// `first = model.from_yaml(text); emitted = first.to_yaml;
/// model.from_yaml(emitted).to_yaml == emitted`.
fn roundtrip<T: Model>(text: &str) -> Outcome {
    let first = T::from_yaml(text).map_err(model_error)?;
    let emitted = first.to_yaml().map_err(model_error)?;
    let again = T::from_yaml(&emitted)
        .and_then(|m| m.to_yaml())
        .map_err(model_error)?;
    let reparsed_equal = again == emitted;
    Ok(Oj::obj(vec![
        ("emitted", Oj::Str(emitted)),
        ("reparsed_equal", Oj::Bool(reparsed_equal)),
    ]))
}

fn model_error(e: ModelError) -> RowError {
    RowError::new(e.ruby_class(), e.message)
}

fn schema_error(e: SchemaError) -> RowError {
    match e.ruby_class() {
        Some(class) => RowError::new(class, e.message()),
        None => RowError::harness(format!("core unsupported: {}", e.message())),
    }
}

/// The corpus encoding of a `safe_load` result.
fn data_to_oj(d: &Data) -> Outcome {
    Ok(match d {
        Data::Null => Oj::Null,
        Data::Bool(b) => Oj::Bool(*b),
        Data::Integer(i) => Oj::Num(i.to_string()),
        Data::Float(f) => Oj::tagged("$float", Oj::Str(float_to_s(*f))),
        Data::String(s) => Oj::Str(s.clone()),
        Data::Symbol(s) => Oj::sym(s.clone()),
        Data::Date { .. } => {
            return Err(RowError::harness(
                "Date has no corpus value encoding (README \"Value encoding\")",
            ))
        }
        Data::Time(raw) => {
            return Err(RowError::harness(format!(
                "Time {raw:?} is kept as its YAML spelling by the core; no UTC instant to encode"
            )))
        }
        Data::Array(items) => Oj::Arr(items.iter().map(data_to_oj).collect::<Result<_, _>>()?),
        Data::Hash(pairs) => {
            if pairs.iter().all(|(k, _)| matches!(k, Data::String(_))) {
                Oj::Obj(
                    pairs
                        .iter()
                        .map(|(k, v)| match k {
                            Data::String(s) => Ok((s.clone(), data_to_oj(v)?)),
                            _ => unreachable!(),
                        })
                        .collect::<Result<_, RowError>>()?,
                )
            } else {
                Oj::tagged(
                    "$map",
                    Oj::Arr(
                        pairs
                            .iter()
                            .map(|(k, v)| Ok(Oj::Arr(vec![data_to_oj(k)?, data_to_oj(v)?])))
                            .collect::<Result<_, RowError>>()?,
                    ),
                )
            }
        }
    })
}

/// `{"class" => name, "ivars" => {...}}`; `ivars` must already be sorted
/// by name, as `instance_variables.sort` orders them.
fn object(class: &str, ivars: Vec<(&str, Oj)>) -> Oj {
    Oj::obj(vec![("class", Oj::s(class)), ("ivars", Oj::obj(ivars))])
}

fn interface_snapshot(i: &Interface) -> Oj {
    object(
        "Ukiryu::Models::Interface",
        vec![
            ("@actions", encode(&i.actions)),
            (
                "@aliases",
                Oj::Arr(i.aliases.iter().map(|a| Oj::sym(a.clone())).collect()),
            ),
            ("@display_name", encode(&i.display_name)),
            ("@name", encode(&i.name)),
            ("@version", encode(&i.version)),
        ],
    )
}

fn version_snapshot(v: &ImplementationVersion) -> Oj {
    object(
        "Ukiryu::Models::ImplementationVersion",
        vec![
            (
                "@aliases",
                Oj::Arr(v.aliases.iter().map(|a| Oj::s(a.clone())).collect()),
            ),
            ("@display_name", encode(&v.display_name)),
            ("@execution_profiles", encode(&v.execution_profiles)),
            ("@implements", encode(&v.implements)),
            ("@version", encode(&v.version)),
            (
                "@version_detection",
                v.version_detection
                    .as_ref()
                    .map_or(Oj::Null, detection_snapshot),
            ),
        ],
    )
}

fn index_snapshot(i: &ImplementationIndex) -> Oj {
    object(
        "Ukiryu::Models::ImplementationIndex",
        vec![
            (
                "@implementations",
                Oj::Arr(i.implementations.iter().map(encode).collect()),
            ),
            ("@interface", encode(&i.interface)),
            ("@interface_version", encode(&i.interface_version)),
            ("@name", encode(&i.name)),
        ],
    )
}

fn text(t: Option<&Text>) -> Oj {
    t.map_or(Oj::Null, |t| encode(&t.to_ruby_value()))
}

fn text_list(items: &Option<Vec<Text>>) -> Oj {
    match items {
        None => Oj::Null,
        Some(items) => Oj::Arr(items.iter().map(|t| encode(&t.to_ruby_value())).collect()),
    }
}

/// The lutaml model's declared attributes, as the core holds them. Lutaml's
/// own bookkeeping ivars have no counterpart in the core model.
fn detection_snapshot(vd: &VersionDetection) -> Oj {
    object(
        "Ukiryu::Models::VersionDetection",
        vec![
            ("@command", text_list(&vd.command)),
            (
                "@detection_methods",
                match &vd.detection_methods {
                    None => Oj::Null,
                    Some(ms) => Oj::Arr(ms.iter().map(method_snapshot).collect()),
                },
            ),
            ("@modern_threshold", text(vd.modern_threshold.as_ref())),
            ("@pattern", text(vd.pattern.as_ref())),
            ("@source", text(vd.source.get())),
        ],
    )
}

fn method_snapshot(m: &VersionDetectionMethod) -> Oj {
    object(
        "Ukiryu::Models::VersionDetectionMethod",
        vec![
            ("@command", text(m.command.as_ref())),
            ("@paths", m.paths.as_ref().map_or(Oj::Null, encode)),
            ("@pattern", text(m.pattern.as_ref())),
            ("@type", text(m.r#type.as_ref())),
        ],
    )
}
