use serde_json::{json, Value};
use ukiryu::config::{self, Environment, OverrideResolver, Values};

fn reference() -> Value {
    serde_json::from_str(include_str!("../../fixtures/reference/config.json")).unwrap()
}

fn outcome(result: Result<Value, config::ConversionError>) -> Value {
    match result {
        Ok(value) => json!({"ok":value}),
        Err(error) => json!({"error":{"class":"ArgumentError","message":error.to_string()}}),
    }
}

#[test]
fn actual_ruby_conversion_values_and_errors_match() {
    let reference = reference();
    let rows = reference["conversions"].as_array().unwrap();
    assert_eq!(rows.len(), 48);
    for row in rows {
        let actual = config::convert(row["attribute"].as_str().unwrap(), row["input"].as_str());
        assert_eq!(outcome(actual), row["expect"], "{}", row["id"]);
    }
}

#[test]
fn actual_ruby_environment_rules_match() {
    let reference = reference();
    let rows = reference["environments"].as_array().unwrap();
    assert_eq!(rows.len(), 8);
    for row in rows {
        let input: Environment = serde_json::from_value(row["input"].clone()).unwrap();
        let actual =
            config::environment_values(&input).map(|values| serde_json::to_value(values).unwrap());
        assert_eq!(outcome(actual), row["expect"], "{}", row["id"]);
    }
}

#[test]
fn actual_ruby_precedence_preserves_false_null_and_absence() {
    let reference = reference();
    let rows = reference["resolvers"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    for row in rows {
        let layer = |key| -> Values {
            row["input"]
                .get(key)
                .map(|v| serde_json::from_value(v.clone()).unwrap())
                .unwrap_or_default()
        };
        let resolver = OverrideResolver {
            defaults: layer("defaults"),
            programmatic: layer("programmatic"),
            env: layer("env"),
            cli: layer("cli"),
        };
        let actual = json!({"value":resolver.resolve("x"),"source":resolver.source_for("x")});
        assert_eq!(actual, row["expect"], "{}", row["id"]);
    }
    assert_eq!(
        serde_json::to_value(config::defaults()).unwrap(),
        reference["defaults"]
    );
}

#[test]
fn wire_uses_explicit_context_and_retains_argument_error_contract() {
    let result = ukiryu::wire::process(br#"{"op":"resolve_config","environment":{"UKIRYU_DEBUG":"true","NO_COLOR":""},"programmatic":{"debug":null},"cli":{"debug":false,"unknown":"ignored"}}"#);
    assert_eq!(result["ok"], true);
    assert_eq!(result["values"]["debug"], false);
    assert_eq!(result["values"]["use_color"], false);
    assert_eq!(result["sources"]["debug"], "cli");
    assert_eq!(result["values"].as_object().unwrap().len(), 9);
    let error =
        ukiryu::wire::process(br#"{"op":"resolve_config","environment":{"UKIRYU_DEBUG":"off"}}"#);
    assert_eq!(error["error"]["source_class"], "ArgumentError");
    assert_eq!(
        error["error"]["message"],
        "Invalid boolean value: off. Valid values: true, 1, yes, false, 0, no"
    );
    assert_eq!(
        ukiryu::wire::process(b"not json")["error"]["code"],
        "INVALID_REQUEST"
    );
}
