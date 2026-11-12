//! Browser-safe transport over the same versioned wire implementation as C.
//!
//! A WASM consumer supplies its data; this module does not read profiles from
//! the host filesystem or emulate a subprocess. Node spawning belongs in its
//! explicit adapter. Portable domain operations are added by the wire owner.
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn abi_version() -> u32 {
    1
}

#[wasm_bindgen]
pub fn core_version() -> String {
    crate::VERSION.to_owned()
}

/// Process a JSON wire request, returning a JSON wire response.
///
/// Keep transport errors structured rather than throwing a JS exception. The
/// allowlist is a capability boundary: future native operations must not become
/// available to browser consumers merely by being added to the shared wire.
#[wasm_bindgen]
pub fn process(input: &str) -> String {
    let response = match serde_json::from_str::<serde_json::Value>(input) {
        Ok(request) => match request.get("op").and_then(|op| op.as_str()) {
            Some(
                "version" | "resolve_config" | "parse_yaml" | "resolve" | "build" | "validate"
                | "escape" | "schema",
            ) => crate::wire::process(input.as_bytes()),
            Some(operation) => crate::wire::failure(
                "UNSUPPORTED_CAPABILITY",
                format!("Operation {operation:?} is not available in WebAssembly"),
            ),
            None => crate::wire::process(input.as_bytes()),
        },
        Err(_) => crate::wire::process(input.as_bytes()),
    };
    serde_json::to_string(&response).unwrap_or_else(|_| {
        "{\"ok\":false,\"error\":{\"code\":\"SERIALIZATION_ERROR\",\"message\":\"Unable to encode the core response\"}}".to_owned()
    })
}
