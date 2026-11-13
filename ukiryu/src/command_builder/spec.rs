//! The command definition fields the builder reads.
//!
//! These mirror the attributes (and Lutaml defaults) of the Ruby models
//! `CommandDefinition`, `OptionDefinition`, `FlagDefinition`,
//! `ArgumentDefinition` and `EnvVarDefinition` that `CommandBuilder` uses,
//! and nothing else. They are the builder's input boundary: the typed models
//! convert into them, and they deserialize from the same YAML/JSON keys.
//! `None` stands for a Ruby `nil` attribute; an absent key takes the model
//! default.
use crate::models::ruby::str_to_bigint;
use crate::types::{OptionSource, Value};
use num_bigint::BigInt;
use serde::Deserialize;
use std::collections::BTreeMap;

/// `PlatformProfile#env_var_sets`: set name to its env var definitions.
pub type EnvVarSets = BTreeMap<String, Vec<EnvVarSpec>>;

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct CommandSpec {
    pub name: Option<String>,
    pub subcommand: Option<String>,
    pub use_env_vars: Option<Vec<String>>,
    pub options: Option<Vec<OptionSpec>>,
    pub post_options: Option<Vec<OptionSpec>>,
    pub flags: Option<Vec<FlagSpec>>,
    pub arguments: Option<Vec<ArgumentSpec>>,
    pub env_vars: Option<Vec<EnvVarSpec>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct OptionSpec {
    pub name: Option<String>,
    pub cli: Option<String>,
    /// Lutaml default `'string'`.
    pub r#type: Option<String>,
    /// Lutaml default `'auto'`.
    pub assignment_delimiter: Option<String>,
    pub separator: Option<String>,
    pub range: Option<Vec<i64>>,
    pub values: Option<Vec<String>>,
    pub of: Option<String>,
}

impl Default for OptionSpec {
    fn default() -> Self {
        OptionSpec {
            name: None,
            cli: None,
            r#type: Some("string".into()),
            assignment_delimiter: Some("auto".into()),
            separator: None,
            range: None,
            values: None,
            of: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FlagSpec {
    pub name: Option<String>,
    pub cli: Option<String>,
    /// Lutaml default `false`.
    pub default: Option<bool>,
    pub position_constraint: Option<String>,
}

impl Default for FlagSpec {
    fn default() -> Self {
        FlagSpec {
            name: None,
            cli: None,
            default: Some(false),
            position_constraint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ArgumentSpec {
    pub name: Option<String>,
    /// Lutaml default `'string'`.
    pub r#type: Option<String>,
    /// Lutaml default `'99'`.
    pub position: Option<String>,
    /// Lutaml default `false`.
    pub variadic: Option<bool>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub size: Option<Vec<i64>>,
    pub of: Option<String>,
    pub range: Option<Vec<i64>>,
    pub values: Option<Vec<String>>,
}

impl Default for ArgumentSpec {
    fn default() -> Self {
        ArgumentSpec {
            name: None,
            r#type: Some("string".into()),
            position: Some("99".into()),
            variadic: Some(false),
            min: None,
            max: None,
            size: None,
            of: None,
            range: None,
            values: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct EnvVarSpec {
    pub name: Option<String>,
    pub value: Option<String>,
    pub from: Option<String>,
    /// Lutaml `initialize_empty: true`.
    pub platforms: Option<Vec<String>>,
}

impl Default for EnvVarSpec {
    fn default() -> Self {
        EnvVarSpec {
            name: None,
            value: None,
            from: None,
            platforms: Some(Vec::new()),
        }
    }
}

impl ArgumentSpec {
    /// `ArgumentDefinition#last?`.
    pub fn is_last(&self) -> bool {
        self.position.as_deref() == Some("last")
    }

    /// `ArgumentDefinition#numeric_position`: `last` 999, `first` 1, a
    /// string with an all-digit line `to_i`, anything else 99.
    pub fn numeric_position(&self) -> BigInt {
        match self.position.as_deref() {
            Some("last") => BigInt::from(999),
            Some("first") => BigInt::from(1),
            Some(p) if p.split('\n').any(all_digits) => str_to_bigint(p),
            _ => BigInt::from(99),
        }
    }
}

/// A line matching Ruby `/^\d+$/` (`\d` is ASCII only; `$` stops at `\n`).
fn all_digits(line: &str) -> bool {
    !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit())
}

fn int_list(v: &Option<Vec<i64>>) -> Value {
    match v {
        Some(items) => Value::Array(items.iter().map(|&i| Value::int(i)).collect()),
        None => Value::Nil,
    }
}

fn text_list(v: &Option<Vec<String>>) -> Value {
    match v {
        Some(items) => Value::Array(items.iter().map(|s| Value::str(s.as_str())).collect()),
        None => Value::Nil,
    }
}

fn text(v: &Option<String>) -> Value {
    v.as_deref().map_or(Value::Nil, Value::str)
}

/// `OptionDefinition#[]`: the attribute when the model responds to the key,
/// otherwise `nil`. Only the keys `Type.validate` reads matter here.
impl OptionSource for OptionSpec {
    fn option(&self, key: &str) -> Value {
        match key {
            "range" => int_list(&self.range),
            "values" => text_list(&self.values),
            "of" => text(&self.of),
            "separator" => text(&self.separator),
            "cli" => text(&self.cli),
            "name" => text(&self.name),
            "type" => text(&self.r#type),
            _ => Value::Nil,
        }
    }
}

/// `ArgumentDefinition#[]`.
impl OptionSource for ArgumentSpec {
    fn option(&self, key: &str) -> Value {
        match key {
            "min" => self.min.map_or(Value::Nil, Value::int),
            "max" => self.max.map_or(Value::Nil, Value::int),
            "size" => int_list(&self.size),
            "range" => int_list(&self.range),
            "values" => text_list(&self.values),
            "of" => text(&self.of),
            "name" => text(&self.name),
            "type" => text(&self.r#type),
            "position" => text(&self.position),
            "variadic" => self.variadic.map_or(Value::Nil, Value::Bool),
            _ => Value::Nil,
        }
    }
}
