use ukiryu::schema::{safe_load, Permitted, SchemaError};

#[test]
fn unfinished_flow_sequence_diagnostics_match_for_lf_and_crlf() {
    for (yaml, line, column) in [
        ("a: [1\n", 1, 4),
        ("name: [1\n", 1, 7),
        ("name: tool\nargs: [1\n", 2, 7),
    ] {
        let expected = SchemaError::YamlSyntax {
            message: format!("(<unknown>): did not find expected ',' or ']' while parsing a flow sequence at line {line} column {column}"),
        };
        assert_eq!(safe_load(yaml, Permitted::SYMBOL), Err(expected.clone()));
        assert_eq!(
            safe_load(&yaml.replace('\n', "\r\n"), Permitted::SYMBOL),
            Err(expected),
            "CRLF input {yaml:?}"
        );
    }
}
