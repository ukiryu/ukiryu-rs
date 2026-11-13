//! `Ukiryu::Shell::InstanceCache`: one shared formatter per shell name.
//!
//! Source: `lib/ukiryu/shell/instance_cache.rb` and the built-in arm of
//! `Shell.class_for` in `lib/ukiryu/shell.rb` (ukiryu/ukiryu ab6df353).
//!
//! Entries are keyed by the requested name, so `unix` and `bash` are two
//! entries holding two different `Bash` instances, as in Ruby. Creation runs
//! under the cache lock, so concurrent first requests for a name all get the
//! same instance. A failed lookup stores nothing.
//!
//! Ruby keys on symbols and raises for a String name; Rust names are `&str`
//! and match the symbol spelling. Custom shells (Ruby `Shell::Registry`) are
//! supported by passing a factory to [`ShellInstanceCache::with_factory`].

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::shell::cmd::Cmd;
use crate::shell::posix::{Bash, Dash, Sh, Zsh};
use crate::shell::powershell::PowerShell;
use crate::shell::unix_special::{Fish, Tcsh};
use crate::shell::ShellFormatting;

/// A shell formatter shared between threads.
pub type SharedShell = Arc<dyn ShellFormatting + Send + Sync>;

/// `Ukiryu::Errors::UnknownShellError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownShellError {
    pub name: String,
}

impl UnknownShellError {
    /// The Ruby exception class name.
    pub const RUBY_CLASS: &'static str = "Ukiryu::Errors::UnknownShellError";
}

impl fmt::Display for UnknownShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unknown shell: {}", self.name)
    }
}

impl std::error::Error for UnknownShellError {}

/// Built-in `Shell.class_for(name).new`. Platform groups map to one shell:
/// `unix` to Bash, `windows` to Cmd, `powershell` and `pwsh` to PowerShell.
pub fn builtin_shell(name: &str) -> Result<SharedShell, UnknownShellError> {
    let shell: SharedShell = match name {
        "unix" | "bash" => Arc::new(Bash),
        "windows" | "cmd" => Arc::new(Cmd),
        "powershell" | "pwsh" => Arc::new(PowerShell),
        "zsh" => Arc::new(Zsh),
        "fish" => Arc::new(Fish),
        "sh" => Arc::new(Sh),
        "dash" => Arc::new(Dash),
        "tcsh" => Arc::new(Tcsh),
        _ => {
            return Err(UnknownShellError {
                name: name.to_owned(),
            })
        }
    };
    Ok(shell)
}

type Factory = dyn Fn(&str) -> Result<SharedShell, UnknownShellError> + Send + Sync;

/// `Ukiryu::Shell::InstanceCache`, owned by the caller.
pub struct ShellInstanceCache {
    factory: Box<Factory>,
    instances: Mutex<HashMap<String, SharedShell>>,
}

impl Default for ShellInstanceCache {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ShellInstanceCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellInstanceCache")
            .field("size", &self.size())
            .finish()
    }
}

impl ShellInstanceCache {
    /// A cache over the built-in shells ([`builtin_shell`]).
    pub fn new() -> Self {
        Self::with_factory(builtin_shell)
    }

    /// A cache whose instances come from `factory` (for example a custom
    /// registry consulted before [`builtin_shell`]).
    pub fn with_factory<F>(factory: F) -> Self
    where
        F: Fn(&str) -> Result<SharedShell, UnknownShellError> + Send + Sync + 'static,
    {
        Self {
            factory: Box::new(factory),
            instances: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SharedShell>> {
        self.instances.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `InstanceCache.instance_for(name)`.
    pub fn instance_for(&self, name: &str) -> Result<SharedShell, UnknownShellError> {
        let mut instances = self.lock();
        if let Some(found) = instances.get(name) {
            return Ok(Arc::clone(found));
        }
        let created = (self.factory)(name)?;
        instances.insert(name.to_owned(), Arc::clone(&created));
        Ok(created)
    }

    /// `InstanceCache.clear`.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// `InstanceCache.size`.
    pub fn size(&self) -> usize {
        self.lock().len()
    }
}
