//! Metadata about a discovered tool definition.
//!
//! Source: `lib/ukiryu/definition/metadata.rb`
//! (`Ukiryu::Definition::DefinitionMetadata`, alias `Metadata`).
use super::loader::{DefinitionDocument, Loader, ValidationMode};
use super::sources::file::expand_path;
use super::{ruby_inspect_str, Result};
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

/// Where a definition was discovered (`source_type` symbol).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    User,
    Bundled,
    LocalSystem,
    System,
    Register,
    Other(String),
}

impl SourceKind {
    pub fn as_str(&self) -> &str {
        match self {
            SourceKind::User => "user",
            SourceKind::Bundled => "bundled",
            SourceKind::LocalSystem => "local_system",
            SourceKind::System => "system",
            SourceKind::Register => "register",
            SourceKind::Other(s) => s,
        }
    }

    /// Lower number = higher priority; unknown kinds are 999.
    pub fn priority(&self) -> u32 {
        match self {
            SourceKind::User => 1,
            SourceKind::Bundled => 2,
            SourceKind::LocalSystem => 3,
            SourceKind::System => 4,
            SourceKind::Register => 5,
            SourceKind::Other(_) => 999,
        }
    }
}

/// `Ukiryu::Definition::DefinitionMetadata`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DefinitionMetadata {
    pub name: String,
    pub version: String,
    /// Expanded absolute path (`File.expand_path`).
    pub path: String,
    pub source_type: SourceKind,
}

impl DefinitionMetadata {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        path: impl AsRef<Path>,
        source_type: SourceKind,
    ) -> Self {
        DefinitionMetadata {
            name: name.into(),
            version: version.into(),
            path: expand_path(path.as_ref()),
            source_type,
        }
    }

    /// `load_definition`: `Loader.load_from_file(path)` (strict validation).
    pub fn load_definition<T: DefinitionDocument>(&self, loader: &Loader<T>) -> Result<Arc<T>> {
        loader.load_from_file(&self.path, ValidationMode::Strict)
    }

    /// `exists?`
    pub fn exists(&self) -> bool {
        Path::new(&self.path).exists()
    }

    /// `mtime`: `None` when the file does not exist.
    pub fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok()
    }

    pub fn priority(&self) -> u32 {
        self.source_type.priority()
    }

    /// `inspect`.
    pub fn inspect(&self) -> String {
        format!(
            "#<Ukiryu::Definition::DefinitionMetadata name={} version={} path={} source_type=:{}>",
            ruby_inspect_str(&self.name),
            ruby_inspect_str(&self.version),
            ruby_inspect_str(&self.path),
            self.source_type.as_str()
        )
    }

    /// `<=>`: priority ascending, then version descending (zero-padded
    /// `to_i` components), then name ascending. Metadata differing only by
    /// path compare equal.
    pub fn compare(&self, other: &Self) -> Ordering {
        self.priority()
            .cmp(&other.priority())
            .then_with(|| super::VersionResolver::compare_versions(&other.version, &self.version))
            .then_with(|| self.name.as_bytes().cmp(other.name.as_bytes()))
    }
}

impl std::fmt::Display for DefinitionMetadata {
    /// `to_s`: `"name/version (source_type) - path"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} ({}) - {}",
            self.name,
            self.version,
            self.source_type.as_str(),
            self.path
        )
    }
}
