//! Source: lib/ukiryu/execution/output.rb (`Ukiryu::Execution::Output`).
//!
//! Captured process output. Not the serialisable `Ukiryu::Models::OutputInfo`
//! response model (K17 models).
use regex::Regex;
use serde::Serialize;

use super::ruby::split_lines;
use crate::types::ruby::strip;

/// A `stdout_contains?`/`stderr_contains?` argument: a String (substring test)
/// or a Regexp (match test).
///
/// Ruby Regexp and the `regex` crate differ in syntax: Ruby's `^`/`$` are
/// line anchors, so a caller porting `/^a$/` passes `(?m)^a$`.
#[derive(Debug, Clone, Copy)]
pub enum Pattern<'a> {
    Text(&'a str),
    Regex(&'a Regex),
}

impl Pattern<'_> {
    fn found_in(&self, haystack: &str) -> bool {
        match self {
            Pattern::Text(t) => haystack.contains(t),
            Pattern::Regex(r) => r.is_match(haystack),
        }
    }
}

impl<'a> From<&'a str> for Pattern<'a> {
    fn from(s: &'a str) -> Self {
        Pattern::Text(s)
    }
}

impl<'a> From<&'a Regex> for Pattern<'a> {
    fn from(r: &'a Regex) -> Self {
        Pattern::Regex(r)
    }
}

/// `Ukiryu::Execution::Output`. The exit status is the integer the executor
/// extracted (signals already mapped to `128 + n`); only `0` is success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub raw_stdout: String,
    pub raw_stderr: String,
    pub exit_status: i64,
}

/// `Output#to_h`, in Ruby key order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputHash {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: i64,
    pub success: bool,
    pub stdout_lines: Vec<String>,
    pub stderr_lines: Vec<String>,
}

impl Output {
    pub fn new(stdout: impl Into<String>, stderr: impl Into<String>, exit_status: i64) -> Self {
        Output {
            raw_stdout: stdout.into(),
            raw_stderr: stderr.into(),
            exit_status,
        }
    }

    /// `#stdout`: stripped stdout.
    pub fn stdout(&self) -> &str {
        strip(&self.raw_stdout)
    }

    /// `#stderr`: stripped stderr.
    pub fn stderr(&self) -> &str {
        strip(&self.raw_stderr)
    }

    /// `#stdout_lines`: `raw_stdout.split(/\r?\n/)`.
    pub fn stdout_lines(&self) -> Vec<String> {
        split_lines(&self.raw_stdout)
    }

    pub fn stderr_lines(&self) -> Vec<String> {
        split_lines(&self.raw_stderr)
    }

    /// `#stdout_contains?(pattern)` on the raw stdout.
    pub fn stdout_contains<'p>(&self, pattern: impl Into<Pattern<'p>>) -> bool {
        pattern.into().found_in(&self.raw_stdout)
    }

    pub fn stderr_contains<'p>(&self, pattern: impl Into<Pattern<'p>>) -> bool {
        pattern.into().found_in(&self.raw_stderr)
    }

    /// `#stdout_empty?`: empty after strip.
    pub fn is_stdout_empty(&self) -> bool {
        self.stdout().is_empty()
    }

    pub fn is_stderr_empty(&self) -> bool {
        self.stderr().is_empty()
    }

    /// `#stdout_length`: `String#length`, a character count (the Ruby doc
    /// comment says bytes; the code counts characters).
    pub fn stdout_length(&self) -> usize {
        self.raw_stdout.chars().count()
    }

    pub fn stderr_length(&self) -> usize {
        self.raw_stderr.chars().count()
    }

    /// `#success?`: `exit_status.zero?`.
    pub fn is_success(&self) -> bool {
        self.exit_status == 0
    }

    /// `#failure?`: `exit_status != 0`.
    pub fn is_failure(&self) -> bool {
        self.exit_status != 0
    }

    /// `#to_h`.
    pub fn to_h(&self) -> OutputHash {
        OutputHash {
            stdout: self.raw_stdout.clone(),
            stderr: self.raw_stderr.clone(),
            exit_status: self.exit_status,
            success: self.is_success(),
            stdout_lines: self.stdout_lines(),
            stderr_lines: self.stderr_lines(),
        }
    }

    /// `#inspect`.
    pub fn inspect(&self) -> String {
        format!(
            "#<Ukiryu::Execution::Output exit={} success={}>",
            self.exit_status,
            self.is_success()
        )
    }
}

/// `#to_s`.
impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (exit: {}, stdout: {} bytes, stderr: {} bytes)",
            if self.is_success() {
                "Success"
            } else {
                "Failed"
            },
            self.exit_status,
            self.stdout_length(),
            self.stderr_length()
        )
    }
}
