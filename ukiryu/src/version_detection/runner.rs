//! Injected boundaries: process execution, man-page dates.
//!
//! Detection never starts processes or reads files itself. The root executor
//! and the man-page parser (K21-MAN) plug in through these traits; tests use
//! scripted implementations that record every request.

/// One `Ukiryu::Executor.execute(executable, args, shell:, timeout:, allow_failure: true)` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRequest {
    pub executable: String,
    pub args: Vec<String>,
    /// The shell the caller passed. `None` is where the source calls
    /// `Ukiryu::Shell.detect`; that policy is owner-pending (D2), so the runner
    /// resolves it, not this module.
    pub shell: Option<String>,
    /// Seconds. The source hardcodes 30 for profile detection and 5 for
    /// implementation-index probes.
    pub timeout_secs: u64,
    /// Always `true` on detection paths: a non-zero exit is a result, not an error.
    pub allow_failure: bool,
}

/// Raw captured output, as `Open3.capture3` returns it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Exit status after the executor's own normalisation.
    pub exit_status: i32,
}

impl CommandOutput {
    /// `Execution::Output#success?`
    pub fn success(&self) -> bool {
        self.exit_status == 0
    }
}

/// Any failure to run the command (spawn error, timeout, ...). The source
/// rescues these as `StandardError`, so detection treats them as "no result".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerError {
    pub message: String,
}

impl RunnerError {
    pub fn new(message: impl Into<String>) -> Self {
        RunnerError {
            message: message.into(),
        }
    }
}

/// Executes detection commands.
pub trait CommandRunner {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, RunnerError>;
}

/// `Ukiryu::ManPageParser.parse_date(path)`: an ISO date or `None`.
pub trait ManPageDates {
    fn parse_date(&mut self, path: &str) -> Option<String>;
}
