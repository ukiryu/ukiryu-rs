//! Source: lib/ukiryu/models/flag_definition.rb (`Ukiryu::Models::FlagDefinition`).
use super::argument_definition::sym_of;
use super::attr::{de_fns as d, omit_list, omit_opt, Attr, Model};
use super::error::ModelError;
use super::option_definition::applies;
use super::value::Text;
use serde::{Deserialize, Serialize};

fn default_false() -> Attr<bool> {
    Attr::Set(false)
}

/// A boolean command flag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlagDefinition {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub cli: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub cli_short: Option<Text>,
    #[serde(default = "default_false", skip_serializing_if = "Attr::omit")]
    pub default: Attr<bool>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub description: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub platforms: Option<Vec<Text>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub position_constraint: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub position_after: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub conflicts_with: Option<Vec<Text>>,
}

impl Default for FlagDefinition {
    fn default() -> Self {
        FlagDefinition {
            name: None,
            cli: None,
            cli_short: None,
            default: Attr::Default(false),
            description: None,
            platforms: Some(Vec::new()),
            position_constraint: None,
            position_after: None,
            conflicts_with: Some(Vec::new()),
        }
    }
}

impl FlagDefinition {
    /// `#default_value` (`default || false`).
    pub fn default_value(&self) -> bool {
        self.default.get().copied().unwrap_or(false)
    }

    /// `#applies_to?(platform)`
    pub fn applies_to(&self, platform: &str) -> Result<bool, ModelError> {
        applies(&self.platforms, platform)
    }

    /// `#name_sym`
    pub fn name_sym(&self) -> Result<String, ModelError> {
        sym_of(self.name.as_ref())
    }

    /// `#position_constraint_sym` (`nil` stays `nil`).
    pub fn position_constraint_sym(&self) -> Result<Option<String>, ModelError> {
        self.position_constraint
            .as_ref()
            .map(Text::to_sym)
            .transpose()
    }
}

impl Model for FlagDefinition {}
