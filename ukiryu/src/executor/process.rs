//! Spawning one process with concurrent stdin feeding and stdout/stderr
//! capture, bounded by a deadline.
//!
//! Each pipe has its own thread, so a child that fills one pipe while the
//! executor is busy with another never blocks (the source writes all of
//! stdin before it reads anything, and a large input to a command that
//! echoes it blocks until the timeout). The run is complete when the child
//! has exited and both output pipes are closed, which waits for descendants
//! that inherited the pipes, as `Open3` does.
//!
//! On Unix the child leads a new process group. At the deadline the whole
//! group gets `SIGKILL`, the child is reaped and the run reports a timeout.
//! Elsewhere only the child itself is killed; pipe-reading threads that are
//! still blocked by a surviving descendant are left to finish on their own.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use super::invocation::EnvPlan;
use super::status::ProcessStatus;
use super::Stdin;

/// What to run.
pub(super) struct Spec {
    pub argv: Vec<String>,
    pub env: EnvPlan,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<Stdin>,
}

/// A run that ended before the deadline.
pub(super) struct Finished {
    pub status: ProcessStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub(super) enum RunError {
    Spawn(io::Error),
    TimedOut,
    Wait(io::Error),
    Read(io::Error),
    Stdin(io::Error),
}

pub(super) fn run(spec: Spec, timeout: Duration) -> Result<Finished, RunError> {
    let start = Instant::now();
    let deadline = start.checked_add(timeout);

    let (program, rest) = spec
        .argv
        .split_first()
        .expect("an invocation always names a program");
    let mut command = Command::new(program);
    command
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &spec.env.set {
        command.env(key, value);
    }
    for key in &spec.env.remove {
        command.env_remove(key);
    }
    if let Some(dir) = &spec.cwd {
        command.current_dir(dir);
    }
    new_process_group(&mut command);

    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let stdin_pipe = child.stdin.take();
    // Absent stdin is an empty, closed pipe (as `Open3.capture3` without
    // `stdin_data`), never the caller's own stdin.
    let writer = match (stdin_pipe, spec.stdin) {
        (Some(pipe), Some(input)) => Some(spawn_writer(pipe, input)),
        _ => None,
    };
    let stdout = spawn_reader(child.stdout.take());
    let stderr = spawn_reader(child.stderr.take());

    let status = match wait_child(&mut child, deadline) {
        Ok(Some(status)) => status,
        Ok(None) => return Err(timed_out(&mut child)),
        Err(err) => {
            terminate(&mut child);
            return Err(RunError::Wait(err));
        }
    };

    let stdout = collect(&stdout, deadline, &mut child)?.map_err(RunError::Read)?;
    let stderr = collect(&stderr, deadline, &mut child)?.map_err(RunError::Read)?;
    if let Some(writer) = writer {
        collect(&writer, deadline, &mut child)?.map_err(RunError::Stdin)?;
    }
    Ok(Finished {
        status: ProcessStatus::from_exit_status(&status),
        stdout,
        stderr,
    })
}

/// Polls the child until it exits or the deadline passes (`Ok(None)`).
fn wait_child(
    child: &mut Child,
    deadline: Option<Instant>,
) -> io::Result<Option<std::process::ExitStatus>> {
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        let mut sleep = pause;
        if let Some(deadline) = deadline {
            if now >= deadline {
                return Ok(None);
            }
            sleep = sleep.min(deadline - now);
        }
        thread::sleep(sleep);
        pause = (pause * 2).min(Duration::from_millis(25));
    }
}

/// Waits for a helper thread's value until the deadline. A deadline that
/// passes first kills what is left of the run and reports a timeout.
fn collect<T>(
    rx: &Receiver<io::Result<T>>,
    deadline: Option<Instant>,
    child: &mut Child,
) -> Result<io::Result<T>, RunError> {
    let received = match deadline {
        Some(deadline) => {
            let left = deadline.saturating_duration_since(Instant::now());
            rx.recv_timeout(left)
        }
        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
    };
    match received {
        Ok(value) => Ok(value),
        Err(RecvTimeoutError::Timeout) => Err(timed_out(child)),
        Err(RecvTimeoutError::Disconnected) => {
            Ok(Err(io::Error::other("pipe thread ended without a result")))
        }
    }
}

fn timed_out(child: &mut Child) -> RunError {
    terminate(child);
    RunError::TimedOut
}

/// Kills the child (and on Unix its process group) and reaps it.
fn terminate(child: &mut Child) {
    kill_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_reader<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let result = match pipe {
            Some(mut pipe) => pipe.read_to_end(&mut buffer).map(|_| buffer),
            None => Ok(buffer),
        };
        let _ = tx.send(result);
    });
    rx
}

fn spawn_writer(mut pipe: ChildStdin, input: Stdin) -> Receiver<io::Result<()>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let written = match input {
            Stdin::Bytes(bytes) => pipe.write_all(&bytes),
            Stdin::Reader(mut reader) => io::copy(&mut reader, &mut pipe).map(|_| ()),
        };
        drop(pipe);
        // A child that stops reading early closes the pipe; the source
        // ignores that (`rescue Errno::EPIPE`).
        let result = match written {
            Err(err) if is_broken_pipe(&err) => Ok(()),
            other => other,
        };
        let _ = tx.send(result);
    });
    rx
}

fn is_broken_pipe(err: &io::Error) -> bool {
    // ERROR_NO_DATA (232): Windows' "pipe is being closed".
    err.kind() == io::ErrorKind::BrokenPipe || (cfg!(windows) && err.raw_os_error() == Some(232))
}

#[cfg(unix)]
fn new_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn new_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn kill_group(child: &Child) {
    use std::os::raw::c_int;

    extern "C" {
        fn kill(pid: c_int, sig: c_int) -> c_int;
    }
    const SIGKILL: c_int = 9;

    let Ok(pid) = c_int::try_from(child.id()) else {
        return;
    };
    // SAFETY: `kill` takes plain integers and touches no memory. The child
    // was spawned as the leader of process group `pid`; while any member
    // is alive that id cannot be reused, and an empty group yields ESRCH.
    unsafe {
        kill(-pid, SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_child: &Child) {}
