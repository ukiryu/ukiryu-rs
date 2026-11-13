//! Source: lib/ukiryu/models/exit_codes.rb (`Ukiryu::Models::ExitCodes`).
use super::attr::{Attr, Model};
use super::error::ModelError;
use super::value::RubyValue;
use serde::{Deserialize, Serialize};

fn default_hash() -> Attr<RubyValue> {
    Attr::Set(RubyValue::Hash(Vec::new()))
}

/// Exit code meanings (`standard` and tool-specific `custom`), keyed as in YAML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitCodes {
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub standard: Attr<RubyValue>,
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub custom: Attr<RubyValue>,
}

impl Default for ExitCodes {
    fn default() -> Self {
        ExitCodes {
            standard: Attr::Default(RubyValue::Hash(Vec::new())),
            custom: Attr::Default(RubyValue::Hash(Vec::new())),
        }
    }
}

fn to_h(v: &Attr<RubyValue>) -> RubyValue {
    match v.get() {
        Some(h @ RubyValue::Hash(_)) => h.clone(),
        _ => RubyValue::Hash(Vec::new()),
    }
}

impl ExitCodes {
    /// `#meaning(code)`: `custom` then `standard`, looked up by `code.to_s`.
    pub fn meaning(&self, code: &RubyValue) -> Option<&RubyValue> {
        let key = RubyValue::Str(code.to_ruby_s());
        self.custom
            .get()
            .and_then(|h| h.get(&key))
            .filter(|v| v.truthy())
            .or_else(|| self.standard.get().and_then(|h| h.get(&key)))
    }

    /// `#defined?(code)`
    pub fn is_defined(&self, code: &RubyValue) -> bool {
        self.meaning(code).is_some_and(|v| !v.is_nil())
    }

    /// `#success?(code)`: `code.zero? || meaning(code) == 'success'`.
    pub fn is_success(&self, code: &RubyValue) -> Result<bool, ModelError> {
        let zero = match code {
            RubyValue::Int(i) => *i == 0,
            RubyValue::Float(f) => *f == 0.0,
            other => return Err(ModelError::no_method("zero?", other)),
        };
        Ok(zero || self.meaning(code) == Some(&RubyValue::str("success")))
    }

    /// `#all_codes`: `standard.to_h.merge(custom.to_h)`.
    pub fn all_codes(&self) -> RubyValue {
        let mut out = to_h(&self.standard);
        if let RubyValue::Hash(pairs) = to_h(&self.custom) {
            for (k, v) in pairs {
                out.insert(k, v);
            }
        }
        out
    }

    /// `#standard_codes`
    pub fn standard_codes(&self) -> RubyValue {
        to_h(&self.standard)
    }

    /// `#custom_codes`
    pub fn custom_codes(&self) -> RubyValue {
        to_h(&self.custom)
    }
}

impl Model for ExitCodes {}
