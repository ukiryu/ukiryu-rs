//! Versionian 0.1.0 schemes reachable from `Ukiryu::VersionSchemeResolver`:
//! the built-in semantic, calver, solover and wendtver schemes and inline
//! calver and declarative declarations.
use super::declarative::{self, Segment};
use super::gem_version::GemVersion;
use super::ruby::{lines, no_method, strip, strip_build};
use super::{RangeError, RangeKind, SpecValue, VersionRange};
use num_bigint::BigInt;
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub(crate) enum Kind {
    Semantic,
    CalVer(CalVer),
    SoloVer,
    WendtVer,
    Declarative(Vec<Segment>),
}

/// The receiver as a `String`, or `NoMethodError` for `method`.
fn need_str<'a>(v: &'a SpecValue, method: &str) -> Result<&'a str, RangeError> {
    match v {
        SpecValue::Str(s) => Ok(s),
        other => Err(no_method(method, other)),
    }
}

fn digits(s: &str) -> BigInt {
    BigInt::parse_bytes(s.as_bytes(), 10).expect("ascii digits")
}

// ------------------------------------------------------------------ semantic

/// `Schemes::Semantic#parse` (only the `Gem::Version` matters for ranges).
fn semantic_parse(v: &str) -> Result<GemVersion, RangeError> {
    if strip(v).is_empty() {
        return Err(RangeError::invalid_version(
            "Version string cannot be empty",
        ));
    }
    GemVersion::new(&strip_build(v))
        .map_err(|m| RangeError::invalid_version(format!("Invalid semantic version '{v}': {m}")))
}

/// `::Gem::Version.new(boundary.gsub(/\+.*$/, ""))`: errors are raw `ArgumentError`s.
fn semantic_bound(b: &SpecValue) -> Result<GemVersion, RangeError> {
    let s = need_str(b, "gsub")?;
    GemVersion::new(&strip_build(s)).map_err(RangeError::argument)
}

/// `Semantic::SEMVER_PATTERN`: `/\A\d{1,5}\.\d{1,5}(?:\.\d{1,5})?(?:-[\w.]+)?(?:\+[\w.]+)?\z/`.
fn semver_pattern(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let num = |i: &mut usize| {
        let st = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        (1..=5).contains(&(*i - st))
    };
    let word = |i: &mut usize| {
        let st = *i;
        while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_' || b[*i] == b'.') {
            *i += 1;
        }
        *i > st
    };
    if !num(&mut i) || b.get(i) != Some(&b'.') {
        return false;
    }
    i += 1;
    if !num(&mut i) {
        return false;
    }
    if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        if !num(&mut i) {
            return false;
        }
    }
    if b.get(i) == Some(&b'-') {
        i += 1;
        if !word(&mut i) {
            return false;
        }
    }
    if b.get(i) == Some(&b'+') {
        i += 1;
        if !word(&mut i) {
            return false;
        }
    }
    i == b.len()
}

fn semantic_valid(v: &str) -> bool {
    if strip(v).is_empty() || !semver_pattern(v) {
        return false;
    }
    let first = v.split(['-', '.', '+']).next().unwrap_or_default();
    if first.len() == 4 {
        let year: u32 = first.parse().unwrap_or(0);
        if (1900..=2100).contains(&year) {
            return false;
        }
    }
    semantic_parse(v).is_ok()
}

fn semantic_matches(v: &str, r: &VersionRange<'_>) -> Result<bool, RangeError> {
    let g = semantic_parse(v)?;
    Ok(match r.kind {
        RangeKind::Equals => g.cmp(&semantic_bound(r.bound("version"))?) == Ordering::Equal,
        RangeKind::Before => g.cmp(&semantic_bound(r.bound("version"))?) == Ordering::Less,
        RangeKind::After => g.cmp(&semantic_bound(r.bound("version"))?) != Ordering::Less,
        RangeKind::Between => {
            g.cmp(&semantic_bound(r.bound("from"))?) != Ordering::Less
                && g.cmp(&semantic_bound(r.bound("to"))?) != Ordering::Greater
        }
    })
}

// ------------------------------------------------------------------ calver

#[derive(Clone, Debug)]
pub(crate) struct CalVer {
    pub(crate) format: String,
    pattern: &'static str,
    widths: &'static [usize],
    components: Vec<&'static str>,
}

impl CalVer {
    /// `Schemes::CalVer.new(format:)` after the `0M`/`0W` normalisation.
    pub(crate) fn new(format: &str) -> CalVer {
        let format = format.replace("0M", "MM").replace("0W", "WW");
        let (pattern, widths): (&'static str, &'static [usize]) = match format.as_str() {
            "YYYY.MM" | "YYYY.WW" => (r"^(\d{4})\.(\d{2})$", &[4, 2]),
            "YY.MM.DD" => (r"^(\d{2})\.(\d{2})\.(\d{2})$", &[2, 2, 2]),
            "YY.MM" | "YY.WW" => (r"^(\d{2})\.(\d{2})$", &[2, 2]),
            _ => (r"^(\d{4})\.(\d{2})\.(\d{2})$", &[4, 2, 2]),
        };
        let mut components = vec!["year"];
        for (marker, name) in [("MM", "month"), ("DD", "day"), ("WW", "week")] {
            if format.contains(marker) {
                components.push(name);
            }
        }
        CalVer {
            format,
            pattern,
            widths,
            components,
        }
    }

    fn captures<'a>(&self, s: &'a str) -> Option<Vec<&'a str>> {
        lines(s).find_map(|line| {
            let mut caps = Vec::new();
            let mut rest = line;
            for (n, w) in self.widths.iter().enumerate() {
                if n > 0 {
                    rest = rest.strip_prefix('.')?;
                }
                let d = rest.get(..*w)?;
                if !d.bytes().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                caps.push(d);
                rest = &rest[*w..];
            }
            rest.is_empty().then_some(caps)
        })
    }

    /// `Schemes::Pattern#parse` with this format's pattern; returns the comparable array.
    fn parse(&self, v: &SpecValue) -> Result<Vec<BigInt>, RangeError> {
        let s = need_str(v, "strip")?;
        if strip(s).is_empty() {
            return Err(RangeError::invalid_version(
                "Version string cannot be empty",
            ));
        }
        let caps = self.captures(s).ok_or_else(|| {
            RangeError::parse(format!(
                "Version '{s}' does not match pattern {}",
                self.pattern
            ))
        })?;
        let mut out = Vec::new();
        for (i, name) in self.components.iter().enumerate() {
            let raw = caps.get(i).ok_or_else(|| {
                RangeError::parse(format!("Required component '{name}' is missing"))
            })?;
            out.push(declarative::date_part(
                raw,
                Some(&SpecValue::Sym((*name).into())),
            )?);
        }
        Ok(out)
    }

    /// `Schemes::Pattern#matches_range?`: only `==` exists on Ruby arrays.
    fn matches(&self, v: &str, r: &VersionRange<'_>) -> Result<bool, RangeError> {
        let a = self.parse(&SpecValue::Str(v.into()))?;
        let array = SpecValue::Array(Vec::new());
        match r.kind {
            RangeKind::Equals => Ok(a == self.parse(r.bound("version"))?),
            RangeKind::Before => {
                self.parse(r.bound("version"))?;
                Err(no_method("<", &array))
            }
            RangeKind::After => {
                self.parse(r.bound("version"))?;
                Err(no_method(">=", &array))
            }
            RangeKind::Between => {
                self.parse(r.bound("from"))?;
                Err(no_method(">=", &array))
            }
        }
    }
}

// ------------------------------------------------------------------ solover / wendtver

fn validate_nonempty(v: &SpecValue) -> Result<&str, RangeError> {
    if matches!(v, SpecValue::Nil) {
        return Err(RangeError::invalid_version("Version string cannot be nil"));
    }
    let s = need_str(v, "strip")?;
    if strip(s).is_empty() {
        return Err(RangeError::invalid_version(
            "Version string cannot be empty",
        ));
    }
    Ok(s)
}

#[derive(Debug)]
struct Solo {
    number: BigInt,
    postfix: Option<(u8, String)>,
}

/// `/^(\d+)(([+-])([a-zA-Z0-9]+))?$/` on the first matching line.
fn solo_parse(v: &SpecValue) -> Result<Solo, RangeError> {
    let s = validate_nonempty(v)?;
    lines(s)
        .find_map(|line| {
            let end = line
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(line.len());
            if end == 0 {
                return None;
            }
            let number = digits(&line[..end]);
            let rest = &line[end..];
            if rest.is_empty() {
                return Some(Solo {
                    number,
                    postfix: None,
                });
            }
            let order = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => 2,
                _ => return None,
            };
            let ident = &rest[1..];
            (!ident.is_empty() && ident.bytes().all(|c| c.is_ascii_alphanumeric())).then(|| Solo {
                number,
                postfix: Some((order, ident.to_string())),
            })
        })
        .ok_or_else(|| RangeError::parse(format!("Invalid SoloVer format '{s}'")))
}

fn solo_cmp(a: &Solo, b: &Solo) -> Ordering {
    a.number
        .cmp(&b.number)
        .then_with(|| match (&a.postfix, &b.postfix) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some((pa, ia)), Some((pb, ib))) => {
                pa.cmp(pb).then_with(|| ia.as_bytes().cmp(ib.as_bytes()))
            }
        })
}

/// `/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/` plus `validate_ranges`.
fn wendt_parse(v: &SpecValue) -> Result<Vec<BigInt>, RangeError> {
    let s = validate_nonempty(v)?;
    let parts = lines(s)
        .find_map(|line| {
            let p: Vec<&str> = line.split('.').collect();
            (p.len() == 4
                && p.iter()
                    .all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit())))
            .then_some(p)
        })
        .ok_or_else(|| RangeError::parse(format!("Invalid WendtVer format '{s}'")))?;
    let n: Vec<BigInt> = parts.iter().map(|p| digits(p)).collect();
    for (i, max, label) in [
        (1, 99, "Minor must be 0-99"),
        (2, 99, "Patch must be 0-99"),
        (3, 999, "Build must be 0-999"),
    ] {
        if n[i] > BigInt::from(max) {
            return Err(RangeError::parse(label));
        }
    }
    Ok(n)
}

/// SoloVer/WendtVer `matches_range?`: every present boundary is parsed first.
fn parse_all<T>(
    v: &str,
    r: &VersionRange<'_>,
    parse: impl Fn(&SpecValue) -> Result<T, RangeError>,
    cmp: impl Fn(&T, &T) -> Ordering,
) -> Result<bool, RangeError> {
    let a = parse(&SpecValue::Str(v.into()))?;
    let parsed = |key: &str| -> Result<Option<T>, RangeError> {
        match r.bounds.get(key) {
            Some(b) if b.truthy() => parse(b).map(Some),
            _ => Ok(None),
        }
    };
    let b = parsed("version")?;
    let from = parsed("from")?;
    let to = parsed("to")?;
    let c = |x: &Option<T>| cmp(&a, x.as_ref().expect("validated boundary"));
    Ok(match r.kind {
        RangeKind::Equals => c(&b) == Ordering::Equal,
        RangeKind::Before => c(&b) == Ordering::Less,
        RangeKind::After => c(&b) != Ordering::Less,
        RangeKind::Between => c(&from) != Ordering::Less && c(&to) != Ordering::Greater,
    })
}

// ------------------------------------------------------------------ dispatch

impl Kind {
    pub(crate) fn class_name(&self) -> &'static str {
        match self {
            Kind::Semantic => "Versionian::Schemes::Semantic",
            Kind::CalVer(_) => "Versionian::Schemes::CalVer",
            Kind::SoloVer => "Versionian::Schemes::SoloVer",
            Kind::WendtVer => "Versionian::Schemes::WendtVer",
            Kind::Declarative(_) => "Versionian::Schemes::Declarative",
        }
    }

    /// `scheme.valid?(version_string)`.
    pub(crate) fn valid(&self, v: &str) -> Result<bool, RangeError> {
        let rescued = |r: Result<(), RangeError>| match r {
            Ok(()) => Ok(true),
            Err(e) if e.class.is_parse_or_invalid_version() => Ok(false),
            Err(e) => Err(e),
        };
        let sv = SpecValue::Str(v.into());
        match self {
            Kind::Semantic => Ok(semantic_valid(v)),
            Kind::CalVer(c) => rescued(c.parse(&sv).map(|_| ())),
            Kind::SoloVer => rescued(solo_parse(&sv).map(|_| ())),
            Kind::WendtVer => rescued(wendt_parse(&sv).map(|_| ())),
            Kind::Declarative(segs) => rescued(declarative::parse(segs, v)),
        }
    }

    /// `scheme.matches_range?(version_string, range)`.
    pub(crate) fn matches(&self, v: &str, r: &VersionRange<'_>) -> Result<bool, RangeError> {
        match self {
            Kind::Semantic => semantic_matches(v, r),
            Kind::CalVer(c) => c.matches(v, r),
            Kind::SoloVer => parse_all(v, r, solo_parse, solo_cmp),
            Kind::WendtVer => parse_all(v, r, wendt_parse, |a, b| a.cmp(b)),
            Kind::Declarative(segs) => {
                declarative::parse(segs, v)?;
                Err(RangeError::name(
                    "uninitialized constant Versionian::VersionRange::Version",
                ))
            }
        }
    }
}
