use serde_json::json;
use ukiryu::types::Value;
use ukiryu::wire::value::{encode, EncodeError};

#[test]
fn binary_strings_are_always_bytes_tagged() {
    // The oracle's codec writes every ASCII-8BIT String as `$bytes`, even
    // when its bytes are ASCII.
    assert_eq!(
        encode(&Value::Bytes(b"ab".to_vec())),
        Ok(json!({"$bytes": "6162"}))
    );
    assert_eq!(
        encode(&Value::Bytes(vec![0x00, 0xff])),
        Ok(json!({"$bytes": "00ff"}))
    );
    assert_eq!(
        encode(&Value::Array(vec![
            Value::Bytes(vec![0x80]),
            Value::str("x")
        ])),
        Ok(json!([{"$bytes": "80"}, "x"]))
    );
}

#[test]
fn binary_symbols_fail_as_json_generate_does() {
    // `"\xFF".b.to_sym` through JSON.generate (json 3.1.0, Ruby 4.0.1):
    // JSON::GeneratorError, `"\xFF" from ASCII-8BIT to UTF-8`.
    let error = EncodeError(r#""\xFF" from ASCII-8BIT to UTF-8"#.to_owned());
    assert_eq!(encode(&Value::BinarySymbol(vec![0xff])), Err(error.clone()));
    assert_eq!(
        encode(&Value::BinarySymbol(vec![b'a', 0xff, 0x80])),
        Err(error.clone())
    );
    assert_eq!(
        encode(&Value::Hash(vec![(
            Value::BinarySymbol(vec![0xff]),
            Value::Nil
        )])),
        Err(error)
    );
    assert_eq!(EncodeError::CLASS, "JSON::GeneratorError");
}
