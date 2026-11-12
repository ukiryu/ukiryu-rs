//! `Ukiryu::CacheRegistry`: clear and report a fixed set of caches.
//!
//! Source: `lib/ukiryu/cache_registry.rb` (ukiryu/ukiryu ab6df353). Ruby
//! registers exactly the tool cache (`tool_cache`) and the definition
//! loader's profile cache (`definition_cache`); [`CacheRegistry::standard`]
//! builds that set. The shell instance cache, the options class cache and
//! the generated class cache are not part of it, so `clear_all` leaves them
//! alone.
//!
//! `stats` reports the raw entry count: entries past their TTL that nobody
//! has read yet are still counted, because Ruby uses `cache.size`, which
//! does not drop expired entries.

use std::sync::Arc;

use super::tool::ToolCache;
use crate::definition::{DefinitionDocument, Loader};
use crate::options::cache::Cache;

/// One cache the registry can clear and size.
pub trait RegisteredCache: Send + Sync {
    /// `cache.clear`.
    fn clear(&self);
    /// `cache.size`, expired entries included.
    fn size(&self) -> usize;
    /// `cache.max_size`.
    fn max_size(&self) -> usize;
}

impl<K, V> RegisteredCache for Cache<K, V>
where
    K: PartialEq + Clone + Send,
    V: Clone + Send,
{
    fn clear(&self) {
        Cache::clear(self);
    }

    fn size(&self) -> usize {
        self.len()
    }

    fn max_size(&self) -> usize {
        Cache::max_size(self)
    }
}

impl<T: Clone + Send> RegisteredCache for ToolCache<T> {
    fn clear(&self) {
        ToolCache::clear(self);
    }

    fn size(&self) -> usize {
        self.len()
    }

    fn max_size(&self) -> usize {
        Self::MAX_SIZE
    }
}

/// `Definition::Loader.profile_cache` as a registry entry. The loader does
/// not expose its size limit, so the limit it was built with is passed in
/// ([`Loader::new`] uses [`DefinitionLoaderCache::DEFAULT_MAX_SIZE`]).
pub struct DefinitionLoaderCache<T> {
    loader: Arc<Loader<T>>,
    max_size: usize,
}

impl<T> DefinitionLoaderCache<T> {
    /// `Cache.new(max_size: 100, ttl: 3600)` in `Loader.profile_cache`.
    pub const DEFAULT_MAX_SIZE: usize = 100;

    /// For a loader built with [`Loader::new`].
    pub fn new(loader: Arc<Loader<T>>) -> Self {
        Self::with_max_size(loader, Self::DEFAULT_MAX_SIZE)
    }

    /// For a loader built with `Loader::with_cache_limits(max_size, _)`.
    pub fn with_max_size(loader: Arc<Loader<T>>, max_size: usize) -> Self {
        Self { loader, max_size }
    }

    pub fn loader(&self) -> &Arc<Loader<T>> {
        &self.loader
    }
}

impl<T: DefinitionDocument + Send + Sync> RegisteredCache for DefinitionLoaderCache<T> {
    fn clear(&self) {
        // `clear_cache(None)` has no fallible step.
        let _ = self.loader.clear_cache(None);
    }

    fn size(&self) -> usize {
        self.loader.cache_size()
    }

    fn max_size(&self) -> usize {
        self.max_size
    }
}

/// The per-cache hash in `CacheRegistry.stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryCacheStats {
    pub size: usize,
    pub max_size: usize,
}

/// `Ukiryu::CacheRegistry`, owned by the caller.
#[derive(Default, Clone)]
pub struct CacheRegistry {
    caches: Vec<(String, Arc<dyn RegisteredCache>)>,
}

impl CacheRegistry {
    /// Stats key of the tool cache.
    pub const TOOL_CACHE: &'static str = "tool_cache";
    /// Stats key of the definition loader's profile cache.
    pub const DEFINITION_CACHE: &'static str = "definition_cache";

    pub fn new() -> Self {
        Self::default()
    }

    /// The Ruby set: `tool_cache` then `definition_cache`.
    pub fn standard(
        tool_cache: Arc<dyn RegisteredCache>,
        definition_cache: Arc<dyn RegisteredCache>,
    ) -> Self {
        let mut registry = Self::new();
        registry.register(Self::TOOL_CACHE, tool_cache);
        registry.register(Self::DEFINITION_CACHE, definition_cache);
        registry
    }

    /// Adds a cache under `name`, replacing one already registered under
    /// that name in place.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        cache: Arc<dyn RegisteredCache>,
    ) -> &mut Self {
        let name = name.into();
        match self.caches.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = cache,
            None => self.caches.push((name, cache)),
        }
        self
    }

    /// `CacheRegistry.caches`, in registration order.
    pub fn caches(&self) -> Vec<Arc<dyn RegisteredCache>> {
        self.caches.iter().map(|(_, c)| Arc::clone(c)).collect()
    }

    /// `CacheRegistry.clear_all`.
    pub fn clear_all(&self) {
        for (_, cache) in &self.caches {
            cache.clear();
        }
    }

    /// `CacheRegistry.stats`, in registration order.
    pub fn stats(&self) -> Vec<(String, RegistryCacheStats)> {
        self.caches
            .iter()
            .map(|(name, cache)| {
                (
                    name.clone(),
                    RegistryCacheStats {
                        size: cache.size(),
                        max_size: cache.max_size(),
                    },
                )
            })
            .collect()
    }
}
