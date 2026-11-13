//! `resolve` kind: version extraction (`ukiryu::version_detection`), Versionian
//! range matching (`ukiryu::version_ranges`) and shell detection
//! (`ukiryu::runtime::detect_shell` over an explicit `Environment`).
use ukiryu::environment::Environment;
use ukiryu::errors::ErrorKind;
use ukiryu::runtime::{detect_shell, Platform, ShellError, ShellName};
use ukiryu::types::Value;
use ukiryu::version_detection::{extract_version_from_pattern, PatternError};
use ukiryu::version_ranges::{
    resolve, Bounds, ErrorClass, RangeError, RangeKind, SpecValue, VersionRange,
};

use super::codec;
use super::harness::{input_str, Ctx, Outcome, RowError};
use super::json::Oj;

/// `Tool::Loader.extract_version_from_pattern(output, pattern)`.
///
/// A `$bytes` output is a BINARY Ruby string; the captured group of a BINARY
/// string is BINARY too, so it is encoded back as `$bytes`.
pub fn extract_version(input: &Oj, _ctx: &Ctx) -> Outcome {
    let output = input.get("output").filter(|o| !matches!(o, Oj::Null));
    let binary = output.is_some_and(|o| o.tag().is_some());
    let raw = output
        .map(codec::bytes)
        .transpose()
        .map_err(RowError::harness)?;
    let text = raw
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| {
            RowError::harness(
                "no core API: extract_version_from_pattern takes &str; output is non-UTF-8 bytes",
            )
        })?;
    let pattern = input
        .get("pattern")
        .filter(|p| !matches!(p, Oj::Null))
        .map(|p| {
            p.str()
                .ok_or_else(|| RowError::harness("pattern must be a string or null"))
        })
        .transpose()?;
    match extract_version_from_pattern(text, pattern) {
        Ok(None) => Ok(Oj::Null),
        Ok(Some(v)) if binary => Ok(Oj::tagged(
            "$bytes",
            Oj::Str(v.bytes().map(|x| format!("{x:02x}")).collect()),
        )),
        Ok(Some(v)) => Ok(Oj::Str(v)),
        Err(PatternError::RubyInvalid { reason, .. }) => Err(RowError::new("RegexpError", reason)),
        Err(e @ PatternError::Unsupported { .. }) => Err(RowError::harness(e.to_string())),
    }
}

/// `Versionian::VersionRange.new(range.to_sym, VersionSchemeResolver.resolve(scheme),
/// **bounds).matches?(version)`.
pub fn range(input: &Oj, _ctx: &Ctx) -> Outcome {
    let scheme_spec = spec_value(
        input
            .get("scheme")
            .ok_or_else(|| RowError::harness("resolve.range needs scheme"))?,
    )?;
    let scheme = resolve(&scheme_spec).map_err(range_error)?;
    let kind = RangeKind::from_name(input_str(input, "range")?).map_err(range_error)?;
    let mut bounds = Bounds::new();
    match input.get("bounds") {
        Some(Oj::Obj(pairs)) => {
            for (k, v) in pairs {
                bounds = bounds.with(k, spec_value(v)?);
            }
        }
        _ => return Err(RowError::harness("resolve.range bounds must be an object")),
    }
    let version = input_str(input, "version")?;
    VersionRange::new(kind, &scheme, bounds)
        .and_then(|r| r.matches(version))
        .map(Oj::Bool)
        .map_err(range_error)
}

fn range_error(e: RangeError) -> RowError {
    match e.class {
        ErrorClass::Unsupported => RowError::harness(format!("unsupported: {}", e.message)),
        class => RowError::new(class.ruby_name(), e.message),
    }
}

/// A corpus input as the range module's Ruby value shape.
fn spec_value(j: &Oj) -> Result<SpecValue, RowError> {
    from_value(codec::decode(j).map_err(RowError::harness)?)
}

fn from_value(v: Value) -> Result<SpecValue, RowError> {
    Ok(match v {
        Value::Nil => SpecValue::Nil,
        Value::Bool(b) => SpecValue::Bool(b),
        Value::Integer(i) => SpecValue::Int(i),
        Value::Float(f) => SpecValue::Float(f),
        Value::String(s) => SpecValue::Str(s),
        Value::Symbol(s) => SpecValue::Sym(s),
        Value::Array(items) => SpecValue::Array(
            items
                .into_iter()
                .map(from_value)
                .collect::<Result<_, _>>()?,
        ),
        Value::Hash(pairs) => SpecValue::Map(
            pairs
                .into_iter()
                .map(|(k, x)| Ok((from_value(k)?, from_value(x)?)))
                .collect::<Result<_, RowError>>()?,
        ),
        other => {
            return Err(RowError::harness(format!(
                "no SpecValue form for {other:?}"
            )))
        }
    })
}

/// `Shell.detect` / `Shell.shell_class` over the README sandbox environment
/// with the row's `env` applied (`null` unsets). The process environment is
/// never read or written.
pub fn shell_detect(input: &Oj, ctx: &Ctx) -> Outcome {
    let platform = Platform::from_name(ctx.platform)
        .ok_or_else(|| RowError::harness(format!("unknown platform {}", ctx.platform)))?;
    let mut env = Environment::from_pairs([
        ("HOME", ctx.home.as_str()),
        ("TZ", "UTC"),
        ("LANG", "C.UTF-8"),
        ("LC_ALL", "C.UTF-8"),
        ("SHELL", "/bin/bash"),
    ])
    .set("UKIRYU_REGISTER", ctx.input_path("register"));
    match input.get("env") {
        Some(Oj::Obj(pairs)) => {
            for (k, v) in pairs {
                env = match v {
                    Oj::Null => env.delete(k),
                    Oj::Str(s) => env.set(k, s),
                    _ => return Err(RowError::harness(format!("env {k} must be string or null"))),
                };
            }
        }
        None => {}
        _ => {
            return Err(RowError::harness(
                "resolve.shell_detect env must be an object",
            ))
        }
    }
    let shell = detect_shell(platform, &env).map_err(shell_error)?;
    Ok(Oj::obj(vec![
        ("detect", Oj::sym(shell.as_str())),
        ("shell_class", Oj::s(shell_class(shell))),
    ]))
}

fn shell_error(e: ShellError) -> RowError {
    RowError::new(ErrorKind::UnknownShell.ruby_class(), e.to_string())
}

/// The Ruby class `Shell.class_for` returns for a concrete shell.
fn shell_class(shell: ShellName) -> &'static str {
    match shell {
        ShellName::Bash => "Ukiryu::Shell::Bash",
        ShellName::Zsh => "Ukiryu::Shell::Zsh",
        ShellName::Fish => "Ukiryu::Shell::Fish",
        ShellName::Sh => "Ukiryu::Shell::Sh",
        ShellName::Dash => "Ukiryu::Shell::Dash",
        ShellName::Tcsh => "Ukiryu::Shell::Tcsh",
        ShellName::PowerShell => "Ukiryu::Shell::PowerShell",
        ShellName::Cmd => "Ukiryu::Shell::Cmd",
    }
}
