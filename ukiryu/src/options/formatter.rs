//! `Ukiryu::OptionsBuilder::Formatter`: turns one option, flag or argument
//! value into command-line pieces.
//!
//! Source: `lib/ukiryu/options_builder/formatter.rb` (ukiryu/ukiryu ab6df353).
//! The shell instance Ruby passes to `format_option` and `format_flag` is
//! unused there, so it is not a parameter here.

use super::{OptionDef, OptionValue, OptionsError, TypeRef};
use crate::shell::ShellFormatting;

/// What `format_option` returns: `nil`, one string, or `[cli, value]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Formatted {
    Nil,
    One(String),
    Pair(String, String),
}

impl Formatted {
    /// `Array(formatted)`.
    pub fn into_pieces(self) -> Vec<String> {
        match self {
            Formatted::Nil => Vec::new(),
            Formatted::One(s) => vec![s],
            Formatted::Pair(a, b) => vec![a, b],
        }
    }
}

fn is_boolean(type_ref: Option<TypeRef<'_>>) -> bool {
    // `[:boolean, TrueClass, 'boolean'].include?(type)`; a class object cannot
    // come from a definition model.
    matches!(
        type_ref,
        Some(TypeRef::Str("boolean") | TypeRef::Sym("boolean"))
    )
}

/// `Array#join(separator)`: nested arrays join recursively, `nil` is empty.
pub fn ruby_join<V: OptionValue>(items: &[V], separator: &str) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        match item.as_array() {
            Some(inner) => out.push_str(&ruby_join(inner, separator)),
            None => out.push_str(&item.to_ruby_s()),
        }
    }
    out
}

/// `Formatter.format_option(opt_def, value, shell)`.
pub fn format_option_value<O: OptionDef + ?Sized, V: OptionValue>(
    def: &O,
    value: &V,
) -> Result<Formatted, OptionsError> {
    if is_boolean(def.type_ref()) {
        if value.is_nil() || value.is_false() {
            return Ok(Formatted::Nil);
        }
        return Ok(Formatted::One(def.cli().unwrap_or_default().to_owned()));
    }
    let cli = def.cli().unwrap_or_default();
    let style = def.format()?.map_or("double_dash_equals", |f| f.as_str());
    let spaced = matches!(style, "double_dash_space" | "single_dash_space");

    if let (Some(items), Some(separator)) = (value.as_array(), def.separator()) {
        let joined = ruby_join(items, separator);
        return Ok(if spaced {
            Formatted::Pair(cli.to_owned(), joined)
        } else {
            Formatted::One(format!("{cli}{joined}"))
        });
    }

    let value = value.to_ruby_s();
    Ok(match style {
        _ if spaced => Formatted::Pair(cli.to_owned(), value),
        "slash_colon" => Formatted::One(format!("{cli}:{value}")),
        "slash_space" => Formatted::One(format!("{cli} {value}")),
        // double_dash_equals, single_dash_equals and anything unknown
        _ => Formatted::One(format!("{cli}={value}")),
    })
}

/// [`format_option_value`] as the pieces `to_shell` appends.
pub fn format_option<O: OptionDef + ?Sized, V: OptionValue>(
    def: &O,
    value: &V,
) -> Result<Vec<String>, OptionsError> {
    format_option_value(def, value).map(Formatted::into_pieces)
}

/// `Formatter.format_flag(flag_def, shell)`.
pub fn format_flag<F: super::FlagDef + ?Sized>(def: &F) -> String {
    def.cli().unwrap_or_default().to_owned()
}

/// `Formatter.format_arg(value, arg_def, shell)`. Only a `:file` *symbol*
/// type reaches `format_path`; definition models store the type as the
/// string `"file"`, so model-built options never call it (pinned behaviour).
pub fn format_arg<V: OptionValue>(
    value: &V,
    type_ref: Option<TypeRef<'_>>,
    shell: &dyn ShellFormatting,
) -> String {
    if type_ref == Some(TypeRef::Sym("file")) {
        shell.format_path(&value.to_ruby_s())
    } else {
        value.to_ruby_s()
    }
}
