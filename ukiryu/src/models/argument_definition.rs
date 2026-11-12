//! Source: lib/ukiryu/models/argument_definition.rb (`Ukiryu::Models::ArgumentDefinition`).
use super::attr::{de_fns as d, omit_list, omit_opt, ser_int_list, Attr, Model};
use super::error::ModelError;
use super::value::{RubyValue, Text};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParsedPosition {
    Last,
    First,
    Index(i128),
}

/// `/^\d+$/` with Ruby line anchors.
pub(crate) fn digits_line(s: &str) -> bool {
    s.split('\n')
        .any(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()))
}

pub(crate) fn opt_value(v: Option<&Text>) -> RubyValue {
    v.map_or(RubyValue::Nil, Text::to_ruby_value)
}

pub(crate) fn int_list_value(v: &Option<Vec<Option<i64>>>) -> RubyValue {
    match v {
        None => RubyValue::Nil,
        Some(items) => RubyValue::Array(
            items
                .iter()
                .map(|i| i.map_or(RubyValue::Nil, |i| RubyValue::Int(i128::from(i))))
                .collect(),
        ),
    }
}

pub(crate) fn text_list_value(v: &Option<Vec<Text>>) -> RubyValue {
    match v {
        None => RubyValue::Nil,
        Some(items) => RubyValue::Array(items.iter().map(Text::to_ruby_value).collect()),
    }
}

pub(crate) fn sym_of(v: Option<&Text>) -> Result<String, ModelError> {
    match v {
        Some(t) => t.to_sym(),
        None => Err(ModelError::no_method("to_sym", &RubyValue::Nil)),
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
            Some(p) if digits_line(p) => ParsedPosition::Index(super::ruby::str_to_i(p)),
            _ => ParsedPosition::Index(99),
        }
    }

    /// `#numeric_position` (`last` sorts as 999, `first` as 1).
    pub fn numeric_position(&self) -> i128 {
        match self.position_str() {
            Some("last") => 999,
            Some("first") => 1,
            Some(p) if digits_line(p) => super::ruby::str_to_i(p),
            _ => 99,
        }
    }

    /// `#[](key)`: attribute readers by name (other methods are not exposed).
    pub fn index(&self, key: &str) -> Option<RubyValue> {
        Some(match key {
            "name" => opt_value(self.name.as_ref()),
            "type" => opt_value(self.r#type.get()),
            "required" => self
                .required
                .get()
                .map_or(RubyValue::Nil, |b| RubyValue::Bool(*b)),
            "position" => opt_value(self.position.get()),
            "variadic" => self
                .variadic
                .get()
                .map_or(RubyValue::Nil, |b| RubyValue::Bool(*b)),
            "min" => self
                .min
                .map_or(RubyValue::Nil, |i| RubyValue::Int(i128::from(i))),
            "max" => self
                .max
                .map_or(RubyValue::Nil, |i| RubyValue::Int(i128::from(i))),
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
