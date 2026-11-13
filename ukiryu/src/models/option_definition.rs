//! Source: lib/ukiryu/models/option_definition.rb (`Ukiryu::Models::OptionDefinition`).
use super::argument_definition::{int_list_value, opt_value, sym_of, text_list_value};
use super::attr::{de_fns as d, omit_list, omit_opt, ser_int_list, Attr, Model};
use super::error::ModelError;
use super::value::{Text, Value};
use serde::{Deserialize, Serialize};

fn default_type() -> Attr<Text> {
    Attr::Set(Text::new("string"))
}
fn default_delimiter() -> Attr<Text> {
    Attr::Set(Text::new("auto"))
}

/// A command option (`-q 90`, `--quality=90`, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptionDefinition {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub cli: Option<Text>,
    #[serde(default = "default_type", skip_serializing_if = "Attr::omit")]
    pub r#type: Attr<Text>,
    #[serde(default = "default_delimiter", skip_serializing_if = "Attr::omit")]
    pub assignment_delimiter: Attr<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub separator: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub default: Option<Text>,
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
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub of: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub description: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub platforms: Option<Vec<Text>>,
}

impl Default for OptionDefinition {
    fn default() -> Self {
        OptionDefinition {
            name: None,
            cli: None,
            r#type: Attr::Default(Text::new("string")),
            assignment_delimiter: Attr::Default(Text::new("auto")),
            separator: None,
            default: None,
            range: None,
            values: None,
            of: None,
            description: None,
            platforms: Some(Vec::new()),
        }
    }
}

/// `platforms.nil? || platforms.empty?` else membership by Symbol.
pub(crate) fn applies(platforms: &Option<Vec<Text>>, platform: &str) -> Result<bool, ModelError> {
    match platforms {
        None => Ok(true),
        Some(list) if list.is_empty() => Ok(true),
        Some(list) => {
            let syms = list
                .iter()
                .map(Text::to_sym)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(syms.iter().any(|s| s == platform))
        }
    }
}

impl OptionDefinition {
    /// `#applies_to?(platform)`
    pub fn applies_to(&self, platform: &str) -> Result<bool, ModelError> {
        applies(&self.platforms, platform)
    }

    /// `#boolean?`
    pub fn is_boolean(&self) -> bool {
        self.r#type.get().is_some_and(|t| t.eq_str("boolean"))
    }

    /// `#assignment_delimiter_sym`
    pub fn assignment_delimiter_sym(&self) -> Result<String, ModelError> {
        sym_of(self.assignment_delimiter.get())
    }

    /// `#[](key)`: attribute readers by name.
    pub fn index(&self, key: &str) -> Option<Value> {
        Some(match key {
            "name" => opt_value(self.name.as_ref()),
            "cli" => opt_value(self.cli.as_ref()),
            "type" => opt_value(self.r#type.get()),
            "assignment_delimiter" => opt_value(self.assignment_delimiter.get()),
            "separator" => opt_value(self.separator.as_ref()),
            "default" => opt_value(self.default.as_ref()),
            "range" => int_list_value(&self.range),
            "values" => text_list_value(&self.values),
            "of" => opt_value(self.of.as_ref()),
            "description" => opt_value(self.description.as_ref()),
            "platforms" => text_list_value(&self.platforms),
            _ => return None,
        })
    }

    /// `#name_sym`
    pub fn name_sym(&self) -> Result<String, ModelError> {
        sym_of(self.name.as_ref())
    }
}

impl Model for OptionDefinition {}

/// `Ukiryu::Type` reads its options through `OptionDefinition#[]`:
/// attribute readers by name, `nil` for every other key (`min`, `max` and
/// `size` included: the model has no such attributes).
impl crate::types::OptionSource for OptionDefinition {
    fn option(&self, key: &str) -> Value {
        self.index(key).unwrap_or(Value::Nil)
    }
}
