//! Source: lib/ukiryu/models/argument_definition.rb (`Ukiryu::Models::ArgumentDefinition`).
use super::attr::{de_fns as d, omit_list, omit_opt, ser_int_list, Attr, Model};
use super::error::ModelError;
use super::ruby::str_to_bigint;
use super::value::{Text, Value};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

fn default_type() -> Attr<Text> {
    Attr::Set(Text::new("string"))
}
fn default_false() -> Attr<bool> {
    Attr::Set(false)
}
fn default_position() -> Attr<Text> {
    Attr::Set(Text::new("99"))
}
fn default_separator() -> Attr<Text> {
    Attr::Set(Text::new(" "))
}

/// A positional argument of a command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArgumentDefinition {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default = "default_type", skip_serializing_if = "Attr::omit")]
    pub r#type: Attr<Text>,
    #[serde(default = "default_false", skip_serializing_if = "Attr::omit")]
    pub required: Attr<bool>,
    #[serde(default = "default_position", skip_serializing_if = "Attr::omit")]
    pub position: Attr<Text>,
    #[serde(default = "default_false", skip_serializing_if = "Attr::omit")]
    pub variadic: Attr<bool>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub min: Option<i64>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub max: Option<i64>,
    #[serde(
        default,
        deserialize_with = "d::int_list",
        serialize_with = "ser_int_list",
        skip_serializing_if = "omit_list"
    )]
    pub size: Option<Vec<Option<i64>>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub of: Option<Text>,
    #[serde(
        default,
        deserialize_with = "d::int_list",
        serialize_with = "ser_int_list",
        skip_serializing_if = "omit_list"
    )]
    pub range: Option<Vec<Option<i64>>>,
    #[serde(
        default,
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub values: Option<Vec<Text>>,
    #[serde(default = "default_separator", skip_serializing_if = "Attr::omit")]
    pub separator: Attr<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub format: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub description: Option<Text>,
}

impl Default for ArgumentDefinition {
    fn default() -> Self {
        ArgumentDefinition {
            name: None,
            r#type: Attr::Default(Text::new("string")),
            required: Attr::Default(false),
            position: Attr::Default(Text::new("99")),
            variadic: Attr::Default(false),
            min: None,
            max: None,
            size: None,
            of: None,
            range: None,
            values: None,
            separator: Attr::Default(Text::new(" ")),
            format: None,
            description: None,
        }
    }
}

/// `#parsed_position` (`:last`, `:first` or an Integer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedPosition {
    Last,
    First,
    /// Ruby `Integer`: exact at any size, like `position.to_i`.
    Index(BigInt),
}

/// `/^\d+$/` with Ruby line anchors.
pub(crate) fn digits_line(s: &str) -> bool {
    s.split('\n')
        .any(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()))
}

pub(crate) fn opt_value(v: Option<&Text>) -> Value {
    v.map_or(Value::Nil, Text::to_ruby_value)
}

pub(crate) fn int_list_value(v: &Option<Vec<Option<i64>>>) -> Value {
    match v {
        None => Value::Nil,
        Some(items) => Value::Array(
            items
                .iter()
                .map(|i| i.map_or(Value::Nil, Value::int))
                .collect(),
        ),
    }
}

pub(crate) fn text_list_value(v: &Option<Vec<Text>>) -> Value {
    match v {
        None => Value::Nil,
        Some(items) => Value::Array(items.iter().map(Text::to_ruby_value).collect()),
    }
}

pub(crate) fn sym_of(v: Option<&Text>) -> Result<String, ModelError> {
    match v {
        Some(t) => t.to_sym(),
        None => Err(ModelError::no_method("to_sym", &Value::Nil)),
    }
}

impl ArgumentDefinition {
    fn position_str(&self) -> Option<&str> {
        self.position.get().and_then(Text::as_str)
    }

    /// `#last?`
    pub fn is_last(&self) -> bool {
        self.position_str() == Some("last")
    }

    /// `#parsed_position`
    pub fn parsed_position(&self) -> ParsedPosition {
        match self.position_str() {
            Some("last") => ParsedPosition::Last,
            Some("first") => ParsedPosition::First,
            Some(p) if digits_line(p) => ParsedPosition::Index(str_to_bigint(p)),
            _ => ParsedPosition::Index(BigInt::from(99)),
        }
    }

    /// `#numeric_position` (`last` sorts as 999, `first` as 1). Exact at any
    /// size, like Ruby's `position.to_i`.
    pub fn numeric_position(&self) -> BigInt {
        match self.position_str() {
            Some("last") => BigInt::from(999),
            Some("first") => BigInt::from(1),
            Some(p) if digits_line(p) => str_to_bigint(p),
            _ => BigInt::from(99),
        }
    }

    /// `#[](key)`: attribute readers by name (other methods are not exposed).
    pub fn index(&self, key: &str) -> Option<Value> {
        Some(match key {
            "name" => opt_value(self.name.as_ref()),
            "type" => opt_value(self.r#type.get()),
            "required" => self.required.get().map_or(Value::Nil, |b| Value::Bool(*b)),
            "position" => opt_value(self.position.get()),
            "variadic" => self.variadic.get().map_or(Value::Nil, |b| Value::Bool(*b)),
            "min" => self.min.map_or(Value::Nil, Value::int),
            "max" => self.max.map_or(Value::Nil, Value::int),
            "size" => int_list_value(&self.size),
            "of" => opt_value(self.of.as_ref()),
            "range" => int_list_value(&self.range),
            "values" => text_list_value(&self.values),
            "separator" => opt_value(self.separator.get()),
            "format" => opt_value(self.format.as_ref()),
            "description" => opt_value(self.description.as_ref()),
            _ => return None,
        })
    }

    /// `#name_sym`
    pub fn name_sym(&self) -> Result<String, ModelError> {
        sym_of(self.name.as_ref())
    }

    /// `#type_sym`
    pub fn type_sym(&self) -> Result<String, ModelError> {
        sym_of(self.r#type.get())
    }
}

impl Model for ArgumentDefinition {}

/// `Ukiryu::Type` reads its options through `ArgumentDefinition#[]`:
/// attribute readers by name, `nil` for every other key.
impl crate::types::OptionSource for ArgumentDefinition {
    fn option(&self, key: &str) -> Value {
        self.index(key).unwrap_or(Value::Nil)
    }
}
