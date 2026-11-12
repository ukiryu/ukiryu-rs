//! Shared helpers for the model tests. Golden data under `models_data/` was
//! captured from the pinned Ruby oracle (ukiryu ab6df353, Ruby 4.0.1,
//! lutaml-model 0.8.175); see the `producer` field of each file.
#![allow(dead_code)]

use serde_json::{json, Map, Value as Json};
use std::path::PathBuf;
use ukiryu::models::ruby::float_to_s;
use ukiryu::models::{Attr, Model, RubyValue, Text};

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/models_data")
}

pub fn golden(name: &str) -> Json {
    let text = std::fs::read_to_string(data_dir().join(name)).expect("golden file");
    serde_json::from_str(&text).expect("golden json")
}

/// The oracle's value encoding (`$sym`, `$float`, `$map`).
pub fn enc(v: &RubyValue) -> Json {
    match v {
        RubyValue::Nil => Json::Null,
        RubyValue::Bool(b) => json!(b),
        RubyValue::Int(i) => match i64::try_from(*i) {
            Ok(n) => json!(n),
            Err(_) => json!(i.to_string()),
        },
        RubyValue::Float(f) => json!({ "$float": float_to_s(*f) }),
        RubyValue::Str(s) => json!(s),
        RubyValue::Sym(s) => json!({ "$sym": s }),
        RubyValue::Array(items) => Json::Array(items.iter().map(enc).collect()),
        RubyValue::Hash(pairs) => {
            if pairs.iter().all(|(k, _)| matches!(k, RubyValue::Str(_))) {
                let mut m = Map::new();
                for (k, v) in pairs {
                    m.insert(k.to_ruby_s(), enc(v));
                }
                Json::Object(m)
            } else {
                json!({ "$map": pairs.iter().map(|(k, v)| json!([enc(k), enc(v)])).collect::<Vec<_>>() })
            }
        }
    }
}

pub fn enc_text(t: Option<&Text>) -> Json {
    t.map_or(Json::Null, |t| enc(&t.to_ruby_value()))
}

pub fn enc_model<T: Model>(m: &T) -> Json {
    json!({ "hash": enc(&m.to_hash().expect("to_hash")) })
}

/// Compares an oracle `$model` encoding by its `hash` part.
pub fn same(expected: &Json, actual: &Json) -> bool {
    match (expected, actual) {
        (Json::Object(e), Json::Object(a)) if e.contains_key("$model") => {
            Some(&e["hash"]) == a.get("hash")
        }
        (Json::Object(e), Json::Object(a)) => {
            e.len() == a.len() && e.iter().all(|(k, v)| a.get(k).is_some_and(|x| same(v, x)))
        }
        (Json::Array(e), Json::Array(a)) => {
            e.len() == a.len() && e.iter().zip(a).all(|(x, y)| same(x, y))
        }
        _ => expected == actual,
    }
}

pub fn yaml_value(text: &str) -> yeptris::Value {
    yeptris::from_str::<yeptris::Value>(text).unwrap_or_else(|e| panic!("yaml {text:?}: {e}"))
}

/// Integral floats emitted as integers (`0.0` written as `0`). Yeptris 0.6.63
/// did this; yeptris-rs#57 fixed it, so any occurrence is now a regression.
pub static INTEGRAL_FLOAT_EMISSIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn tree_equiv(e: &yeptris::Value, a: &yeptris::Value) -> bool {
    use yeptris::{Number, Value};
    match (e, a) {
        (Value::Number(Number::F64(f)), Value::Number(Number::I64(i)))
            if f.fract() == 0.0 && *f == *i as f64 =>
        {
            INTEGRAL_FLOAT_EMISSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            true
        }
        (Value::Sequence(x), Value::Sequence(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| tree_equiv(p, q))
        }
        (Value::Mapping(x), Value::Mapping(y)) => {
            x.0.len() == y.0.len()
                && x.0
                    .iter()
                    .zip(&y.0)
                    .all(|((k1, v1), (k2, v2))| k1 == k2 && tree_equiv(v1, v2))
        }
        _ => e == a,
    }
}

/// Parses both documents and compares the trees (Psych and the Yeptris
/// emitter differ in quoting style and document markers). Mapping order is
/// compared too, since lutaml writes in mapping order.
pub fn yaml_equiv(expected: &str, actual: &str) -> bool {
    tree_equiv(&yaml_value(expected), &yaml_value(actual))
}

pub fn report_integral_floats() {
    let n = INTEGRAL_FLOAT_EMISSIONS.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(n, 0, "Yeptris wrote {n} integral floats as integers");
}

pub fn opt_text(v: &Option<Text>) -> Json {
    enc_text(v.as_ref())
}
pub fn attr_text(v: &Attr<Text>) -> Json {
    enc_text(v.get())
}
pub fn attr_bool(v: &Attr<bool>) -> Json {
    v.get().map_or(Json::Null, |b| json!(b))
}
pub fn attr_int(v: &Attr<i64>) -> Json {
    v.get().map_or(Json::Null, |i| json!(i))
}
pub fn attr_float(v: &Attr<f64>) -> Json {
    v.get()
        .map_or(Json::Null, |f| json!({ "$float": float_to_s(*f) }))
}
pub fn attr_hash(v: &Attr<RubyValue>) -> Json {
    v.get().map_or(Json::Null, enc)
}
pub fn opt_hash(v: &Option<RubyValue>) -> Json {
    v.as_ref().map_or(Json::Null, enc)
}
pub fn opt_int(v: &Option<i64>) -> Json {
    v.map_or(Json::Null, |i| json!(i))
}
pub fn opt_float(v: &Option<f64>) -> Json {
    v.map_or(Json::Null, |f| json!({ "$float": float_to_s(f) }))
}
pub fn text_list(v: &Option<Vec<Text>>) -> Json {
    v.as_ref().map_or(Json::Null, |items| {
        Json::Array(items.iter().map(|t| enc(&t.to_ruby_value())).collect())
    })
}
pub fn int_list(v: &Option<Vec<Option<i64>>>) -> Json {
    v.as_ref().map_or(Json::Null, |items| {
        Json::Array(
            items
                .iter()
                .map(|i| i.map_or(Json::Null, |i| json!(i)))
                .collect(),
        )
    })
}
pub fn model<T: Model>(v: &Option<Box<T>>) -> Json {
    v.as_ref().map_or(Json::Null, |m| enc_model(&**m))
}
pub fn model_list<T: Model>(v: &Option<Vec<T>>) -> Json {
    v.as_ref().map_or(Json::Null, |items| {
        Json::Array(items.iter().map(enc_model).collect())
    })
}
