//! The bounded LRU + TTL cache behind `OptionsBuilder.for`.
//!
//! Source: `lib/ukiryu/cache.rb` and `OptionsBuilder.option_classes_cache`
//! (`max_size: 100, ttl: 3600`) and `OptionsBuilder.for` (ukiryu/ukiryu
//! ab6df353). Semantics kept: an entry expires when strictly more than `ttl`
//! seconds passed since it was *created*; reads refresh only the access time;
//! inserting while full evicts the least recently accessed entry (first in
//! insertion order on ties) even when the key already exists; overwriting a
//! key keeps its insertion position.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CommandDef, OptionsClass, OptionsError};

/// Seconds since an arbitrary epoch (`Time.now`).
pub trait Clock: Send + Sync {
    fn now(&self) -> f64;
}

/// The system wall clock, as `Time.now`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
}

struct Entry<V> {
    value: V,
    created_at: f64,
    accessed_at: f64,
}

/// `cache.stats`.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheStats {
    pub size: usize,
    pub max_size: usize,
    pub ttl: Option<f64>,
    pub utilization: f64,
}

/// `Ukiryu::Cache`, thread safe.
pub struct Cache<K, V> {
    max_size: usize,
    ttl: Option<f64>,
    clock: Arc<dyn Clock>,
    data: Mutex<Vec<(K, Entry<V>)>>,
}

impl<K: PartialEq + Clone, V: Clone> Cache<K, V> {
    pub fn new(max_size: usize, ttl: Option<f64>) -> Self {
        Self::with_clock(max_size, ttl, Arc::new(SystemClock))
    }

    pub fn with_clock(max_size: usize, ttl: Option<f64>, clock: Arc<dyn Clock>) -> Self {
        Self {
            max_size,
            ttl,
            clock,
            data: Mutex::new(Vec::new()),
        }
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    pub fn ttl(&self) -> Option<f64> {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(K, Entry<V>)>> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn expired(&self, e: &Entry<V>, now: f64) -> bool {
        self.ttl.is_some_and(|ttl| now - e.created_at > ttl)
    }

    /// `cache[key]`.
    pub fn get(&self, key: &K) -> Option<V> {
        let now = self.clock.now();
        let mut data = self.lock();
        let pos = data.iter().position(|(k, _)| k == key)?;
        if self.expired(&data[pos].1, now) {
            data.remove(pos);
            return None;
        }
        data[pos].1.accessed_at = now;
        Some(data[pos].1.value.clone())
    }

    /// `cache[key] = value`.
    pub fn insert(&self, key: K, value: V) {
        let now = self.clock.now();
        let mut data = self.lock();
        if data.len() >= self.max_size {
            let mut lru: Option<usize> = None;
            for (i, (_, e)) in data.iter().enumerate() {
                if lru.is_none_or(|j| e.accessed_at < data[j].1.accessed_at) {
                    lru = Some(i);
                }
            }
            if let Some(i) = lru {
                data.remove(i);
            }
        }
        let entry = Entry {
            value,
            created_at: now,
            accessed_at: now,
        };
        match data.iter().position(|(k, _)| *k == key) {
            Some(pos) => data[pos].1 = entry,
            None => data.push((key, entry)),
        }
    }

    /// `cache.key?(key)`.
    pub fn contains_key(&self, key: &K) -> bool {
        let now = self.clock.now();
        let mut data = self.lock();
        let Some(pos) = data.iter().position(|(k, _)| k == key) else {
            return false;
        };
        if self.expired(&data[pos].1, now) {
            data.remove(pos);
            return false;
        }
        true
    }

    /// `cache.delete(key)`.
    pub fn delete(&self, key: &K) -> Option<V> {
        let mut data = self.lock();
        let pos = data.iter().position(|(k, _)| k == key)?;
        Some(data.remove(pos).1.value)
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// `cache.size` (does not drop expired entries).
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn cleanup(&self, data: &mut Vec<(K, Entry<V>)>) {
        let now = self.clock.now();
        data.retain(|(_, e)| !self.expired(e, now));
    }

    /// `cache.keys` (drops expired entries first).
    pub fn keys(&self) -> Vec<K> {
        let mut data = self.lock();
        self.cleanup(&mut data);
        data.iter().map(|(k, _)| k.clone()).collect()
    }

    /// `cache.stats`.
    pub fn stats(&self) -> CacheStats {
        let mut data = self.lock();
        self.cleanup(&mut data);
        CacheStats {
            size: data.len(),
            max_size: self.max_size,
            ttl: self.ttl,
            utilization: data.len() as f64 / self.max_size as f64,
        }
    }
}

/// `OptionsBuilder.option_classes_cache` plus `OptionsBuilder.for`.
pub struct OptionsClassCache<C> {
    cache: Cache<(String, String), Arc<OptionsClass<C>>>,
}

impl<C: CommandDef> Default for OptionsClassCache<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: CommandDef> OptionsClassCache<C> {
    /// `max_size: 100, ttl: 3600`.
    pub const MAX_SIZE: usize = 100;
    pub const TTL: f64 = 3600.0;

    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            cache: Cache::with_clock(Self::MAX_SIZE, Some(Self::TTL), clock),
        }
    }

    pub fn cache(&self) -> &Cache<(String, String), Arc<OptionsClass<C>>> {
        &self.cache
    }

    /// `OptionsBuilder.for(tool_name, command_name)`. `resolve` stands for
    /// `Tool.get(tool).command_definition(command)`: its error propagates,
    /// and `Ok(None)` is an unknown command.
    pub fn options_for<F>(
        &self,
        tool_name: &str,
        command_name: &str,
        resolve: F,
    ) -> Result<Arc<OptionsClass<C>>, OptionsError>
    where
        F: FnOnce(&str, &str) -> Result<Option<C>, OptionsError>,
    {
        let key = (tool_name.to_owned(), command_name.to_owned());
        if let Some(found) = self.cache.get(&key) {
            return Ok(found);
        }
        let def = resolve(tool_name, command_name)?.ok_or_else(|| {
            OptionsError::new(
                "ArgumentError",
                format!("Unknown command: {command_name} for tool: {tool_name}"),
            )
        })?;
        let class = Arc::new(OptionsClass::new(tool_name, command_name, Arc::new(def))?);
        self.cache.insert(key, Arc::clone(&class));
        Ok(class)
    }

    /// `OptionsBuilder.clear_cache`.
    pub fn clear(&self) {
        self.cache.clear();
    }
}
