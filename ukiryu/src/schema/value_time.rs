//! Psych timestamp spellings using the core's exact Ruby Time formatter.
use super::SchemaError;
use crate::types::{ruby, Host, StdHost, Time};
use num_bigint::BigInt;

pub(super) fn parse(raw: &str) -> Result<Time, SchemaError> {
    if raw.is_empty() {
        return Err(SchemaError::ruby("TypeError", "uninitialized Time"));
    }
    // Only called for timestamp-shaped scalars admitted by Psych's scanner.
    let pattern = regex::Regex::new(
        r"^(-?\d{4})-(\d{1,2})-(\d{1,2})(?:[Tt]|\s+)(\d{1,2}):(\d\d):(\d\d)(?:\.(\d*))?\s*(Z|[-+]\d{1,2}:?(?:\d\d)?)?$",
    ).expect("timestamp pattern");
    let c = pattern
        .captures(raw)
        .ok_or_else(|| SchemaError::ruby("ArgumentError", "invalid YAML timestamp"))?;
    let n = |i: usize| c[i].parse::<i64>().expect("timestamp field");
    let (y, m, d, h, mi, se) = (n(1), n(2), n(3), n(4), n(5), n(6));
    let local = ruby::days_from_civil(y, m as u32, d as u32) * 86400 + h * 3600 + mi * 60 + se;
    let zone = c.get(8).map(|m| m.as_str());
    let (seconds, offset, utc) = match zone {
        Some("Z") => (local, 0, true),
        Some(z) => {
            let body = &z[1..];
            let (hours, minutes) = if let Some((h, m)) = body.split_once(':') {
                (h.parse::<i32>().unwrap(), m.parse::<i32>().unwrap_or(0))
            } else if body.len() > 2 {
                let (h, m) = body.split_at(body.len() - 2);
                (h.parse::<i32>().unwrap(), m.parse::<i32>().unwrap())
            } else {
                (body.parse::<i32>().unwrap(), 0)
            };
            let offset = (hours * 3600 + minutes * 60) * if z.starts_with('-') { -1 } else { 1 };
            if offset.abs() >= 86400 {
                return Err(SchemaError::ruby(
                    "ArgumentError",
                    "utc_offset out of range",
                ));
            }
            (local - i64::from(offset), offset, false)
        }
        None => {
            // Psych constructs UTC first, then calls getlocal. The core only
            // supplies local-zone data when the host explicitly selects UTC.
            let offset = StdHost
                .local_offset(y, m as u32, d as u32, h as u32, mi as u32, se as u32)
                .ok_or_else(|| {
                    SchemaError::unsupported(
                        "schema local timezone",
                        "a timezone-less YAML timestamp needs a host local UTC offset",
                    )
                })?;
            (local, offset, false)
        }
    };
    let fraction = c.get(7).map_or("", |v| v.as_str());
    let denom = BigInt::from(10).pow(fraction.len() as u32);
    let frac = BigInt::parse_bytes(fraction.as_bytes(), 10).unwrap_or_else(|| 0.into());
    let numer = BigInt::from(seconds) * &denom + frac;
    Ok(Time::from_rational(&numer, &denom, offset, utc).expect("timestamp range"))
}
