//! `build` kind (`build.args`) and `resolve.version_spec`: register documents
//! loaded through the core's model entry points (`ImplementationIndex`,
//! `Interface`, `ImplementationVersion` `::from_register_yaml`), argv built by
//! `ukiryu::command_builder::build_args`, version specs selected by
//! `ukiryu::version_ranges::find_matching_version_spec`.
//!
//! The register is `inputs/register` (the oracle's `UKIRYU_REGISTER`). The
//! only glue here is the file lookup the Ruby `Register` does before parsing
//! (`tools/<tool>/index.yaml`, `interfaces/<name>.yaml`,
//! `tools/<tool>/<impl>/<file>`; a missing file is `nil`).
use std::path::Path;

use ukiryu::command_builder::{build_args as core_build_args, BuildContext, CommandSpec};
use ukiryu::models::{
    ImplementationIndex, ImplementationVersion, Interface, Model, ModelError, ToolDefinition,
};
use ukiryu::types::{StdHost, Value};
use ukiryu::version_ranges::{
    find_matching_version_spec, resolve, ErrorClass, RangeError, SpecValue,
};

use super::codec;
use super::harness::{input_str, Ctx, Outcome, RowError};
use super::json::Oj;
use super::ops_shell::shell;

fn model_error(e: ModelError) -> RowError {
    RowError::new(e.ruby_class(), e.message)
}

fn range_error(e: RangeError) -> RowError {
    match e.class {
        ErrorClass::Unsupported => RowError::harness(format!("unsupported: {}", e.message)),
        class => RowError::new(class.ruby_name(), e.message),
    }
}

/// `File.read(file)` when `File.exist?(file)`, else `None` (the register
/// returns nil for a missing document).
fn register_file(ctx: &Ctx, rel: &str) -> Result<Option<String>, RowError> {
    let path = ctx.input_path(&format!("register/{rel}"));
    if !Path::new(&path).exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| RowError::harness(format!("read {path}: {e}")))
}

/// `Register.load_implementation_index(tool)`.
fn load_index(ctx: &Ctx, tool: &str) -> Result<Option<ImplementationIndex>, RowError> {
    match register_file(ctx, &format!("tools/{tool}/index.yaml"))? {
        Some(text) => ImplementationIndex::from_register_yaml(&text).map_err(model_error),
        None => Ok(None),
    }
}

/// `Register.load_interface(name)`.
fn load_interface(ctx: &Ctx, name: &str) -> Result<Option<Interface>, RowError> {
    match register_file(ctx, &format!("interfaces/{name}.yaml"))? {
        Some(text) => Interface::from_register_yaml(&text).map_err(model_error),
        None => Ok(None),
    }
}

/// `Register.load_implementation_version(tool, impl, file)`.
fn load_version(
    ctx: &Ctx,
    tool: &str,
    implementation: &str,
    file: &str,
) -> Result<Option<ImplementationVersion>, RowError> {
    match register_file(ctx, &format!("tools/{tool}/{implementation}/{file}"))? {
        Some(text) => ImplementationVersion::from_register_yaml(&text).map_err(model_error),
        None => Ok(None),
    }
}

/// `Tool::Loader.convert_to_tool_definition` after its profile selection.
/// The core ports the typed constructors it calls
/// (`ukiryu::model_loading::{command_definition_new, platform_profile_new,
/// tool_definition_new}`) but not the Hash orchestration that produces their
/// keywords (`extract_profile_data`, `convert_actions_to_array`,
/// `flatten_signature`, `build_interface_commands_hash`,
/// `build_command_definitions`/`deep_merge_hashes`); `model_loading.rs` says
/// it stays with the register/loader owner.
fn convert_to_tool_definition(
    _tool: &str,
    _interface: &Interface,
    _version: &ImplementationVersion,
    _implementation: &str,
    _profile: &Value,
) -> Result<ToolDefinition, RowError> {
    Err(RowError::harness(
        "no core API: Tool::Loader.convert_to_tool_definition (profile/interface Hash \
         merge into CommandDefinition keywords is not ported)",
    ))
}

/// `build.args`: the oracle's `load_tool` then `Tool#command_definition` and
/// `Tool#build_args(command, params.transform_keys(&:to_sym))`.
pub fn build_args(input: &Oj, ctx: &Ctx) -> Outcome {
    let tool_name = input_str(input, "tool")?;
    let implementation = input_str(input, "implementation")?;
    let file = input_str(input, "file")?;
    let platform = input_str(input, "platform")?;
    let shell_name = input_str(input, "shell")?;
    let command_name = input_str(input, "command")?;
    let not_found = || {
        RowError::new(
            "Ukiryu::Errors::ToolNotFoundError",
            format!("Tool not found: {tool_name}"),
        )
    };

    let index = load_index(ctx, tool_name)?.ok_or_else(not_found)?;
    let interface = load_interface(ctx, &index.interface.to_ruby_s())?.ok_or_else(not_found)?;
    let version = load_version(ctx, tool_name, implementation, file)?.ok_or_else(not_found)?;
    // `convert_to_tool_definition` returns nil when no profile is compatible.
    let profile = version
        .compatible_profile(platform, shell_name)
        .map_err(model_error)?
        .ok_or_else(not_found)?;
    let mut tool =
        convert_to_tool_definition(tool_name, &interface, &version, implementation, &profile)?;
    tool.resolve_inheritance();

    // `Tool#initialize` -> `find_command_profile`: the converted definition
    // has exactly one profile, which Ruby always takes.
    let command_profile = match tool.profiles.as_deref() {
        Some([only]) => only,
        _ => {
            return Err(RowError::harness(
                "no core API: Tool#find_command_profile for a multi-profile definition",
            ))
        }
    };
    let command = command_profile.command(command_name).ok_or_else(|| {
        RowError::new("ArgumentError", format!("Unknown command: {command_name}"))
    })?;
    // The builder's input boundary deserializes from the model's keys.
    let json = command.to_json().map_err(model_error)?;
    let spec: CommandSpec = serde_json::from_str(&json)
        .map_err(|e| RowError::harness(format!("command spec from model json: {e}")))?;

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
    let ctx_build = BuildContext {
        shell: shell(shell_name)?,
        platform,
        host: &StdHost,
    };
    core_build_args(&spec, &params, &ctx_build)
        .map(|args| Oj::Arr(args.into_iter().map(Oj::Str).collect()))
        .map_err(|e| RowError::new(e.class.ruby_name(), e.message))
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

    let index = load_index(ctx, tool_name)?
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
