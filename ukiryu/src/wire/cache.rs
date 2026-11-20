//! Cache policy for language-owned objects. Bindings keep their references and
//! send opaque IDs plus the previous snapshot; no Ruby values or global handles
//! cross this boundary. Eviction, access times and TTL use the existing cache.
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::cache::{cache_key_for, Cache, Clock, RuntimeDefaults, ToolKeyOptions};

// Internally tagged serde enums buffer arbitrary-precision JSON numbers as
// maps. Deserialize through Number, which preserves that representation.
#[derive(Deserialize)]
#[serde(try_from = "serde_json::Number")]
pub struct Seconds(f64);

impl TryFrom<serde_json::Number> for Seconds {
    type Error = &'static str;
    fn try_from(value: serde_json::Number) -> Result<Self, Self::Error> {
        value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Self)
            .ok_or("cache timestamps must be finite numbers")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRequest {
    pub max_size: usize,
    pub ttl: Option<Seconds>,
    pub now: Seconds,
    /// Object ID, creation time, last access time, in insertion order.
    pub state: Vec<(u64, Seconds, Seconds)>,
    pub action: Action,
}

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Get {
        key: u64,
    },
    Set {
        key: u64,
    },
    ContainsKey {
        key: u64,
    },
    Delete {
        key: u64,
    },
    Clear,
    Keys,
    Stats,
    ToolKey {
        name: String,
        platform: String,
        shell: String,
        version: Option<String>,
    },
}

struct RequestClock(f64);

impl Clock for RequestClock {
    fn now(&self) -> f64 {
        self.0
    }
}

pub fn process(request: CacheRequest) -> Value {
    let cache = Cache::restore(
        request.max_size,
        request.ttl.map(|seconds| seconds.0),
        Arc::new(RequestClock(request.now.0)),
        request
            .state
            .into_iter()
            .map(|(key, created, accessed)| (key, key, created.0, accessed.0))
            .collect(),
    );
    let value = match request.action {
        Action::Get { key } => json!(cache.get(&key)),
        Action::Set { key } => {
            cache.insert(key, key);
            json!(key)
        }
        Action::ContainsKey { key } => json!(cache.contains_key(&key)),
        Action::Delete { key } => json!(cache.delete(&key)),
        Action::Clear => {
            cache.clear();
            Value::Null
        }
        Action::Keys => json!(cache.keys()),
        Action::Stats => {
            let stats = cache.stats();
            json!({"size": stats.size, "max_size": stats.max_size,
                   "ttl": stats.ttl, "utilization": stats.utilization})
        }
        Action::ToolKey {
            name,
            platform,
            shell,
            version,
        } => json!(cache_key_for(
            &name,
            &ToolKeyOptions {
                version: version.as_deref(),
                ..ToolKeyOptions::default()
            },
            &RuntimeDefaults {
                platform: &platform,
                shell: &shell
            },
        )),
    };
    let state: Vec<_> = cache
        .snapshot()
        .into_iter()
        .map(|(key, _, created, accessed)| (key, created, accessed))
        .collect();
    json!({"ok": true, "value": {"result": value, "state": state}})
}
