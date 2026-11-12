//! `Gem::Version` (rubygems 4.0.3, Ruby 4.0.1), which Versionian's semantic
//! scheme uses for parsing and ordering. This is not strict SemVer:
//! "2.43.0.windows.1" is accepted and "1.0.0-alpha" becomes "1.0.0.pre.alpha".
use super::ruby::{is_regex_space, strip};
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    /// Decimal digits with leading zeros removed ("" is zero).
    Num(String),
    Str(String),
}

impl Segment {
    fn is_zero(&self) -> bool {
        matches!(self, Segment::Num(d) if d.is_empty())
    }
}

fn cmp_num(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

#[derive(Clone, Debug)]
pub(crate) struct GemVersion {
    version: String,
    canonical: Vec<Segment>,
}

/// `Gem::Version.correct?`: `/\A\s*([0-9]+(?>\.[0-9a-zA-Z]+)*(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?)?\s*\z/`.
fn correct(s: &str) -> bool {
    let core = s.trim_matches(is_regex_space);
    if core.is_empty() {
        return true;
    }
    let b = core.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let st = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > st
    };
    if !digits(&mut i) {
        return false;
    }
    while i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_alphanumeric() {
        i += 1;
        while i < b.len() && b[i].is_ascii_alphanumeric() {
            i += 1;
        }
    }
    if i == b.len() {
        return true;
    }
    if b[i] != b'-' {
        return false;
    }
    i += 1;
    let pre = |c: u8| c.is_ascii_alphanumeric() || c == b'-';
    let st = i;
    while i < b.len() && pre(b[i]) {
        i += 1;
    }
    if i == st {
        return false;
    }
    while i < b.len() {
        if b[i] != b'.' {
            return false;
        }
        i += 1;
        let st = i;
        while i < b.len() && pre(b[i]) {
            i += 1;
        }
        if i == st {
            return false;
        }
    }
    true
}

/// `ver.scan(/\d+|[a-z]+/i)` mapped to numbers and strings.
fn partition(v: &str) -> Vec<Segment> {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let st = i;
        if b[i].is_ascii_digit() {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Segment::Num(v[st..i].trim_start_matches('0').to_string()));
        } else if b[i].is_ascii_alphabetic() {
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            out.push(Segment::Str(v[st..i].to_string()));
        } else {
            i += 1;
        }
    }
    out
}

/// `@version.sub(/(?<=[a-zA-Z.])[.0]+\z/, "")`.
fn drop_trailing_zeros(v: &str) -> String {
    let b = v.as_bytes();
    let mut start = b.len();
    while start > 0 && (b[start - 1] == b'.' || b[start - 1] == b'0') {
        start -= 1;
    }
    for p in start..b.len() {
        if p > 0 && (b[p - 1].is_ascii_alphabetic() || b[p - 1] == b'.') {
            return v[..p].to_string();
        }
    }
    v.to_string()
}

/// `canonical_version.sub!(/(?<=\.|\A)[0.]+(?=[a-zA-Z])/, "")`.
fn drop_zeros_before_letters(v: &str) -> String {
    let b = v.as_bytes();
    for p in 0..b.len() {
        if !(p == 0 || b[p - 1] == b'.') || !(b[p] == b'0' || b[p] == b'.') {
            continue;
        }
        let mut e = p;
        while e < b.len() && (b[e] == b'0' || b[e] == b'.') {
            e += 1;
        }
        if e < b.len() && b[e].is_ascii_alphabetic() {
            return format!("{}{}", &v[..p], &v[e..]);
        }
    }
    v.to_string()
}

impl GemVersion {
    /// `Gem::Version.new(string)`; the error is the ArgumentError message.
    pub(crate) fn new(input: &str) -> Result<GemVersion, String> {
        if !correct(input) {
            return Err(format!("Malformed version number string {input}"));
        }
        let version = if input.chars().all(is_regex_space) {
            "0".to_string()
        } else {
            strip(input).replace('-', ".pre.")
        };
        let prerelease = version.bytes().any(|c| c.is_ascii_alphabetic());
        let mut canonical = drop_trailing_zeros(&version);
        if prerelease {
            canonical = drop_zeros_before_letters(&canonical);
        }
        Ok(GemVersion {
            canonical: partition(&canonical),
            version,
        })
    }

    /// `Gem::Version#<=>` against another `Gem::Version`.
    pub(crate) fn cmp(&self, other: &GemVersion) -> Ordering {
        if self.version == other.version || self.canonical == other.canonical {
            return Ordering::Equal;
        }
        let (l, r) = (&self.canonical, &other.canonical);
        let limit = l.len().min(r.len());
        for i in 0..limit {
            match (&l[i], &r[i]) {
                (a, b) if a == b => continue,
                (Segment::Str(_), Segment::Num(_)) => return Ordering::Less,
                (Segment::Num(_), Segment::Str(_)) => return Ordering::Greater,
                (Segment::Num(a), Segment::Num(b)) => return cmp_num(a, b),
                (Segment::Str(a), Segment::Str(b)) => return a.as_bytes().cmp(b.as_bytes()),
            }
        }
        if l.len() <= limit {
            for s in &r[limit..] {
                if matches!(s, Segment::Str(_)) {
                    return Ordering::Greater;
                }
                if !s.is_zero() {
                    return Ordering::Less;
                }
            }
        } else {
            for s in &l[limit..] {
                if matches!(s, Segment::Str(_)) {
                    return Ordering::Less;
                }
                if !s.is_zero() {
                    return Ordering::Greater;
                }
            }
        }
        Ordering::Equal
    }
}
