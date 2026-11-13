//! Source: lib/ukiryu/models/env_var_definition.rb (`Ukiryu::Models::EnvVarDefinition`).
use super::attr::{de_fns as d, omit_list, omit_opt, Model};
use super::error::ModelError;
use super::value::Text;
use serde::{Deserialize, Serialize};

/// An environment variable a command sets (literal `value` or copied `from`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvVarDefinition {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub value: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub from: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub platforms: Option<Vec<Text>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub description: Option<Text>,
}

impl Default for EnvVarDefinition {
    fn default() -> Self {
        EnvVarDefinition {
            name: None,
            value: None,
            from: None,
            platforms: Some(Vec::new()),
            description: None,
        }
    }
}

impl EnvVarDefinition {
    /// `#platforms_sym` (`nil` platforms read as `[]`).
    pub fn platforms_sym(&self) -> Result<Vec<String>, ModelError> {
        self.platforms.iter().flatten().map(Text::to_sym).collect()
    }
}

impl Model for EnvVarDefinition {}
