//! In-process CLI capture for language bindings. The dispatcher and command
//! handlers are shared with the executable; only stdout/stderr and the host
//! snapshot are supplied by the caller. No process-global environment or
//! working directory is modified. Files and subprocesses remain real, as in
//! the executable. Relative paths are relative to the caller's cwd.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{c_char, CString};
use ukiryu::environment::Environment;
use ukiryu::runtime::{platform_from_host_os, Host, Platform};

#[derive(Debug, Default, Serialize)]
pub struct CapturedOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    argv: Vec<String>,
    platform: String,
    env: BTreeMap<String, String>,
}

/// Capture one CLI invocation. `argv` omits the program name. `platform` and
/// `env` are a complete host snapshot, never an overlay on the process ENV.
/// The reply uses the core's usual success/error envelope.
pub fn process(bytes: &[u8]) -> serde_json::Value {
    let request: Request = match serde_json::from_slice(bytes) {
        Ok(request) => request,
        Err(error) => return ukiryu::wire::failure("INVALID_REQUEST", error.to_string()),
    };
    let platform = match request.platform.as_str() {
        "macos" => Ok(Platform::Macos),
        name => platform_from_host_os(name),
    };
    let platform = match platform {
        Ok(platform) => platform,
        Err(error) => return ukiryu::wire::failure("INVALID_REQUEST", error.to_string()),
    };
    let host = Host {
        platform,
        env: Environment::from_snapshot(request.env),
    };
    let argv = std::iter::once("ukiryu".to_owned()).chain(request.argv);
    let output = crate::dispatch::capture(argv, host);
    serde_json::json!({"ok": true, "value": output})
}

/// Process a length-delimited CLI capture request. Release the reply with the
/// core's `ukiryu_free`, using the same allocator and ownership as ukiryu_json.
///
/// # Safety
/// For nonzero length, input must point to that many readable bytes for this
/// call. The returned pointer is an owned, unmodified C string, freed once.
#[no_mangle]
pub unsafe extern "C" fn ukiryu_cli_json(input: *const u8, length: usize) -> *mut c_char {
    let reply = std::panic::catch_unwind(|| {
        if length > isize::MAX as usize || input.is_null() && length != 0 {
            return ukiryu::wire::failure("INVALID_REQUEST", "invalid request byte span");
        }
        let bytes = if length == 0 {
            &[]
        } else {
            // SAFETY: caller guarantees the documented readable byte span.
            unsafe { std::slice::from_raw_parts(input, length) }
        };
        process(bytes)
    })
    .unwrap_or_else(|_| ukiryu::wire::failure("INTERNAL_ERROR", "internal CLI panic"));
    CString::new(reply.to_string())
        .expect("serialized JSON contains no NUL")
        .into_raw()
}
