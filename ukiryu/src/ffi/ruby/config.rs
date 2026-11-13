//! `Ukiryu::Native::Config`: the core configuration conversions with the
//! Ruby types the source returns.
//!
//! The core keeps configuration values in its JSON profile, where Ruby
//! Symbols are text ("Ruby symbols have textual wire values; adapters expose
//! their native types", `crate::config`). The Ruby source's `EnvSchema` types
//! `shell` and `format` as `:symbol` (lib/ukiryu/config/env_schema.rb), so
//! those two come back as Symbols here; every other value is unchanged.
//! Hashes are keyed by Symbols in the schema's attribute order, which is the
//! order `EnvProvider.load_all` and `Config#to_h` produce.
use super::errors::wire_error;
use super::values::{text, to_ruby};
use super::wire::json_value;
use crate::config::{self, Attribute, Environment};
use crate::types::Value;
use magnus::{function, prelude::*, Error, RHash, RModule, RString, Ruby, Value as RValue};
use serde_json::json;

pub(super) fn define(_ruby: &Ruby, native: RModule) -> Result<(), Error> {
    let module = native.define_module("Config")?;
    module.define_module_function("convert", function!(convert, 2))?;
    module.define_module_function("environment_values", function!(environment_values, 1))?;
    module.define_module_function("defaults", function!(defaults, 0))?;
    Ok(())
}

/// Attributes whose Ruby type is `:symbol` in `EnvSchema::ATTRIBUTE_TYPES`.
pub(super) fn symbol_attribute(name: &str) -> bool {
    name == Attribute::Shell.name() || name == Attribute::Format.name()
}

/// A core configuration value with the Ruby type of its attribute.
pub(super) fn typed_config_value(name: &str, value: Value) -> Value {
    match value {
        Value::String(s) if symbol_attribute(name) => Value::Symbol(s),
        other => other,
    }
}

fn attribute_name(ruby: &Ruby, attribute: RValue) -> Result<String, Error> {
    if let Some(symbol) = magnus::Symbol::from_value(attribute) {
        return Ok(symbol.name()?.into_owned());
    }
    if let Some(string) = RString::from_value(attribute) {
        return text(ruby, string);
    }
    Err(Error::new(
        ruby.exception_type_error(),
        "configuration attribute must be a Symbol or String",
    ))
}

fn conversion_error(ruby: &Ruby, error: config::ConversionError) -> Error {
    wire_error(
        ruby,
        &json!({"code": "INVALID_ARGUMENT", "source_class": "ArgumentError", "message": error.to_string()}),
    )
}

/// `TypeConverter.convert(attribute, value)`; `value` is a String or nil.
fn convert(ruby: &Ruby, attribute: RValue, value: Option<RString>) -> Result<RValue, Error> {
    let name = attribute_name(ruby, attribute)?;
    let input = value.map(|s| text(ruby, s)).transpose()?;
    let converted =
        config::convert(&name, input.as_deref()).map_err(|e| conversion_error(ruby, e))?;
    to_ruby(
        ruby,
        &typed_config_value(&name, json_value(ruby, &converted)?),
    )
}

/// `EnvProvider.load_all` over an explicit environment Hash (process ENV is
/// never read).
fn environment_values(ruby: &Ruby, environment: RHash) -> Result<RHash, Error> {
    let mut env = Environment::new();
    environment.foreach(|key: RString, value: RString| {
        env.insert(text(ruby, key)?, text(ruby, value)?);
        Ok(magnus::r_hash::ForEach::Continue)
    })?;
    let values = config::environment_values(&env).map_err(|e| conversion_error(ruby, e))?;
    ordered(ruby, &values)
}

/// `Config.new.to_h` defaults.
fn defaults(ruby: &Ruby) -> Result<RHash, Error> {
    ordered(ruby, &config::defaults())
}

fn ordered(ruby: &Ruby, values: &config::Values) -> Result<RHash, Error> {
    let out = ruby.hash_new();
    for attribute in Attribute::ALL {
        let name = attribute.name();
        if let Some(value) = values.get(name) {
            let value = typed_config_value(name, json_value(ruby, value)?);
            out.aset(ruby.to_symbol(name), to_ruby(ruby, &value)?)?;
        }
    }
    Ok(out)
}
