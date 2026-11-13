//! Public request API and error contract
//! (`RunFileCommand#validate_request!` / `#load_execution_request` and
//! `BaseCommand#stringify_keys` at `ab6df353c6dc2d7faf7817a86aafcafa82579896`).
//! The behaviour asserted here is the behaviour recorded in
//! `fixtures/request/reference.json`; these tests pin the API shape.
use ukiryu::request::{
    load_execution_request, ExecutionRequest, RequestError, ValidationError, Value,
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
        ("", "Request must be a YAML object (hash)"),
        ("# only a comment\n", "Request must be a YAML object (hash)"),
        ("--- ~\n", "Request must be a YAML object (hash)"),
        ("- a\n", "Request must be a YAML object (hash)"),
        ("text\n", "Request must be a YAML object (hash)"),
        ("{}\n", "Request must include 'tool' field"),
        ("command: b\n", "Request must include 'tool' field"),
        (
            ":tool: a\ncommand: b\narguments: {}\n",
            "Request must include 'tool' field",
        ),
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
        assert_eq!(error.source_class(), Some("RuntimeError"));
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn missing_nil_and_false_are_rejected_but_other_values_are_present() {
    for field in ["tool", "command", "arguments"] {
        let expected = format!("Request must include '{field}' field");
        // Psych YAML 1.1: `no` and `off` are false, `~` and `null` are nil.
        for value in [
            None,
            Some("null"),
            Some("~"),
            Some(""),
            Some("false"),
            Some("no"),
            Some("off"),
        ] {
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
    // Ruby truthiness: empty strings, zero, collections, Symbols and the
    // quoted string 'false' are present values.
    for tool in ["''", "0", "[a]", "{a: 1}", "'false'", ":sym", "yes"] {
        let yaml = format!("tool: {tool}\ncommand: b\narguments: {{}}\n");
        assert!(ExecutionRequest::from_yaml(&yaml).is_ok(), "{yaml:?}");
    }
    // A present but non-Hash arguments value reports the type error.
    for arguments in ["''", "0", "text", "[]", "yes"] {
        let yaml = format!("tool: a\ncommand: b\narguments: {arguments}\n");
        assert_eq!(invalid(&yaml), ValidationError::ArgumentsNotMapping);
    }
}

#[test]
fn psych_scalar_resolution_keeps_ruby_types() {
    let request = ExecutionRequest::from_yaml(
        "tool: :magick\ncommand: yes\narguments:\n  quoted: 'yes'\n  flag: on\n  n: 42\n  f: 1.5\n  none: ~\n",
    )
    .unwrap();
    assert_eq!(request.tool(), &Value::sym("magick"));
    assert_eq!(request.tool_name(), None);
    assert_eq!(request.command(), &Value::Bool(true));
    let args = request.arguments();
    assert_eq!(args.get("quoted"), Some(&Value::str("yes")));
    assert_eq!(args.get("flag"), Some(&Value::Bool(true)));
    assert_eq!(args.get("n"), Some(&Value::int(42)));
    assert_eq!(args.get("f"), Some(&Value::Float(1.5)));
    assert_eq!(args.get("none"), Some(&Value::Nil));
}

#[test]
fn file_errors_are_thor_errors_with_ruby_prefixes() {
    let missing = load_execution_request("does/not/exist.yaml").unwrap_err();
    assert_eq!(missing.source_class(), Some("Thor::Error"));
    assert_eq!(
        missing.to_string(),
        "Request file not found: does/not/exist.yaml"
    );

    let syntax = ExecutionRequest::from_yaml("tool: [unclosed\n").unwrap_err();
    assert_eq!(syntax.source_class(), Some("Thor::Error"));
    assert!(
        syntax
            .to_string()
            .starts_with("Invalid YAML in request file: "),
        "{syntax}"
    );

    let directory = load_execution_request(env!("CARGO_MANIFEST_DIR")).unwrap_err();
    assert_eq!(directory.source_class(), Some("Thor::Error"));
    assert!(
        directory
            .to_string()
            .starts_with("Error loading request file: Is a directory @ "),
        "{directory}"
    );

    let class = ExecutionRequest::from_yaml("tool: a\ncommand: b\narguments: {d: 2024-01-02}\n")
        .unwrap_err();
    assert_eq!(class.source_class(), Some("Thor::Error"));
    assert_eq!(
        class.to_string(),
        "Error loading request file: Tried to load unspecified class: Date"
    );
}

#[test]
fn nested_argument_types_and_extra_fields_round_trip() {
    let yaml = "timeout: 5\ntool: a\ncommand: b\narguments:\n  list: [1, two, 3.5, null, false, [n], {k: v}]\n  map: {inner: {deep: [x]}, flag: false, none: ~}\n  empty: {}\n";
    let request = ExecutionRequest::from_yaml(yaml).unwrap();
    assert_eq!(request.tool_name(), Some("a"));
    assert_eq!(request.command_name(), Some("b"));
    assert_eq!(request.arguments().len(), 3);
    assert!(matches!(
        request.arguments().get("map"),
        Some(Value::Hash(_))
    ));
    assert_eq!(
        request.other_fields(),
        vec![(&Value::str("timeout"), &Value::int(5))]
    );
    // The loaded Hash keeps the document's key order.
    let Value::Hash(pairs) = request.to_value() else {
        panic!("Hash expected");
    };
    let keys: Vec<Value> = pairs.into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        ["timeout", "tool", "command", "arguments"].map(Value::str)
    );

    let tree = yeptris::to_value(&request).unwrap();
    let text = yeptris::to_string(&tree).unwrap();
    assert_eq!(ExecutionRequest::from_yaml(&text).unwrap(), request);
}

#[test]
fn stringify_keys_symbolizes_top_level_keys_only() {
    let request = ExecutionRequest::from_yaml(
        "tool: a\ncommand: b\narguments:\n  output: x\n  nested: {inner: 1}\n  :output: y\n",
    )
    .unwrap();
    let Value::Hash(keys) = request.arguments().stringify_keys().unwrap() else {
        panic!("Hash expected");
    };
    // `"output"` and `:output` collapse to one Symbol: first position, last value.
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0], (Value::sym("output"), Value::str("y")));
    assert_eq!(keys[1].0, Value::sym("nested"));
    let Value::Hash(nested) = &keys[1].1 else {
        panic!("nested Hash expected");
    };
    assert_eq!(nested[0].0, Value::str("inner"));

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
        assert_eq!(error.ruby_class(), "NoMethodError");
        assert_eq!(
            error.message,
            format!("undefined method 'to_sym' for {receiver}")
        );
    }
}
