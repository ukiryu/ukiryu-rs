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
//! At the deadline the whole process tree started for the run is killed and
//! the child is reaped:
//!
//! - Unix: the child leads a new process group, and the group gets
//!   `SIGKILL`.
//! - Windows: the child starts suspended, is assigned to a new job object
//!   and only then resumed, so every process it starts belongs to the job
//!   before it can run. The deadline terminates the job. The job has no
//!   kill-on-close limit: a run that ends normally leaves descendants that
//!   no longer hold the pipes alone, as on Unix.
//!
//! Only processes created for the run are signalled. Once every pipe is
//! closed the reader threads end on their own.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
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

/// The child and the means to kill everything it started.
struct Tree {
    child: Child,
    group: tree::Group,
}

impl Tree {
    /// Kills the whole tree and reaps the child.
    fn terminate(&mut self) {
        self.group.kill(&self.child);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    if let Some(dir) = &spec.cwd {
        command.current_dir(dir);
    }
    tree::prepare(&mut command);

    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let group = match tree::Group::adopt(&child) {
        Ok(group) => group,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError::Spawn(err));
        }
    };
    let mut tree = Tree { child, group };

    let stdin_pipe = tree.child.stdin.take();
    // Absent stdin is an empty, closed pipe (as `Open3.capture3` without
    // `stdin_data`), never the caller's own stdin.
    let writer = match (stdin_pipe, spec.stdin) {
        (Some(pipe), Some(input)) => Some(spawn_writer(pipe, input)),
        _ => None,
    };
    let stdout = spawn_reader(tree.child.stdout.take());
    let stderr = spawn_reader(tree.child.stderr.take());

    let status = match wait_child(&mut tree.child, deadline) {
        Ok(Some(status)) => status,
        Ok(None) => return Err(timed_out(&mut tree)),
        Err(err) => {
            tree.terminate();
            return Err(RunError::Wait(err));
        }
    };

    let stdout = collect(&stdout, deadline, &mut tree)?.map_err(RunError::Read)?;
    let stderr = collect(&stderr, deadline, &mut tree)?.map_err(RunError::Read)?;
    if let Some(writer) = writer {
        collect(&writer, deadline, &mut tree)?.map_err(RunError::Stdin)?;
    }
    Ok(Finished {
        status: ProcessStatus::from_exit_status(&status),
        stdout,
        stderr,
    })
}

/// Polls the child until it exits or the deadline passes (`Ok(None)`).
fn wait_child(child: &mut Child, deadline: Option<Instant>) -> io::Result<Option<ExitStatus>> {
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
    tree: &mut Tree,
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
        Err(RecvTimeoutError::Timeout) => Err(timed_out(tree)),
        Err(RecvTimeoutError::Disconnected) => {
            Ok(Err(io::Error::other("pipe thread ended without a result")))
        }
    }
}

fn timed_out(tree: &mut Tree) -> RunError {
    tree.terminate();
    RunError::TimedOut
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
mod tree {
    use std::io;
    use std::os::raw::c_int;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    extern "C" {
        fn kill(pid: c_int, sig: c_int) -> c_int;
    }
    const SIGKILL: c_int = 9;

    /// The child leads a new process group.
    pub(super) fn prepare(command: &mut Command) {
        command.process_group(0);
    }

    /// The process group led by the child.
    pub(super) struct Group;

    impl Group {
        pub(super) fn adopt(_child: &Child) -> io::Result<Group> {
            Ok(Group)
        }

        pub(super) fn kill(&self, child: &Child) {
            let Ok(pid) = c_int::try_from(child.id()) else {
                return;
            };
            // SAFETY: `kill` takes plain integers and touches no memory.
            // The group id is the child's pid. At the deadline the child
            // is still unreaped, so the id names this run's group. After
            // the child has exited, a pipe still open means a member of the
            // group may hold it; POSIX does not reuse an id while its
            // process group exists, and an empty group yields ESRCH.
            unsafe {
                kill(-pid, SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
mod tree {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};

    type Handle = *mut c_void;

    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    /// Exit code given to processes the deadline terminates.
    const TIMED_OUT_EXIT_CODE: u32 = 1;

    /// `THREADENTRY32`.
    #[repr(C)]
    #[allow(dead_code)]
    struct ThreadEntry32 {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> Handle;
        fn ResumeThread(thread: Handle) -> u32;
    }

    /// The child starts suspended until it belongs to the job.
    pub(super) fn prepare(command: &mut Command) {
        command.creation_flags(CREATE_SUSPENDED);
    }

    /// A job object holding the child and everything it starts. Closing the
    /// handle does not kill the processes.
    pub(super) struct Group {
        job: Handle,
    }

    impl Group {
        /// Creates the job, assigns the suspended child to it, then resumes
        /// the child. An error leaves the child suspended for the caller to
        /// kill.
        pub(super) fn adopt(child: &Child) -> io::Result<Group> {
            // SAFETY: null attributes and name create an anonymous job with
            // default security; the returned handle is owned by `Group`.
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let group = Group { job };
            // SAFETY: both handles are open: the job above and the child's
            // process handle, which `child` owns for the duration.
            if unsafe { AssignProcessToJobObject(group.job, child.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            resume(child.id())?;
            Ok(group)
        }

        pub(super) fn kill(&self, _child: &Child) {
            // SAFETY: `job` is the open handle owned by this value; only
            // processes started for this run are in it.
            unsafe {
                TerminateJobObject(self.job, TIMED_OUT_EXIT_CODE);
            }
        }
    }

    impl Drop for Group {
        fn drop(&mut self) {
            // SAFETY: `job` is open and closed exactly once, here.
            unsafe {
                CloseHandle(self.job);
            }
        }
    }

    /// Resumes the threads of a process created suspended (it has exactly
    /// one, its primary thread).
    fn resume(process_id: u32) -> io::Result<()> {
        // SAFETY: the snapshot handle is checked and closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry = ThreadEntry32 {
            size: std::mem::size_of::<ThreadEntry32>() as u32,
            usage: 0,
            thread_id: 0,
            owner_process_id: 0,
            base_priority: 0,
            delta_priority: 0,
            flags: 0,
        };
        let mut resumed = 0;
        let mut failure = None;
        // SAFETY: `entry` is a valid THREADENTRY32 with `size` set; the
        // snapshot handle is open.
        let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while more {
            if entry.owner_process_id == process_id {
                // SAFETY: the thread handle is checked and closed here.
                unsafe {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id);
                    if thread.is_null() {
                        failure = Some(io::Error::last_os_error());
                    } else {
                        if ResumeThread(thread) == u32::MAX {
                            failure = Some(io::Error::last_os_error());
                        } else {
                            resumed += 1;
                        }
                        CloseHandle(thread);
                    }
                }
            }
            // SAFETY: as for `Thread32First`.
            more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        // SAFETY: the snapshot handle is open and closed exactly once.
        unsafe {
            CloseHandle(snapshot);
        }
        match (resumed, failure) {
            (0, Some(err)) => Err(err),
            (0, None) => Err(io::Error::other("no thread found to resume the child")),
            _ => Ok(()),
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod tree {
    use std::io;
    use std::process::{Child, Command};

    pub(super) fn prepare(_command: &mut Command) {}

    pub(super) struct Group;

    impl Group {
        pub(super) fn adopt(_child: &Child) -> io::Result<Group> {
            Ok(Group)
        }

        pub(super) fn kill(&self, _child: &Child) {}
    }
}
