//! Structured Execution Request loading.
//!
//! Ports `RunFileCommand#load_execution_request`, `#validate_request!` and
//! `BaseCommand#stringify_keys` from the Ruby source
//! (`lib/ukiryu/cli_commands/run_file_command.rb` and `base_command.rb` at
//! `ab6df353c6dc2d7faf7817a86aafcafa82579896`).
//!
//! The Ruby method reads the file with `File.read` and parses it with
//! `YAML.safe_load(text, permitted_classes: [Symbol])`. The port parses with
//! the crate's shared `safe_load` caller policy ([`crate::schema::safe_load`]
//! with [`Permitted::SYMBOL`]): Yeptris builds the document, the first
//! document of a stream is used, an empty or comment-only stream is `nil`,
//! and plain scalars resolve with Psych's YAML 1.1 rules (symbols,
//! `yes`/`no`/`on`/`off`, sexagesimal and legacy integers, ...). Date, Time
//! and other classes raise `Psych::DisallowedClass` and aliases raise
//! `Psych::AliasesNotEnabled`, as in Ruby. Loaded values are the crate's
//! unified Ruby value ([`Value`]), so Symbols, Strings, Integers and Floats
//! keep their Ruby identities.
//!
//! Inputs that the shared policy does not reproduce (some YAML tags, for
//! example) fail with [`RequestError::Unsupported`] instead of a guessed
//! value; they carry no Ruby class.
//!
//! This module only loads and validates a request. It does not resolve tools,
//! build commands, execute processes or implement the `run-file` CLI verb.
//! [`Arguments::stringify_keys`] returns the Symbol-keyed Hash that
//! `Tool#execute` hands to the command builder
//! ([`crate::command_builder::Params`] is implemented for it).
use std::fmt;
use std::io::Read;
use std::path::Path;

use serde::ser::{Serialize, Serializer};

use crate::models::ModelError;
use crate::schema::{safe_load, Data, Permitted, SchemaError};
pub use crate::types::Value;

const TOOL: &str = "tool";
const COMMAND: &str = "command";
const ARGUMENTS: &str = "arguments";

/// A validated Structured Execution Request.
///
/// `tool` and `command` hold whatever truthy value the document supplied,
/// because the Ruby validation only rejects missing, `nil` and `false`
/// values. Use [`ExecutionRequest::tool_name`] and
/// [`ExecutionRequest::command_name`] for the usual String form.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionRequest {
    tool: Value,
    command: Value,
    arguments: Arguments,
    /// The loaded Hash's key order, so [`ExecutionRequest::to_value`]
    /// returns the Hash Ruby returns.
    layout: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq)]
enum Field {
    Tool,
    Command,
    Arguments,
    Other(Value, Value),
}

/// The `arguments` Hash, in document order, with its original keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arguments(Vec<(Value, Value)>);

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
    /// The YAML parser rejected the document (`Thor::Error` wrapping
    /// `Psych::SyntaxError`). `detail` is the parser diagnostic, which is
    /// Yeptris text, not libyaml text.
    InvalidYaml { detail: String },
    /// The file could not be read, or loading raised another
    /// `StandardError` such as `Psych::DisallowedClass` (`Thor::Error`).
    Load { detail: String },
    /// The document loaded but is not a valid request (`RuntimeError`).
    Invalid(ValidationError),
    /// The document needs YAML behaviour the shared loader does not
    /// reproduce. Ruby has no matching error; this is not a parity result.
    Unsupported { feature: String, detail: String },
}

impl RequestError {
    /// The Ruby exception class this error corresponds to, `None` for
    /// [`RequestError::Unsupported`].
    pub fn source_class(&self) -> Option<&'static str> {
        match self {
            Self::Invalid(_) => Some("RuntimeError"),
            Self::Unsupported { .. } => None,
            _ => Some("Thor::Error"),
        }
    }

    fn from_schema(error: SchemaError) -> Self {
        match error {
            SchemaError::YamlSyntax { message } => Self::InvalidYaml { detail: message },
            SchemaError::Unsupported { feature, detail } => Self::Unsupported { feature, detail },
            other => Self::Load {
                detail: other.message(),
            },
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
            Self::Unsupported { feature, detail } => write!(f, "unsupported ({feature}): {detail}"),
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

/// Load and validate a request file (`load_execution_request`).
pub fn load_execution_request(path: impl AsRef<Path>) -> Result<ExecutionRequest, RequestError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(RequestError::NotFound {
            path: path.display().to_string(),
        });
    }
    let bytes = read_file(path)?;
    ExecutionRequest::from_yaml_slice(&bytes)
}

/// `File.read(path)` with Ruby's `Errno` message shape: the error text, the
/// failing call (`rb_sysopen` when opening, `io_fread` when reading) and the
/// path. A directory is reported as `Is a directory` on every platform, as
/// Ruby does (Windows refuses to open it, POSIX systems refuse to read it).
fn read_file(path: &Path) -> Result<Vec<u8>, RequestError> {
    let shown = path.display();
    let errno = |error: &std::io::Error| {
        if path.is_dir() {
            "Is a directory".to_owned()
        } else {
            crate::definition::sources::file::strerror(error)
        }
    };
    let mut file = std::fs::File::open(path).map_err(|error| RequestError::Load {
        detail: format!("{} @ rb_sysopen - {shown}", errno(&error)),
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| RequestError::Load {
            detail: format!("{} @ io_fread - {shown}", errno(&error)),
        })?;
    Ok(bytes)
}

/// The Ruby object `YAML.safe_load` built, as the unified value. `Date` and
/// `Time` cannot occur because the request loader does not permit them.
fn ruby_value(data: Data) -> Result<Value, RequestError> {
    Ok(match data {
        Data::Null => Value::Nil,
        Data::Bool(flag) => Value::Bool(flag),
        Data::Integer(number) => Value::Integer(number),
        Data::Float(number) => Value::Float(number),
        Data::String(text) => Value::String(text),
        Data::Symbol(name) => Value::Symbol(name),
        Data::Date { .. } | Data::Time(_) => {
            return Err(RequestError::Unsupported {
                feature: "YAML timestamp".to_owned(),
                detail: "Date and Time are not permitted request classes".to_owned(),
            })
        }
        Data::Array(items) => Value::Array(
            items
                .into_iter()
                .map(ruby_value)
                .collect::<Result<_, _>>()?,
        ),
        Data::Hash(pairs) => Value::Hash(
            pairs
                .into_iter()
                .map(|(key, item)| Ok((ruby_value(key)?, ruby_value(item)?)))
                .collect::<Result<_, RequestError>>()?,
        ),
    })
}

/// The diagnostic for a request file that is not UTF-8. Ruby hands the
/// bytes to libyaml, which raises `Psych::SyntaxError`; the Yeptris
/// diagnostic for the same bytes is used when it has one.
fn invalid_encoding(bytes: &[u8], error: std::str::Utf8Error) -> RequestError {
    let detail = match yeptris::Document::parse(bytes) {
        Err(parse) => parse
            .message()
            .map(str::to_owned)
            .unwrap_or_else(|| parse.to_string()),
        Ok(_) => error.to_string(),
    };
    RequestError::InvalidYaml { detail }
}

impl ExecutionRequest {
    /// Parse and validate request file contents.
    pub fn from_yaml_slice(yaml: &[u8]) -> Result<Self, RequestError> {
        match std::str::from_utf8(yaml) {
            Ok(text) => Self::from_yaml(text),
            Err(error) => Err(invalid_encoding(yaml, error)),
        }
    }

    /// Parse and validate a request document held as text.
    pub fn from_yaml(yaml: &str) -> Result<Self, RequestError> {
        let data = safe_load(yaml, Permitted::SYMBOL).map_err(RequestError::from_schema)?;
        Ok(Self::from_value(ruby_value(data)?)?)
    }

    /// Validate a loaded object (`validate_request!`). Checks run in the
    /// Ruby order, so the first failing requirement is the one reported.
    /// Fields are looked up with String keys (`request['tool']`), so a
    /// Symbol key `:tool` does not count.
    pub fn from_value(value: Value) -> Result<Self, ValidationError> {
        let Value::Hash(pairs) = value else {
            return Err(ValidationError::NotMapping);
        };
        let mut tool = None;
        let mut command = None;
        let mut arguments = None;
        let mut layout = Vec::with_capacity(pairs.len());
        for (key, item) in pairs {
            match key.as_str() {
                Some(TOOL) => {
                    tool = Some(item);
                    layout.push(Field::Tool);
                }
                Some(COMMAND) => {
                    command = Some(item);
                    layout.push(Field::Command);
                }
                Some(ARGUMENTS) => {
                    arguments = Some(item);
                    layout.push(Field::Arguments);
                }
                _ => layout.push(Field::Other(key, item)),
            }
        }
        let tool = tool
            .filter(Value::truthy)
            .ok_or(ValidationError::MissingTool)?;
        let command = command
            .filter(Value::truthy)
            .ok_or(ValidationError::MissingCommand)?;
        let arguments = arguments
            .filter(Value::truthy)
            .ok_or(ValidationError::MissingArguments)?;
        let Value::Hash(arguments) = arguments else {
            return Err(ValidationError::ArgumentsNotMapping);
        };
        Ok(Self {
            tool,
            command,
            arguments: Arguments(arguments),
            layout,
        })
    }

    /// The `tool` value as supplied.
    pub fn tool(&self) -> &Value {
        &self.tool
    }

    /// The `tool` value when it is a String.
    pub fn tool_name(&self) -> Option<&str> {
        self.tool.as_str()
    }

    /// The `command` value as supplied.
    pub fn command(&self) -> &Value {
        &self.command
    }

    /// The `command` value when it is a String.
    pub fn command_name(&self) -> Option<&str> {
        self.command.as_str()
    }

    /// The validated `arguments` Hash.
    pub fn arguments(&self) -> &Arguments {
        &self.arguments
    }

    /// Top-level entries other than `tool`, `command` and `arguments`, in
    /// document order. Ruby keeps them in the loaded request.
    pub fn other_fields(&self) -> Vec<(&Value, &Value)> {
        self.layout
            .iter()
            .filter_map(|field| match field {
                Field::Other(key, item) => Some((key, item)),
                _ => None,
            })
            .collect()
    }

    /// The request Hash `load_execution_request` returns, keys in document
    /// order.
    pub fn to_value(&self) -> Value {
        Value::Hash(
            self.layout
                .iter()
                .map(|field| match field {
                    Field::Tool => (Value::str(TOOL), self.tool.clone()),
                    Field::Command => (Value::str(COMMAND), self.command.clone()),
                    Field::Arguments => (Value::str(ARGUMENTS), self.arguments.to_value()),
                    Field::Other(key, item) => (key.clone(), item.clone()),
                })
                .collect(),
        )
    }
}

impl Arguments {
    /// Entries in document order with their original keys.
    pub fn entries(&self) -> &[(Value, Value)] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `arguments['name']` (a String key; Symbol keys do not match).
    pub fn get(&self, name: &str) -> Option<&Value> {
        let key = Value::str(name);
        self.0
            .iter()
            .find(|(k, _)| crate::types::value::eql(k, &key))
            .map(|(_, item)| item)
    }

    /// The `arguments` Hash.
    pub fn to_value(&self) -> Value {
        Value::Hash(self.0.clone())
    }

    /// `BaseCommand#stringify_keys`: despite its name the Ruby method
    /// converts every top-level key to a Symbol (`transform_keys(&:to_sym)`)
    /// and leaves nested values untouched. Keys that become the same Symbol
    /// keep the first key's position and the last key's value. The first key
    /// without `to_sym` raises `NoMethodError`, as in Ruby.
    pub fn stringify_keys(&self) -> Result<Value, ModelError> {
        let mut result = Value::Hash(Vec::with_capacity(self.0.len()));
        for (key, item) in &self.0 {
            result.insert(Value::Symbol(key.to_sym()?), item.clone());
        }
        Ok(result)
    }
}

impl Serialize for ExecutionRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}
