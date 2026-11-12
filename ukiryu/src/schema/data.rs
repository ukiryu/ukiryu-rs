//! The Ruby object model that schema validation sees, and the Ruby
//! spellings (`to_s`, `inspect`, `JSON.dump`) that end up in messages and in
//! the schema identifier.

use super::inspect_table::ESCAPED;
use super::SchemaError;
use num_bigint::BigInt;

/// A value as the original Ruby code holds it after `YAML.safe_load`.
///
/// Hash keys stay as loaded (`1:` is an Integer key); validation applies
/// Ruby's `to_s` to keys exactly where the original does.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Null,
    Bool(bool),
    Integer(BigInt),
    Float(f64),
    String(String),
    /// A Ruby Symbol (only produced when the loader permits `Symbol`).
    Symbol(String),
    /// A Ruby `Date` (only produced when the loader permits `Date`).
    Date {
        year: i64,
        month: u32,
        day: u32,
    },
    /// A Ruby `Time`, kept as its YAML spelling. Validation treats it as an
    /// opaque object; any message that needs its Ruby spelling fails loudly.
    Time(String),
    Array(Vec<Data>),
    /// An insertion-ordered Ruby Hash (no duplicate keys under `eql?`).
    Hash(Vec<(Data, Data)>),
}

impl Data {
    /// Ruby `Hash#[]=`: replace the value of an `eql?` key in place,
    /// otherwise append.
    pub fn hash_insert(pairs: &mut Vec<(Data, Data)>, key: Data, value: Data) {
        if let Some(slot) = pairs.iter_mut().find(|(k, _)| k.eql(&key)) {
            slot.1 = value;
        } else {
            pairs.push((key, value));
        }
    }

    /// Ruby `eql?`: no numeric coercion, structural for containers.
    pub fn eql(&self, other: &Data) -> bool {
        match (self, other) {
            (Data::Null, Data::Null) => true,
            (Data::Bool(a), Data::Bool(b)) => a == b,
            (Data::Integer(a), Data::Integer(b)) => a == b,
            (Data::Float(a), Data::Float(b)) => a == b,
            (Data::String(a), Data::String(b)) => a == b,
            (Data::Symbol(a), Data::Symbol(b)) => a == b,
            (Data::Time(a), Data::Time(b)) => a == b,
            (
                Data::Date { year, month, day },
                Data::Date {
                    year: y2,
                    month: m2,
                    day: d2,
                },
            ) => year == y2 && month == m2 && day == d2,
            (Data::Array(a), Data::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eql(y))
            }
            (Data::Hash(a), Data::Hash(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.iter()
                            .find(|(k2, _)| k2.eql(k))
                            .is_some_and(|(_, v2)| v2.eql(v))
                    })
            }
            _ => false,
        }
    }

    /// Ruby `==`: numerics compare by value, containers element-wise.
    pub fn ruby_eq(&self, other: &Data) -> bool {
        match (self, other) {
            (Data::Integer(_) | Data::Float(_), Data::Integer(_) | Data::Float(_)) => {
                num_cmp(self, other) == Some(std::cmp::Ordering::Equal)
            }
            (Data::Array(a), Data::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.ruby_eq(y))
            }
            (Data::Hash(a), Data::Hash(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.iter()
                            .find(|(k2, _)| k2.eql(k))
                            .is_some_and(|(_, v2)| v2.ruby_eq(v))
                    })
            }
            _ => self.eql(other),
        }
    }

    /// Ruby truthiness: everything except `nil` and `false`.
    pub fn truthy(&self) -> bool {
        !matches!(self, Data::Null | Data::Bool(false))
    }

    /// Ruby `to_s`.
    pub fn ruby_to_s(&self) -> Result<String, SchemaError> {
        Ok(match self {
            Data::Null => String::new(),
            Data::Bool(b) => b.to_string(),
            Data::Integer(i) => i.to_string(),
            Data::Float(f) => ruby_float_to_s(*f),
            Data::String(s) | Data::Symbol(s) => s.clone(),
            Data::Date { year, month, day } => date_to_s(*year, *month, *day),
            Data::Time(raw) => {
                return Err(SchemaError::unsupported(
                    "Ruby Time spelling",
                    format!("Time#to_s of the YAML timestamp {raw:?} is not reproduced"),
                ))
            }
            Data::Array(_) | Data::Hash(_) => self.ruby_inspect()?,
        })
    }

    /// Ruby `inspect` (Ruby 3.4+ Hash spelling).
    pub fn ruby_inspect(&self) -> Result<String, SchemaError> {
        Ok(match self {
            Data::Null => "nil".to_owned(),
            Data::String(s) => inspect_str(s),
            Data::Symbol(s) => {
                return Err(SchemaError::unsupported(
                    "Ruby Symbol spelling",
                    format!("Symbol#inspect of :{s} is not reproduced"),
                ))
            }
            Data::Date { year, month, day } => date_inspect(*year, *month, *day),
            Data::Time(raw) => {
                return Err(SchemaError::unsupported(
                    "Ruby Time spelling",
                    format!("Time#inspect of the YAML timestamp {raw:?} is not reproduced"),
                ))
            }
            Data::Array(items) => {
                let parts: Result<Vec<_>, _> = items.iter().map(Data::ruby_inspect).collect();
                format!("[{}]", parts?.join(", "))
            }
            Data::Hash(pairs) => {
                if pairs.is_empty() {
                    return Ok("{}".to_owned());
                }
                let mut parts = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let key = match k {
                        Data::Symbol(_) => {
                            return Err(SchemaError::unsupported(
                                "Ruby Symbol spelling",
                                "Hash#inspect with Symbol keys is not reproduced",
                            ))
                        }
                        other => other.ruby_inspect()?,
                    };
                    parts.push(format!("{key} => {}", v.ruby_inspect()?));
                }
                format!("{{{}}}", parts.join(", "))
            }
            Data::Bool(_) | Data::Integer(_) | Data::Float(_) => self.ruby_to_s()?,
        })
    }
}

/// Exact Ruby `<=>` between two numerics (Integer/Float). `None` for NaN or
/// non-numerics.
pub(crate) fn num_cmp(a: &Data, b: &Data) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Data::Integer(x), Data::Integer(y)) => Some(x.cmp(y)),
        (Data::Float(x), Data::Float(y)) => x.partial_cmp(y),
        (Data::Integer(x), Data::Float(y)) => int_float_cmp(x, *y),
        (Data::Float(x), Data::Integer(y)) => int_float_cmp(y, *x).map(|o| o.reverse()),
        _ => None,
    }
}

fn int_float_cmp(i: &BigInt, f: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let floor = f.floor();
    let floor_int = float_to_bigint(floor);
    match i.cmp(&floor_int) {
        Ordering::Equal if f > floor => Some(Ordering::Less),
        other => Some(other),
    }
}

/// Exact integer value of an integral, finite f64.
fn float_to_bigint(f: f64) -> BigInt {
    let bits = f.to_bits();
    let negative = bits >> 63 == 1;
    let exponent = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 {
        return BigInt::from(0);
    }
    let mantissa = BigInt::from(fraction | (1u64 << 52));
    let shift = exponent - 1075;
    let magnitude = if shift >= 0 {
        mantissa << (shift as usize)
    } else {
        mantissa >> ((-shift) as usize)
    };
    if negative {
        -magnitude
    } else {
        magnitude
    }
}

/// Ruby `Integer#to_f` (round to nearest).
pub(crate) fn bigint_to_f64(i: &BigInt) -> f64 {
    i.to_string().parse::<f64>().unwrap_or(f64::NAN)
}

/// Shortest round-trip digits and the decimal exponent of the first digit,
/// for a finite, non-zero magnitude.
fn shortest_digits(f: f64) -> (String, i32) {
    let s = format!("{:e}", f.abs());
    let (mantissa, exp) = s.split_once('e').expect("exponent form");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exp.parse().expect("exponent"))
}

/// Ruby 4.0 `Float#to_s`.
pub fn ruby_float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    if f == 0.0 {
        return format!("{sign}0.0");
    }
    let (digits, exp10) = shortest_digits(f);
    let decpt = exp10 + 1;
    if decpt > -4 && decpt <= 15 {
        if decpt <= 0 {
            format!("{sign}0.{}{}", "0".repeat((-decpt) as usize), digits)
        } else {
            let decpt = decpt as usize;
            if digits.len() <= decpt {
                format!("{sign}{}{}.0", digits, "0".repeat(decpt - digits.len()))
            } else {
                format!("{sign}{}.{}", &digits[..decpt], &digits[decpt..])
            }
        }
    } else {
        let rest = if digits.len() > 1 { &digits[1..] } else { "0" };
        let esign = if exp10 < 0 { '-' } else { '+' };
        format!(
            "{sign}{}.{}e{}{:02}",
            &digits[..1],
            rest,
            esign,
            exp10.unsigned_abs()
        )
    }
}

/// `JSON.dump` float spelling of json 3.1.0 (vendored fpconv `emit_digits`).
pub(crate) fn json_float(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let neg = f.is_sign_negative();
    let sign = if neg { "-" } else { "" };
    if f == 0.0 {
        return format!("{sign}0.0");
    }
    let (digits, exp10) = shortest_digits(f);
    let mut ndigits = digits.len() as i32;
    let k = exp10 - (ndigits - 1);
    let exp = (k + ndigits - 1).abs();
    if k >= 0 && exp < 15 {
        return format!("{sign}{}{}.0", digits, "0".repeat(k as usize));
    }
    if k < 0 && (k > -7 || exp < 10) {
        let offset = ndigits - k.abs();
        return if offset <= 0 {
            format!("{sign}0.{}{}", "0".repeat((-offset) as usize), digits)
        } else {
            let o = offset as usize;
            format!("{sign}{}.{}", &digits[..o], &digits[o..])
        };
    }
    ndigits = ndigits.min(18 - i32::from(neg));
    let mut out = String::from(sign);
    out.push_str(&digits[..1]);
    if ndigits > 1 {
        out.push('.');
        out.push_str(&digits[1..ndigits as usize]);
    }
    out.push('e');
    out.push(if k + ndigits - 1 < 0 { '-' } else { '+' });
    let mut e = exp;
    let mut cent = 0;
    if e > 99 {
        cent = e / 100;
        out.push(char::from(b'0' + cent as u8));
        e -= cent * 100;
    }
    if e > 9 {
        let dec = e / 10;
        out.push(char::from(b'0' + dec as u8));
        e -= dec * 10;
    } else if cent != 0 {
        out.push('0');
    }
    out.push(char::from(b'0' + (e % 10) as u8));
    out
}

/// Ruby `String#inspect` for a UTF-8 string.
pub fn inspect_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            '\u{8}' => out.push_str("\\b"),
            '\u{7}' => out.push_str("\\a"),
            '\u{1b}' => out.push_str("\\e"),
            '#' => {
                if matches!(chars.peek(), Some('{' | '$' | '@')) {
                    out.push_str("\\#");
                } else {
                    out.push('#');
                }
            }
            c if is_escaped(c) => {
                let cp = c as u32;
                if cp > 0xFFFF {
                    out.push_str(&format!("\\u{{{cp:X}}}"));
                } else {
                    out.push_str(&format!("\\u{cp:04X}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_escaped(c: char) -> bool {
    let cp = c as u32;
    ESCAPED
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn date_to_s(year: i64, month: u32, day: u32) -> String {
    if year < 0 {
        format!("-{:04}-{month:02}-{day:02}", -year)
    } else {
        format!("{year:04}-{month:02}-{day:02}")
    }
}

/// Julian Day Number of a proleptic Gregorian date.
fn jdn(year: i64, month: u32, day: u32) -> i64 {
    let a = (14 - i64::from(month)) / 12;
    let y = year + 4800 - a;
    let m = i64::from(month) + 12 * a - 3;
    i64::from(day) + (153 * m + 2) / 5 + 365 * y + y.div_euclid(4) - y.div_euclid(100)
        + y.div_euclid(400)
        - 32045
}

fn date_inspect(year: i64, month: u32, day: u32) -> String {
    format!(
        "#<Date: {} (({}j,0s,0n),+0s,2299161j)>",
        date_to_s(year, month, day),
        jdn(year, month, day)
    )
}

/// Whether a proleptic Gregorian civil date exists.
pub(crate) fn valid_civil(year: i64, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = (year.rem_euclid(4) == 0 && year.rem_euclid(100) != 0) || year.rem_euclid(400) == 0;
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day <= days
}

impl From<&serde_json::Value> for Data {
    /// Map a JSON value onto the Ruby model. Object key order is whatever
    /// `serde_json` preserves (sorted unless its `preserve_order` feature is
    /// on), which matters for error order; prefer the YAML loaders when the
    /// document order is known.
    fn from(v: &serde_json::Value) -> Data {
        match v {
            serde_json::Value::Null => Data::Null,
            serde_json::Value::Bool(b) => Data::Bool(*b),
            serde_json::Value::Number(n) => {
                let text = n.to_string();
                if text.contains(['.', 'e', 'E']) {
                    Data::Float(text.parse().unwrap_or(f64::NAN))
                } else {
                    text.parse::<BigInt>()
                        .map(Data::Integer)
                        .unwrap_or(Data::Float(f64::NAN))
                }
            }
            serde_json::Value::String(s) => Data::String(s.clone()),
            serde_json::Value::Array(items) => Data::Array(items.iter().map(Data::from).collect()),
            serde_json::Value::Object(map) => {
                let mut pairs = Vec::with_capacity(map.len());
                for (k, v) in map {
                    Data::hash_insert(&mut pairs, Data::String(k.clone()), Data::from(v));
                }
                Data::Hash(pairs)
            }
        }
    }
}

impl Data {
    /// Map a `yeptris` YAML 1.2 value onto the Ruby model as-is (no Psych
    /// YAML 1.1 re-resolution: `yes` stays a string). Tagged nodes fail
    /// loudly.
    pub fn from_yaml_value(v: &yeptris::Value) -> Result<Data, SchemaError> {
        Ok(match v {
            yeptris::Value::Null => Data::Null,
            yeptris::Value::Bool(b) => Data::Bool(*b),
            yeptris::Value::Number(yeptris::Number::I64(i)) => Data::Integer(BigInt::from(*i)),
            yeptris::Value::Number(yeptris::Number::U64(u)) => Data::Integer(BigInt::from(*u)),
            yeptris::Value::Number(yeptris::Number::F64(f)) => Data::Float(*f),
            yeptris::Value::String(s) => Data::String(s.clone()),
            yeptris::Value::Sequence(items) => Data::Array(
                items
                    .iter()
                    .map(Data::from_yaml_value)
                    .collect::<Result<_, _>>()?,
            ),
            yeptris::Value::Mapping(map) => {
                let mut pairs = Vec::with_capacity(map.0.len());
                for (k, v) in &map.0 {
                    Data::hash_insert(
                        &mut pairs,
                        Data::from_yaml_value(k)?,
                        Data::from_yaml_value(v)?,
                    );
                }
                Data::Hash(pairs)
            }
            yeptris::Value::Tagged(t) => {
                return Err(SchemaError::unsupported(
                    "YAML tag",
                    format!("tagged node {} is not mapped onto Ruby objects", t.tag),
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_spellings_match_probe() {
        // Pairs captured from Ruby 4.0.1 / json 3.1.0 (K17-SCHEMA evidence,
        // captures/json-float-probe.txt).
        let cases: &[(f64, &str, &str)] = &[
            (1.0, "1.0", "1.0"),
            (2.5, "2.5", "2.5"),
            (1e15, "1.0e+15", "1e+15"),
            (
                1.2345678901234568e17,
                "1.2345678901234568e+17",
                "1.2345678901234568e+17",
            ),
            (1e20, "1.0e+20", "1e+20"),
            (1e-4, "0.0001", "0.0001"),
            (1e-5, "1.0e-05", "0.00001"),
            (1.5e-7, "1.5e-07", "0.00000015"),
            (-0.0, "-0.0", "-0.0"),
            (5e-324, "5.0e-324", "5e-324"),
            (
                1.7976931348623157e308,
                "1.7976931348623157e+308",
                "1.7976931348623157e+308",
            ),
            (3.0e10, "30000000000.0", "30000000000.0"),
            (999999999999999.0, "999999999999999.0", "999999999999999.0"),
            (12345678.9, "12345678.9", "12345678.9"),
            (-2.5e-8, "-2.5e-08", "-0.000000025"),
            (1.5e300, "1.5e+300", "1.5e+300"),
        ];
        for (f, to_s, json) in cases {
            assert_eq!(ruby_float_to_s(*f), *to_s, "to_s {f:e}");
            assert_eq!(json_float(*f), *json, "json {f:e}");
        }
    }

    #[test]
    fn inspect_matches_probe() {
        assert_eq!(
            inspect_str("\u{1}\u{7f}\u{80}\u{200b}\u{e0001}\u{1b}\u{7}#{x}#$a#@b#c\"\\"),
            "\"\\u0001\\u007F\\u0080\u{200b}\u{e0001}\\e\\a\\#{x}\\#$a\\#@b#c\\\"\\\\\""
        );
    }

    #[test]
    fn mixed_numeric_comparison_is_exact() {
        use std::cmp::Ordering::*;
        let i = |v: i64| Data::Integer(BigInt::from(v));
        assert_eq!(num_cmp(&i(1), &Data::Float(1.0)), Some(Equal));
        assert_eq!(num_cmp(&i(1), &Data::Float(1.5)), Some(Less));
        assert_eq!(num_cmp(&i(-2), &Data::Float(-1.5)), Some(Less));
        assert_eq!(
            num_cmp(&Data::Float(9007199254740993.0), &i(9007199254740993)),
            Some(Less)
        );
    }
}
