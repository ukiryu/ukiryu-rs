//! Exercise the actual shared-library exports, including allocation ownership.
use libloading::{Library, Symbol};
use std::ffi::{c_char, CStr};

#[test]
fn cli_capture_export_uses_the_core_reply_ownership() {
    assert_eq!(ukiryu_ffi::ukiryu_abi_version(), 1);
    let name = format!(
        "{}ukiryu_ffi{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(name);
    unsafe {
        let library = Library::new(path).unwrap();
        let capture: Symbol<unsafe extern "C" fn(*const u8, usize) -> *mut c_char> =
            library.get(b"ukiryu_cli_json").unwrap();
        let free: Symbol<unsafe extern "C" fn(*mut c_char)> = library.get(b"ukiryu_free").unwrap();
        let request = br#"{"argv":["version"],"platform":"linux","env":{}}"#;
        let reply = capture(request.as_ptr(), request.len());
        assert!(!reply.is_null());
        let value: serde_json::Value =
            serde_json::from_slice(CStr::from_ptr(reply).to_bytes()).unwrap();
        free(reply);
        assert_eq!(
            value,
            serde_json::json!({"ok": true, "value": {
                "stdout": "\u{1b}[36mUkiryu version 0.3.1\u{1b}[0m\n", "stderr": "", "exit": 0
            }})
        );
    }
}
