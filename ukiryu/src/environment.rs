//! Port of `Ukiryu::Environment` (`lib/ukiryu/environment.rb`): an
//! immutable set of environment variables for one command execution.
//!
//! Every write returns a new [`Environment`]; the receiver never changes.
//! Variables keep Ruby `Hash` insertion order (`set` on an existing key keeps
//! its position, new keys go last), while equality and hashing ignore order
//! exactly as Ruby's `Hash#==` and `Hash#hash` do.
//!
//! PATH helpers follow the source's Unix rule on every platform: the
//! separator is always `:` and only the exact key `PATH` is read or written.
//! A Windows `Path` key is aliased to `PATH` only when an environment is
//! captured from a snapshot ([`Environment::from_snapshot`]), as
//! `Environment.from_env` does. The port does not substitute the native
//! Windows `;` separator.
//!
//! Capturing never mutates process state: [`Environment::from_snapshot`]
//! takes an explicit list of variables and [`Environment::from_process`]
//! only reads the current process environment.

use crate::shell::ShellFormatting;
use std::ffi::OsString;
use std::fmt;
use std::hash::{Hash, Hasher};

/// `Environment::SEPARATOR`.
pub const SEPARATOR: &str = ":";

/// The exact PATH key the source reads and writes.
pub const PATH_KEY: &str = "PATH";

/// An immutable, insertion-ordered set of environment variables.
#[derive(Clone, Default)]
pub struct Environment {
    vars: Vec<(String, String)>,
}

/// A process variable whose name or value is not valid Unicode. The source
/// only ever holds Ruby strings, so the port refuses such a variable instead
/// of replacing bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonUnicodeVariable {
    pub name: OsString,
}

impl fmt::Display for NonUnicodeVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "environment variable {:?} is not valid Unicode",
            self.name
        )
    }
}

impl std::error::Error for NonUnicodeVariable {}

impl Environment {
    /// `Environment.new({})` and `Environment.system`: no variables.
    pub fn new() -> Environment {
        Environment::default()
    }

    /// `Environment.system`: an empty environment that inherits nothing.
    pub fn system() -> Environment {
        Environment::default()
    }

    /// `Environment.new(hash)`. A repeated key behaves like a Ruby hash
    /// literal: the later value wins and keeps the first key's position.
    pub fn from_pairs<I, K, V>(pairs: I) -> Environment
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut vars = Vec::new();
        for (k, v) in pairs {
            upsert(&mut vars, k.into(), v.into());
        }
        Environment { vars }
    }

    /// `Environment.from_env` over an explicit snapshot of the process
    /// variables, in the order the snapshot lists them. When no exact
    /// `PATH` key exists, the first key whose ASCII upper case is `PATH`
    /// (for example Windows `Path`) is copied to a new trailing `PATH`
    /// entry; the original key stays.
    pub fn from_snapshot<I, K, V>(snapshot: I) -> Environment
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut env = Environment::from_pairs(snapshot);
        if !env.contains_key(PATH_KEY) {
            let alias = env
                .vars
                .iter()
                .find(|(k, _)| k.to_uppercase() == PATH_KEY)
                .map(|(_, v)| v.clone());
            if let Some(value) = alias {
                env.vars.push((PATH_KEY.to_string(), value));
            }
        }
        env
    }

    /// `Environment.from_env` for the current process: reads (never writes)
    /// the process environment and applies the [`Environment::from_snapshot`]
    /// rules.
    pub fn from_process() -> Result<Environment, NonUnicodeVariable> {
        let mut snapshot = Vec::new();
        for (name, value) in std::env::vars_os() {
            let key = name
                .into_string()
                .map_err(|name| NonUnicodeVariable { name })?;
            let value = value.into_string().map_err(|_| NonUnicodeVariable {
                name: OsString::from(key.clone()),
            })?;
            snapshot.push((key, value));
        }
        Ok(Environment::from_snapshot(snapshot))
    }

    /// `env[key]`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `env.key?(key)`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.vars.iter().any(|(k, _)| k == key)
    }

    /// `env.keys`, in insertion order.
    pub fn keys(&self) -> Vec<&str> {
        self.vars.iter().map(|(k, _)| k.as_str()).collect()
    }

    /// The variables in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// `env.to_h`: an independent copy of the variables.
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        self.vars.clone()
    }

    pub fn len(&self) -> usize {
        self.vars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// `env.set(key, value)`: the value is rendered with `Display`, as the
    /// source calls `value.to_s`.
    pub fn set(&self, key: &str, value: impl fmt::Display) -> Environment {
        let mut vars = self.vars.clone();
        upsert(&mut vars, key.to_string(), value.to_string());
        Environment { vars }
    }

    /// `env.delete(key)`; a missing key is not an error.
    pub fn delete(&self, key: &str) -> Environment {
        Environment {
            vars: self
                .vars
                .iter()
                .filter(|(k, _)| k != key)
                .cloned()
                .collect(),
        }
    }

    /// `env.merge(other_environment)`: `other` wins on shared keys.
    pub fn merge(&self, other: &Environment) -> Environment {
        self.merge_pairs(other.vars.iter().cloned())
    }

    /// `env.merge(hash)`.
    pub fn merge_pairs<I, K, V>(&self, pairs: I) -> Environment
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut vars = self.vars.clone();
        for (k, v) in pairs {
            upsert(&mut vars, k.into(), v.into());
        }
        Environment { vars }
    }

    /// `env.prepend_path(additions)`: joins the additions with `:` and puts
    /// them before the existing PATH. An absent or empty PATH becomes the
    /// joined additions alone.
    pub fn prepend_path<S: AsRef<str>>(&self, additions: &[S]) -> Environment {
        let added = join_path(additions);
        let existing = self.get(PATH_KEY).unwrap_or("");
        let path = if existing.is_empty() {
            added
        } else {
            format!("{added}{SEPARATOR}{existing}")
        };
        self.set(PATH_KEY, path)
    }

    /// `env.append_path(additions)`.
    pub fn append_path<S: AsRef<str>>(&self, additions: &[S]) -> Environment {
        let added = join_path(additions);
        let existing = self.get(PATH_KEY).unwrap_or("");
        let path = if existing.is_empty() {
            added
        } else {
            format!("{existing}{SEPARATOR}{added}")
        };
        self.set(PATH_KEY, path)
    }

    /// `env.remove_path(directory)`: drops every PATH entry equal to the
    /// directory (one trailing `/` removed) or below it. PATH is always
    /// written, so an absent PATH becomes an empty one, and empty trailing
    /// entries vanish because the source splits with `String#split`.
    pub fn remove_path(&self, directory: &str) -> Environment {
        let dir = directory.strip_suffix('/').unwrap_or(directory);
        let below = format!("{dir}/");
        let kept: Vec<&str> = ruby_split(self.get(PATH_KEY).unwrap_or(""))
            .into_iter()
            .filter(|part| *part != dir && !part.starts_with(&below))
            .collect();
        self.set(PATH_KEY, kept.join(SEPARATOR))
    }

    /// `env.path_contains?(directory)`: exact entry match.
    pub fn path_contains(&self, directory: &str) -> bool {
        match self.get(PATH_KEY) {
            Some(path) => ruby_split(path).contains(&directory),
            None => false,
        }
    }

    /// `env.path_array`.
    pub fn path_array(&self) -> Vec<String> {
        ruby_split(self.get(PATH_KEY).unwrap_or(""))
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// `env.inspect`: the key count only, never the values.
    pub fn inspect(&self) -> String {
        format!("#<Ukiryu::Environment keys={}>", self.vars.len())
    }

    /// A reference to variable `name` in the given shell's syntax
    /// (`Shell::Base#env_var`), for example `$NAME` or `%NAME%`.
    pub fn var_reference(shell: &dyn ShellFormatting, name: &str) -> String {
        shell.env_var(name)
    }

    fn sorted(&self) -> Vec<&(String, String)> {
        let mut v: Vec<&(String, String)> = self.vars.iter().collect();
        v.sort();
        v
    }
}

fn upsert(vars: &mut Vec<(String, String)>, key: String, value: String) {
    match vars.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = value,
        None => vars.push((key, value)),
    }
}

fn join_path<S: AsRef<str>>(additions: &[S]) -> String {
    additions
        .iter()
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join(SEPARATOR)
}

/// Ruby `String#split(':')`: interior and leading empty fields stay,
/// trailing empty fields are dropped, and an empty string has no fields.
fn ruby_split(value: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = value.split(SEPARATOR).collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

impl PartialEq for Environment {
    fn eq(&self, other: &Environment) -> bool {
        self.sorted() == other.sorted()
    }
}

impl Eq for Environment {}

impl Hash for Environment {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.sorted().hash(state);
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inspect())
    }
}
