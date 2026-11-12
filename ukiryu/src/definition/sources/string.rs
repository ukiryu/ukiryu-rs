//! Source: `lib/ukiryu/definition/sources/string.rb`
//! (`Ukiryu::Definition::Sources::StringSource`).
use crate::definition::source::{sha256_hex, Source, SourceType};
use crate::definition::{ruby_strip, DefinitionError, Result};

/// A definition provided as YAML text. Content is stored stripped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringSource {
    content: String,
    content_hash: String,
}

impl StringSource {
    /// `StringSource.new(content)`: empty content raises
    /// `DefinitionLoadError`; whitespace-only content is accepted and
    /// stored as `""` (the emptiness check runs before stripping).
    pub fn new(content: &str) -> Result<Self> {
        if content.is_empty() {
            return Err(DefinitionError::load("Definition content cannot be empty"));
        }
        let content = ruby_strip(content).to_string();
        let content_hash = sha256_hex(&content);
        Ok(StringSource {
            content,
            content_hash,
        })
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// `size`: content byte size.
    pub fn size(&self) -> usize {
        self.content.len()
    }
}

impl Source for StringSource {
    fn load(&mut self) -> Result<String> {
        Ok(self.content.clone())
    }

    fn cache_key(&self) -> Result<String> {
        Ok(format!("string:{}", self.content_hash))
    }

    fn source_type(&self) -> SourceType {
        SourceType::String
    }

    fn ruby_class_name(&self) -> &'static str {
        "Ukiryu::Definition::Sources::StringSource"
    }
}
