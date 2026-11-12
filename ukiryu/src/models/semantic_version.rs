//! Source: lib/ukiryu/models/semantic_version.rb (`Ukiryu::Models::SemanticVersion`).
use super::ruby::str_to_i;
use std::cmp::Ordering;

/// Numeric dotted version comparison (`"1.10" > "1.9"`).
#[derive(Clone, Debug)]
pub struct SemanticVersion {
    pub original: String,
    pub segments: Vec<i128>,
}

/// The right-hand side accepted by `SemanticVersion#<=>`.
#[derive(Clone, Copy, Debug)]
pub enum VersionOperand<'a> {
    Version(&'a SemanticVersion),
    Str(&'a str),
    Int(i128),
}

/// `String#split('.')`: trailing empty fields are dropped.
pub(crate) fn split_dots(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split('.').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

fn cmp_padded(a: &[i128], b: &[i128]) -> Ordering {
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

impl SemanticVersion {
    /// `SemanticVersion.parse(version_string)` (`nil`/empty → `[0]`).
    pub fn parse(version_string: Option<&str>) -> Vec<i128> {
        match version_string {
            None | Some("") => vec![0],
            Some(s) => split_dots(s).into_iter().map(str_to_i).collect(),
        }
    }

    /// `SemanticVersion.compare(a, b)` → -1, 0 or 1.
    pub fn compare(a: Option<&str>, b: Option<&str>) -> i32 {
        match cmp_padded(&Self::parse(a), &Self::parse(b)) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }

    /// `SemanticVersion.new(version_string)` (`nil.to_s` is `""`).
    pub fn new(version_string: Option<&str>) -> SemanticVersion {
        let original = version_string.unwrap_or("").to_owned();
        let segments = Self::parse(Some(&original));
        SemanticVersion { original, segments }
    }

    /// `#<=>(other)`; `None` for `nil`.
    pub fn cmp_with(&self, other: Option<VersionOperand<'_>>) -> Option<Ordering> {
        let other_segments = match other? {
            VersionOperand::Version(v) => v.segments.clone(),
            VersionOperand::Str(s) => Self::parse(Some(s)),
            VersionOperand::Int(i) => Self::parse(Some(&i.to_string())),
        };
        Some(cmp_padded(&self.segments, &other_segments))
    }

    /// `#==`
    pub fn eq_with(&self, other: Option<VersionOperand<'_>>) -> bool {
        other.is_some() && self.cmp_with(other) == Some(Ordering::Equal)
    }

    /// `#>`
    pub fn gt(&self, other: Option<VersionOperand<'_>>) -> bool {
        self.cmp_with(other) == Some(Ordering::Greater)
    }

    /// `#<`
    pub fn lt(&self, other: Option<VersionOperand<'_>>) -> bool {
        self.cmp_with(other) == Some(Ordering::Less)
    }

    /// `#>=`
    pub fn ge(&self, other: Option<VersionOperand<'_>>) -> bool {
        matches!(
            self.cmp_with(other),
            Some(Ordering::Greater | Ordering::Equal)
        )
    }

    /// `#<=`
    pub fn le(&self, other: Option<VersionOperand<'_>>) -> bool {
        matches!(self.cmp_with(other), Some(Ordering::Less | Ordering::Equal))
    }

    /// `#to_s` (normalized segments).
    pub fn to_ruby_s(&self) -> String {
        self.segments
            .iter()
            .map(i128::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }

    /// `#inspect`
    pub fn inspect(&self) -> String {
        format!("#<Ukiryu::Models::SemanticVersion {}>", self.to_ruby_s())
    }

    /// `#eql?` (same segments, unpadded).
    pub fn eql(&self, other: &SemanticVersion) -> bool {
        self.segments == other.segments
    }
}

impl PartialEq for SemanticVersion {
    fn eq(&self, other: &Self) -> bool {
        self.eq_with(Some(VersionOperand::Version(other)))
    }
}

impl PartialOrd for SemanticVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.cmp_with(Some(VersionOperand::Version(other)))
    }
}
