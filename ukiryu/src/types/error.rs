//! Errors raised by type validation and constraints, keyed by the Ruby
//! exception class the pinned implementation raises.
use std::fmt;

use super::value::Value;

/// The Ruby exception class of an error. Messages are the exact Ruby messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// `Ukiryu::Errors::ValidationError`
    Validation,
    /// `Ukiryu::Validation::ValidationIssue` (constraint failures)
    ValidationIssue,
    /// Ruby `ArgumentError` (for example a failed numeric comparison)
    Argument,
    /// Ruby `TypeError`
    Type,
    /// Ruby `FloatDomainError` (`Integer(Float::NAN)`)
    FloatDomain,
    /// Ruby `NoMethodError`
    NoMethod,
    /// Ruby `NameError` (invalid instance variable name)
    Name,
    /// Ruby `SystemStackError` (unbounded `of: :array` recursion)
    SystemStack,
    /// `URI::InvalidComponentError` (raised by `URI::MailTo`, not rescued by `Type`)
    UriInvalidComponent,
    /// The input needs Ruby behaviour this port does not reproduce. Never a
    /// Ruby class: callers must treat it as "not decided", not as a rejection.
    Unsupported,
}

impl ErrorClass {
    /// The Ruby class name, as the conformance corpus records it.
    pub fn ruby_name(self) -> &'static str {
        match self {
            ErrorClass::Validation => "Ukiryu::Errors::ValidationError",
            ErrorClass::ValidationIssue => "Ukiryu::Validation::ValidationIssue",
            ErrorClass::Argument => "ArgumentError",
            ErrorClass::Type => "TypeError",
            ErrorClass::FloatDomain => "FloatDomainError",
            ErrorClass::NoMethod => "NoMethodError",
            ErrorClass::Name => "NameError",
            ErrorClass::SystemStack => "SystemStackError",
            ErrorClass::UriInvalidComponent => "URI::InvalidComponentError",
            ErrorClass::Unsupported => "Unsupported",
        }
    }
}

/// A raised error. `attribute` and `validation_type` are set for
/// `ValidationIssue` only (`ValidationIssue#attribute_name`/`#validation_type`).
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub class: ErrorClass,
    pub message: String,
    pub attribute: Option<Value>,
    pub validation_type: Option<&'static str>,
}

impl Error {
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        Error {
            class,
            message: message.into(),
            attribute: None,
            validation_type: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Error::new(ErrorClass::Validation, message)
    }

    pub fn issue(attribute: Value, validation_type: &'static str, message: String) -> Self {
        Error {
            class: ErrorClass::ValidationIssue,
            message,
            attribute: Some(attribute),
            validation_type: Some(validation_type),
        }
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Error::new(ErrorClass::Unsupported, message)
    }

    pub fn is_unsupported(&self) -> bool {
        self.class == ErrorClass::Unsupported
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class.ruby_name(), self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
