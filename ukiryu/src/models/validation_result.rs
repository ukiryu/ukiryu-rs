//! Source: lib/ukiryu/models/validation_result.rb (`Ukiryu::Models::ValidationResult`).

/// The result of validating one tool definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationResult {
    pub tool_name: String,
    pub valid: bool,
    pub errors: Vec<String>,
}

impl ValidationResult {
    pub fn new(tool_name: &str, valid: bool, errors: Vec<String>) -> Self {
        ValidationResult {
            tool_name: tool_name.to_owned(),
            valid,
            errors,
        }
    }

    /// `ValidationResult.valid(tool_name)`
    pub fn valid(tool_name: &str) -> Self {
        Self::new(tool_name, true, Vec::new())
    }

    /// `ValidationResult.invalid(tool_name, errors)`
    pub fn invalid(tool_name: &str, errors: Vec<String>) -> Self {
        Self::new(tool_name, false, errors)
    }

    /// `ValidationResult.not_found(tool_name)`
    pub fn not_found(tool_name: &str) -> Self {
        Self::new(tool_name, false, vec!["Tool not found".into()])
    }

    /// `#status_message`
    pub fn status_message(&self) -> String {
        if self.valid {
            "\u{2713} Valid".into()
        } else {
            let n = self.errors.len();
            let plural = if n == 1 { "" } else { "s" };
            format!("\u{2717} Invalid ({n} error{plural})")
        }
    }

    /// `#valid?`
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// `#invalid?`
    pub fn is_invalid(&self) -> bool {
        !self.valid
    }

    /// `#not_found?`
    pub fn is_not_found(&self) -> bool {
        self.errors == ["Tool not found"]
    }
}
