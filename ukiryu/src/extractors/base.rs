//! Real-process host for the extractors: port of
//! `Ukiryu::Extractors::BaseExtractor#execute_command`
//! (lib/ukiryu/extractors/base_extractor.rb at ab6df353).
//!
//! The source builds `Environment.from_env`, calls `Shell.detect` and runs
//! `Executor.execute(executable, args, env:, shell:, allow_failure: true,
//! timeout:)`, returning `{stdout:, stderr:, exit_status:}` and turning
//! `Errno::ENOENT` into a 127 result. Here the environment and the shell are
//! given by the caller: both are owner decisions (D1, D2), so
//! [`ProcessHost`] does not choose them. Everything else runs through
//! [`crate::executor::execute`].
//!
//! [`ProcessHost`] implements [`CommandHost`], so it drives
//! [`super::help::extract`], [`super::help::NativeExtractor`] and
//! [`super::help::HelpParser`] against real tools, and
//! [`CommandRunner`], so version detection (whose man-page dates come from
//! [`super::man::ManPageParser`]) can run real commands too.
//!
//! # Error policy
//!
//! - A non-zero exit is a normal result (`allow_failure: true`).
//! - The process failing to start with "not found" is [`HostError::NotFound`],
//!   which the extractors turn into the 127 `Command not found` result. A
//!   missing tool run through a Unix shell is not this case: the shell itself
//!   starts and reports exit 127 with its own message, as in the source.
//! - Everything else propagates as [`HostError::Raised`] with the Ruby class
//!   the source would raise: `Ukiryu::Errors::TimeoutError` for a timeout,
//!   `RuntimeError` for a missing Unix shell, `Errno::*` for other start
//!   failures.
//!
//! The executor kills a run at its timeout; the source only raises after the
//! child finishes (see the executor's documented differences).

use std::io;

use super::help::{CommandHost, CommandOutput, HostError};
use crate::environment::Environment;
use crate::execution::Seconds;
use crate::executor::{execute, ExecuteError, ExecuteRequest, Execution};
use crate::runtime::ShellName;
use crate::version_detection::{
    CommandOutput as RunnerOutput, CommandRequest, CommandRunner, RunnerError,
};

/// Runs extractor commands as real processes through the executor, with
/// the environment and shell the caller chose.
#[derive(Clone, Debug)]
pub struct ProcessHost {
    environment: Environment,
    shell: ShellName,
}

impl ProcessHost {
    /// A host that runs every command through `shell`, with `environment`
    /// applied over the inherited process environment (an empty
    /// [`Environment`] inherits it unchanged, like `Environment.from_env`).
    pub fn new(environment: Environment, shell: ShellName) -> ProcessHost {
        ProcessHost { environment, shell }
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn shell(&self) -> ShellName {
        self.shell
    }

    fn run_with(
        &self,
        shell: ShellName,
        executable: &str,
        args: &[&str],
        timeout_secs: u64,
        allow_failure: bool,
    ) -> Result<Execution, ExecuteError> {
        let timeout =
            i64::try_from(timeout_secs).map_or(Seconds::Float(timeout_secs as f64), Seconds::Int);
        let request = ExecuteRequest::new(
            executable,
            args.iter().map(|a| (*a).to_owned()).collect(),
            self.environment.clone(),
            shell,
            timeout,
        )
        .allow_failure(allow_failure);
        execute(request)
    }
}

impl CommandHost for ProcessHost {
    fn execute(
        &mut self,
        executable: &str,
        args: &[&str],
        timeout_secs: u64,
    ) -> Result<CommandOutput, HostError> {
        match self.run_with(self.shell, executable, args, timeout_secs, true) {
            Ok(execution) => Ok(output(execution)),
            // Unreachable with allow_failure, but the run still finished.
            Err(ExecuteError::Failed { execution, .. }) => Ok(output(*execution)),
            Err(error) => Err(host_error(error)),
        }
    }
}

impl CommandRunner for ProcessHost {
    /// A request's own shell wins; without one (where the source calls
    /// `Shell.detect`) the host's configured shell is used.
    fn run(&mut self, request: &CommandRequest) -> Result<RunnerOutput, RunnerError> {
        let shell = match request.shell.as_deref() {
            Some(name) => ShellName::lookup(name)
                .ok_or_else(|| RunnerError::new(format!("Unknown shell: {name}")))?,
            None => self.shell,
        };
        let args: Vec<&str> = request.args.iter().map(String::as_str).collect();
        let execution = match self.run_with(
            shell,
            &request.executable,
            &args,
            request.timeout_secs,
            request.allow_failure,
        ) {
            Ok(execution) => execution,
            Err(ExecuteError::Failed { execution, .. }) if request.allow_failure => *execution,
            Err(error) => return Err(RunnerError::new(error.to_string())),
        };
        let out = output(execution);
        Ok(RunnerOutput {
            stdout: out.stdout,
            stderr: out.stderr,
            exit_status: out.exit_status,
        })
    }
}

fn output(execution: Execution) -> CommandOutput {
    let status = execution.result.status();
    CommandOutput {
        exit_status: i32::try_from(status).unwrap_or(if status < 0 { i32::MIN } else { i32::MAX }),
        stdout: execution.stdout,
        stderr: execution.stderr,
    }
}

/// The exception the source lets escape for each executor failure.
fn host_error(error: ExecuteError) -> HostError {
    let raised = |class: &str, message: String| HostError::Raised {
        class: class.to_owned(),
        message,
    };
    match error {
        ExecuteError::Spawn { source, .. } if source.kind() == io::ErrorKind::NotFound => {
            HostError::NotFound
        }
        ExecuteError::Spawn { program, source } => raised(
            errno_class(&source),
            format!("{} - {program}", errno_text(&source)),
        ),
        ExecuteError::TimedOut(error) => raised(error.ruby_class(), error.message().to_owned()),
        ExecuteError::ShellNotFound { message, .. } => raised("RuntimeError", message),
        ExecuteError::InvalidTimeout(_) => raised("ArgumentError", error.to_string()),
        ExecuteError::Stdin(source) | ExecuteError::Io(source) => {
            raised(errno_class(&source), errno_text(&source))
        }
        ExecuteError::Failed { error, .. } => {
            raised(error.ruby_class(), error.message().to_owned())
        }
    }
}

/// `Errno::*` class for the common start failures; `SystemCallError`
/// otherwise.
fn errno_class(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "Errno::ENOENT",
        io::ErrorKind::PermissionDenied => "Errno::EACCES",
        io::ErrorKind::BrokenPipe => "Errno::EPIPE",
        _ => "SystemCallError",
    }
}

/// The strerror part of a Ruby `Errno` message (without Rust's
/// "(os error N)" suffix).
fn errno_text(error: &io::Error) -> String {
    let text = error.to_string();
    match text.find(" (os error ") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}
