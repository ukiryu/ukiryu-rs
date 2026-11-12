//! `Ukiryu::ToolIndex` (lib/ukiryu/tool_index.rb) over supplied register files.

use super::version_order::{compare, segments};
use super::SourceFault;
use num_bigint::BigInt;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;

/// One `tools/<tool>/<implementation>/<file>.yaml` file as the loader read it.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterFile {
    /// The file's basename, e.g. `1.0.yaml`.
    pub file: String,
    /// `YAML.safe_load(content, permitted_classes: [Symbol], aliases: true)`,
    /// or the parse error's message.
    pub document: Result<Value, String>,
}

/// One `tools/<tool>/<implementation>/` directory.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterImplementation {
    pub name: String,
    /// Only the directory's `*.yaml` files.
    pub files: Vec<RegisterFile>,
}

/// One `tools/<tool>/` directory. `index.yaml` at the tool root is not part of
/// the index scan and is not listed here.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterTool {
    pub name: String,
    pub implementations: Vec<RegisterImplementation>,
}

/// A register file chosen by the index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileRef {
    pub tool: String,
    pub implementation: String,
    pub file: String,
}

/// A tool whose scan stopped early. The source rescues the error per tool
/// directory, prints a warning and keeps the entries already indexed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexWarning {
    pub tool: String,
    pub fault: SourceFault,
}

/// The versioned-interface fallback order of `Register.load_tool_metadata`:
/// `name`, `name/1.0`, `name/1`, `vname/1.0`. (The loader tries the exact tool
/// name before any of these.)
pub fn interface_lookup_keys(name: &str) -> [String; 4] {
    [
        name.to_string(),
        format!("{name}/1.0"),
        format!("{name}/1"),
        format!("v{name}/1.0"),
    ]
}

/// Interface and alias index of a register.
///
/// Built once from the scanned files (the source rebuilds when the tools
/// directory's mtime or file count changes; change detection stays with the
/// loader, which rebuilds by calling [`ToolIndex::build`] again).
#[derive(Clone, Debug, Default)]
pub struct ToolIndex {
    tools: Vec<RegisterTool>,
    by_tool: HashMap<String, usize>,
    interfaces: Vec<(String, Vec<String>)>,
    interface_slot: HashMap<String, usize>,
    aliases: Vec<(String, Vec<String>)>,
    alias_slot: HashMap<String, usize>,
    warnings: Vec<IndexWarning>,
}

fn push_unique(
    list: &mut Vec<(String, Vec<String>)>,
    slot: &mut HashMap<String, usize>,
    key: &str,
    tool: &str,
) {
    let i = *slot.entry(key.to_string()).or_insert_with(|| {
        list.push((key.to_string(), Vec::new()));
        list.len() - 1
    });
    if !list[i].1.iter().any(|t| t == tool) {
        list[i].1.push(tool.to_string());
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "nil",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Number(n) if n.is_i64() || n.is_u64() => "an instance of Integer",
        Value::Number(_) => "an instance of Float",
        Value::String(_) => "an instance of String",
        Value::Array(_) => "an instance of Array",
        Value::Object(_) => "an instance of Hash",
    }
}

/// `value.to_sym` for a YAML value: only strings have it.
fn to_sym(value: &Value) -> Result<&str, SourceFault> {
    match value {
        Value::String(s) => Ok(s),
        other => Err(SourceFault::new(
            "NoMethodError",
            format!("undefined method 'to_sym' for {}", type_name(other)),
        )),
    }
}

/// `document[key]` with Ruby's receiver-dependent `#[]`. `Ok(None)` is nil.
fn ruby_index(document: &Value, key: &str) -> Result<Option<Value>, SourceFault> {
    match document {
        Value::Object(map) => Ok(map.get(key).cloned()),
        // String#[](str): the substring when present.
        Value::String(s) => Ok(s.contains(key).then(|| Value::String(key.to_string()))),
        // Array#[] and Integer#[] want an integer.
        Value::Array(_) => Err(type_error()),
        Value::Number(n) if !n.is_f64() => Err(type_error()),
        other => Err(SourceFault::new(
            "NoMethodError",
            format!("undefined method '[]' for {}", type_name(other)),
        )),
    }
}

fn type_error() -> SourceFault {
    SourceFault::new("TypeError", "no implicit conversion of String into Integer")
}

/// Ruby truthiness: everything except nil and false.
fn value_truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

fn truthy(value: &Option<Value>) -> bool {
    value.as_ref().is_some_and(value_truthy)
}

/// Ruby `to_s` of a scalar YAML value, `None` for values the port does not
/// render (arrays and hashes would need Ruby `inspect`).
fn scalar_to_s(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

impl ToolIndex {
    /// `build_index`. Tool directories, implementation directories and files
    /// are visited in byte order of their names (the source's sorted globs).
    pub fn build(mut tools: Vec<RegisterTool>) -> ToolIndex {
        tools.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        for tool in &mut tools {
            tool.implementations
                .sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
            for implementation in &mut tool.implementations {
                implementation
                    .files
                    .sort_by(|a, b| a.file.as_bytes().cmp(b.file.as_bytes()));
            }
        }
        let mut index = ToolIndex {
            by_tool: tools
                .iter()
                .enumerate()
                .map(|(i, t)| (t.name.clone(), i))
                .collect(),
            ..ToolIndex::default()
        };
        for tool in &tools {
            if let Err(fault) = index.scan_tool(tool) {
                index.warnings.push(IndexWarning {
                    tool: tool.name.clone(),
                    fault,
                });
            }
        }
        index.tools = tools;
        index
    }

    fn scan_tool(&mut self, tool: &RegisterTool) -> Result<(), SourceFault> {
        for implementation in &tool.implementations {
            for file in &implementation.files {
                let document = file
                    .document
                    .as_ref()
                    .map_err(|m| SourceFault::new("Psych::SyntaxError", m.clone()))?;
                if !value_truthy(document) {
                    continue;
                }
                let implements = ruby_index(document, "implements")?;
                if truthy(&implements) {
                    let interfaces: Vec<String> = match implements.as_ref().unwrap() {
                        Value::Array(items) => items
                            .iter()
                            .map(|v| to_sym(v).map(str::to_string))
                            .collect::<Result<_, _>>()?,
                        other => vec![to_sym(other)?.to_string()],
                    };
                    for interface in &interfaces {
                        push_unique(
                            &mut self.interfaces,
                            &mut self.interface_slot,
                            interface,
                            &tool.name,
                        );
                    }
                }
                // `next unless aliases.respond_to?(:each)`: arrays and hashes.
                match ruby_index(document, "aliases")? {
                    Some(Value::Array(items)) => {
                        for item in &items {
                            let alias = to_sym(item)?;
                            push_unique(&mut self.aliases, &mut self.alias_slot, alias, &tool.name);
                        }
                    }
                    Some(Value::Object(map)) => {
                        if !map.is_empty() {
                            // Hash#each yields [key, value] pairs.
                            return Err(SourceFault::new(
                                "NoMethodError",
                                "undefined method 'to_sym' for an instance of Array",
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Tools whose scan stopped on a fault, in scan order.
    pub fn warnings(&self) -> &[IndexWarning] {
        &self.warnings
    }

    /// `all_tools`: interface name to tool names, in first-seen order.
    pub fn all_tools(&self) -> &[(String, Vec<String>)] {
        &self.interfaces
    }

    /// The alias table: alias to tool names, in first-seen order.
    pub fn aliases(&self) -> &[(String, Vec<String>)] {
        &self.aliases
    }

    /// `find_all_by_interface`.
    pub fn find_all_by_interface(&self, interface: &str) -> &[String] {
        self.interface_slot
            .get(interface)
            .map(|&i| self.interfaces[i].1.as_slice())
            .unwrap_or(&[])
    }

    fn document(&self, file: &FileRef) -> &Result<Value, String> {
        let tool = &self.tools[self.by_tool[&file.tool]];
        let implementation = tool
            .implementations
            .iter()
            .find(|i| i.name == file.implementation)
            .expect("file ref from this index");
        &implementation
            .files
            .iter()
            .find(|f| f.file == file.file)
            .expect("file ref from this index")
            .document
    }

    /// `load_yaml_for_tool`: the file whose content the index reads for a tool.
    ///
    /// The `default` implementation directory wins; otherwise the first
    /// directory (byte order) that has any `.yaml` file. Within it a single
    /// file is taken as is; with several, the file whose `version:` is highest
    /// by `SemanticVersion` order. Files without a parseable document or a
    /// truthy `version` are skipped; equal segment lists keep the first key
    /// position but the later file; among keys that compare equal the first
    /// wins. When no candidate survives the result is `None` and no other
    /// directory is tried.
    pub fn primary_file(&self, tool: &str) -> Option<FileRef> {
        let tool = &self.tools[*self.by_tool.get(tool)?];
        let implementation = tool
            .implementations
            .iter()
            .find(|i| i.name == "default" && !i.files.is_empty())
            .or_else(|| tool.implementations.iter().find(|i| !i.files.is_empty()))?;
        let reference = |file: &RegisterFile| FileRef {
            tool: tool.name.clone(),
            implementation: implementation.name.clone(),
            file: file.file.clone(),
        };
        if implementation.files.len() == 1 {
            return Some(reference(&implementation.files[0]));
        }
        let mut keys: Vec<(Vec<BigInt>, &RegisterFile)> = Vec::new();
        for file in &implementation.files {
            let Ok(document) = &file.document else {
                continue;
            };
            if !value_truthy(document) {
                continue;
            }
            let Ok(version) = ruby_index(document, "version") else {
                continue;
            };
            if !truthy(&version) {
                continue;
            }
            // Non-scalar versions (Ruby would render them with `inspect`) are
            // not ported; they are skipped.
            let Some(text) = scalar_to_s(version.as_ref().unwrap()) else {
                continue;
            };
            let key = segments(&text);
            match keys.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = file,
                None => keys.push((key, file)),
            }
        }
        let mut best: Option<&(Vec<BigInt>, &RegisterFile)> = None;
        for entry in &keys {
            if best.is_none_or(|b| compare(&entry.0, &b.0) == Ordering::Greater) {
                best = Some(entry);
            }
        }
        best.map(|(_, file)| reference(file))
    }

    /// `find_by_interface`: the primary file of the first tool implementing
    /// `interface` whose primary document loads as metadata. The loader builds
    /// `ToolMetadata` from the returned file.
    pub fn find_by_interface(&self, interface: &str) -> Result<Option<FileRef>, SourceFault> {
        for tool in self.find_all_by_interface(interface) {
            let Some(file) = self.primary_file(tool) else {
                continue;
            };
            let document = self
                .document(&file)
                .as_ref()
                .map_err(|m| SourceFault::new("Psych::SyntaxError", m.clone()))?;
            if value_truthy(document) {
                // `ToolMetadata.from_hash` reads `hash['implements']` and friends.
                ruby_index(document, "implements")?;
                return Ok(Some(file));
            }
        }
        Ok(None)
    }

    /// [`find_by_interface`](Self::find_by_interface) over
    /// [`interface_lookup_keys`], the index half of `Register.load_tool_metadata`.
    pub fn find_by_interface_keys(&self, name: &str) -> Result<Option<FileRef>, SourceFault> {
        for key in interface_lookup_keys(name) {
            if let Some(file) = self.find_by_interface(&key)? {
                return Ok(Some(file));
            }
        }
        Ok(None)
    }

    /// `tool_compatible?`: whether the tool's primary document has a
    /// `profiles:` entry for `platform` and `shell`, where a missing or empty
    /// list matches everything. Documents that only carry `execution_profiles`
    /// are never compatible.
    pub fn tool_compatible(
        &self,
        tool: &str,
        platform: &str,
        shell: &str,
    ) -> Result<bool, SourceFault> {
        let Some(file) = self.primary_file(tool) else {
            return Ok(false);
        };
        let document = self
            .document(&file)
            .as_ref()
            .map_err(|m| SourceFault::new("Psych::SyntaxError", m.clone()))?;
        let profiles = match document {
            Value::Object(map) => map.get("profiles"),
            Value::String(_) | Value::Array(_) => {
                return Err(SourceFault::new(
                    "Lutaml::Model::InvalidFormatError",
                    "input format is invalid, try to pass correct `yaml` format",
                ))
            }
            _ => {
                return Err(SourceFault::new(
                    "Unsupported",
                    "a scalar tool document is not ported",
                ))
            }
        };
        let Some(profiles) = profiles else {
            return Ok(false);
        };
        let profiles = match profiles {
            Value::Null => return Ok(false),
            Value::Array(items) => items,
            _ => {
                return Err(SourceFault::new(
                    "Unsupported",
                    "a non-list `profiles` value is not ported",
                ))
            }
        };
        for profile in profiles {
            let Value::Object(profile) = profile else {
                return Err(SourceFault::new(
                    "Unsupported",
                    "a non-hash profile is not ported",
                ));
            };
            if list_matches(profile.get("platforms"), platform)?
                && list_matches(profile.get("shells"), shell)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `find_by_alias`: the only tool with the alias, or the first one
    /// compatible with `platform`/`shell`, or else the first one.
    pub fn find_by_alias(
        &self,
        alias: &str,
        platform: &str,
        shell: &str,
    ) -> Result<Option<&str>, SourceFault> {
        let Some(&slot) = self.alias_slot.get(alias) else {
            return Ok(None);
        };
        let candidates = &self.aliases[slot].1;
        if candidates.len() == 1 {
            return Ok(Some(&candidates[0]));
        }
        for tool in candidates {
            if self.tool_compatible(tool, platform, shell)? {
                return Ok(Some(tool));
            }
        }
        Ok(candidates.first().map(String::as_str))
    }
}

/// An empty or missing list is universal; otherwise exact membership.
fn list_matches(list: Option<&Value>, wanted: &str) -> Result<bool, SourceFault> {
    match list {
        None | Some(Value::Null) => Ok(true),
        Some(Value::Array(items)) => {
            if items.is_empty() {
                return Ok(true);
            }
            for item in items {
                match scalar_to_s(item) {
                    Some(text) if text == wanted => return Ok(true),
                    Some(_) => {}
                    None => {
                        return Err(SourceFault::new(
                            "Unsupported",
                            "a non-scalar platform or shell entry is not ported",
                        ))
                    }
                }
            }
            Ok(false)
        }
        Some(_) => Err(SourceFault::new(
            "Unsupported",
            "a non-list platforms/shells value is not ported",
        )),
    }
}
