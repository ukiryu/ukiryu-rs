//! Definition cache with TTL and mtime staleness.
//!
//! Source: `lib/ukiryu/definition/definition_cache.rb`
//! (`Ukiryu::Definition::DefinitionCache`). The Ruby singleton
//! (`DefinitionCache.instance`) is not reproduced; callers own instances.
use super::metadata::DefinitionMetadata;
use super::sources::FileSource;
use super::{DefinitionError, DefinitionErrorKind, Result};
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

/// `DefinitionCache::DEFAULT_TTL` (seconds).
pub const DEFAULT_TTL: i64 = 300;

/// Refresh strategy (`:lazy`, `:eager`, `:never`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RefreshStrategy {
    #[default]
    Lazy,
    Eager,
    Never,
}

impl RefreshStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            RefreshStrategy::Lazy => "lazy",
            RefreshStrategy::Eager => "eager",
            RefreshStrategy::Never => "never",
        }
    }

    /// `refresh_strategy=` validation for a symbol name.
    pub fn from_symbol(name: &str) -> Result<Self> {
        match name {
            "lazy" => Ok(RefreshStrategy::Lazy),
            "eager" => Ok(RefreshStrategy::Eager),
            "never" => Ok(RefreshStrategy::Never),
            other => Err(DefinitionError::new(
                DefinitionErrorKind::Argument,
                format!("Invalid refresh strategy: {other}"),
            )),
        }
    }
}

/// Duck-typed capabilities the Ruby cache probes with `respond_to?`.
pub trait CachedDefinition: Clone {
    /// `respond_to?(:path)`: file used for mtime staleness.
    fn definition_path(&self) -> Option<String> {
        None
    }

    /// `respond_to?(:load_definition)`: `Some` when the entry can be
    /// reloaded on refresh.
    fn reload(&self) -> Option<Result<Self>> {
        None
    }
}

/// Metadata entries are refreshable in Ruby, but the refresh calls
/// `load_definition` and then `mtime` on the loaded `ToolDefinition`, which
/// has no such method, so every metadata refresh raises `NoMethodError`
/// (after any file-access error from the load).
impl CachedDefinition for DefinitionMetadata {
    fn definition_path(&self) -> Option<String> {
        Some(self.path.clone())
    }

    fn reload(&self) -> Option<Result<Self>> {
        Some(FileSource::new(&self.path).and_then(|_| {
            Err(DefinitionError::no_method(
                "undefined method 'mtime' for an instance of Ukiryu::Models::ToolDefinition",
            ))
        }))
    }
}

#[derive(Clone, Debug)]
struct Entry<D> {
    definition: D,
    mtime: Option<SystemTime>,
    loaded_at: SystemTime,
}

impl<D: CachedDefinition> Entry<D> {
    fn new(definition: D, mtime: Option<SystemTime>) -> Self {
        Entry {
            definition,
            mtime,
            loaded_at: SystemTime::now(),
        }
    }

    fn stale(&self, ttl: i64) -> bool {
        let age = SystemTime::now()
            .duration_since(self.loaded_at)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if age > ttl as f64 {
            return true;
        }
        if let (Some(mtime), Some(path)) = (self.mtime, self.definition.definition_path()) {
            if let Ok(current) = std::fs::metadata(Path::new(&path)).and_then(|m| m.modified()) {
                if current > mtime {
                    return true;
                }
            }
        }
        false
    }

    fn refresh(&self) -> Result<Self> {
        match self.definition.reload() {
            Some(reloaded) => {
                let definition = reloaded?;
                Ok(Entry::new(definition, self.mtime))
            }
            None => Ok(self.clone()),
        }
    }
}

/// `stats`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheStats {
    pub size: usize,
    pub ttl: i64,
    pub refresh_strategy: RefreshStrategy,
    pub entries: Vec<String>,
}

struct Inner<D> {
    entries: Vec<(String, Entry<D>)>,
    ttl: i64,
    refresh_strategy: RefreshStrategy,
}

impl<D> Inner<D> {
    fn index(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|(k, _)| k == key)
    }
}

/// `Ukiryu::Definition::DefinitionCache`.
pub struct DefinitionCache<D> {
    inner: Mutex<Inner<D>>,
}

impl<D: CachedDefinition> Default for DefinitionCache<D> {
    fn default() -> Self {
        Self::new(DEFAULT_TTL)
    }
}

impl<D: CachedDefinition> DefinitionCache<D> {
    /// `DefinitionCache.new(ttl:)`.
    pub fn new(ttl: i64) -> Self {
        DefinitionCache {
            inner: Mutex::new(Inner {
                entries: Vec::new(),
                ttl,
                refresh_strategy: RefreshStrategy::Lazy,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<D>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `get(key)`: stale entries are refreshed under `:lazy` (refresh errors
    /// propagate) and returned as-is otherwise.
    pub fn get(&self, key: &str) -> Result<Option<D>> {
        let mut inner = self.lock();
        let Some(i) = inner.index(key) else {
            return Ok(None);
        };
        let ttl = inner.ttl;
        if inner.entries[i].1.stale(ttl) && inner.refresh_strategy == RefreshStrategy::Lazy {
            let refreshed = inner.entries[i].1.refresh()?;
            inner.entries[i].1 = refreshed;
        }
        Ok(Some(inner.entries[i].1.definition.clone()))
    }

    /// `set(key, definition, metadata: nil)`; the metadata's mtime (if any)
    /// enables file staleness checks.
    pub fn set(&self, key: &str, definition: D, metadata: Option<&DefinitionMetadata>) -> D {
        let entry = Entry::new(
            definition.clone(),
            metadata.and_then(DefinitionMetadata::mtime),
        );
        let mut inner = self.lock();
        match inner.index(key) {
            Some(i) => inner.entries[i].1 = entry,
            None => inner.entries.push((key.to_string(), entry)),
        }
        definition
    }

    /// `key?(key)` (no staleness check).
    pub fn contains_key(&self, key: &str) -> bool {
        self.lock().index(key).is_some()
    }

    /// `invalidate(key)`: whether an entry was removed.
    pub fn invalidate(&self, key: &str) -> bool {
        let mut inner = self.lock();
        match inner.index(key) {
            Some(i) => {
                inner.entries.remove(i);
                true
            }
            None => false,
        }
    }

    /// `clear`: number of entries removed.
    pub fn clear(&self) -> usize {
        let mut inner = self.lock();
        let n = inner.entries.len();
        inner.entries.clear();
        n
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.lock();
        CacheStats {
            size: inner.entries.len(),
            ttl: inner.ttl,
            refresh_strategy: inner.refresh_strategy,
            entries: inner.entries.iter().map(|(k, _)| k.clone()).collect(),
        }
    }

    /// `stale?(key)`: missing keys are stale.
    pub fn is_stale(&self, key: &str) -> bool {
        let inner = self.lock();
        match inner.index(key) {
            Some(i) => inner.entries[i].1.stale(inner.ttl),
            None => true,
        }
    }

    pub fn refresh_strategy(&self) -> RefreshStrategy {
        self.lock().refresh_strategy
    }

    pub fn set_refresh_strategy(&self, strategy: RefreshStrategy) {
        self.lock().refresh_strategy = strategy;
    }

    pub fn ttl(&self) -> i64 {
        self.lock().ttl
    }

    pub fn set_ttl(&self, ttl: i64) {
        self.lock().ttl = ttl;
    }

    /// `refresh_stale`: refreshes every stale entry in order; an error stops
    /// the walk, keeping entries refreshed before it.
    pub fn refresh_stale(&self) -> Result<usize> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        let mut count = 0;
        for i in 0..inner.entries.len() {
            if inner.entries[i].1.stale(ttl) {
                let refreshed = inner.entries[i].1.refresh()?;
                inner.entries[i].1 = refreshed;
                count += 1;
            }
        }
        Ok(count)
    }

    /// `prune`: removes stale entries, returning how many.
    pub fn prune(&self) -> usize {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        let before = inner.entries.len();
        inner.entries.retain(|(_, e)| !e.stale(ttl));
        before - inner.entries.len()
    }
}
