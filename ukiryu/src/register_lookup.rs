//! Register-backed tool lookup: the filesystem half of `Ukiryu::ToolIndex`,
//! `Register.load_tool_metadata`, `Tools::Generator.load_tool_definition` and
//! `ToolFinder.find_by` (lib/ukiryu/tool_index.rb, register.rb,
//! tools/generator.rb and tool_finder.rb of `ukiryu/ukiryu` at `ab6df353`).
//!
//! [`crate::routing::ToolIndex`] holds the index logic but is built from files
//! somebody else scanned. This module is that somebody:
//!
//! * [`scan_register`] walks `tools/<tool>/<implementation>/*.yaml` of a
//!   [`Register`] into [`RegisterTool`] values, and [`load_tool_index`] builds
//!   the [`ToolIndex`] from them. A file that cannot be read or parsed is kept
//!   as an `Err` document, so the index reproduces the source's partial index
//!   (the failing tool stops scanning and is reported in
//!   [`ToolIndex::warnings`]; earlier tools and entries stay indexed).
//! * [`load_tool_metadata`] is `Register.load_tool_metadata`: exact tool name
//!   first, then the interface keys.
//! * [`load_tool_definition`] is `Generator.load_tool_definition`, the
//!   loader of the version file `tools/<name>/index.yaml` lists last (or the
//!   requested one), with inheritance resolved; use
//!   [`ToolDefinition::compatible_profile`] on its result.
//! * [`find_by`] is `ToolFinder.find_by`: exact name through [`tool::get`],
//!   then the interface candidates (the first one that is available), the
//!   alias, and the exhaustive search over every tool of the register.
//!
//! Source-observable behavior kept as is: a missing identifier is `Ok(None)`;
//! only `ToolNotFoundError` and `ProfileNotFoundError` are rescued in the
//! exact, interface and alias steps; the exhaustive step rescues only
//! `ToolNotFoundError` (a `ProfileNotFoundError` there escapes); every other
//! error escapes with its class.
//!
//! Not ported: the `UKIRYU_DEBUG` structured debug output of `find_by`, and
//! the source's mtime-based index refresh (build a new index to refresh).
//! Documents the port cannot represent faithfully (integers outside 64 bits,
//! non-finite floats, timestamps and other classes `YAML.safe_load` rejects)
//! are kept as `Err` documents, which the index treats as a parse failure of
//! that file. An unreadable file is likewise an `Err` document; the source
//! raises an `Errno` there and the index reports it as `Psych::SyntaxError`
//! in the warning, the only difference.

use crate::cache::ToolCache;
use crate::models::{Model, ModelError, ToolDefinition, ToolMetadata, Value};
use crate::register::{self, Register};
use crate::routing::{
    FileRef, RegisterFile, RegisterImplementation, RegisterTool, SourceFault, ToolIndex,
};
use crate::tool::{self, DefinitionSource, Tool, ToolError, ToolOptions};
use serde_json::Value as Json;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Why a lookup failed. Each variant keeps the Ruby class the source raises.
#[derive(Debug)]
pub enum LookupError {
    /// Reading or parsing a register file failed.
    Register(register::Error),
    /// A YAML document could not be parsed or a model could not be built.
    Model(ModelError),
    /// The index code itself raised (`NoMethodError`, `TypeError`, ...).
    Source(SourceFault),
    /// `Tool.get` or `Tool.new` raised something the finder does not rescue.
    Tool(ToolError),
}

impl LookupError {
    /// The Ruby exception class the source raises, as far as the port knows it.
    pub fn class_name(&self) -> String {
        match self {
            LookupError::Register(e) => e.class_name().to_string(),
            LookupError::Model(e) => e.ruby_class().to_string(),
            LookupError::Source(e) => e.class.to_string(),
            LookupError::Tool(e) => match e {
                ToolError::Ruby { class, .. } => (*class).to_string(),
                other => other
                    .kind()
                    .map(|k| k.ruby_class().to_string())
                    .unwrap_or_else(|| "Unclassified".to_string()),
            },
        }
    }
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LookupError::Register(e) => write!(f, "{e}"),
            LookupError::Model(e) => write!(f, "{e}"),
            LookupError::Source(e) => write!(f, "{e}"),
            LookupError::Tool(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LookupError {}

impl From<register::Error> for LookupError {
    fn from(e: register::Error) -> Self {
        LookupError::Register(e)
    }
}

impl From<ModelError> for LookupError {
    fn from(e: ModelError) -> Self {
        LookupError::Model(e)
    }
}

impl From<SourceFault> for LookupError {
    fn from(e: SourceFault) -> Self {
        LookupError::Source(e)
    }
}

impl From<ToolError> for LookupError {
    fn from(e: ToolError) -> Self {
        LookupError::Tool(e)
    }
}

/// `Dir.glob(dir/*)` entries: names not starting with a dot, byte-sorted.
/// An unreadable directory yields nothing (a missing one globs to `[]`).
fn glob_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (!name.starts_with('.')).then(|| (name, e.path()))
        })
        .collect();
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    out
}

fn has_yaml_suffix(name: &str) -> bool {
    if cfg!(windows) {
        name.to_ascii_lowercase().ends_with(".yaml")
    } else {
        name.ends_with(".yaml")
    }
}

/// `YAML.safe_load(File.read(file), permitted_classes: [Symbol], aliases: true)`
/// as the index consumes it: the document, or the failure's message.
fn read_document(file: &Path) -> Result<Json, String> {
    let content = fs::read_to_string(file).map_err(|e| format!("{e} @ {}", file.display()))?;
    if content.trim().is_empty() {
        // `safe_load("")` is `false`, which the index skips like nil.
        return Ok(Json::Null);
    }
    let tree: yeptris::Value = crate::from_yaml(&content).map_err(|e| e.to_string())?;
    let value = Value::from_yaml_value(&tree).map_err(|e| e.to_string())?;
    to_json(&value)
}

fn to_json(value: &Value) -> Result<Json, String> {
    Ok(match value {
        Value::Nil => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Integer(i) => {
            if let Ok(v) = i64::try_from(i) {
                Json::from(v)
            } else if let Ok(v) = u64::try_from(i) {
                Json::from(v)
            } else {
                return Err("Unsupported: integer outside 64 bits".to_string());
            }
        }
        Value::Float(f) => match serde_json::Number::from_f64(*f) {
            Some(n) => Json::Number(n),
            None => return Err("Unsupported: non-finite float".to_string()),
        },
        Value::String(s) | Value::Symbol(s) => Json::String(s.clone()),
        Value::Time(_) => return Err("Tried to load unspecified class: Time".to_string()),
        Value::Regexp(_) => return Err("Tried to load unspecified class: Regexp".to_string()),
        Value::Array(items) => Json::Array(items.iter().map(to_json).collect::<Result<_, _>>()?),
        Value::Hash(pairs) => {
            let mut map = serde_json::Map::new();
            for (key, item) in pairs {
                let key = match key {
                    Value::String(s) | Value::Symbol(s) => s.clone(),
                    other => other.to_ruby_s(),
                };
                map.insert(key, to_json(item)?);
            }
            Json::Object(map)
        }
    })
}

/// The `tools/<tool>/<implementation>/*.yaml` walk of `ToolIndex#build_index`.
///
/// Tool directories, implementation directories and files are listed in byte
/// order; names starting with a dot are skipped (`Dir.glob('*')`); only
/// directories count as tools and implementations; `tools/<tool>/index.yaml`
/// is not part of the walk. Every file's document is read here, so a broken
/// file shows up as an `Err` document for the index to turn into a warning.
/// A register without a `tools` directory scans to nothing.
pub fn scan_register(register: &Register) -> Vec<RegisterTool> {
    let tools_dir = register.path().join("tools");
    glob_entries(&tools_dir)
        .into_iter()
        .filter(|(_, path)| path.is_dir())
        .map(|(name, tool_dir)| RegisterTool {
            name,
            implementations: glob_entries(&tool_dir)
                .into_iter()
                .filter(|(_, path)| path.is_dir())
                .map(|(name, impl_dir)| RegisterImplementation {
                    name,
                    files: glob_entries(&impl_dir)
                        .into_iter()
                        .filter(|(file, _)| has_yaml_suffix(file))
                        .map(|(file, path)| RegisterFile {
                            file,
                            document: read_document(&path),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

/// [`ToolIndex::build`] over [`scan_register`].
pub fn load_tool_index(register: &Register) -> ToolIndex {
    ToolIndex::build(scan_register(register))
}

fn read_value(path: &Path) -> Result<Value, LookupError> {
    let content = fs::read_to_string(path).map_err(|error| register::Error::Io {
        path: path.to_path_buf(),
        error,
    })?;
    if content.trim().is_empty() {
        return Ok(Value::Nil);
    }
    let tree: yeptris::Value = crate::from_yaml(&content).map_err(|e| register::Error::Yaml {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(Value::from_yaml_value(&tree)?)
}

fn indexed_path(register: &Register, file: &FileRef) -> PathBuf {
    register
        .path()
        .join("tools")
        .join(&file.tool)
        .join(&file.implementation)
        .join(&file.file)
}

/// `Register.load_tool_metadata(name)`.
///
/// The exact name goes through [`Register::load_tool_yaml`] (the last
/// version file `tools/<name>/index.yaml` lists); a non-empty document wins. Otherwise
/// `index` is asked for an interface named `name`, `name/1.0`, `name/1` and
/// `vname/1.0`, in that order, and the first tool's primary file becomes the
/// metadata, named after that tool. `Ok(None)` when nothing matches.
pub fn load_tool_metadata(
    register: &Register,
    index: &ToolIndex,
    name: &str,
) -> Result<Option<ToolMetadata>, LookupError> {
    let register_path = register.path().to_string_lossy().into_owned();
    if let Some(content) = register.load_tool_yaml(name, None)? {
        let tree: yeptris::Value =
            crate::from_yaml(&content).map_err(|e| register::Error::Yaml {
                path: register.path().to_path_buf(),
                message: e.to_string(),
            })?;
        let hash = Value::from_yaml_value(&tree)?;
        if hash.truthy() {
            return Ok(Some(ToolMetadata::from_hash(
                &hash,
                name,
                Some(register_path.as_str()),
            )?));
        }
    }
    let Some(file) = index.find_by_interface_keys(name)? else {
        return Ok(None);
    };
    let hash = read_value(&indexed_path(register, &file))?;
    if !hash.truthy() {
        return Ok(None);
    }
    Ok(Some(ToolMetadata::from_hash(
        &hash,
        &file.tool,
        Some(register_path.as_str()),
    )?))
}

/// `Tools::Generator.load_tool_definition(name, version:)`: the
/// version file [`Register::load_tool_yaml`] selects as a typed definition with profile inheritance
/// resolved, or `None` when the tool has no such file. Select the profile for
/// a platform and shell with [`ToolDefinition::compatible_profile`].
pub fn load_tool_definition(
    register: &Register,
    name: &str,
    version: Option<&str>,
) -> Result<Option<ToolDefinition>, LookupError> {
    let Some(content) = register.load_tool_yaml(name, version)? else {
        return Ok(None);
    };
    let mut definition = ToolDefinition::from_yaml(&content)?;
    definition.resolve_inheritance();
    Ok(Some(definition))
}

/// `tool_def.implements` includes `identifier` (an element that is not a
/// string raises like `to_sym` does), or `tool_def.aliases` does.
fn matches_identifier(definition: &ToolDefinition, identifier: &str) -> Result<bool, LookupError> {
    if let Some(implements) = &definition.implements {
        for item in implements {
            if item.to_sym()? == identifier {
                return Ok(true);
            }
        }
    }
    Ok(definition
        .aliases
        .iter()
        .flatten()
        .any(|a| a.eq_str(identifier)))
}

fn not_found(error: &ToolError) -> bool {
    matches!(
        error,
        ToolError::NotFound { .. } | ToolError::ProfileNotFound { .. }
    )
}

/// `ToolFinder.find_by(identifier, options)`.
///
/// Order: exact name; every interface candidate of `index` in index order,
/// returning the first whose executable is found; the alias, chosen by
/// `ToolIndex#find_by_alias`; then the exhaustive search over
/// [`Register::tool_names`]. `Ok(None)` is the source's `nil`.
///
/// `options` carries the platform and shell the tools are built for.
/// `runtime` carries the `Runtime.instance` platform and shell, which the
/// source's alias-collision check reads regardless of `options`; only its
/// platform and shell are used.
pub fn find_by(
    register: &Register,
    index: &ToolIndex,
    source: &dyn DefinitionSource,
    cache: &ToolCache<Rc<Tool>>,
    identifier: &str,
    options: &ToolOptions,
    runtime: &ToolOptions,
) -> Result<Option<Rc<Tool>>, LookupError> {
    // 1. Exact name.
    match tool::get(source, cache, identifier, options) {
        Ok(tool) => return Ok(Some(tool)),
        Err(e) if not_found(&e) => {}
        Err(e) => return Err(e.into()),
    }

    // 2. Interface candidates: the first that is available.
    for name in index.find_all_by_interface(identifier) {
        match tool::get(source, cache, name, options) {
            Ok(tool) if tool.is_available() => return Ok(Some(tool)),
            Ok(_) => {}
            Err(e) if not_found(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }

    // 3. Alias.
    if let Some(name) = index.find_by_alias(
        identifier,
        runtime.platform.as_str(),
        runtime.shell.as_str(),
    )? {
        match tool::get(source, cache, name, options) {
            Ok(tool) => return Ok(Some(tool)),
            Err(e) if not_found(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }

    // 4. Exhaustive search.
    exhaustive_search(register, source, cache, identifier, options)
}

fn exhaustive_search(
    register: &Register,
    source: &dyn DefinitionSource,
    cache: &ToolCache<Rc<Tool>>,
    identifier: &str,
    options: &ToolOptions,
) -> Result<Option<Rc<Tool>>, LookupError> {
    for name in register.tool_names() {
        let tool = match tool::get(source, cache, &name, options) {
            Ok(tool) => tool,
            Err(ToolError::NotFound { .. }) => {
                // Old architecture: the version file `index.yaml` lists.
                let Some(definition) = load_tool_definition(register, &name, None)? else {
                    continue;
                };
                if !matches_identifier(&definition, identifier)? {
                    continue;
                }
                let platform = options.platform.as_str();
                let shell = options.shell.as_str();
                if definition.compatible_profile(platform, shell)?.is_none() {
                    continue;
                }
                let key = tool::cache_key(&name, options);
                if let Some(cached) = cache.get(&key) {
                    return Ok(Some(cached));
                }
                let tool = Rc::new(Tool::new(definition, options)?);
                cache.set(key, Rc::clone(&tool));
                tool
            }
            Err(e) => return Err(e.into()),
        };
        if matches_identifier(tool.definition(), identifier)? {
            return Ok(Some(tool));
        }
    }
    Ok(None)
}
