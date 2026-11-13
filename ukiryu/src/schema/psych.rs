//! `YAML.safe_load` as the original code calls it: the document is parsed
//! by `yeptris`, and plain scalars are then resolved with Psych's YAML 1.1
//! `ScalarScanner#tokenize` rules (Psych 5, default `strict_integer: false`)
//! so that the resulting Ruby objects match the oracle. Classes that the
//! call does not permit raise `Psych::DisallowedClass`, and aliases raise
//! `Psych::AliasesNotEnabled`, as `safe_load` does by default.

use super::data::{valid_civil, Data};
use super::SchemaError;
use num_bigint::BigInt;
use yeptris::{Document, Node, NodeKind, Style};

/// The extra classes a `safe_load` call permits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Permitted {
    pub symbol: bool,
    pub date: bool,
    pub time: bool,
}

impl Permitted {
    /// `permitted_classes: []` (SchemaValidator.load_schema).
    pub const NONE: Permitted = Permitted {
        symbol: false,
        date: false,
        time: false,
    };
    /// `permitted_classes: [Symbol]` (DefinitionValidator.load_schema).
    pub const SYMBOL: Permitted = Permitted {
        symbol: true,
        date: false,
        time: false,
    };
    /// `permitted_classes: [Symbol, Date, Time]` (profile and definition
    /// documents).
    pub const DOCUMENT: Permitted = Permitted {
        symbol: true,
        date: true,
        time: true,
    };
}

/// Load the first document of `text` the way `YAML.safe_load` does.
/// An empty stream is `nil`.
pub fn safe_load(text: &str, permitted: Permitted) -> Result<Data, SchemaError> {
    // A stream with no content is `nil` for Psych; yeptris rejects an
    // empty input instead (with a message left over from an earlier parse).
    if text
        .lines()
        .all(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
    {
        return Ok(Data::Null);
    }
    let doc = Document::parse(text.as_bytes()).map_err(|e| SchemaError::YamlSyntax {
        message: e
            .message()
            .map(str::to_owned)
            .unwrap_or_else(|| e.to_string()),
    })?;
    match doc.first_root() {
        None => Ok(Data::Null),
        Some(root) => node_to_data(root, permitted),
    }
}

fn node_to_data(node: Node<'_>, permitted: Permitted) -> Result<Data, SchemaError> {
    let tag = node.tag().map(|t| String::from_utf8_lossy(t).into_owned());
    match node.kind() {
        NodeKind::Alias => Err(SchemaError::AliasesNotEnabled),
        NodeKind::Scalar => {
            let text = node.str().unwrap_or("");
            match tag.as_deref() {
                None => {}
                Some("tag:yaml.org,2002:str" | "!") => return Ok(Data::String(text.to_owned())),
                Some(other) => {
                    return Err(SchemaError::unsupported(
                        "YAML tag",
                        format!("Psych resolution of the tag {other} is not reproduced"),
                    ))
                }
            }
            if node.style() == Style::Plain {
                tokenize(text, permitted)
            } else {
                Ok(Data::String(text.to_owned()))
            }
        }
        NodeKind::Sequence => {
            reject_tag(tag.as_deref(), "tag:yaml.org,2002:seq")?;
            let mut items = Vec::with_capacity(node.seq_count());
            for item in node.items() {
                items.push(node_to_data(item, permitted)?);
            }
            Ok(Data::Array(items))
        }
        NodeKind::Mapping => {
            reject_tag(tag.as_deref(), "tag:yaml.org,2002:map")?;
            let mut pairs = Vec::with_capacity(node.map_count());
            for (k, v) in node.entries() {
                if k.kind() == NodeKind::Scalar
                    && k.style() == Style::Plain
                    && k.tag().is_none()
                    && k.str() == Some("<<")
                {
                    return Err(SchemaError::unsupported(
                        "YAML merge key",
                        "Psych `<<` merge semantics are not reproduced",
                    ));
                }
                let key = node_to_data(k, permitted)?;
                let value = node_to_data(v, permitted)?;
                Data::hash_insert(&mut pairs, key, value);
            }
            Ok(Data::Hash(pairs))
        }
    }
}

fn reject_tag(tag: Option<&str>, allowed: &str) -> Result<(), SchemaError> {
    match tag {
        None => Ok(()),
        Some(t) if t == allowed || t == "!" => Ok(()),
        Some(t) => Err(SchemaError::unsupported(
            "YAML tag",
            format!("Psych resolution of the tag {t} is not reproduced"),
        )),
    }
}

fn disallowed(class: &str) -> SchemaError {
    SchemaError::DisallowedClass {
        class: class.to_owned(),
    }
}

fn is_ascii_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// The first `tokenize` guard:
/// `%r{^[^\d.:-]?[[:alpha:]_\s!@#$%\^&*(){}<>|/\\~;=]+}`.
fn word_like(s: &str) -> bool {
    let class = |c: char| {
        c.is_alphabetic()
            || c == '_'
            || c.is_ascii_whitespace()
            || c == '\u{b}'
            || "!@#$%^&*(){}<>|/\\~;=".contains(c)
    };
    let mut chars = s.chars();
    match chars.next() {
        None => false,
        Some(c0) if class(c0) => true,
        Some(c0) => !(is_ascii_digit(c0) || ".:-".contains(c0)) && chars.next().is_some_and(class),
    }
}

/// Psych `ScalarScanner#tokenize` for a plain scalar.
pub(crate) fn tokenize(s: &str, permitted: Permitted) -> Result<Data, SchemaError> {
    if s.is_empty() {
        return Ok(Data::Null);
    }
    if word_like(s) || s.contains('\n') {
        if s.chars().count() > 5 {
            return Ok(Data::String(s.to_owned()));
        }
        let lower = s.to_ascii_lowercase();
        let first = lower.chars().next().unwrap_or(' ');
        if !"ytonf~".contains(first) {
            return Ok(Data::String(s.to_owned()));
        }
        return Ok(match lower.as_str() {
            "~" | "null" => Data::Null,
            "yes" | "true" | "on" => Data::Bool(true),
            "no" | "false" | "off" => Data::Bool(false),
            _ => Data::String(s.to_owned()),
        });
    }
    if is_time(s) {
        if !permitted.time {
            return Err(disallowed("Time"));
        }
        return Ok(if time_fields_valid(s) {
            Data::Time(s.to_owned())
        } else {
            Data::String(s.to_owned())
        });
    }
    if let Some((y, m, d)) = date_parts(s) {
        if !permitted.date {
            return Err(disallowed("Date"));
        }
        return Ok(if valid_civil(y, m, d) {
            Data::Date {
                year: y,
                month: m,
                day: d,
            }
        } else {
            Data::String(s.to_owned())
        });
    }
    let lower = s.to_ascii_lowercase();
    if lower == ".inf" || lower == "+.inf" {
        return Ok(Data::Float(f64::INFINITY));
    }
    if lower == "-.inf" {
        return Ok(Data::Float(f64::NEG_INFINITY));
    }
    if lower == ".nan" {
        return Ok(Data::Float(f64::NAN));
    }
    if s.len() >= 2 && s.starts_with(':') {
        if !permitted.symbol {
            return Err(disallowed("Symbol"));
        }
        // `/^:(["'])(.*)\1/` (greedy) then `$2.sub(/^:/, '')`, else
        // `string.sub(/^:/, '')`.
        let body = &s[1..];
        let quoted = body
            .chars()
            .next()
            .filter(|q| *q == '"' || *q == '\'')
            .and_then(|q| body[1..].rfind(q).map(|end| &body[1..1 + end]));
        let name = match quoted {
            Some(inner) => inner.strip_prefix(':').unwrap_or(inner),
            None => body,
        };
        return Ok(Data::Symbol(name.to_owned()));
    }
    if let Some(v) = sexagesimal(s) {
        return Ok(v);
    }
    if is_float(s) {
        if s.trim_start_matches(['-', '+']) == "." {
            return Ok(Data::String(s.to_owned()));
        }
        let cleaned: String = s.chars().filter(|c| *c != ',' && *c != '_').collect();
        let cleaned = drop_dot_before_exponent(&cleaned);
        return cleaned.parse::<f64>().map(Data::Float).map_err(|_| {
            SchemaError::unsupported(
                "YAML float",
                format!("Psych raises ArgumentError for Float({cleaned:?}); not reproduced"),
            )
        });
    }
    if is_legacy_integer(s) {
        let cleaned: String = s.chars().filter(|c| *c != ',' && *c != '_').collect();
        return parse_ruby_integer(&cleaned)
            .map(Data::Integer)
            .ok_or_else(|| {
                SchemaError::unsupported(
                    "YAML integer",
                    format!("Psych raises ArgumentError for Integer({cleaned:?}); not reproduced"),
                )
            });
    }
    Ok(Data::String(s.to_owned()))
}

/// `/\.([Ee]|$)/` → `\1`, applied globally.
fn drop_dot_before_exponent(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == '.' {
            match chars.get(i + 1) {
                None | Some('e' | 'E') => continue,
                _ => {}
            }
        }
        out.push(*c);
    }
    out
}

fn digits_run(s: &[u8], mut i: usize, min: usize, max: usize) -> Option<usize> {
    let start = i;
    while i < s.len() && i - start < max && s[i].is_ascii_digit() {
        i += 1;
    }
    (i - start >= min).then_some(i)
}

/// Psych `TIME`:
/// `^-?\d{4}-\d{1,2}-\d{1,2}(?:[Tt]|\s+)\d{1,2}:\d\d:\d\d(?:\.\d*)?(?:\s*(?:Z|[-+]\d{1,2}:?(?:\d\d)?))?$`
fn is_time(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') {
        i += 1;
    }
    let Some(j) = digits_run(b, i, 4, 4) else {
        return false;
    };
    i = j;
    for _ in 0..2 {
        if b.get(i) != Some(&b'-') {
            return false;
        }
        let Some(j) = digits_run(b, i + 1, 1, 2) else {
            return false;
        };
        i = j;
    }
    match b.get(i) {
        Some(b'T' | b't') => i += 1,
        Some(c) if c.is_ascii_whitespace() => {
            while b.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
        }
        _ => return false,
    }
    let Some(j) = digits_run(b, i, 1, 2) else {
        return false;
    };
    i = j;
    for _ in 0..2 {
        if b.get(i) != Some(&b':') {
            return false;
        }
        let Some(j) = digits_run(b, i + 1, 2, 2) else {
            return false;
        };
        i = j;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    let rest = &s[i..];
    if rest.is_empty() {
        return true;
    }
    let zone = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if zone == "Z" {
        return true;
    }
    let zb = zone.as_bytes();
    if !matches!(zb.first(), Some(b'-' | b'+')) {
        return false;
    }
    let Some(mut k) = digits_run(zb, 1, 1, 2) else {
        return false;
    };
    if zb.get(k) == Some(&b':') {
        k += 1;
    }
    if k == zb.len() {
        return true;
    }
    digits_run(zb, k, 2, 2) == Some(zb.len())
}

/// Field ranges `Time.utc` accepts; outside them Psych keeps the string.
fn time_fields_valid(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let mut nums = body
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().unwrap_or(u32::MAX));
    let (_y, m, d, h, mi, se) = (
        nums.next(),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    );
    (1..=12).contains(&m) && (1..=31).contains(&d) && h <= 24 && mi <= 59 && se <= 60
}

/// Psych date: `^\d{4}-(?:1[012]|0\d|\d)-(?:[12]\d|3[01]|0\d|\d)$`.
fn date_parts(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() != 4 || !y.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let ok_m = match m.as_bytes() {
        [a] => a.is_ascii_digit(),
        [b'1', b'0'..=b'2'] | [b'0', b'0'..=b'9'] => true,
        _ => false,
    };
    let ok_d = match d.as_bytes() {
        [a] => a.is_ascii_digit(),
        [b'1' | b'2', b'0'..=b'9'] | [b'3', b'0' | b'1'] | [b'0', b'0'..=b'9'] => true,
        _ => false,
    };
    if !(ok_m && ok_d) {
        return None;
    }
    Some((y.parse().ok()?, m.parse().ok()?, d.parse().ok()?))
}

/// Sexagesimal integers and floats (`190:20:30`, `190:20:30.15`).
fn sexagesimal(s: &str) -> Option<Data> {
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (int_part, frac) = match body.find('.') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let parts: Vec<&str> = int_part.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let head = parts[0];
    if head.is_empty()
        || !head.as_bytes()[0].is_ascii_digit()
        || !head.bytes().all(|c| c.is_ascii_digit() || c == b'_')
    {
        return None;
    }
    for p in &parts[1..] {
        let ok = match p.as_bytes() {
            [a] => a.is_ascii_digit(),
            [a, b] => (b'0'..=b'5').contains(a) && b.is_ascii_digit(),
            _ => false,
        };
        if !ok {
            return None;
        }
    }
    if let Some(f) = frac {
        if !f.bytes().all(|c| c.is_ascii_digit() || c == b'_') {
            return None;
        }
    }
    // Ruby: each segment's to_i / to_f times 60 ** (e - 2).abs, summed,
    // where e is the segment index over the whole signed string.
    let segments: Vec<&str> = s.split(':').collect();
    if frac.is_none() {
        let mut total = BigInt::from(0);
        for (e, seg) in segments.iter().enumerate() {
            let v = ruby_to_i(seg);
            let pow = BigInt::from(60).pow((e as i64 - 2).unsigned_abs() as u32);
            total += v * pow;
        }
        Some(Data::Integer(total))
    } else {
        let mut total = 0.0f64;
        for (e, seg) in segments.iter().enumerate() {
            let v: f64 = ruby_to_f(seg);
            total += v * 60f64.powi((e as i32 - 2).abs());
        }
        Some(Data::Float(total))
    }
}

/// Ruby `String#to_i` for the restricted shapes sexagesimal produces.
fn ruby_to_i(s: &str) -> BigInt {
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits: String = body
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .filter(|c| *c != '_')
        .collect();
    let v: BigInt = digits.parse().unwrap_or_else(|_| BigInt::from(0));
    if neg {
        -v
    } else {
        v
    }
}

fn ruby_to_f(s: &str) -> f64 {
    let cleaned: String = s.chars().filter(|c| *c != '_').collect();
    let end = cleaned
        .char_indices()
        .take_while(|(i, c)| {
            c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))
        })
        .count();
    cleaned[..end].parse().unwrap_or(0.0)
}

/// Psych `FLOAT`: `^(?:[-+]?([0-9][0-9_,]*)?\.[0-9]*([eE][-+][0-9]+)?)$`.
fn is_float(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i = 1;
    }
    if b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while b
            .get(i)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'_' || *c == b',')
        {
            i += 1;
        }
    }
    if b.get(i) != Some(&b'.') {
        return false;
    }
    i += 1;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == b.len() {
        return true;
    }
    if !matches!(b.get(i), Some(b'e' | b'E')) || !matches!(b.get(i + 1), Some(b'-' | b'+')) {
        return false;
    }
    i += 2;
    let start = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    i > start && i == b.len()
}

/// Psych `INTEGER_LEGACY`.
fn is_legacy_integer(s: &str) -> bool {
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    let b = body.as_bytes();
    let rest_ok = |rest: &[u8], digit: fn(&u8) -> bool| {
        let mut seen = false;
        for c in rest {
            if digit(c) {
                seen = true;
            } else if *c == b'_' || *c == b',' {
                if !seen {
                    continue;
                }
            } else {
                return false;
            }
        }
        seen
    };
    if let Some(rest) = body.strip_prefix("0b") {
        // 0b[_,]*[0-1][0-1_,]*
        let r = rest.as_bytes();
        let first = r.iter().position(|c| *c != b'_' && *c != b',');
        return first.is_some_and(|p| matches!(r[p], b'0' | b'1'))
            && rest_ok(r, |c| matches!(c, b'0' | b'1'));
    }
    if let Some(rest) = body.strip_prefix("0x") {
        let r = rest.as_bytes();
        let first = r.iter().position(|c| *c != b'_' && *c != b',');
        return first.is_some_and(|p| r[p].is_ascii_hexdigit())
            && rest_ok(r, u8::is_ascii_hexdigit);
    }
    if b.first() == Some(&b'0') {
        if b.len() == 1 {
            return true;
        }
        // 0[_,]*[0-7][0-7_,]*
        let r = &b[1..];
        let first = r.iter().position(|c| *c != b'_' && *c != b',');
        return first.is_some_and(|p| (b'0'..=b'7').contains(&r[p]))
            && rest_ok(r, |c| (b'0'..=b'7').contains(c));
    }
    // [1-9](?:[0-9]|,[0-9]|_[0-9])*
    if !b.first().is_some_and(|c| (b'1'..=b'9').contains(c)) {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            i += 1;
        } else if (b[i] == b',' || b[i] == b'_') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            i += 2;
        } else {
            return false;
        }
    }
    true
}

/// Ruby `Integer(str)` for the cleaned legacy-integer shapes.
fn parse_ruby_integer(s: &str) -> Option<BigInt> {
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let v = if let Some(r) = body.strip_prefix("0b") {
        BigInt::parse_bytes(r.as_bytes(), 2)?
    } else if let Some(r) = body.strip_prefix("0x") {
        BigInt::parse_bytes(r.as_bytes(), 16)?
    } else if body.len() > 1 && body.starts_with('0') {
        BigInt::parse_bytes(&body.as_bytes()[1..], 8)?
    } else {
        BigInt::parse_bytes(body.as_bytes(), 10)?
    };
    Some(if neg { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> Data {
        tokenize(s, Permitted::DOCUMENT).unwrap()
    }

    #[test]
    fn yaml_11_scalars() {
        let int = |v: i64| Data::Integer(BigInt::from(v));
        assert_eq!(tok("yes"), Data::Bool(true));
        assert_eq!(tok("Off"), Data::Bool(false));
        assert_eq!(tok("~"), Data::Null);
        assert_eq!(tok("NULL"), Data::Null);
        assert_eq!(tok("nil"), Data::String("nil".into()));
        assert_eq!(tok("0755"), int(493));
        assert_eq!(tok("0x1F"), int(31));
        assert_eq!(tok("0b101"), int(5));
        assert_eq!(tok("1_000"), int(1000));
        assert_eq!(tok("1,000"), int(1000));
        assert_eq!(tok("09"), Data::String("09".into()));
        // Psych weights segment e by 60 ** (e - 2).abs regardless of count.
        assert_eq!(tok("1:30"), int(5400));
        assert_eq!(tok("1:30:15"), int(5415));
        assert_eq!(tok("1.0"), Data::Float(1.0));
        assert_eq!(tok("1."), Data::Float(1.0));
        assert_eq!(tok(".5"), Data::Float(0.5));
        assert_eq!(tok("1e3"), Data::String("1e3".into()));
        assert_eq!(tok("1.5e+3"), Data::Float(1500.0));
        assert_eq!(tok("1.0.0"), Data::String("1.0.0".into()));
        assert_eq!(tok(".inf"), Data::Float(f64::INFINITY));
        assert_eq!(tok(":sym"), Data::Symbol("sym".into()));
        assert_eq!(
            tok("2024-01-01"),
            Data::Date {
                year: 2024,
                month: 1,
                day: 1
            }
        );
        assert_eq!(tok("2024-02-30"), Data::String("2024-02-30".into()));
        assert_eq!(
            tok("2024-01-01T10:00:00Z"),
            Data::Time("2024-01-01T10:00:00Z".into())
        );
        assert_eq!(
            tokenize("2024-01-01", Permitted::NONE),
            Err(SchemaError::DisallowedClass {
                class: "Date".into()
            })
        );
    }
}
