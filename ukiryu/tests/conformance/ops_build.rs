//! `build` kind (`build.args`) and `resolve.version_spec` through the core
//! register loader (`ukiryu::tool_loader`), tool runtime (`ukiryu::tool`)
//! and version-range selection (`ukiryu::version_ranges`).
//!
//! The register is `inputs/register` (the oracle's `UKIRYU_REGISTER`).
use ukiryu::executable::{DiscoveryContext, NoAliases, Platform, SystemCommandRunner};
use ukiryu::register::{Register, Source};
use ukiryu::runtime::ShellName;
use ukiryu::tool::{Tool, ToolError, ToolOptions};
use ukiryu::tool_loader::{self, LoadError};
use ukiryu::types::Value;
use ukiryu::version_ranges::{
    find_matching_version_spec, resolve, ErrorClass, RangeError, SpecValue,
};

use super::codec;
use super::harness::{input_str, Ctx, Outcome, RowError};
use super::json::Oj;

fn range_error(e: RangeError) -> RowError {
    match e.class {
        ErrorClass::Unsupported => RowError::harness(format!("unsupported: {}", e.message)),
        class => RowError::new(class.ruby_name(), e.message),
    }
}

/// The oracle's `BuilderTool` overrides `find_executable` so pure argv rows
/// never run host `where`/`which`. The core equivalent is a declared
/// system-lookup answer: discovery reports `/declared/absent/executable` without
/// touching the host (empty `PATH`, no aliases, no processes). Only argv is
/// compared, never this path.
struct DeclaredExecutable;

impl SystemCommandRunner for DeclaredExecutable {
    fn run(&mut self, _executable: &str, _args: &[String]) -> Option<String> {
        Some("/declared/absent/executable\n".to_owned())
    }
}

fn load_error(e: LoadError) -> RowError {
    RowError::new(e.class_name(), e.message())
}

fn tool_error(e: ToolError) -> RowError {
    match e {
        ToolError::Build(b) => RowError::new(b.class.ruby_name(), b.message),
        ToolError::Argument { message } => RowError::new("ArgumentError", message),
        ToolError::Ruby { class, message } => RowError::new(class, message),
        other => match other.kind() {
            Some(kind) => RowError::new(kind.ruby_class(), other.to_string()),
            None => RowError::harness(format!("tool error without a Ruby class: {other}")),
        },
    }
}

/// `build.args`: the oracle's `load_tool` (register index, interface,
/// implementation version, `Tool::Loader.convert_to_tool_definition`,
/// `Tool.new`) then `Tool#command_definition` and `Tool#build_args`, all
/// through the core loader (`ukiryu::tool_loader`) and tool runtime
/// (`ukiryu::tool`). A nil at any loading step is `ToolNotFoundError`.
pub fn build_args(input: &Oj, ctx: &Ctx) -> Outcome {
    let tool_name = input_str(input, "tool")?;
    let implementation = input_str(input, "implementation")?;
    let file = input_str(input, "file")?;
    let platform_name = input_str(input, "platform")?;
    let shell_name = input_str(input, "shell")?;
    let command_name = input_str(input, "command")?;
    let not_found = || {
        RowError::new(
            "Ukiryu::Errors::ToolNotFoundError",
            format!("Tool not found: {tool_name}"),
        )
    };
    let platform = Platform::from_name(platform_name)
        .ok_or_else(|| RowError::harness(format!("unknown platform {platform_name}")))?;
    let shell = ShellName::lookup(shell_name)
        .ok_or_else(|| RowError::harness(format!("unknown shell {shell_name}")))?;

    let register = Register::new(ctx.input_path("register"), Source::Manual);
    let index = tool_loader::load_implementation_index(&register, tool_name)
        .map_err(load_error)?
        .ok_or_else(not_found)?;
    let interface = tool_loader::load_interface(&register, &index.interface.to_ruby_s())
        .map_err(load_error)?
        .ok_or_else(not_found)?;
    let version =
        tool_loader::load_implementation_version(&register, tool_name, implementation, file)
            .map_err(load_error)?
            .ok_or_else(not_found)?;
    let definition = tool_loader::convert_to_tool_definition(
        tool_name,
        &interface,
        &version,
        Some(implementation),
        None,
        platform_name,
        shell_name,
    )
    .map_err(load_error)?
    .ok_or_else(not_found)?;

    let options = ToolOptions::new(platform, shell).discovery(DiscoveryContext::new(
        platform,
        shell_name,
        Some(""),
        None,
    ));
    let tool = Tool::with_discovery(
        definition,
        &options,
        &mut NoAliases,
        &mut DeclaredExecutable,
    )
    .map_err(tool_error)?;
    if tool.command_definition(command_name).is_none() {
        return Err(RowError::new(
            "ArgumentError",
            format!("Unknown command: {command_name}"),
        ));
    }

    let params = match input.get("params") {
        None | Some(Oj::Null) => Value::Hash(Vec::new()),
        Some(p) => match codec::decode(p).map_err(RowError::harness)? {
            Value::Hash(pairs) => Value::Hash(
                pairs
                    .into_iter()
                    .map(|(k, v)| match k {
                        Value::String(s) | Value::Symbol(s) => Ok((Value::Symbol(s), v)),
                        other => Err(RowError::harness(format!("param key {other:?}"))),
                    })
                    .collect::<Result<_, _>>()?,
            ),
            other => return Err(RowError::harness(format!("params not a Hash: {other:?}"))),
        },
    };
    let built = tool.build(command_name, &params).map_err(tool_error)?;
    Ok(Oj::Arr(
        built.invocation.args.into_iter().map(Oj::Str).collect(),
    ))
}

/// A loaded register value as the range module's Ruby value shape.
fn spec_value(v: &Value) -> Result<SpecValue, RowError> {
    Ok(match v {
        Value::Nil => SpecValue::Nil,
        Value::Bool(b) => SpecValue::Bool(*b),
        Value::Integer(i) => SpecValue::Int(i.clone()),
        Value::Float(f) => SpecValue::Float(*f),
        Value::String(s) => SpecValue::Str(s.clone()),
        Value::Symbol(s) => SpecValue::Sym(s.clone()),
        Value::Array(items) => {
            SpecValue::Array(items.iter().map(spec_value).collect::<Result<_, _>>()?)
        }
        Value::Hash(pairs) => SpecValue::Map(
            pairs
                .iter()
                .map(|(k, x)| Ok((spec_value(k)?, spec_value(x)?)))
                .collect::<Result<_, RowError>>()?,
        ),
        other => {
            return Err(RowError::harness(format!(
                "no SpecValue form for {other:?}"
            )))
        }
    })
}

/// `resolve.version_spec`: the oracle loads the implementation index, picks
/// the named implementation, resolves its `version_scheme` and returns the
/// `file` of the first matching version spec.
pub fn version_spec(input: &Oj, ctx: &Ctx) -> Outcome {
    let tool_name = input_str(input, "tool")?;
    let implementation = input_str(input, "implementation")?;
    let version = input_str(input, "version")?;

    let register = Register::new(ctx.input_path("register"), Source::Manual);
    let index = tool_loader::load_implementation_index(&register, tool_name)
        .map_err(load_error)?
        .ok_or_else(|| RowError::harness(format!("no index for {tool_name}")))?;
    let imp = index
        .implementations
        .iter()
        .find(|m| m.get_sym_or_str("name").map(Value::to_ruby_s).as_deref() == Some(implementation))
        .ok_or_else(|| {
            RowError::new(
                "RuntimeError",
                format!("no implementation {implementation}"),
            )
        })?;

    let scheme_spec = spec_value(imp.get_sym_or_str("version_scheme").unwrap_or(&Value::Nil))?;
    let scheme = resolve(&scheme_spec).map_err(range_error)?;
    let versions = match imp.get_sym_or_str("versions") {
        Some(Value::Array(items)) => items,
        other => {
            return Err(RowError::harness(format!(
                "implementation versions not an Array: {other:?}"
            )))
        }
    };
    let specs = versions
        .iter()
        .map(spec_value)
        .collect::<Result<Vec<_>, _>>()?;
    let found = find_matching_version_spec(&specs, version, &scheme).map_err(range_error)?;
    Ok(match found {
        Some(i) => codec::encode(versions[i].get_sym_or_str("file").unwrap_or(&Value::Nil)),
        None => Oj::Null,
    })
}
