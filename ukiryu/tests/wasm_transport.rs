#![cfg(feature = "wasm")]

#[test]
fn portable_transport_uses_the_existing_wire_implementation() {
    for request in [
        r#"{"op":"version"}"#,
        r#"{"op":"resolve_config","environment":{"UKIRYU_FORMAT":"json"}}"#,
        r#"{"op":"parse_yaml","yaml":"value: [0.0, 1.0]"}"#,
        r#"{"op":"parse_yaml","yaml":".nan"}"#,
        r#"{"op":"parse_yaml","yaml":"[broken"}"#,
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
    for op in ["execute", "register", "load_file", "spawn"] {
        let request = serde_json::json!({"op":op}).to_string();
        let result: serde_json::Value =
            serde_json::from_str(&ukiryu::wasm::process(&request)).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "UNSUPPORTED_CAPABILITY");
    }
}
