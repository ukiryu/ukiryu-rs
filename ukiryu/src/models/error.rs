//! Model errors, classified by the Ruby exception the oracle raises.
use super::value::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorKind {
    /// `Lutaml::Model::InvalidFormatError` (non-mapping root or nested model,
    /// `:hash` attribute given a scalar, ...).
    InvalidFormat,
    /// `Lutaml::Model::IncorrectModelError` (a sequence where one model is declared).
    IncorrectModel,
    /// `NoMethodError` raised by Ruby helpers on unexpected classes.
    NoMethod,
    /// `Psych::SyntaxError` / YAML engine failure.
    Syntax,
    /// `Errno::ENOENT` and other file errors from `from_yaml(path)` loaders.
    Io,
    /// A value lutaml keeps raw that the typed model cannot represent
    /// (for example a mapping assigned to a `:string` attribute). The port
    /// fails loudly instead of guessing; see the models module docs.
    Unsupported,
    /// `Psych::DisallowedClass` (a class the `safe_load` call does not permit).
    DisallowedClass,
    /// `Psych::AliasesNotEnabled` (an alias under `aliases: false`).
    AliasesNotEnabled,
    /// `ArgumentError` (for example `Float("abc")` under a `!!float` tag).
    Argument,
    /// `TypeError` (for example `Float(nil)` under a `!!float` tag).
    Type,
}

impl ModelErrorKind {
    pub fn ruby_class(self) -> &'static str {
        match self {
            ModelErrorKind::InvalidFormat => "Lutaml::Model::InvalidFormatError",
            ModelErrorKind::IncorrectModel => "Lutaml::Model::IncorrectModelError",
            ModelErrorKind::NoMethod => "NoMethodError",
            ModelErrorKind::Syntax => "Psych::SyntaxError",
            ModelErrorKind::Io => "Errno::ENOENT",
            ModelErrorKind::Unsupported => "Ukiryu::Models::UnsupportedValue",
            ModelErrorKind::DisallowedClass => "Psych::DisallowedClass",
            ModelErrorKind::AliasesNotEnabled => "Psych::AliasesNotEnabled",
            ModelErrorKind::Argument => "ArgumentError",
            ModelErrorKind::Type => "TypeError",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            ModelErrorKind::InvalidFormat => "[ukiryu:invalid_format]",
            ModelErrorKind::IncorrectModel => "[ukiryu:incorrect_model]",
            ModelErrorKind::NoMethod => "[ukiryu:no_method]",
            ModelErrorKind::Syntax => "[ukiryu:syntax]",
            ModelErrorKind::Io => "[ukiryu:io]",
            ModelErrorKind::Unsupported => "[ukiryu:unsupported]",
            ModelErrorKind::DisallowedClass => "[ukiryu:disallowed_class]",
            ModelErrorKind::AliasesNotEnabled => "[ukiryu:aliases_not_enabled]",
            ModelErrorKind::Argument => "[ukiryu:argument]",
            ModelErrorKind::Type => "[ukiryu:type]",
        }
    }

    const ALL: [ModelErrorKind; 10] = [
        ModelErrorKind::InvalidFormat,
        ModelErrorKind::IncorrectModel,
        ModelErrorKind::NoMethod,
        ModelErrorKind::Syntax,
        ModelErrorKind::Io,
        ModelErrorKind::Unsupported,
        ModelErrorKind::DisallowedClass,
        ModelErrorKind::AliasesNotEnabled,
        ModelErrorKind::Argument,
        ModelErrorKind::Type,
    ];

    /// Marks a serde custom error message so the kind survives the
    /// deserializer's string-only error channel.
    pub(crate) fn tag(self, message: &str) -> String {
        format!("{}{message}", self.marker())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub kind: ModelErrorKind,
    pub message: String,
}

impl ModelError {
    pub fn new(kind: ModelErrorKind, message: impl Into<String>) -> Self {
        ModelError {
            kind,
            message: message.into(),
        }
    }

    pub fn ruby_class(&self) -> &'static str {
        self.kind.ruby_class()
    }

    pub(crate) fn no_method(method: &str, receiver: &Value) -> Self {
        let target = crate::types::ruby::receiver_description(receiver);
        ModelError::new(
            ModelErrorKind::NoMethod,
            format!("undefined method '{method}' for {target}"),
        )
    }

    /// Recovers a kind from a deserializer message; untagged messages come
    /// from the YAML engine itself and classify as `fallback`.
    pub(crate) fn from_message(message: String, fallback: ModelErrorKind) -> Self {
        for kind in ModelErrorKind::ALL {
            if let Some(pos) = message.find(kind.marker()) {
                let text = message[pos + kind.marker().len()..].to_owned();
                return ModelError::new(kind, text);
            }
        }
        ModelError::new(fallback, message)
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.ruby_class(), self.message)
    }
}

impl std::error::Error for ModelError {}
