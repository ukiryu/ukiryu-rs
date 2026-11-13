//! Ruby-side value shapes the models carry.
//!
//! Models hold plain Ruby data (raw YAML hashes, symbolized by
//! `Ukiryu::Utils`) as the crate's single dynamic value type,
//! [`crate::types::Value`]; this module adds the model-side conversions
//! (YAML documents, `Utils.symbolize_keys`, `#to_sym` errors). `Text` mirrors
//! what a lutaml `:string` attribute holds: lutaml keeps the assigned object
//! and only applies `to_s` while serializing.
use super::error::{ModelError, ModelErrorKind};
use num_bigint::BigInt;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub use crate::types::Value;

/// Former name of the models' value type, now the unified
/// [`crate::types::Value`]. Kept so consumers written against
/// `models::value::RubyValue` keep compiling; its variants are those of
/// `Value` (`Integer`, `String`, `Symbol`, ...).
pub type RubyValue = Value;

impl Value {
    /// `#to_sym` (String/Symbol only; anything else raises NoMethodError).
    pub fn to_sym(&self) -> Result<String, ModelError> {
        match self {
            Value::String(s) | Value::Symbol(s) => Ok(s.clone()),
            other => Err(ModelError::no_method("to_sym", other)),
        }
    }

    /// `Ukiryu::Utils.symbolize_keys`: keys become `k.to_s.to_sym`, nested
    /// hashes (directly or as array elements) are symbolized too.
    pub fn symbolize_keys(&self) -> Value {
        match self {
            Value::Hash(pairs) => Value::Hash(
                pairs
                    .iter()
                    .map(|(k, v)| (Value::Symbol(k.to_ruby_s()), symbolize_value(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Converts a YAML document value (string keys, as Psych.safe_load
    /// yields). Tags are applied as Psych applies them
    /// ([`crate::types::value::apply_yaml_tag`]); a tag that conversion does not
    /// reproduce is an `Unsupported` error, never a silently dropped tag.
    pub fn from_yaml_value(value: &yeptris::Value) -> Result<Value, ModelError> {
        use yeptris::Number;
        Ok(match value {
            yeptris::Value::Null => Value::Nil,
            yeptris::Value::Bool(b) => Value::Bool(*b),
            yeptris::Value::Number(Number::I64(i)) => Value::Integer(BigInt::from(*i)),
            yeptris::Value::Number(Number::U64(u)) => Value::Integer(BigInt::from(*u)),
            yeptris::Value::Number(Number::F64(f)) => Value::Float(*f),
            yeptris::Value::String(s) => Value::String(s.clone()),
            yeptris::Value::Sequence(items) => Value::Array(
                items
                    .iter()
                    .map(Value::from_yaml_value)
                    .collect::<Result<_, _>>()?,
            ),
            yeptris::Value::Mapping(m) => {
                let mut out = Value::Hash(Vec::new());
                for (k, v) in &m.0 {
                    out.insert(Value::from_yaml_value(k)?, Value::from_yaml_value(v)?);
                }
                out
            }
            yeptris::Value::Tagged(t) => {
                crate::types::value::apply_yaml_tag(&t.tag, Value::from_yaml_value(&t.value)?)
                    .map_err(|m| ModelError::new(ModelErrorKind::Unsupported, m))?
            }
        })
    }

    /// Converts to a YAML value for serde-driven model loading
    /// (`lutaml from_hash` accepts symbol and string keys alike).
    pub fn to_yaml_value(&self) -> yeptris::Value {
        use yeptris::{Mapping, Number};
        match self {
            Value::Nil => yeptris::Value::Null,
            Value::Bool(b) => yeptris::Value::Bool(*b),
            Value::Integer(i) => match i64::try_from(i) {
                Ok(v) => yeptris::Value::Number(Number::I64(v)),
                Err(_) => match u64::try_from(i) {
                    Ok(v) => yeptris::Value::Number(Number::U64(v)),
                    Err(_) => yeptris::Value::String(i.to_string()),
                },
            },
            Value::Float(f) => yeptris::Value::Number(Number::F64(*f)),
            Value::String(s) | Value::Symbol(s) => yeptris::Value::String(s.clone()),
            Value::Time(_) | Value::Regexp(_) => yeptris::Value::String(self.to_ruby_s()),
            Value::Array(items) => {
                yeptris::Value::Sequence(items.iter().map(Value::to_yaml_value).collect())
            }
            Value::Hash(pairs) => {
                let mut m = Mapping::new();
                for (k, v) in pairs {
                    m.insert(k.to_yaml_value(), v.to_yaml_value());
                }
                yeptris::Value::Mapping(m)
            }
        }
    }
}

fn symbolize_value(v: &Value) -> Value {
    match v {
        Value::Hash(_) => v.symbolize_keys(),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|x| match x {
                    Value::Hash(_) => x.symbolize_keys(),
                    other => other.clone(),
                })
                .collect(),
        ),
        other => other.clone(),
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
    List(Vec<Value>),
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

    pub fn to_ruby_value(&self) -> Value {
        match self {
            Text::Str(s) => Value::String(s.clone()),
            Text::Nil => Value::Nil,
            Text::List(items) => Value::Array(items.clone()),
        }
    }

    /// Lutaml's cast of one scalar (`to_s`).
    pub fn from_scalar(v: &Value) -> Text {
        match v {
            Value::Nil => Text::Nil,
            other => Text::Str(other.to_ruby_s()),
        }
    }

    /// Builds a `Text` from a Ruby value assigned to a single `:string` attribute.
    pub fn from_ruby_value(v: &Value) -> Result<Option<Text>, ModelError> {
        Ok(Some(match v {
            Value::Nil => return Ok(None),
            Value::Array(items) => Text::List(items.clone()),
            Value::Hash(_) => return Err(unsupported_map()),
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
        let v = Value::deserialize(d)?;
        Text::from_ruby_value(&v)
            .map(|t| t.unwrap_or(Text::Nil))
            .map_err(|e| de::Error::custom(e.kind.tag(&e.message)))
    }
}

/// One element of a `:string, collection: true` attribute: every element
/// is cast with `to_s` (an Array element becomes its `inspect` text).
pub(crate) fn text_item(v: &Value) -> Result<Text, ModelError> {
    match v {
        Value::Nil => Ok(Text::Nil),
        Value::Hash(_) => Err(ModelError::new(
            ModelErrorKind::Unsupported,
            "a Hash element in a :string collection is kept raw by lutaml; not representable",
        )),
        other => Ok(Text::Str(other.to_ruby_s())),
    }
}
