//! Structured Execution Request loading.
//!
//! Ports `RunFileCommand#load_execution_request`, `#validate_request!` and
//! `BaseCommand#stringify_keys` from the Ruby source
//! (`lib/ukiryu/cli_commands/run_file_command.rb` and `base_command.rb` at
//! `ab6df353c6dc2d7faf7817a86aafcafa82579896`).
//!
//! This module only loads and validates a request. It does not resolve tools,
//! build commands, execute processes or implement the `run-file` CLI verb.
//!
//! Error messages are the Ruby public messages. [`RequestError::source_class`]
//! names the Ruby exception class, because the Ruby command reports file and
//! YAML problems as `Thor::Error` while structure problems escape as
//! `RuntimeError`.
//!
//! Argument values are the shared Yeptris tree ([`ArgumentValue`]). Ruby's
//! YAML 1.1 scalar resolution (symbols, `yes`/`no` booleans, sexagesimal and
//! leading-zero integers, disallowed date/time classes, aliases and Ruby tags)
//! belongs to the shared typed Ruby value model, not to this module; until that
//! model is integrated those inputs are recorded as pending divergences in
//! `fixtures/request/pending.json` rather than approximated here.
use std::fmt;
use std::path::Path;

use serde::de::{Deserialize, Deserializer, Error as _};
use serde::ser::{Serialize, Serializer};

/// A dynamic YAML value as produced by the shared Yeptris parser.
pub type ArgumentValue = yeptris::Value;

const TOOL: &str = "tool";
const COMMAND: &str = "command";
const ARGUMENTS: &str = "arguments";

/// A validated Structured Execution Request.
///
/// `tool` and `command` hold whatever truthy value the document supplied,
/// because the Ruby validation only rejects missing, `null` and `false`
/// values. Use [`ExecutionRequest::tool_name`] and
/// [`ExecutionRequest::command_name`] for the usual string form.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionRequest {
    tool: ArgumentValue,
    command: ArgumentValue,
    arguments: Arguments,
    /// Other top-level entries, in document order. Ruby keeps them in the
    /// loaded request hash, so they are preserved rather than rejected.
    other: Vec<(ArgumentValue, ArgumentValue)>,
}

/// The `arguments` mapping, in document order, with its original key values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arguments(Vec<(ArgumentValue, ArgumentValue)>);

/// A structure error raised by `validate_request!` (Ruby `RuntimeError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    NotMapping,
    MissingTool,
    MissingCommand,
    MissingArguments,
    ArgumentsNotMapping,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotMapping => "Request must be a YAML object (hash)",
            Self::MissingTool => "Request must include 'tool' field",
            Self::MissingCommand => "Request must include 'command' field",
            Self::MissingArguments => "Request must include 'arguments' field",
            Self::ArgumentsNotMapping => "'arguments' must be a YAML object (hash)",
        })
    }
}

impl std::error::Error for ValidationError {}

/// Failure to load a request file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// The path does not exist (`Thor::Error`).
    NotFound { path: String },
    /// The YAML parser rejected the document (`Thor::Error`). `detail` is
    /// the parser diagnostic, which is parser-specific text.
    InvalidYaml { detail: String },
    /// The file could not be read or loaded (`Thor::Error`).
    Load { detail: String },
    /// The document loaded but is not a valid request (`RuntimeError`).
    Invalid(ValidationError),
}

impl RequestError {
    /// The Ruby exception class this error corresponds to.
    pub fn source_class(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "RuntimeError",
            _ => "Thor::Error",
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "Request file not found: {path}"),
            Self::InvalidYaml { detail } => write!(f, "Invalid YAML in request file: {detail}"),
            Self::Load { detail } => write!(f, "Error loading request file: {detail}"),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ValidationError> for RequestError {
    fn from(error: ValidationError) -> Self {
        Self::Invalid(error)
    }
}

/// `BaseCommand#stringify_keys` failure: Ruby calls `to_sym` on every
/// argument key, which raises `NoMethodError` for non-string keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyError {
    receiver: &'static str,
}

impl KeyError {
    /// The Ruby exception class (`NoMethodError`).
    pub fn source_class(&self) -> &'static str {
        "NoMethodError"
    }
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "undefined method 'to_sym' for {}", self.receiver)
    }
}

impl std::error::Error for KeyError {}

/// Ruby truthiness: only `nil` and `false` are falsy.
fn truthy(value: &ArgumentValue) -> bool {
    !matches!(value, ArgumentValue::Null | ArgumentValue::Bool(false))
}

fn is_key(key: &ArgumentValue, name: &str) -> bool {
    matches!(key, ArgumentValue::String(text) if text == name)
}

/// Load and validate a request file (`load_execution_request`).
pub fn load_execution_request(path: impl AsRef<Path>) -> Result<ExecutionRequest, RequestError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(RequestError::NotFound {
            path: path.display().to_string(),
        });
    }
    let bytes = std::fs::read(path).map_err(|error| RequestError::Load {
        detail: error.to_string(),
    })?;
    ExecutionRequest::from_yaml_slice(&bytes)
}

impl ExecutionRequest {
    /// Parse and validate a request document.
    pub fn from_yaml_slice(yaml: &[u8]) -> Result<Self, RequestError> {
        let value: ArgumentValue =
            crate::from_yaml_slice(yaml).map_err(|error| RequestError::InvalidYaml {
                detail: error.to_string(),
            })?;
        Ok(Self::from_value(value)?)
    }

    /// Parse and validate a request document held as text.
    pub fn from_yaml(yaml: &str) -> Result<Self, RequestError> {
        Self::from_yaml_slice(yaml.as_bytes())
    }

    /// Validate a loaded document (`validate_request!`). Checks run in the
    /// Ruby order, so the first failing requirement is the one reported.
    pub fn from_value(value: ArgumentValue) -> Result<Self, ValidationError> {
        let ArgumentValue::Mapping(mapping) = value else {
            return Err(ValidationError::NotMapping);
        };
        let mut tool = None;
        let mut command = None;
        let mut arguments = None;
        let mut other = Vec::new();
        // Yeptris resolves duplicate keys last-value-wins; keep the last
        // occurrence here as well so a repeated key cannot shadow it.
        for (key, item) in mapping.0 {
            if is_key(&key, TOOL) {
                tool = Some(item);
            } else if is_key(&key, COMMAND) {
                command = Some(item);
            } else if is_key(&key, ARGUMENTS) {
                arguments = Some(item);
            } else {
                other.push((key, item));
            }
        }
        let tool = tool.filter(truthy).ok_or(ValidationError::MissingTool)?;
        let command = command
            .filter(truthy)
            .ok_or(ValidationError::MissingCommand)?;
        let arguments = arguments
            .filter(truthy)
            .ok_or(ValidationError::MissingArguments)?;
        let ArgumentValue::Mapping(arguments) = arguments else {
            return Err(ValidationError::ArgumentsNotMapping);
        };
        Ok(Self {
            tool,
            command,
            arguments: Arguments(arguments.0),
            other,
        })
    }

    /// The `tool` value as supplied.
    pub fn tool(&self) -> &ArgumentValue {
        &self.tool
    }

    /// The `tool` value when it is a string.
    pub fn tool_name(&self) -> Option<&str> {
        as_str(&self.tool)
    }

    /// The `command` value as supplied.
    pub fn command(&self) -> &ArgumentValue {
        &self.command
    }

    /// The `command` value when it is a string.
    pub fn command_name(&self) -> Option<&str> {
        as_str(&self.command)
    }

    /// The validated `arguments` mapping.
    pub fn arguments(&self) -> &Arguments {
        &self.arguments
    }

    /// Top-level entries other than `tool`, `command` and `arguments`.
    pub fn other_fields(&self) -> &[(ArgumentValue, ArgumentValue)] {
        &self.other
    }

    /// The request as a YAML mapping: `tool`, `command`, `arguments`, then
    /// the other fields in document order.
    pub fn to_value(&self) -> ArgumentValue {
        let mut entries = vec![
            (ArgumentValue::String(TOOL.into()), self.tool.clone()),
            (ArgumentValue::String(COMMAND.into()), self.command.clone()),
            (
                ArgumentValue::String(ARGUMENTS.into()),
                ArgumentValue::Mapping(yeptris::Mapping(self.arguments.0.clone())),
            ),
        ];
        entries.extend(self.other.iter().cloned());
        ArgumentValue::Mapping(yeptris::Mapping(entries))
    }
}

fn as_str(value: &ArgumentValue) -> Option<&str> {
    match value {
        ArgumentValue::String(text) => Some(text),
        _ => None,
    }
}

impl Arguments {
    /// Entries in document order with their original keys.
    pub fn entries(&self) -> &[(ArgumentValue, ArgumentValue)] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The value for a string key.
    pub fn get(&self, name: &str) -> Option<&ArgumentValue> {
        self.0
            .iter()
            .rev()
            .find(|(key, _)| is_key(key, name))
            .map(|(_, value)| value)
    }

    /// `BaseCommand#stringify_keys`: despite its name the Ruby method
    /// converts every top-level key to a symbol (`transform_keys(&:to_sym)`)
    /// and leaves nested values untouched. Keys that become the same symbol
    /// keep the first key's position and the last key's value, as Ruby's hash
    /// insertion does. The first key without `to_sym` fails as in Ruby.
    pub fn stringify_keys(&self) -> Result<Vec<(String, ArgumentValue)>, KeyError> {
        let mut result: Vec<(String, ArgumentValue)> = Vec::with_capacity(self.0.len());
        for (key, value) in &self.0 {
            let name = match key {
                ArgumentValue::String(text) => text.clone(),
                other => {
                    return Err(KeyError {
                        receiver: receiver(other),
                    })
                }
            };
            match result.iter_mut().find(|(existing, _)| *existing == name) {
                Some(slot) => slot.1 = value.clone(),
                None => result.push((name, value.clone())),
            }
        }
        Ok(result)
    }
}

/// Ruby's `NoMethodError` receiver description for a non-symbolizable key.
fn receiver(value: &ArgumentValue) -> &'static str {
    use yeptris::Number;
    match value {
        ArgumentValue::Null => "nil",
        ArgumentValue::Bool(true) => "true",
        ArgumentValue::Bool(false) => "false",
        ArgumentValue::Number(Number::F64(_)) => "an instance of Float",
        ArgumentValue::Number(_) => "an instance of Integer",
        ArgumentValue::Sequence(_) => "an instance of Array",
        ArgumentValue::Mapping(_) => "an instance of Hash",
        ArgumentValue::String(_) => unreachable!("string keys convert to symbols"),
        // Ruby's class for a tagged node depends on Psych tag resolution,
        // which belongs to the typed Ruby value model; do not guess it.
        ArgumentValue::Tagged(_) => "a tagged YAML node (Ruby class unresolved)",
    }
}

impl<'de> Deserialize<'de> for ExecutionRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = ArgumentValue::deserialize(deserializer)?;
        Self::from_value(value).map_err(D::Error::custom)
    }
}

impl Serialize for ExecutionRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}
