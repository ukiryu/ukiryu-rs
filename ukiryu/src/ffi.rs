//! Runtime boundaries. The C ABI (`c` feature) owns replies; callers release
//! them exactly once with ukiryu_free. The Ruby binding (`ruby` feature) lives
//! in [`ruby`] and is entered from a thin extension crate.
#[cfg(feature = "c")]
use std::ffi::{c_char, CString};

#[cfg(feature = "ruby")]
pub mod ruby;

#[cfg(feature = "c")]
#[no_mangle]
pub extern "C" fn ukiryu_abi_version() -> u32 {
    1
}

/// Process a length-delimited UTF-8 JSON request.
///
/// # Safety
/// For nonzero length, input must point to that many readable bytes for this call.
/// The returned NUL-terminated string is owned by the caller and must be passed
/// exactly once to ukiryu_free. It must not be modified or freed by another allocator.
#[cfg(feature = "c")]
#[no_mangle]
pub unsafe extern "C" fn ukiryu_json(input: *const u8, length: usize) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if length > isize::MAX as usize || input.is_null() && length != 0 {
            return crate::wire::failure("INVALID_REQUEST", "invalid request byte span");
        }
        let bytes = if length == 0 {
            &[]
        } else {
            // SAFETY: caller guarantees the documented readable span; size checked above.
            unsafe { std::slice::from_raw_parts(input, length) }
        };
        crate::wire::process(bytes)
    })
    .unwrap_or_else(|_| crate::wire::failure("INTERNAL_ERROR", "internal core panic"));
    // JSON escapes embedded NUL bytes, so valid serialized JSON is a C string.
    CString::new(result.to_string())
        .expect("serialized JSON contains no NUL")
        .into_raw()
}

/// Release a reply returned by ukiryu_json. A null pointer is accepted.
///
/// # Safety
/// A non-null pointer must be an unmodified, still-owned ukiryu_json reply.
#[cfg(feature = "c")]
#[no_mangle]
pub unsafe extern "C" fn ukiryu_free(reply: *mut c_char) {
    if !reply.is_null() {
        // SAFETY: caller guarantees that this is exactly one owned reply allocation.
        drop(unsafe { CString::from_raw(reply) });
    }
}
