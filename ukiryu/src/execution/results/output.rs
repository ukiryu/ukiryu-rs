//! Source: lib/ukiryu/execution/output.rb (`Ukiryu::Execution::Output`).
//!
//! Captured process output. Not the serialisable `Ukiryu::Models::OutputInfo`
//! response model (K17 models).
//!
//! The streams are the bytes Ruby read from the pipes (a UTF-8 tagged String
//! that may hold invalid sequences). Readers that Ruby implements with a
//! method that raises for an invalid sequence return that error; see
//! [`super::bytes`].
use regex::Regex;
use serde::Serialize;

use super::bytes::{self, InvalidByteSequence};

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
    /// `include?` works on any bytes; a Regexp match raises `ArgumentError`
    /// on an invalid sequence.
    fn found_in(&self, haystack: &[u8]) -> Result<bool, InvalidByteSequence> {
        match self {
            Pattern::Text(t) => Ok(bytes::contains(haystack, t)),
            Pattern::Regex(r) => bytes::valid_text(haystack).map(|text| r.is_match(text)),
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
    /// stdout as Ruby read it from the pipe.
    pub raw_stdout: Vec<u8>,
    /// stderr as Ruby read it from the pipe.
    pub raw_stderr: Vec<u8>,
    pub exit_status: i64,
}

/// `Output#to_h`, in Ruby key order. Only built when both streams are valid
/// UTF-8 (`stdout_lines`/`stderr_lines` raise otherwise).
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
    pub fn new(stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>, exit_status: i64) -> Self {
        Output {
            raw_stdout: stdout.into(),
            raw_stderr: stderr.into(),
            exit_status,
        }
    }

    /// `#stdout`: stripped stdout.
    pub fn stdout(&self) -> Result<&str, InvalidByteSequence> {
        bytes::strip(&self.raw_stdout)
    }

    /// `#stderr`: stripped stderr.
    pub fn stderr(&self) -> Result<&str, InvalidByteSequence> {
        bytes::strip(&self.raw_stderr)
    }

    /// `#stdout_lines`: `raw_stdout.split(/\r?\n/)`.
    pub fn stdout_lines(&self) -> Result<Vec<String>, InvalidByteSequence> {
        bytes::split_lines(&self.raw_stdout)
    }

    pub fn stderr_lines(&self) -> Result<Vec<String>, InvalidByteSequence> {
        bytes::split_lines(&self.raw_stderr)
    }

    /// `#stdout_contains?(pattern)` on the raw stdout.
    pub fn stdout_contains<'p>(
        &self,
        pattern: impl Into<Pattern<'p>>,
    ) -> Result<bool, InvalidByteSequence> {
        pattern.into().found_in(&self.raw_stdout)
    }

    pub fn stderr_contains<'p>(
        &self,
        pattern: impl Into<Pattern<'p>>,
    ) -> Result<bool, InvalidByteSequence> {
        pattern.into().found_in(&self.raw_stderr)
    }

    /// `#stdout_empty?`: empty after strip.
    pub fn is_stdout_empty(&self) -> Result<bool, InvalidByteSequence> {
        self.stdout().map(str::is_empty)
    }

    pub fn is_stderr_empty(&self) -> Result<bool, InvalidByteSequence> {
        self.stderr().map(str::is_empty)
    }

    /// `#stdout_length`: `String#length`, a character count (the Ruby doc
    /// comment says bytes; the code counts characters).
    pub fn stdout_length(&self) -> usize {
        bytes::length(&self.raw_stdout)
    }

    pub fn stderr_length(&self) -> usize {
        bytes::length(&self.raw_stderr)
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
    pub fn to_h(&self) -> Result<OutputHash, InvalidByteSequence> {
        let stdout_lines = self.stdout_lines()?;
        let stderr_lines = self.stderr_lines()?;
        Ok(OutputHash {
            stdout: bytes::valid_text(&self.raw_stdout)?.to_owned(),
            stderr: bytes::valid_text(&self.raw_stderr)?.to_owned(),
            exit_status: self.exit_status,
            success: self.is_success(),
            stdout_lines,
            stderr_lines,
        })
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
