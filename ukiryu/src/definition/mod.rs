//! Tool definition loading, discovery, composition and caching.
//!
//! Source: `lib/ukiryu/definition.rb` and `lib/ukiryu/definition/*` at the
//! pinned Ruby oracle. The concrete tool document model
//! (`Ukiryu::Models::ToolDefinition`) is owned by the models module; this
//! module reaches it only through [`loader::DefinitionDocument`] and the
//! [`composer`] traits so the two stay separate.
//!
//! Linter and documentation generator modules are added next to these by
//! their own owners (additive `pub mod` lines only).
pub mod cache;
pub mod composer;
pub mod definition_validator;
pub mod discovery;
pub mod documentation;
pub mod lint_issue;
pub mod linter;
pub mod loader;
pub mod metadata;
pub mod source;
pub mod sources;
pub mod validation_result;
pub mod version_resolver;

pub use cache::{CacheStats, CachedDefinition, DefinitionCache, RefreshStrategy};
pub use composer::{
    ComposableCommand, ComposableDefinition, ComposableProfile, DefinitionComposer, MergeStrategy,
};
pub use definition_validator::{SchemaStage, StageIssues, ValidationReport, ValidatorEnv};
pub use discovery::{Discovery, DiscoveryEnv};
pub use lint_issue::{LintIssue, LintIssueHash, Severity};
pub use loader::{DefinitionDocument, DocumentParseError, Loader, ValidationMode};
pub use metadata::{DefinitionMetadata, SourceKind};
pub use source::{Source, SourceType};
pub use sources::{FileSource, StringSource};
pub use validation_result::ValidationResult;
pub use version_resolver::{Constraint, Operator, VersionResolver};

use std::fmt;

/// Ruby exception class an error corresponds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefinitionErrorKind {
    /// `Ukiryu::Errors::DefinitionNotFoundError`
    NotFound,
    /// `Ukiryu::Errors::DefinitionLoadError`
    Load,
    /// `Ukiryu::Errors::DefinitionValidationError`
    Validation,
    /// `ArgumentError`
    Argument,
    /// `NoMethodError` raised by the Ruby code paths ported as-is.
    NoMethod,
}

/// An error raised by the definition layer, carrying the Ruby class and the
/// exact Ruby message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionError {
    pub kind: DefinitionErrorKind,
    pub message: String,
}

impl DefinitionError {
    pub fn new(kind: DefinitionErrorKind, message: impl Into<String>) -> Self {
        DefinitionError {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(DefinitionErrorKind::NotFound, message)
    }

    pub(crate) fn load(message: impl Into<String>) -> Self {
        Self::new(DefinitionErrorKind::Load, message)
    }

    pub(crate) fn no_method(message: impl Into<String>) -> Self {
        Self::new(DefinitionErrorKind::NoMethod, message)
    }

    /// Fully qualified Ruby class name.
    pub fn ruby_class(&self) -> &'static str {
        match self.kind {
            DefinitionErrorKind::NotFound => "Ukiryu::Errors::DefinitionNotFoundError",
            DefinitionErrorKind::Load => "Ukiryu::Errors::DefinitionLoadError",
            DefinitionErrorKind::Validation => "Ukiryu::Errors::DefinitionValidationError",
            DefinitionErrorKind::Argument => "ArgumentError",
            DefinitionErrorKind::NoMethod => "NoMethodError",
        }
    }

    /// `Ukiryu::Errors::Error#suggestions` for the definition error classes.
    pub fn suggestions(&self) -> &'static [&'static str] {
        match self.kind {
            DefinitionErrorKind::NotFound => &[
                "Verify file path is correct",
                "Check file has .yaml extension",
                "Use absolute path if relative path fails",
            ],
            DefinitionErrorKind::Load => &[
                "Validate YAML syntax",
                "Check file is readable",
                "Verify file encoding is UTF-8",
            ],
            DefinitionErrorKind::Validation => &[
                "Run 'ukiryu validate' for detailed errors",
                "Compare with schema definition",
                "Check tool definition examples",
            ],
            DefinitionErrorKind::Argument | DefinitionErrorKind::NoMethod => &[],
        }
    }
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DefinitionError {}

pub type Result<T> = std::result::Result<T, DefinitionError>;

/// Ruby `String#strip`: removes leading/trailing ASCII whitespace and NUL.
pub(crate) fn ruby_strip(s: &str) -> &str {
    let ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\0');
    s.trim_matches(ws)
}

/// Ruby `String#split(sep)` with a literal one-character separator: trailing
/// empty fields are removed, and an empty string yields no fields.
pub(crate) fn ruby_split(s: &str, sep: char) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(sep).collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

/// Ruby `String#inspect` for UTF-8 strings.
pub(crate) fn ruby_inspect_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\x0c' => out.push_str("\\f"),
            '\x0b' => out.push_str("\\v"),
            '\x08' => out.push_str("\\b"),
            '\x07' => out.push_str("\\a"),
            '\x1b' => out.push_str("\\e"),
            '#' if matches!(chars.peek(), Some('{') | Some('$') | Some('@')) => out.push_str("\\#"),
            '\x7f' => out.push_str("\\x7F"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
