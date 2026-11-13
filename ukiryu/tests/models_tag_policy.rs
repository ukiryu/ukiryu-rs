//! Per-call `safe_load` policies on YAML tags (`ukiryu::models::safe_load`).
//!
//! `models_data/tag_policy.json` was captured by running the pinned Ruby
//! (ukiryu/ukiryu ab6df353, Ruby 4.0.1, Psych 5.3.1) for three source call
//! shapes: the register's `YAML.safe_load(text, permitted_classes: [Symbol],
//! aliases: true)` with and without `Utils.symbolize_keys`, and the models'
//! `Psych.safe_load_file(path, permitted_classes: [Symbol, String, Integer,
//! Array, Hash, TrueClass, FalseClass])`. Every row records the typed Ruby
//! result (class, String encoding and bytes, Float bits, Hash class and
//! order) or the raised exception class and message.
//!
//! Each (row, policy) must match exactly, or be listed in [`UNSUPPORTED`]
//! (the port raises `Ukiryu::Models::UnsupportedValue`) or [`UNTAGGED`].
//! Two documented lowerings apply, both values Ruby's own `==` treats as
//! equal: an ASCII-only `!!binary` result (ASCII-8BIT) is a UTF-8 String,
//! and a sequence `!!omap` (`Psych::Omap`) is a Hash with the same pairs.
#[path = "models_data/support.rs"]
mod support;

use serde_json::Value as Json;
use support::golden;
use ukiryu::models::safe_load::{safe_load, LoadPolicy};
use ukiryu::models::{register_hash, ModelError, ModelErrorKind, Value};

const POLICIES: [&str; 3] = ["register", "register_raw", "model_file"];

/// (row, policy) pairs the port refuses instead of guessing, with the reason.
const UNSUPPORTED: &[(&str, &str, &str)] = &[
    // Non-ASCII ASCII-8BIT bytes have no Value representation.
    ("binary-ff", "*", "non-ASCII binary"),
    ("binary-utf8-bytes", "*", "non-ASCII binary"),
    ("binary-number", "*", "non-ASCII binary"),
    ("binary-key-nonascii", "*", "non-ASCII binary"),
    // Psych raises NoMethodError from inside its visitor.
    ("omap-empty-map-item", "*", "visit_NilClass"),
    // A permitted class revives the Class object String.
    ("ruby-class", "model_file", "Class object"),
    // A self-referencing Array.
    ("alias-recursive", "register", "recursive alias"),
    ("alias-recursive", "register_raw", "recursive alias"),
];

/// Untagged plain scalars keep the Yeptris (YAML 1.2) resolution the
/// framework reader uses; Psych's YAML 1.1 reading differs for these rows.
/// The port's value is asserted instead.
const UNTAGGED: &[(&str, &str)] = &[("untagged-colon", "\":abc\"")];

fn listed_unsupported(id: &str, policy: &str) -> bool {
    UNSUPPORTED
        .iter()
        .any(|(i, p, _)| *i == id && (*p == "*" || *p == policy))
}

fn load(policy: &str, yaml: &str) -> Result<Value, ModelError> {
    match policy {
        "register" => Ok(register_hash(yaml)?
            .and_then(|h| h.get_sym("v").cloned())
            .unwrap_or(Value::Nil)),
        "register_raw" => Ok(safe_load(yaml, LoadPolicy::REGISTER)?
            .get_str("v")
            .cloned()
            .unwrap_or(Value::Nil)),
        "model_file" => Ok(safe_load(yaml, LoadPolicy::MODEL_FILE)?
            .get_str("v")
            .cloned()
            .unwrap_or(Value::Nil)),
        other => panic!("unknown policy {other}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the port's value is the captured typed Ruby value.
fn matches(want: &Json, got: &Value) -> bool {
    let t = want["t"].as_str().unwrap();
    match (t, got) {
        ("nil", Value::Nil) => true,
        ("bool", Value::Bool(b)) => want["v"].as_bool() == Some(*b),
        ("int", Value::Integer(i)) => want["v"].as_str() == Some(i.to_string().as_str()),
        ("float", Value::Float(f)) => {
            want["bits"].as_str() == Some(hex(&f.to_bits().to_be_bytes()).as_str())
        }
        ("sym", Value::Symbol(s)) => want["v"].as_str() == Some(s.as_str()),
        ("str", Value::String(s)) => {
            want["enc"].as_str() == Some("UTF-8") && want["v"].as_str() == Some(s.as_str())
        }
        // ASCII-only ASCII-8BIT String, held as a UTF-8 String.
        ("bin", Value::String(s)) => {
            s.is_ascii() && want["hex"].as_str() == Some(hex(s.as_bytes()).as_str())
        }
        ("arr", Value::Array(items)) => {
            let w = want["v"].as_array().unwrap();
            want["class"] == "Array"
                && w.len() == items.len()
                && w.iter().zip(items).all(|(a, b)| matches(a, b))
        }
        // `Psych::Omap` is held as a Hash with the same ordered pairs.
        ("hash", Value::Hash(pairs)) => {
            let w = want["v"].as_array().unwrap();
            matches!(want["class"].as_str(), Some("Hash" | "Psych::Omap"))
                && w.len() == pairs.len()
                && w.iter()
                    .zip(pairs)
                    .all(|(pair, (k, v))| matches(&pair[0], k) && matches(&pair[1], v))
        }
        _ => false,
    }
}

#[test]
fn safe_load_policies_match_psych() {
    let doc = golden("tag_policy.json");
    assert_eq!(doc["source"]["psych"], "5.3.1");
    let rows = doc["rows"].as_array().unwrap();
    let mut failures = Vec::new();
    let mut counts = [0usize; 3]; // exact, unsupported, untagged
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let yaml = row["yaml"].as_str().unwrap();
        for policy in POLICIES {
            let want = &row["expect"][policy];
            let got = load(policy, yaml);
            if listed_unsupported(id, policy) {
                counts[1] += 1;
                if !matches!(&got, Err(e) if e.kind == ModelErrorKind::Unsupported) {
                    failures.push(format!("{id}/{policy}: listed unsupported, got {got:?}"));
                }
                continue;
            }
            if let Some((_, inspect)) = UNTAGGED.iter().find(|(i, _)| *i == id) {
                counts[2] += 1;
                match &got {
                    Ok(v) if v.inspect() == *inspect => {}
                    _ => failures.push(format!("{id}/{policy}: untagged row, got {got:?}")),
                }
                continue;
            }
            counts[0] += 1;
            let ok = match (&got, want.get("error")) {
                (Ok(v), None) => matches(&want["value"], v),
                (Err(e), Some(class)) => {
                    class.as_str() == Some(e.ruby_class())
                        && want["message"].as_str() == Some(e.message.as_str())
                }
                _ => false,
            };
            if !ok {
                failures.push(format!("{id}/{policy}: want {want}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(counts.iter().sum::<usize>(), rows.len() * POLICIES.len());
    // exact, refused, untagged (row, policy) cases.
    assert_eq!(counts, [447, 18, 3]);
}

/// Every listed refusal names a captured row.
#[test]
fn unsupported_list_names_captured_rows() {
    let doc = golden("tag_policy.json");
    let ids: Vec<&str> = doc["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    for (id, _, _) in UNSUPPORTED {
        assert!(ids.contains(id), "{id}");
    }
    for (id, _) in UNTAGGED {
        assert!(ids.contains(id), "{id}");
    }
}

/// A tag never survives as a one-entry Hash keyed by the tag.
#[test]
fn tags_are_never_lowered_to_one_entry_maps() {
    fn walk(v: &Value) {
        match v {
            Value::Hash(pairs) => {
                if pairs.len() == 1 {
                    if let Value::String(k) = &pairs[0].0 {
                        assert!(!k.starts_with('!') && !k.starts_with("tag:"), "{v:?}");
                    }
                }
                for (k, x) in pairs {
                    walk(k);
                    walk(x);
                }
            }
            Value::Array(items) => items.iter().for_each(walk),
            _ => {}
        }
    }
    let doc = golden("tag_policy.json");
    for row in doc["rows"].as_array().unwrap() {
        let yaml = row["yaml"].as_str().unwrap();
        for policy in [LoadPolicy::REGISTER, LoadPolicy::MODEL_FILE] {
            if let Ok(v) = safe_load(yaml, policy) {
                walk(&v);
            }
        }
    }
}

/// `Symbol`-free policies refuse `!ruby/symbol` and `:sym` scalars, and the
/// policy constants mirror their source calls.
#[test]
fn policy_constants_and_symbol_permission() {
    assert_eq!(LoadPolicy::REGISTER.permitted_classes, ["Symbol"]);
    assert!(LoadPolicy::REGISTER.aliases);
    assert!(!LoadPolicy::MODEL_FILE.aliases);
    assert!(!LoadPolicy::SYMBOL.aliases);
    let none = LoadPolicy {
        permitted_classes: &[],
        aliases: false,
    };
    for yaml in ["v: !ruby/symbol abc\n", "v: !custom :abc\n"] {
        let err = safe_load(yaml, none).unwrap_err();
        assert_eq!(err.kind, ModelErrorKind::DisallowedClass, "{yaml}");
        assert_eq!(err.message, "Tried to load unspecified class: Symbol");
    }
    assert_eq!(
        safe_load("v: !ruby/symbol abc\n", LoadPolicy::SYMBOL)
            .unwrap()
            .get_str("v"),
        Some(&Value::sym("abc"))
    );
}
