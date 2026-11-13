//! Source: lib/ukiryu/models/exit_codes.rb (`Ukiryu::Models::ExitCodes`).
use super::attr::{Attr, Model};
use super::error::ModelError;
use super::value::Value;
use serde::{Deserialize, Serialize};

fn default_hash() -> Attr<Value> {
    Attr::Set(Value::Hash(Vec::new()))
}

/// Exit code meanings (`standard` and tool-specific `custom`), keyed as in YAML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitCodes {
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub standard: Attr<Value>,
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub custom: Attr<Value>,
}

impl Default for ExitCodes {
    fn default() -> Self {
        ExitCodes {
            standard: Attr::Default(Value::Hash(Vec::new())),
            custom: Attr::Default(Value::Hash(Vec::new())),
        }
    }
}

fn to_h(v: &Attr<Value>) -> Value {
    match v.get() {
        Some(h @ Value::Hash(_)) => h.clone(),
        _ => Value::Hash(Vec::new()),
    }
}

impl ExitCodes {
    /// `#meaning(code)`: `custom` then `standard`, looked up by `code.to_s`.
    pub fn meaning(&self, code: &Value) -> Option<&Value> {
        let key = Value::String(code.to_ruby_s());
        self.custom
            .get()
            .and_then(|h| h.hash_get(&key))
            .filter(|v| v.truthy())
            .or_else(|| self.standard.get().and_then(|h| h.hash_get(&key)))
    }

    /// `#defined?(code)`
    pub fn is_defined(&self, code: &Value) -> bool {
        self.meaning(code).is_some_and(|v| !v.is_nil())
    }

    /// `#success?(code)`: `code.zero? || meaning(code) == 'success'`.
    pub fn is_success(&self, code: &Value) -> Result<bool, ModelError> {
        let zero = match code {
            Value::Integer(i) => i.sign() == num_bigint::Sign::NoSign,
            Value::Float(f) => *f == 0.0,
            other => return Err(ModelError::no_method("zero?", other)),
        };
        Ok(zero || self.meaning(code) == Some(&Value::str("success")))
    }

    /// `#all_codes`: `standard.to_h.merge(custom.to_h)`.
    pub fn all_codes(&self) -> Value {
        let mut out = to_h(&self.standard);
        if let Value::Hash(pairs) = to_h(&self.custom) {
            for (k, v) in pairs {
                out.insert(k, v);
            }
        }
        out
    }

    /// `#standard_codes`
    pub fn standard_codes(&self) -> Value {
        to_h(&self.standard)
    }

    /// `#custom_codes`
    pub fn custom_codes(&self) -> Value {
        to_h(&self.custom)
    }
}

impl Model for ExitCodes {}
