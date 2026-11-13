//! Source: lib/ukiryu/models/implementation_version.rb
//! (`Ukiryu::Models::ImplementationVersion`, `Ukiryu::Models::ExecutionProfile`).
//!
//! Execution profiles stay raw Ruby Hashes, as in the source. `from_yaml`
//! hands Psych's String-keyed Hash to `from_hash`, whose Symbol lookups then
//! miss (only the String fallbacks hit); the port keeps that behaviour.
//! The `UKIRYU_DEBUG_EXECUTABLE`/`CI` stderr tracing is not reproduced.
use super::attr::Model;
use super::error::ModelError;
use super::interface::{load_yaml_file, register_hash};
use super::value::Value;
use super::version_detection::VersionDetection;
use std::path::Path;

/// One implementation version file (`<impl>/<version>.yaml`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImplementationVersion {
    pub implements: Value,
    pub version: Value,
    pub display_name: Value,
    /// `Array(aliases).map(&:to_s)`.
    pub aliases: Vec<String>,
    pub execution_profiles: Value,
    pub version_detection: Option<VersionDetection>,
}

fn sym_get(data: &Value, key: &str) -> Result<Value, ModelError> {
    match data {
        Value::Hash(_) => Ok(data.get_sym(key).cloned().unwrap_or(Value::Nil)),
        other => Err(ModelError::no_method("[]", other)),
    }
}

fn either(data: &Value, key: &str) -> Option<Value> {
    data.get_sym_or_str(key).cloned()
}

fn profiles(list: &Value) -> Result<&[Value], ModelError> {
    match list {
        Value::Array(items) => Ok(items),
        other => Err(ModelError::no_method("find", other)),
    }
}

fn index(prof: &Value, key: &str) -> Result<Value, ModelError> {
    match prof {
        Value::Hash(_) => Ok(prof.get_sym(key).cloned().unwrap_or(Value::Nil)),
        other => Err(ModelError::no_method("[]", other)),
    }
}

impl ImplementationVersion {
    /// `ImplementationVersion.new(implements:, version:, execution_profiles:, display_name: nil, aliases: [], version_detection: nil)`.
    pub fn new(
        implements: Value,
        version: Value,
        execution_profiles: Value,
        display_name: Value,
        aliases: &Value,
        version_detection: Option<VersionDetection>,
    ) -> ImplementationVersion {
        let display_name = if display_name.truthy() {
            display_name
        } else {
            Value::String(format!(
                "{} {}",
                implements.to_ruby_s(),
                version.to_ruby_s()
            ))
        };
        ImplementationVersion {
            aliases: aliases.to_array().iter().map(Value::to_ruby_s).collect(),
            implements,
            version,
            display_name,
            execution_profiles,
            version_detection,
        }
    }

    /// `ImplementationVersion.from_yaml(path)`.
    pub fn from_yaml(path: &Path) -> Result<ImplementationVersion, ModelError> {
        ImplementationVersion::from_hash(&load_yaml_file(path)?)
    }

    /// `ImplementationVersion.from_hash(data)`.
    pub fn from_hash(data: &Value) -> Result<ImplementationVersion, ModelError> {
        let version_detection = match either(data, "version_detection") {
            Some(vd) => Some(VersionDetection::from_ruby(&vd)?),
            None => None,
        };
        let profiles = either(data, "execution_profiles").unwrap_or(Value::Array(Vec::new()));
        let aliases = match sym_get(data, "aliases")? {
            v if v.truthy() => v,
            _ => Value::Array(Vec::new()),
        };
        Ok(ImplementationVersion::new(
            sym_get(data, "implements")?,
            sym_get(data, "version")?,
            profiles,
            sym_get(data, "display_name")?,
            &aliases,
            version_detection,
        ))
    }

    /// `Register#load_implementation_version` minus file lookup.
    pub fn from_register_yaml(yaml: &str) -> Result<Option<ImplementationVersion>, ModelError> {
        register_hash(yaml)?
            .map(|h| ImplementationVersion::from_hash(&h))
            .transpose()
    }

    /// `#compatible_profile(platform:, shell:)`: the first profile listing
    /// both, with the parent's commands merged in when it inherits and has none.
    pub fn compatible_profile(
        &self,
        platform: &str,
        shell: &str,
    ) -> Result<Option<Value>, ModelError> {
        let list = profiles(&self.execution_profiles)?;
        let mut found = None;
        for prof in list {
            let platforms = match index(prof, "platforms")? {
                v if v.truthy() => v,
                _ => index(prof, "platform")?,
            };
            let shells = match index(prof, "shells")? {
                v if v.truthy() => v,
                _ => index(prof, "shell")?,
            };
            let has = |v: &Value, want: &str| v.to_array().iter().any(|x| x.to_ruby_s() == want);
            if has(&platforms, platform) && has(&shells, shell) {
                found = Some(prof.clone());
                break;
            }
        }
        let Some(mut profile) = found else {
            return Ok(None);
        };
        if let Some(inherits) = either(&profile, "inherits") {
            let parent = list.iter().find(|p| {
                either(p, "name").unwrap_or(Value::Nil).to_ruby_s() == inherits.to_ruby_s()
            });
            let own = index(&profile, "commands")?;
            if let Some(parent) = parent {
                if own.is_nil() || own.is_empty_container() {
                    let parent_commands = either(parent, "commands").unwrap_or(Value::Nil);
                    profile.insert(Value::sym("commands"), parent_commands);
                }
            }
        }
        Ok(Some(profile))
    }

    /// `#profile(profile_name)`
    pub fn profile(&self, profile_name: &Value) -> Result<Option<&Value>, ModelError> {
        Ok(profiles(&self.execution_profiles)?
            .iter()
            .find(|p| p.get_sym("name").unwrap_or(&Value::Nil) == profile_name))
    }

    /// `#to_s`
    pub fn to_ruby_s(&self) -> String {
        format!(
            "{} {}",
            self.implements.to_ruby_s(),
            self.version.to_ruby_s()
        )
    }

    /// `#inspect`
    pub fn inspect(&self) -> Result<String, ModelError> {
        Ok(format!(
            "#<Ukiryu::Models::ImplementationVersion {} profiles={}>",
            self.to_ruby_s(),
            profiles(&self.execution_profiles)?.len()
        ))
    }
}

/// `Ukiryu::Models::ExecutionProfile` (platforms, shells and option style
/// as Symbols; actions keyed by Symbol).
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionProfile {
    pub name: Value,
    pub platforms: Vec<String>,
    pub shells: Vec<String>,
    pub option_style: String,
    pub executable_name: Value,
    pub actions: Value,
}

impl ExecutionProfile {
    /// `ExecutionProfile.new(name:, platforms:, shells:, option_style:, executable_name:, actions: {})`.
    pub fn new(
        name: Value,
        platforms: &Value,
        shells: &Value,
        option_style: &Value,
        executable_name: Value,
        actions: &Value,
    ) -> Result<ExecutionProfile, ModelError> {
        let syms = |v: &Value| -> Result<Vec<String>, ModelError> {
            v.to_array().iter().map(Value::to_sym).collect()
        };
        let actions = match actions {
            Value::Hash(pairs) => Value::Hash(
                pairs
                    .iter()
                    .map(|(k, v)| Ok((Value::Symbol(k.to_sym()?), v.clone())))
                    .collect::<Result<_, ModelError>>()?,
            ),
            other => return Err(ModelError::no_method("transform_keys", other)),
        };
        Ok(ExecutionProfile {
            name,
            platforms: syms(platforms)?,
            shells: syms(shells)?,
            option_style: option_style.to_sym()?,
            executable_name,
            actions,
        })
    }

    /// `#compatible?(platform, shell)`: Symbol membership, so a String
    /// argument never matches.
    pub fn is_compatible(&self, platform: &Value, shell: &Value) -> bool {
        let member = |list: &[String], v: &Value| match v {
            Value::Symbol(s) => list.iter().any(|x| x == s),
            _ => false,
        };
        member(&self.platforms, platform) && member(&self.shells, shell)
    }

    /// `#command(action_name)`
    pub fn command(&self, action_name: &Value) -> Option<&Value> {
        self.actions.hash_get(action_name)
    }

    /// `#commands`
    pub fn commands(&self) -> &Value {
        &self.actions
    }

    /// `#to_s`
    pub fn to_ruby_s(&self) -> String {
        format!(
            "{} ({} / {})",
            self.name.to_ruby_s(),
            self.platforms.join(","),
            self.shells.join(",")
        )
    }

    /// `#inspect` (interpolates the never-assigned `@to_s`, i.e. nil).
    pub fn inspect(&self) -> String {
        "#<Ukiryu::Models::ExecutionProfile >".into()
    }
}
