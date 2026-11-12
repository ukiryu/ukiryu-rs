//! Structured command options: state, formatting, validation and the
//! per-command options cache.
//!
//! Source: `lib/ukiryu/options_builder.rb`, `options_builder/formatter.rb`,
//! `options_builder/validator.rb` and the `Ukiryu::Cache` it uses
//! (ukiryu/ukiryu ab6df353). The Ruby module builds an anonymous class per
//! command with one accessor per argument, option, flag and post-option. Here
//! that class is [`OptionsClass`] and an instance is [`Options`]; the dynamic
//! Ruby class exposure itself belongs to the later Ruby adapter.
//!
//! This module does not own the command models, the type coercion rules or
//! shell quoting. It reads definitions through [`CommandDef`] and its
//! companion traits (implemented for the typed command models), coerces
//! assigned values through a [`Coercer`] (backed by the type validator), keeps
//! values as any [`OptionValue`], and joins through
//! [`ShellFormatting`](crate::shell::ShellFormatting).
//!
//! Pinned behaviour kept on purpose, each covered by oracle captures:
//! - assigning `nil` to an option or post-option keeps the previous value;
//!   flags store Ruby truthiness; arguments always go through coercion;
//! - output order is subcommand, options, flags (unset flags use their
//!   default), regular arguments sorted by their *string* position (so `"10"`
//!   sorts before `"2"`; ties keep definition order), post-options, then the
//!   `last` argument; empty formatted pieces are dropped;
//! - the typed option model has no `format` attribute, so formatting a set
//!   non-boolean option fails with Ruby's `NoMethodError` (see
//!   [`OptionDef::format`]);
//! - the setter for a name is the last one Ruby defined (arguments, options,
//!   flags, post-options, in that order) and every accessor of that name
//!   shares one value slot.

pub mod cache;
pub mod formatter;
pub mod integration;
pub mod validator;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::shell::ShellFormatting;

pub use cache::{Cache, CacheStats, Clock, OptionsClassCache, SystemClock};

/// A Ruby error surfaced by the options layer: the Ruby class name and its
/// message, exactly as the pinned implementation raises them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsError {
    pub class: String,
    pub message: String,
}

impl OptionsError {
    pub fn new(class: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            message: message.into(),
        }
    }

    /// `Ukiryu::Errors::ValidationError`.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("Ukiryu::Errors::ValidationError", message)
    }

    fn argument(message: impl Into<String>) -> Self {
        Self::new("ArgumentError", message)
    }

    fn no_method(message: impl Into<String>) -> Self {
        Self::new("NoMethodError", message)
    }
}

impl fmt::Display for OptionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}

impl std::error::Error for OptionsError {}

/// A Ruby type designator as the formatter compares it: models carry
/// strings (`"file"`), duck-typed callers may pass symbols (`:file`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRef<'a> {
    Str(&'a str),
    Sym(&'a str),
}

impl<'a> TypeRef<'a> {
    pub fn as_str(&self) -> &'a str {
        match self {
            TypeRef::Str(s) | TypeRef::Sym(s) => s,
        }
    }
}

/// An argument position as stored on the definition (`attribute :position,
/// :string`), or an integer from a duck-typed definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position<'a> {
    Nil,
    Int(i64),
    Str(&'a str),
    Sym(&'a str),
}

/// A value held in an options slot. Implemented by the shared Ruby-value
/// model; this module needs only these Ruby semantics.
pub trait OptionValue: Clone {
    /// `nil`.
    fn nil() -> Self;
    /// `true` / `false`.
    fn from_bool(value: bool) -> Self;
    fn is_nil(&self) -> bool;
    /// Ruby truthiness (everything except `nil` and `false`).
    fn truthy(&self) -> bool;
    /// `value == false`.
    fn is_false(&self) -> bool {
        !self.truthy() && !self.is_nil()
    }
    /// The elements when the value is an `Array`.
    fn as_array(&self) -> Option<&[Self]>;
    /// `value.to_s` (`nil.to_s` is `""`, `Array#to_s` is `inspect`).
    fn to_ruby_s(&self) -> String;
    /// An `Array` of these elements.
    fn from_array(items: Vec<Self>) -> Self;
}

/// The option model fields the options layer reads (`OptionDefinition`).
pub trait OptionDef {
    fn name(&self) -> Option<&str>;
    fn cli(&self) -> Option<&str>;
    /// `opt_def.type` (`nil` only when explicitly null).
    fn type_ref(&self) -> Option<TypeRef<'_>>;
    /// `opt_def.format`. The typed `OptionDefinition` has no such attribute,
    /// and Ruby's private `Kernel#format` answers instead, so the pinned
    /// implementation raises `NoMethodError` here; model implementations
    /// return [`missing_format`]. Duck-typed definitions return their style.
    fn format(&self) -> Result<Option<TypeRef<'_>>, OptionsError>;
    fn separator(&self) -> Option<&str>;
}

/// The error Ruby raises for `opt_def.format` on a typed option model.
pub fn missing_format(model_class: &str) -> OptionsError {
    OptionsError::no_method(format!(
        "private method 'format' called for an instance of {model_class}"
    ))
}

/// The argument model fields the options layer reads (`ArgumentDefinition`).
pub trait ArgumentDef {
    fn name(&self) -> Option<&str>;
    fn type_ref(&self) -> Option<TypeRef<'_>>;
    fn position(&self) -> Position<'_>;
    /// Ruby truthiness of `variadic`.
    fn variadic(&self) -> bool;
    fn min(&self) -> Option<i64>;
}

/// The flag model fields the options layer reads (`FlagDefinition`).
pub trait FlagDef {
    fn name(&self) -> Option<&str>;
    fn cli(&self) -> Option<&str>;
    /// Ruby truthiness of `default`.
    fn default_truthy(&self) -> bool;
}

/// The command model fields the options layer reads (`CommandDefinition`).
/// Absent and `nil` lists both read as empty (`list || []`).
pub trait CommandDef {
    type Option: OptionDef;
    type Argument: ArgumentDef;
    type Flag: FlagDef;

    fn subcommand(&self) -> Option<&str>;
    fn options(&self) -> &[Self::Option];
    fn flags(&self) -> &[Self::Flag];
    fn arguments(&self) -> &[Self::Argument];
    fn post_options(&self) -> &[Self::Option];
}

/// The definition a setter coerces against: what Ruby passes as the third
/// argument of `Ukiryu::Type.validate`.
pub enum DefRef<'a, C: CommandDef + ?Sized> {
    Argument(&'a C::Argument),
    Option(&'a C::Option),
}

/// `Ukiryu::Type.validate(value, type, definition)`: implemented over the
/// shared type validator.
pub trait Coercer<C: CommandDef + ?Sized, V> {
    fn coerce(
        &self,
        value: &V,
        type_ref: TypeRef<'_>,
        def: DefRef<'_, C>,
    ) -> Result<V, OptionsError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Argument(usize),
    Option(usize),
    Flag,
    PostOption(usize),
}

/// The options "class" for one command: the command definition plus the
/// accessor table Ruby's `define_method` calls would produce.
pub struct OptionsClass<C> {
    tool_name: String,
    command_name: String,
    def: Arc<C>,
    setters: HashMap<String, Kind>,
}

impl<C: CommandDef> OptionsClass<C> {
    /// `OptionsBuilder.create_options_class`. Fails like Ruby when a
    /// definition has no name.
    pub fn new(
        tool_name: impl Into<String>,
        command_name: impl Into<String>,
        def: Arc<C>,
    ) -> Result<Self, OptionsError> {
        let mut setters = HashMap::new();
        let nameless = || OptionsError::new("TypeError", "nil is not a symbol nor a string");
        for (i, a) in def.arguments().iter().enumerate() {
            setters.insert(a.name().ok_or_else(nameless)?.to_owned(), Kind::Argument(i));
        }
        for (i, o) in def.options().iter().enumerate() {
            setters.insert(o.name().ok_or_else(nameless)?.to_owned(), Kind::Option(i));
        }
        for f in def.flags() {
            setters.insert(f.name().ok_or_else(nameless)?.to_owned(), Kind::Flag);
        }
        for (i, o) in def.post_options().iter().enumerate() {
            setters.insert(
                o.name().ok_or_else(nameless)?.to_owned(),
                Kind::PostOption(i),
            );
        }
        Ok(Self {
            tool_name: tool_name.into(),
            command_name: command_name.into(),
            def,
            setters,
        })
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn command_name(&self) -> &str {
        &self.command_name
    }

    pub fn command_def(&self) -> &C {
        &self.def
    }

    /// Whether an accessor of this name exists.
    pub fn has_accessor(&self, name: &str) -> bool {
        self.setters.contains_key(name)
    }
}

/// One options object (`options_class.new`).
pub struct Options<C, V> {
    class: Arc<OptionsClass<C>>,
    values: HashMap<String, V>,
    extra_args: Option<V>,
}

fn undefined(name: &str) -> OptionsError {
    OptionsError::no_method(format!(
        "undefined method '{name}' for an instance of #<Class:ANON>"
    ))
}

impl<C: CommandDef, V: OptionValue> Options<C, V> {
    pub fn new(class: Arc<OptionsClass<C>>) -> Self {
        Self {
            class,
            values: HashMap::new(),
            extra_args: None,
        }
    }

    pub fn class(&self) -> &Arc<OptionsClass<C>> {
        &self.class
    }

    /// The reader for `name` (`nil` when never assigned).
    pub fn get(&self, name: &str) -> Result<V, OptionsError> {
        if !self.class.has_accessor(name) {
            return Err(undefined(name));
        }
        Ok(self.slot(name))
    }

    fn slot(&self, name: &str) -> V {
        self.values.get(name).cloned().unwrap_or_else(V::nil)
    }

    /// The writer for `name`. On error the slot keeps its previous value.
    pub fn set(
        &mut self,
        name: &str,
        value: V,
        coercer: &dyn Coercer<C, V>,
    ) -> Result<(), OptionsError> {
        let kind = *self
            .class
            .setters
            .get(name)
            .ok_or_else(|| undefined(&format!("{name}=")))?;
        let def = &*self.class.def;
        let stored = match kind {
            Kind::Argument(i) => {
                let a = &def.arguments()[i];
                let ty = a.type_ref().unwrap_or(TypeRef::Sym("string"));
                match value.as_array() {
                    Some(items) if a.variadic() => {
                        let mut out = Vec::with_capacity(items.len());
                        for item in items {
                            out.push(coercer.coerce(item, ty, DefRef::Argument(a))?);
                        }
                        V::from_array(out)
                    }
                    _ => coercer.coerce(&value, ty, DefRef::Argument(a))?,
                }
            }
            Kind::Option(i) | Kind::PostOption(i) => {
                if value.is_nil() {
                    return Ok(());
                }
                let o = match kind {
                    Kind::Option(_) => &def.options()[i],
                    _ => &def.post_options()[i],
                };
                let ty = o.type_ref().unwrap_or(TypeRef::Sym("string"));
                coercer.coerce(&value, ty, DefRef::Option(o))?
            }
            Kind::Flag => V::from_bool(value.truthy()),
        };
        self.values.insert(name.to_owned(), stored);
        Ok(())
    }

    /// `extra_args` exists only on classes that define it (the tool class
    /// generator); `None` means the accessor is absent or `nil`.
    pub fn extra_args(&self) -> Option<&V> {
        self.extra_args.as_ref()
    }

    pub fn set_extra_args(&mut self, value: Option<V>) {
        self.extra_args = value.filter(|v| !v.is_nil());
    }

    /// `OptionsBuilder.to_hash(options)`: the params handed to command
    /// assembly. Order is Ruby hash insertion order (first definition of a
    /// name wins the position, the shared slot supplies the value).
    pub fn to_hash(&self) -> Vec<(String, V)> {
        let def = &*self.class.def;
        let mut out: Vec<(String, V)> = Vec::new();
        let mut put = |name: &str, value: V| {
            if let Some(entry) = out.iter_mut().find(|(k, _)| k == name) {
                entry.1 = value;
            } else {
                out.push((name.to_owned(), value));
            }
        };
        for a in def.arguments() {
            let name = a.name().unwrap_or_default();
            let v = self.slot(name);
            if !v.is_nil() {
                put(name, v);
            }
        }
        for o in def.options() {
            let name = o.name().unwrap_or_default();
            let v = self.slot(name);
            if !v.is_nil() {
                put(name, v);
            }
        }
        for f in def.flags() {
            let name = f.name().unwrap_or_default();
            let v = self.slot(name);
            if v.truthy() {
                put(name, v);
            }
        }
        for o in def.post_options() {
            let name = o.name().unwrap_or_default();
            let v = self.slot(name);
            if !v.is_nil() {
                put(name, v);
            }
        }
        if let Some(extra) = &self.extra_args {
            put("extra_args", extra.clone());
        }
        out
    }

    /// The argument vector `to_shell` hands to the shell's `join`.
    pub fn argv(&self, shell: &dyn ShellFormatting) -> Result<Vec<String>, OptionsError> {
        let def = &*self.class.def;
        let mut args = Vec::new();
        if let Some(sub) = def.subcommand() {
            args.push(sub.to_owned());
        }
        let push_all = |args: &mut Vec<String>, pieces: Vec<String>| {
            args.extend(pieces.into_iter().filter(|p| !p.is_empty()));
        };
        for o in def.options() {
            let v = self.slot(o.name().unwrap_or_default());
            if v.is_nil() {
                continue;
            }
            push_all(&mut args, formatter::format_option(o, &v)?);
        }
        for f in def.flags() {
            let v = self.slot(f.name().unwrap_or_default());
            let on = if v.is_nil() {
                f.default_truthy()
            } else {
                v.truthy()
            };
            if on {
                push_all(&mut args, vec![formatter::format_flag(f)]);
            }
        }
        let is_last =
            |a: &C::Argument| matches!(a.position(), Position::Str("last") | Position::Sym("last"));
        let mut regular: Vec<&C::Argument> =
            def.arguments().iter().filter(|a| !is_last(a)).collect();
        sort_by_position(&mut regular)?;
        for a in regular {
            let v = self.slot(a.name().unwrap_or_default());
            if v.is_nil() {
                continue;
            }
            self.push_argument(&mut args, a, &v, shell);
        }
        for o in def.post_options() {
            let v = self.slot(o.name().unwrap_or_default());
            if v.is_nil() {
                continue;
            }
            push_all(&mut args, formatter::format_option(o, &v)?);
        }
        if let Some(last) = def.arguments().iter().find(|a| is_last(a)) {
            let v = self.slot(last.name().unwrap_or_default());
            if v.truthy() {
                self.push_argument(&mut args, last, &v, shell);
            }
        }
        Ok(args)
    }

    fn push_argument(
        &self,
        args: &mut Vec<String>,
        a: &C::Argument,
        v: &V,
        shell: &dyn ShellFormatting,
    ) {
        let ty = a.type_ref();
        if a.variadic() {
            // `Array(value)`
            match v.as_array() {
                Some(items) => {
                    for item in items {
                        args.push(formatter::format_arg(item, ty, shell));
                    }
                }
                None => args.push(formatter::format_arg(v, ty, shell)),
            }
        } else {
            args.push(formatter::format_arg(v, ty, shell));
        }
    }

    /// `options.to_shell(shell_type:)`: `shell.join(*argv)`. Ruby's `join`
    /// takes the first element as the executable, so an empty vector is an
    /// `ArgumentError`.
    pub fn to_shell(&self, shell: &dyn ShellFormatting) -> Result<String, OptionsError> {
        let argv = self.argv(shell)?;
        let Some((first, rest)) = argv.split_first() else {
            return Err(OptionsError::argument(
                "wrong number of arguments (given 0, expected 1+)",
            ));
        };
        Ok(shell.join(first, rest))
    }

    /// `options.validate!`.
    pub fn validate(&self) -> Result<(), OptionsError> {
        validator::validate(self.class.command_def(), |name| self.slot(name))
    }
}

/// `sort_by { |a| pos.is_a?(Integer) ? pos : (pos || 99) }` on the
/// non-`last` arguments.
///
/// Ruby's `sort_by` on the pinned Linux runtime is glibc's merge sort: stable,
/// and its comparison order decides which mixed pair raises first. This
/// replays that merge sort so both the order and the `ArgumentError` text
/// (`rb_cmperr`) match.
fn sort_by_position<A: ArgumentDef>(args: &mut [&A]) -> Result<(), OptionsError> {
    let keys: Vec<Key<'_>> = args
        .iter()
        .map(|a| match a.position() {
            Position::Int(i) => Key::Int(i),
            Position::Nil => Key::Int(99),
            Position::Str(s) => Key::Str(s),
            Position::Sym(s) => Key::Sym(s),
        })
        .collect();
    let mut idx: Vec<usize> = (0..args.len()).collect();
    merge_sort(&mut idx, &|i, j| keys[i].ruby_cmp(&keys[j]))?;
    let sorted: Vec<&A> = idx.into_iter().map(|i| args[i]).collect();
    args.copy_from_slice(&sorted);
    Ok(())
}

#[derive(Clone, Copy)]
enum Key<'a> {
    Int(i64),
    Str(&'a str),
    Sym(&'a str),
}

impl Key<'_> {
    /// `a <=> b` as `sort_by` uses it, raising `rb_cmperr(a, b)` when nil.
    fn ruby_cmp(&self, other: &Key<'_>) -> Result<std::cmp::Ordering, OptionsError> {
        match (self, other) {
            (Key::Int(x), Key::Int(y)) => Ok(x.cmp(y)),
            (Key::Str(x), Key::Str(y)) | (Key::Sym(x), Key::Sym(y)) => {
                Ok(x.as_bytes().cmp(y.as_bytes()))
            }
            _ => {
                // Special constants (Integer, Symbol) are shown by inspect.
                let rhs = match other {
                    Key::Int(i) => i.to_string(),
                    Key::Sym(s) => symbol_inspect(s),
                    Key::Str(_) => "String".to_owned(),
                };
                Err(OptionsError::argument(format!(
                    "comparison of {} with {rhs} failed",
                    self.class_name()
                )))
            }
        }
    }

    fn class_name(&self) -> &'static str {
        match self {
            Key::Int(_) => "Integer",
            Key::Str(_) => "String",
            Key::Sym(_) => "Symbol",
        }
    }
}

fn symbol_inspect(s: &str) -> String {
    let simple = s
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if simple {
        format!(":{s}")
    } else {
        format!(":{s:?}")
    }
}

/// glibc `msort_with_tmp`: split at `n / 2`, sort both halves, merge taking
/// the left element while `cmp(left, right) <= 0`.
fn merge_sort<F>(v: &mut [usize], cmp: &F) -> Result<(), OptionsError>
where
    F: Fn(usize, usize) -> Result<std::cmp::Ordering, OptionsError>,
{
    let n = v.len();
    if n <= 1 {
        return Ok(());
    }
    let n1 = n / 2;
    merge_sort(&mut v[..n1], cmp)?;
    merge_sort(&mut v[n1..], cmp)?;
    let (left, right) = (v[..n1].to_vec(), v[n1..].to_vec());
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        if cmp(left[i], right[j])? != std::cmp::Ordering::Greater {
            v[k] = left[i];
            i += 1;
        } else {
            v[k] = right[j];
            j += 1;
        }
        k += 1;
    }
    for &x in left[i..].iter().chain(&right[j..]) {
        v[k] = x;
        k += 1;
    }
    Ok(())
}
