//! Register-backed tool loading (port of `Ukiryu::Tool::Loader`,
//! `lib/ukiryu/tool/loader.rb` at ukiryu ab6df353).
//!
//! [`load_definition`] is `Loader.load_with_implementation_index` up to (not
//! including) `Tool.new`: it loads the implementation index and the interface
//! from a [`Register`], picks the implementation and version file, loads the
//! implementation version and converts it into a typed [`ToolDefinition`].
//! Building the runtime tool from that definition belongs to the tool runtime
//! ([`crate::tool`]); [`RegisterSource`] is the register-backed
//! [`DefinitionSource`] that `tool::get` and `tool::find` take.
//!
//! Every stage reuses the module that owns it; nothing is reimplemented here:
//!
//! * register documents load through the models' register entry points
//!   ([`ImplementationIndex::from_register_yaml`],
//!   [`Interface::from_register_yaml`],
//!   [`ImplementationVersion::from_register_yaml`]), which are
//!   `YAML.safe_load` + `Utils.symbolize_keys` + `from_hash`;
//! * implementation and version-file selection is
//!   [`routing::detect_implementation_and_version`], with this module as its
//!   [`SelectionEnv`]: detection probes go through
//!   [`crate::version_detection::probe_index_detection`] and the caller's
//!   [`CommandRunner`], version schemes and ranges through
//!   [`version_ranges::resolve`] and [`version_ranges::find_matching_version_spec`];
//! * the typed models are built with [`crate::model_loading`]
//!   (`CommandDefinition.new`, `PlatformProfile.new`, `ToolDefinition.new`)
//!   and finished with [`ToolDefinition::resolve_inheritance`].
//!
//! What stays here is the loader's own code: the orchestration, the Hash
//! helpers ([`deep_merge_hashes`], [`convert_actions_to_array`],
//! [`build_interface_commands_hash`], [`flatten_signature`],
//! [`extract_profile_data`]) and the conversion of the selected execution
//! profile into keyword arguments for the model constructors. The helpers
//! work on the unified [`Value`] and keep the source's results, its in-place
//! mutation of the caller's Hashes and its exceptions.
//!
//! The caller passes everything the source reads from the process: the
//! current platform (`Platform.detect`), the shell (`options[:shell] ||
//! Shell.detect`; choosing a default shell is the owner-pending D2 decision,
//! so it is always explicit here) and the runner for detection commands.
//!
//! Errors keep the class the source raises: register and model failures,
//! `RegexpError` from a detection pattern, Versionian errors from scheme and
//! range handling, and `NoMethodError`/`TypeError`/`ArgumentError` raised by
//! the loader code itself ([`LoadError::Source`]). Input shapes the source
//! would hand to Ruby in ways this port does not reproduce are reported as
//! [`LoadError::Unsupported`], never guessed.
use crate::executable::Platform;
use crate::model_loading::{command_definition_new, platform_profile_new, tool_definition_new};
use crate::models::{
    CommandDefinition, ImplementationIndex, ImplementationVersion, Interface, ModelError,
    PlatformProfile, ToolDefinition, Value,
};
use crate::register::{self, Register};
use crate::routing::{
    self, Implementation, LoadedVersion, Probe, ProfileView, RoutingError, SelectionEnv,
    SelectionRequest, SourceFault, VersionEntry,
};
use crate::tool::{DefinitionSource, ToolError, ToolOptions};
use crate::types::ruby::receiver_description;
use crate::version_detection::{
    probe_index_detection, CommandRunner, IndexDetection, IndexProbe, PatternError,
};
use crate::version_ranges::{self, RangeError, Scheme, SpecValue};
use std::cell::RefCell;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Why loading failed, classified by the exception the source raises.
#[derive(Debug)]
pub enum LoadError {
    /// Reading a register file failed (`Errno::*`).
    Register(register::Error),
    /// YAML or model construction failed.
    Model(ModelError),
    /// A detection pattern Ruby rejects (`RegexpError`), or one this port
    /// does not reproduce.
    Pattern(PatternError),
    /// A Versionian error from scheme resolution or range matching.
    Range(RangeError),
    /// An exception the loader or selection code itself raises.
    Source(SourceFault),
    /// A value shape the source would pass on in a way this port does not
    /// reproduce.
    Unsupported(String),
}

impl LoadError {
    /// The Ruby exception class the source raises (`"Unsupported"` for the
    /// non-Ruby gaps).
    pub fn class_name(&self) -> &'static str {
        match self {
            LoadError::Register(e) => e.class_name(),
            LoadError::Model(e) => e.ruby_class(),
            LoadError::Pattern(PatternError::RubyInvalid { .. }) => "RegexpError",
            LoadError::Pattern(PatternError::Unsupported { .. }) => "Unsupported",
            LoadError::Range(e) => e.class.ruby_name(),
            LoadError::Source(f) => f.class,
            LoadError::Unsupported(_) => "Unsupported",
        }
    }

    /// The exception message, in the source's wording where it is ported.
    pub fn message(&self) -> String {
        match self {
            LoadError::Register(e) => e.to_string(),
            LoadError::Model(e) => e.message.clone(),
            LoadError::Pattern(PatternError::RubyInvalid { pattern, reason }) => {
                format!("{reason}: /{pattern}/")
            }
            LoadError::Pattern(e) => e.to_string(),
            LoadError::Range(e) => e.message.clone(),
            LoadError::Source(f) => f.message.clone(),
            LoadError::Unsupported(m) => m.clone(),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class_name(), self.message())
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Register(e) => Some(e),
            LoadError::Model(e) => Some(e),
            LoadError::Pattern(e) => Some(e),
            LoadError::Range(e) => Some(e),
            LoadError::Source(e) => Some(e),
            LoadError::Unsupported(_) => None,
        }
    }
}

impl From<ModelError> for LoadError {
    fn from(e: ModelError) -> Self {
        LoadError::Model(e)
    }
}

impl From<SourceFault> for LoadError {
    fn from(f: SourceFault) -> Self {
        LoadError::Source(f)
    }
}

fn no_method(method: &str, receiver: &Value) -> LoadError {
    LoadError::Source(SourceFault::new(
        "NoMethodError",
        format!(
            "undefined method '{method}' for {}",
            receiver_description(receiver)
        ),
    ))
}

fn type_error(message: impl Into<String>) -> LoadError {
    LoadError::Source(SourceFault::new("TypeError", message))
}

fn unsupported(message: impl Into<String>) -> LoadError {
    LoadError::Unsupported(message.into())
}

/// `TypeError` for `recv[:sym]` on an Array or String receiver.
fn symbol_index_error() -> LoadError {
    type_error("no implicit conversion of Symbol into Integer")
}

/// What the loader needs from the process, passed explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadOptions<'a> {
    /// `Platform.detect` (`linux`, `macos`, `windows`, ...).
    pub current_platform: &'a str,
    /// `options[:platform]`: a target platform, `None` for the current one.
    pub platform: Option<&'a str>,
    /// `options[:shell]`; never defaulted here (D2 is owner-pending).
    pub shell: &'a str,
    /// `options[:executable]`, the detection executable fallback.
    pub executable: Option<&'a str>,
}

// ------------------------------------------------------------ Hash access

/// `hash[:key] || hash['key']` (the second value, possibly `nil`/`false`,
/// when the first is falsy).
fn sym_or_str(hash: &Value, key: &str) -> Value {
    match hash.get_sym(key) {
        Some(v) if v.truthy() => v.clone(),
        _ => hash.get_str(key).cloned().unwrap_or(Value::Nil),
    }
}

/// `hash['key'] || hash[:key]`.
fn str_or_sym(hash: &Value, key: &str) -> Value {
    match hash.get_str(key) {
        Some(v) if v.truthy() => v.clone(),
        _ => hash.get_sym(key).cloned().unwrap_or(Value::Nil),
    }
}

/// `a || b` over already-fetched values.
fn or(a: Value, b: impl FnOnce() -> Value) -> Value {
    if a.truthy() {
        a
    } else {
        b()
    }
}

/// `Hash#delete(key)`.
fn hash_delete(hash: &mut Value, key: &Value) {
    if let Value::Hash(pairs) = hash {
        pairs.retain(|(k, _)| !crate::types::value::eql(k, key));
    }
}

/// `Hash#[]` on the receivers a register document can hold: `nil` for a
/// missing key, the source's exception for a non-Hash receiver.
fn index_key(recv: &Value, key: &str, method: &str) -> Result<Value, LoadError> {
    match recv {
        Value::Hash(_) => Ok(recv.get_sym(key).cloned().unwrap_or(Value::Nil)),
        Value::Nil | Value::Bool(_) => Err(no_method(method, recv)),
        Value::Array(_) | Value::String(_) => Err(symbol_index_error()),
        other => Err(unsupported(format!(
            "indexing an instance of {} is not ported",
            other.class_name()
        ))),
    }
}

/// `recv.to_h`: a Hash is returned as is (shared with the caller), `nil`
/// and an empty Array give a new empty Hash.
enum ToH {
    Same,
    New(Value),
}

fn to_h(recv: &Value) -> Result<ToH, LoadError> {
    match recv {
        Value::Hash(_) => Ok(ToH::Same),
        Value::Nil => Ok(ToH::New(Value::Hash(Vec::new()))),
        Value::Array(items) if items.is_empty() => Ok(ToH::New(Value::Hash(Vec::new()))),
        Value::Array(_) => Err(unsupported("Array#to_h on a non-empty Array is not ported")),
        other => Err(no_method("to_h", other)),
    }
}

/// `recv.empty?`.
fn is_empty(recv: &Value) -> Result<bool, LoadError> {
    match recv {
        Value::Array(a) => Ok(a.is_empty()),
        Value::Hash(h) => Ok(h.is_empty()),
        Value::String(s) | Value::Symbol(s) => Ok(s.is_empty()),
        other => Err(no_method("empty?", other)),
    }
}

// ---------------------------------------------------------- Hash helpers

/// `deep_merge_hashes(base, override)`: `base.merge(override)`, recursing
/// into Hash pairs, with a `nil` override keeping the base value. Keys keep
/// the base's order; new keys are appended. Neither argument is mutated.
pub fn deep_merge_hashes(base: &Value, overrides: &Value) -> Result<Value, LoadError> {
    let Value::Hash(base_pairs) = base else {
        return Err(no_method("merge", base));
    };
    let Value::Hash(override_pairs) = overrides else {
        let what = match overrides {
            Value::Nil => "nil".to_string(),
            Value::Bool(b) => b.to_string(),
            other => other.class_name().to_string(),
        };
        return Err(type_error(format!(
            "no implicit conversion of {what} into Hash"
        )));
    };
    let mut merged = Value::Hash(base_pairs.clone());
    for (key, new_val) in override_pairs {
        let value = match merged.hash_get(key) {
            None => new_val.clone(),
            Some(old_val) => match (old_val, new_val) {
                (Value::Hash(_), Value::Hash(_)) => deep_merge_hashes(old_val, new_val)?,
                (_, Value::Nil) => old_val.clone(),
                _ => new_val.clone(),
            },
        };
        merged.insert(key.clone(), value);
    }
    Ok(merged)
}

/// `flatten_signature(command_def)`: lifts the `signature` entries into the
/// command (`inputs` Hash entries as Symbol keys, a nested `inputs` becoming
/// `arguments`) and deletes both `signature` keys. Mutates `command` in
/// place, as the source does, and returns the result.
pub fn flatten_signature(command: &mut Value) -> Result<Value, LoadError> {
    let signature = match &*command {
        Value::Hash(_) => sym_or_str(command, "signature"),
        Value::Nil | Value::Bool(_) => return Err(no_method("[]", command)),
        Value::Array(_) | Value::String(_) => return Err(symbol_index_error()),
        other => {
            return Err(unsupported(format!(
                "flatten_signature on an instance of {} is not ported",
                other.class_name()
            )))
        }
    };
    if !signature.truthy() {
        return Ok(command.clone());
    }
    let entries: Vec<(Value, Value)> = match &signature {
        Value::Hash(pairs) => pairs.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Array(parts) => (
                    parts.first().cloned().unwrap_or(Value::Nil),
                    parts.get(1).cloned().unwrap_or(Value::Nil),
                ),
                other => (other.clone(), Value::Nil),
            })
            .collect(),
        other => return Err(no_method("each", other)),
    };
    let is_signature =
        |k: &Value| matches!(k, Value::Symbol(s) | Value::String(s) if s == "signature");
    for (key, value) in entries {
        let is_inputs = matches!(&key, Value::Symbol(s) | Value::String(s) if s == "inputs");
        match (&value, is_inputs) {
            (Value::Hash(nested), true) => {
                for (nested_key, nested_value) in nested {
                    let name = nested_key.to_ruby_s();
                    let target = if name == "inputs" {
                        "arguments".to_string()
                    } else {
                        name
                    };
                    if !is_signature(nested_key) {
                        command.insert(Value::Symbol(target), nested_value.clone());
                    }
                }
            }
            _ => {
                if !is_signature(&key) {
                    command.insert(key, value);
                }
            }
        }
    }
    hash_delete(command, &Value::sym("signature"));
    hash_delete(command, &Value::str("signature"));
    Ok(command.clone())
}

/// `convert_actions_to_array(actions_data)`: interface actions as a list of
/// command Hashes. A Hash of actions gets each entry's String `name` filled
/// in from its key (`||=`); a list of actions is flattened with
/// [`flatten_signature`]. Action Hashes are updated in place inside
/// `actions`, as in the source.
pub fn convert_actions_to_array(actions: &mut Value) -> Result<Vec<Value>, LoadError> {
    if actions.is_nil() || is_empty(actions)? {
        return Ok(Vec::new());
    }
    match actions {
        Value::Hash(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for (command_name, command_def) in pairs.iter_mut() {
                let mut fresh = match to_h(command_def)? {
                    ToH::Same => None,
                    ToH::New(h) => Some(h),
                };
                let target = fresh.as_mut().unwrap_or(command_def);
                if !target.get_str("name").is_some_and(Value::truthy) {
                    target.insert(Value::str("name"), Value::str(command_name.to_ruby_s()));
                }
                out.push(target.clone());
            }
            Ok(out)
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for command_def in items.iter_mut() {
                let result = match to_h(command_def)? {
                    ToH::Same => flatten_signature(command_def)?,
                    ToH::New(mut h) => flatten_signature(&mut h)?,
                };
                out.push(result);
            }
            Ok(out)
        }
        other => Err(no_method("map", other)),
    }
}

/// `build_interface_commands_hash(actions)`: the converted actions keyed by
/// `cmd[:name] || cmd['name']` (a later duplicate replaces the earlier value
/// in the earlier position). `nil`/`false` actions give an empty Hash.
pub fn build_interface_commands_hash(actions: &mut Value) -> Result<Value, LoadError> {
    let mut empty = Value::Array(Vec::new());
    let source = if actions.truthy() {
        actions
    } else {
        &mut empty
    };
    let mut hash = Value::Hash(Vec::new());
    for command in convert_actions_to_array(source)? {
        let name = sym_or_str(&command, "name");
        hash.insert(name, command);
    }
    Ok(hash)
}

const PROFILE_KEYS: [&str; 7] = [
    "name",
    "display_name",
    "platforms",
    "shells",
    "option_style",
    "inherits",
    "executable_name",
];

/// `extract_profile_data(profile)`: the `PlatformProfile.new` keywords of an
/// execution profile Hash (`profile[:key] || profile['key']` for each).
pub fn extract_profile_data(profile: &Value) -> Result<Value, LoadError> {
    match profile {
        Value::Hash(_) => Ok(Value::Hash(
            PROFILE_KEYS
                .iter()
                .map(|key| (Value::sym(*key), sym_or_str(profile, key)))
                .collect(),
        )),
        // Symbol#name exists; the next reader does not.
        Value::Symbol(_) => Err(no_method("display_name", profile)),
        other => Err(no_method("name", other)),
    }
}

// ---------------------------------------------------- model construction

/// `convert_hash_to_command_definition(cmd_hash)`.
fn convert_hash_to_command_definition(cmd: &Value) -> Result<CommandDefinition, LoadError> {
    let keyword = |key: &str| (Value::sym(key), str_or_sym(cmd, key));
    let defaulted =
        |key: &str, default: Value| (Value::sym(key), or(str_or_sym(cmd, key), || default));
    let kwargs = Value::Hash(vec![
        keyword("name"),
        keyword("description"),
        keyword("usage"),
        keyword("subcommand"),
        keyword("belongs_to"),
        keyword("cli_flag"),
        defaulted("standalone_executable", Value::Bool(false)),
        defaulted("aliases", Value::Array(Vec::new())),
        defaulted("use_env_vars", Value::Array(Vec::new())),
        defaulted("implements", Value::Array(Vec::new())),
        keyword("options"),
        keyword("flags"),
        keyword("arguments"),
        keyword("post_options"),
        keyword("env_vars"),
        keyword("exit_codes"),
    ]);
    Ok(command_definition_new(&kwargs)?)
}

/// `build_command_definitions(profile_commands, interface_commands_hash)`:
/// the interface's commands when the profile has none, otherwise each
/// profile command deep-merged over the interface command of the same name
/// (`name`, falling back to `subcommand`).
fn build_command_definitions(
    profile_commands: &Value,
    interface_commands: &Value,
) -> Result<Vec<CommandDefinition>, LoadError> {
    if is_empty(profile_commands)? {
        let Value::Hash(pairs) = interface_commands else {
            return Ok(Vec::new());
        };
        return pairs
            .iter()
            .map(|(_, cmd)| convert_hash_to_command_definition(cmd))
            .collect();
    }
    let items = match profile_commands {
        Value::Array(items) => items,
        Value::Hash(_) => return Err(symbol_index_error()),
        other => return Err(no_method("map", other)),
    };
    let mut out = Vec::with_capacity(items.len());
    for cmd in items {
        match cmd {
            Value::Hash(_) => {}
            Value::Nil | Value::Bool(_) => return Err(no_method("[]", cmd)),
            Value::Array(_) | Value::String(_) => return Err(symbol_index_error()),
            other => {
                return Err(unsupported(format!(
                    "a profile command that is an instance of {} is not ported",
                    other.class_name()
                )))
            }
        }
        let name = or(sym_or_str(cmd, "name"), || sym_or_str(cmd, "subcommand"));
        let merged = match interface_commands.hash_get(&name) {
            Some(interface_cmd) if interface_cmd.truthy() => deep_merge_hashes(interface_cmd, cmd)?,
            _ => cmd.clone(),
        };
        out.push(convert_hash_to_command_definition(&merged)?);
    }
    Ok(out)
}

/// `convert_profile_to_platform_profile(profile, actions)`. `actions` is
/// updated in place as [`convert_actions_to_array`] describes.
fn convert_profile_to_platform_profile(
    profile: &Value,
    actions: &mut Value,
) -> Result<PlatformProfile, LoadError> {
    let data = extract_profile_data(profile)?;
    let commands = or(sym_or_str(profile, "commands"), || Value::Array(Vec::new()));
    let interface_commands = build_interface_commands_hash(actions)?;
    let definitions = build_command_definitions(&commands, &interface_commands)?;
    Ok(platform_profile_new(&data, definitions)?)
}

/// `convert_to_tool_definition(tool_name, interface, impl_version,
/// implementation_name, detected_version, platform:, shell:)`, followed by
/// `resolve_inheritance!`. `None` when no execution profile lists both the
/// platform and the shell.
pub fn convert_to_tool_definition(
    tool_name: &str,
    interface: &Interface,
    impl_version: &ImplementationVersion,
    implementation_name: Option<&str>,
    detected_version: Option<&str>,
    platform: &str,
    shell: &str,
) -> Result<Option<ToolDefinition>, LoadError> {
    let Some(profile) = impl_version.compatible_profile(platform, shell)? else {
        return Ok(None);
    };
    let name = routing::build_tool_name(tool_name, implementation_name);
    let version = match detected_version {
        Some(v) => Value::str(v),
        None => impl_version.version.clone(),
    };
    let display_name = if impl_version.display_name.truthy() {
        impl_version.display_name.clone()
    } else {
        Value::String(format!(
            "{} {} {}",
            interface.name.to_ruby_s(),
            implementation_name.unwrap_or(""),
            version.to_ruby_s()
        ))
    };
    let mut actions = interface.actions.clone();
    let platform_profile = convert_profile_to_platform_profile(&profile, &mut actions)?;
    let kwargs = Value::Hash(vec![
        (Value::sym("name"), Value::String(name)),
        (Value::sym("version"), version),
        (Value::sym("display_name"), display_name),
        (
            Value::sym("implements"),
            Value::Array(interface.name.to_array()),
        ),
        (
            Value::sym("aliases"),
            Value::Array(impl_version.aliases.iter().map(Value::str).collect()),
        ),
    ]);
    let mut tool = tool_definition_new(
        &kwargs,
        vec![platform_profile],
        impl_version.version_detection.clone(),
    )?;
    tool.resolve_inheritance();
    Ok(Some(tool))
}

// ------------------------------------------------------- register files

/// `File.join(base, *parts)`: separators between parts collapse, so a part
/// with a leading `/` stays under `base` (unlike `Path::join`).
fn ruby_join(base: &Path, parts: &[&str]) -> PathBuf {
    let mut path = OsString::from(base.as_os_str());
    for part in parts {
        path.push("/");
        path.push(part.trim_start_matches('/'));
    }
    PathBuf::from(path)
}

/// `File.exist?(file)` then `File.read(file)`.
fn read_document(path: &Path) -> Result<Option<String>, LoadError> {
    if !path.exists() {
        return Ok(None);
    }
    fs::read_to_string(path).map(Some).map_err(|error| {
        LoadError::Register(register::Error::Io {
            path: path.to_path_buf(),
            error,
        })
    })
}

/// `Register#load_implementation_index(tool)`: `tools/<tool>/index.yaml`.
pub fn load_implementation_index(
    register: &Register,
    tool: &str,
) -> Result<Option<ImplementationIndex>, LoadError> {
    let path = ruby_join(register.path(), &["tools", tool, "index.yaml"]);
    match read_document(&path)? {
        None => Ok(None),
        Some(text) => Ok(ImplementationIndex::from_register_yaml(&text)?),
    }
}

/// `Register#load_interface(path)`: `interfaces/<path>.yaml`.
pub fn load_interface(
    register: &Register,
    interface_path: &str,
) -> Result<Option<Interface>, LoadError> {
    let file = format!("{interface_path}.yaml");
    let path = ruby_join(register.path(), &["interfaces", &file]);
    match read_document(&path)? {
        None => Ok(None),
        Some(text) => Ok(Interface::from_register_yaml(&text)?),
    }
}

/// `Register#load_implementation_version(tool, implementation, file)`:
/// `tools/<tool>/<implementation>/<file>`.
pub fn load_implementation_version(
    register: &Register,
    tool: &str,
    implementation: &str,
    file: &str,
) -> Result<Option<ImplementationVersion>, LoadError> {
    let path = ruby_join(register.path(), &["tools", tool, implementation, file]);
    match read_document(&path)? {
        None => Ok(None),
        Some(text) => Ok(ImplementationVersion::from_register_yaml(&text)?),
    }
}

// ------------------------------------------------------------ selection

/// A String-valued register entry (`file`, `default`, `name`, ...).
fn string_entry(value: Value, what: &str) -> Result<Option<String>, LoadError> {
    match value {
        v if !v.truthy() => Ok(None),
        Value::String(s) => Ok(Some(s)),
        other => Err(unsupported(format!(
            "{what} that is an instance of {} is not ported",
            other.class_name()
        ))),
    }
}

/// A list of Strings (`Array(value)`), as detection commands use them.
fn string_list(value: &Value, what: &str) -> Result<Vec<String>, LoadError> {
    value
        .to_array()
        .into_iter()
        .map(|item| match item {
            Value::String(s) => Ok(s),
            other => Err(unsupported(format!(
                "{what} element that is an instance of {} is not ported",
                other.class_name()
            ))),
        })
        .collect()
}

/// The selection view of one `implementations:` entry.
fn implementation(raw: &Value) -> Result<Implementation, LoadError> {
    if !matches!(raw, Value::Hash(_)) {
        return Err(unsupported(format!(
            "an implementation that is an instance of {} is not ported",
            raw.class_name()
        )));
    }
    let name = match sym_or_str(raw, "name") {
        v if !v.truthy() => None,
        v => Some(v.to_ruby_s()),
    };
    let default = string_entry(sym_or_str(raw, "default"), "an implementation default")?;
    let versions = match sym_or_str(raw, "versions") {
        v if !v.truthy() => None,
        Value::Array(items) => Some(
            items
                .iter()
                .map(|spec| -> Result<VersionEntry, LoadError> {
                    match spec {
                        Value::Hash(_) => Ok(VersionEntry {
                            file: string_entry(sym_or_str(spec, "file"), "a version file")?,
                            default: sym_or_str(spec, "default").truthy(),
                        }),
                        other => Err(unsupported(format!(
                            "a version spec that is an instance of {} is not ported",
                            other.class_name()
                        ))),
                    }
                })
                .collect::<Result<_, _>>()?,
        ),
        other => {
            return Err(unsupported(format!(
                "implementation versions that are an instance of {} are not ported",
                other.class_name()
            )))
        }
    };
    Ok(Implementation {
        name,
        default,
        versions,
    })
}

/// The `detection:` Hash of an implementation as [`IndexDetection`].
fn index_detection(raw: &Value) -> Result<IndexDetection, LoadError> {
    let detection = sym_or_str(raw, "detection");
    match &detection {
        Value::Hash(_) => {}
        Value::Nil | Value::Bool(_) => return Err(no_method("[]", &detection)),
        Value::Array(_) | Value::String(_) => return Err(symbol_index_error()),
        other => {
            return Err(unsupported(format!(
                "detection that is an instance of {} is not ported",
                other.class_name()
            )))
        }
    }
    let command = match sym_or_str(&detection, "command") {
        v if !v.truthy() => None,
        v => Some(string_list(&v, "a detection command")?),
    };
    let executables = match sym_or_str(&detection, "executables") {
        v if !v.truthy() => None,
        v => Some(string_list(&v, "a detection executable")?),
    };
    Ok(IndexDetection {
        command,
        executables,
        executable: string_entry(
            sym_or_str(&detection, "executable"),
            "a detection executable",
        )?,
        pattern: string_entry(sym_or_str(&detection, "pattern"), "a detection pattern")?,
    })
}

/// A register value as a version-range input.
fn spec_value(value: &Value) -> Result<SpecValue, LoadError> {
    Ok(match value {
        Value::Nil => SpecValue::Nil,
        Value::Bool(b) => SpecValue::Bool(*b),
        Value::Integer(i) => SpecValue::Int(i.clone()),
        Value::Float(f) => SpecValue::Float(*f),
        Value::String(s) => SpecValue::Str(s.clone()),
        Value::Symbol(s) => SpecValue::Sym(s.clone()),
        Value::Array(items) => {
            SpecValue::Array(items.iter().map(spec_value).collect::<Result<_, _>>()?)
        }
        Value::Hash(pairs) => SpecValue::Map(
            pairs
                .iter()
                .map(|(k, v)| -> Result<(SpecValue, SpecValue), LoadError> {
                    Ok((spec_value(k)?, spec_value(v)?))
                })
                .collect::<Result<_, LoadError>>()?,
        ),
        other => {
            return Err(unsupported(format!(
                "a version spec value that is an instance of {} is not ported",
                other.class_name()
            )))
        }
    })
}

/// The selection view of a loaded version's execution profiles
/// (`compatible_profile`'s reads).
fn profile_views(profiles: &Value) -> Result<Vec<ProfileView>, LoadError> {
    let Value::Array(items) = profiles else {
        return Err(no_method("find", profiles));
    };
    items
        .iter()
        .map(|prof| -> Result<ProfileView, LoadError> {
            let platforms = or(index_key(prof, "platforms", "[]")?, || {
                prof.sym_get("platform")
            });
            let shells = or(index_key(prof, "shells", "[]")?, || prof.sym_get("shell"));
            let text = |v: Value| {
                if v.truthy() {
                    Some(v.to_ruby_s())
                } else {
                    None
                }
            };
            let commands = prof.sym_get("commands");
            Ok(ProfileView {
                name: text(sym_or_str(prof, "name")),
                platforms: platforms.to_array().iter().map(Value::to_ruby_s).collect(),
                shells: shells.to_array().iter().map(Value::to_ruby_s).collect(),
                inherits: text(sym_or_str(prof, "inherits")),
                has_commands: !(commands.is_nil() || commands.is_empty_container()),
            })
        })
        .collect()
}

/// The loader's collaborators for [`routing::detect_implementation_and_version`].
struct Selection<'a, 'r> {
    register: &'a Register,
    tool: &'a str,
    raw: &'a [Value],
    implementations: &'a [Implementation],
    executable: Option<&'a str>,
    shell: &'a str,
    runner: &'r mut dyn CommandRunner,
    scheme: Option<Scheme>,
}

impl SelectionEnv for Selection<'_, '_> {
    type Error = LoadError;

    fn probe(&mut self, implementation: usize) -> Result<Probe, LoadError> {
        let detection = index_detection(&self.raw[implementation])?;
        let probe = probe_index_detection(
            &detection,
            self.tool,
            self.executable,
            Some(self.shell),
            self.runner,
        )
        .map_err(LoadError::Pattern)?;
        Ok(match probe {
            IndexProbe::NotDetected => Probe::NotDetected,
            IndexProbe::Detected { version, .. } => Probe::Detected { version },
        })
    }

    fn resolve_scheme(&mut self, implementation: usize) -> Result<(), LoadError> {
        let spec = spec_value(&sym_or_str(&self.raw[implementation], "version_scheme"))?;
        self.scheme = Some(version_ranges::resolve(&spec).map_err(LoadError::Range)?);
        Ok(())
    }

    fn match_version(
        &mut self,
        implementation: usize,
        version: &str,
    ) -> Result<Option<usize>, LoadError> {
        let versions = sym_or_str(&self.raw[implementation], "versions");
        let specs = match &versions {
            Value::Array(items) => items
                .iter()
                .map(spec_value)
                .collect::<Result<Vec<_>, _>>()?,
            Value::Nil | Value::Bool(_) => return Err(no_method("each", &versions)),
            other => {
                return Err(unsupported(format!(
                    "implementation versions that are an instance of {} are not ported",
                    other.class_name()
                )))
            }
        };
        let scheme = self
            .scheme
            .as_ref()
            .ok_or_else(|| unsupported("version matched before its scheme was resolved"))?;
        version_ranges::find_matching_version_spec(&specs, version, scheme)
            .map_err(LoadError::Range)
    }

    fn load_version(
        &mut self,
        implementation: usize,
        file: &str,
    ) -> Result<Option<LoadedVersion>, LoadError> {
        let name = self.implementations[implementation]
            .name
            .clone()
            .unwrap_or_default();
        let Some(version) = load_implementation_version(self.register, self.tool, &name, file)?
        else {
            return Ok(None);
        };
        Ok(Some(LoadedVersion {
            version: if version.version.truthy() {
                Some(version.version.to_ruby_s())
            } else {
                None
            },
            profiles: profile_views(&version.execution_profiles)?,
        }))
    }
}

/// `Loader.load_with_implementation_index(name, options)` up to `Tool.new`:
/// the typed definition of tool `name` in `register`, or `None` when the
/// index, interface, implementation, version file or a compatible execution
/// profile is missing.
///
/// Detection commands run through `runner` with the explicit shell and the
/// source's five-second timeout. [`RegisterSource`] wraps this for the tool
/// runtime.
pub fn load_definition(
    register: &Register,
    name: &str,
    options: &LoadOptions<'_>,
    runner: &mut dyn CommandRunner,
) -> Result<Option<ToolDefinition>, LoadError> {
    let Some(index) = load_implementation_index(register, name)? else {
        return Ok(None);
    };
    let Some(interface) = load_interface(register, &index.interface.to_ruby_s())? else {
        return Ok(None);
    };
    let implementations = index
        .implementations
        .iter()
        .map(implementation)
        .collect::<Result<Vec<_>, _>>()?;
    let request = SelectionRequest {
        current_platform: options.current_platform,
        target_platform: options.platform,
        shell: options.shell,
    };
    let mut selection = Selection {
        register,
        tool: name,
        raw: &index.implementations,
        implementations: &implementations,
        executable: options.executable,
        shell: options.shell,
        runner,
        scheme: None,
    };
    let spec = match routing::detect_implementation_and_version(
        &implementations,
        &request,
        &mut selection,
    ) {
        Ok(spec) => spec,
        Err(RoutingError::Source(fault)) => return Err(LoadError::Source(fault)),
        Err(RoutingError::Env(error)) => return Err(error),
    };
    let Some(spec) = spec else {
        return Ok(None);
    };
    let Some(file) = spec.file.as_deref() else {
        return Err(type_error("no implicit conversion of nil into String"));
    };
    let implementation_name = spec.implementation_name.clone().unwrap_or_default();
    let Some(version) = load_implementation_version(register, name, &implementation_name, file)?
    else {
        return Ok(None);
    };
    convert_to_tool_definition(
        name,
        &interface,
        &version,
        spec.implementation_name.as_deref(),
        spec.version.as_deref(),
        options.platform.unwrap_or(options.current_platform),
        options.shell,
    )
}

/// The register as the tool runtime's [`DefinitionSource`]: `Tool.get`'s
/// `load_with_implementation_index` step. `ToolOptions::platform` is the
/// loader's `options[:platform]` and `ToolOptions::shell` its shell; the
/// current platform (`Platform.detect`) and the detection runner are fixed
/// when the source is built. A missing tool is `Ok(None)`, which `tool::get`
/// reports as `ToolNotFoundError`; loader exceptions keep their Ruby class.
pub struct RegisterSource<R> {
    register: Register,
    current_platform: Platform,
    executable: Option<String>,
    runner: RefCell<R>,
}

impl<R: CommandRunner> RegisterSource<R> {
    pub fn new(register: Register, current_platform: Platform, runner: R) -> Self {
        RegisterSource {
            register,
            current_platform,
            executable: None,
            runner: RefCell::new(runner),
        }
    }

    /// `options[:executable]`, the detection executable fallback.
    pub fn executable(mut self, executable: impl Into<String>) -> Self {
        self.executable = Some(executable.into());
        self
    }

    pub fn register(&self) -> &Register {
        &self.register
    }

    /// The runner, for callers that inspect what detection ran.
    pub fn into_runner(self) -> R {
        self.runner.into_inner()
    }
}

impl From<LoadError> for ToolError {
    fn from(e: LoadError) -> Self {
        match e {
            LoadError::Unsupported(_) | LoadError::Pattern(PatternError::Unsupported { .. }) => {
                ToolError::Source {
                    message: e.to_string(),
                }
            }
            other => ToolError::Ruby {
                class: other.class_name(),
                message: other.message(),
            },
        }
    }
}

impl<R: CommandRunner> DefinitionSource for RegisterSource<R> {
    fn load_definition(
        &self,
        name: &str,
        options: &ToolOptions,
    ) -> Result<Option<ToolDefinition>, ToolError> {
        let load = LoadOptions {
            current_platform: self.current_platform.as_str(),
            platform: Some(options.platform.as_str()),
            shell: options.shell.as_str(),
            executable: self.executable.as_deref(),
        };
        let mut runner = self.runner.borrow_mut();
        load_definition(&self.register, name, &load, &mut *runner).map_err(ToolError::from)
    }
}
