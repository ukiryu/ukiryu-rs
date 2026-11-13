//! Process execution: port of `Ukiryu::Executor.execute`
//! (lib/ukiryu/executor.rb) and the shell `execute_command` methods it
//! calls (lib/ukiryu/shell/{unix_base,tcsh,cmd,powershell}.rb) at
//! ukiryu/ukiryu ab6df353.
//!
//! # Contract
//!
//! An [`ExecuteRequest`] cannot be built without an environment, a shell and
//! a timeout (D1, issues #20 and #29). The environment's variables are
//! applied over the inherited process environment, as Ruby's `Open3` does
//! with an environment hash: an empty [`Environment`] is valid and does not
//! isolate the child. As in the source, a `DISPLAY` in the request is
//! dropped while an inherited `DISPLAY` still reaches the child.
//!
//! Start and finish times are local times as the source's `Time.now`
//! gives them: the host's UTC offset at that instant, or the offset of an
//! explicit [`Clock::System`] passed with [`ExecuteRequest::clock`].
//!
//! The command runs through the shell as the source does: Unix shells run
//! `<shell> -c <join>` with the shell found on `PATH`, cmd runs
//! `cmd /c <join>`, and PowerShell runs the merged
//! [`PowerShell::command_argv`](crate::shell::powershell::PowerShell::command_argv)
//! (or the `Get-Content` stdin variant). The result is an
//! [`ExecutionResult`]; a non-zero status raises `ExecutionError` unless
//! failure is allowed.
//!
//! # Intentional differences from the pinned source
//!
//! Each is recorded with the Ruby observation in
//! `tests/executor_data/observations.json`:
//!
//! - The timeout kills the run (issue #20). The source raises only after the
//!   child has finished on its own (a 0.01 s timeout on `sleep 10` returns
//!   after about 10 s). Here every process started for the run is killed at
//!   the deadline (a process group on Unix, a job object on Windows) and the
//!   child is reaped.
//! - `TimeoutError#timeout` carries the timeout; the source leaves it nil.
//! - A failed run keeps its result ([`ExecuteError::Failed`]); the source's
//!   `ExecutionError` has no `#result`.
//! - stdin is written while output is read, so large inputs do not block.
//! - The timeout must be a positive, finite number. The source treats `0` as
//!   "no limit" and raises `ArgumentError` for a negative value.
//! - Output is kept as bytes ([`Execution::stdout`]). The result's streams
//!   are the bytes Ruby reads from the pipe (on Windows, CR LF becomes LF),
//!   never decoded or replaced; invalid UTF-8 stays. As in the source, a
//!   failed run whose output is not valid UTF-8 raises the `strip` error
//!   ([`ExecuteError::InvalidByteSequence`]) instead of `ExecutionError`.

mod clock;
mod invocation;
mod process;
pub mod status;

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::environment::Environment;
use crate::errors::{command_timed_out_message, execution_failed_message, Error, ErrorKind};
use crate::execution::results::bytes;
use crate::execution::{
    CommandInfo, ExecutionMetadata, ExecutionResult, InvalidByteSequence, Output, Seconds,
};
use crate::execution_context::RunSettings;
use crate::runtime::{Clock, ShellName};

pub use clock::native_utc_offset;
pub use invocation::find_executable_in;
pub use status::{extract_status, ProcessStatus};

/// Data written to the child's stdin.
pub enum Stdin {
    /// A byte string (`stdin: String`).
    Bytes(Vec<u8>),
    /// A reader copied to the child until it ends (`stdin: IO`).
    Reader(Box<dyn Read + Send>),
}

impl fmt::Debug for Stdin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stdin::Bytes(bytes) => f.debug_tuple("Bytes").field(&bytes.len()).finish(),
            Stdin::Reader(_) => f.write_str("Reader"),
        }
    }
}

/// One command to run. The constructor takes every required setting.
#[derive(Debug)]
pub struct ExecuteRequest {
    executable: String,
    arguments: Vec<String>,
    environment: Environment,
    shell: ShellName,
    timeout: Seconds,
    cwd: Option<PathBuf>,
    stdin: Option<Stdin>,
    tool_name: Option<String>,
    command_name: Option<String>,
    allow_failure: bool,
    search_path: Option<OsString>,
    clock: Option<Clock>,
}

impl ExecuteRequest {
    /// `Executor.execute(executable, args, env:, shell:, timeout:)`.
    pub fn new(
        executable: impl Into<String>,
        arguments: Vec<String>,
        environment: Environment,
        shell: ShellName,
        timeout: Seconds,
    ) -> ExecuteRequest {
        ExecuteRequest {
            executable: executable.into(),
            arguments,
            environment,
            shell,
            timeout,
            cwd: None,
            stdin: None,
            tool_name: None,
            command_name: None,
            allow_failure: false,
            search_path: None,
            clock: None,
        }
    }

    /// A request from an execution context's
    /// [`run_settings`](crate::execution_context::ExecutionContext::run_settings).
    pub fn from_settings(
        executable: impl Into<String>,
        arguments: Vec<String>,
        settings: RunSettings,
    ) -> ExecuteRequest {
        let timeout = i64::try_from(settings.timeout_secs)
            .map_or(Seconds::Float(settings.timeout_secs as f64), Seconds::Int);
        ExecuteRequest::new(
            executable,
            arguments,
            settings.environment,
            settings.shell,
            timeout,
        )
    }

    /// `cwd:`: the child's working directory.
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// `stdin:` as a byte string. An empty one still selects the stdin path.
    pub fn stdin_bytes(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(Stdin::Bytes(data.into()));
        self
    }

    /// `stdin:` as a reader (an `IO`).
    pub fn stdin_reader(mut self, reader: impl Read + Send + 'static) -> Self {
        self.stdin = Some(Stdin::Reader(Box::new(reader)));
        self
    }

    /// `tool_name:`, recorded in the command info.
    pub fn tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = Some(name.into());
        self
    }

    /// `command_name:`, recorded in the command info.
    pub fn command_name(mut self, name: impl Into<String>) -> Self {
        self.command_name = Some(name.into());
        self
    }

    /// `allow_failure:`: return a non-zero status instead of failing.
    pub fn allow_failure(mut self, allow: bool) -> Self {
        self.allow_failure = allow;
        self
    }

    /// The `PATH` value searched for a Unix shell. Defaults to the current
    /// process `PATH`, which is what the source searches.
    pub fn shell_search_path(mut self, path: impl Into<OsString>) -> Self {
        self.search_path = Some(path.into());
        self
    }

    /// The clock for the result's start and finish times. Without one, or
    /// with [`Clock::Fixed`], times carry the host's UTC offset at that
    /// instant; [`Clock::System`] gives its explicit offset.
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// `shell.join(executable, *args)`: the command line shown in results
    /// and errors.
    pub fn command_line(&self) -> String {
        invocation::formatter(self.shell).join(&self.executable, &self.arguments)
    }
}

/// A finished run: the result plus the raw output bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Execution {
    pub result: ExecutionResult,
    /// stdout exactly as the child wrote it.
    pub stdout: Vec<u8>,
    /// stderr exactly as the child wrote it.
    pub stderr: Vec<u8>,
}

/// Why a run produced no successful [`Execution`].
#[derive(Debug)]
pub enum ExecuteError {
    /// The timeout is not a positive, finite number of seconds.
    InvalidTimeout(Seconds),
    /// A Unix shell is not on the search path. The message is the source's
    /// `RuntimeError` text.
    ShellNotFound { shell: ShellName, message: String },
    /// The process could not be started (the source lets `Errno` errors
    /// from `Open3` escape).
    Spawn { program: String, source: io::Error },
    /// Preparing or writing stdin failed.
    Stdin(io::Error),
    /// Waiting for the child or reading its output failed.
    Io(io::Error),
    /// `TimeoutError`, with its `timeout` set.
    TimedOut(Error),
    /// `ExecutionError` for a non-zero status, with the run it describes.
    Failed {
        error: Error,
        execution: Box<Execution>,
    },
    /// A non-zero status whose stdout or stderr is not valid UTF-8: building
    /// the `ExecutionError` message strips the streams, and `strip` raises
    /// (`ArgumentError` or `Encoding::CompatibilityError`). The source keeps
    /// no result; the run is kept here so its bytes are not lost.
    InvalidByteSequence {
        error: InvalidByteSequence,
        execution: Box<Execution>,
    },
}

impl ExecuteError {
    /// The `Ukiryu::Errors` class this error corresponds to, when it has one.
    pub fn kind(&self) -> Option<ErrorKind> {
        match self {
            ExecuteError::TimedOut(error) | ExecuteError::Failed { error, .. } => {
                Some(error.kind())
            }
            _ => None,
        }
    }
}

impl fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecuteError::InvalidTimeout(timeout) => write!(
                f,
                "timeout must be a positive, finite number of seconds (got {})",
                timeout.to_ruby_s()
            ),
            ExecuteError::ShellNotFound { message, .. } => f.write_str(message),
            ExecuteError::Spawn { program, source } => {
                write!(f, "failed to start {program}: {source}")
            }
            ExecuteError::Stdin(err) => write!(f, "failed to write stdin: {err}"),
            ExecuteError::Io(err) => write!(f, "failed to collect process output: {err}"),
            ExecuteError::TimedOut(error) | ExecuteError::Failed { error, .. } => {
                f.write_str(error.message())
            }
            ExecuteError::InvalidByteSequence { error, .. } => f.write_str(error.message()),
        }
    }
}

impl std::error::Error for ExecuteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExecuteError::Spawn { source, .. } => Some(source),
            ExecuteError::Stdin(err) | ExecuteError::Io(err) => Some(err),
            ExecuteError::TimedOut(error) | ExecuteError::Failed { error, .. } => Some(error),
            ExecuteError::InvalidByteSequence { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// `Executor.execute`: runs the request through its shell and returns the
/// result.
pub fn execute(request: ExecuteRequest) -> Result<Execution, ExecuteError> {
    let limit = timeout_duration(request.timeout)?;
    let host = invocation::Host::current();
    let search_path = match &request.search_path {
        Some(path) => Some(path.clone()),
        None => std::env::var_os("PATH"),
    };
    let pathext = std::env::var("PATHEXT").ok();
    let command = request.command_line();

    let ExecuteRequest {
        executable,
        arguments,
        environment,
        shell,
        timeout,
        cwd,
        stdin,
        tool_name,
        command_name,
        allow_failure,
        clock,
        ..
    } = request;

    // PowerShell reads stdin from a file through `Get-Content`; the child's
    // own stdin stays empty, as in the source.
    let (stdin, stdin_file) = match (shell, stdin) {
        (ShellName::PowerShell, Some(input)) => (None, Some(StdinFile::write(input)?)),
        (_, input) => (input, None),
    };

    let argv = invocation::argv(
        shell,
        &executable,
        &arguments,
        stdin_file.as_ref().map(StdinFile::path),
        host,
        search_path.as_deref(),
        pathext.as_deref(),
    )
    .map_err(|missing| ExecuteError::ShellNotFound {
        shell,
        message: missing.0,
    })?;

    let program = argv[0].clone();
    let spec = process::Spec {
        argv,
        env: invocation::env_plan(&environment, shell, host),
        cwd,
        stdin,
    };

    let started_at = clock::now(clock);
    let outcome = process::run(spec, limit);
    let finished_at = clock::now(clock);
    drop(stdin_file);

    let finished = outcome.map_err(|err| match err {
        process::RunError::Spawn(source) => ExecuteError::Spawn { program, source },
        process::RunError::TimedOut => ExecuteError::TimedOut(Error::timeout_error(
            Some(command_timed_out_message(timeout, &executable).as_str()),
            Some(timeout),
        )),
        process::RunError::Stdin(err) => ExecuteError::Stdin(err),
        process::RunError::Wait(err) | process::RunError::Read(err) => ExecuteError::Io(err),
    })?;

    let status = extract_status(finished.status);
    let stdout_text = pipe_text(&finished.stdout, host.windows);
    let stderr_text = pipe_text(&finished.stderr, host.windows);
    // `format_error` strips stdout, then stderr; either raises for invalid
    // UTF-8 before the message exists.
    let failure =
        (status != 0 && !allow_failure).then(|| -> Result<String, InvalidByteSequence> {
            let stdout = bytes::strip(&stdout_text)?;
            let stderr = bytes::strip(&stderr_text)?;
            Ok(execution_failed_message(
                &executable,
                &command,
                Some(stdout),
                Some(stderr),
                status,
            ))
        });

    let command_info = CommandInfo {
        executable,
        arguments,
        full_command: command,
        shell: Some(shell.as_str().to_owned()),
        tool_name,
        command_name,
    };
    let execution = Execution {
        result: ExecutionResult::new(
            command_info,
            Output::new(stdout_text, stderr_text, status),
            ExecutionMetadata::new(Some(started_at), Some(finished_at), Some(timeout)),
        ),
        stdout: finished.stdout,
        stderr: finished.stderr,
    };

    match failure {
        Some(Ok(message)) => Err(ExecuteError::Failed {
            error: Error::new(ErrorKind::Execution, Some(message.as_str())),
            execution: Box::new(execution),
        }),
        Some(Err(error)) => Err(ExecuteError::InvalidByteSequence {
            error,
            execution: Box::new(execution),
        }),
        None => Ok(execution),
    }
}

/// The String Ruby reads from an `Open3` pipe: the bytes, tagged UTF-8 and
/// not validated. On Windows the pipe is in text mode, so every CR LF pair
/// reads as LF (a lone CR stays). The bytes as written stay available in
/// [`Execution::stdout`] and [`Execution::stderr`].
fn pipe_text(bytes: &[u8], windows: bool) -> Vec<u8> {
    if !windows {
        return bytes.to_vec();
    }
    let mut text = Vec::with_capacity(bytes.len());
    let mut iter = bytes.iter().peekable();
    while let Some(&byte) = iter.next() {
        if byte == b'\r' && iter.peek() == Some(&&b'\n') {
            continue;
        }
        text.push(byte);
    }
    text
}

/// The run limit for a timeout value.
fn timeout_duration(timeout: Seconds) -> Result<Duration, ExecuteError> {
    let seconds = match timeout {
        Seconds::Int(i) if i > 0 => Some(Duration::from_secs(i.unsigned_abs())),
        Seconds::Float(f) if f.is_finite() && f > 0.0 => Duration::try_from_secs_f64(f).ok(),
        _ => None,
    };
    seconds
        .filter(|d| !d.is_zero())
        .ok_or(ExecuteError::InvalidTimeout(timeout))
}

/// The temporary file PowerShell reads stdin from; removed when dropped.
struct StdinFile {
    path: PathBuf,
    display: String,
}

impl StdinFile {
    fn write(input: Stdin) -> Result<StdinFile, ExecuteError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let data = match input {
            Stdin::Bytes(bytes) => bytes,
            Stdin::Reader(mut reader) => {
                let mut buffer = Vec::new();
                reader
                    .read_to_end(&mut buffer)
                    .map_err(ExecuteError::Stdin)?;
                buffer
            }
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!(
            "ukiryu_stdin{}-{}-{}",
            std::process::id(),
            unique,
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        // The source writes this file through Ruby's `Tempfile`, which on
        // Windows is in text mode: every LF is written as CR LF.
        let data = if cfg!(windows) {
            let mut text = Vec::with_capacity(data.len());
            for byte in data {
                if byte == b'\n' {
                    text.push(b'\r');
                }
                text.push(byte);
            }
            text
        } else {
            data
        };
        std::fs::write(&path, data).map_err(ExecuteError::Stdin)?;
        let display = path.to_string_lossy().into_owned();
        Ok(StdinFile { path, display })
    }

    fn path(&self) -> &str {
        &self.display
    }
}

impl Drop for StdinFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
