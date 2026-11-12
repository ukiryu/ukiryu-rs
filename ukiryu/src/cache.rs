//! Runtime caches: the bounded tool cache, the shell instance cache and the
//! registry that clears and reports them.
//!
//! Source (ukiryu/ukiryu ab6df353): `lib/ukiryu/tool_cache.rb`,
//! `lib/ukiryu/shell/instance_cache.rb`, `lib/ukiryu/cache_registry.rb`.
//! The bounded LRU + TTL store itself (`lib/ukiryu/cache.rb`) is already
//! ported as [`crate::options::Cache`]; it is reused here, not duplicated.
//!
//! The Ruby versions are process-wide singletons. Here every cache is an
//! ordinary value the caller owns, so separate runtimes (and tests) stay
//! isolated and no global mutable state exists.

pub mod registry;
pub mod shell_instances;
pub mod tool;

pub use crate::options::cache::{Cache, CacheStats, Clock, SystemClock};
pub use registry::{CacheRegistry, DefinitionLoaderCache, RegisteredCache, RegistryCacheStats};
pub use shell_instances::{builtin_shell, SharedShell, ShellInstanceCache, UnknownShellError};
pub use tool::{cache_key_for, RuntimeDefaults, ToolCache, ToolKeyOptions};
