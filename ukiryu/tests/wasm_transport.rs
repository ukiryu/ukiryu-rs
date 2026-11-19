#![cfg(feature = "wasm")]

#[test]
fn portable_transport_uses_the_existing_wire_implementation() {
    for request in [
        r#"{"op":"version"}"#,
        r#"{"op":"resolve_config","environment":{"UKIRYU_FORMAT":"json"}}"#,
        r#"{"op":"parse_yaml","yaml":"value: [0.0, 1.0]"}"#,
        r#"{"op":"parse_yaml","yaml":".nan"}"#,
        r#"{"op":"parse_yaml","yaml":"[broken"}"#,
        r#"{"op":"lint","method":"string","yaml":"invalid: yaml: content: [unclosed"}"#,
        r#"{"op":"yaml","method":"safe_load","yaml":":sym: 1.5\n2: [.nan, ~]"}"#,
        r#"{"op":"yaml","method":"safe_load","yaml":"a: [1"}"#,
        r#"{"op":"yaml","method":"model_to_yaml","model":"ToolDefinition","yaml":"name: demo\n"}"#,
        "not JSON",
        "{}",
    ] {
        let expected = ukiryu::wire::process(request.as_bytes());
        let actual: serde_json::Value =
            serde_json::from_str(&ukiryu::wasm::process(request)).unwrap();
        assert_eq!(actual, expected, "{request}");
    }
}

#[test]
fn host_operations_never_reach_the_shared_dispatcher() {
    for op in [
        "execute",
        "executor",
        "find_by",
        "register",
        "load_file",
        "spawn",
    ] {
        let request = serde_json::json!({"op":op}).to_string();
        let result: serde_json::Value =
            serde_json::from_str(&ukiryu::wasm::process(&request)).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "UNSUPPORTED_CAPABILITY");
    }
}

/// `yaml` `model_load` reads the file at `path`, so WebAssembly refuses it
/// although the other `yaml` methods are portable.
#[test]
fn model_load_is_native_only() {
    let request = r#"{"op":"yaml","method":"model_load","model":"Interface","path":"/etc/passwd"}"#;
    let result: serde_json::Value = serde_json::from_str(&ukiryu::wasm::process(request)).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "UNSUPPORTED_CAPABILITY");
}
