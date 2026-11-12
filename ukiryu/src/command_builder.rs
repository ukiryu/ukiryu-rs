//! Command argv and environment assembly.
//!
//! Port of `Ukiryu::CommandBuilder` (`lib/ukiryu/command_builder.rb`) from
//! ukiryu/ukiryu at ab6df353c6dc2d7faf7817a86aafcafa82579896:
//! `build_args`, `format_arg`, `format_option`, `detect_delimiter`,
//! `format_flag` and `build_env_vars`.
//!
//! The builder returns structured data: an argv vector and an ordered
//! environment. Nothing here quotes or escapes; the only shell call is
//! [`ShellFormatting::format_path`] for `file` typed values, as in Ruby.
//! Joining into a command line goes through [`ShellFormatting::join`] at the
//! shell boundary ([`Invocation::command_line`]). Choosing the executable and
//! running the process stay with the executor.
//!
//! Parameter values are [`types::Value`]s and every value is validated by
//! [`types::validate`], exactly where Ruby calls `Type.validate`.
//!
//! One deliberate difference from the pinned Ruby: positional arguments that
//! share a position keep their declaration order (a stable sort). Ruby's
//! `sort_by` is not stable, and for some larger argument lists MRI emits
//! equal-position arguments in a different order. The oracle tests list
//! those rows by id.
mod args;
mod env;
mod spec;

pub use args::{build_args, detect_delimiter, format_arg, format_flag, format_option, Delimiter};
pub use env::{build_env_vars, EnvVars};
pub use spec::{ArgumentSpec, CommandSpec, EnvVarSets, EnvVarSpec, FlagSpec, OptionSpec};

use crate::shell::ShellFormatting;
use crate::types::{self, Host, Value};
use std::collections::{BTreeMap, HashMap};

/// What a `CommandBuilder` instance reads from its tool: the shell used for
/// `file` path formatting (`@shell`), the platform used for env var
/// restrictions (`@platform`), and the host that type validation reads.
#[derive(Clone, Copy)]
pub struct BuildContext<'a> {
    pub shell: &'a dyn ShellFormatting,
    pub platform: &'a str,
    pub host: &'a dyn Host,
}

/// The parameter hash `build_args` receives (`params[name.to_sym]`).
///
/// A missing key and a `nil` value behave the same everywhere in the
/// builder, so `get` may return either `None` or `Some(Value::Nil)`.
pub trait Params {
    fn get(&self, name: &str) -> Option<&Value>;
}

impl Params for HashMap<String, Value> {
    fn get(&self, name: &str) -> Option<&Value> {
        HashMap::get(self, name)
    }
}

impl Params for BTreeMap<String, Value> {
    fn get(&self, name: &str) -> Option<&Value> {
        BTreeMap::get(self, name)
    }
}

/// A Ruby `Hash` with Symbol keys, as `OptionsBuilder` hands it over.
impl Params for Value {
    fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Hash(pairs) => pairs
                .iter()
                .find(|(k, _)| matches!(k, Value::Symbol(s) if s == name))
                .map(|(_, v)| v),
            _ => None,
        }
    }
}

/// A built command: the executable chosen by the caller, the argv the
/// command definition produced, and its environment additions.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub executable: String,
    pub args: Vec<String>,
    pub env: EnvVars,
}

impl Invocation {
    /// The command line for a shell, quoted only by that shell's `join`.
    pub fn command_line(&self, shell: &dyn ShellFormatting) -> String {
        shell.join(&self.executable, &self.args)
    }
}

/// `build_args` then `build_env_vars`, in the order `Tool#execute` runs them.
pub fn assemble(
    executable: impl Into<String>,
    command: &CommandSpec,
    env_var_sets: Option<&EnvVarSets>,
    params: &dyn Params,
    ctx: &BuildContext<'_>,
) -> types::Result<Invocation> {
    let args = build_args(command, params, ctx)?;
    let env = build_env_vars(command, env_var_sets, params, ctx.platform);
    Ok(Invocation {
        executable: executable.into(),
        args,
        env,
    })
}

/// `NoMethodError` raised by `nil.to_sym` (a definition without a name).
fn nil_to_sym() -> types::Error {
    types::Error::new(
        types::ErrorClass::NoMethod,
        "undefined method 'to_sym' for nil",
    )
}

/// Ruby `name_sym` (`name.to_sym`).
fn name_sym(name: &Option<String>) -> types::Result<&str> {
    name.as_deref().ok_or_else(nil_to_sym)
}

/// Ruby `params[key]` treating a missing key and `nil` alike.
fn param<'p>(params: &'p dyn Params, key: &str) -> Option<&'p Value> {
    params.get(key).filter(|v| !v.is_nil())
}

/// Every line start of a Ruby string, where `^` can match.
fn line_starts(s: &str) -> impl Iterator<Item = &str> {
    s.split('\n')
}
