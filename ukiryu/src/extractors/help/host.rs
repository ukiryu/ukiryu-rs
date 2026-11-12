//! The process boundary of the extractors (`BaseExtractor#execute_command`).
//!
//! The source runs commands through `Ukiryu::Executor` with
//! `Environment.from_env` and `Shell.detect`. Both are owner decisions (D1,
//! D2) and the executor belongs to the root crate, so the extractors take
//! the host as an injected trait instead. Nothing in this module starts a
//! process.

/// Timeout the source passes for every extraction command, in seconds.
pub const EXTRACTION_TIMEOUT_SECS: u64 = 30;

/// One finished command, as the source's `{stdout:, stderr:, exit_status:}`
/// hash. Output is raw bytes because the source keeps whatever bytes the
/// process wrote and only fails once a string operation meets invalid UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: i32,
}

impl CommandOutput {
    /// The result the source substitutes when the executable is missing
    /// (`rescue Errno::ENOENT`).
    pub fn command_not_found() -> CommandOutput {
        CommandOutput {
            stdout: Vec::new(),
            stderr: b"Command not found".to_vec(),
            exit_status: 127,
        }
    }
}

/// A failure reported by the host instead of a finished command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The executable does not exist (`Errno::ENOENT`); the extractor turns
    /// this into [`CommandOutput::command_not_found`].
    NotFound,
    /// Any other exception from the executor. The source does not rescue it,
    /// so it propagates out of the extractor unchanged.
    Raised { class: String, message: String },
}

/// Runs one command for an extractor. The source always runs with
/// `allow_failure: true`, so a non-zero exit is a normal result.
pub trait CommandHost {
    fn execute(
        &mut self,
        executable: &str,
        args: &[&str],
        timeout_secs: u64,
    ) -> Result<CommandOutput, HostError>;
}

/// An error the source raises out of an extraction call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractError {
    class: String,
    message: String,
}

impl ExtractError {
    fn new(class: &str, message: &str) -> ExtractError {
        ExtractError {
            class: class.to_owned(),
            message: message.to_owned(),
        }
    }

    /// A regexp, `split` or leading `strip` scan met invalid UTF-8.
    pub(crate) fn invalid_byte_sequence() -> ExtractError {
        ExtractError::new("ArgumentError", "invalid byte sequence in UTF-8")
    }

    /// The trailing `strip` scan of a broken string.
    pub(crate) fn encoding_compatibility() -> ExtractError {
        ExtractError::new(
            "Encoding::CompatibilityError",
            "invalid byte sequence in UTF-8",
        )
    }

    /// `String#downcase` of a broken string.
    pub(crate) fn input_string_invalid() -> ExtractError {
        ExtractError::new("ArgumentError", "input string invalid")
    }

    /// The YAML emitter refused the definition. Psych has no equivalent
    /// failure for this plain string tree; it is not expected in practice.
    pub(crate) fn yaml_emit(message: &str) -> ExtractError {
        ExtractError::new("yeptris::Error", message)
    }

    /// The Ruby exception class name the source raises.
    pub fn class_name(&self) -> &str {
        &self.class
    }

    /// The exception message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}

impl std::error::Error for ExtractError {}

/// Source `execute_command`: run through the host, mapping a missing
/// executable to the 127 result and propagating any other host failure.
pub(crate) fn execute_command<H: CommandHost + ?Sized>(
    host: &mut H,
    executable: &str,
    args: &[&str],
) -> Result<CommandOutput, ExtractError> {
    match host.execute(executable, args, EXTRACTION_TIMEOUT_SECS) {
        Ok(output) => Ok(output),
        Err(HostError::NotFound) => Ok(CommandOutput::command_not_found()),
        Err(HostError::Raised { class, message }) => Err(ExtractError { class, message }),
    }
}

/// `stdout + stderr`.
pub(crate) fn joined(output: &CommandOutput) -> Vec<u8> {
    [output.stdout.as_slice(), output.stderr.as_slice()].concat()
}
