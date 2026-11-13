use serde_json::json;

fn parse(yaml: &str) -> serde_json::Value {
    ukiryu::wire::process(
        json!({"op": "parse_yaml", "yaml": yaml})
            .to_string()
            .as_bytes(),
    )
}

#[test]
fn json_diagnostics_reject_yaml_values_that_json_cannot_represent() {
    for yaml in [
        ".nan",
        ".inf",
        "-.inf",
        "nested: [.nan]",
        "1: number-key",
        "true: bool-key",
        "!custom tagged",
    ] {
        let reply = parse(yaml);
        assert_eq!(reply["ok"], false, "silently changed YAML: {yaml}: {reply}");
    }
}

#[test]
fn json_diagnostics_preserve_compatible_types_and_quoted_scalars() {
    let reply = parse("literal: '.nan'\nrange: [0.0, 1.0]\nflag: false\nitems: [null, 42, text]\n");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["value"]["literal"], ".nan");
    assert_eq!(reply["value"]["range"], json!([0.0, 1.0]));
    assert!(reply["value"]["range"][0].is_f64());
    assert_eq!(reply["value"]["flag"], false);
    assert_eq!(reply["value"]["items"], json!([null, 42, "text"]));
}

/// The JSON diagnostic profile refuses each of the seven Psych-applied tag
/// kinds (top level and nested) with a typed error instead of lowering the
/// tag to a one-entry object or dropping it.
#[test]
fn json_diagnostics_reject_each_tag_kind_with_a_typed_error() {
    for tagged in [
        "!!str 0123",
        "!!int \"010\"",
        "!!binary aGk=",
        "!!timestamp 2024-01-17 10:20:30 Z",
        "!!set {a: null}",
        "!!omap [a: 1]",
        "!ruby/symbol abc",
    ] {
        for yaml in [tagged.to_owned(), format!("v: {tagged}\n")] {
            let reply = parse(&yaml);
            assert_eq!(reply["ok"], false, "{yaml}: {reply}");
            assert_eq!(
                reply["error"]["code"], "YAML_NOT_JSON_COMPATIBLE",
                "{yaml}: {reply}"
            );
            assert_eq!(
                reply["error"]["message"], "JSON cannot preserve a YAML tag",
                "{yaml}: {reply}"
            );
        }
    }
}
