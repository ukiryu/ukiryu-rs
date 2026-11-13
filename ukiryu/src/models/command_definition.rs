//! Source: lib/ukiryu/models/command_definition.rb (`Ukiryu::Models::CommandDefinition`).
use super::argument_definition::ArgumentDefinition;
use super::attr::{de_fns as d, omit_list, omit_model, omit_opt, ser_model_list, Attr, Model};
use super::env_var_definition::EnvVarDefinition;
use super::exit_codes::ExitCodes;
use super::flag_definition::FlagDefinition;
use super::option_definition::OptionDefinition;
use super::value::Text;
use crate::command_builder::ruby_sort_by;
use serde::{Deserialize, Serialize};

fn default_false() -> Attr<bool> {
    Attr::Set(false)
}

/// One command (action) of a platform profile. Fields follow the lutaml
/// mapping order, which is the serialization order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandDefinition {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub description: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub usage: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub subcommand: Option<Text>,
    #[serde(
        default,
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub options: Option<Vec<OptionDefinition>>,
    #[serde(
        default,
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub flags: Option<Vec<FlagDefinition>>,
    #[serde(
        default,
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub arguments: Option<Vec<ArgumentDefinition>>,
    #[serde(
        default,
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub post_options: Option<Vec<OptionDefinition>>,
    #[serde(
        default,
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub env_vars: Option<Vec<EnvVarDefinition>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub exit_codes: Option<Box<ExitCodes>>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub use_env_vars: Option<Vec<Text>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub belongs_to: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub cli_flag: Option<Text>,
    #[serde(default = "default_false", skip_serializing_if = "Attr::omit")]
    pub standalone_executable: Attr<bool>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub aliases: Option<Vec<Text>>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub implements: Option<Vec<Text>>,
}

impl Default for CommandDefinition {
    fn default() -> Self {
        CommandDefinition {
            name: None,
            description: None,
            usage: None,
            subcommand: None,
            options: None,
            flags: None,
            arguments: None,
            post_options: None,
            env_vars: None,
            exit_codes: None,
            use_env_vars: Some(Vec::new()),
            belongs_to: None,
            cli_flag: None,
            standalone_executable: Attr::Default(false),
            aliases: Some(Vec::new()),
            implements: Some(Vec::new()),
        }
    }
}

/// `!x.nil? && !x.empty?`
pub(crate) fn present(v: &Option<Text>) -> bool {
    v.as_ref().is_some_and(|t| !t.is_empty())
}

/// The `to_h { |x| [x.name, x] }[name.to_s]` index: the last duplicate wins.
pub(crate) fn index_lookup<'a, T>(
    items: &'a Option<Vec<T>>,
    name: &str,
    key: impl Fn(&T) -> &Option<Text>,
) -> Option<&'a T> {
    items
        .as_ref()?
        .iter()
        .rev()
        .find(|item| key(item).as_ref().is_some_and(|t| t.eq_str(name)))
}

impl CommandDefinition {
    /// `#belongs_to_command?`
    pub fn is_belongs_to_command(&self) -> bool {
        present(&self.belongs_to)
    }

    /// `#flag_action?`
    pub fn is_flag_action(&self) -> bool {
        present(&self.cli_flag)
    }

    /// `#option(name)`
    pub fn option(&self, name: &str) -> Option<&OptionDefinition> {
        index_lookup(&self.options, name, |o| &o.name)
    }

    /// `#flag(name)`
    pub fn flag(&self, name: &str) -> Option<&FlagDefinition> {
        index_lookup(&self.flags, name, |f| &f.name)
    }

    /// `#argument(name)`
    pub fn argument(&self, name: &str) -> Option<&ArgumentDefinition> {
        index_lookup(&self.arguments, name, |a| &a.name)
    }

    /// `#last_argument`
    pub fn last_argument(&self) -> Option<&ArgumentDefinition> {
        self.arguments.as_ref()?.iter().find(|a| a.is_last())
    }

    /// `#regular_arguments`: non-`last` arguments ordered by
    /// `sort_by(&:numeric_position)`. MRI's `sort_by` is not stable, so
    /// arguments that share a position keep MRI's order, not source order.
    pub fn regular_arguments(&self) -> Vec<&ArgumentDefinition> {
        let args: Vec<&ArgumentDefinition> = self
            .arguments
            .iter()
            .flatten()
            .filter(|a| !a.is_last())
            .collect();
        ruby_sort_by(args, |a| a.numeric_position())
    }

    /// `#has_subcommand?`
    pub fn has_subcommand(&self) -> bool {
        present(&self.subcommand)
    }

    /// `#standalone_executable?`
    pub fn is_standalone_executable(&self) -> bool {
        self.standalone_executable.get() == Some(&true)
    }

    /// `#clear_indexes!`: lookups here are computed on demand, so there is
    /// no cached index to drop.
    pub fn clear_indexes(&mut self) {}
}

impl Model for CommandDefinition {}
