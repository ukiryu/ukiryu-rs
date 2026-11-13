//! The corpus value encoding (README "Value encoding") to and from the
//! core's Ruby value model, `ukiryu::types::Value`.
use std::str::FromStr;

use num_bigint::BigInt;
use ukiryu::types::ruby::float_to_s;
use ukiryu::types::{Regexp, Time, Value};

use super::json::Oj;

/// Decode a row input into a core value. `$bytes` has no `Value` form
/// (strings are UTF-8); ops that take raw bytes read them with [`bytes`].
pub fn decode(j: &Oj) -> Result<Value, String> {
    Ok(match j {
        Oj::Null => Value::Nil,
        Oj::Bool(b) => Value::Bool(*b),
        Oj::Num(n) => {
            Value::Integer(BigInt::from_str(n).map_err(|_| format!("non-integer JSON number {n}"))?)
        }
        Oj::Str(s) => Value::String(s.clone()),
        Oj::Arr(items) => Value::Array(items.iter().map(decode).collect::<Result<_, _>>()?),
        Oj::Obj(pairs) => match j.tag() {
            Some(("$sym", Oj::Str(s))) => Value::Symbol(s.clone()),
            Some(("$float", Oj::Str(s))) => Value::Float(parse_float(s)?),
            Some(("$time", Oj::Str(s))) => Value::Time(parse_time(s)?),
            Some(("$regexp", Oj::Str(s))) => Value::Regexp(Regexp {
                source: s.clone(),
                options: 0,
            }),
            Some(("$map", Oj::Arr(entries))) => Value::Hash(
                entries
                    .iter()
                    .map(|e| match e {
                        Oj::Arr(kv) if kv.len() == 2 => Ok((decode(&kv[0])?, decode(&kv[1])?)),
                        _ => Err(format!("bad $map entry {e:?}")),
                    })
                    .collect::<Result<_, String>>()?,
            ),
            Some(("$bytes", _)) => return Err("$bytes has no core Value form".into()),
            _ => Value::Hash(
                pairs
                    .iter()
                    .map(|(k, v)| Ok((Value::String(k.clone()), decode(v)?)))
                    .collect::<Result<_, String>>()?,
            ),
        },
    })
}

/// Raw bytes of a string or `$bytes` input.
pub fn bytes(j: &Oj) -> Result<Vec<u8>, String> {
    match j {
        Oj::Str(s) => Ok(s.as_bytes().to_vec()),
        _ => match j.tag() {
            Some(("$bytes", Oj::Str(hex))) => {
                if hex.len() % 2 != 0 {
                    return Err("odd $bytes length".into());
                }
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
                    .collect()
            }
            _ => Err(format!("expected string or $bytes, got {j:?}")),
        },
    }
}

/// Encode raw bytes the way the oracle does: UTF-8 text stays a string,
/// anything else is `$bytes`.
pub fn encode_bytes(b: &[u8]) -> Oj {
    match std::str::from_utf8(b) {
        Ok(s) => Oj::Str(s.to_string()),
        Err(_) => Oj::tagged(
            "$bytes",
            Oj::Str(b.iter().map(|x| format!("{x:02x}")).collect()),
        ),
    }
}

pub fn encode(v: &Value) -> Oj {
    match v {
        Value::Nil => Oj::Null,
        Value::Bool(b) => Oj::Bool(*b),
        Value::Integer(i) => Oj::Num(i.to_string()),
        Value::Float(f) => Oj::tagged("$float", Oj::Str(float_to_s(*f))),
        Value::String(s) => Oj::Str(s.clone()),
        Value::Symbol(s) => Oj::sym(s.clone()),
        Value::Time(t) => Oj::tagged("$time", Oj::Str(time_iso(t))),
        Value::Regexp(r) => Oj::tagged("$regexp", Oj::Str(r.source.clone())),
        Value::Array(items) => Oj::Arr(items.iter().map(encode).collect()),
        Value::Hash(pairs) => {
            if pairs.iter().all(|(k, _)| matches!(k, Value::String(_))) {
                Oj::Obj(
                    pairs
                        .iter()
                        .map(|(k, x)| match k {
                            Value::String(s) => (s.clone(), encode(x)),
                            _ => unreachable!(),
                        })
                        .collect(),
                )
            } else {
                Oj::tagged(
                    "$map",
                    Oj::Arr(
                        pairs
                            .iter()
                            .map(|(k, x)| Oj::Arr(vec![encode(k), encode(x)]))
                            .collect(),
                    ),
                )
            }
        }
    }
}

fn parse_float(s: &str) -> Result<f64, String> {
    match s {
        "NaN" => Ok(f64::NAN),
        "Infinity" => Ok(f64::INFINITY),
        "-Infinity" => Ok(f64::NEG_INFINITY),
        other => f64::from_str(other).map_err(|_| format!("bad $float {other}")),
    }
}

/// `YYYY-MM-DDTHH:MM:SS.fffffffffZ` (the oracle writes UTC, 9 digits).
fn parse_time(iso: &str) -> Result<Time, String> {
    let bad = || format!("bad $time {iso}");
    if iso.len() != 30 || !iso.ends_with('Z') {
        return Err(bad());
    }
    let n = |r: std::ops::Range<usize>| iso[r].parse::<i64>().map_err(|_| bad());
    let days = days_from_civil(n(0..4)?, n(5..7)?, n(8..10)?);
    Ok(Time::new(
        days * 86_400 + n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?,
        n(20..29)? as u32,
        0,
        true,
    ))
}

/// Ruby `Time#utc.iso8601(9)`.
fn time_iso(t: &Time) -> String {
    let (y, mo, d) = civil_from_days(t.seconds.div_euclid(86_400));
    let sod = t.seconds.rem_euclid(86_400);
    let year = if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    };
    format!(
        "{year}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:09}Z",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60,
        t.nanos
    )
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
