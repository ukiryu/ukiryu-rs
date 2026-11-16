//! `Ukiryu::Type.validate` / `Ukiryu::Type.valid_type?`.
//!
//! Source: `lib/ukiryu/type.rb` (ukiryu/ukiryu ab6df353). Every branch,
//! message and returned Ruby type follows the pinned implementation, including
//! its quirks (for example `of:` validates each element with the array's own
//! options, and `allow_empty` does not skip a pattern check).

use super::datetime::{self, Now};
use super::error::{Error, ErrorClass, Result};
use super::number::{parse_float, parse_integer};
use super::onig;
use super::ruby::{
    bigint_from_integral_f64, bigint_to_f64, downcase, float_to_s, gt, inspect, lt, strip, to_s,
};
use super::uri::{self, UriError};
use super::value::{eql, ruby_eq, OptionSource, Regexp, Value};
use crate::runtime::Clock;

/// The environment that type validation reads but pure data cannot supply.
pub trait Host {
    /// `File.exist?(path)` (follows symlinks).
    fn file_exists(&self, path: &str) -> bool;

    /// `string =~ regexp`. The default runs Ruby 4.0.1's Onigmo
    /// ([`super::onig`]); `None` means no answer (Ruby would not create the
    /// `Regexp`, or the engine failed), and validation then fails with an
    /// `Unsupported` error.
    fn regexp_match(&self, regexp: &Regexp, string: &str) -> Option<bool> {
        onig::regexp_match(regexp, string)
    }

    /// The local UTC offset (seconds) for a local civil time, as Ruby's
    /// `Time.local` would use. `None` when the host cannot tell.
    fn local_offset(
        &self,
        _year: i64,
        _month: u32,
        _day: u32,
        _hour: u32,
        _minute: u32,
        _second: u32,
    ) -> Option<i32> {
        None
    }

    /// `Time.local(year, month, day).zone`. `None` when the host cannot tell.
    fn local_zone_abbreviation(&self, _year: i64, _month: u32, _day: u32) -> Option<String> {
        None
    }

    /// `Time.now`, which `Time.parse` uses for the fields a text leaves out.
    /// `None` when the host has no clock.
    fn now(&self) -> Option<Now> {
        None
    }
}

/// The process environment: the real filesystem; the local zone and the
/// system clock only when `TZ` names UTC.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdHost;

fn tz_is_utc() -> bool {
    matches!(
        std::env::var("TZ").ok().as_deref(),
        Some("UTC" | "UTC0" | "Etc/UTC" | "Etc/UCT" | "UCT" | "Zulu" | "Etc/Zulu")
    )
}

impl Host for StdHost {
    fn file_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn local_offset(&self, _: i64, _: u32, _: u32, _: u32, _: u32, _: u32) -> Option<i32> {
        tz_is_utc().then_some(0)
    }

    fn local_zone_abbreviation(&self, _: i64, _: u32, _: u32) -> Option<String> {
        tz_is_utc().then(|| "UTC".to_string())
    }

    fn now(&self) -> Option<Now> {
        // wasm32-unknown-unknown has no system clock.
        if cfg!(all(target_arch = "wasm32", not(target_os = "wasi"))) || !tz_is_utc() {
            return None;
        }
        Some(Now::from_clock(Clock::System { utc_offset_secs: 0 }, 0))
    }
}

/// The ten type names `Type` accepts.
pub const VALID_TYPES: [&str; 10] = [
    "file", "string", "integer", "float", "symbol", "boolean", "uri", "datetime", "hash", "array",
];

/// `Type.normalize_type(type)[:name]`.
fn type_name(spec: &Value) -> Value {
    match spec {
        Value::Hash(_) => spec.sym_get("name"),
        Value::String(s) => Value::Symbol(s.clone()),
        other => other.clone(),
    }
}

/// `Ukiryu::Type.valid_type?(type)`.
pub fn valid_type(spec: &Value) -> bool {
    matches!(type_name(spec), Value::Symbol(s) if VALID_TYPES.contains(&s.as_str()))
}

/// `Ukiryu::Type.validate(value, type, options)`.
pub fn validate(
    value: &Value,
    spec: &Value,
    options: &dyn OptionSource,
    host: &dyn Host,
) -> Result<Value> {
    validate_at(value, spec, options, host, 0)
}

/// Ruby raises SystemStackError long before this depth for the only
/// unbounded recursion (`of: :array`); the outcome does not depend on depth.
const MAX_DEPTH: usize = 256;

fn validate_at(
    value: &Value,
    spec: &Value,
    options: &dyn OptionSource,
    host: &dyn Host,
    depth: usize,
) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err(Error::new(ErrorClass::SystemStack, "stack level too deep"));
    }
    let name = type_name(spec);
    let known = match &name {
        Value::Symbol(s) => s.as_str(),
        _ => "",
    };
    match known {
        "file" => validate_file(value, options, host),
        "string" => validate_string(value, options, host),
        "integer" => validate_integer(value, options),
        "float" => validate_float(value, options),
        "symbol" => validate_symbol(value, options),
        "boolean" => validate_boolean(value),
        "uri" => validate_uri(value),
        "datetime" => validate_datetime(value, host),
        "hash" => validate_hash(value, options),
        "array" => validate_array(value, options, host, depth),
        _ => Err(Error::validation(format!("Unknown type: {}", to_s(&name)))),
    }
}

fn validate_file(value: &Value, options: &dyn OptionSource, host: &dyn Host) -> Result<Value> {
    let path = to_s(value);
    if path.is_empty() {
        return Err(Error::validation("File path cannot be empty"));
    }
    if options.option("require_existing").truthy() {
        if path.contains('\0') {
            return Err(Error::new(
                ErrorClass::Argument,
                "string contains null byte",
            ));
        }
        if !host.file_exists(&path) {
            return Err(Error::validation(format!("File not found: {path}")));
        }
    }
    Ok(Value::String(path))
}

fn validate_string(value: &Value, options: &dyn OptionSource, host: &dyn Host) -> Result<Value> {
    let s = to_s(value);
    if s.is_empty() && !options.option("allow_empty").truthy() {
        return Err(Error::validation("String cannot be empty"));
    }
    let pattern = options.option("pattern");
    if pattern.truthy() {
        let Value::Regexp(re) = &pattern else {
            return Err(Error::new(
                ErrorClass::Type,
                format!("type mismatch: {} given", pattern.class_name()),
            ));
        };
        let matched = host.regexp_match(re, &s).ok_or_else(|| {
            Error::unsupported(format!(
                "matching {} needs a Ruby-compatible regular expression engine",
                inspect(&pattern)
            ))
        })?;
        if !matched {
            return Err(Error::validation(format!(
                "String does not match required pattern: {}",
                to_s(&pattern)
            )));
        }
    }
    Ok(Value::String(s))
}

/// `min, max = options[:range]`.
fn range_bounds(range: &Value) -> (Value, Value) {
    match range {
        Value::Array(items) => (
            items.first().cloned().unwrap_or(Value::Nil),
            items.get(1).cloned().unwrap_or(Value::Nil),
        ),
        other => (other.clone(), Value::Nil),
    }
}

/// `Integer(value)` as `validate_integer` calls it; `Ok(None)` is a rescued
/// ArgumentError/TypeError.
fn kernel_integer(value: &Value) -> Result<Option<Value>> {
    Ok(match value {
        Value::Integer(_) => Some(value.clone()),
        Value::Float(f) => {
            if f.is_nan() {
                return Err(Error::new(ErrorClass::FloatDomain, "NaN"));
            }
            if f.is_infinite() {
                let m = if *f > 0.0 { "Infinity" } else { "-Infinity" };
                return Err(Error::new(ErrorClass::FloatDomain, m));
            }
            Some(Value::Integer(bigint_from_integral_f64(f.trunc())))
        }
        Value::String(s) => parse_integer(s).map(Value::Integer),
        Value::Time(t) => Some(Value::int(t.seconds)),
        _ => None,
    })
}

fn validate_integer(value: &Value, options: &dyn OptionSource) -> Result<Value> {
    let value = match value {
        Value::String(s) => Value::String(strip(s).to_string()),
        other => other.clone(),
    };
    let integer = kernel_integer(&value)?
        .ok_or_else(|| Error::validation(format!("Invalid integer: {}", inspect(&value))))?;
    let range = options.option("range");
    if range.truthy() {
        let (min, max) = range_bounds(&range);
        if lt(&integer, &min)? || gt(&integer, &max)? {
            return Err(Error::validation(format!(
                "Integer {} out of range [{}, {}]",
                to_s(&integer),
                to_s(&min),
                to_s(&max)
            )));
        }
    }
    let min = options.option("min");
    if min.truthy() && lt(&integer, &min)? {
        return Err(Error::validation(format!(
            "Integer {} below minimum {}",
            to_s(&integer),
            to_s(&min)
        )));
    }
    let max = options.option("max");
    if max.truthy() && gt(&integer, &max)? {
        return Err(Error::validation(format!(
            "Integer {} above maximum {}",
            to_s(&integer),
            to_s(&max)
        )));
    }
    Ok(integer)
}

/// `Float(value)`; `Ok(None)` is a rescued ArgumentError/TypeError.
fn kernel_float(value: &Value) -> Result<Option<f64>> {
    Ok(match value {
        Value::Float(f) => Some(*f),
        Value::Integer(i) => Some(bigint_to_f64(i)),
        Value::String(s) => parse_float(s)?,
        Value::Time(t) => Some(t.to_f()),
        _ => None,
    })
}

fn validate_float(value: &Value, options: &dyn OptionSource) -> Result<Value> {
    let value = match value {
        Value::String(s) => Value::String(strip(s).to_string()),
        other => other.clone(),
    };
    let float = kernel_float(&value)?
        .ok_or_else(|| Error::validation(format!("Invalid float: {}", inspect(&value))))?;
    let fv = Value::Float(float);
    let range = options.option("range");
    if range.truthy() {
        let (min, max) = range_bounds(&range);
        if lt(&fv, &min)? || gt(&fv, &max)? {
            return Err(Error::validation(format!(
                "Float {} out of range [{}, {}]",
                float_to_s(float),
                to_s(&min),
                to_s(&max)
            )));
        }
    }
    Ok(fv)
}

fn validate_symbol(value: &Value, options: &dyn OptionSource) -> Result<Value> {
    let symbol = match value {
        Value::Symbol(_) => value.clone(),
        other => Value::Symbol(downcase(&to_s(other))),
    };
    let values = options.option("values");
    if values.truthy() {
        let items = match &values {
            Value::Array(items) => items,
            Value::Hash(_) => {
                return Err(Error::unsupported(
                    "symbol values given as a Hash are not ported",
                ))
            }
            other => return Err(super::ruby::no_method("map", other)),
        };
        let valid = items.iter().map(|v| match v {
            Value::String(s) => Value::Symbol(s.clone()),
            other => other.clone(),
        });
        if !valid.clone().any(|v| ruby_eq(&v, &symbol)) {
            return Err(Error::validation(format!(
                "Invalid symbol: {}. Valid values: {}",
                inspect(&symbol),
                inspect(&values)
            )));
        }
    }
    Ok(symbol)
}

fn validate_boolean(value: &Value) -> Result<Value> {
    match value {
        Value::Bool(b) => Ok(Value::Bool(*b)),
        Value::String(s) if matches!(s.as_str(), "true" | "1" | "yes" | "on") => {
            Ok(Value::Bool(true))
        }
        Value::String(s) if matches!(s.as_str(), "false" | "0" | "no" | "off" | "") => {
            Ok(Value::Bool(false))
        }
        other => Err(Error::validation(format!(
            "Invalid boolean: {}",
            inspect(other)
        ))),
    }
}

fn validate_uri(value: &Value) -> Result<Value> {
    let s = to_s(value);
    if s.is_empty() {
        return Err(Error::validation("URI cannot be empty"));
    }
    match uri::parse_to_s(&s) {
        Ok(out) => Ok(Value::String(out)),
        Err(UriError::Invalid(m)) => Err(Error::validation(format!("Invalid URI: {s} - {m}"))),
        Err(e) => Err(e.into()),
    }
}

fn validate_datetime(value: &Value, host: &dyn Host) -> Result<Value> {
    if let Value::Time(_) = value {
        return Ok(value.clone());
    }
    match datetime::parse(&to_s(value), host) {
        Ok(t) => Ok(Value::Time(t)),
        Err(e) if e.class == ErrorClass::Argument => Err(Error::validation(format!(
            "Invalid datetime: {} - {}",
            inspect(value),
            e.message
        ))),
        Err(e) => Err(e),
    }
}

fn validate_hash(value: &Value, options: &dyn OptionSource) -> Result<Value> {
    let Value::Hash(pairs) = value else {
        return Err(Error::validation(format!(
            "Hash expected, got {}: {}",
            value.class_name(),
            inspect(value)
        )));
    };
    let keys = options.option("keys");
    if keys.truthy() {
        let Value::Array(allowed) = &keys else {
            return Err(Error::new(
                ErrorClass::Type,
                format!("no implicit conversion of {} into Array", keys.class_name()),
            ));
        };
        let unknown: Vec<Value> = pairs
            .iter()
            .map(|(k, _)| k)
            .filter(|k| !allowed.iter().any(|a| eql(a, k)))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            return Err(Error::validation(format!(
                "Unknown hash keys: {}. Valid keys: {}",
                inspect(&Value::Array(unknown)),
                inspect(&keys)
            )));
        }
    }
    Ok(value.clone())
}

fn validate_array(
    value: &Value,
    options: &dyn OptionSource,
    host: &dyn Host,
    depth: usize,
) -> Result<Value> {
    let array = match value {
        Value::Array(items) => items.clone(),
        other => vec![other.clone()],
    };
    let size = Value::int(array.len() as i64);
    let min = options.option("min");
    if min.truthy() && lt(&size, &min)? {
        return Err(Error::validation(format!(
            "Array has {} elements, minimum is {}",
            array.len(),
            to_s(&min)
        )));
    }
    let max = options.option("max");
    if max.truthy() && gt(&size, &max)? {
        return Err(Error::validation(format!(
            "Array has {} elements, maximum is {}",
            array.len(),
            to_s(&max)
        )));
    }
    match options.option("size") {
        Value::Integer(n) if Value::Integer(n.clone()) != size => {
            return Err(Error::validation(format!(
                "Array has {} elements, expected {}",
                array.len(),
                n
            )));
        }
        sizes @ Value::Array(_) => {
            let Value::Array(items) = &sizes else {
                unreachable!()
            };
            if !items.iter().any(|v| ruby_eq(v, &size)) {
                return Err(Error::validation(format!(
                    "Array has {} elements, expected one of: {}",
                    array.len(),
                    inspect(&sizes)
                )));
            }
        }
        _ => {}
    }
    let of = options.option("of");
    if of.truthy() {
        let mut out = Vec::with_capacity(array.len());
        for item in &array {
            out.push(validate_at(item, &of, options, host, depth + 1)?);
        }
        return Ok(Value::Array(out));
    }
    Ok(Value::Array(array))
}
