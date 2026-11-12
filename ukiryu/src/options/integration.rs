//! The options layer over the shared core types: command definitions as
//! command assembly reads them ([`CommandSpec`] and friends), values as
//! [`types::Value`], coercion through [`types::validate`], and the
//! `to_hash` hand-off to [`command_builder::Params`](crate::command_builder::Params).
//!
//! This is the path `Tool#execute` takes in Ruby: an options object is
//! converted with `OptionsBuilder.to_hash` and its hash goes to
//! `build_args`.

use super::{
    missing_format, ArgumentDef, Coercer, CommandDef, DefRef, FlagDef, OptionDef, OptionValue,
    Options, OptionsError, Position, TypeRef,
};
use crate::command_builder::{ArgumentSpec, CommandSpec, FlagSpec, OptionSpec};
use crate::types::{self, ruby, Host, Value};

/// The Ruby model class whose missing `format` attribute the pinned
/// formatter trips over.
const OPTION_MODEL: &str = "Ukiryu::Models::OptionDefinition";

impl OptionValue for Value {
    fn nil() -> Self {
        Value::Nil
    }
    fn from_bool(value: bool) -> Self {
        Value::Bool(value)
    }
    fn is_nil(&self) -> bool {
        Value::is_nil(self)
    }
    fn truthy(&self) -> bool {
        Value::truthy(self)
    }
    fn as_array(&self) -> Option<&[Self]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }
    fn to_ruby_s(&self) -> String {
        ruby::to_s(self)
    }
    fn from_array(items: Vec<Self>) -> Self {
        Value::Array(items)
    }
}

impl OptionDef for OptionSpec {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn cli(&self) -> Option<&str> {
        self.cli.as_deref()
    }
    fn type_ref(&self) -> Option<TypeRef<'_>> {
        self.r#type.as_deref().map(TypeRef::Str)
    }
    fn format(&self) -> Result<Option<TypeRef<'_>>, OptionsError> {
        Err(missing_format(OPTION_MODEL))
    }
    fn separator(&self) -> Option<&str> {
        self.separator.as_deref()
    }
}

impl ArgumentDef for ArgumentSpec {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn type_ref(&self) -> Option<TypeRef<'_>> {
        self.r#type.as_deref().map(TypeRef::Str)
    }
    fn position(&self) -> Position<'_> {
        self.position
            .as_deref()
            .map_or(Position::Nil, Position::Str)
    }
    fn variadic(&self) -> bool {
        self.variadic.unwrap_or(false)
    }
    fn min(&self) -> Option<i64> {
        self.min
    }
}

impl FlagDef for FlagSpec {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn cli(&self) -> Option<&str> {
        self.cli.as_deref()
    }
    fn default_truthy(&self) -> bool {
        self.default.unwrap_or(false)
    }
}

impl CommandDef for CommandSpec {
    type Option = OptionSpec;
    type Argument = ArgumentSpec;
    type Flag = FlagSpec;

    fn subcommand(&self) -> Option<&str> {
        self.subcommand.as_deref()
    }
    fn options(&self) -> &[OptionSpec] {
        self.options.as_deref().unwrap_or(&[])
    }
    fn flags(&self) -> &[FlagSpec] {
        self.flags.as_deref().unwrap_or(&[])
    }
    fn arguments(&self) -> &[ArgumentSpec] {
        self.arguments.as_deref().unwrap_or(&[])
    }
    fn post_options(&self) -> &[OptionSpec] {
        self.post_options.as_deref().unwrap_or(&[])
    }
}

impl From<types::Error> for OptionsError {
    fn from(e: types::Error) -> Self {
        OptionsError::new(e.class.ruby_name(), e.message)
    }
}

/// `Ukiryu::Type.validate(value, type, definition)` through the shared type
/// validator, reading the environment from `host`.
pub struct TypeCoercer<'h> {
    pub host: &'h dyn Host,
}

impl Coercer<CommandSpec, Value> for TypeCoercer<'_> {
    fn coerce(
        &self,
        value: &Value,
        type_ref: TypeRef<'_>,
        def: DefRef<'_, CommandSpec>,
    ) -> Result<Value, OptionsError> {
        let spec = match type_ref {
            TypeRef::Str(s) => Value::str(s),
            TypeRef::Sym(s) => Value::sym(s),
        };
        let result = match def {
            DefRef::Argument(a) => types::validate(value, &spec, a, self.host),
            DefRef::Option(o) => types::validate(value, &spec, o, self.host),
        };
        result.map_err(OptionsError::from)
    }
}

impl Options<CommandSpec, Value> {
    /// `OptionsBuilder.to_hash(options)` as the Symbol-keyed Ruby `Hash`
    /// that command assembly accepts as its params.
    pub fn to_params(&self) -> Value {
        Value::Hash(
            self.to_hash()
                .into_iter()
                .map(|(k, v)| (Value::Symbol(k), v))
                .collect(),
        )
    }
}
