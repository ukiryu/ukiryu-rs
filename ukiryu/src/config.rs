//! Source: Ukiryu::Config::{EnvSchema,TypeConverter,EnvProvider,OverrideResolver}.
//! Maps are explicit contexts: reading one configuration never mutates process ENV.
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

pub type Values = BTreeMap<String, Value>;
pub type Environment = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribute {
    Timeout,
    Debug,
    DryRun,
    Metrics,
    Shell,
    Format,
    Output,
    Register,
    UseColor,
}

impl Attribute {
    pub const ALL: [Self; 9] = [
        Self::Timeout,
        Self::Debug,
        Self::DryRun,
        Self::Metrics,
        Self::Shell,
        Self::Format,
        Self::Output,
        Self::Register,
        Self::UseColor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Debug => "debug",
            Self::DryRun => "dry_run",
            Self::Metrics => "metrics",
            Self::Shell => "shell",
            Self::Format => "format",
            Self::Output => "output",
            Self::Register => "register",
            Self::UseColor => "use_color",
        }
    }

    pub fn env_key(self) -> String {
        format!("UKIRYU_{}", self.name().to_ascii_uppercase())
    }

    pub fn convert(self, input: Option<&str>) -> Result<Value, ConversionError> {
        let Some(input) = input.filter(|value| !value.is_empty()) else {
            return Ok(Value::Null);
        };
        match self {
            Self::Timeout => integer(input),
            Self::Debug | Self::DryRun | Self::Metrics | Self::UseColor => {
                match input.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => Ok(Value::Bool(true)),
                    "false" | "0" | "no" => Ok(Value::Bool(false)),
                    _ => Err(ConversionError(format!(
                        "Invalid boolean value: {input}. Valid values: true, 1, yes, false, 0, no"
                    ))),
                }
            }
            // Ruby symbols have textual wire values; adapters expose their native types.
            Self::Shell | Self::Format | Self::Output | Self::Register => {
                Ok(Value::String(input.to_owned()))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionError(pub String);

impl fmt::Display for ConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConversionError {}

fn integer(input: &str) -> Result<Value, ConversionError> {
    let invalid = || ConversionError(format!("Invalid integer value: {input}"));
    let text = input.trim_matches(|c: char| c.is_ascii_whitespace());
    let (negative, body) = if let Some(body) = text.strip_prefix('-') {
        (true, body)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    let (radix, digits) = match body.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => (16, &body[2..]),
        Some("0o") => (8, &body[2..]),
        Some("0b") => (2, &body[2..]),
        Some("0d") => (10, &body[2..]),
        _ if body.starts_with('0') && body.len() > 1 => (8, body),
        _ => (10, body),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits
            .chars()
            .all(|c| c == '_' || c.is_ascii() && c.is_digit(radix))
    {
        return Err(invalid());
    }
    let unsigned =
        BigInt::parse_bytes(digits.replace('_', "").as_bytes(), radix).ok_or_else(invalid)?;
    let number = if negative { -unsigned } else { unsigned };
    Number::from_str(&number.to_string())
        .map(Value::Number)
        .map_err(|_| invalid())
}

/// Only attributes in the source EnvSchema participate. Presence of NO_COLOR,
/// including an empty value, overrides UKIRYU_USE_COLOR exactly as Ruby does.
pub fn environment_values(environment: &Environment) -> Result<Values, ConversionError> {
    let mut output = Values::new();
    for attribute in Attribute::ALL {
        if let Some(value) = environment.get(&attribute.env_key()) {
            output.insert(attribute.name().to_owned(), attribute.convert(Some(value))?);
        }
    }
    if environment.contains_key("NO_COLOR") {
        output.insert("use_color".to_owned(), Value::Bool(false));
    }
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Cli,
    Env,
    Programmatic,
    Default,
}

#[derive(Debug, Clone, Default)]
pub struct OverrideResolver {
    pub defaults: Values,
    pub programmatic: Values,
    pub env: Values,
    pub cli: Values,
}

impl OverrideResolver {
    fn layers(&self) -> [(Source, &Values); 4] {
        [
            (Source::Cli, &self.cli),
            (Source::Env, &self.env),
            (Source::Programmatic, &self.programmatic),
            (Source::Default, &self.defaults),
        ]
    }

    pub fn resolve(&self, name: &str) -> Option<&Value> {
        self.layers()
            .into_iter()
            .find_map(|(_, layer)| layer.get(name))
    }

    pub fn source_for(&self, name: &str) -> Option<Source> {
        self.layers()
            .into_iter()
            .find_map(|(source, layer)| layer.contains_key(name).then_some(source))
    }

    pub fn resolved(&self) -> Values {
        self.layers()
            .into_iter()
            .rev()
            .flat_map(|(_, layer)| {
                layer
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
            })
            .collect()
    }
}

pub fn defaults() -> Values {
    [
        ("timeout", Value::Null),
        ("debug", Value::Bool(false)),
        ("dry_run", Value::Bool(false)),
        ("metrics", Value::Bool(false)),
        ("shell", Value::Null),
        ("format", Value::String("yaml".to_owned())),
        ("output", Value::Null),
        ("register", Value::Null),
        ("use_color", Value::Null),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}
