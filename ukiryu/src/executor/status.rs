//! `Executor.extract_status` (lib/ukiryu/executor.rb): one integer status for
//! every way a process can end.

use std::process::ExitStatus;

/// How a process ended, in the order `extract_status` asks `Process::Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// `exited?`: the exit code.
    Exited(i64),
    /// `signaled?`: the terminating signal.
    Signaled(i64),
    /// `stopped?`: the stop signal.
    Stopped(i64),
    /// None of the above.
    Unknown,
}

impl ProcessStatus {
    /// Classifies a status reported by the host.
    ///
    /// Windows reports every end as an exit code (a 32-bit value that std
    /// exposes as `i32`); there are no signals there.
    pub fn from_exit_status(status: &ExitStatus) -> ProcessStatus {
        if let Some(code) = status.code() {
            return ProcessStatus::Exited(i64::from(code));
        }
        unix_status(status)
    }
}

#[cfg(unix)]
fn unix_status(status: &ExitStatus) -> ProcessStatus {
    use std::os::unix::process::ExitStatusExt;
    if let Some(signal) = status.signal() {
        ProcessStatus::Signaled(i64::from(signal))
    } else if let Some(signal) = status.stopped_signal() {
        ProcessStatus::Stopped(i64::from(signal))
    } else {
        ProcessStatus::Unknown
    }
}

#[cfg(not(unix))]
fn unix_status(_status: &ExitStatus) -> ProcessStatus {
    ProcessStatus::Unknown
}

/// `Executor.extract_status`: the exit code, `128 + signal` for a signalled
/// or stopped process, and `1` otherwise.
pub fn extract_status(status: ProcessStatus) -> i64 {
    match status {
        ProcessStatus::Exited(code) => code,
        ProcessStatus::Signaled(signal) | ProcessStatus::Stopped(signal) => 128 + signal,
        ProcessStatus::Unknown => 1,
    }
}
