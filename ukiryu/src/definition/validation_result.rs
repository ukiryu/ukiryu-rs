//! Result of validating a definition.
//!
//! Source: `lib/ukiryu/definition/validation_result.rb`
//! (`Ukiryu::Definition::ValidationResult`). Distinct from the models'
//! register `ValidationResult`.
use serde::Serialize;

/// `Ukiryu::Definition::ValidationResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationResult {
    valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub schema_path: Option<String>,
}

#[derive(Serialize)]
struct ValidationResultHash<'a> {
    valid: bool,
    errors: &'a [String],
    warnings: &'a [String],
    schema_path: Option<&'a str>,
    error_count: usize,
    warning_count: usize,
}

impl ValidationResult {
    /// `ValidationResult.new(valid:, errors: [], warnings: [], schema_path: nil)`.
    pub fn new(
        valid: bool,
        errors: Vec<String>,
        warnings: Vec<String>,
        schema_path: Option<String>,
    ) -> Self {
        ValidationResult {
            valid,
            errors,
            warnings,
            schema_path,
        }
    }

    pub fn success() -> Self {
        Self::new(true, Vec::new(), Vec::new(), None)
    }

    pub fn failure(errors: Vec<String>, warnings: Vec<String>) -> Self {
        Self::new(false, errors, warnings, None)
    }

    pub fn with_warnings(warnings: Vec<String>) -> Self {
        Self::new(true, Vec::new(), warnings, None)
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn is_invalid(&self) -> bool {
        !self.valid
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn issue_count(&self) -> usize {
        self.errors.len() + self.warnings.len()
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// `to_json` (Ruby `to_h.to_json`, key order preserved).
    pub fn to_json(&self) -> String {
        serde_json::to_string(&ValidationResultHash {
            valid: self.valid,
            errors: &self.errors,
            warnings: &self.warnings,
            schema_path: self.schema_path.as_deref(),
            error_count: self.error_count(),
            warning_count: self.warning_count(),
        })
        .expect("strings serialize")
    }

    /// `summary`.
    pub fn summary(&self) -> String {
        if self.valid {
            if self.has_warnings() {
                format!("Valid with {} warning(s)", self.warning_count())
            } else {
                "Valid".to_string()
            }
        } else {
            let mut msg = format!("Invalid ({} error(s)", self.error_count());
            if self.has_warnings() {
                msg.push_str(&format!(", {} warning(s)", self.warning_count()));
            }
            msg + ")"
        }
    }
}

impl std::fmt::Display for ValidationResult {
    /// `to_s`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = vec![format!("Validation: {}", self.summary())];
        if self.has_errors() {
            out.push(String::new());
            out.push("Errors:".to_string());
            for (i, e) in self.errors.iter().enumerate() {
                out.push(format!("  {}. {}", i + 1, e));
            }
        }
        if self.has_warnings() {
            out.push(String::new());
            out.push("Warnings:".to_string());
            for (i, w) in self.warnings.iter().enumerate() {
                out.push(format!("  {}. {}", i + 1, w));
            }
        }
        f.write_str(&out.join("\n"))
    }
}
