//! Source: lib/ukiryu/execution/result.rb (`Ukiryu::Execution::Result`).
use serde::Serialize;

use super::results::command_info::{CommandInfo, CommandInfoHash};
use super::results::metadata::{ExecutionMetadata, ExecutionMetadataHash};
use super::results::output::{Output, OutputHash, Pattern};
use super::results::ruby::{to_ruby_json, JsonGeneratorError, PathStyle};
use crate::types::value::Time;

/// `Ukiryu::Execution::Result`: command, output and timing of one run.
///
/// Success is `exit_status == 0` only. A profile exit-code meaning (even one
/// named `success`) never turns a non-zero status into success here.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionResult {
    pub command_info: CommandInfo,
    /// The composed `Output`. Ruby's `#output` method shadows this reader and
    /// returns stripped stdout; [`ExecutionResult::output`] does the same.
    pub output: Output,
    pub metadata: ExecutionMetadata,
}

/// `Result#to_h`, in Ruby key order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionResultHash {
    pub command: CommandInfoHash,
    pub output: OutputHash,
    pub metadata: ExecutionMetadataHash,
    pub success: bool,
    pub status: i64,
}

impl ExecutionResult {
    pub fn new(command_info: CommandInfo, output: Output, metadata: ExecutionMetadata) -> Self {
        ExecutionResult {
            command_info,
            output,
            metadata,
        }
    }

    /// `#command`: the full command string.
    pub fn command(&self) -> &str {
        &self.command_info.full_command
    }

    pub fn executable(&self) -> &str {
        &self.command_info.executable
    }

    pub fn executable_name(&self) -> String {
        self.command_info.executable_name()
    }

    /// `#stdout`: raw stdout.
    pub fn stdout(&self) -> &str {
        &self.output.raw_stdout
    }

    /// `#stderr`: raw stderr.
    pub fn stderr(&self) -> &str {
        &self.output.raw_stderr
    }

    /// `#status` (aliases `#exit_code`, `#exit_status`).
    pub fn status(&self) -> i64 {
        self.output.exit_status
    }

    pub fn exit_code(&self) -> i64 {
        self.status()
    }

    pub fn exit_status(&self) -> i64 {
        self.status()
    }

    pub fn started_at(&self) -> Option<&Time> {
        self.metadata.started_at.as_ref()
    }

    pub fn finished_at(&self) -> Option<&Time> {
        self.metadata.finished_at.as_ref()
    }

    /// `#duration` (alias `#execution_time`), seconds.
    pub fn duration(&self) -> Option<f64> {
        self.metadata.duration()
    }

    pub fn execution_time(&self) -> Option<f64> {
        self.metadata.duration_seconds()
    }

    pub fn is_success(&self) -> bool {
        self.output.is_success()
    }

    pub fn is_failure(&self) -> bool {
        self.output.is_failure()
    }

    /// `#output`: stripped stdout.
    pub fn output(&self) -> &str {
        self.output.stdout()
    }

    /// `#error_output`: stripped stderr.
    pub fn error_output(&self) -> &str {
        self.output.stderr()
    }

    pub fn stdout_lines(&self) -> Vec<String> {
        self.output.stdout_lines()
    }

    pub fn stderr_lines(&self) -> Vec<String> {
        self.output.stderr_lines()
    }

    pub fn stdout_contains<'p>(&self, pattern: impl Into<Pattern<'p>>) -> bool {
        self.output.stdout_contains(pattern)
    }

    pub fn stderr_contains<'p>(&self, pattern: impl Into<Pattern<'p>>) -> bool {
        self.output.stderr_contains(pattern)
    }

    /// `#to_h`.
    pub fn to_h(&self) -> ExecutionResultHash {
        self.to_h_with(PathStyle::host())
    }

    pub fn to_h_with(&self, style: PathStyle) -> ExecutionResultHash {
        ExecutionResultHash {
            command: self.command_info.to_h_with(style),
            output: self.output.to_h(),
            metadata: self.metadata.to_h(),
            success: self.is_success(),
            status: self.status(),
        }
    }

    /// `#to_json`: compact JSON as the json gem writes it. A NaN or Infinity
    /// (only possible through a Float timeout) fails like
    /// `JSON::GeneratorError`.
    pub fn to_json(&self) -> Result<String, JsonGeneratorError> {
        self.to_json_with(PathStyle::host())
    }

    pub fn to_json_with(&self, style: PathStyle) -> Result<String, JsonGeneratorError> {
        let h = self.to_h_with(style);
        let floats = h.metadata.floats();
        to_ruby_json(&h, &floats)
    }

    /// `#inspect`.
    pub fn inspect(&self) -> String {
        format!("#<Ukiryu::Execution::Result {self}>")
    }
}

/// `#to_s`.
impl std::fmt::Display for ExecutionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_success() {
            write!(
                f,
                "Success: {} ({})",
                self.command(),
                self.metadata.formatted_duration()
            )
        } else {
            write!(
                f,
                "Failed: {} (exit: {}, {})",
                self.command(),
                self.status(),
                self.metadata.formatted_duration()
            )
        }
    }
}
