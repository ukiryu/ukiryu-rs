//! The Ruby value domain that `Ukiryu::Type` and `Ukiryu::Validation` operate on.
//!
//! Source: `lib/ukiryu/type.rb` and `lib/ukiryu/validation/*.rb` at
//! ukiryu/ukiryu ab6df353. Those modules receive arbitrary Ruby objects (option
//! values, Hash option bags, Hash command definitions) and return coerced Ruby
//! objects, so the port keeps the exact runtime type of every value: an
//! Integer is never a Float, a Symbol is never a String, and Hash keys keep
//! their type and insertion order.
//!
//! This is the crate's single dynamic value type: the models module holds raw
//! Ruby data (YAML hashes, routing tables, exit-code maps) as `Value` too and
//! adds its YAML and `Utils` conversions in `models::value`. Definition models
//! reach `Ukiryu::Type` through [`OptionSource`].
use std::fmt;

use num_bigint::BigInt;
use serde::de::{self, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};

pub use super::time::{SubNanos, Time};

/// A Ruby `Regexp` literal: its source and option bits
/// (`1` IGNORECASE, `2` EXTENDED, `4` MULTILINE, as `Regexp#options`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regexp {
    pub source: String,
    pub options: u32,
}

impl Regexp {
    pub const IGNORECASE: u32 = 1;
    pub const EXTENDED: u32 = 2;
    pub const MULTILINE: u32 = 4;
}

/// A Ruby object as seen by type validation.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Integer(BigInt),
    Float(f64),
    String(String),
    Symbol(String),
    Time(Time),
    Regexp(Regexp),
    Array(Vec<Value>),
    /// Insertion-ordered pairs; keys are unique under Ruby `eql?`.
    Hash(Vec<(Value, Value)>),
}

impl Value {
    pub fn int(i: i64) -> Self {
        Value::Integer(BigInt::from(i))
    }

    pub fn str(s: impl Into<String>) -> Self {
        Value::String(s.into())
    }

    pub fn sym(s: impl Into<String>) -> Self {
        Value::Symbol(s.into())
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Ruby truthiness: everything except `nil` and `false`.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    /// `Object#class.name`.
    pub fn class_name(&self) -> &'static str {
        match self {
            Value::Nil => "NilClass",
            Value::Bool(true) => "TrueClass",
            Value::Bool(false) => "FalseClass",
            Value::Integer(_) => "Integer",
            Value::Float(_) => "Float",
            Value::String(_) => "String",
            Value::Symbol(_) => "Symbol",
            Value::Time(_) => "Time",
            Value::Regexp(_) => "Regexp",
            Value::Array(_) => "Array",
            Value::Hash(_) => "Hash",
        }
    }

    /// Ruby `Hash#[]` for a Hash value (`eql?` key lookup); `nil` otherwise absent.
    pub fn hash_get(&self, key: &Value) -> Option<&Value> {
        match self {
            Value::Hash(pairs) => pairs.iter().find(|(k, _)| eql(k, key)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `Hash#[]` with a Symbol key, `nil` when absent.
    pub fn sym_get(&self, key: &str) -> Value {
        self.hash_get(&Value::sym(key))
            .cloned()
            .unwrap_or(Value::Nil)
    }

    /// `Hash#key?` with a Symbol key.
    pub fn sym_key(&self, key: &str) -> bool {
        self.hash_get(&Value::sym(key)).is_some()
    }

    /// `hash[:key]`.
    pub fn get_sym(&self, key: &str) -> Option<&Value> {
        self.hash_get(&Value::sym(key))
    }

    /// `hash['key']`.
    pub fn get_str(&self, key: &str) -> Option<&Value> {
        self.hash_get(&Value::str(key))
    }

    /// `hash[:key] || hash['key']` with Ruby truthiness.
    pub fn get_sym_or_str(&self, key: &str) -> Option<&Value> {
        self.get_sym(key)
            .filter(|v| v.truthy())
            .or_else(|| self.get_str(key).filter(|v| v.truthy()))
    }

    /// Ruby `Hash#[]=` (replaces in place, appends when new); a no-op on
    /// anything but a Hash.
    pub fn insert(&mut self, key: Value, value: Value) {
        if let Value::Hash(pairs) = self {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| eql(k, &key)) {
                slot.1 = value;
            } else {
                pairs.push((key, value));
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// `#empty?` for String, Array and Hash; `false` for anything else.
    pub fn is_empty_container(&self) -> bool {
        match self {
            Value::Array(a) => a.is_empty(),
            Value::Hash(h) => h.is_empty(),
            Value::String(s) => s.is_empty(),
            _ => false,
        }
    }

    /// Ruby `#to_s` ([`crate::types::ruby::to_s`]).
    pub fn to_ruby_s(&self) -> String {
        crate::types::ruby::to_s(self)
    }

    /// Ruby `#inspect` ([`crate::types::ruby::inspect`], Ruby 3.4+ Hash style).
    pub fn inspect(&self) -> String {
        crate::types::ruby::inspect(self)
    }

    /// `Kernel#Array`: nil -> [], Array -> itself, Hash -> pairs, else [x].
    pub fn to_array(&self) -> Vec<Value> {
        match self {
            Value::Nil => Vec::new(),
            Value::Array(items) => items.clone(),
            Value::Hash(pairs) => pairs
                .iter()
                .map(|(k, v)| Value::Array(vec![k.clone(), v.clone()]))
                .collect(),
            other => vec![other.clone()],
        }
    }

    /// Whether Ruby stores this value as an immediate ("special constant"):
    /// nil, true, false, Symbols, Floats and Integers in the Fixnum range.
    /// Ruby's comparison error messages inspect these and name the class of
    /// anything else.
    pub(crate) fn is_special_const(&self) -> bool {
        match self {
            Value::Nil | Value::Bool(_) | Value::Symbol(_) | Value::Float(_) => true,
            Value::Integer(i) => {
                let min = -(BigInt::from(1) << 62u32);
                let max = (BigInt::from(1) << 62u32) - 1;
                *i >= min && *i <= max
            }
            _ => false,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_owned())
    }
}

/// Plain data serialization (YAML/JSON): Symbols are written as their name,
/// `Time` and `Regexp` as their `to_s`, Integers outside the i128 range as
/// decimal strings.
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Nil => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Integer(i) => match i64::try_from(i) {
                Ok(v) => serializer.serialize_i64(v),
                Err(_) => match i128::try_from(i) {
                    Ok(v) => serializer.serialize_i128(v),
                    Err(_) => serializer.serialize_str(&i.to_string()),
                },
            },
            Value::Float(f) => serializer.serialize_f64(*f),
            Value::String(s) | Value::Symbol(s) => serializer.serialize_str(s),
            Value::Time(_) | Value::Regexp(_) => serializer.serialize_str(&self.to_ruby_s()),
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Hash(pairs) => {
                let mut map = serializer.serialize_map(Some(pairs.len()))?;
                for (k, v) in pairs {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any YAML value")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }
    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        Value::deserialize(d)
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Integer(BigInt::from(v)))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Integer(BigInt::from(v)))
    }
    fn visit_i128<E>(self, v: i128) -> Result<Value, E> {
        Ok(Value::Integer(BigInt::from(v)))
    }
    fn visit_u128<E>(self, v: u128) -> Result<Value, E> {
        Ok(Value::Integer(BigInt::from(v)))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(item) = seq.next_element::<Value>()? {
            out.push(item);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = Value::Hash(Vec::new());
        while let Some((k, v)) = map.next_entry::<Value, Value>()? {
            out.insert(k, v);
        }
        Ok(out)
    }
    /// A YAML-tagged node (Yeptris visits tags as enums: variant = tag,
    /// payload = the tag-stripped node). The tag is applied the way Psych
    /// applies it ([`apply_yaml_tag`]); it is never kept as a one-entry Hash.
    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Value, A::Error> {
        let (tag, variant): (String, _) = data.variant()?;
        let payload: Value = variant.newtype_variant()?;
        // The marker classifies the failure as `ModelErrorKind::Unsupported`
        // when a model loader reports it.
        apply_yaml_tag(&tag, payload)
            .map_err(|m| de::Error::custom(format!("[ukiryu:unsupported]{m}")))
    }
}

/// Psych's (`YAML.safe_load`) application of a YAML tag to the value the
/// tag-stripped node resolved to. Ruby objects carry no tags: a tag either
/// converts the value or is dropped.
///
/// Applied exactly: local tags (`!name`, other than `!ruby/...`) are dropped;
/// the core `!!str`, `!!int`, `!!float`, `!!bool`, `!!null`, `!!seq` and
/// `!!map` tags when the result does not depend on the scalar's source text.
/// Everything else (for example `!!str` on a plain `123`, whose Psych result
/// needs the original text, `!!binary`, `!!timestamp`, `!!set`, `!!omap`,
/// `!ruby/...` objects, foreign global tags) is reported as unsupported
/// rather than guessed.
pub fn apply_yaml_tag(tag: &str, payload: Value) -> Result<Value, String> {
    const CORE: &str = "tag:yaml.org,2002:";
    let unsupported = |payload: &Value| -> Result<Value, String> {
        Err(format!(
            "YAML tag {tag} on {} is not reproduced",
            payload.class_name()
        ))
    };
    if let Some(name) = tag.strip_prefix(CORE) {
        let unchanged = matches!(
            (name, &payload),
            ("str", Value::String(_))
                | ("seq", Value::Array(_))
                | ("map", Value::Hash(_))
                | ("bool", Value::Bool(_))
                | ("null", Value::Nil)
                | ("int", Value::Integer(_))
                | ("float", Value::Float(_))
        );
        if unchanged {
            return Ok(payload);
        }
        return match (name, &payload) {
            ("null", Value::String(s)) if s.is_empty() => Ok(Value::Nil),
            ("bool", Value::String(s)) if s == "true" => Ok(Value::Bool(true)),
            ("bool", Value::String(s)) if s == "false" => Ok(Value::Bool(false)),
            ("int", Value::String(s)) if is_plain_decimal(s) => {
                let digits = s.strip_prefix('+').unwrap_or(s);
                Ok(Value::Integer(digits.parse().expect("decimal digits")))
            }
            ("float", Value::Integer(i)) => match i64::try_from(i) {
                Ok(n) if n.unsigned_abs() <= 1u64 << 53 => Ok(Value::Float(n as f64)),
                _ => unsupported(&payload),
            },
            _ => unsupported(&payload),
        };
    }
    if tag.starts_with('!') && !tag.starts_with("!ruby/") {
        return Ok(payload);
    }
    unsupported(&payload)
}

/// `-?` / `+?` then `0` or a decimal without leading zeros (Psych reads a
/// leading-zero `!!int` string as octal).
fn is_plain_decimal(s: &str) -> bool {
    let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

/// Ruby `eql?` (also the identity Hash keys and `Array#-` use): no numeric
/// conversion, `1.eql?(1.0)` is false.
pub fn eql(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Integer(x), Value::Integer(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Symbol(x), Value::Symbol(y)) => x == y,
        (Value::Time(x), Value::Time(y)) => x.instant_eql(y),
        (Value::Regexp(x), Value::Regexp(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eql(p, q))
        }
        (Value::Hash(x), Value::Hash(y)) => hash_contents_equal(x, y, eql),
        _ => false,
    }
}

/// Ruby `==`: numeric values compare across Integer and Float.
pub fn ruby_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(_) | Value::Float(_), Value::Integer(_) | Value::Float(_)) => {
            crate::types::ruby::numeric_cmp(a, b) == Some(std::cmp::Ordering::Equal)
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| ruby_eq(p, q))
        }
        (Value::Hash(x), Value::Hash(y)) => hash_contents_equal(x, y, ruby_eq),
        _ => eql(a, b),
    }
}

fn hash_contents_equal(
    x: &[(Value, Value)],
    y: &[(Value, Value)],
    value_eq: fn(&Value, &Value) -> bool,
) -> bool {
    x.len() == y.len()
        && x.iter().all(|(k, v)| {
            y.iter()
                .find(|(k2, _)| eql(k, k2))
                .is_some_and(|(_, v2)| value_eq(v, v2))
        })
}

/// Reads one option the way `Ukiryu::Type` does: `options[:key]`.
///
/// Ruby passes either a Hash (symbol keys) or a definition model whose `#[]`
/// returns the attribute named by the key and `nil` for anything else. Both
/// collapse "absent" and `nil`; `Type` only ever tests truthiness or reads the
/// value.
pub trait OptionSource {
    fn option(&self, key: &str) -> Value;
}

impl OptionSource for Value {
    fn option(&self, key: &str) -> Value {
        self.sym_get(key)
    }
}

/// No options at all (`{}`).
impl OptionSource for () {
    fn option(&self, _key: &str) -> Value {
        Value::Nil
    }
}
