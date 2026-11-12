//! Versioned, length-delimited JSON boundary for thin runtime adapters.
use crate::config::{self, Environment, OverrideResolver, Values};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Version,
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
