//! `CommandBuilder#build_args` and its formatting helpers.
use super::sort::ruby_sort_by;
use super::spec::{ArgumentSpec, CommandSpec, FlagSpec, OptionSpec};
use super::{line_starts, name_sym, nil_to_sym, param, BuildContext, Params};
use crate::types::{self, ruby, Value};

/// `CommandBuilder#build_args`: the argv for a command and its parameters.
///
/// Order: subcommand, prefix flags, options, other flags, positional
/// arguments by position, post options, then the `last` argument. Values
/// that are missing or `nil` are skipped; flags fall back to their default.
pub fn build_args(
    command: &CommandSpec,
    params: &dyn Params,
    ctx: &BuildContext<'_>,
) -> types::Result<Vec<String>> {
    let mut args = Vec::new();

    if let Some(sub) = &command.subcommand {
        args.push(sub.clone());
    }

    let flags = command.flags.as_deref().unwrap_or_default();
    for flag in flags {
        let key = name_sym(&flag.name)?;
        if !is_prefix(flag) {
            continue;
        }
        push_flag(&mut args, flag, param(params, key));
    }

    for opt in command.options.as_deref().unwrap_or_default() {
        let key = name_sym(&opt.name)?;
        if let Some(value) = param(params, key) {
            push_all(&mut args, format_option(opt, value, ctx)?);
        }
    }

    for flag in flags {
        let key = name_sym(&flag.name)?;
        if is_prefix(flag) {
            continue;
        }
        push_flag(&mut args, flag, param(params, key));
    }

    let arguments = command.arguments.as_deref().unwrap_or_default();
    let last_arg = arguments.iter().find(|a| a.is_last());
    let regular: Vec<&ArgumentSpec> = arguments.iter().filter(|a| !a.is_last()).collect();
    // `sort_by(&:numeric_position)`, with MRI's order for equal positions.
    let regular = ruby_sort_by(regular, |a| a.numeric_position());

    for arg in regular {
        let key = name_sym(&arg.name)?;
        if let Some(value) = param(params, key) {
            push_argument(&mut args, arg, value, ctx)?;
        }
    }

    for opt in command.post_options.as_deref().unwrap_or_default() {
        let key = name_sym(&opt.name)?;
        if let Some(value) = param(params, key) {
            push_all(&mut args, format_option(opt, value, ctx)?);
        }
    }

    if let Some(arg) = last_arg {
        let key = name_sym(&arg.name)?;
        if let Some(value) = param(params, key) {
            push_argument(&mut args, arg, value, ctx)?;
        }
    }

    Ok(args)
}

fn is_prefix(flag: &FlagSpec) -> bool {
    flag.position_constraint.as_deref() == Some("prefix")
}

fn push_flag(args: &mut Vec<String>, flag: &FlagSpec, value: Option<&Value>) {
    let fallback = flag.default.map(Value::Bool);
    let value = value.or(fallback.as_ref());
    if let Some(cli) = format_flag(flag, value) {
        if !cli.is_empty() {
            args.push(cli);
        }
    }
}

/// Ruby `Array(result).each { |a| args << a unless a.nil? || a.empty? }`.
fn push_all(args: &mut Vec<String>, formatted: Vec<String>) {
    args.extend(formatted.into_iter().filter(|a| !a.is_empty()));
}

fn push_argument(
    args: &mut Vec<String>,
    arg: &ArgumentSpec,
    value: &Value,
    ctx: &BuildContext<'_>,
) -> types::Result<()> {
    if arg.variadic == Some(true) {
        let array = types::validate(value, &Value::sym("array"), arg, ctx.host)?;
        let items = match array {
            Value::Array(items) => items,
            other => vec![other],
        };
        for item in &items {
            args.push(format_arg(item, arg, ctx)?);
        }
    } else {
        args.push(format_arg(value, arg, ctx)?);
    }
    Ok(())
}

/// The type spec Ruby passes to `Type.validate`: `def.type || :string`.
fn type_spec(t: &Option<String>) -> Value {
    match t {
        Some(t) => Value::str(t.as_str()),
        None => Value::sym("string"),
    }
}

fn is_file(t: &Option<String>) -> bool {
    t.as_deref() == Some("file")
}

/// `CommandBuilder#format_arg`: validate, then `to_s` (through the shell's
/// `format_path` for `file` arguments).
pub fn format_arg(
    value: &Value,
    arg: &ArgumentSpec,
    ctx: &BuildContext<'_>,
) -> types::Result<String> {
    types::validate(value, &type_spec(&arg.r#type), arg, ctx.host)?;
    let text = ruby::to_s(value);
    Ok(if is_file(&arg.r#type) {
        ctx.shell.format_path(&text)
    } else {
        text
    })
}

/// `assignment_delimiter` after `:auto` detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// `--opt=value` (`equals`, and any unrecognised style).
    Equals,
    /// `-o value`, as two argv elements.
    Space,
    /// `/opt:value`.
    Colon,
    /// The CLI flag alone; the value is dropped.
    None,
}

impl Delimiter {
    fn from_sym(sym: &str, cli: &str) -> Self {
        match sym {
            "auto" => detect_delimiter(cli),
            "space" => Delimiter::Space,
            "colon" => Delimiter::Colon,
            "none" => Delimiter::None,
            _ => Delimiter::Equals,
        }
    }
}

/// `CommandBuilder#detect_delimiter`: `--x` equals, `-x` space, `/x` colon,
/// anything else equals. Like Ruby's `^`, a match may start on any line.
pub fn detect_delimiter(cli: &str) -> Delimiter {
    if line_starts(cli).any(|l| l.starts_with("--")) {
        Delimiter::Equals
    } else if line_starts(cli).any(|l| l.starts_with('-')) {
        Delimiter::Space
    } else if line_starts(cli).any(|l| l.starts_with('/')) {
        Delimiter::Colon
    } else {
        Delimiter::Equals
    }
}

/// `CommandBuilder#format_option`: zero, one or two argv elements (empty
/// strings are dropped by the caller, as in Ruby).
pub fn format_option(
    opt: &OptionSpec,
    value: &Value,
    ctx: &BuildContext<'_>,
) -> types::Result<Vec<String>> {
    types::validate(value, &type_spec(&opt.r#type), opt, ctx.host)?;

    let cli = opt.cli.clone().unwrap_or_default();
    if opt.r#type.as_deref() == Some("boolean") {
        return Ok(match value {
            Value::Nil | Value::Bool(false) => Vec::new(),
            _ => vec![cli],
        });
    }

    let delimiter_sym = opt.assignment_delimiter.as_deref().ok_or_else(nil_to_sym)?;
    let separator = opt.separator.as_deref().unwrap_or("=");
    let delimiter = Delimiter::from_sym(delimiter_sym, &cli);

    let file = is_file(&opt.r#type);
    let value_str = match value {
        Value::Symbol(s) => s.clone(),
        _ if file => ctx.shell.format_path(&ruby::to_s(value)),
        _ => ruby::to_s(value),
    };

    // `opt_def.separator || '='` is never nil, so every Array is joined.
    let rendered = match value {
        Value::Array(items) => items
            .iter()
            .map(|v| {
                let s = ruby::to_s(v);
                if file {
                    ctx.shell.format_path(&s)
                } else {
                    s
                }
            })
            .collect::<Vec<_>>()
            .join(separator),
        _ => value_str,
    };

    Ok(match delimiter {
        Delimiter::Equals => vec![format!("{cli}={rendered}")],
        Delimiter::Space => vec![cli, rendered],
        Delimiter::Colon => vec![format!("{cli}:{rendered}")],
        Delimiter::None => vec![cli],
    })
}

/// `CommandBuilder#format_flag`: the CLI flag unless the value is `nil` or
/// `false` (any other value, including `"false"` and `0`, enables it).
pub fn format_flag(flag: &FlagSpec, value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Nil) | Some(Value::Bool(false)) => None,
        Some(_) => Some(flag.cli.clone().unwrap_or_default()),
    }
}
