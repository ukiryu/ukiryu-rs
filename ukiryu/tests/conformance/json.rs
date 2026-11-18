//! Order-preserving JSON for corpus rows.
//!
//! `serde_json::Value` (without `preserve_order`) sorts object keys, but row
//! inputs such as `build.args` parameters are Ruby hashes whose insertion
//! order is part of the input. `Oj` keeps every object's keys in document
//! order and every number as its exact source text.
use std::fmt::Write as _;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

#[derive(Clone, Debug, PartialEq)]
pub enum Oj {
    Null,
    Bool(bool),
    /// Exact number text from the source.
    Num(String),
    Str(String),
    Arr(Vec<Oj>),
    /// Keys in document order.
    Obj(Vec<(String, Oj)>),
}

/// serde_json's `arbitrary_precision` feature (enabled workspace-wide)
/// hands numbers to `visit_map` as a one-entry map under this key.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

impl<'de> Deserialize<'de> for Oj {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Oj, D::Error> {
        d.deserialize_any(OjVisitor)
    }
}

struct OjVisitor;

impl<'de> Visitor<'de> for OjVisitor {
    type Value = Oj;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_unit<E>(self) -> Result<Oj, E> {
        Ok(Oj::Null)
    }
    fn visit_none<E>(self) -> Result<Oj, E> {
        Ok(Oj::Null)
    }
    fn visit_bool<E>(self, v: bool) -> Result<Oj, E> {
        Ok(Oj::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Oj, E> {
        Ok(Oj::Num(v.to_string()))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Oj, E> {
        Ok(Oj::Num(v.to_string()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Oj, E> {
        // The corpus never carries bare JSON floats (floats are `$float`).
        Err(E::custom(format!("bare JSON float {v} in corpus row")))
    }
    fn visit_str<E>(self, v: &str) -> Result<Oj, E> {
        Ok(Oj::Str(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Oj, E> {
        Ok(Oj::Str(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Oj, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element::<Oj>()? {
            out.push(v);
        }
        Ok(Oj::Arr(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Oj, A::Error> {
        let mut out: Vec<(String, Oj)> = Vec::new();
        while let Some(k) = map.next_key::<String>()? {
            if k == NUMBER_TOKEN && out.is_empty() {
                let text: String = map.next_value()?;
                return Ok(Oj::Num(text));
            }
            let v: Oj = map.next_value()?;
            if out.iter().any(|(e, _)| *e == k) {
                return Err(serde::de::Error::custom(format!("duplicate key {k:?}")));
            }
            out.push((k, v));
        }
        Ok(Oj::Obj(out))
    }
}

pub fn parse(text: &str) -> Result<Oj, String> {
    serde_json::from_str::<Oj>(text).map_err(|e| e.to_string())
}

impl Oj {
    pub fn get(&self, key: &str) -> Option<&Oj> {
        match self {
            Oj::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Oj::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn arr(&self) -> Option<&[Oj]> {
        match self {
            Oj::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn field_str(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .and_then(Oj::str)
            .ok_or_else(|| format!("input field {key:?} must be a string"))
    }
    /// `{"$tag": payload}` when this is a tagged value.
    pub fn tag(&self) -> Option<(&str, &Oj)> {
        match self {
            Oj::Obj(pairs) if pairs.len() == 1 && pairs[0].0.starts_with('$') => {
                Some((pairs[0].0.as_str(), &pairs[0].1))
            }
            _ => None,
        }
    }
    pub fn s(s: impl Into<String>) -> Oj {
        Oj::Str(s.into())
    }
    pub fn obj(pairs: Vec<(&str, Oj)>) -> Oj {
        Oj::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn tagged(tag: &str, payload: Oj) -> Oj {
        Oj::Obj(vec![(tag.to_string(), payload)])
    }
    pub fn sym(s: impl Into<String>) -> Oj {
        Oj::tagged("$sym", Oj::Str(s.into()))
    }
}

/// Corpus comparison (`compare: structural`): objects compare independent
/// of key order, arrays (and `$map` pair lists) in order, strings byte for
/// byte, numbers by exact integer text, and type always matters.
pub fn structural_eq(a: &Oj, b: &Oj) -> bool {
    match (a, b) {
        (Oj::Null, Oj::Null) => true,
        (Oj::Bool(x), Oj::Bool(y)) => x == y,
        (Oj::Num(x), Oj::Num(y)) => x == y,
        (Oj::Str(x), Oj::Str(y)) => x.as_bytes() == y.as_bytes(),
        (Oj::Arr(x), Oj::Arr(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| structural_eq(p, q))
        }
        (Oj::Obj(x), Oj::Obj(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, v)| {
                    y.iter()
                        .find(|(k2, _)| k2 == k)
                        .is_some_and(|(_, v2)| structural_eq(v, v2))
                })
        }
        _ => false,
    }
}

/// Deterministic JSON text with object keys sorted (for reports only).
pub fn canonical(v: &Oj) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Oj, out: &mut String) {
    match v {
        Oj::Null => out.push_str("null"),
        Oj::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Oj::Num(n) => out.push_str(n),
        Oj::Str(s) => out.push_str(&serde_json::to_string(s).unwrap()),
        Oj::Arr(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        Oj::Obj(pairs) => {
            let mut sorted: Vec<&(String, Oj)> = pairs.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push('{');
            for (i, (k, x)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}:", serde_json::to_string(k).unwrap());
                write_canonical(x, out);
            }
            out.push('}');
        }
    }
}

/// Truncate report text without splitting a UTF-8 character.
pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
