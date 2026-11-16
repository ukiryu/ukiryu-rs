//! Hash keys that start with `$` in the wire value encoding.
//!
//! The oracle (`corpus/oracle-ruby/lib/codec.rb`) reads an object as a tag
//! exactly when it has one key and that key starts with `$`; with more keys
//! it is a plain Hash. Its encoder never escapes, so a Hash whose only key is
//! a tag name reads back as that tag there. These tests pin the Rust codec to
//! the oracle's reading and check that every Hash the encoder writes reads
//! back as itself, except the one form the oracle cannot read either.
use std::path::Path;

use num_bigint::BigInt;
use serde_json::{json, Value as Json};
use ukiryu::types::Value;
use ukiryu::wire::value::{encode, Typed, TAGS};

fn decode(text: &str) -> Result<Value, String> {
    serde_json::from_str::<Typed>(text)
        .map(|Typed(v)| v)
        .map_err(|e| e.to_string())
}

fn s(text: &str) -> Value {
    Value::String(text.to_owned())
}

fn hash(pairs: &[(&str, Value)]) -> Value {
    Value::Hash(pairs.iter().map(|(k, v)| (s(k), v.clone())).collect())
}

fn round_trip(value: &Value) -> Value {
    decode(&encode(value).to_string())
        .unwrap_or_else(|e| panic!("{value:?} did not read back: {e}"))
}

/// Oracle probe, Ruby 4.0.1 and json 3.1.0 at ab6df353:
/// `decode({"$sym":"x","b":1})` is `{"$sym" => "x", "b" => 1}` and
/// `decode({"$sym":"x","$float":"1.0"})` keeps both keys as Strings.
#[test]
fn objects_with_more_than_one_key_are_plain_hashes() {
    for (text, expected) in [
        (
            r#"{"$sym":"x","b":1}"#,
            hash(&[("$sym", s("x")), ("b", Value::int(1))]),
        ),
        (
            r#"{"b":1,"$sym":"x"}"#,
            hash(&[("b", Value::int(1)), ("$sym", s("x"))]),
        ),
        (
            r#"{"$sym":"x","$float":"1.0"}"#,
            hash(&[("$sym", s("x")), ("$float", s("1.0"))]),
        ),
        (
            r#"{"$schema":"s","$id":"i"}"#,
            hash(&[("$schema", s("s")), ("$id", s("i"))]),
        ),
        (
            r#"{"$a":{"$float":"1.5"},"$b":[{"$sym":"y"}]}"#,
            hash(&[
                ("$a", Value::Float(1.5)),
                ("$b", Value::Array(vec![Value::Symbol("y".to_owned())])),
            ]),
        ),
    ] {
        assert_eq!(decode(text).as_ref(), Ok(&expected), "{text}");
        assert_eq!(round_trip(&expected), expected, "{text}");
    }
    assert!(decode(r#"{"$a":1,"$a":2}"#)
        .unwrap_err()
        .contains("duplicate key"));
}

/// Oracle probe: a lone unknown `$` key raises `ArgumentError: unknown tag`.
#[test]
fn a_lone_unknown_dollar_key_is_an_unknown_tag() {
    for text in [r#"{"$foo":1}"#, r#"{"$":1}"#, r##"{"$ref":"#/x"}"##] {
        let error = decode(text).unwrap_err();
        assert!(error.contains("unknown value tag"), "{text}: {error}");
    }
}

/// The oracle writes `{"$ref" => "#/x"}` as `{"$ref":"#/x"}` (14 yaml.load
/// rows of the corpus hold such objects), so the encoder writes those bytes
/// too; reading them back is an error, as it is in the oracle.
#[test]
fn a_lone_other_dollar_key_is_written_as_the_oracle_writes_it() {
    let value = hash(&[("$ref", s("#/x"))]);
    assert_eq!(encode(&value), json!({"$ref": "#/x"}));
    assert!(decode(&encode(&value).to_string()).is_err());
}

/// A Hash whose only key is a tag name is written as `$map`, which the
/// oracle's decoder also reads as that Hash (probe:
/// `decode({"$map":[["$sym","x"]]})` is `{"$sym" => "x"}`).
#[test]
fn a_lone_tag_name_key_round_trips_through_map() {
    for tag in TAGS {
        let value = hash(&[(tag, s("x"))]);
        assert_eq!(encode(&value), json!({"$map": [[tag, "x"]]}), "{tag}");
        assert_eq!(round_trip(&value), value, "{tag}");
    }
    let nested = hash(&[("k", hash(&[("$float", s("1.5"))]))]);
    assert_eq!(encode(&nested), json!({"k": {"$map": [["$float", "1.5"]]}}));
    assert_eq!(round_trip(&nested), nested);
    // The tags themselves are unchanged.
    for value in [
        Value::Symbol("x".to_owned()),
        Value::Float(1.5),
        Value::Hash(vec![(Value::Symbol("a".to_owned()), Value::int(1))]),
        Value::Hash(Vec::new()),
        Value::Integer(BigInt::from(7)),
    ] {
        assert_eq!(round_trip(&value), value);
    }
    assert_eq!(encode(&Value::Symbol("x".to_owned())), json!({"$sym": "x"}));
}

fn objects_with_dollar_keys(value: &Json, out: &mut Vec<Json>) {
    match value {
        Json::Object(map) => {
            if map.len() > 1 && map.keys().any(|k| k.starts_with('$')) {
                out.push(value.clone());
            }
            map.values().for_each(|v| objects_with_dollar_keys(v, out));
        }
        Json::Array(items) => items.iter().for_each(|v| objects_with_dollar_keys(v, out)),
        _ => {}
    }
}

/// Whether an encoded value holds an object whose only key is a `$` name
/// that is not a tag (a JSON-schema `$ref`, say).
fn holds_lone_unknown_key(value: &Json) -> bool {
    match value {
        Json::Object(map) => {
            let lone = map.len() == 1
                && map
                    .keys()
                    .all(|k| k.starts_with('$') && !TAGS.contains(&k.as_str()));
            lone || map.values().any(holds_lone_unknown_key)
        }
        Json::Array(items) => items.iter().any(holds_lone_unknown_key),
        _ => false,
    }
}

/// Every plain object in the corpus that has a `$` key among others
/// (`$schema`, `$id`, `$self`) reads as a Hash and is written back as the
/// same object, unless it holds a lone `$ref` object somewhere inside,
/// which the oracle's decoder rejects too.
#[test]
fn corpus_objects_with_dollar_keys_read_as_the_oracle_reads_them() {
    let cases = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance/cases");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&cases).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            for line in std::fs::read_to_string(&path).unwrap().lines() {
                if !line.trim().is_empty() {
                    let row: Json = serde_json::from_str(line).unwrap();
                    objects_with_dollar_keys(&row["expect"], &mut found);
                }
            }
        }
    }
    let (mut read, mut rejected) = (0, 0);
    for object in found {
        let decoded = decode(&object.to_string());
        if holds_lone_unknown_key(&object) {
            let error = decoded.expect_err(&object.to_string());
            assert!(error.contains("unknown value tag"), "{error}");
            rejected += 1;
        } else {
            let value = decoded.unwrap_or_else(|e| panic!("{object}: {e}"));
            assert!(matches!(value, Value::Hash(_)), "{object}");
            assert_eq!(encode(&value), object);
            read += 1;
        }
    }
    assert!(read > 0 && rejected > 0, "read {read}, rejected {rejected}");
}

/// Through the wire: `Type.validate` returns a Hash as given, so a Hash
/// with `$` keys comes back in the same form it went in.
#[test]
fn dollar_key_hashes_cross_the_wire_unchanged() {
    for value in [
        json!({"$sym": "a", "x": 1}),
        json!({"$schema": "s", "$id": "i"}),
        json!({"$map": [["$sym", "a"]]}),
        json!({"$map": [["$map", []]]}),
    ] {
        let request =
            json!({"op": "validate", "method": "type", "value": value, "type": {"$sym": "hash"}});
        let reply = ukiryu::wire::process(request.to_string().as_bytes());
        assert_eq!(reply["ok"], true, "{value}: {reply}");
        assert_eq!(reply["value"], value, "{value}");
    }
    let request = json!({"op": "validate", "method": "type", "value": {"$ref": "#/x"}, "type": {"$sym": "hash"}});
    let reply = ukiryu::wire::process(request.to_string().as_bytes());
    assert_eq!(reply["error"]["code"], "INVALID_REQUEST", "{reply}");
}
