//! Source: lib/ukiryu/models/tool_definition.rb (`Ukiryu::Models::ToolDefinition`).
use super::attr::{de_fns as d, omit_list, omit_model, omit_opt, ser_model_list, Model};
use super::command_definition::CommandDefinition;
use super::components::Components;
use super::error::ModelError;
use super::invocation::Invocation;
use super::platform_profile::PlatformProfile;
use super::value::{Text, Value};
use super::version_detection::VersionDetection;
use serde::{Deserialize, Serialize};

/// A tool profile document (`ukiryu_schema`, `$self`, profiles, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub ukiryu_schema: Option<Text>,
    /// YAML key `$self`.
    #[serde(
        rename = "$self",
        default,
        deserialize_with = "d::opt",
        skip_serializing_if = "omit_opt"
    )]
    pub self_uri: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub display_name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub homepage: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub version: Option<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub implements: Option<Vec<Text>>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub aliases: Option<Vec<Text>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub invocation: Option<Box<Invocation>>,
    #[serde(
        default,
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub profiles: Option<Vec<PlatformProfile>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub version_detection: Option<Box<VersionDetection>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub components: Option<Box<Components>>,
}

impl Default for ToolDefinition {
    fn default() -> Self {
        ToolDefinition {
            ukiryu_schema: None,
            self_uri: None,
            name: None,
            display_name: None,
            homepage: None,
            version: None,
            implements: Some(Vec::new()),
            aliases: Some(Vec::new()),
            invocation: None,
            profiles: None,
            version_detection: None,
            components: None,
        }
    }
}

impl ToolDefinition {
    /// `#compatible_profile(platform:, shell:)` with explicit values (the
    /// Ruby defaults call platform/shell detection, which is out of scope).
    pub fn compatible_profile(
        &self,
        platform: &str,
        shell: &str,
    ) -> Result<Option<&PlatformProfile>, ModelError> {
        let Some(profiles) = &self.profiles else {
            return Ok(None);
        };
        for p in profiles {
            if p.is_compatible(platform, shell)? {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// `#implements?(interface_name)`
    pub fn is_implements(&self, interface_name: &str) -> Result<bool, ModelError> {
        self.is_implements_any(&[interface_name])
    }

    /// `#implements_any?(*interface_names)`
    pub fn is_implements_any(&self, interface_names: &[&str]) -> Result<bool, ModelError> {
        let Some(implements) = self.implements.as_ref().filter(|v| !v.is_empty()) else {
            return Ok(false);
        };
        let syms = implements
            .iter()
            .map(Text::to_sym)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(interface_names.iter().any(|n| syms.iter().any(|s| s == n)))
    }

    /// `#interfaces` (`implements || []`).
    pub fn interfaces(&self) -> Vec<Text> {
        self.implements.clone().unwrap_or_default()
    }

    /// `#available_on?(platform)` (`profiles.empty?` raises when profiles is nil).
    pub fn is_available_on(&self, platform: &str) -> Result<bool, ModelError> {
        let Some(profiles) = &self.profiles else {
            return Err(ModelError::no_method("empty?", &Value::Nil));
        };
        if profiles.is_empty() {
            return Ok(true);
        }
        for p in profiles {
            if p.supports_platform(platform)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `#resolve_inheritance!`: a child profile gains each parent command it
    /// does not define by name, appended after its own commands.
    pub fn resolve_inheritance(&mut self) -> &mut Self {
        let Some(profiles) = self.profiles.as_mut() else {
            return self;
        };
        for i in 0..profiles.len() {
            let Some(parent_name) = profiles[i].inherits.clone() else {
                continue;
            };
            let Some(parent_idx) = profiles
                .iter()
                .position(|p| p.name.as_ref() == Some(&parent_name))
            else {
                continue;
            };
            let parent_commands: Vec<CommandDefinition> =
                profiles[parent_idx].commands.clone().unwrap_or_default();
            let child_commands: Vec<CommandDefinition> =
                profiles[i].commands.clone().unwrap_or_default();
            let mut merged = child_commands.clone();
            for parent_cmd in parent_commands {
                if !child_commands.iter().any(|c| c.name == parent_cmd.name) {
                    merged.push(parent_cmd);
                }
            }
            profiles[i].commands = Some(merged);
            profiles[i].clear_commands_index();
        }
        self
    }

    /// `#schema_version`
    pub fn schema_version(&self) -> Option<&Text> {
        self.ukiryu_schema.as_ref()
    }

    /// `#schema_version?(version)`
    pub fn is_schema_version(&self, version: &str) -> bool {
        self.ukiryu_schema
            .as_ref()
            .is_some_and(|s| s.eq_str(version))
    }
}

impl Model for ToolDefinition {}
