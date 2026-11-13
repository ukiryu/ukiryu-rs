//! Source: lib/ukiryu/models/interface.rb (`Ukiryu::Models::Interface`).
//!
//! A plain frozen Ruby object built from a symbol-keyed Hash; the action
//! list stays raw Ruby data, as in the source.
use super::error::{ModelError, ModelErrorKind};
use super::ruby;
use super::value::RubyValue;
use std::path::Path;

/// An interface contract (`name/version` plus raw `actions`).
#[derive(Clone, Debug, PartialEq)]
pub struct Interface {
    pub name: RubyValue,
    pub version: RubyValue,
    pub display_name: RubyValue,
    pub actions: RubyValue,
    /// Symbols.
    pub aliases: Vec<String>,
}

/// `Psych.safe_load_file` of a register YAML file (String keys).
pub(crate) fn load_yaml_file(path: &Path) -> Result<RubyValue, ModelError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ModelError::new(
            ModelErrorKind::Io,
            format!(
                "No such file or directory @ rb_sysopen - {}: {e}",
                path.display()
            ),
        )
    })?;
    if text.trim().is_empty() {
        return Ok(RubyValue::Nil);
    }
    let value: yeptris::Value = yeptris::from_str(&text)
        .map_err(|e| ModelError::new(ModelErrorKind::Syntax, e.to_string()))?;
    Ok(RubyValue::from_yaml_value(&value))
}

/// The register's effective load path (`Register#load_interface`,
/// `#load_implementation_index`, `#load_implementation_version`):
/// `YAML.safe_load(text)` then `Utils.symbolize_keys`; `None` when the
/// document is empty or null (the register returns nil).
pub fn register_hash(yaml: &str) -> Result<Option<RubyValue>, ModelError> {
    if yaml.trim().is_empty() {
        return Ok(None);
    }
    let value: yeptris::Value = yeptris::from_str(yaml)
        .map_err(|e| ModelError::new(ModelErrorKind::Syntax, e.to_string()))?;
    let data = RubyValue::from_yaml_value(&value);
    if !data.truthy() {
        return Ok(None);
    }
    Ok(Some(data.symbolize_keys()))
}

/// `Interface.symbolize_keys`: `transform_keys(&:to_sym)` recursively
/// (an Integer key raises NoMethodError).
fn symbolize(v: &RubyValue) -> Result<RubyValue, ModelError> {
    match v {
        RubyValue::Hash(pairs) => {
            let mut out = RubyValue::Hash(Vec::new());
            for (k, val) in pairs {
                let key = RubyValue::Sym(k.to_sym()?);
                let val = match val {
                    RubyValue::Hash(_) => symbolize(val)?,
                    RubyValue::Array(items) => RubyValue::Array(
                        items
                            .iter()
                            .map(|x| match x {
                                RubyValue::Hash(_) => symbolize(x),
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    other => other.clone(),
                };
                out.insert(key, val);
            }
            Ok(out)
        }
        other => Ok(other.clone()),
    }
}

fn sym_get(data: &RubyValue, key: &str) -> Result<RubyValue, ModelError> {
    match data {
        RubyValue::Hash(_) => Ok(data.get_sym(key).cloned().unwrap_or(RubyValue::Nil)),
        RubyValue::Nil => Err(ModelError::no_method("[]", data)),
        other => Err(ModelError::no_method("[]", other)),
    }
}

impl Interface {
    /// `Interface.new(name:, version:, actions:, display_name: nil, aliases: [])`.
    pub fn new(
        name: RubyValue,
        version: RubyValue,
        actions: RubyValue,
        display_name: RubyValue,
        aliases: Vec<String>,
    ) -> Interface {
        let display_name = if display_name.truthy() {
            display_name
        } else {
            RubyValue::Str(ruby::capitalize(&name.to_ruby_s()))
        };
        Interface {
            name,
            version,
            display_name,
            actions,
            aliases,
        }
    }

    /// `Interface.from_yaml(path)`.
    pub fn from_yaml(path: &Path) -> Result<Interface, ModelError> {
        let data = load_yaml_file(path)?;
        Interface::from_hash(&symbolize(&data)?)
    }

    /// `Interface.from_hash(data)` (Symbol keys).
    pub fn from_hash(data: &RubyValue) -> Result<Interface, ModelError> {
        let aliases = match sym_get(data, "aliases")? {
            v if v.truthy() => v.to_array(),
            _ => Vec::new(),
        };
        Ok(Interface::new(
            sym_get(data, "name")?,
            sym_get(data, "version")?,
            sym_get(data, "actions")?,
            sym_get(data, "display_name")?,
            aliases
                .iter()
                .map(RubyValue::to_sym)
                .collect::<Result<_, _>>()?,
        ))
    }

    /// `Register#load_interface` minus file lookup:
    /// `Interface.from_hash(Utils.symbolize_keys(YAML.safe_load(yaml)))`.
    pub fn from_register_yaml(yaml: &str) -> Result<Option<Interface>, ModelError> {
        register_hash(yaml)?
            .map(|h| Interface::from_hash(&h))
            .transpose()
    }

    /// `#action(action_name)`: the first action whose `:name` equals it.
    pub fn action(&self, action_name: &RubyValue) -> Result<Option<&RubyValue>, ModelError> {
        match &self.actions {
            RubyValue::Array(items) => Ok(items
                .iter()
                .find(|a| a.get_sym("name").unwrap_or(&RubyValue::Nil) == action_name)),
            other => Err(ModelError::no_method("find", other)),
        }
    }

    /// `#action?(action_name)`
    pub fn has_action(&self, action_name: &RubyValue) -> Result<bool, ModelError> {
        Ok(self.action(action_name)?.is_some())
    }

    /// `#to_s`
    pub fn to_ruby_s(&self) -> String {
        format!("{}/{}", self.name.to_ruby_s(), self.version.to_ruby_s())
    }

    /// `#inspect` (`actions.length` raises when actions is nil).
    pub fn inspect(&self) -> Result<String, ModelError> {
        let n = match &self.actions {
            RubyValue::Array(a) => a.len(),
            RubyValue::Hash(h) => h.len(),
            RubyValue::Str(s) => s.chars().count(),
            other => return Err(ModelError::no_method("length", other)),
        };
        Ok(format!(
            "#<Ukiryu::Models::Interface {} actions={n}>",
            self.to_ruby_s()
        ))
    }
}
