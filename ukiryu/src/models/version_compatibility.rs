//! Source: lib/ukiryu/models/version_compatibility.rb
//! (`Ukiryu::VersionCompatibility`, `Ukiryu::VersionCompatibility::RequirementParser`).
use super::ruby::str_to_i;
use super::semantic_version::split_dots;
use std::cmp::Ordering;

/// The outcome of checking an installed version against a requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionCompatibility {
    pub installed_version: String,
    pub required_version: Option<String>,
    pub compatible: bool,
    pub reason: Option<String>,
}

impl VersionCompatibility {
    /// `VersionCompatibility.new(installed_version:, required_version:, compatible:, reason: nil)`.
    pub fn new(
        installed_version: &str,
        required_version: Option<&str>,
        compatible: bool,
        reason: Option<String>,
    ) -> Self {
        VersionCompatibility {
            installed_version: installed_version.to_owned(),
            required_version: required_version.map(str::to_owned),
            compatible,
            reason,
        }
    }

    /// `#compatible?`
    pub fn is_compatible(&self) -> bool {
        self.compatible
    }

    /// `#incompatible?`
    pub fn is_incompatible(&self) -> bool {
        !self.compatible
    }

    /// `#status_message`
    pub fn status_message(&self) -> String {
        let req = self.required_version.as_deref().unwrap_or("");
        if self.compatible {
            format!(
                "Version {} is compatible with requirement {req}",
                self.installed_version
            )
        } else {
            self.reason.clone().unwrap_or_else(|| {
                format!(
                    "Version {} is not compatible with requirement {req}",
                    self.installed_version
                )
            })
        }
    }

    /// `VersionCompatibility.check(installed_version, requirement)`.
    pub fn check(installed_version: &str, requirement: Option<&str>) -> Self {
        let Some(req) = requirement.filter(|r| !r.is_empty()) else {
            return Self::new(installed_version, requirement, true, None);
        };
        if RequirementParser::new(req).is_satisfied_by(installed_version) {
            Self::new(installed_version, Some(req), true, None)
        } else {
            Self::new(
                installed_version,
                Some(req),
                false,
                Some(format!(
                    "Version {installed_version} does not satisfy requirement: {req}"
                )),
            )
        }
    }
}

/// One `operator version` constraint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub operator: String,
    pub version: String,
}

/// Parses `">= 2.30, < 3.0"`-style requirements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequirementParser {
    pub requirement: String,
    pub constraints: Vec<Constraint>,
}

/// `/^([><=!~]+)\s*(.+)/` with Ruby backtracking: the operator run gives
/// back characters when nothing else would be left for `(.+)`.
fn parse_constraint(req: &str) -> Constraint {
    let ws: &[char] = &[' ', '\t', '\r', '\x0b', '\x0c'];
    for line in req.split('\n') {
        let op_len = line
            .bytes()
            .take_while(|b| matches!(b, b'>' | b'<' | b'=' | b'!' | b'~'))
            .count();
        for k in (1..=op_len).rev() {
            let rem = &line[k..];
            if rem.is_empty() {
                continue;
            }
            let trimmed = rem.trim_start_matches(ws);
            let version = if trimmed.is_empty() {
                &rem[rem.len() - 1..]
            } else {
                trimmed
            };
            return Constraint {
                operator: line[..k].to_owned(),
                version: version.to_owned(),
            };
        }
    }
    Constraint {
        operator: "==".into(),
        version: req.to_owned(),
    }
}

fn parse_version(version: &str) -> Vec<i128> {
    split_dots(version).into_iter().map(str_to_i).collect()
}

fn compare_versions(v1: &[i128], v2: &[i128]) -> Ordering {
    let n = v1.len().max(v2.len());
    let pad = |v: &[i128]| {
        let mut out = v.to_vec();
        out.resize(n, 0);
        out
    };
    pad(v1).cmp(&pad(v2))
}

impl RequirementParser {
    /// `RequirementParser.new(requirement)`.
    pub fn new(requirement: &str) -> Self {
        let constraints = split_dots_commas(requirement)
            .into_iter()
            .map(|r| parse_constraint(r.trim()))
            .collect();
        RequirementParser {
            requirement: requirement.to_owned(),
            constraints,
        }
    }

    /// `#satisfied_by?(version)`
    pub fn is_satisfied_by(&self, version: &str) -> bool {
        self.constraints.iter().all(|c| satisfied(version, c))
    }
}

/// `String#split(',')` (trailing empty fields dropped).
fn split_dots_commas(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(',').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

fn satisfied(version: &str, c: &Constraint) -> bool {
    let v = parse_version(version);
    let r = parse_version(&c.version);
    match c.operator.as_str() {
        ">" | ">=" => {
            compare_versions(&v, &r) == Ordering::Greater || (c.operator == ">=" && v == r)
        }
        "<" | "<=" => compare_versions(&v, &r) == Ordering::Less || (c.operator == "<=" && v == r),
        "==" | "=" => v == r,
        "!=" => v != r,
        "~>" => compare_versions(&v, &r) != Ordering::Less && v.first() == r.first(),
        _ => false,
    }
}
