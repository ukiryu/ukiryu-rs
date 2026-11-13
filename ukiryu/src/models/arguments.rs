//! Source: lib/ukiryu/models/arguments.rb (`Ukiryu::Models::Arguments`).
use super::argument::Argument;
use super::attr::{de_fns as d, omit_list, ser_model_list, Model};
use super::command_definition::CommandDefinition;
use super::error::ModelError;
use super::value::{RubyValue, Text};
use serde::{Deserialize, Serialize};

/// Structured request arguments of a success response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(
        default = "d::empty",
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub options: Option<Vec<Argument>>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub flags: Option<Vec<Text>>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub positional: Option<Vec<Argument>>,
}

impl Default for Arguments {
    fn default() -> Self {
        Arguments {
            options: Some(Vec::new()),
            flags: Some(Vec::new()),
            positional: Some(Vec::new()),
        }
    }
}

/// `Arguments.determine_argument_type` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentKind {
    Flag,
    Option,
    Positional,
}

fn name_matches(name: &Option<Text>, key: &str) -> Result<bool, ModelError> {
    match name {
        Some(t) => Ok(t.to_sym()? == key),
        None => Err(ModelError::no_method("to_sym", &RubyValue::Nil)),
    }
}

impl Arguments {
    /// `Arguments.from_params(params, command_definition)`; `params` keys are
    /// the Ruby Symbol names, in Hash order.
    pub fn from_params(
        params: &[(String, RubyValue)],
        command_definition: Option<&CommandDefinition>,
    ) -> Result<Arguments, ModelError> {
        let mut options = Vec::new();
        let mut flags = Vec::new();
        let mut positional = Vec::new();
        for (key, value) in params {
            match Self::determine_argument_type(key, value, command_definition)? {
                ArgumentKind::Flag => {
                    if value.truthy() {
                        flags.push(Text::new(key.clone()));
                    }
                }
                ArgumentKind::Option => {
                    options.push(Argument::new(key, &Self::stringify_value(value), "option"))
                }
                ArgumentKind::Positional => positional.push(Argument::new(
                    key,
                    &Self::stringify_value(value),
                    "positional",
                )),
            }
        }
        let mut args = Arguments::default();
        if !options.is_empty() {
            args.options = Some(options);
        }
        if !flags.is_empty() {
            args.flags = Some(flags);
        }
        if !positional.is_empty() {
            args.positional = Some(positional);
        }
        Ok(args)
    }

    /// `Arguments.determine_argument_type(key, value, command_definition)`.
    pub fn determine_argument_type(
        key: &str,
        value: &RubyValue,
        command_definition: Option<&CommandDefinition>,
    ) -> Result<ArgumentKind, ModelError> {
        if let Some(cmd) = command_definition {
            for f in cmd.flags.iter().flatten() {
                if name_matches(&f.name, key)? {
                    return Ok(ArgumentKind::Flag);
                }
            }
            for o in cmd.options.iter().flatten() {
                if name_matches(&o.name, key)? {
                    return Ok(ArgumentKind::Option);
                }
            }
            for a in cmd.arguments.iter().flatten() {
                if name_matches(&a.name, key)? {
                    return Ok(ArgumentKind::Positional);
                }
            }
        }
        Ok(match value {
            RubyValue::Bool(_) => ArgumentKind::Flag,
            _ => ArgumentKind::Option,
        })
    }

    /// `Arguments.stringify_value(value)`.
    pub fn stringify_value(value: &RubyValue) -> String {
        match value {
            RubyValue::Array(items) => items
                .iter()
                .map(RubyValue::to_ruby_s)
                .collect::<Vec<_>>()
                .join(", "),
            other => other.to_ruby_s(),
        }
    }
}

impl Model for Arguments {}
