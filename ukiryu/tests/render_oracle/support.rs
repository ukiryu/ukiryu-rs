//! Shared helpers of the render tests: the oracle responses and the
//! canonical value encoding used to compare loaded YAML across engines.
#![allow(dead_code)]

use serde::{Serialize, Serializer};
use serde_json::{json, Value as Json};
use ukiryu::render::{PaintSupport, Renderable, RequestSummary, ResponseView, SuccessView};
use yeptris::{Mapping, Number, Tagged, Value};

pub fn golden() -> Json {
    serde_json::from_str(include_str!("golden.json")).expect("golden json")
}

pub fn s(v: &Json) -> &str {
    v.as_str().expect("string")
}

/// An oracle response: the Ruby `to_yaml` document as a Yeptris tree.
pub struct Oracle {
    pub error: bool,
    pub tree: Value,
}

impl Serialize for Oracle {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.tree.serialize(ser)
    }
}

pub fn get<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for key in path {
        let Value::Mapping(m) = cur else { return None };
        cur =
            m.0.iter()
                .find(|(k, _)| matches!(k, Value::String(s) if s == key))
                .map(|(_, v)| v)?;
    }
    Some(cur)
}

fn text(v: &Value, path: &[&str]) -> String {
    match get(v, path) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => panic!("non-string at {path:?}: {other:?}"),
    }
}

fn list<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    match get(v, path) {
        Some(Value::Sequence(items)) => items,
        _ => &[],
    }
}

impl Renderable for Oracle {
    fn view(&self) -> ResponseView {
        let t = &self.tree;
        if self.error {
            return ResponseView::Error {
                error: text(t, &["error"]),
            };
        }
        let exit_code = match get(t, &["exit_code"]) {
            Some(Value::Number(Number::I64(i))) => Some(*i),
            None => None,
            other => panic!("exit_code {other:?}"),
        };
        let pair = |a: &Value| (text(a, &["name"]), text(a, &["value"]));
        ResponseView::Success(SuccessView {
            exit_code,
            formatted_duration: text(t, &["metadata", "formatted_duration"]),
            request: RequestSummary {
                options: list(t, &["request", "options"]).iter().map(pair).collect(),
                flags: list(t, &["request", "flags"])
                    .iter()
                    .map(|f| match f {
                        Value::String(s) => s.clone(),
                        other => panic!("flag {other:?}"),
                    })
                    .collect(),
                positional: list(t, &["request", "positional"])
                    .iter()
                    .map(|a| text(a, &["value"]))
                    .collect(),
            },
            full_command: text(t, &["command", "full_command"]),
            stdout: text(t, &["output", "stdout"]),
            stderr: text(t, &["output", "stderr"]),
        })
    }
}

/// `(id, response, Psych YAML, json gem JSON)` of every oracle response.
pub fn responses(g: &Json) -> Vec<(String, Oracle, String, String)> {
    g["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let yaml = s(&r["yaml"]).to_owned();
            let tree: Value = yeptris::from_str(&yaml).expect("oracle yaml loads");
            let oracle = Oracle {
                error: s(&r["class"]) == "ErrorResponse",
                tree,
            };
            (
                s(&r["id"]).to_owned(),
                oracle,
                yaml,
                s(&r["json"]).to_owned(),
            )
        })
        .collect()
}

pub fn paint(name: &str) -> PaintSupport {
    match name {
        "unavailable" => PaintSupport::UNAVAILABLE,
        "available" => PaintSupport::AVAILABLE,
        "loaded" => PaintSupport::LOADED,
        "loaded-mode0" => PaintSupport {
            enabled: false,
            ..PaintSupport::LOADED
        },
        other => panic!("paint state {other}"),
    }
}

/// The stable ID of a captured render row.
pub fn render_case_id(row: &Json) -> String {
    format!(
        "{}/{}/color={}/tty={}/paint={}/file={}",
        s(&row["response"]),
        s(&row["format"]),
        row["use_color"],
        row["tty"],
        s(&row["paint"]),
        row["output_file"]
    )
}

/// The canonical, type-sensitive encoding both engines' loads are compared
/// in: integers `{"$int": text}`, floats `{"$float": bits-hex | nan | inf |
/// -inf}`, mappings `{"$map": [[k, v], ...]}` (key types and order kept),
/// tags `{"$tag": tag, "value": v}`. A string is a JSON string, so `"no"`,
/// `false`, `"123"` and `{"$int": "123"}` all differ. The Ruby side adds
/// `{"$time": ...}`, `{"$date": ...}` and `{"$sym": ...}` for Psych types.
pub fn canon(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(Number::I64(i)) => json!({ "$int": i.to_string() }),
        Value::Number(Number::U64(u)) => json!({ "$int": u.to_string() }),
        Value::Number(Number::F64(f)) => json!({ "$float": float_key(*f) }),
        Value::String(s) => Json::String(s.clone()),
        Value::Sequence(items) => Json::Array(items.iter().map(canon).collect()),
        Value::Mapping(m) => {
            let pairs: Vec<Json> =
                m.0.iter()
                    .map(|(k, v)| json!([canon(k), canon(v)]))
                    .collect();
            json!({ "$map": pairs })
        }
        Value::Tagged(t) => json!({ "$tag": t.tag, "value": canon(&t.value) }),
    }
}

pub fn float_key(f: f64) -> String {
    if f.is_nan() {
        "nan".into()
    } else if f == f64::INFINITY {
        "inf".into()
    } else if f == f64::NEG_INFINITY {
        "-inf".into()
    } else {
        format!("{:016x}", f.to_bits())
    }
}

/// The inverse of [`canon`] for the encodings Yeptris can hold.
pub fn decode(c: &Json) -> Value {
    match c {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::String(s) => Value::String(s.clone()),
        Json::Array(items) => Value::Sequence(items.iter().map(decode).collect()),
        Json::Object(o) => {
            if let Some(t) = o.get("$int") {
                let t = s(t);
                match t.parse::<i64>() {
                    Ok(i) => Value::Number(Number::I64(i)),
                    Err(_) => Value::Number(Number::U64(t.parse().expect("u64"))),
                }
            } else if let Some(f) = o.get("$float") {
                Value::Number(Number::F64(match s(f) {
                    "nan" => f64::NAN,
                    "inf" => f64::INFINITY,
                    "-inf" => f64::NEG_INFINITY,
                    hex => f64::from_bits(u64::from_str_radix(hex, 16).expect("bits")),
                }))
            } else if let Some(pairs) = o.get("$map") {
                Value::Mapping(Mapping(
                    pairs
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| (decode(&p[0]), decode(&p[1])))
                        .collect(),
                ))
            } else if let Some(tag) = o.get("$tag") {
                Value::Tagged(Box::new(Tagged {
                    tag: s(tag).to_owned(),
                    value: decode(&o["value"]),
                }))
            } else {
                panic!("not a Yeptris-encodable canonical value: {c}")
            }
        }
        Json::Number(n) => panic!("bare JSON number {n}: use $int/$float"),
    }
}
