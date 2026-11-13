//! Diagnostic (Windows only): does a native command that PowerShell runs
//! outside a pipeline write to the captured stdout under each way of
//! starting PowerShell? Prints the observations; asserts nothing about them.
#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::{Command, Stdio};
use ukiryu::environment::Environment;
use ukiryu::execution::Seconds;
use ukiryu::executor::{execute, ExecuteRequest};
use ukiryu::runtime::ShellName;

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
}

const SCRIPT: &str = "& 'cmd' /c echo hello; exit $LASTEXITCODE";

fn plain(job: bool, stdin_null: bool) -> String {
    let mut command = Command::new("powershell");
    command
        .args(["-NoLogo", "-Command", SCRIPT])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(if stdin_null {
        Stdio::null()
    } else {
        Stdio::piped()
    });
    let mut child = command.spawn().expect("spawn");
    if job {
        // SAFETY: diagnostic; the handles are valid for the call.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            AssignProcessToJobObject(job, child.as_raw_handle());
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    format!(
        "stdout={:?} stderr={:?} status={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        out.status.code()
    )
}

#[test]
fn powershell_native_output_diagnostic() {
    println!("DIAG plain piped stdin: {}", plain(false, false));
    println!("DIAG plain null stdin: {}", plain(false, true));
    println!("DIAG job after spawn: {}", plain(true, false));
    let execution = execute(
        ExecuteRequest::new(
            "cmd",
            vec!["/c".into(), "echo".into(), "hello".into()],
            Environment::new(),
            ShellName::PowerShell,
            Seconds::Int(60),
        )
        .allow_failure(true),
    )
    .expect("execute");
    println!(
        "DIAG executor: stdout={:?} stderr={:?} status={}",
        execution.result.stdout(),
        execution.result.stderr(),
        execution.result.exit_status()
    );
}
