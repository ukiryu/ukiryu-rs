//! Schema validation, ported from ukiryu/ukiryu at ab6df353:
//! `Ukiryu::SchemaValidator` and the JSON Schema stage of
//! `Ukiryu::Definition::DefinitionValidator`.
//!
//! The contract is what the original code returns with json-schema 6.2.1
//! (captured, not inferred): the same accepted documents, the same error
//! strings in the same order, and the same failures (reported with the Ruby
//! class name through [`SchemaError::ruby_class`]).
//!
//! What that implies, and what this module deliberately does not do:
//!
//! - json-schema 6.2.1 implements drafts 1 to 6 only. A schema declaring
//!   draft-07, which includes all four bundled schemas, fails with
//!   `JSON::Schema::SchemaError: Schema not found: ...`. This module
//!   reproduces that; it does not validate draft-07.
//! - Only the draft-04/draft-06 keywords listed in the engine are
//!   evaluated. Everything else the gem would act on (regular expressions,
//!   `date-time`/`ipv4`/`ipv6` formats, schema `id`s, external `$ref`s, the
//!   draft-01..03 dialects, string schemas that the gem reads from a file or
//!   URL) is reported as [`SchemaError::Unsupported`] instead of guessed.
//! - No network access and no remote schema download: the original
//!   `DefinitionValidator` falls back to downloading a schema when none is
//!   given; that path is not ported. Its structural checks are not schema
//!   validation and are not provided here.

mod data;
mod digest;
mod engine;
mod inspect_table;
mod json;
mod psych;
mod uri;

pub mod bundled;

pub use data::Data;
pub use psych::{safe_load, Permitted};

use std::fmt;
use std::path::{Path, PathBuf};

/// Environment variable naming the default schema file.
pub const SCHEMA_PATH_ENV: &str = "UKIRYU_SCHEMA_PATH";

/// The single error `validate_profile` returns when no schema loads.
pub const FAILED_TO_LOAD_SCHEMA: &str = "Failed to load schema";

/// A failure the original code raises (rather than returns), or an input
/// this port refuses to emulate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// `Psych::SyntaxError`. The message is the YAML engine's, not Psych's.
    YamlSyntax { message: String },
    /// `Psych::DisallowedClass`.
    DisallowedClass { class: String },
    /// `Psych::AliasesNotEnabled`.
    AliasesNotEnabled,
    /// `JSON::Schema::SchemaError`.
    Schema { message: String },
    /// `JSON::Schema::SchemaParseError`.
    SchemaParse { message: String },
    /// `Errno::*` while reading a schema file.
    Io { errno: String, message: String },
    /// Behaviour of the original that this port does not reproduce.
    Unsupported { feature: String, detail: String },
}

impl SchemaError {
    pub(crate) fn unsupported(feature: &str, detail: impl Into<String>) -> SchemaError {
        SchemaError::Unsupported {
            feature: feature.to_owned(),
            detail: detail.into(),
        }
    }

    /// The Ruby exception class the original raises, when there is one.
    pub fn ruby_class(&self) -> Option<String> {
        Some(match self {
            SchemaError::YamlSyntax { .. } => "Psych::SyntaxError".to_owned(),
            SchemaError::DisallowedClass { .. } => "Psych::DisallowedClass".to_owned(),
            SchemaError::AliasesNotEnabled => "Psych::AliasesNotEnabled".to_owned(),
            SchemaError::Schema { .. } => "JSON::Schema::SchemaError".to_owned(),
            SchemaError::SchemaParse { .. } => "JSON::Schema::SchemaParseError".to_owned(),
            SchemaError::Io { errno, .. } => format!("Errno::{errno}"),
            SchemaError::Unsupported { .. } => return None,
        })
    }

    /// The exception message (Ruby's text where this port reproduces it).
    pub fn message(&self) -> String {
        match self {
            SchemaError::YamlSyntax { message }
            | SchemaError::Schema { message }
            | SchemaError::SchemaParse { message }
            | SchemaError::Io { message, .. } => message.clone(),
            SchemaError::DisallowedClass { class } => {
                format!("Tried to load unspecified class: {class}")
            }
            SchemaError::AliasesNotEnabled => "Alias parsing was not enabled. To enable it, pass `aliases: true` to `Psych::load` or `Psych::safe_load`.".to_owned(),
            SchemaError::Unsupported { feature, detail } => {
                format!("unsupported ({feature}): {detail}")
            }
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ruby_class() {
            Some(class) => write!(f, "{class}: {}", self.message()),
            None => f.write_str(&self.message()),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Options of `SchemaValidator.validate_profile`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidateOptions {
    /// `:schema_path`; when `None`, [`SCHEMA_PATH_ENV`] is used.
    pub schema_path: Option<PathBuf>,
    /// `:strict`: every declared property is required and undeclared
    /// properties are reported (json-schema's `strict` option).
    pub strict: bool,
}

/// `SchemaValidator.default_schema_path`: the raw value of
/// [`SCHEMA_PATH_ENV`].
pub fn default_schema_path() -> Option<PathBuf> {
    std::env::var_os(SCHEMA_PATH_ENV).map(PathBuf::from)
}

fn read_text(path: &Path) -> Result<String, SchemaError> {
    if path.is_dir() {
        return Err(SchemaError::Io {
            errno: "EISDIR".to_owned(),
            message: format!("Is a directory @ io_fread - {}", path.display()),
        });
    }
    let bytes = std::fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => SchemaError::Io {
            errno: "EACCES".to_owned(),
            message: format!("Permission denied @ rb_sysopen - {}", path.display()),
        },
        _ => SchemaError::unsupported("schema file read", format!("{}: {e}", path.display())),
    })?;
    String::from_utf8(bytes).map_err(|_| {
        SchemaError::unsupported(
            "schema file encoding",
            format!("{} is not UTF-8", path.display()),
        )
    })
}

/// `SchemaValidator.load_schema(path)`: `Ok(None)` when no path is given
/// (and [`SCHEMA_PATH_ENV`] is unset) or the file does not exist; otherwise
/// the `YAML.safe_load` result, which may itself be `nil`/`false`.
pub fn load_schema(path: Option<&Path>) -> Result<Option<Data>, SchemaError> {
    let env = default_schema_path();
    let Some(path) = path.or(env.as_deref()) else {
        return Ok(None);
    };
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(None);
    }
    let text = read_text(path)?;
    safe_load(&text, Permitted::NONE).map(Some)
}

/// `SchemaValidator.validate_profile(profile, schema_path:, strict:)`.
///
/// `Ok` holds the validation messages (empty when valid, or
/// `["Failed to load schema"]`); `Err` is what the original raises.
pub fn validate_profile(
    profile: &Data,
    options: &ValidateOptions,
) -> Result<Vec<String>, SchemaError> {
    let schema = match load_schema(options.schema_path.as_deref())? {
        Some(s) if s.truthy() => s,
        _ => return Ok(vec![FAILED_TO_LOAD_SCHEMA.to_owned()]),
    };
    Ok(run(&schema, profile, options.strict)?
        .iter()
        .map(|e| e.to_string_at(0))
        .collect())
}

/// `JSON::Validator.fully_validate(schema, data, strict:)` with the
/// key stringification `SchemaValidator` applies first.
pub fn fully_validate(
    schema: &Data,
    data: &Data,
    strict: bool,
) -> Result<Vec<String>, SchemaError> {
    Ok(run(schema, data, strict)?
        .iter()
        .map(|e| e.to_string_at(0))
        .collect())
}

/// `DefinitionValidator.load_schema(path)`: `.json` through `JSON.parse`,
/// `.yaml`/`.yml` through `YAML.safe_load(permitted_classes: [Symbol])`;
/// `Ok(None)` for a missing file, an unknown extension, a JSON parse error
/// or a YAML syntax error (the original rescues those).
pub fn load_definition_schema(path: &Path) -> Result<Option<Data>, SchemaError> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(None);
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => {
            let text = read_text(path)?;
            Ok(json::parse(&text))
        }
        Some("yaml" | "yml") => {
            let text = read_text(path)?;
            match safe_load(&text, Permitted::SYMBOL) {
                Err(SchemaError::YamlSyntax { .. }) => Ok(None),
                other => other.map(Some),
            }
        }
        _ => Ok(None),
    }
}

/// The JSON Schema stage of `DefinitionValidator.validate`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefinitionSchemaReport {
    /// `"#{message} in schema #{uri} (at #{fragment})"` per error.
    pub errors: Vec<String>,
    /// Always empty with json-schema 6.2.1 (no error has type `unknown`).
    pub warnings: Vec<String>,
}

/// `DefinitionValidator.validate_against_schema(definition, schema)`.
pub fn definition_schema_report(
    definition: &Data,
    schema: &Data,
) -> Result<DefinitionSchemaReport, SchemaError> {
    Ok(DefinitionSchemaReport {
        errors: run(schema, definition, false)?
            .iter()
            .map(|e| e.definition_line())
            .collect(),
        warnings: Vec::new(),
    })
}

/// Ruby `to_s` for Hash keys, then the key/Symbol stringification of
/// `stringify_keys` and `JSON::Schema.stringify`.
fn stringify(value: &Data) -> Result<Data, SchemaError> {
    Ok(match value {
        Data::Symbol(s) => Data::String(s.clone()),
        Data::Array(items) => Data::Array(items.iter().map(stringify).collect::<Result<_, _>>()?),
        Data::Hash(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                let key = match k {
                    Data::String(s) => s.clone(),
                    other => other.ruby_to_s()?,
                };
                Data::hash_insert(&mut out, Data::String(key), stringify(v)?);
            }
            Data::Hash(out)
        }
        other => other.clone(),
    })
}

const NETWORK_SCHEMES: [&str; 13] = [
    "http", "https", "ftp", "tftp", "sftp", "ssh", "svn+ssh", "telnet", "nntp", "gopher", "wais",
    "ldap", "prospero",
];

/// `JSON::Validator#initialize_data` for a top-level String: parsed as JSON
/// when it is JSON; otherwise the gem tries to read it as a file path or
/// URL, which this port refuses whenever that read could succeed.
fn initialize_data(data: &Data) -> Result<Data, SchemaError> {
    let Data::String(s) = data else {
        return stringify(data);
    };
    if let Some(parsed) = json::parse(s) {
        return stringify(&parsed);
    }
    if s.contains(['\n', '\r']) {
        return Err(SchemaError::unsupported(
            "string document",
            "a multi-line top-level string is resolved as a URI by the gem; not reproduced",
        ));
    }
    if !uri::addressable_parses(s) {
        return Ok(data.clone());
    }
    let parts = uri::split(s);
    if let Some(scheme) = parts.scheme {
        if NETWORK_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
            return Err(SchemaError::unsupported(
                "string document",
                format!("the gem would fetch {s:?} over the network; not reproduced"),
            ));
        }
    }
    let path = uri::unescape(parts.path);
    if !path.is_empty() && Path::new(&path).is_file() {
        return Err(SchemaError::unsupported(
            "string document",
            format!("the gem would read and parse the file {path:?}; not reproduced"),
        ));
    }
    Ok(data.clone())
}

fn run(schema: &Data, data: &Data, strict: bool) -> Result<Vec<engine::VErr>, SchemaError> {
    let (stringified, uuid) = match schema {
        Data::Hash(_) => (
            stringify(schema)?,
            digest::uuid_v5_nil(&json::dump(schema)?),
        ),
        Data::String(_) => return Err(SchemaError::unsupported(
            "string schema",
            "the gem parses a string schema as JSON or reads it from a file or URL; not reproduced",
        )),
        _ => {
            return Err(SchemaError::SchemaParse {
                message: "Invalid schema - must be either a string or a hash".to_owned(),
            })
        }
    };
    let data = stringify(data)?;
    engine::fully_validate(&stringified, &uuid, || initialize_data(&data), strict)
}
