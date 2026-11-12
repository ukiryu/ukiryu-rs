//! A single linting issue.
//!
//! Source: `lib/ukiryu/definition/lint_issue.rb` (`Ukiryu::Definition::LintIssue`).
//! The linter itself lives in its own module; this is the shared issue type.
use serde::Serialize;

/// Issue severity (a Ruby Symbol in the source).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Style,
    /// Any other symbol given to `LintIssue.new(severity:)`.
    Other(String),
}

impl Severity {
    /// `SEVERITY_ERROR`, `SEVERITY_WARNING`, `SEVERITY_INFO`, `SEVERITY_STYLE`.
    pub const ALL: [Severity; 4] = [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Style,
    ];

    /// The symbol name (`severity.to_s`).
    pub fn as_str(&self) -> &str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Style => "style",
            Severity::Other(s) => s,
        }
    }
}

/// `Ukiryu::Definition::LintIssue`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintIssue {
    pub severity: Severity,
    pub message: String,
    pub location: Option<String>,
    pub suggestion: Option<String>,
    pub rule_id: Option<String>,
}

/// `LintIssue#to_h`, in Ruby key order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LintIssueHash {
    pub severity: String,
    pub severity_string: String,
    pub message: String,
    pub location: Option<String>,
    pub suggestion: Option<String>,
    pub rule_id: Option<String>,
}

impl LintIssue {
    pub fn new(
        severity: Severity,
        message: impl Into<String>,
        location: Option<String>,
        suggestion: Option<String>,
        rule_id: Option<String>,
    ) -> Self {
        LintIssue {
            severity,
            message: message.into(),
            location,
            suggestion,
            rule_id,
        }
    }

    pub fn error(
        message: impl Into<String>,
        location: Option<String>,
        suggestion: Option<String>,
        rule_id: Option<String>,
    ) -> Self {
        Self::new(Severity::Error, message, location, suggestion, rule_id)
    }

    pub fn warning(
        message: impl Into<String>,
        location: Option<String>,
        suggestion: Option<String>,
        rule_id: Option<String>,
    ) -> Self {
        Self::new(Severity::Warning, message, location, suggestion, rule_id)
    }

    pub fn info(
        message: impl Into<String>,
        location: Option<String>,
        suggestion: Option<String>,
        rule_id: Option<String>,
    ) -> Self {
        Self::new(Severity::Info, message, location, suggestion, rule_id)
    }

    pub fn style(
        message: impl Into<String>,
        location: Option<String>,
        suggestion: Option<String>,
        rule_id: Option<String>,
    ) -> Self {
        Self::new(Severity::Style, message, location, suggestion, rule_id)
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn is_warning(&self) -> bool {
        self.severity == Severity::Warning
    }

    pub fn is_info(&self) -> bool {
        self.severity == Severity::Info
    }

    pub fn is_style(&self) -> bool {
        self.severity == Severity::Style
    }

    /// `has_suggestion?`: present and non-empty.
    pub fn has_suggestion(&self) -> bool {
        self.suggestion.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// `has_location?`: present and non-empty.
    pub fn has_location(&self) -> bool {
        self.location.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// `severity_string`: the upcased symbol name.
    pub fn severity_string(&self) -> String {
        self.severity.as_str().to_uppercase()
    }

    pub fn to_h(&self) -> LintIssueHash {
        LintIssueHash {
            severity: self.severity.as_str().to_string(),
            severity_string: self.severity_string(),
            message: self.message.clone(),
            location: self.location.clone(),
            suggestion: self.suggestion.clone(),
            rule_id: self.rule_id.clone(),
        }
    }
}

impl std::fmt::Display for LintIssue {
    /// `LintIssue#to_s`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.severity_string(), self.message)?;
        if self.has_location() {
            write!(f, " (at {})", self.location.as_deref().unwrap_or_default())?;
        }
        if self.has_suggestion() {
            write!(
                f,
                "\n  Suggestion: {}",
                self.suggestion.as_deref().unwrap_or_default()
            )?;
        }
        Ok(())
    }
}
