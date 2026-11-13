//! Source: lib/ukiryu/models/interface.rb (`Ukiryu::Models::Interface`).
//!
//! A plain frozen Ruby object built from a symbol-keyed Hash; the action
//! list stays raw Ruby data, as in the source.
use super::error::{ModelError, ModelErrorKind};
use super::ruby;
use super::safe_load::{safe_load, LoadPolicy};
use super::value::Value;
use std::path::Path;

/// An interface contract (`name/version` plus raw `actions`).
#[derive(Clone, Debug, PartialEq)]
pub struct Interface {
    pub name: Value,
    pub version: Value,
    pub display_name: Value,
    pub actions: Value,
    /// Symbols.
    pub aliases: Vec<String>,
}

/// `Psych.safe_load_file(path, permitted_classes: [Symbol, String, Integer,
/// Array, Hash, TrueClass, FalseClass])` of a register YAML file (String
/// keys, aliases disabled): [`LoadPolicy::MODEL_FILE`].
pub(crate) fn load_yaml_file(path: &Path) -> Result<Value, ModelError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ModelError::new(
            ModelErrorKind::Io,
            format!(
                "No such file or directory @ rb_sysopen - {}: {e}",
                path.display()
            ),
        )
    })?;
    safe_load(&text, LoadPolicy::MODEL_FILE)
}

/// The register's effective load path (`Register#load_interface`,
/// `#load_implementation_index`, `#load_implementation_version`):
/// `YAML.safe_load(text, permitted_classes: [Symbol], aliases: true)`
/// ([`LoadPolicy::REGISTER`]) then `Utils.symbolize_keys`; `None` when the
/// document is empty or null (the register returns nil).
pub fn register_hash(yaml: &str) -> Result<Option<Value>, ModelError> {
    let data = safe_load(yaml, LoadPolicy::REGISTER)?;
    if !data.truthy() {
        return Ok(None);
    }
    Ok(Some(data.symbolize_keys()))
}

/// `Interface.symbolize_keys`: `transform_keys(&:to_sym)` recursively
/// (an Integer key raises NoMethodError).
fn symbolize(v: &Value) -> Result<Value, ModelError> {
    match v {
        Value::Hash(pairs) => {
            let mut out = Value::Hash(Vec::new());
            for (k, val) in pairs {
                let key = Value::Symbol(k.to_sym()?);
                let val = match val {
                    Value::Hash(_) => symbolize(val)?,
                    Value::Array(items) => Value::Array(
                        items
                            .iter()
                            .map(|x| match x {
                                Value::Hash(_) => symbolize(x),
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

fn sym_get(data: &Value, key: &str) -> Result<Value, ModelError> {
    match data {
        Value::Hash(_) => Ok(data.get_sym(key).cloned().unwrap_or(Value::Nil)),
        Value::Nil => Err(ModelError::no_method("[]", data)),
        other => Err(ModelError::no_method("[]", other)),
    }
}

impl Interface {
    /// `Interface.new(name:, version:, actions:, display_name: nil, aliases: [])`.
    pub fn new(
        name: Value,
        version: Value,
        actions: Value,
        display_name: Value,
        aliases: Vec<String>,
    ) -> Interface {
        let display_name = if display_name.truthy() {
            display_name
        } else {
            Value::String(ruby::capitalize(&name.to_ruby_s()))
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
    pub fn from_hash(data: &Value) -> Result<Interface, ModelError> {
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
                .map(Value::to_sym)
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
    pub fn action(&self, action_name: &Value) -> Result<Option<&Value>, ModelError> {
        match &self.actions {
            Value::Array(items) => Ok(items
                .iter()
                .find(|a| a.get_sym("name").unwrap_or(&Value::Nil) == action_name)),
            other => Err(ModelError::no_method("find", other)),
        }
    }

    /// `#action?(action_name)`
    pub fn has_action(&self, action_name: &Value) -> Result<bool, ModelError> {
        Ok(self.action(action_name)?.is_some())
    }

    /// `#to_s`
    pub fn to_ruby_s(&self) -> String {
        format!("{}/{}", self.name.to_ruby_s(), self.version.to_ruby_s())
    }

    /// `#inspect` (`actions.length` raises when actions is nil).
    pub fn inspect(&self) -> Result<String, ModelError> {
        let n = match &self.actions {
            Value::Array(a) => a.len(),
            Value::Hash(h) => h.len(),
            Value::String(s) => s.chars().count(),
            other => return Err(ModelError::no_method("length", other)),
        };
        Ok(format!(
            "#<Ukiryu::Models::Interface {} actions={n}>",
            self.to_ruby_s()
        ))
    }
}
