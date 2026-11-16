//! Ruby core behaviour embedded in Ukiryu's messages and checks: `to_s`,
//! `inspect`, `dump`, `Float#to_s`, `String#downcase`, `String#strip`, and the
//! `<`/`>` comparisons (including their failure messages). Each function
//! reproduces Ruby 4.0.1 as captured by the K19-TYPES oracle rows.
use std::cmp::Ordering;
use std::fmt::Write as _;
use std::str::FromStr;

use num_bigint::BigInt;

use super::error::{Error, ErrorClass, Result};
use super::nonprintable::NONPRINTABLE;
use super::value::{Regexp, Time, Value};

/// `Float#to_s`.
pub fn float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    // Shortest round-trip digits, as Ruby's dtoa mode 0.
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').expect("LowerExp has an exponent");
    let exp: i32 = exp.parse().expect("integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt = exp + 1;
    let n = digits.len() as i32;
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if decpt > -4 && (decpt < 16 || n > decpt) {
        if decpt <= 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-decpt) as usize));
            out.push_str(&digits);
        } else if decpt >= n {
            out.push_str(&digits);
            out.extend(std::iter::repeat_n('0', (decpt - n) as usize));
            out.push_str(".0");
        } else {
            out.push_str(&digits[..decpt as usize]);
            out.push('.');
            out.push_str(&digits[decpt as usize..]);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if n > 1 { &digits[1..] } else { "0" });
        let e = decpt - 1;
        let _ = write!(out, "e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    out
}

/// `Object#to_s` for the value domain (string interpolation).
pub fn to_s(v: &Value) -> String {
    match v {
        Value::Nil => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => float_to_s(*f),
        Value::String(s) | Value::Symbol(s) => s.clone(),
        Value::Time(t) => time_to_s(t),
        Value::Regexp(r) => regexp_to_s(r),
        Value::Array(_) | Value::Hash(_) => inspect(v),
    }
}

/// `Object#inspect`.
pub fn inspect(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::String(s) => string_inspect(s),
        Value::Symbol(s) => symbol_inspect(s),
        Value::Time(t) => time_inspect(t),
        Value::Regexp(r) => regexp_inspect(r),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(inspect).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Hash(pairs) => {
            if pairs.is_empty() {
                return "{}".into();
            }
            let parts: Vec<String> = pairs
                .iter()
                .map(|(k, val)| match k {
                    Value::Symbol(name) if is_simple_symbol_key(name) => {
                        format!("{name}: {}", inspect(val))
                    }
                    Value::Symbol(name) => format!("{}: {}", string_inspect(name), inspect(val)),
                    _ => format!("{} => {}", inspect(k), inspect(val)),
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        _ => to_s(v),
    }
}

/// `Array#join(', ')`: elements by `to_s`, nested arrays joined recursively.
pub fn join(items: &[Value], sep: &str) -> String {
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::Array(inner) => parts.push(join(inner, sep)),
            other => parts.push(to_s(other)),
        }
    }
    parts.join(sep)
}

fn is_printable(c: char) -> bool {
    let cp = c as u32;
    NONPRINTABLE
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                Ordering::Less
            } else if lo > cp {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_err()
}

fn push_common_escape(out: &mut String, c: char, next: Option<char>) -> bool {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\x0C' => out.push_str("\\f"),
        '\x0B' => out.push_str("\\v"),
        '\x08' => out.push_str("\\b"),
        '\x07' => out.push_str("\\a"),
        '\x1B' => out.push_str("\\e"),
        '#' if matches!(next, Some('{' | '$' | '@')) => out.push_str("\\#"),
        _ => return false,
    }
    true
}

fn push_unicode_escape(out: &mut String, c: char) {
    let cp = c as u32;
    if cp > 0xFFFF {
        let _ = write!(out, "\\u{{{cp:X}}}");
    } else {
        let _ = write!(out, "\\u{cp:04X}");
    }
}

/// `String#inspect` for a UTF-8 string.
pub fn string_inspect(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if push_common_escape(&mut out, c, chars.get(i + 1).copied()) {
            continue;
        }
        if is_printable(c) {
            out.push(c);
        } else {
            push_unicode_escape(&mut out, c);
        }
    }
    out.push('"');
    out
}

/// `String#dump` for a UTF-8 string.
pub fn dump(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if push_common_escape(&mut out, c, chars.get(i + 1).copied()) {
            continue;
        }
        if c.is_ascii() {
            if (' '..='~').contains(&c) {
                out.push(c);
            } else {
                let _ = write!(out, "\\x{:02X}", c as u32);
            }
        } else {
            push_unicode_escape(&mut out, c);
        }
    }
    out.push('"');
    out
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || !c.is_ascii()
}

fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii()
}

fn is_ident(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if is_ident_start(c) => it.all(is_ident_char),
        _ => false,
    }
}

const OPERATOR_SYMBOLS: &[&str] = &[
    "+", "-", "*", "**", "/", "%", "==", "===", "=~", "!~", "!", "!=", "<", "<=", ">", ">=", "<=>",
    "<<", ">>", "&", "|", "^", "~", "+@", "-@", "[]", "[]=", "`",
];

fn symbol_needs_no_quotes(s: &str) -> bool {
    if s.is_empty() || !s.chars().all(is_printable) {
        return false;
    }
    if OPERATOR_SYMBOLS.contains(&s) {
        return true;
    }
    if let Some(rest) = s.strip_prefix('$') {
        let chars: Vec<char> = rest.chars().collect();
        if chars.len() == 1 && "~*$?!@/\\;,.=:<>\"&`'+0".contains(chars[0]) {
            return true;
        }
        if chars.len() == 2 && chars[0] == '-' && is_ident_char(chars[1]) {
            return true;
        }
        if !chars.is_empty() && chars.iter().all(char::is_ascii_digit) {
            return true;
        }
        return is_ident(rest);
    }
    if let Some(rest) = s.strip_prefix("@@") {
        return is_ident(rest);
    }
    if let Some(rest) = s.strip_prefix('@') {
        return is_ident(rest);
    }
    let base = s
        .strip_suffix('?')
        .or_else(|| s.strip_suffix('!'))
        .or_else(|| s.strip_suffix('='))
        .unwrap_or(s);
    is_ident(base)
}

fn is_simple_symbol_key(s: &str) -> bool {
    let base = s
        .strip_suffix('?')
        .or_else(|| s.strip_suffix('!'))
        .unwrap_or(s);
    s.chars().all(is_printable) && is_ident(base)
}

/// `Symbol#inspect`.
pub fn symbol_inspect(s: &str) -> String {
    if symbol_needs_no_quotes(s) {
        format!(":{s}")
    } else {
        format!(":{}", string_inspect(s))
    }
}

/// `String#downcase` (full Unicode mapping, applied per character; Ruby does
/// not apply the final-sigma rule).
pub fn downcase(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// `String#strip`: removes leading and trailing NUL and ASCII whitespace.
pub fn strip(s: &str) -> &str {
    let ws = |c: char| matches!(c, '\0' | ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r');
    s.trim_matches(ws)
}

fn regexp_flags(options: u32, set: bool) -> String {
    let mut out = String::new();
    for (bit, ch) in [
        (Regexp::MULTILINE, 'm'),
        (Regexp::IGNORECASE, 'i'),
        (Regexp::EXTENDED, 'x'),
    ] {
        if (options & bit != 0) == set {
            out.push(ch);
        }
    }
    out
}

/// The source with unescaped `/` written as `\/`, as `Regexp#to_s` and
/// `#inspect` print it (`Regexp#source` itself keeps a bare `/`).
fn regexp_source_escaped(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut escaped = false;
    for c in source.chars() {
        if c == '/' && !escaped {
            out.push('\\');
        }
        escaped = c == '\\' && !escaped;
        out.push(c);
    }
    out
}

/// `Regexp#to_s`, e.g. `(?-mix:\A[a-z]+\z)`.
pub fn regexp_to_s(r: &Regexp) -> String {
    let on = regexp_flags(r.options, true);
    let off = regexp_flags(r.options, false);
    if off.is_empty() {
        format!("(?{on}:{})", regexp_source_escaped(&r.source))
    } else {
        format!("(?{on}-{off}:{})", regexp_source_escaped(&r.source))
    }
}

/// `Regexp#inspect`, e.g. `/a\/b/i`.
pub fn regexp_inspect(r: &Regexp) -> String {
    format!(
        "/{}/{}",
        regexp_source_escaped(&r.source),
        regexp_flags(r.options, true)
    )
}

/// Civil date from days since 1970-01-01 (proleptic Gregorian).
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn time_parts(t: &Time) -> String {
    let local = t.seconds + t.utc_offset as i64;
    let (y, mo, d) = civil_from_days(local.div_euclid(86_400));
    let sod = local.rem_euclid(86_400);
    // Ruby pads the year to four digits after the sign (`-0099`).
    let y = if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    };
    format!(
        "{y}-{mo:02}-{d:02} {:02}:{:02}:{:02}",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

fn time_zone(t: &Time) -> String {
    if t.utc {
        "UTC".into()
    } else {
        let o = t.utc_offset;
        let sign = if o < 0 { '-' } else { '+' };
        let a = o.abs();
        format!("{sign}{:02}{:02}", a / 3600, a % 3600 / 60)
    }
}

/// `Time#to_s`.
pub fn time_to_s(t: &Time) -> String {
    format!("{} {}", time_parts(t), time_zone(t))
}

/// `Time#inspect` (sub-second digits without trailing zeros, or the exact
/// `Rational` of a fraction finer than a nanosecond, `10:20:30
/// 1234567891/10000000000`; an offset with seconds prints them, `+053015`).
pub fn time_inspect(t: &Time) -> String {
    let mut s = time_parts(t);
    s.push_str(&t.inspect_subsec());
    let mut zone = time_zone(t);
    if !t.utc && t.utc_offset % 60 != 0 {
        zone.push_str(&format!("{:02}", t.utc_offset.abs() % 60));
    }
    format!("{s} {zone}")
}

/// Exact decimal BigInt for an integral finite f64.
pub(crate) fn bigint_from_integral_f64(f: f64) -> BigInt {
    BigInt::from_str(&format!("{f:.0}")).expect("integral float formats as an integer")
}

/// Ruby's Integer-to-Float conversion (correctly rounded; overflows to Infinity).
pub(crate) fn bigint_to_f64(i: &BigInt) -> f64 {
    f64::from_str(&i.to_string()).expect("decimal integer parses as f64")
}

fn int_float_cmp(i: &BigInt, f: f64) -> Option<Ordering> {
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
    let t = f.trunc();
    let ti = bigint_from_integral_f64(t);
    match i.cmp(&ti) {
        Ordering::Equal => {
            let frac = f - t;
            Some(if frac > 0.0 {
                Ordering::Less
            } else if frac < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            })
        }
        o => Some(o),
    }
}

/// Exact comparison of two numeric values (`None` when either is NaN or a
/// value is not numeric).
pub fn numeric_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Integer(x), Value::Float(y)) => int_float_cmp(x, *y),
        (Value::Float(x), Value::Integer(y)) => int_float_cmp(y, *x).map(Ordering::reverse),
        _ => None,
    }
}

/// Ruby's `rb_cmperr` message: `comparison of Integer with nil failed`.
fn cmp_failed(a: &Value, b: &Value) -> Error {
    let other = if b.is_special_const() {
        inspect(b)
    } else {
        b.class_name().to_string()
    };
    Error::new(
        ErrorClass::Argument,
        format!("comparison of {} with {other} failed", a.class_name()),
    )
}

/// The receiver part of a Ruby 3.4+ `NoMethodError` message.
pub(crate) fn receiver_description(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => b.to_string(),
        other => format!("an instance of {}", other.class_name()),
    }
}

pub(crate) fn no_method(name: &str, receiver: &Value) -> Error {
    Error::new(
        ErrorClass::NoMethod,
        format!(
            "undefined method '{name}' for {}",
            receiver_description(receiver)
        ),
    )
}

/// Ruby `a <op> b` for `<` and `>` as used by Ukiryu's range checks.
fn compare(a: &Value, b: &Value, op: &str) -> Result<Option<Ordering>> {
    match a {
        Value::Integer(_) | Value::Float(_) => match b {
            Value::Integer(_) | Value::Float(_) => Ok(numeric_cmp(a, b)),
            _ => Err(cmp_failed(a, b)),
        },
        Value::String(x) => match b {
            Value::String(y) => Ok(Some(x.as_bytes().cmp(y.as_bytes()))),
            _ => Err(cmp_failed(a, b)),
        },
        Value::Symbol(x) => match b {
            Value::Symbol(y) => Ok(Some(x.as_bytes().cmp(y.as_bytes()))),
            _ => Err(cmp_failed(a, b)),
        },
        Value::Time(x) => match b {
            Value::Time(y) => Ok(Some(x.instant_cmp(y))),
            _ => Err(cmp_failed(a, b)),
        },
        Value::Hash(_) => Err(Error::unsupported(format!(
            "Hash#{op} (subset comparison) is not ported"
        ))),
        _ => Err(no_method(op, a)),
    }
}

/// Ruby `a < b`.
pub fn lt(a: &Value, b: &Value) -> Result<bool> {
    Ok(compare(a, b, "<")? == Some(Ordering::Less))
}

/// Ruby `a > b`.
pub fn gt(a: &Value, b: &Value) -> Result<bool> {
    Ok(compare(a, b, ">")? == Some(Ordering::Greater))
}
