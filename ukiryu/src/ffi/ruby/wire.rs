//! Operations shared with the C ABI. Each Ruby method sends the same typed
//! request the ffi tier sends and dispatches the same [`crate::wire::process`]
//! in process, so both tiers answer with one implementation. The Ruby tier
//! only changes the transport: replies become Ruby objects (no JSON text),
//! Ruby Symbols take the textual wire form on the way in and come back as
//! Symbols where the Ruby source returns Symbols, and failures raise.
//!
//! | wire op          | Ruby                                                       |
//! |------------------|------------------------------------------------------------|
//! | `escape`         | `Ukiryu::Native.escape(shell, method, arguments)`           |
//! | `version`        | `Ukiryu::Native.core_version`, `.abi_version`               |
//! | `parse_yaml`     | `Ukiryu::Native.parse_yaml(yaml)`                           |
//! | `resolve_config` | `Ukiryu::Native.resolve_config(environment, programmatic, cli)` |
use super::config::{symbol_attribute, typed_config_value};
use super::errors::wire_error;
use super::values::{from_ruby, to_ruby};
use crate::config::Attribute;
use crate::types::Value;
use magnus::{function, prelude::*, Error, RHash, RModule, Ruby, Value as RValue};
use num_bigint::BigInt;
use serde_json::{json, Map, Number, Value as Json};
use std::str::FromStr;

pub(super) fn define(_ruby: &Ruby, native: RModule) -> Result<(), Error> {
    native.define_module_function("core_version", function!(core_version, 0))?;
    native.define_module_function("abi_version", function!(abi_version, 0))?;
    native.define_module_function("parse_yaml", function!(parse_yaml, 1))?;
    native.define_module_function("resolve_config", function!(resolve_config, 3))?;
    native.define_module_function("escape", function!(escape, 3))?;
    Ok(())
}

/// Dispatches one wire request; the `ok:false` envelope raises.
fn call(ruby: &Ruby, request: Json) -> Result<Json, Error> {
    let reply = crate::wire::process(request.to_string().as_bytes());
    if reply["ok"] == Json::Bool(true) {
        Ok(reply)
    } else {
        Err(wire_error(ruby, &reply["error"]))
    }
}

fn core_version(ruby: &Ruby) -> Result<String, Error> {
    let reply = call(ruby, json!({"op": "version"}))?;
    Ok(reply["version"].as_str().unwrap_or_default().to_owned())
}

fn abi_version(ruby: &Ruby) -> Result<u64, Error> {
    let reply = call(ruby, json!({"op": "version"}))?;
    Ok(reply["abi_version"].as_u64().unwrap_or_default())
}

/// The wire's JSON value profile: string keys, finite numbers, no tags.
fn parse_yaml(ruby: &Ruby, yaml: magnus::RString) -> Result<RValue, Error> {
    let yaml = super::values::text(ruby, yaml)?;
    let reply = call(ruby, json!({"op": "parse_yaml", "yaml": yaml}))?;
    to_ruby(ruby, &json_value(ruby, &reply["value"])?)
}

/// One shell formatter call: `escape("bash", :quote, {string: "a b"})`.
/// `arguments` holds the method's own fields (`string`, `for_exe`, `path`,
/// `name`, `executable`, `args`), with Symbol or String keys.
fn escape(ruby: &Ruby, shell: RString, method: RValue, arguments: RHash) -> Result<RValue, Error> {
    let method = match from_ruby(ruby, method)? {
        Value::Symbol(s) | Value::String(s) => s,
        _ => {
            return Err(Error::new(
                ruby.exception_type_error(),
                "escape method must be a Symbol or String",
            ))
        }
    };
    let mut request = layer(ruby, arguments)?;
    request.insert("op".into(), Json::String("escape".into()));
    request.insert(
        "shell".into(),
        Json::String(super::values::text(ruby, shell)?),
    );
    request.insert("method".into(), Json::String(method));
    let reply = call(ruby, Json::Object(request))?;
    to_ruby(ruby, &json_value(ruby, &reply["value"])?)
}

/// `{values:, sources:}` as `Config#to_h` keys and `source_for` would give
/// them: Symbol keys in schema order, Symbol-typed `shell`/`format` values,
/// and Symbol sources (`:cli`, `:env`, `:programmatic`, `:default`) or nil.
fn resolve_config(
    ruby: &Ruby,
    environment: RHash,
    programmatic: RHash,
    cli: RHash,
) -> Result<RHash, Error> {
    let request = json!({
        "op": "resolve_config",
        "environment": string_map(ruby, environment)?,
        "programmatic": layer(ruby, programmatic)?,
        "cli": layer(ruby, cli)?,
    });
    let reply = call(ruby, request)?;
    let values = ruby.hash_new();
    let sources = ruby.hash_new();
    for attribute in Attribute::ALL {
        let name = attribute.name();
        let key = ruby.to_symbol(name);
        let value = json_value(ruby, &reply["values"][name])?;
        values.aset(key, to_ruby(ruby, &typed_config_value(name, value))?)?;
        let source = match reply["sources"][name].as_str() {
            Some(text) => ruby.to_symbol(text).as_value(),
            None => ruby.qnil().as_value(),
        };
        sources.aset(key, source)?;
    }
    let out = ruby.hash_new();
    out.aset(ruby.to_symbol("values"), values)?;
    out.aset(ruby.to_symbol("sources"), sources)?;
    Ok(out)
}

fn string_map(ruby: &Ruby, hash: RHash) -> Result<Map<String, Json>, Error> {
    let mut out = Map::new();
    match from_ruby(ruby, hash.as_value())? {
        Value::Hash(pairs) => {
            for (key, value) in pairs {
                match (key, value) {
                    (Value::String(k), Value::String(v)) => {
                        out.insert(k, Json::String(v));
                    }
                    _ => {
                        return Err(Error::new(
                            ruby.exception_type_error(),
                            "environment keys and values must be Strings",
                        ))
                    }
                }
            }
        }
        _ => unreachable!("an RHash converts to a Hash"),
    }
    Ok(out)
}

/// One programmatic/CLI layer: Symbol or String attribute keys. A Symbol
/// value is accepted only for an attribute the Ruby schema types as Symbol;
/// anywhere else the JSON-valued core configuration could not give it back.
fn layer(ruby: &Ruby, hash: RHash) -> Result<Map<String, Json>, Error> {
    let mut out = Map::new();
    let Value::Hash(pairs) = from_ruby(ruby, hash.as_value())? else {
        unreachable!("an RHash converts to a Hash")
    };
    for (key, value) in pairs {
        let name = match key {
            Value::Symbol(s) | Value::String(s) => s,
            _ => {
                return Err(Error::new(
                    ruby.exception_type_error(),
                    "configuration keys must be Symbols or Strings",
                ))
            }
        };
        let json = match value {
            Value::Symbol(s) if symbol_attribute(&name) => Json::String(s),
            other => json_of(ruby, &other)?,
        };
        out.insert(name, json);
    }
    Ok(out)
}

/// A core value in the wire's JSON profile; values JSON cannot carry raise.
fn json_of(ruby: &Ruby, value: &Value) -> Result<Json, Error> {
    let refuse = |what: &str| {
        Error::new(
            ruby.exception_type_error(),
            format!("{what} has no JSON configuration value"),
        )
    };
    Ok(match value {
        Value::Nil => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Integer(i) => {
            Json::Number(Number::from_str(&i.to_string()).map_err(|_| refuse("an Integer"))?)
        }
        Value::Float(f) => {
            Json::Number(Number::from_f64(*f).ok_or_else(|| refuse("a non-finite Float"))?)
        }
        Value::String(s) => Json::String(s.clone()),
        Value::Symbol(_) => return Err(refuse("a Symbol outside shell/format")),
        Value::Time(_) => return Err(refuse("a Time")),
        Value::Regexp(_) => return Err(refuse("a Regexp")),
        Value::Array(items) => Json::Array(
            items
                .iter()
                .map(|item| json_of(ruby, item))
                .collect::<Result<_, _>>()?,
        ),
        Value::Hash(pairs) => {
            let mut map = Map::new();
            for (key, item) in pairs {
                let Value::String(key) = key else {
                    return Err(refuse("a non-String Hash key"));
                };
                map.insert(key.clone(), json_of(ruby, item)?);
            }
            Json::Object(map)
        }
    })
}

/// A wire JSON value as a core value: integers keep their exact digits.
pub(super) fn json_value(ruby: &Ruby, json: &Json) -> Result<Value, Error> {
    Ok(match json {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            let text = n.to_string();
            if let Ok(i) = BigInt::from_str(&text) {
                Value::Integer(i)
            } else {
                Value::Float(n.as_f64().ok_or_else(|| {
                    Error::new(ruby.exception_range_error(), format!("number {text}"))
                })?)
            }
        }
        Json::String(s) => Value::String(s.clone()),
        Json::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| json_value(ruby, item))
                .collect::<Result<_, _>>()?,
        ),
        Json::Object(map) => Value::Hash(
            map.iter()
                .map(|(k, v)| Ok((Value::String(k.clone()), json_value(ruby, v)?)))
                .collect::<Result<_, Error>>()?,
        ),
    })
}
