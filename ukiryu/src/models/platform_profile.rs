//! Source: lib/ukiryu/models/platform_profile.rb (`Ukiryu::Models::PlatformProfile`).
use super::attr::{de_fns as d, omit_list, omit_model, omit_opt, ser_model_list, Attr, Model};
use super::command_definition::CommandDefinition;
use super::error::ModelError;
use super::exit_codes::ExitCodes;
use super::routing::Routing;
use super::value::{Text, Value};
use serde::{Deserialize, Serialize};

fn default_option_style() -> Attr<Text> {
    Attr::Set(Text::new("single_dash_space"))
}
fn default_hash() -> Attr<Value> {
    Attr::Set(Value::Hash(Vec::new()))
}

/// The commands of a tool on a set of platforms and shells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlatformProfile {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub display_name: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub platforms: Option<Vec<Text>>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub shells: Option<Vec<Text>>,
    #[serde(default = "default_option_style", skip_serializing_if = "Attr::omit")]
    pub option_style: Attr<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub executable_name: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub commands: Option<Vec<CommandDefinition>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub inherits: Option<Text>,
    /// Raw routing table (YAML key `routing`).
    #[serde(
        rename = "routing",
        default = "default_hash",
        skip_serializing_if = "Attr::omit"
    )]
    pub routing_data: Attr<Value>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub version_requirement: Option<Text>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub exit_codes: Option<Box<ExitCodes>>,
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub env_var_sets: Attr<Value>,
    /// The memoized `#routing` object.
    #[serde(skip)]
    routing_cache: std::cell::OnceCell<Routing>,
}

impl Default for PlatformProfile {
    fn default() -> Self {
        PlatformProfile {
            name: None,
            display_name: None,
            platforms: Some(Vec::new()),
            shells: Some(Vec::new()),
            option_style: Attr::Default(Text::new("single_dash_space")),
            executable_name: None,
            commands: Some(Vec::new()),
            inherits: None,
            routing_data: Attr::Default(Value::Hash(Vec::new())),
            version_requirement: None,
            exit_codes: None,
            env_var_sets: Attr::Default(Value::Hash(Vec::new())),
            routing_cache: std::cell::OnceCell::new(),
        }
    }
}

/// `list.nil? || list.empty? || list.map(&:to_sym).include?(value)`.
fn supports(list: &Option<Vec<Text>>, value: &str) -> Result<bool, ModelError> {
    match list {
        None => Ok(true),
        Some(items) => {
            let syms = items
                .iter()
                .map(Text::to_sym)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(syms.is_empty() || syms.iter().any(|s| s == value))
        }
    }
}

impl PlatformProfile {
    /// `#routing` (memoized `Routing.new(routing_data || {})`).
    pub fn routing(&self) -> Routing {
        self.routing_cache
            .get_or_init(|| {
                Routing::new(self.routing_data.get().unwrap_or(&Value::Hash(Vec::new())))
            })
            .clone()
    }

    /// `#routing?`
    pub fn has_routing(&self) -> bool {
        self.routing_data
            .get()
            .is_some_and(|r| !r.is_empty_container())
    }

    /// `#supports_platform?(platform)`
    pub fn supports_platform(&self, platform: &str) -> Result<bool, ModelError> {
        supports(&self.platforms, platform)
    }

    /// `#supports_shell?(shell)`
    pub fn supports_shell(&self, shell: &str) -> Result<bool, ModelError> {
        supports(&self.shells, shell)
    }

    /// `#compatible?(platform, shell)`
    pub fn is_compatible(&self, platform: &str, shell: &str) -> Result<bool, ModelError> {
        Ok(self.supports_platform(platform)? && self.supports_shell(shell)?)
    }

    /// `#command(name)` (the last command with that name wins).
    pub fn command(&self, name: &str) -> Option<&CommandDefinition> {
        super::command_definition::index_lookup(&self.commands, name, |c| &c.name)
    }

    /// `#command_names` (unique names, first-seen order).
    pub fn command_names(&self) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for c in self.commands.iter().flatten() {
            let key = c.name.as_ref().map_or(Value::Nil, Text::to_ruby_value);
            if !out.contains(&key) {
                out.push(key);
            }
        }
        out
    }

    /// `#universal?` (no platform and no shell restriction).
    pub fn is_universal(&self) -> bool {
        self.platforms.as_ref().is_none_or(Vec::is_empty)
            && self.shells.as_ref().is_none_or(Vec::is_empty)
    }

    /// `#clear_commands_index!`: lookups are computed on demand, so this
    /// only exists for API parity.
    pub fn clear_commands_index(&mut self) {}
}

impl Model for PlatformProfile {}
