//! Semantic version constraints.
//!
//! Source: `lib/ukiryu/definition/version_resolver.rb`
//! (`Ukiryu::Definition::VersionResolver`). Version components follow Ruby
//! `String#split('.')` + `String#to_i`; exact (`==`) matches compare the
//! unpadded component arrays while ordering comparisons pad with zeros.
use super::{ruby_split, ruby_strip, DefinitionError, Result};
use num_bigint::BigInt;
use std::cmp::Ordering;

/// Constraint operator (`Constraint#operator`, a Ruby Symbol).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Operator {
    Eq,
    Ge,
    Gt,
    Le,
    Lt,
    Pessimistic,
    /// Any other symbol passed to `Constraint.new`; never satisfied.
    Other(String),
}

impl Operator {
    pub fn as_str(&self) -> &str {
        match self {
            Operator::Eq => "==",
            Operator::Ge => ">=",
            Operator::Gt => ">",
            Operator::Le => "<=",
            Operator::Lt => "<",
            Operator::Pessimistic => "~>",
            Operator::Other(s) => s,
        }
    }
}

/// `VersionResolver::Constraint`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Constraint {
    pub operator: Operator,
    pub version: String,
    pub raw: String,
}

impl Constraint {
    /// `Constraint.new(operator, version, raw = nil)`.
    pub fn new(operator: Operator, version: impl Into<String>, raw: Option<String>) -> Self {
        let version = version.into();
        let raw = raw.unwrap_or_else(|| format!("{} {}", operator.as_str(), version));
        Constraint {
            operator,
            version,
            raw,
        }
    }

    pub fn exact(version: impl Into<String>) -> Self {
        let version = version.into();
        Self::new(Operator::Eq, version.clone(), Some(version))
    }

    pub fn min(version: impl Into<String>) -> Self {
        Self::new(Operator::Ge, version, None)
    }

    pub fn max(version: impl Into<String>) -> Self {
        Self::new(Operator::Le, version, None)
    }

    pub fn pessimistic(version: impl Into<String>) -> Self {
        Self::new(Operator::Pessimistic, version, None)
    }

    /// `Constraint.range(min, max)` returns `[>= min, < max]`.
    pub fn range(min_version: impl Into<String>, max_version: impl Into<String>) -> Vec<Self> {
        vec![
            Self::new(Operator::Ge, min_version, None),
            Self::new(Operator::Lt, max_version, None),
        ]
    }
}

impl std::fmt::Display for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// `Ukiryu::Definition::VersionResolver` class methods.
pub struct VersionResolver;

fn is_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// Ruby `\s*(.+)` anchored at the start of `rest` (with backtracking);
/// returns the capture. `.` does not match `\n`.
fn space_then_rest(rest: &str) -> Option<&str> {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            rest.char_indices()
                .take_while(|(_, c)| is_regex_space(*c))
                .map(|(i, c)| i + c.len_utf8()),
        )
        .collect();
    for &start in starts.iter().rev() {
        let tail = &rest[start..];
        let end = tail.find('\n').unwrap_or(tail.len());
        if end > 0 {
            return Some(&tail[..end]);
        }
    }
    None
}

/// Ruby `String#to_i` (base 10): leading whitespace, optional sign, digits
/// with single underscores between digits; anything else ends the number.
pub(crate) fn ruby_to_i(s: &str) -> BigInt {
    let s = s.trim_start_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r'));
    let (negative, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let bytes = body.as_bytes();
    let mut digits = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            digits.push(b as char);
        } else if b == b'_'
            && !digits.is_empty()
            && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
        {
            // single underscore between digits
        } else {
            break;
        }
        i += 1;
    }
    if digits.is_empty() {
        return BigInt::from(0);
    }
    let value: BigInt = digits.parse().expect("ascii digits");
    if negative {
        -value
    } else {
        value
    }
}

/// Ruby `Array#<=>` on integer arrays (lexicographic; a strict prefix is smaller).
fn array_cmp(a: &[BigInt], b: &[BigInt]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        match x.cmp(y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

fn nil_plus() -> DefinitionError {
    DefinitionError::no_method("undefined method '+' for nil")
}

/// Ruby `Enumerable#max_by` by version components: the first maximal element wins.
fn max_by_version<S: AsRef<str>>(versions: &[S]) -> Option<String> {
    let mut best: Option<(&S, Vec<BigInt>)> = None;
    for v in versions {
        let parts = VersionResolver::parse_version(v.as_ref());
        match &best {
            Some((_, bp)) if array_cmp(&parts, bp) != Ordering::Greater => {}
            _ => best = Some((v, parts)),
        }
    }
    best.map(|(v, _)| v.as_ref().to_string())
}

impl VersionResolver {
    /// `VersionResolver.parse_constraint(constraint_string)`.
    pub fn parse_constraint(constraint_string: &str) -> Vec<Constraint> {
        if !constraint_string.contains(['<', '>', '=', '~']) {
            return vec![Constraint::exact(constraint_string)];
        }
        ruby_split(constraint_string, ',')
            .into_iter()
            .map(ruby_strip)
            .map(Self::parse_part)
            .collect()
    }

    fn parse_part(part: &str) -> Constraint {
        if let Some(rest) = part.strip_prefix("~>") {
            let version = space_then_rest(rest).unwrap_or("");
            return Constraint::pessimistic(ruby_strip(version));
        }
        let ordered: [(&str, fn(&str) -> Constraint); 5] = [
            (">=", |v| Constraint::min(v)),
            (">", |v| Constraint::new(Operator::Gt, v, None)),
            ("<=", |v| Constraint::max(v)),
            ("<", |v| Constraint::new(Operator::Lt, v, None)),
            ("==", |v| Constraint::exact(v)),
        ];
        for (prefix, build) in ordered {
            if let Some(version) = part.strip_prefix(prefix).and_then(space_then_rest) {
                return build(version);
            }
        }
        Constraint::exact(ruby_strip(part))
    }

    /// `VersionResolver.satisfies?(version, constraint_string)`.
    pub fn satisfies(version: &str, constraint: &str) -> Result<bool> {
        Self::satisfies_all(version, &Self::parse_constraint(constraint))
    }

    /// `VersionResolver.satisfies?(version, [Constraint, ...])`.
    pub fn satisfies_all(version: &str, constraints: &[Constraint]) -> Result<bool> {
        let v = Self::parse_version(version);
        for c in constraints {
            let ok = match &c.operator {
                Operator::Eq => v == Self::parse_version(&c.version),
                Operator::Ge => Self::compare_parts(&v, &Self::parse_version(&c.version)).is_ge(),
                Operator::Gt => Self::compare_parts(&v, &Self::parse_version(&c.version)).is_gt(),
                Operator::Le => Self::compare_parts(&v, &Self::parse_version(&c.version)).is_le(),
                Operator::Lt => Self::compare_parts(&v, &Self::parse_version(&c.version)).is_lt(),
                Operator::Pessimistic => {
                    let base = Self::parse_version(&c.version);
                    let (last, init) = base.split_last().ok_or_else(nil_plus)?;
                    let mut upper = init.to_vec();
                    upper.push(last + 1);
                    upper.push(BigInt::from(0));
                    Self::compare_parts(&v, &base).is_ge()
                        && Self::compare_parts(&v, &upper).is_lt()
                }
                Operator::Other(_) => false,
            };
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `VersionResolver.resolve(constraint, available_versions)`.
    pub fn resolve<S: AsRef<str>>(
        constraint: &str,
        available: Option<&[S]>,
    ) -> Result<Option<String>> {
        let Some(available) = available.filter(|a| !a.is_empty()) else {
            return Ok(None);
        };
        let constraints = Self::parse_constraint(constraint);
        let mut matching = Vec::new();
        for v in available {
            if Self::satisfies_all(v.as_ref(), &constraints)? {
                matching.push(v.as_ref());
            }
        }
        Ok(max_by_version(&matching))
    }

    /// `VersionResolver.compare_versions(v1, v2)` (zero-padded, -1/0/1).
    pub fn compare_versions(v1: &str, v2: &str) -> Ordering {
        Self::compare_parts(&Self::parse_version(v1), &Self::parse_version(v2))
    }

    /// `compare_versions` on already parsed component arrays.
    pub fn compare_parts(a: &[BigInt], b: &[BigInt]) -> Ordering {
        let zero = BigInt::from(0);
        for i in 0..a.len().max(b.len()) {
            let x = a.get(i).unwrap_or(&zero);
            let y = b.get(i).unwrap_or(&zero);
            match x.cmp(y) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        Ordering::Equal
    }

    /// `VersionResolver.parse_version(version_string)`.
    pub fn parse_version(version: &str) -> Vec<BigInt> {
        ruby_split(version, '.')
            .into_iter()
            .map(ruby_to_i)
            .collect()
    }

    /// `VersionResolver.latest(versions)`.
    pub fn latest<S: AsRef<str>>(versions: Option<&[S]>) -> Option<String> {
        versions.and_then(max_by_version)
    }

    /// `VersionResolver.pessimistic_range(version)` -> `[min, max]`.
    pub fn pessimistic_range(version: &str) -> Result<(String, String)> {
        let parts = Self::parse_version(version);
        let (last, init) = parts.split_last().ok_or_else(nil_plus)?;
        let init: Vec<String> = init.iter().map(ToString::to_string).collect();
        Ok((
            version.to_string(),
            format!("{}.{}", init.join("."), last + 1),
        ))
    }
}
