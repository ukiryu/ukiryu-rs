//! Source: lib/ukiryu/models/command_info.rb (`Ukiryu::Models::CommandInfo`).
use super::arguments::Arguments;
use super::attr::{de_fns as d, omit_model, omit_opt, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

/// The executed command of a success response.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandInfo {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub executable: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub executable_name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub tool_name: Option<Text>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub arguments: Option<Box<Arguments>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub full_command: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub shell: Option<Text>,
}

impl Model for CommandInfo {}
