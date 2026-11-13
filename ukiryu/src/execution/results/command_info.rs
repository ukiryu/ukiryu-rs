//! Source: lib/ukiryu/execution/command_info.rb (`Ukiryu::Execution::CommandInfo`).
//!
//! The command an execution ran. Not the serialisable
//! `Ukiryu::Models::CommandInfo` response model (K17 models).
use serde::Serialize;

use super::ruby::{basename, PathStyle};
use crate::types::ruby::string_inspect;

/// `Ukiryu::Execution::CommandInfo`. `shell`, `tool_name` and `command_name`
/// hold the `to_s` of the Ruby Symbol or String the executor passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    pub executable: String,
    pub arguments: Vec<String>,
    pub full_command: String,
    pub shell: Option<String>,
    pub tool_name: Option<String>,
    pub command_name: Option<String>,
}

/// `CommandInfo#to_h`, in Ruby key order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandInfoHash {
    pub executable: String,
    pub executable_name: String,
    pub tool_name: Option<String>,
    pub command_name: Option<String>,
    pub arguments: Vec<String>,
    pub full_command: String,
    pub shell: Option<String>,
}

impl CommandInfo {
    /// `#executable_name`: `File.basename(executable)` with the host's
    /// separator rules.
    pub fn executable_name(&self) -> String {
        self.executable_name_with(PathStyle::host())
    }

    /// `#executable_name` with explicit separator rules.
    pub fn executable_name_with(&self, style: PathStyle) -> String {
        basename(&self.executable, style)
    }

    /// `#argument_count`.
    pub fn argument_count(&self) -> usize {
        self.arguments.len()
    }

    /// `#to_h`.
    pub fn to_h(&self) -> CommandInfoHash {
        self.to_h_with(PathStyle::host())
    }

    pub fn to_h_with(&self, style: PathStyle) -> CommandInfoHash {
        CommandInfoHash {
            executable: self.executable.clone(),
            executable_name: self.executable_name_with(style),
            tool_name: self.tool_name.clone(),
            command_name: self.command_name.clone(),
            arguments: self.arguments.clone(),
            full_command: self.full_command.clone(),
            shell: self.shell.clone(),
        }
    }

    /// `#inspect`.
    pub fn inspect(&self) -> String {
        self.inspect_with(PathStyle::host())
    }

    pub fn inspect_with(&self, style: PathStyle) -> String {
        format!(
            "#<Ukiryu::Execution::CommandInfo exe={} args={}>",
            string_inspect(&self.executable_name_with(style)),
            self.argument_count()
        )
    }
}

/// `#to_s`: the full command.
impl std::fmt::Display for CommandInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.full_command)
    }
}
