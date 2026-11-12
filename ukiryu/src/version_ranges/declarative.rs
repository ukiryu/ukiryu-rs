//! `Versionian::Schemes::Declarative` with `Versionian::Parsers::Declarative`
//! and the component types it dispatches to. Only acceptance and error
//! outcomes are modelled: Versionian 0.1.0 never compares declarative values
//! in a range, because `VersionRange#includes?` raises `NameError` first.
use super::ruby::{kernel_float, kernel_integer, opt_to_sym, to_sym};
use super::{RangeError, SpecValue};
use num_bigint::BigInt;

#[derive(Clone, Debug)]
pub(crate) struct Segment {
    name: Option<String>,
    ctype: Option<String>,
    subtype: Option<SpecValue>,
    values: Vec<String>,
    optional: bool,
    prefix: Option<Vec<char>>,
    /// `prefix: false` passes the nil check but is never used as a marker.
    prefix_is_false: bool,
    separator: Option<Vec<char>>,
    include_prefix_in_value: bool,
}

/// A `prefix`/`separator` value: nil and false are absent, strings are kept.
fn marker(v: Option<&SpecValue>, field: &str) -> Result<Option<Vec<char>>, RangeError> {
    match v {
        None | Some(SpecValue::Nil) | Some(SpecValue::Bool(false)) => Ok(None),
        Some(SpecValue::Str(s)) => Ok(Some(s.chars().collect())),
        Some(other) => Err(RangeError::unsupported(format!(
            "declarative component {field} of class {} is not ported",
            other.ruby_class()
        ))),
    }
}

fn symbol_list(v: Option<&SpecValue>, field: &str) -> Result<Vec<String>, RangeError> {
    match v {
        None | Some(SpecValue::Nil) | Some(SpecValue::Bool(false)) => Ok(Vec::new()),
        Some(SpecValue::Array(items)) => items.iter().map(to_sym).collect(),
        Some(other) => Err(RangeError::unsupported(format!(
            "declarative component {field} of class {} is not ported",
            other.ruby_class()
        ))),
    }
}

/// `SchemeLoader.parse_component_definitions` followed by
/// `ComponentDefinition.from_hash` and `Parsers::Declarative#validate_definitions!`.
pub(crate) fn segments_from(components: Option<&SpecValue>) -> Result<Vec<Segment>, RangeError> {
    let items = match components {
        None | Some(SpecValue::Nil) | Some(SpecValue::Bool(false)) => return Ok(Vec::new()),
        Some(SpecValue::Array(items)) => items,
        Some(other) => {
            return Err(RangeError::unsupported(format!(
                "declarative components of class {} are not ported",
                other.ruby_class()
            )))
        }
    };
    let mut segs = Vec::new();
    for comp in items {
        if !matches!(comp, SpecValue::Map(_)) {
            return Err(RangeError::unsupported(format!(
                "declarative component of class {} is not ported",
                comp.ruby_class()
            )));
        }
        let get = |k: &str| comp.str_key(k);
        let name = opt_to_sym(get("name"))?;
        let ctype = opt_to_sym(get("type"))?;
        let subtype = get("subtype").cloned();
        let values = symbol_list(get("values"), "values")?;
        symbol_list(get("order"), "order")?;
        segs.push(Segment {
            name,
            ctype,
            subtype,
            values,
            optional: get("optional").is_some_and(SpecValue::truthy),
            separator: marker(get("separator"), "separator")?,
            prefix: marker(get("prefix"), "prefix")?,
            prefix_is_false: matches!(get("prefix"), Some(SpecValue::Bool(false))),
            include_prefix_in_value: get("include_prefix_in_value").is_some_and(SpecValue::truthy),
        });
    }
    for s in &segs {
        if s.name.is_none() {
            return Err(RangeError::invalid_scheme("segment name required"));
        }
        if s.ctype.is_none() {
            return Err(RangeError::invalid_scheme("segment type required"));
        }
    }
    for (i, s) in segs.iter().enumerate() {
        if s.optional && i > 0 && s.prefix.is_none() && !s.prefix_is_false {
            return Err(RangeError::invalid_scheme(format!(
                "Optional segment '{}' must have prefix or separator",
                s.name.as_deref().unwrap_or_default()
            )));
        }
    }
    Ok(segs)
}

/// `Range` of Versionian's `DatePart::RANGES`.
fn date_range(subtype: &str) -> Option<(i64, i64)> {
    Some(match subtype {
        "year" => (1, 9999),
        "month" => (1, 12),
        "day" => (1, 31),
        "week" => (1, 53),
        "hour" => (0, 23),
        "minute" => (0, 59),
        "second" => (0, 59),
        _ => return None,
    })
}

/// `ComponentTypes::DatePart.parse` for a non-empty value. `Kernel#Integer`
/// errors are not rescued, so they surface as `ArgumentError`.
pub(crate) fn date_part(raw: &str, subtype: Option<&SpecValue>) -> Result<BigInt, RangeError> {
    let n = kernel_integer(raw).map_err(RangeError::argument)?;
    let subtype = match subtype {
        None | Some(SpecValue::Nil) | Some(SpecValue::Bool(false)) => return Ok(n),
        Some(v) => to_sym(v)?,
    };
    if let Some((min, max)) = date_range(&subtype) {
        if n < BigInt::from(min) || n > BigInt::from(max) {
            return Err(RangeError::parse(format!(
                "Invalid {subtype} '{n}'. Must be between {min} and {max}"
            )));
        }
    }
    Ok(n)
}

fn parse_component(seg: &Segment, raw: &str) -> Result<(), RangeError> {
    let ctype = seg.ctype.as_deref().unwrap_or_default();
    match ctype {
        "integer" => kernel_integer(raw)
            .map(|_| ())
            .map_err(|m| RangeError::parse(format!("Invalid integer '{raw}': {m}"))),
        "float" => {
            kernel_float(raw).map_err(|m| RangeError::parse(format!("Invalid float '{raw}': {m}")))
        }
        "enum" => {
            if !seg.values.is_empty() && !seg.values.iter().any(|v| v == raw) {
                return Err(RangeError::parse(format!(
                    "Invalid enum value '{raw}' for {}. Allowed: {}",
                    seg.name.as_deref().unwrap_or_default(),
                    seg.values.join(", ")
                )));
            }
            Ok(())
        }
        "date_part" => date_part(raw, seg.subtype.as_ref()).map(|_| ()),
        "string" | "prerelease" | "postfix" | "hash" => Ok(()),
        other => Err(RangeError::invalid_scheme(format!(
            "Unknown component type: {other}"
        ))),
    }
}

fn at(s: &[char], pos: usize, pat: &[char]) -> bool {
    pos <= s.len() && s[pos..].starts_with(pat)
}

/// `String#index(pattern, pos)` on characters.
fn index_from(s: &[char], pat: &[char], pos: usize) -> Option<usize> {
    if pos > s.len() {
        return None;
    }
    (pos..=s.len()).find(|&p| s[p..].starts_with(pat))
}

fn non_empty(m: &Option<Vec<char>>) -> Option<&[char]> {
    m.as_deref().filter(|m| !m.is_empty())
}

/// `Parsers::Declarative#parse`.
pub(crate) fn parse(segs: &[Segment], input: &str) -> Result<(), RangeError> {
    if input.is_empty() {
        return Err(RangeError::parse("Version string cannot be empty"));
    }
    let s: Vec<char> = input.chars().collect();
    let len = s.len();
    let mut pos = 0;
    for (idx, seg) in segs.iter().enumerate() {
        if let (true, Some(prefix)) = (seg.optional, seg.prefix.as_deref()) {
            if pos >= len {
                continue;
            }
            let pl = prefix.len();
            let at_pos = at(&s, pos, prefix);
            let before = pos >= pl && at(&s, pos - pl, prefix);
            let like = seg.include_prefix_in_value && (s[pos] == '+' || s[pos] == '-');
            if !(at_pos || before || like) {
                continue;
            }
            if at_pos && !seg.include_prefix_in_value {
                pos += pl;
            }
        }
        let end = segment_end(segs, &s, pos, idx);
        let raw: String = s[pos..end].iter().collect();
        if raw.is_empty() {
            if seg.optional {
                continue;
            }
            return Err(RangeError::parse(format!(
                "Required segment '{}' is missing or empty",
                seg.name.as_deref().unwrap_or_default()
            )));
        }
        parse_component(seg, &raw)?;
        pos = end;
        if let Some(sep) = non_empty(&seg.separator) {
            if pos < len && at(&s, pos, sep) {
                pos += sep.len();
            }
        }
    }
    if pos < len {
        let rest: String = s[pos..].iter().collect();
        return Err(RangeError::parse(format!(
            "Unexpected trailing content after parsing: '{rest}'"
        )));
    }
    Ok(())
}

/// `Parsers::Declarative#find_segment_end_position`.
fn segment_end(segs: &[Segment], s: &[char], pos: usize, idx: usize) -> usize {
    let mut best = s.len();
    let mut cand = |p: usize| best = best.min(p);
    if let Some(sep) = non_empty(&segs[idx].separator) {
        if let Some(p) = index_from(s, sep, pos) {
            cand(p);
        }
    }
    for next in &segs[idx + 1..] {
        if let Some(sep) = non_empty(&next.separator) {
            if let Some(p) = index_from(s, sep, pos) {
                cand(p);
            }
        }
        if let Some(prefix) = non_empty(&next.prefix) {
            let mut found = false;
            if let Some(p) = index_from(s, prefix, pos) {
                cand(p);
                found = true;
            }
            if next.include_prefix_in_value {
                for alt in ['+', '-'] {
                    if prefix != [alt] {
                        if let Some(p) = index_from(s, &[alt], pos) {
                            cand(p);
                            found = true;
                        }
                    }
                }
            }
            if found {
                break;
            }
        }
    }
    best
}
