#![cfg(feature = "c")]
use std::ffi::CStr;
use ukiryu::ffi::{ukiryu_free, ukiryu_json};

unsafe fn request(pointer: *const u8, length: usize) -> serde_json::Value {
    let result = unsafe { ukiryu_json(pointer, length) };
    assert!(!result.is_null());
    let bytes = unsafe { CStr::from_ptr(result) }.to_bytes().to_vec();
    unsafe { ukiryu_free(result) };
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn invalid_spans_return_owned_errors_without_dereferencing() {
    let invalid = unsafe { request(std::ptr::null(), 1) };
    assert_eq!(invalid["error"]["code"], "INVALID_REQUEST");
    let too_long = unsafe { request(std::ptr::NonNull::<u8>::dangling().as_ptr(), usize::MAX) };
    assert_eq!(too_long["error"]["code"], "INVALID_REQUEST");
    let empty = unsafe { request(std::ptr::null(), 0) };
    assert_eq!(empty["error"]["code"], "INVALID_REQUEST");
    unsafe { ukiryu_free(std::ptr::null_mut()) };
}

#[test]
fn independent_replies_are_freed_and_unicode_nul_content_is_json_escaped() {
    let input =
        serde_json::json!({"op":"resolve_config","programmatic":{"output":"Ω\0value"}}).to_string();
    for _ in 0..1000 {
        let value = unsafe { request(input.as_ptr(), input.len()) };
        assert_eq!(value["values"]["output"], "Ω\0value");
    }
    let invalid_utf8 = unsafe { request([0xff].as_ptr(), 1) };
    assert_eq!(invalid_utf8["error"]["code"], "INVALID_REQUEST");
}
