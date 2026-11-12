//! Public request API and error contract
//! (`RunFileCommand#validate_request!` / `#load_execution_request` at
//! `ab6df353c6dc2d7faf7817a86aafcafa82579896`).
use ukiryu::request::{
    load_execution_request, ArgumentValue, ExecutionRequest, RequestError, ValidationError,
};

fn invalid(yaml: &str) -> ValidationError {
    match ExecutionRequest::from_yaml(yaml) {
        Err(RequestError::Invalid(error)) => error,
        other => panic!("{yaml:?}: expected a validation error, got {other:?}"),
    }
}

#[test]
fn validation_messages_and_order_follow_ruby() {
    let cases = [
        ("--- ~\n", "Request must be a YAML object (hash)"),
        ("- a\n", "Request must be a YAML object (hash)"),
        ("text\n", "Request must be a YAML object (hash)"),
        ("{}\n", "Request must include 'tool' field"),
        ("command: b\n", "Request must include 'tool' field"),
        ("tool: a\n", "Request must include 'command' field"),
        (
            "tool: a\ncommand: b\n",
            "Request must include 'arguments' field",
        ),
        (
            "tool: a\ncommand: b\narguments: [x]\n",
            "'arguments' must be a YAML object (hash)",
        ),
    ];
    for (yaml, message) in cases {
        let error = invalid(yaml);
        assert_eq!(error.to_string(), message, "{yaml:?}");
        let error = RequestError::from(error);
        assert_eq!(error.source_class(), "RuntimeError");
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn missing_null_and_false_are_rejected_but_other_falsy_looking_values_are_not() {
    for field in ["tool", "command", "arguments"] {
        let expected = format!("Request must include '{field}' field");
        for value in [None, Some("null"), Some("~"), Some(""), Some("false")] {
            let mut yaml = String::new();
            for (name, default) in [("tool", "a"), ("command", "b"), ("arguments", "{}")] {
                match (name == field, value) {
                    (true, None) => {}
                    (true, Some(v)) => yaml.push_str(&format!("{name}: {v}\n")),
                    (false, _) => yaml.push_str(&format!("{name}: {default}\n")),
                }
            }
            assert_eq!(invalid(&yaml).to_string(), expected, "{yaml:?}");
        }
    }
    // Ruby truthiness: empty strings, zero, collections and the quoted
    // string 'false' are present values.
    for tool in ["''", "0", "[a]", "{a: 1}", "'false'"] {
        let yaml = format!("tool: {tool}\ncommand: b\narguments: {{}}\n");
        assert!(ExecutionRequest::from_yaml(&yaml).is_ok(), "{yaml:?}");
    }
    // A present but non-mapping arguments value reports the type error.
    for arguments in ["''", "0", "text", "[]"] {
        let yaml = format!("tool: a\ncommand: b\narguments: {arguments}\n");
        assert_eq!(invalid(&yaml), ValidationError::ArgumentsNotMapping);
    }
}

#[test]
fn file_errors_are_thor_errors_with_ruby_prefixes() {
    let missing = load_execution_request("does/not/exist.yaml").unwrap_err();
    assert_eq!(missing.source_class(), "Thor::Error");
    assert_eq!(
        missing.to_string(),
        "Request file not found: does/not/exist.yaml"
    );

    let syntax = ExecutionRequest::from_yaml("tool: [unclosed\n").unwrap_err();
    assert_eq!(syntax.source_class(), "Thor::Error");
    assert!(
        syntax
            .to_string()
            .starts_with("Invalid YAML in request file: "),
        "{syntax}"
    );

    let directory = load_execution_request(env!("CARGO_MANIFEST_DIR")).unwrap_err();
    assert_eq!(directory.source_class(), "Thor::Error");
    assert!(
        directory
            .to_string()
            .starts_with("Error loading request file: "),
        "{directory}"
    );
}

#[test]
fn nested_argument_types_and_extra_fields_round_trip() {
    let yaml = "tool: a\ncommand: b\narguments:\n  list: [1, two, 3.5, null, false, [n], {k: v}]\n  map: {inner: {deep: [x]}, flag: false, none: ~}\n  empty: {}\ntimeout: 5\n";
    let request = ExecutionRequest::from_yaml(yaml).unwrap();
    assert_eq!(request.tool_name(), Some("a"));
    assert_eq!(request.command_name(), Some("b"));
    assert_eq!(request.arguments().len(), 3);
    assert!(matches!(
        request.arguments().get("map"),
        Some(ArgumentValue::Mapping(_))
    ));
    assert_eq!(request.other_fields().len(), 1);

    let tree = yeptris::to_value(&request).unwrap();
    let text = yeptris::to_string(&tree).unwrap();
    assert_eq!(ExecutionRequest::from_yaml(&text).unwrap(), request);
}

#[test]
fn serde_deserialization_applies_validation() {
    let ok: ExecutionRequest =
        yeptris::from_str("tool: a\ncommand: b\narguments: {x: 1}\n").unwrap();
    assert_eq!(ok.arguments().len(), 1);
    let error = yeptris::from_str::<ExecutionRequest>("tool: a\ncommand: b\n").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Request must include 'arguments' field"),
        "{error}"
    );
}

#[test]
fn stringify_keys_symbolizes_top_level_keys_only() {
    let request = ExecutionRequest::from_yaml(
        "tool: a\ncommand: b\narguments:\n  output: x\n  nested: {inner: 1}\n",
    )
    .unwrap();
    let keys = request.arguments().stringify_keys().unwrap();
    let names: Vec<&str> = keys.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["output", "nested"]);
    let ArgumentValue::Mapping(nested) = &keys[1].1 else {
        panic!("nested mapping expected");
    };
    assert_eq!(nested.0[0].0, ArgumentValue::String("inner".into()));

    for (key, receiver) in [
        ("1", "an instance of Integer"),
        ("1.5", "an instance of Float"),
        ("true", "true"),
        ("false", "false"),
        ("~", "nil"),
        ("[a]", "an instance of Array"),
        ("{a: 1}", "an instance of Hash"),
    ] {
        let yaml = format!("tool: a\ncommand: b\narguments:\n  ? {key}\n  : v\n");
        let request = ExecutionRequest::from_yaml(&yaml).unwrap();
        let error = request.arguments().stringify_keys().unwrap_err();
        assert_eq!(error.source_class(), "NoMethodError");
        assert_eq!(
            error.to_string(),
            format!("undefined method 'to_sym' for {receiver}")
        );
    }
}
