//! Versioned, length-delimited JSON boundary for thin runtime adapters.
//!
//! One request is one JSON object whose `op` names the operation; the
//! reply is `{"ok": true, ...}` or `{"ok": false, "error": {"code", ...}}`
//! (see [`error`] for the codes). Ruby values use the typed encoding of
//! [`value`], the conformance corpus value encoding.
//!
//! | op | what it runs |
//! |---|---|
//! | `version` | the core and ABI versions |
//! | `resolve_config` | `Config` resolution |
//! | `parse_yaml` | YAML through the core's YAML engine, as JSON |
//! | `escape` | shell formatting ([`portable`]) |
//! | `resolve` | version extraction, range matching, version-spec routing, `Tool.get` ([`ops`]) |
//! | `build` | `Tool#execute` up to the executor: argv, env, stdin, executable ([`ops`]) |
//! | `validate` | `Type.validate`, `DefinitionValidator.validate_string` ([`ops`]) |
//! | `schema` | `SchemaValidator.validate_profile`, `JSON::Validator.fully_validate` ([`ops`]) |
//! | `results` | exit-code meanings of a tool's commands ([`ops`]) |
//! | `execute` | `Tool#execute`, native builds only ([`native`]); its value is a [`results`] payload |
//!
//! Register directories and process execution exist only in native builds:
//! [`native`] is not compiled for WebAssembly.
pub mod error;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
pub mod ops;
pub mod portable;
pub mod results;
pub mod value;

use crate::config::{self, Environment, OverrideResolver, Values};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Version,
    Escape(portable::EscapeRequest),
    Resolve(Box<ops::ResolveRequest>),
    Build(Box<ops::BuildRequest>),
    Validate(ops::ValidateRequest),
    Schema(ops::SchemaRequest),
    Results(Box<ops::ResultsRequest>),
    #[cfg(not(target_arch = "wasm32"))]
    Execute(Box<native::ExecuteRequest>),
    ParseYaml {
        yaml: String,
    },
    ResolveConfig {
        #[serde(default)]
        environment: Environment,
        #[serde(default)]
        programmatic: Values,
        #[serde(default)]
        cli: Values,
    },
}

pub fn failure(code: &str, message: impl ToString) -> Value {
    json!({"ok":false,"error":{"code":code,"message":message.to_string()}})
}

pub fn process(input: &[u8]) -> Value {
    let request: Request = match serde_json::from_slice(input) {
        Ok(request) => request,
        Err(error) => return failure("INVALID_REQUEST", error),
    };
    match request {
        Request::Escape(request) => portable::escape(request),
        Request::Resolve(request) => ops::resolve(*request),
        Request::Build(request) => ops::build(*request),
        Request::Validate(request) => ops::validate(request),
        Request::Schema(request) => ops::schema(request),
        Request::Results(request) => ops::results(*request),
        #[cfg(not(target_arch = "wasm32"))]
        Request::Execute(request) => native::execute(*request),
        Request::Version => json!({"ok":true,"version":crate::VERSION,"abi_version":1}),
        Request::ParseYaml { yaml } => parse_yaml_json(&yaml),
        Request::ResolveConfig {
            environment,
            programmatic,
            cli,
        } => {
            let env = match config::environment_values(&environment) {
                Ok(env) => env,
                Err(error) => {
                    return json!({"ok":false,"error":{
                        "code":"INVALID_ARGUMENT","source_class":"ArgumentError","message":error.to_string()
                    }})
                }
            };
            let resolver = OverrideResolver {
                defaults: config::defaults(),
                programmatic,
                env,
                cli,
            };
            // Config#to_h exposes the declared attributes; the underlying resolver
            // remains usable for arbitrary keys through its own API.
            let values: Values = config::Attribute::ALL
                .into_iter()
                .map(|attribute| {
                    let key = attribute.name();
                    (
                        key.to_owned(),
                        resolver.resolve(key).cloned().unwrap_or(Value::Null),
                    )
                })
                .collect();
            let sources: std::collections::BTreeMap<_, _> = values
                .keys()
                .map(|key| (key.clone(), resolver.source_for(key)))
                .collect();
            json!({"ok":true,"values":values,"sources":sources})
        }
    }
}

/// The diagnostic transport has a JSON value profile. Validate the YAML tree
/// before handing it to serde, which otherwise maps non-finite floats to null
/// and may turn non-string YAML mapping keys into JSON strings.
fn parse_yaml_json(yaml: &str) -> Value {
    use yeptris::{Number, Value as Yaml};
    let tree = match crate::from_yaml::<Yaml>(yaml) {
        Ok(tree) => tree,
        Err(error) => return failure("YAML_PARSE_ERROR", error),
    };
    let mut pending = vec![&tree];
    while let Some(value) = pending.pop() {
        match value {
            Yaml::Number(Number::F64(number)) if !number.is_finite() => {
                return failure(
                    "YAML_NOT_JSON_COMPATIBLE",
                    "JSON cannot represent a non-finite YAML number",
                );
            }
            Yaml::Sequence(items) => pending.extend(items),
            Yaml::Mapping(mapping) => {
                for (key, value) in &mapping.0 {
                    if !matches!(key, Yaml::String(_)) {
                        return failure(
                            "YAML_NOT_JSON_COMPATIBLE",
                            "JSON object keys must be YAML strings",
                        );
                    }
                    pending.push(value);
                }
            }
            Yaml::Tagged(_) => {
                return failure(
                    "YAML_NOT_JSON_COMPATIBLE",
                    "JSON cannot preserve a YAML tag",
                );
            }
            _ => {}
        }
    }
    match yeptris::from_value::<Value>(&tree) {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => failure("YAML_PARSE_ERROR", error),
    }
}
