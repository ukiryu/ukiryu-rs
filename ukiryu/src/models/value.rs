//! Ruby-side value shapes the models carry.
//!
//! `RubyValue` mirrors the plain Ruby data the non-lutaml models hold as
//! instance variables (raw YAML hashes, symbolized by `Ukiryu::Utils`).
//! `Text` mirrors what a lutaml `:string` attribute holds: lutaml keeps
//! the assigned object and only applies `to_s` while serializing.
use super::error::{ModelError, ModelErrorKind};
use super::ruby;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A Ruby object loaded from YAML (Psych) or built by Ukiryu helpers.
#[derive(Clone, Debug, PartialEq)]
pub enum RubyValue {
    Nil,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Sym(String),
    Array(Vec<RubyValue>),
    /// Ordered Ruby Hash; keys keep their Ruby class (String, Symbol, Integer).
    Hash(Vec<(RubyValue, RubyValue)>),
}

impl RubyValue {
    pub fn str(s: impl Into<String>) -> Self {
        RubyValue::Str(s.into())
    }

    pub fn sym(s: impl Into<String>) -> Self {
        RubyValue::Sym(s.into())
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, RubyValue::Nil)
    }

    /// Ruby truthiness (`nil` and `false` are falsy).
    pub fn truthy(&self) -> bool {
        !matches!(self, RubyValue::Nil | RubyValue::Bool(false))
    }

    /// `hash[key]` for a Hash; `None` (Ruby `nil`) when absent or not a Hash.
    pub fn get(&self, key: &RubyValue) -> Option<&RubyValue> {
        match self {
            RubyValue::Hash(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `hash[:key]`.
    pub fn get_sym(&self, key: &str) -> Option<&RubyValue> {
        self.get(&RubyValue::Sym(key.to_owned()))
    }

    /// `hash['key']`.
    pub fn get_str(&self, key: &str) -> Option<&RubyValue> {
        self.get(&RubyValue::Str(key.to_owned()))
    }

    /// `hash[:key] || hash['key']` with Ruby truthiness.
    pub fn get_sym_or_str(&self, key: &str) -> Option<&RubyValue> {
        self.get_sym(key)
            .filter(|v| v.truthy())
            .or_else(|| self.get_str(key).filter(|v| v.truthy()))
    }

    /// Ruby `Hash#[]=` (replaces in place, appends when new).
    pub fn insert(&mut self, key: RubyValue, value: RubyValue) {
        if let RubyValue::Hash(pairs) = self {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                pairs.push((key, value));
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            RubyValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[RubyValue]> {
        match self {
            RubyValue::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_empty_container(&self) -> bool {
        match self {
            RubyValue::Array(a) => a.is_empty(),
            RubyValue::Hash(h) => h.is_empty(),
            RubyValue::Str(s) => s.is_empty(),
            _ => false,
        }
    }

    /// Ruby `#to_s`.
    pub fn to_ruby_s(&self) -> String {
        match self {
            RubyValue::Nil => String::new(),
            RubyValue::Str(s) | RubyValue::Sym(s) => s.clone(),
            other => other.inspect(),
        }
    }

    /// Ruby `#inspect` (Ruby 3.4+ Hash style).
    pub fn inspect(&self) -> String {
        match self {
            RubyValue::Nil => "nil".into(),
            RubyValue::Bool(b) => b.to_string(),
            RubyValue::Int(i) => i.to_string(),
            RubyValue::Float(f) => ruby::float_to_s(*f),
            RubyValue::Str(s) => ruby::string_inspect(s),
            RubyValue::Sym(s) => format!(":{s}"),
            RubyValue::Array(items) => {
                let parts: Vec<String> = items.iter().map(RubyValue::inspect).collect();
                format!("[{}]", parts.join(", "))
            }
            RubyValue::Hash(pairs) => {
                if pairs.is_empty() {
                    return "{}".into();
                }
                let parts: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| match k {
                        RubyValue::Sym(s) => format!("{s}: {}", v.inspect()),
                        _ => format!("{} => {}", k.inspect(), v.inspect()),
                    })
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
        }
    }

    /// `Kernel#Array`: nil → [], Array → itself, Hash → pairs, else [x].
    pub fn to_array(&self) -> Vec<RubyValue> {
        match self {
            RubyValue::Nil => Vec::new(),
            RubyValue::Array(items) => items.clone(),
            RubyValue::Hash(pairs) => pairs
                .iter()
                .map(|(k, v)| RubyValue::Array(vec![k.clone(), v.clone()]))
                .collect(),
            other => vec![other.clone()],
        }
    }

    /// `#to_sym` (String/Symbol only; anything else raises NoMethodError).
    pub fn to_sym(&self) -> Result<String, ModelError> {
        match self {
            RubyValue::Str(s) | RubyValue::Sym(s) => Ok(s.clone()),
            other => Err(ModelError::no_method("to_sym", other)),
        }
    }

    /// Ruby class name, for NoMethodError messages.
    pub fn class_name(&self) -> &'static str {
        match self {
            RubyValue::Nil => "nil",
            RubyValue::Bool(true) => "true",
            RubyValue::Bool(false) => "false",
            RubyValue::Int(_) => "Integer",
            RubyValue::Float(_) => "Float",
            RubyValue::Str(_) => "String",
            RubyValue::Sym(_) => "Symbol",
            RubyValue::Array(_) => "Array",
            RubyValue::Hash(_) => "Hash",
        }
    }

    /// `Ukiryu::Utils.symbolize_keys`: keys become `k.to_s.to_sym`, nested
    /// hashes (directly or as array elements) are symbolized too.
    pub fn symbolize_keys(&self) -> RubyValue {
        match self {
            RubyValue::Hash(pairs) => RubyValue::Hash(
                pairs
                    .iter()
                    .map(|(k, v)| (RubyValue::Sym(k.to_ruby_s()), symbolize_value(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Converts a YAML document value (string keys, as Psych.safe_load yields).
    pub fn from_yaml_value(value: &yeptris::Value) -> RubyValue {
        use yeptris::{Number, Value};
        match value {
            Value::Null => RubyValue::Nil,
            Value::Bool(b) => RubyValue::Bool(*b),
            Value::Number(Number::I64(i)) => RubyValue::Int(i128::from(*i)),
            Value::Number(Number::U64(u)) => RubyValue::Int(i128::from(*u)),
            Value::Number(Number::F64(f)) => RubyValue::Float(*f),
            Value::String(s) => RubyValue::Str(s.clone()),
            Value::Sequence(items) => {
                RubyValue::Array(items.iter().map(RubyValue::from_yaml_value).collect())
            }
            Value::Mapping(m) => {
                let mut out = RubyValue::Hash(Vec::new());
                for (k, v) in &m.0 {
                    out.insert(RubyValue::from_yaml_value(k), RubyValue::from_yaml_value(v));
                }
                out
            }
            Value::Tagged(t) => RubyValue::from_yaml_value(&t.value),
        }
    }

    /// Converts to a YAML value for serde-driven model loading
    /// (`lutaml from_hash` accepts symbol and string keys alike).
    pub fn to_yaml_value(&self) -> yeptris::Value {
        use yeptris::{Mapping, Number, Value};
        match self {
            RubyValue::Nil => Value::Null,
            RubyValue::Bool(b) => Value::Bool(*b),
            RubyValue::Int(i) => match i64::try_from(*i) {
                Ok(v) => Value::Number(Number::I64(v)),
                Err(_) => match u64::try_from(*i) {
                    Ok(v) => Value::Number(Number::U64(v)),
                    Err(_) => Value::String(i.to_string()),
                },
            },
            RubyValue::Float(f) => Value::Number(Number::F64(*f)),
            RubyValue::Str(s) | RubyValue::Sym(s) => Value::String(s.clone()),
            RubyValue::Array(items) => {
                Value::Sequence(items.iter().map(RubyValue::to_yaml_value).collect())
            }
            RubyValue::Hash(pairs) => {
                let mut m = Mapping::new();
                for (k, v) in pairs {
                    m.insert(k.to_yaml_value(), v.to_yaml_value());
                }
                Value::Mapping(m)
            }
        }
    }
}

fn symbolize_value(v: &RubyValue) -> RubyValue {
    match v {
        RubyValue::Hash(_) => v.symbolize_keys(),
        RubyValue::Array(items) => RubyValue::Array(
            items
                .iter()
                .map(|x| match x {
                    RubyValue::Hash(_) => x.symbolize_keys(),
                    other => other.clone(),
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

impl From<&str> for RubyValue {
    fn from(s: &str) -> Self {
        RubyValue::Str(s.to_owned())
    }
}

impl Serialize for RubyValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RubyValue::Nil => serializer.serialize_unit(),
            RubyValue::Bool(b) => serializer.serialize_bool(*b),
            RubyValue::Int(i) => match i64::try_from(*i) {
                Ok(v) => serializer.serialize_i64(v),
                Err(_) => serializer.serialize_i128(*i),
            },
            RubyValue::Float(f) => serializer.serialize_f64(*f),
            RubyValue::Str(s) | RubyValue::Sym(s) => serializer.serialize_str(s),
            RubyValue::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            RubyValue::Hash(pairs) => {
                let mut map = serializer.serialize_map(Some(pairs.len()))?;
                for (k, v) in pairs {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

struct RubyValueVisitor;

impl<'de> Visitor<'de> for RubyValueVisitor {
    type Value = RubyValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any YAML value")
    }
    fn visit_unit<E>(self) -> Result<RubyValue, E> {
        Ok(RubyValue::Nil)
    }
    fn visit_none<E>(self) -> Result<RubyValue, E> {
        Ok(RubyValue::Nil)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<RubyValue, D::Error> {
        RubyValue::deserialize(d)
    }
    fn visit_bool<E>(self, v: bool) -> Result<RubyValue, E> {
        Ok(RubyValue::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<RubyValue, E> {
        Ok(RubyValue::Int(i128::from(v)))
    }
    fn visit_u64<E>(self, v: u64) -> Result<RubyValue, E> {
        Ok(RubyValue::Int(i128::from(v)))
    }
    fn visit_i128<E>(self, v: i128) -> Result<RubyValue, E> {
        Ok(RubyValue::Int(v))
    }
    fn visit_f64<E>(self, v: f64) -> Result<RubyValue, E> {
        Ok(RubyValue::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<RubyValue, E> {
        Ok(RubyValue::Str(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<RubyValue, E> {
        Ok(RubyValue::Str(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<RubyValue, A::Error> {
        let mut out = Vec::new();
        while let Some(item) = seq.next_element::<RubyValue>()? {
            out.push(item);
        }
        Ok(RubyValue::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RubyValue, A::Error> {
        let mut out = RubyValue::Hash(Vec::new());
        while let Some((k, v)) = map.next_entry::<RubyValue, RubyValue>()? {
            out.insert(k, v);
        }
        Ok(out)
    }
}

impl<'de> Deserialize<'de> for RubyValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<RubyValue, D::Error> {
        d.deserialize_any(RubyValueVisitor)
    }
}

/// The object held by a lutaml `:string` attribute.
///
/// Lutaml casts scalars with `to_s` when they are assigned (`position: 1`
/// reads back as `"1"`), but keeps a YAML sequence assigned to a single
/// `:string` attribute as an Array and only writes `Array#to_s` on output.
#[derive(Clone, Debug, PartialEq)]
pub enum Text {
    Str(String),
    /// A raw Array held by a single-valued `:string` attribute (elements
    /// keep their Ruby classes).
    List(Vec<RubyValue>),
    /// A `nil` element of a `:string` collection.
    Nil,
}

impl Text {
    pub fn new(s: impl Into<String>) -> Self {
        Text::Str(s.into())
    }

    /// The String value when the attribute holds a Ruby String.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Text::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Ruby `#to_s` (what lutaml writes on output).
    pub fn to_ruby_s(&self) -> String {
        match self {
            Text::Str(s) => s.clone(),
            Text::Nil => String::new(),
            Text::List(_) => self.inspect(),
        }
    }

    pub fn inspect(&self) -> String {
        self.to_ruby_value().inspect()
    }

    /// Ruby `#to_sym` (String only; an Array raises NoMethodError).
    pub fn to_sym(&self) -> Result<String, ModelError> {
        match self {
            Text::Str(s) => Ok(s.clone()),
            other => Err(ModelError::no_method("to_sym", &other.to_ruby_value())),
        }
    }

    /// Ruby `#empty?`.
    pub fn is_empty(&self) -> bool {
        match self {
            Text::Str(s) => s.is_empty(),
            Text::List(items) => items.is_empty(),
            Text::Nil => true,
        }
    }

    /// Ruby `==` against a String literal.
    pub fn eq_str(&self, other: &str) -> bool {
        matches!(self, Text::Str(s) if s == other)
    }

    pub fn to_ruby_value(&self) -> RubyValue {
        match self {
            Text::Str(s) => RubyValue::Str(s.clone()),
            Text::Nil => RubyValue::Nil,
            Text::List(items) => RubyValue::Array(items.clone()),
        }
    }

    /// Lutaml's cast of one scalar (`to_s`).
    pub fn from_scalar(v: &RubyValue) -> Text {
        match v {
            RubyValue::Nil => Text::Nil,
            other => Text::Str(other.to_ruby_s()),
        }
    }

    /// Builds a `Text` from a Ruby value assigned to a single `:string` attribute.
    pub fn from_ruby_value(v: &RubyValue) -> Result<Option<Text>, ModelError> {
        Ok(Some(match v {
            RubyValue::Nil => return Ok(None),
            RubyValue::Array(items) => Text::List(items.clone()),
            RubyValue::Hash(_) => return Err(unsupported_map()),
            other => Text::from_scalar(other),
        }))
    }
}

fn unsupported_map() -> ModelError {
    ModelError::new(
        ModelErrorKind::Unsupported,
        "a Hash assigned to a :string attribute is kept raw by lutaml; not representable",
    )
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::Str(s.to_owned())
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Text::Str(s)
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_ruby_s())
    }
}

impl Serialize for Text {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_ruby_s())
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Text, D::Error> {
        let v = RubyValue::deserialize(d)?;
        Text::from_ruby_value(&v)
            .map(|t| t.unwrap_or(Text::Nil))
            .map_err(|e| de::Error::custom(e.kind.tag(&e.message)))
    }
}

/// One element of a `:string, collection: true` attribute: every element
/// is cast with `to_s` (an Array element becomes its `inspect` text).
pub(crate) fn text_item(v: &RubyValue) -> Result<Text, ModelError> {
    match v {
        RubyValue::Nil => Ok(Text::Nil),
        RubyValue::Hash(_) => Err(ModelError::new(
            ModelErrorKind::Unsupported,
            "a Hash element in a :string collection is kept raw by lutaml; not representable",
        )),
        other => Ok(Text::Str(other.to_ruby_s())),
    }
}
