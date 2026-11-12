//! Version ranges and version-scheme resolution.
//!
//! Port of `Ukiryu::VersionSchemeResolver` (lib/ukiryu/version_scheme_resolver.rb),
//! the range selection in `Ukiryu::Tool::Loader#find_matching_version_spec`
//! (lib/ukiryu/tool/loader.rb) and the parts of the versionian 0.1.0 gem they
//! reach: `VersionRange`, `SchemeRegistry`, `SchemeLoader.from_hash` and the
//! semantic, calver, solover, wendtver and declarative schemes.
//!
//! Outcomes follow the pinned Ruby source, including its errors: a calver
//! `before`/`after`/`between` range raises `NoMethodError` (Ruby arrays have no
//! `<`/`>=`), a calver field such as "08" raises `ArgumentError` (`Integer("08")`
//! is octal), and every declarative range raises `NameError` after parsing.
//! The semantic scheme orders with `Gem::Version`, not strict SemVer. These
//! semantics are distinct from `Ukiryu::Models::SemanticVersion` and
//! `VersionCompatibility`, which are separate APIs.
//!
//! Inline `pattern` and `composite` schemes need Ruby's regexp engine and
//! array ordering of mixed values; they return [`ErrorClass::Unsupported`].
mod declarative;
mod gem_version;
mod ruby;
mod schemes;

use num_bigint::BigInt;
use schemes::{CalVer, Kind};
use std::fmt;

/// The Ruby exception class a source operation raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    /// `Versionian::Errors::InvalidVersionError`
    InvalidVersion,
    /// `Versionian::Errors::ParseError`
    Parse,
    /// `Versionian::Errors::InvalidSchemeError`
    InvalidScheme,
    /// `ArgumentError`
    Argument,
    /// `NoMethodError`
    NoMethod,
    /// `NameError`
    Name,
    /// `TypeError`
    Type,
    /// Reachable in Ruby but not ported; never a parity claim.
    Unsupported,
}

impl ErrorClass {
    /// The Ruby class name (`"Unsupported"` for the non-Ruby variant).
    pub fn ruby_name(self) -> &'static str {
        match self {
            ErrorClass::InvalidVersion => "Versionian::Errors::InvalidVersionError",
            ErrorClass::Parse => "Versionian::Errors::ParseError",
            ErrorClass::InvalidScheme => "Versionian::Errors::InvalidSchemeError",
            ErrorClass::Argument => "ArgumentError",
            ErrorClass::NoMethod => "NoMethodError",
            ErrorClass::Name => "NameError",
            ErrorClass::Type => "TypeError",
            ErrorClass::Unsupported => "Unsupported",
        }
    }

    fn is_parse_or_invalid_version(self) -> bool {
        matches!(self, ErrorClass::Parse | ErrorClass::InvalidVersion)
    }
}

/// An error with the source's class and message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeError {
    pub class: ErrorClass,
    pub message: String,
}

impl RangeError {
    fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        RangeError {
            class,
            message: message.into(),
        }
    }
    fn invalid_version(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::InvalidVersion, m)
    }
    fn parse(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::Parse, m)
    }
    fn invalid_scheme(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::InvalidScheme, m)
    }
    fn argument(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::Argument, m)
    }
    fn no_method(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::NoMethod, m)
    }
    fn name(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::Name, m)
    }
    fn type_error(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::Type, m)
    }
    fn unsupported(m: impl Into<String>) -> Self {
        Self::new(ErrorClass::Unsupported, m)
    }
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class.ruby_name(), self.message)
    }
}

impl std::error::Error for RangeError {}

/// The Ruby value shapes a scheme specification or version spec can hold.
/// Typed-model integration converts loaded YAML into this input type.
#[derive(Clone, Debug, PartialEq)]
pub enum SpecValue {
    Nil,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(String),
    Sym(String),
    Array(Vec<SpecValue>),
    /// A Hash in insertion order.
    Map(Vec<(SpecValue, SpecValue)>),
}

static NIL: SpecValue = SpecValue::Nil;

impl SpecValue {
    /// Ruby truthiness: everything except `nil` and `false`.
    pub fn truthy(&self) -> bool {
        !matches!(self, SpecValue::Nil | SpecValue::Bool(false))
    }

    /// The Ruby class name.
    pub fn ruby_class(&self) -> &'static str {
        match self {
            SpecValue::Nil => "NilClass",
            SpecValue::Bool(true) => "TrueClass",
            SpecValue::Bool(false) => "FalseClass",
            SpecValue::Int(_) => "Integer",
            SpecValue::Float(_) => "Float",
            SpecValue::Str(_) => "String",
            SpecValue::Sym(_) => "Symbol",
            SpecValue::Array(_) => "Array",
            SpecValue::Map(_) => "Hash",
        }
    }

    /// `Hash#[]` with `eql?` keys; `None` for non-Hash receivers.
    fn map_get(&self, key: &SpecValue) -> Option<&SpecValue> {
        match self {
            SpecValue::Map(pairs) => pairs.iter().find(|(k, _)| eql(k, key)).map(|(_, v)| v),
            _ => None,
        }
    }

    fn str_key(&self, key: &str) -> Option<&SpecValue> {
        self.map_get(&SpecValue::Str(key.into()))
    }

    /// `recv[key]` for the receivers a version spec can hold.
    fn index(&self, key: &SpecValue) -> Result<&SpecValue, RangeError> {
        match self {
            SpecValue::Map(_) => Ok(self.map_get(key).unwrap_or(&NIL)),
            SpecValue::Str(_) | SpecValue::Array(_) | SpecValue::Int(_) => {
                Err(RangeError::type_error(format!(
                    "no implicit conversion of {} into Integer",
                    key.ruby_class()
                )))
            }
            SpecValue::Nil => Err(ruby::no_method("[]", self)),
            other => Err(RangeError::unsupported(format!(
                "indexing a {} is not ported",
                other.ruby_class()
            ))),
        }
    }

    /// `recv[:key] || recv['key']`.
    fn either(&self, key: &str) -> Result<&SpecValue, RangeError> {
        let v = self.index(&SpecValue::Sym(key.into()))?;
        if v.truthy() {
            Ok(v)
        } else {
            self.index(&SpecValue::Str(key.into()))
        }
    }

    /// `#inspect` (Ruby 3.4+ Hash format).
    pub fn ruby_inspect(&self) -> String {
        match self {
            SpecValue::Nil => "nil".into(),
            SpecValue::Bool(b) => b.to_string(),
            SpecValue::Int(n) => n.to_string(),
            SpecValue::Float(f) => ruby::float_to_s(*f),
            SpecValue::Str(s) => ruby::inspect_str(s),
            SpecValue::Sym(s) if simple_symbol(s) => format!(":{s}"),
            SpecValue::Sym(s) => format!(":{}", ruby::inspect_str(s)),
            SpecValue::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|v| v.ruby_inspect())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            SpecValue::Map(pairs) if pairs.is_empty() => "{}".into(),
            SpecValue::Map(pairs) => {
                let body: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| match k {
                        SpecValue::Sym(s) if simple_symbol(s) => {
                            format!("{s}: {}", v.ruby_inspect())
                        }
                        SpecValue::Sym(s) => {
                            format!("{}: {}", ruby::inspect_str(s), v.ruby_inspect())
                        }
                        k => format!("{} => {}", k.ruby_inspect(), v.ruby_inspect()),
                    })
                    .collect();
                format!("{{{}}}", body.join(", "))
            }
        }
    }

    /// `"#{value}"`.
    fn ruby_to_s(&self) -> String {
        match self {
            SpecValue::Nil => String::new(),
            SpecValue::Str(s) | SpecValue::Sym(s) => s.clone(),
            SpecValue::Int(n) => n.to_string(),
            SpecValue::Float(f) => ruby::float_to_s(*f),
            SpecValue::Bool(b) => b.to_string(),
            other => other.ruby_inspect(),
        }
    }
}

fn simple_symbol(s: &str) -> bool {
    let mut c = s.chars();
    let body = s.trim_end_matches(['?', '!', '=']);
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && body
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && s.len() - body.len() <= 1
}

/// `Object#eql?` for Hash keys (1 and 1.0 differ, "a" and :a differ).
fn eql(a: &SpecValue, b: &SpecValue) -> bool {
    match (a, b) {
        (SpecValue::Float(x), SpecValue::Float(y)) => x.to_bits() == y.to_bits() || x == y,
        (SpecValue::Array(x), SpecValue::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eql(p, q))
        }
        (SpecValue::Map(x), SpecValue::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| b.map_get(k).is_some_and(|w| eql(v, w)))
                && y.iter().all(|(k, _)| a.map_get(k).is_some())
        }
        _ => a == b,
    }
}

/// A resolved `Versionian::VersionScheme`.
#[derive(Clone, Debug)]
pub struct Scheme {
    name: Option<String>,
    description: SpecValue,
    kind: Kind,
}

impl Scheme {
    /// `Versionian.get_scheme(name)` over the built-in registry.
    pub fn builtin(name: &str) -> Result<Scheme, RangeError> {
        let (kind, description) = match name {
            "semantic" => (
                Kind::Semantic,
                "Semantic versioning (semver.org)".to_string(),
            ),
            "calver" => (
                Kind::CalVer(CalVer::new("YYYY.MM.DD")),
                "Calendar versioning (YYYY.MM.DD)".to_string(),
            ),
            "solover" => (
                Kind::SoloVer,
                "SoloVer single number with optional postfix".to_string(),
            ),
            "wendtver" => (
                Kind::WendtVer,
                "WendtVer auto-incrementing with carryover".to_string(),
            ),
            _ => {
                return Err(RangeError::invalid_scheme(format!(
                    "Unknown scheme: {name}"
                )))
            }
        };
        Ok(Scheme {
            name: Some(name.to_string()),
            description: SpecValue::Str(description),
            kind,
        })
    }

    /// `scheme.name` (a Symbol in Ruby; `None` for nil).
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `scheme.description` (any value an inline declaration supplied).
    pub fn description(&self) -> &SpecValue {
        &self.description
    }

    /// The Versionian class, e.g. `"Versionian::Schemes::Semantic"`.
    pub fn class_name(&self) -> &'static str {
        self.kind.class_name()
    }

    /// `CalVer#format` after normalisation; `None` for other schemes.
    pub fn format(&self) -> Option<&str> {
        match &self.kind {
            Kind::CalVer(c) => Some(&c.format),
            _ => None,
        }
    }

    /// `scheme.valid?(version_string)`.
    pub fn is_valid(&self, version: &str) -> Result<bool, RangeError> {
        self.kind.valid(version)
    }
}

/// `VersionSchemeResolver.resolve(scheme_spec)`.
pub fn resolve(spec: &SpecValue) -> Result<Scheme, RangeError> {
    match spec {
        SpecValue::Map(_) => from_hash(spec),
        SpecValue::Str(name) | SpecValue::Sym(name) => Scheme::builtin(name),
        other => Err(RangeError::argument(format!(
            "Invalid scheme specification: {}",
            other.ruby_inspect()
        ))),
    }
}

/// `VersionSchemeResolver.inline?`.
pub fn is_inline(spec: &SpecValue) -> bool {
    matches!(spec, SpecValue::Map(_))
}

/// `VersionSchemeResolver.reference?`.
pub fn is_reference(spec: &SpecValue) -> bool {
    matches!(spec, SpecValue::Str(_) | SpecValue::Sym(_))
}

/// `Versionian::SchemeLoader.from_hash` (string keys only, as in the source).
fn from_hash(data: &SpecValue) -> Result<Scheme, RangeError> {
    let get = |k: &str| data.str_key(k);
    let ty = ruby::opt_to_sym(get("type"))?;
    let description = get("description").cloned().unwrap_or(SpecValue::Nil);
    match ty.as_deref() {
        Some("declarative") => {
            let name = ruby::opt_to_sym(get("name"))?;
            let segments = declarative::segments_from(get("components"))?;
            Ok(Scheme {
                name,
                description,
                kind: Kind::Declarative(segments),
            })
        }
        Some("calver") => {
            let format = match get("format") {
                Some(f) if f.truthy() => f.clone(),
                _ => SpecValue::Str("YYYY.MM.DD".into()),
            };
            let name = ruby::opt_to_sym(get("name"))?.unwrap_or_else(|| "calver".into());
            let SpecValue::Str(format) = format else {
                return Err(ruby::no_method("gsub", &format));
            };
            let calver = CalVer::new(&format);
            let description = if description.truthy() {
                description
            } else {
                SpecValue::Str(format!("Calendar versioning ({})", calver.format))
            };
            Ok(Scheme {
                name: Some(name),
                description,
                kind: Kind::CalVer(calver),
            })
        }
        Some(t @ ("pattern" | "composite")) => Err(RangeError::unsupported(format!(
            "inline {t} schemes are not ported"
        ))),
        _ => Err(RangeError::invalid_scheme(format!(
            "Unknown scheme type: {}",
            get("type").unwrap_or(&NIL).ruby_to_s()
        ))),
    }
}

/// `Versionian::VersionRange` range types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    Equals,
    Before,
    After,
    Between,
}

impl RangeKind {
    /// The range type for a symbol name; other names fail as in `VersionRange#validate!`.
    pub fn from_name(name: &str) -> Result<RangeKind, RangeError> {
        Ok(match name {
            "equals" => RangeKind::Equals,
            "before" => RangeKind::Before,
            "after" => RangeKind::After,
            "between" => RangeKind::Between,
            other => return Err(RangeError::argument(format!("Unknown range type: {other}"))),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RangeKind::Equals => "equals",
            RangeKind::Before => "before",
            RangeKind::After => "after",
            RangeKind::Between => "between",
        }
    }
}

/// Range boundaries: the `version:`, `from:` and `to:` keywords.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bounds {
    entries: Vec<(String, SpecValue)>,
}

impl Bounds {
    pub fn new() -> Bounds {
        Bounds::default()
    }

    /// Add a keyword boundary (a later value for the same key replaces it).
    pub fn with(mut self, key: &str, value: SpecValue) -> Bounds {
        self.entries.retain(|(k, _)| k != key);
        self.entries.push((key.to_string(), value));
        self
    }

    pub fn get(&self, key: &str) -> Option<&SpecValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn truthy(&self, key: &str) -> bool {
        self.get(key).is_some_and(SpecValue::truthy)
    }
}

/// `Versionian::VersionRange`.
#[derive(Clone, Debug)]
pub struct VersionRange<'s> {
    kind: RangeKind,
    scheme: &'s Scheme,
    bounds: Bounds,
}

impl<'s> VersionRange<'s> {
    /// `VersionRange.new(type, scheme, **boundaries)`.
    pub fn new(kind: RangeKind, scheme: &'s Scheme, bounds: Bounds) -> Result<Self, RangeError> {
        match kind {
            RangeKind::Between if !(bounds.truthy("from") && bounds.truthy("to")) => {
                return Err(RangeError::argument("between range requires :from and :to"))
            }
            RangeKind::Equals | RangeKind::Before | RangeKind::After
                if !bounds.truthy("version") =>
            {
                return Err(RangeError::argument(format!(
                    "{} range requires :version",
                    kind.as_str()
                )))
            }
            _ => {}
        }
        Ok(VersionRange {
            kind,
            scheme,
            bounds,
        })
    }

    pub fn kind(&self) -> RangeKind {
        self.kind
    }

    pub fn bounds(&self) -> &Bounds {
        &self.bounds
    }

    fn bound(&self, key: &str) -> &SpecValue {
        self.bounds.get(key).unwrap_or(&NIL)
    }

    /// `range.matches?(version_string)`.
    pub fn matches(&self, version: &str) -> Result<bool, RangeError> {
        self.scheme.kind.matches(version, self)
    }
}

/// `Tool::Loader#find_matching_version_spec(versions, detected_version, scheme)`:
/// the index of the first version spec whose range matches.
pub fn find_matching_version_spec(
    versions: &[SpecValue],
    detected_version: &str,
    scheme: &Scheme,
) -> Result<Option<usize>, RangeError> {
    for (i, spec) in versions.iter().enumerate() {
        let mut kind = None;
        for k in [
            RangeKind::Equals,
            RangeKind::Before,
            RangeKind::After,
            RangeKind::Between,
        ] {
            if spec.either(k.as_str())?.truthy() {
                kind = Some(k);
                break;
            }
        }
        let Some(kind) = kind else { continue };
        let bounds = if kind == RangeKind::Between {
            let between = spec.either("between")?;
            Bounds::new()
                .with("from", between.either("from")?.clone())
                .with("to", between.either("to")?.clone())
        } else {
            Bounds::new().with("version", spec.either(kind.as_str())?.clone())
        };
        if VersionRange::new(kind, scheme, bounds)?.matches(detected_version)? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}
