#![cfg(not(target_arch = "wasm32"))]
use serde_json::{json, Value};

fn call(request: Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

#[test]
fn loader_validates_and_keeps_native_models_and_cache_modes() {
    let yaml = "name: s276_missing_profiles\nversion: '1.0'\n";
    let strict = call(json!({"op":"definition","method":"load","yaml":yaml}));
    assert_eq!(strict["ok"], false);
    assert_eq!(
        strict["error"]["source_class"],
        "Ukiryu::Errors::DefinitionValidationError"
    );
    assert_eq!(
        strict["error"]["message"],
        "Profile validation failed:\n  - Missing 'profiles' field or profiles is empty"
    );
    let unchecked =
        call(json!({"op":"definition","method":"load","yaml":yaml,"validation":"none"}));
    assert_eq!(unchecked["ok"], true, "{unchecked}");
    assert_eq!(unchecked["value"]["name"], "s276_missing_profiles");
    assert_eq!(
        call(json!({"op":"definition","method":"load","yaml":yaml})),
        unchecked
    );
    assert_eq!(
        call(json!({"op":"definition","method":"clear_cache","yaml":yaml}))["ok"],
        true
    );
    assert_eq!(
        call(json!({"op":"definition","method":"load","yaml":yaml})),
        strict
    );
}

#[test]
fn loader_preserves_native_errors_and_version_resolution() {
    let invalid = call(json!({"op":"definition","method":"load","yaml":"name: [unclosed\n"}));
    assert_eq!(
        invalid["error"]["source_class"],
        "Ukiryu::Errors::DefinitionLoadError"
    );
    let resolved = call(
        json!({"op":"definition","method":"resolve_version","constraint":">= 1.0","versions":["1.2","2.0","1.9"]}),
    );
    assert_eq!(resolved["value"], "2.0");
    assert_eq!(
        call(
            json!({"op":"definition","method":"resolve_version","constraint":"~> 1.0","versions":[]})
        )["value"],
        Value::Null
    );
}

#[test]
fn documentation_and_discovery_are_reachable_through_the_wire() {
    let docs = call(
        json!({"op":"definition","method":"documentation","value":{"sym":"unused"},"format":"invalid"}),
    );
    assert_eq!(docs["error"]["source_class"], "ArgumentError");
    let docs =
        call(json!({"op":"definition","method":"documentation","value":{},"format":"markdown"}));
    assert_eq!(docs["ok"], true, "{docs}");
    assert!(docs["value"].as_str().unwrap().starts_with("# Unknown"));
    let discovery = call(json!({"op":"definition","method":"discover"}));
    assert_eq!(discovery["ok"], true, "{discovery}");
    assert!(discovery["value"].is_object());
}

#[test]
fn file_linter_reports_a_missing_source_as_an_issue() {
    let reply = call(json!({"op":"lint","method":"file","path":"s276-missing-definition.yaml"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply
        .to_string()
        .contains("File not found: s276-missing-definition.yaml"));
}
