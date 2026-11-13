//! Oracle tests for `ukiryu::cache` against captures of the pinned Ruby
//! caches (ukiryu/ukiryu ab6df353): `ToolCache`, `Shell::InstanceCache`
//! and `CacheRegistry`, with the shared `Ukiryu::Cache` LRU/TTL store.
//!
//! Golden data: `tests/cache_data/caches.json`, produced by running the
//! original Ruby code with `Time.now` replaced by a fake clock (capture
//! script recorded in its `provenance`). Each LRU scenario is a list of
//! operations with the result Ruby returned; the test replays the same
//! operations with an injected clock and compares every result.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value as J;
use ukiryu::cache::{
    builtin_shell, cache_key_for, CacheRegistry, CacheStats, Clock, DefinitionLoaderCache,
    RegisteredCache, RuntimeDefaults, SharedShell, ShellInstanceCache, ToolCache, ToolKeyOptions,
    UnknownShellError,
};
use ukiryu::definition::{DefinitionDocument, DocumentParseError, Loader, ValidationMode};
use ukiryu::options::Cache;

const GOLDEN: &str = include_str!("cache_data/caches.json");

fn golden() -> J {
    serde_json::from_str(GOLDEN).expect("golden json")
}

/// The capture's fake clock: starts at 1_000_000 s, moves only on `advance`.
struct ManualClock(Mutex<f64>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(ManualClock(Mutex::new(1_000_000.0)))
    }
    fn advance(&self, by: f64) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> f64 {
        *self.0.lock().unwrap()
    }
}

fn opt_str(v: &J) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn str_list(v: &J) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_str().expect("string").to_owned())
        .collect()
}

fn usize_of(v: &J) -> usize {
    v.as_u64().expect("integer") as usize
}

// ---------------------------------------------------------------- tool cache

#[test]
fn tool_cache_limits_match_ruby() {
    let g = golden();
    let cfg = &g["tool_cache_config"];
    assert_eq!(ToolCache::<String>::MAX_SIZE, usize_of(&cfg["max_size"]));
    assert_eq!(ToolCache::<String>::TTL, cfg["ttl"].as_f64().unwrap());
    let tc: ToolCache<String> = ToolCache::new();
    assert_eq!(tc.cache().max_size(), ToolCache::<String>::MAX_SIZE);
    assert_eq!(tc.cache().ttl(), Some(ToolCache::<String>::TTL));
    assert_eq!(
        DefinitionLoaderCache::<TDef>::DEFAULT_MAX_SIZE,
        usize_of(&g["definition_cache_config"]["max_size"])
    );
}

#[test]
fn cache_keys_match_ruby() {
    let g = golden();
    let cases = g["key_cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let platform = opt_str(&case["platform"]);
        let shell = opt_str(&case["shell"]);
        // Ruby nil and false both fall back to "latest"; both map to None.
        let version = opt_str(&case["version"]);
        let options = ToolKeyOptions {
            platform: platform.as_deref(),
            shell: shell.as_deref(),
            version: version.as_deref(),
        };
        let runtime = RuntimeDefaults {
            platform: case["runtime_platform"].as_str().unwrap(),
            shell: case["runtime_shell"].as_str().unwrap(),
        };
        let key = cache_key_for(case["name"].as_str().unwrap(), &options, &runtime);
        assert_eq!(key, case["key"].as_str().unwrap(), "case {case}");
    }
}

fn check_stats(got: CacheStats, want: &J, ctx: &str) {
    assert_eq!(got.size, usize_of(&want["size"]), "{ctx} size");
    assert_eq!(got.max_size, usize_of(&want["max_size"]), "{ctx} max_size");
    assert_eq!(got.ttl, want["ttl"].as_f64(), "{ctx} ttl");
    assert_eq!(
        got.utilization,
        want["utilization"].as_f64().unwrap(),
        "{ctx} utilization"
    );
}

#[test]
fn lru_scenarios_match_ruby() {
    let g = golden();
    let scenarios = g["lru_scenarios"].as_array().unwrap();
    let mut replayed = 0;
    for scenario in scenarios {
        let name = scenario["name"].as_str().unwrap();
        let clock = ManualClock::new();
        let tc: ToolCache<String> = ToolCache::with_clock(clock.clone());
        for (i, op) in scenario["ops"].as_array().unwrap().iter().enumerate() {
            let ctx = format!("{name} op#{i} {op}");
            let key = op["key"].as_str().map(str::to_owned);
            match op["op"].as_str().unwrap() {
                "set" => tc.set(key.unwrap(), op["value"].as_str().unwrap().to_owned()),
                "advance" => clock.advance(op["by"].as_f64().unwrap()),
                "get" => assert_eq!(tc.get(&key.unwrap()), opt_str(&op["result"]), "{ctx}"),
                "key?" => assert_eq!(
                    tc.cache().contains_key(&key.unwrap()),
                    op["result"].as_bool().unwrap(),
                    "{ctx}"
                ),
                "delete" => assert_eq!(
                    tc.cache().delete(&key.unwrap()),
                    opt_str(&op["result"]),
                    "{ctx}"
                ),
                "size" => assert_eq!(tc.len(), usize_of(&op["result"]), "{ctx}"),
                "empty?" => assert_eq!(tc.is_empty(), op["result"].as_bool().unwrap(), "{ctx}"),
                "keys" => assert_eq!(tc.cache().keys(), str_list(&op["result"]), "{ctx}"),
                "stats" => check_stats(tc.cache().stats(), &op["result"], &ctx),
                "clear" => tc.clear(),
                other => panic!("unknown op {other}"),
            }
        }
        replayed += 1;
    }
    assert_eq!(replayed, scenarios.len());
    assert!(replayed >= 10);
}

#[test]
fn tool_caches_are_isolated_instances() {
    let a: ToolCache<String> = ToolCache::new();
    let b: ToolCache<String> = ToolCache::new();
    a.set("ffmpeg-linux-bash-latest", "a".to_owned());
    assert_eq!(b.get("ffmpeg-linux-bash-latest"), None);
    b.clear();
    assert_eq!(a.get("ffmpeg-linux-bash-latest").as_deref(), Some("a"));
}

#[test]
fn tool_cache_is_bounded_under_concurrency() {
    let tc: Arc<ToolCache<usize>> = Arc::new(ToolCache::new());
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let tc = Arc::clone(&tc);
            thread::spawn(move || {
                for i in 0..200 {
                    let key = format!("tool{t}-{i}");
                    tc.set(key.clone(), i);
                    assert!(tc.len() <= ToolCache::<usize>::MAX_SIZE);
                    let _ = tc.get(&key);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("worker panicked");
    }
    assert_eq!(tc.len(), ToolCache::<usize>::MAX_SIZE);
    assert_eq!(tc.cache().keys().len(), ToolCache::<usize>::MAX_SIZE);
}

// ---------------------------------------------------------------- registry

#[derive(Debug)]
struct TDef(String);

impl DefinitionDocument for TDef {
    fn parse_definition(yaml: &str) -> Result<Self, DocumentParseError> {
        Ok(TDef(yaml.to_owned()))
    }
    fn definition_name(&self) -> Option<&str> {
        Some(&self.0)
    }
    fn definition_version(&self) -> Option<&str> {
        None
    }
    fn has_profiles(&self) -> bool {
        true
    }
    fn ukiryu_schema(&self) -> Option<&str> {
        None
    }
    fn self_uri(&self) -> Option<&str> {
        None
    }
    fn resolve_inheritance(&mut self) {}
}

struct World {
    clock: Arc<ManualClock>,
    tools: Arc<ToolCache<String>>,
    loader: Arc<Loader<TDef>>,
    options: Cache<(String, String), String>,
    shells: ShellInstanceCache,
    registry: CacheRegistry,
}

impl World {
    fn new() -> Self {
        let clock = ManualClock::new();
        let tools = Arc::new(ToolCache::with_clock(clock.clone()));
        let loader = Arc::new(Loader::<TDef>::new());
        let registry = CacheRegistry::standard(
            tools.clone(),
            Arc::new(DefinitionLoaderCache::new(loader.clone())),
        );
        World {
            clock: clock.clone(),
            tools,
            loader,
            options: Cache::with_clock(100, Some(3600.0), clock),
            shells: ShellInstanceCache::new(),
            registry,
        }
    }

    fn load(&self, yaml: &str) {
        self.loader
            .load_from_string(yaml, ValidationMode::None)
            .expect("load");
    }

    fn check(&self, want: &J, step: &str) {
        let stats = self.registry.stats();
        let want_stats = want["registry_stats"].as_object().unwrap();
        let names: Vec<&str> = stats.iter().map(|(n, _)| n.as_str()).collect();
        let want_names: Vec<&str> = want["registry_stats_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(names, want_names, "{step} stats keys");
        for (name, s) in &stats {
            let w = &want_stats[name];
            assert_eq!(s.size, usize_of(&w["size"]), "{step} {name} size");
            assert_eq!(
                s.max_size,
                usize_of(&w["max_size"]),
                "{step} {name} max_size"
            );
        }
        assert_eq!(
            self.registry.caches().len(),
            usize_of(&want["registry_cache_count"]),
            "{step} cache count"
        );
        assert_eq!(
            self.tools.len(),
            usize_of(&want["tool_size"]),
            "{step} tools"
        );
        assert_eq!(
            self.loader.cache_size(),
            usize_of(&want["definition_size"]),
            "{step} definitions"
        );
        assert_eq!(
            self.options.len(),
            usize_of(&want["options_size"]),
            "{step} options"
        );
        assert_eq!(
            self.shells.size(),
            usize_of(&want["shell_instances"]),
            "{step} shells"
        );
        // `generator_size` is not checked: `Tools::Generator` has no Rust
        // counterpart, so `ToolCache::clear` cannot clear it.
    }
}

fn state_of<'a>(steps: &'a [J], name: &str) -> &'a J {
    &steps
        .iter()
        .find(|s| s["step"] == name)
        .unwrap_or_else(|| panic!("step {name}"))["state"]
}

#[test]
fn registry_matches_ruby() {
    let g = golden();
    let steps = g["registry"].as_array().unwrap();
    let state = |name: &str| state_of(steps, name);
    let w = World::new();
    w.check(state("empty"), "empty");

    w.tools.set("t1", "tool1".to_owned());
    w.tools.set("t2", "tool2".to_owned());
    w.load("def1");
    w.options
        .insert(("t".to_owned(), "c".to_owned()), "opts1".to_owned());
    w.shells.instance_for("bash").unwrap();
    w.check(state("populated"), "populated");

    // Expired but unread entries are still counted by `stats`.
    w.clock.advance(3601.0);
    w.check(state("after_ttl_no_access"), "after_ttl_no_access");

    w.registry.clear_all();
    w.check(state("after_clear_all"), "after_clear_all");

    w.tools.set("t3", "tool3".to_owned());
    w.load("def2");
    w.tools.clear();
    w.check(state("after_tool_cache_clear"), "after_tool_cache_clear");

    w.tools.set("t4", "tool4".to_owned());
    // `ToolCache.clear_definition_cache` is `Definition::Loader.clear_cache`.
    w.loader.clear_cache(None).unwrap();
    w.check(
        state("after_clear_definition_cache"),
        "after_clear_definition_cache",
    );
}

struct Fixed(usize, usize);

impl RegisteredCache for Fixed {
    fn clear(&self) {}
    fn size(&self) -> usize {
        self.0
    }
    fn max_size(&self) -> usize {
        self.1
    }
}

#[test]
fn registry_register_replaces_in_place() {
    let mut r = CacheRegistry::standard(Arc::new(Fixed(1, 2)), Arc::new(Fixed(3, 4)));
    r.register(CacheRegistry::TOOL_CACHE, Arc::new(Fixed(5, 6)));
    r.register("extra", Arc::new(Fixed(7, 8)));
    let names: Vec<String> = r.stats().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["tool_cache", "definition_cache", "extra"]);
    assert_eq!(r.stats()[0].1.size, 5);
    assert_eq!(r.caches().len(), 3);
}

// ---------------------------------------------------------------- shells

fn unknown(r: Result<SharedShell, UnknownShellError>) -> UnknownShellError {
    match r {
        Ok(s) => panic!("expected UnknownShellError, got {}", s.name()),
        Err(e) => e,
    }
}

fn addr(s: &SharedShell) -> usize {
    Arc::as_ptr(s) as *const () as usize
}

#[test]
fn shell_instances_match_ruby() {
    let g = golden();
    let ic = &g["instance_cache"];
    let cache = ShellInstanceCache::new();
    let rows = ic["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 11);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let a = cache.instance_for(name).expect(name);
        let b = cache.instance_for(name).expect(name);
        assert_eq!(a.name(), row["shell_name"].as_str().unwrap(), "{name}");
        assert_eq!(
            Arc::ptr_eq(&a, &b),
            row["same_object"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(cache.size(), usize_of(&row["size_after"]), "{name}");
    }
    for (pair, (x, y)) in [
        ("unix_vs_bash_same_object", ("unix", "bash")),
        ("pwsh_vs_powershell_same_object", ("pwsh", "powershell")),
        ("windows_vs_cmd_same_object", ("windows", "cmd")),
    ] {
        let same = Arc::ptr_eq(
            &cache.instance_for(x).unwrap(),
            &cache.instance_for(y).unwrap(),
        );
        assert_eq!(same, ic["aliases"][pair].as_bool().unwrap(), "{pair}");
    }

    let mut checked = 0;
    for err in ic["errors"].as_array().unwrap() {
        // Ruby keys on symbols; a String name raises there but is just a
        // name here, so only symbol rows apply.
        if !err["is_symbol"].as_bool().unwrap() {
            continue;
        }
        let name = err["name"].as_str().unwrap();
        let before = cache.size();
        assert_eq!(before, usize_of(&err["size_before"]));
        let e = unknown(cache.instance_for(name));
        assert_eq!(
            UnknownShellError::RUBY_CLASS,
            err["error_class"].as_str().unwrap()
        );
        assert_eq!(e.to_string(), err["message"].as_str().unwrap());
        assert_eq!(cache.size(), usize_of(&err["size_after"]), "{name}");
        checked += 1;
    }
    assert_eq!(checked, 3);

    let clear = &ic["clear"];
    assert_eq!(cache.size(), usize_of(&clear["size_before"]));
    let old = cache.instance_for("zsh").unwrap();
    cache.clear();
    assert_eq!(cache.size(), usize_of(&clear["size_after"]));
    let fresh = cache.instance_for("zsh").unwrap();
    assert_eq!(
        !Arc::ptr_eq(&old, &fresh),
        clear["new_object_after_clear"].as_bool().unwrap()
    );
}

#[test]
fn concurrent_first_requests_share_one_instance() {
    let g = golden();
    let ic = &g["instance_cache"];
    let threads = usize_of(&ic["concurrent_threads"]);
    let cache = Arc::new(ShellInstanceCache::new());
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                let s = cache.instance_for("fish").unwrap();
                (addr(&s), s)
            })
        })
        .collect();
    let results: Vec<(usize, SharedShell)> =
        handles.into_iter().map(|h| h.join().unwrap()).collect();
    let distinct: HashSet<usize> = results.iter().map(|(a, _)| *a).collect();
    assert_eq!(distinct.len(), usize_of(&ic["concurrent_distinct_objects"]));
    assert_eq!(cache.size(), 1);
}

#[test]
fn shell_caches_are_isolated_and_accept_a_factory() {
    let a = ShellInstanceCache::new();
    let b = ShellInstanceCache::new();
    let sa = a.instance_for("bash").unwrap();
    let sb = b.instance_for("bash").unwrap();
    assert!(!Arc::ptr_eq(&sa, &sb));
    a.clear();
    assert_eq!(b.size(), 1);

    let custom = ShellInstanceCache::with_factory(|name| match name {
        "mybash" => builtin_shell("bash"),
        other => builtin_shell(other),
    });
    assert_eq!(custom.instance_for("mybash").unwrap().name(), "bash");
    assert_eq!(
        unknown(custom.instance_for("nushell")).to_string(),
        "Unknown shell: nushell"
    );
    assert_eq!(custom.size(), 1);
}
