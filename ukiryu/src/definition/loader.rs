//! Loading tool definitions from sources.
//!
//! Source: `lib/ukiryu/definition/loader.rb` (`Ukiryu::Definition::Loader`).
//! The Ruby loader is a class-level singleton parsing into
//! `Models::ToolDefinition`; here a [`Loader`] instance owns its cache and is
//! generic over the document type through [`DefinitionDocument`].
use super::source::Source;
use super::sources::{FileSource, StringSource};
use super::{DefinitionError, DefinitionErrorKind, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Parse failure reported by a [`DefinitionDocument`]; selects the Ruby
/// message prefix (`Psych::SyntaxError`, `Lutaml::Model::InvalidFormatError`
/// or any other `StandardError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentParseError {
    Syntax(String),
    Format(String),
    Other(String),
}

/// What the loader needs from a tool definition document model
/// (`Models::ToolDefinition`). Implemented by the models module.
pub trait DefinitionDocument: Sized {
    /// `ToolDefinition.from_yaml(content)`.
    fn parse_definition(yaml: &str) -> std::result::Result<Self, DocumentParseError>;
    /// `name` (`None` = nil; an empty string is present).
    fn definition_name(&self) -> Option<&str>;
    /// `version`.
    fn definition_version(&self) -> Option<&str>;
    /// `profiles&.any?`.
    fn has_profiles(&self) -> bool;
    /// `ukiryu_schema`.
    fn ukiryu_schema(&self) -> Option<&str>;
    /// `self_uri` (YAML key `$self`).
    fn self_uri(&self) -> Option<&str>;
    /// `resolve_inheritance!`.
    fn resolve_inheritance(&mut self);
}

/// `options[:validation]`. Ruby treats any value other than `:strict` and
/// `:none` like `:lenient`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ValidationMode {
    #[default]
    Strict,
    Lenient,
    None,
}

/// Ruby `^\d+\.\d+$` (multi-line anchors: any line may match).
fn schema_format_ok(s: &str) -> bool {
    s.split('\n').any(|line| {
        let mut parts = line.splitn(2, '.');
        let a = parts.next().unwrap_or("");
        let b = parts.next().unwrap_or("");
        !a.is_empty()
            && !b.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit())
            && b.bytes().all(|c| c.is_ascii_digit())
    })
}

/// Ruby `uri =~ %r{^https?://} || uri =~ %r{^file://}` (any line start).
fn valid_uri(s: &str) -> bool {
    s.split('\n').any(|line| {
        line.starts_with("http://") || line.starts_with("https://") || line.starts_with("file://")
    })
}

/// `Loader.validate_profile(profile, mode)` (private in Ruby). Strict mode
/// returns `DefinitionValidationError`; lenient mode returns the warning
/// message that Ruby prints as `"[Ukiryu] #{message}"`.
pub fn validate_profile<T: DefinitionDocument>(
    profile: &T,
    mode: ValidationMode,
) -> Result<Option<String>> {
    let mut errors = Vec::new();
    if profile.definition_name().is_none() {
        errors.push("Missing 'name' field".to_string());
    }
    if profile.definition_version().is_none() {
        errors.push("Missing 'version' field".to_string());
    }
    if !profile.has_profiles() {
        errors.push("Missing 'profiles' field or profiles is empty".to_string());
    }
    if let Some(schema) = profile.ukiryu_schema() {
        if !schema_format_ok(schema) {
            errors.push(format!("Invalid ukiryu_schema format: {schema}"));
        }
    }
    if let Some(uri) = profile.self_uri() {
        if !valid_uri(uri) && mode == ValidationMode::Strict {
            errors.push(format!("Invalid $self URI format: {uri}"));
        }
    }
    if errors.is_empty() {
        return Ok(None);
    }
    let message = format!("Profile validation failed:\n  - {}", errors.join("\n  - "));
    if mode == ValidationMode::Strict {
        return Err(DefinitionError::new(
            DefinitionErrorKind::Validation,
            message,
        ));
    }
    Ok(Some(message))
}

struct CacheEntry<V> {
    value: V,
    created_at: Instant,
    accessed_at: Instant,
}

/// `Ukiryu::Cache` as used by the loader: bounded LRU with TTL.
struct ProfileCache<V> {
    max_size: usize,
    ttl: Option<Duration>,
    data: Vec<(String, CacheEntry<V>)>,
}

impl<V: Clone> ProfileCache<V> {
    fn position_live(&mut self, key: &str) -> Option<usize> {
        let idx = self.data.iter().position(|(k, _)| k == key)?;
        if let Some(ttl) = self.ttl {
            if self.data[idx].1.created_at.elapsed() > ttl {
                self.data.remove(idx);
                return None;
            }
        }
        Some(idx)
    }

    fn get(&mut self, key: &str) -> Option<V> {
        let idx = self.position_live(key)?;
        let entry = &mut self.data[idx].1;
        entry.accessed_at = Instant::now();
        Some(entry.value.clone())
    }

    fn insert(&mut self, key: String, value: V) {
        if self.data.len() >= self.max_size {
            let lru = self
                .data
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    a.1 .1
                        .accessed_at
                        .cmp(&b.1 .1.accessed_at)
                        .then(a.0.cmp(&b.0))
                })
                .map(|(i, _)| i);
            if let Some(i) = lru {
                self.data.remove(i);
            }
        }
        let now = Instant::now();
        let entry = CacheEntry {
            value,
            created_at: now,
            accessed_at: now,
        };
        match self.data.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = entry,
            None => self.data.push((key, entry)),
        }
    }
}

/// `Ukiryu::Definition::Loader`, with an instance-owned profile cache
/// (default: 100 entries, 3600 s TTL).
pub struct Loader<T> {
    cache: Mutex<ProfileCache<Arc<T>>>,
    warnings: Mutex<Vec<String>>,
}

impl<T: DefinitionDocument> Default for Loader<T> {
    fn default() -> Self {
        Self::with_cache_limits(100, Some(Duration::from_secs(3600)))
    }
}

impl<T: DefinitionDocument> Loader<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cache_limits(max_size: usize, ttl: Option<Duration>) -> Self {
        Loader {
            cache: Mutex::new(ProfileCache {
                max_size,
                ttl,
                data: Vec::new(),
            }),
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// `load_from_source(source, validation:)`. A cache hit is returned
    /// before any validation, so a definition cached by a lenient load is
    /// later returned to strict callers unchanged (Ruby behavior).
    pub fn load_from_source(
        &self,
        source: &mut dyn Source,
        mode: ValidationMode,
    ) -> Result<Arc<T>> {
        let key = source.cache_key()?;
        if let Some(hit) = self.lock_cache().get(&key) {
            return Ok(hit);
        }
        let content = source.load()?;
        let mut profile = T::parse_definition(&content).map_err(|e| {
            let src = source.describe().unwrap_or_default();
            DefinitionError::load(match e {
                DocumentParseError::Syntax(m) => format!("Invalid YAML in {src}: {m}"),
                DocumentParseError::Format(m) => format!("Invalid YAML format in {src}: {m}"),
                DocumentParseError::Other(m) => {
                    format!("Failed to parse definition from {src}: {m}")
                }
            })
        })?;
        if mode != ValidationMode::None {
            if let Some(warning) = validate_profile(&profile, mode)? {
                eprintln!("[Ukiryu] {warning}");
                self.warnings
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(warning);
            }
        }
        profile.resolve_inheritance();
        let profile = Arc::new(profile);
        self.lock_cache().insert(key, Arc::clone(&profile));
        Ok(profile)
    }

    /// `load_from_file(path, options)`.
    pub fn load_from_file(&self, path: impl AsRef<Path>, mode: ValidationMode) -> Result<Arc<T>> {
        let mut source = FileSource::new(path)?;
        self.load_from_source(&mut source, mode)
    }

    /// `load_from_string(yaml, options)`.
    pub fn load_from_string(&self, yaml: &str, mode: ValidationMode) -> Result<Arc<T>> {
        let mut source = StringSource::new(yaml)?;
        self.load_from_source(&mut source, mode)
    }

    /// `clear_cache(source = nil)`.
    pub fn clear_cache(&self, source: Option<&dyn Source>) -> Result<()> {
        let mut cache = self.lock_cache();
        match source {
            Some(s) => {
                let key = s.cache_key()?;
                cache.data.retain(|(k, _)| *k != key);
            }
            None => cache.data.clear(),
        }
        Ok(())
    }

    /// Number of cached profiles (`profile_cache.size`).
    pub fn cache_size(&self) -> usize {
        self.lock_cache().data.len()
    }

    /// Lenient-mode validation messages emitted so far (also written to
    /// stderr as `"[Ukiryu] <message>"`), drained.
    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.lock().unwrap_or_else(|p| p.into_inner()))
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, ProfileCache<Arc<T>>> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }
}
