//! `Ukiryu::ToolCache`: tool instances in a bounded LRU cache (50 entries,
//! one hour TTL) keyed `name-platform-shell-version`.
//!
//! Source: `lib/ukiryu/tool_cache.rb` (ukiryu/ukiryu ab6df353).

use std::sync::Arc;

use crate::options::cache::{Cache, Clock, SystemClock};

/// The runtime platform and shell that `cache_key_for` falls back to
/// (`Runtime.instance.platform` / `.shell` in Ruby). The caller resolves
/// them; this module never detects anything itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeDefaults<'a> {
    pub platform: &'a str,
    pub shell: &'a str,
}

/// The `options` hash entries `cache_key_for` reads. `None` stands for a
/// missing or nil (or `false`) entry; an empty string is a present value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolKeyOptions<'a> {
    pub platform: Option<&'a str>,
    pub shell: Option<&'a str>,
    pub version: Option<&'a str>,
}

/// `ToolCache.cache_key_for(name, options)`:
/// `"#{name}-#{platform}-#{shell}-#{version}"`, version defaulting to
/// `latest`. Components are joined as they are; a `-` inside one is not
/// escaped, so distinct inputs can share a key exactly as in Ruby.
pub fn cache_key_for(
    name: &str,
    options: &ToolKeyOptions<'_>,
    runtime: &RuntimeDefaults<'_>,
) -> String {
    let platform = options.platform.unwrap_or(runtime.platform);
    let shell = options.shell.unwrap_or(runtime.shell);
    let version = options.version.unwrap_or("latest");
    format!("{name}-{platform}-{shell}-{version}")
}

/// `Ukiryu::ToolCache`, owned by the caller instead of being a singleton.
///
/// `clear` only clears this cache. In Ruby it also clears the
/// `Tools::Generator` class cache, which has no Rust counterpart yet.
pub struct ToolCache<T> {
    cache: Cache<String, T>,
}

impl<T: Clone> Default for ToolCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> ToolCache<T> {
    /// `Cache.new(max_size: 50, ttl: 3600)`.
    pub const MAX_SIZE: usize = 50;
    pub const TTL: f64 = 3600.0;

    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// Same limits with an injected clock (tests, embedders).
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            cache: Cache::with_clock(Self::MAX_SIZE, Some(Self::TTL), clock),
        }
    }

    /// `ToolCache.cache`.
    pub fn cache(&self) -> &Cache<String, T> {
        &self.cache
    }

    /// `ToolCache.get(key)`.
    pub fn get(&self, key: &str) -> Option<T> {
        self.cache.get(&key.to_owned())
    }

    /// `ToolCache.set(key, tool)`.
    pub fn set(&self, key: impl Into<String>, tool: T) {
        self.cache.insert(key.into(), tool);
    }

    /// `ToolCache.clear` (tool entries only, see the type docs).
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Number of entries, expired ones included (`cache.size`).
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}
