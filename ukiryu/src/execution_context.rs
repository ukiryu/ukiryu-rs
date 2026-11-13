//! Port of `Ukiryu::ExecutionContext` (`lib/ukiryu/execution_context.rb`):
//! execution-scoped configuration passed explicitly instead of read from the
//! `Runtime` singleton.
//!
//! The source keeps the "current" context in a thread-local. The port keeps
//! it in a caller-owned [`ContextSlot`], so no global state is read or
//! written. Shells are registered concrete [`ShellName`]s (D2); a context
//! also carries the injected [`Clock`] for date-dependent extraction.
//!
//! [`ExecutionContext::run_settings`] is the context side of the decided
//! executor contract (D1, issues #20 and #29): environment, shell and
//! timeout are all explicit. An empty environment is allowed and does not
//! by itself mean an isolated process environment.

use crate::environment::Environment;
use crate::extractors::man::Today;
use crate::runtime::{Clock, Platform, Runtime, ShellError, ShellName};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// `options`: user-defined values in Ruby hash insertion order.
pub type ContextOptions = Vec<(String, Value)>;

/// One execution context. Built with [`ExecutionContext::new`] and the
/// `with_*` methods; never changed in place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionContext {
    platform: Option<Platform>,
    shell: Option<ShellName>,
    register_path: Option<String>,
    timeout_secs: Option<u64>,
    debug: bool,
    metrics: bool,
    options: ContextOptions,
    clock: Clock,
}

/// Values the source reads from `Register` and `Config` in `from_runtime`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeSettings {
    pub register_path: Option<String>,
    pub timeout_secs: Option<u64>,
    pub debug: bool,
    pub metrics: bool,
}

/// `merge(changes)`. `None` leaves a field unchanged; `Some(None)` clears
/// an optional field, as passing `nil` does in the source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextChanges {
    pub platform: Option<Option<Platform>>,
    pub shell: Option<Option<ShellName>>,
    pub register_path: Option<Option<String>>,
    pub timeout_secs: Option<Option<u64>>,
    pub debug: Option<bool>,
    pub metrics: Option<bool>,
    /// Merged into the existing options; later keys win.
    pub options: ContextOptions,
    pub clock: Option<Clock>,
}

/// The explicit inputs an executor run requires.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSettings {
    pub environment: Environment,
    pub shell: ShellName,
    pub timeout_secs: u64,
}

/// A required run setting the context does not hold. Messages are the
/// source executor's `ArgumentError` texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingSetting {
    Shell,
    Timeout,
}

impl fmt::Display for MissingSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MissingSetting::Shell => {
                "shell is required - specify :shell option (e.g., shell: :bash)"
            }
            MissingSetting::Timeout => {
                "timeout is required - specify :timeout option (e.g., timeout: 90)"
            }
        })
    }
}

impl std::error::Error for MissingSetting {}

impl ExecutionContext {
    /// `ExecutionContext.new` with every default: no platform, shell,
    /// register path or timeout; debug and metrics off; no options; the
    /// system clock at UTC.
    pub fn new() -> ExecutionContext {
        ExecutionContext::default()
    }

    /// `ExecutionContext.from_runtime`.
    pub fn from_runtime(
        runtime: &Runtime,
        settings: RuntimeSettings,
    ) -> Result<ExecutionContext, ShellError> {
        Ok(ExecutionContext {
            platform: Some(runtime.platform()),
            shell: Some(runtime.shell()?),
            register_path: settings.register_path,
            timeout_secs: settings.timeout_secs,
            debug: settings.debug,
            metrics: settings.metrics,
            options: Vec::new(),
            clock: runtime.clock(),
        })
    }

    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = Some(platform);
        self
    }

    pub fn with_shell(mut self, shell: ShellName) -> Self {
        self.shell = Some(shell);
        self
    }

    pub fn with_register_path(mut self, path: impl Into<String>) -> Self {
        self.register_path = Some(path.into());
        self
    }

    pub fn with_timeout_secs(mut self, timeout: u64) -> Self {
        self.timeout_secs = Some(timeout);
        self
    }

    pub fn with_debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    pub fn with_metrics(mut self, metrics: bool) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_options(mut self, options: ContextOptions) -> Self {
        self.options = Vec::new();
        merge_options(&mut self.options, options);
        self
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn platform(&self) -> Option<Platform> {
        self.platform
    }

    pub fn shell(&self) -> Option<ShellName> {
        self.shell
    }

    pub fn register_path(&self) -> Option<&str> {
        self.register_path.as_deref()
    }

    pub fn timeout_secs(&self) -> Option<u64> {
        self.timeout_secs
    }

    pub fn debug(&self) -> bool {
        self.debug
    }

    pub fn metrics(&self) -> bool {
        self.metrics
    }

    pub fn options(&self) -> &[(String, Value)] {
        &self.options
    }

    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// `Date.today` through the context's clock.
    pub fn today(&self) -> Today {
        self.clock.today()
    }

    pub fn on_platform(&self, platform: Platform) -> bool {
        self.platform == Some(platform)
    }

    pub fn using_shell(&self, shell: ShellName) -> bool {
        self.shell == Some(shell)
    }

    pub fn is_windows(&self) -> bool {
        self.on_platform(Platform::Windows)
    }

    pub fn is_macos(&self) -> bool {
        self.on_platform(Platform::Macos)
    }

    pub fn is_linux(&self) -> bool {
        self.on_platform(Platform::Linux)
    }

    /// `unix_shell?`; `false` without a shell.
    pub fn unix_shell(&self) -> bool {
        self.shell.is_some_and(ShellName::is_unix_shell)
    }

    /// `windows_shell?`; `false` without a shell.
    pub fn windows_shell(&self) -> bool {
        self.shell.is_some_and(ShellName::is_windows_shell)
    }

    /// `merge(changes)`: a new context; the receiver is unchanged.
    pub fn merge(&self, changes: &ContextChanges) -> ExecutionContext {
        let mut options = self.options.clone();
        merge_options(&mut options, changes.options.clone());
        ExecutionContext {
            platform: changes.platform.unwrap_or(self.platform),
            shell: changes.shell.unwrap_or(self.shell),
            register_path: changes
                .register_path
                .clone()
                .unwrap_or_else(|| self.register_path.clone()),
            timeout_secs: changes.timeout_secs.unwrap_or(self.timeout_secs),
            debug: changes.debug.unwrap_or(self.debug),
            metrics: changes.metrics.unwrap_or(self.metrics),
            options,
            clock: changes.clock.unwrap_or(self.clock),
        }
    }

    /// `inspect`.
    pub fn inspect(&self) -> String {
        format!("#<Ukiryu::ExecutionContext {self}>")
    }

    /// The explicit inputs for one executor run. `environment` is always
    /// supplied by the caller (an empty one is valid); shell and timeout
    /// must be set on the context.
    pub fn run_settings(&self, environment: Environment) -> Result<RunSettings, MissingSetting> {
        Ok(RunSettings {
            environment,
            shell: self.shell.ok_or(MissingSetting::Shell)?,
            timeout_secs: self.timeout_secs.ok_or(MissingSetting::Timeout)?,
        })
    }
}

fn merge_options(into: &mut ContextOptions, from: ContextOptions) {
    for (k, v) in from {
        match into.iter_mut().find(|(key, _)| *key == k) {
            Some(slot) => slot.1 = v,
            None => into.push((k, v)),
        }
    }
}

/// `to_s`: absent values render as empty text, as Ruby interpolates `nil`.
impl fmt::Display for ExecutionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ExecutionContext(platform={}, shell={}, register={})",
            self.platform.map(Platform::as_str).unwrap_or(""),
            self.shell.map(ShellName::as_str).unwrap_or(""),
            self.register_path.as_deref().unwrap_or("")
        )
    }
}

/// The source's `ExecutionContext.current` storage, owned by the caller.
#[derive(Clone, Debug, Default)]
pub struct ContextSlot {
    current: Option<Arc<ExecutionContext>>,
}

impl ContextSlot {
    pub fn new() -> ContextSlot {
        ContextSlot::default()
    }

    /// `ExecutionContext.current`: the stored context, creating it with
    /// `init` (the source's `from_runtime`) when none is stored. A failed
    /// `init` stores nothing.
    pub fn current_or_init<E>(
        &mut self,
        init: impl FnOnce() -> Result<ExecutionContext, E>,
    ) -> Result<Arc<ExecutionContext>, E> {
        if let Some(ctx) = &self.current {
            return Ok(Arc::clone(ctx));
        }
        let ctx = Arc::new(init()?);
        self.current = Some(Arc::clone(&ctx));
        Ok(ctx)
    }

    /// The stored context, without creating one.
    pub fn current(&self) -> Option<Arc<ExecutionContext>> {
        self.current.clone()
    }

    /// `ExecutionContext.current = context`.
    pub fn set_current(&mut self, context: Arc<ExecutionContext>) {
        self.current = Some(context);
    }

    /// `ExecutionContext.reset_current!`.
    pub fn reset(&mut self) {
        self.current = None;
    }

    /// `ExecutionContext.with_context(context) { ... }`: `f` runs with
    /// `context` stored; the previous value is restored afterwards, also
    /// when `f` panics.
    pub fn with_context<R>(
        &mut self,
        context: Arc<ExecutionContext>,
        f: impl FnOnce(&mut ContextSlot) -> R,
    ) -> R {
        struct Restore<'a> {
            slot: &'a mut ContextSlot,
            previous: Option<Option<Arc<ExecutionContext>>>,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.slot.current = self.previous.take().flatten();
            }
        }
        let previous = self.current.replace(context);
        let guard = Restore {
            slot: self,
            previous: Some(previous),
        };
        f(&mut *guard.slot)
    }
}
