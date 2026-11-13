//! Profile choice, routed action paths and the executable a command runs:
//! `CommandResolution#find_command_profile`, `#resolve_action_path`,
//! `#find_action_with_parent`, the executable choice in
//! `Tool#execute_simple`, and `Tool.profile_is_modern?`.
use std::sync::OnceLock;

use regex::Regex;

use super::{text, Tool, ToolError};
use crate::executable::{is_executable, join_path, Platform};
use crate::models::{CommandDefinition, Text, ToolDefinition, VersionDetection};
use crate::runtime::{native_platform, ShellName};
use crate::types::Value;
use crate::version_ranges::{Bounds, RangeKind, Scheme, SpecValue, VersionRange};

/// `#find_command_profile`: a single profile is always used; among several,
/// the first whose platform and shell lists are empty or contain the
/// current platform and shell.
pub(super) fn find_command_profile(
    definition: &ToolDefinition,
    platform: Platform,
    shell: ShellName,
) -> Result<usize, ToolError> {
    let name = definition.name.as_ref().and_then(text).unwrap_or_default();
    let Some(profiles) = definition.profiles.as_ref() else {
        return Err(ToolError::ProfileNotFound {
            message: format!("No compatible profile for {name}"),
        });
    };
    if profiles.len() == 1 {
        return Ok(0);
    }
    let lists = |list: &Option<Vec<Text>>, want: &str| {
        let items = list.as_deref().unwrap_or_default();
        items.is_empty() || items.iter().any(|t| t.to_ruby_s() == want)
    };
    profiles
        .iter()
        .position(|p| lists(&p.platforms, platform.as_str()) && lists(&p.shells, shell.as_str()))
        .ok_or_else(|| ToolError::ProfileNotFound {
            message: format!(
                "No compatible profile for {name}. Current: {}/{}",
                platform.as_str(),
                shell.as_str()
            ),
        })
}

/// `#resolve_action_path` result: `{executable:, action:, path:}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionResolution<'a> {
    /// The routing target of the first path element (an executable name).
    pub executable: String,
    /// The action definition, when one exists.
    pub action: Option<&'a CommandDefinition>,
    pub path: Vec<String>,
}

/// Ruby `String#split(':')`: trailing empty fields are dropped.
pub(super) fn split_root_path(root_path: &str) -> Vec<String> {
    let mut parts: Vec<String> = root_path
        .split(':')
        .map(|s| s.trim_matches(ruby_strip).to_owned())
        .collect();
    // Ruby drops trailing empties before `map(&:strip)`.
    let raw: Vec<&str> = root_path.split(':').collect();
    let keep = raw.iter().rposition(|s| !s.is_empty()).map_or(0, |i| i + 1);
    parts.truncate(keep);
    parts
}

/// Characters `String#strip` removes (whitespace and NUL).
fn ruby_strip(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\0')
}

/// Ruby `Array#inspect` of a path of Strings.
pub(super) fn inspect_path(path: &[String]) -> String {
    Value::Array(path.iter().map(|s| Value::str(s.as_str())).collect()).inspect()
}

impl Tool {
    /// `#routing?`.
    pub fn has_routing(&self) -> bool {
        self.command_profile().has_routing()
    }

    /// `#resolve_action_path(path)`: the routing target of the first element
    /// and the action: for two or more elements the command named by the
    /// second that `belongs_to` the first, otherwise the command named by
    /// the only element.
    pub fn resolve_action_path(&self, path: &[String]) -> Option<ActionResolution<'_>> {
        if !self.has_routing() {
            return None;
        }
        let first = path.first()?;
        let target = self.command_profile().routing().resolve(first)?;
        let action = if path.len() > 1 {
            self.find_action_with_parent(&path[0], &path[1])
        } else {
            self.command_definition(&path[0])
        };
        Some(ActionResolution {
            executable: target.to_ruby_s(),
            action,
            path: path.to_vec(),
        })
    }

    /// `#find_action_with_parent(parent, action)`: the first command with
    /// that name whose `belongs_to` is the parent.
    pub fn find_action_with_parent(
        &self,
        parent: &str,
        action: &str,
    ) -> Option<&CommandDefinition> {
        self.commands().iter().find(|c| {
            c.belongs_to.as_ref().is_some_and(|b| b.eq_str(parent))
                && c.name.as_ref().is_some_and(|n| n.eq_str(action))
        })
    }

    /// The executable `#execute_simple` runs a command with: the tool's
    /// executable for a command with a subcommand or a modern profile;
    /// otherwise a standalone executable named after the command in the
    /// same directory, when the command allows it and it exists.
    pub(super) fn command_executable(
        &self,
        command: &CommandDefinition,
    ) -> Result<String, ToolError> {
        let missing = || ToolError::ExecutableNotFound {
            tool: self.name().unwrap_or_default(),
        };
        let executable = self.executable().ok_or_else(missing)?.to_owned();
        if command.has_subcommand() {
            return Ok(executable);
        }
        let version = self.definition.version.as_ref();
        let detection = self.definition.version_detection.as_deref();
        if profile_is_modern(version, detection)? == Some(true) {
            return Ok(executable);
        }
        let host = native_platform().unwrap_or(self.platform);
        let exe_dir = dirname(host, &executable);
        let exe_name = command
            .name
            .as_ref()
            .map(Text::to_ruby_s)
            .unwrap_or_default();
        let exe_path = join_path(host, &exe_dir, &exe_name);
        if command.is_standalone_executable()
            && is_executable(&exe_path)
            && dirname(host, &exe_path) == exe_dir
        {
            return Ok(exe_path);
        }
        Ok(executable)
    }
}

/// Ruby `File.dirname(path)` on `platform` (`/`, and `\` on Windows).
fn dirname(platform: Platform, path: &str) -> String {
    let sep = |c: char| c == '/' || (platform.is_windows() && c == '\\');
    let trimmed = path.trim_end_matches(sep);
    if trimmed.is_empty() {
        return if path.is_empty() {
            ".".into()
        } else {
            path[..1].into()
        };
    }
    match trimmed.rfind(sep) {
        None => ".".into(),
        Some(i) => {
            let dir = trimmed[..i].trim_end_matches(sep);
            if dir.is_empty() {
                trimmed[..1].into()
            } else {
                dir.into()
            }
        }
    }
}

fn line_regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static pattern"))
}

/// `Tool.profile_is_modern?(profile_version, version_detection)`: whether
/// the version is at least the detection block's `modern_threshold`.
/// `None` when there is no threshold, the version is not numeric, or either
/// side is not a valid `Gem::Version`. Date versions (`YYYY.MM.DD`) compare
/// as `YYYYMMDD` strings against an eight-digit threshold. A missing version
/// with a threshold is the source's `NoMethodError`.
pub fn profile_is_modern(
    profile_version: Option<&Text>,
    version_detection: Option<&VersionDetection>,
) -> Result<Option<bool>, ToolError> {
    static NUMERIC: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    static DIGITS8: OnceLock<Regex> = OnceLock::new();

    let Some(threshold) = version_detection.and_then(|vd| vd.modern_threshold.as_ref()) else {
        return Ok(None);
    };
    let Some(threshold) = text(threshold) else {
        return Ok(None);
    };
    let Some(version) = profile_version.and_then(Text::as_str) else {
        let receiver = profile_version.map_or(Value::Nil, Text::to_ruby_value);
        return Err(ToolError::Ruby {
            class: "NoMethodError",
            message: format!(
                "undefined method 'match?' for {}",
                if receiver.is_nil() {
                    "nil".to_owned()
                } else {
                    format!("an instance of {}", receiver.class_name())
                }
            ),
        });
    };
    if !line_regex(&NUMERIC, r"(?m)^[0-9]").is_match(version) {
        return Ok(None);
    }
    if line_regex(&DATE, r"(?m)^[0-9]{4}\.[0-9]{2}\.[0-9]{2}$").is_match(version)
        && line_regex(&DIGITS8, r"(?m)^[0-9]{8}$").is_match(&threshold)
    {
        let date = version.replace('.', "");
        return Ok(Some(date.as_str() >= threshold.as_str()));
    }
    Ok(gem_version_at_least(version, &threshold))
}

/// `Gem::Version.new(version) >= Gem::Version.new(threshold)`, `None` when
/// either is malformed. Uses the Versionian semantic `after` range, which
/// orders with the same `Gem::Version` port; it strips `+build` suffixes
/// first, which `Gem::Version` rejects instead, so those stay `None`.
fn gem_version_at_least(version: &str, threshold: &str) -> Option<bool> {
    if version.contains('+') || threshold.contains('+') {
        return None;
    }
    let scheme = Scheme::builtin("semantic").ok()?;
    let bounds = Bounds::new().with("version", SpecValue::Str(threshold.to_owned()));
    VersionRange::new(RangeKind::After, &scheme, bounds)
        .ok()?
        .matches(version)
        .ok()
}
