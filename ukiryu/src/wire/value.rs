//! The typed value encoding of the wire: Ruby values as JSON, with tags for
//! what JSON cannot carry losslessly. It is the value encoding of the
//! conformance corpus (`corpus/conformance/README.md`, "Value encoding"),
//! extended only where the corpus encoding has no form for a value.
//!
//! | Value | Encoding |
//! |---|---|
//! | nil, booleans, strings, arrays | themselves |
//! | Integer | a JSON integer (`{"$int": "decimal"}` is also accepted) |
//! | Symbol | `{"$sym": "name"}` |
//! | Float | `{"$float": "0.0"}`, exactly Ruby `Float#to_s` |
//! | Time in UTC | `{"$time": "2024-01-17T10:20:30.000000000Z"}` |
//! | any other Time | `{"$time": {"seconds": "<decimal>", "nanos": n, "utc_offset": s, "utc": b}}` |
//! | Regexp | `{"$regexp": "source"}`, or `{"$regexp": {"source": s, "options": n}}` with options |
//! | bytes that are not UTF-8 | `{"$bytes": "hex"}` |
//! | Hash with only String keys | a JSON object |
//! | any other Hash | `{"$map": [[key, value], ...]}` |
//!
//! Nothing is converted through a lossy form: a JSON number with a fraction
//! or exponent is rejected (a Float must be `$float`), a `$float` must be the
//! exact `Float#to_s` text of the value it denotes, and a Time keeps its whole
//! seconds as a decimal string and its fraction as integer nanoseconds.
//! A JSON object decodes in document order; object key order is not
//! significant in replies (the corpus compares objects structurally).
use std::fmt;

use num_bigint::BigInt;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{json, Map, Value as Json};

use crate::models::ruby::float_to_s;
use crate::types::{Regexp, Time, Value};

/// The token `serde_json` (`arbitrary_precision`) uses for a number.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// A wire value decoded into the core's Ruby value type.
#[derive(Debug, Clone, PartialEq)]
pub struct Typed(pub Value);

impl<'de> Deserialize<'de> for Typed {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Typed, D::Error> {
        deserializer.deserialize_any(TypedVisitor).map(Typed)
    }
}

struct TypedVisitor;

fn integer<E: de::Error>(text: &str) -> Result<Value, E> {
    if text.contains(['.', 'e', 'E']) {
        return Err(E::custom(format!(
            "the number {text} has a fraction or exponent; encode a Float as {{\"$float\": ...}}"
        )));
    }
    text.parse::<BigInt>()
        .map(Value::Integer)
        .map_err(|_| E::custom(format!("invalid integer {text}")))
}

impl<'de> Visitor<'de> for TypedVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a typed wire value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Integer(BigInt::from(v)))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Integer(BigInt::from(v)))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Err(E::custom(format!(
            "the number {v} is not an integer; encode a Float as {{\"$float\": ...}}"
        )))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Typed(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let Some(first) = map.next_key::<String>()? else {
            return Ok(Value::Hash(Vec::new()));
        };
        let tagged = match first.as_str() {
            NUMBER_TOKEN => {
                let text: String = map.next_value()?;
                Some(integer::<A::Error>(&text)?)
            }
            "$sym" | "$float" | "$int" | "$time" | "$regexp" | "$bytes" => {
                let inner: Json = map.next_value()?;
                Some(tag(&first, &inner).map_err(<A::Error as de::Error>::custom)?)
            }
            "$map" => {
                let pairs: Vec<(Typed, Typed)> = map.next_value()?;
                Some(Value::Hash(
                    pairs
                        .into_iter()
                        .map(|(Typed(k), Typed(v))| (k, v))
                        .collect(),
                ))
            }
            _ => None,
        };
        if let Some(value) = tagged {
            if map.next_key::<String>()?.is_some() {
                return Err(de::Error::custom(format!(
                    "{first} must be the only key of its object"
                )));
            }
            return Ok(value);
        }
        if first.starts_with('$') {
            return Err(de::Error::custom(format!(
                "unknown value tag {first}; a Hash with this String key must use $map"
            )));
        }
        let Typed(value) = map.next_value()?;
        let mut pairs = vec![(Value::String(first), value)];
        while let Some(key) = map.next_key::<String>()? {
            if pairs
                .iter()
                .any(|(k, _)| matches!(k, Value::String(s) if *s == key))
            {
                return Err(de::Error::custom(format!("duplicate key {key:?}")));
            }
            let Typed(value) = map.next_value()?;
            pairs.push((Value::String(key), value));
        }
        Ok(Value::Hash(pairs))
    }
}

/// One tagged scalar.
fn tag(name: &str, inner: &Json) -> Result<Value, String> {
    let text = || {
        inner
            .as_str()
            .ok_or_else(|| format!("{name} takes a string"))
    };
    match name {
        "$sym" => Ok(Value::Symbol(text()?.to_owned())),
        "$int" => integer::<serde_json::Error>(text()?)
            .map_err(|e| e.to_string())
            .and_then(|v| match v {
                Value::Integer(_) => Ok(v),
                _ => Err("$int takes a decimal integer".to_owned()),
            }),
        "$float" => parse_float(text()?).map(Value::Float),
        "$time" => decode_time(inner).map(Value::Time),
        "$regexp" => decode_regexp(inner).map(Value::Regexp),
        "$bytes" => {
            let bytes = decode_hex(text()?)?;
            // The core's String values are UTF-8; bytes that are not cannot
            // be held without changing them.
            String::from_utf8(bytes)
                .map(Value::String)
                .map_err(|_| "a non-UTF-8 string cannot be a parameter value here".to_owned())
        }
        other => Err(format!("unknown value tag {other}")),
    }
}

/// `$float` text: exactly what Ruby `Float#to_s` prints for the value.
fn parse_float(text: &str) -> Result<f64, String> {
    let value = match text {
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        _ => text
            .parse::<f64>()
            .map_err(|_| format!("invalid $float {text:?}"))?,
    };
    if float_to_s(value) != text {
        return Err(format!(
            "$float {text:?} is not Ruby's Float#to_s form ({:?})",
            float_to_s(value)
        ));
    }
    Ok(value)
}

pub(super) fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if text.len() % 2 != 0 {
        return Err("$bytes takes an even number of hex digits".to_owned());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| format!("invalid hex in $bytes at {i}"))
        })
        .collect()
}

fn decode_regexp(inner: &Json) -> Result<Regexp, String> {
    match inner {
        Json::String(source) => Ok(Regexp {
            source: source.clone(),
            options: 0,
        }),
        Json::Object(fields) => {
            let source = fields
                .get("source")
                .and_then(Json::as_str)
                .ok_or("$regexp.source must be a string")?;
            let options = fields
                .get("options")
                .and_then(Json::as_u64)
                .and_then(|o| u32::try_from(o).ok())
                .ok_or("$regexp.options must be an integer")?;
            if fields.len() != 2 {
                return Err("$regexp takes only source and options".to_owned());
            }
            Ok(Regexp {
                source: source.to_owned(),
                options,
            })
        }
        _ => Err("$regexp takes a string or an object".to_owned()),
    }
}

// ------------------------------------------------------------------ time

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

fn digits(text: &str, range: std::ops::Range<usize>) -> Option<i64> {
    let part = text.get(range)?;
    if part.bytes().all(|b| b.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

/// `YYYY-MM-DDTHH:MM:SS.fffffffffZ`, the corpus form of a UTC time.
fn parse_utc(text: &str) -> Option<Time> {
    let b = text.as_bytes();
    if b.len() != 30
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[29] != b'Z'
    {
        return None;
    }
    let (y, mo, d) = (
        digits(text, 0..4)?,
        digits(text, 5..7)?,
        digits(text, 8..10)?,
    );
    let (h, mi, s) = (
        digits(text, 11..13)?,
        digits(text, 14..16)?,
        digits(text, 17..19)?,
    );
    let nanos = u32::try_from(digits(text, 20..29)?).ok()?;
    if !(1..=12).contains(&mo) || d < 1 || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    if civil_from_days(days) != (y, mo, d) {
        return None;
    }
    Some(Time::new(
        days * 86_400 + h * 3600 + mi * 60 + s,
        nanos,
        0,
        true,
    ))
}

fn decode_time(inner: &Json) -> Result<Time, String> {
    match inner {
        Json::String(text) => parse_utc(text)
            .ok_or_else(|| format!("$time {text:?} is not YYYY-MM-DDTHH:MM:SS.fffffffffZ")),
        Json::Object(fields) => {
            let seconds = fields
                .get("seconds")
                .and_then(Json::as_str)
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or("$time.seconds must be a decimal string")?;
            let nanos = fields
                .get("nanos")
                .and_then(Json::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n < 1_000_000_000)
                .ok_or("$time.nanos must be an integer below 10^9")?;
            let utc_offset = fields
                .get("utc_offset")
                .and_then(Json::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("$time.utc_offset must be an integer")?;
            let utc = fields
                .get("utc")
                .and_then(Json::as_bool)
                .ok_or("$time.utc must be a boolean")?;
            if fields.len() != 4 {
                return Err("$time takes only seconds, nanos, utc_offset and utc".to_owned());
            }
            if utc && utc_offset != 0 {
                return Err("a UTC $time has utc_offset 0".to_owned());
            }
            Ok(Time::new(seconds, nanos, utc_offset, utc))
        }
        _ => Err("$time takes a string or an object".to_owned()),
    }
}

/// A Time in the wire encoding.
pub fn time(t: &Time) -> Json {
    let (y, mo, d) = civil_from_days(t.seconds.div_euclid(86_400));
    let rest = t.seconds.rem_euclid(86_400);
    if t.utc && t.utc_offset == 0 && (0..=9999).contains(&y) {
        return json!({"$time": format!(
            "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:09}Z",
            rest / 3600,
            rest % 3600 / 60,
            rest % 60,
            t.nanos
        )});
    }
    json!({"$time": {
        "seconds": t.seconds.to_string(),
        "nanos": t.nanos,
        "utc_offset": t.utc_offset,
        "utc": t.utc,
    }})
}

// --------------------------------------------------------------- encoding

/// Bytes as a wire string: the text when it is UTF-8, `$bytes` otherwise.
pub fn bytes(data: &[u8]) -> Json {
    match std::str::from_utf8(data) {
        Ok(text) => Json::String(text.to_owned()),
        Err(_) => {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            json!({ "$bytes": hex })
        }
    }
}

/// A Float in the wire encoding.
pub fn float(f: f64) -> Json {
    json!({ "$float": float_to_s(f) })
}

/// An Integer in the wire encoding.
pub fn int(i: &BigInt) -> Json {
    // serde_json (arbitrary_precision) writes any integer exactly.
    serde_json::from_str(&i.to_string()).unwrap_or_else(|_| json!({ "$int": i.to_string() }))
}

/// A Ruby value in the wire encoding.
pub fn encode(value: &Value) -> Json {
    match value {
        Value::Nil => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Integer(i) => int(i),
        Value::Float(f) => float(*f),
        Value::String(s) => Json::String(s.clone()),
        Value::Symbol(s) => json!({ "$sym": s }),
        Value::Time(t) => time(t),
        Value::Regexp(r) if r.options == 0 => json!({ "$regexp": r.source }),
        Value::Regexp(r) => json!({ "$regexp": {"source": r.source, "options": r.options} }),
        Value::Array(items) => Json::Array(items.iter().map(encode).collect()),
        Value::Hash(pairs) => {
            let string_keys = pairs.iter().all(|(k, _)| matches!(k, Value::String(_)));
            if string_keys {
                let mut object = Map::new();
                for (k, v) in pairs {
                    if let Value::String(k) = k {
                        object.insert(k.clone(), encode(v));
                    }
                }
                Json::Object(object)
            } else {
                json!({ "$map": pairs.iter().map(|(k, v)| json!([encode(k), encode(v)])).collect::<Vec<_>>() })
            }
        }
    }
}
