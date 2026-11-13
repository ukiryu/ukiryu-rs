//! YAML tags, Symbols and Time across the unified value (`types::Value`)
//! and the models' YAML load/serialize paths.
//!
//! Tag rows in `models_data/register_tags.json` were captured by running
//! the pinned Ruby (ukiryu/ukiryu ab6df353, Ruby 4.0.1, Psych 5.5.0): the
//! register's `YAML.safe_load(text, permitted_classes: [Symbol], aliases:
//! true)` followed by `Utils.symbolize_keys`. Ruby objects carry no tags, so
//! a tag is applied or dropped, never kept as a one-entry Hash. Rows whose
//! Psych result the port cannot reproduce exactly are listed in [`GAPS`]
//! and must fail as `Unsupported`; an unlisted mismatch or a listed row that
//! now passes fails the test.
#[path = "models_data/support.rs"]
mod support;

use serde_json::{json, Value as Json};
use support::*;
use ukiryu::models::*;
use ukiryu::types::value::apply_yaml_tag;
use ukiryu::types::{OptionSource, Time};

/// Tag rows the port reports as unsupported instead of guessing.
const GAPS: &[&str] = &[
    // `!!str` on a plain scalar: Psych keeps the source text; Yeptris
    // resolves the node to an Integer first and the text is gone.
    "tag-str-plain-int",
    // Psych reads a leading-zero `!!int` string as octal.
    "tag-int-octal-string",
    // Binary String, Time, Set and Omap objects are outside the value domain.
    "tag-binary",
    "tag-timestamp",
    "tag-set",
    "tag-omap",
    // `!ruby/...` object tags.
    "tag-ruby-symbol",
];

fn tag_rows() -> Vec<Json> {
    golden("register_tags.json")["rows"]
        .as_array()
        .unwrap()
        .clone()
}

/// What the register load path yields for `v`, as Ruby `inspect`, or the
/// error kind.
fn register_outcome(yaml: &str) -> Result<String, ModelErrorKind> {
    match register_hash(yaml) {
        Ok(Some(h)) => Ok(h.get_sym("v").cloned().unwrap_or(Value::Nil).inspect()),
        Ok(None) => Ok("nil".into()),
        Err(e) => Err(e.kind),
    }
}

#[test]
fn register_yaml_tags_match_psych() {
    let mut failures = Vec::new();
    for row in tag_rows() {
        let id = row["id"].as_str().unwrap();
        let yaml = row["yaml"].as_str().unwrap();
        let got = register_outcome(yaml);
        if GAPS.contains(&id) {
            if got != Err(ModelErrorKind::Unsupported) {
                failures.push(format!("{id}: listed gap, got {got:?}"));
            }
            continue;
        }
        let want = row["expect"]["inspect"].as_str();
        if want.is_none() || got.as_deref().ok() != want {
            failures.push(format!("{id}: want {:?}, got {got:?}", row["expect"]));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The serde route (`Deserialize for Value`, used by model fields loaded
/// with `Model::from_yaml`) applies tags the same way.
#[test]
fn serde_yaml_tags_match_register_route() {
    for row in tag_rows() {
        let id = row["id"].as_str().unwrap();
        let yaml = row["yaml"].as_str().unwrap();
        let via_serde = yeptris::from_str::<Value>(yaml);
        if GAPS.contains(&id) {
            let err = via_serde.expect_err(id).to_string();
            assert!(err.contains("is not reproduced"), "{id}: {err}");
            continue;
        }
        let v = via_serde.unwrap_or_else(|e| panic!("{id}: {e}"));
        let got = v.symbolize_keys().get_sym("v").cloned().unwrap();
        assert_eq!(
            Some(got.inspect().as_str()),
            row["expect"]["inspect"].as_str(),
            "{id}"
        );
        // Never a one-entry Hash keyed by the tag.
        assert!(
            !matches!(&got, Value::Hash(p) if p.len() == 1
                && p[0].0.as_str().is_some_and(|k| k.starts_with('!') || k.starts_with("tag:"))),
            "{id}: tag lowered to a Hash"
        );
    }
}

#[test]
fn apply_yaml_tag_negative_cases() {
    // Foreign global tags and core tags on the wrong kind are not guessed.
    assert!(apply_yaml_tag("tag:example.com,2000:x", Value::str("a")).is_err());
    assert!(apply_yaml_tag("tag:yaml.org,2002:int", Value::str("1_000")).is_err());
    assert!(apply_yaml_tag("tag:yaml.org,2002:bool", Value::str("yes")).is_err());
    assert!(apply_yaml_tag("tag:yaml.org,2002:seq", Value::str("a")).is_err());
    assert!(apply_yaml_tag("!ruby/object:Foo", Value::Hash(vec![])).is_err());
    // Local tags drop, whatever the payload.
    assert_eq!(
        apply_yaml_tag("!custom", Value::Array(vec![Value::int(1)])),
        Ok(Value::Array(vec![Value::int(1)]))
    );
}

/// Symbols keep their class in memory and in `inspect`; plain-data
/// serialization (JSON, YAML documents, `from_hash` input) writes the name,
/// so a Symbol does not survive a YAML round trip (Psych would write `:b`).
#[test]
fn symbol_serializes_as_its_name() {
    let h = Value::Hash(vec![(Value::sym("a"), Value::sym("b"))]);
    assert_eq!(h.inspect(), "{a: :b}");
    assert_eq!(serde_json::to_value(&h).unwrap(), json!({ "a": "b" }));
    let back = Value::from_yaml_value(&h.to_yaml_value()).unwrap();
    assert_eq!(
        back,
        Value::Hash(vec![(Value::str("a"), Value::str("b"))]),
        "documented lowering: Symbol -> String"
    );
}

/// `Time` (types-only today) serializes as its Ruby `to_s` text and comes
/// back as a String: no model attribute holds a Time.
#[test]
fn time_serializes_as_ruby_to_s() {
    let t = Value::Time(Time {
        seconds: 1_705_486_830,
        nanos: 0,
        utc_offset: 0,
        utc: true,
    });
    let text = t.to_ruby_s();
    assert_eq!(serde_json::to_value(&t).unwrap(), json!(text));
    assert_eq!(
        Value::from_yaml_value(&t.to_yaml_value()).unwrap(),
        Value::String(text)
    );
}

/// A definition model built from Ruby data answers `Ukiryu::Type`'s option
/// reads through `#[]`; a Symbol `type` is cast to its name by the `:string`
/// attribute, and keys the model lacks read as nil.
#[test]
fn definition_models_round_trip_as_option_sources() {
    let attrs = Value::Hash(vec![
        (Value::sym("name"), Value::str("q")),
        (Value::sym("type"), Value::sym("integer")),
        (
            Value::sym("range"),
            Value::Array(vec![Value::int(0), Value::int(100)]),
        ),
    ]);
    let arg = ArgumentDefinition::from_ruby(&attrs).unwrap();
    let opt = OptionDefinition::from_ruby(&attrs).unwrap();
    for src in [&arg as &dyn OptionSource, &opt as &dyn OptionSource] {
        assert_eq!(src.option("type"), Value::str("integer"));
        assert_eq!(
            src.option("range"),
            Value::Array(vec![Value::int(0), Value::int(100)])
        );
        assert_eq!(src.option("pattern"), Value::Nil);
    }
    assert_eq!(arg.option("min"), Value::Nil);
    assert_eq!(opt.option("min"), Value::Nil);
}
