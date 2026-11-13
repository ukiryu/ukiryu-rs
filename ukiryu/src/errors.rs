//! `Ukiryu::Errors`: the public error taxonomy.
//!
//! Source: lib/ukiryu/errors.rb at ukiryu/ukiryu ab6df353, plus the two
//! message formats the executor raises (`Executor.format_error` and the
//! timeout message in `Executor.execute`, lib/ukiryu/executor.rb). Every class
//! name, parent, default message and suggestion list is checked against the
//! oracle capture in `tests/execution_result_golden/errors.jsonl`.
//!
//! Observed source behaviour kept as is:
//! - `ExecutionError` suggestions mention `e.result`, but the Ruby class has no
//!   `result` reader.
//! - The executor raises `TimeoutError` with a message only, so `timeout` is
//!   nil on errors it raises; callers constructing one may still set it.
use std::fmt;

use crate::execution::Seconds;

/// One `Ukiryu::Errors` class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Error,
    UnknownShell,
    UnsupportedPlatform,
    Validation,
    ProfileNotFound,
    ProfileLoad,
    Definition,
    DefinitionNotFound,
    DefinitionLoad,
    DefinitionValidation,
    Load,
    ToolNotFound,
    ExecutableNotFound,
    Execution,
    Timeout,
    VersionDetection,
}

impl ErrorKind {
    pub const ALL: [ErrorKind; 16] = [
        ErrorKind::Error,
        ErrorKind::UnknownShell,
        ErrorKind::UnsupportedPlatform,
        ErrorKind::Validation,
        ErrorKind::ProfileNotFound,
        ErrorKind::ProfileLoad,
        ErrorKind::Definition,
        ErrorKind::DefinitionNotFound,
        ErrorKind::DefinitionLoad,
        ErrorKind::DefinitionValidation,
        ErrorKind::Load,
        ErrorKind::ToolNotFound,
        ErrorKind::ExecutableNotFound,
        ErrorKind::Execution,
        ErrorKind::Timeout,
        ErrorKind::VersionDetection,
    ];

    /// The Ruby class name.
    pub fn ruby_class(self) -> &'static str {
        match self {
            ErrorKind::Error => "Ukiryu::Errors::Error",
            ErrorKind::UnknownShell => "Ukiryu::Errors::UnknownShellError",
            ErrorKind::UnsupportedPlatform => "Ukiryu::Errors::UnsupportedPlatformError",
            ErrorKind::Validation => "Ukiryu::Errors::ValidationError",
            ErrorKind::ProfileNotFound => "Ukiryu::Errors::ProfileNotFoundError",
            ErrorKind::ProfileLoad => "Ukiryu::Errors::ProfileLoadError",
            ErrorKind::Definition => "Ukiryu::Errors::DefinitionError",
            ErrorKind::DefinitionNotFound => "Ukiryu::Errors::DefinitionNotFoundError",
            ErrorKind::DefinitionLoad => "Ukiryu::Errors::DefinitionLoadError",
            ErrorKind::DefinitionValidation => "Ukiryu::Errors::DefinitionValidationError",
            ErrorKind::Load => "Ukiryu::Errors::LoadError",
            ErrorKind::ToolNotFound => "Ukiryu::Errors::ToolNotFoundError",
            ErrorKind::ExecutableNotFound => "Ukiryu::Errors::ExecutableNotFoundError",
            ErrorKind::Execution => "Ukiryu::Errors::ExecutionError",
            ErrorKind::Timeout => "Ukiryu::Errors::TimeoutError",
            ErrorKind::VersionDetection => "Ukiryu::Errors::VersionDetectionError",
        }
    }

    /// The kind for an exact Ruby class name; anything else is `None`
    /// (an unknown name is never mapped to the base class).
    pub fn from_ruby_class(name: &str) -> Option<Self> {
        ErrorKind::ALL.into_iter().find(|k| k.ruby_class() == name)
    }

    /// The parent class inside `Ukiryu::Errors` (`None` for the base `Error`,
    /// whose Ruby superclass is `StandardError`).
    pub fn parent(self) -> Option<Self> {
        match self {
            ErrorKind::Error => None,
            ErrorKind::DefinitionNotFound
            | ErrorKind::DefinitionLoad
            | ErrorKind::DefinitionValidation => Some(ErrorKind::Definition),
            _ => Some(ErrorKind::Error),
        }
    }

    /// The class and its `Ukiryu::Errors` ancestors, nearest first.
    pub fn ancestors(self) -> Vec<Self> {
        let mut out = vec![self];
        let mut k = self;
        while let Some(p) = k.parent() {
            out.push(p);
            k = p;
        }
        out
    }

    /// `is_a?(other)`.
    pub fn is_a(self, other: ErrorKind) -> bool {
        self.ancestors().contains(&other)
    }

    /// `#suggestions`. `ruby_platform` fills the `UnsupportedPlatformError`
    /// line that Ruby builds from `RUBY_PLATFORM` (e.g. `x86_64-linux`).
    pub fn suggestions(self, ruby_platform: &str) -> Vec<String> {
        let lines: &[&str] = match self {
            ErrorKind::Error | ErrorKind::Definition | ErrorKind::Load => &[],
            ErrorKind::UnknownShell => &[
                "Supported shells: bash, zsh, fish, sh, dash, tcsh, powershell, cmd",
                "Platform groups: :unix (all Unix shells), :windows, :powershell",
                "Set explicitly: Ukiryu.configure { |c| c.default_shell = :bash }",
            ],
            ErrorKind::UnsupportedPlatform => {
                return vec![
                    "Ukiryu supports: macOS, Linux, Windows".to_owned(),
                    format!("Current platform: {ruby_platform}"),
                    "Check if running on a supported operating system".to_owned(),
                ]
            }
            ErrorKind::Validation => &[
                "Check the parameter type against the tool definition",
                "Verify value is within allowed range",
                "Ensure value is in the allowed values list",
            ],
            ErrorKind::ProfileNotFound => &[
                "Check tool definition has profile for your platform",
                "Verify tool definition has profile for your shell",
                "Try specifying platform/shell explicitly",
            ],
            ErrorKind::ProfileLoad => &[
                "Verify YAML syntax is correct",
                "Check profile structure matches schema",
                "Review error message for specific issue",
            ],
            ErrorKind::DefinitionNotFound => &[
                "Verify file path is correct",
                "Check file has .yaml extension",
                "Use absolute path if relative path fails",
            ],
            ErrorKind::DefinitionLoad => &[
                "Validate YAML syntax",
                "Check file is readable",
                "Verify file encoding is UTF-8",
            ],
            ErrorKind::DefinitionValidation => &[
                "Run 'ukiryu validate' for detailed errors",
                "Compare with schema definition",
                "Check tool definition examples",
            ],
            ErrorKind::ToolNotFound => &[
                "Check tool name spelling",
                "Verify register path is correct",
                "List available tools: Ukiryu::Register.tool_names",
            ],
            ErrorKind::ExecutableNotFound => &[
                "Install the tool (e.g., brew install imagemagick)",
                "Add executable to PATH",
                "Configure search_paths in tool definition",
            ],
            ErrorKind::Execution => &[
                "Check e.result.exit_status for exit code",
                "Check e.result.stderr for error message",
                "Verify parameters are correct",
            ],
            ErrorKind::Timeout => &[
                "Increase timeout parameter",
                "Check UKIRYU_TIMEOUT environment variable",
                "Verify tool is not hanging",
            ],
            ErrorKind::VersionDetection => &[
                "Verify tool is installed correctly",
                "Check version_detection command in tool definition",
                "Test version command manually: tool --version",
            ],
        };
        lines.iter().map(|s| (*s).to_owned()).collect()
    }
}

/// A raised `Ukiryu::Errors::*` exception: class, message and the structured
/// fields its class defines (`timeout` on `TimeoutError`).
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    timeout: Option<Seconds>,
}

impl Error {
    /// `Kind.new(message)`. A nil message reads back as the class name, as
    /// Ruby's `Exception#message` does.
    pub fn new(kind: ErrorKind, message: Option<&str>) -> Self {
        Error {
            kind,
            message: message.unwrap_or(kind.ruby_class()).to_owned(),
            timeout: None,
        }
    }

    /// `TimeoutError.new(message, timeout:)`.
    pub fn timeout_error(message: Option<&str>, timeout: Option<Seconds>) -> Self {
        Error {
            timeout,
            ..Error::new(ErrorKind::Timeout, message)
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn ruby_class(&self) -> &'static str {
        self.kind.ruby_class()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// `TimeoutError#timeout`; `None` on every other class.
    pub fn timeout(&self) -> Option<Seconds> {
        self.timeout
    }

    pub fn suggestions(&self, ruby_platform: &str) -> Vec<String> {
        self.kind.suggestions(ruby_platform)
    }

    pub fn is_a(&self, kind: ErrorKind) -> bool {
        self.kind.is_a(kind)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The `ExecutionError` message (`Executor.format_error`): stdout and stderr
/// are stripped, a missing stream is empty.
pub fn execution_failed_message(
    executable: &str,
    command: &str,
    stdout: Option<&str>,
    stderr: Option<&str>,
    status: i64,
) -> String {
    let strip = crate::types::ruby::strip;
    format!(
        "Command failed: {executable}\n\nCommand: {command}\nExit status: {status}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
        stdout.map_or("", strip),
        stderr.map_or("", strip)
    )
}

/// The `TimeoutError` message `Executor.execute` raises.
pub fn command_timed_out_message(timeout: Seconds, executable: &str) -> String {
    format!(
        "Command timed out after {} seconds: {executable}",
        timeout.to_ruby_s()
    )
}
