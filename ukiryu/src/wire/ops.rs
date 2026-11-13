//! Typed domain operations over the core APIs: `resolve`, `build`,
//! `validate`, `schema` and `results`. Each request names its method and
//! carries every input explicitly; nothing is defaulted from the host except
//! where the original reads the process (noted per method).
//!
//! Tools come from a [`Source`]: definition YAML supplied by the caller
//! (portable), or, in native builds only, a register directory.
use serde::Deserialize;
use serde_json::{json, Value as Json};

use super::error::{model_error, range_error, ruby, schema_error, tool_error, transport};
use super::value::{self, Typed};
use crate::definition::definition_validator::{self, SchemaStage, ValidationReport};
use crate::definition::ValidatorEnv;
use crate::executable::Platform;
use crate::models::{ImplementationIndex, Model, ToolDefinition};
use crate::runtime::{ShellName, ShellSource};
use crate::schema::{self, Permitted, ValidateOptions};
use crate::tool::{BuiltCommand, Tool, ToolError, ToolOptions};
use crate::types::{self, StdHost, Value};
use crate::version_detection::{extract_version_from_pattern, PatternError};
use crate::version_ranges::{self, Bounds, RangeKind, SpecValue, VersionRange};

/// Where a tool definition comes from.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    /// A `ToolDefinition` YAML document (`ToolDefinition.from_yaml`).
    Definition { yaml: String },
    /// One version file of a register, chosen by the caller:
    /// `Tool::Loader.load_with_implementation_index` with the implementation
    /// and file given instead of detected (the corpus `build.args` chain).
    #[cfg(not(target_arch = "wasm32"))]
    VersionFile {
        register: String,
        tool: String,
        implementation: String,
        file: String,
    },
    /// `Tool.get(name)` over a register, including implementation and
    /// version detection, which runs the detection commands.
    #[cfg(not(target_arch = "wasm32"))]
    Register { register: String, tool: String },
}

/// `Tool.new(profile, platform:, shell:, version:)` options.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRequest {
    pub source: Source,
    pub platform: String,
    pub shell: String,
    #[serde(default)]
    pub version: Option<String>,
}

pub(super) fn tool_options(request: &ToolRequest) -> Result<ToolOptions, Json> {
    let platform = Platform::from_name(&request.platform).ok_or_else(|| {
        transport(
            "INVALID_ARGUMENT",
            format!("Unknown platform: {}", request.platform),
        )
    })?;
    let shell = ShellName::parse(&request.shell, ShellSource::Explicit)
        .map_err(|e| ruby("Ukiryu::Errors::UnknownShellError", e.to_string()))?;
    let mut options = ToolOptions::new(platform, shell);
    if let Some(version) = &request.version {
        options = options.version(version.clone());
    }
    Ok(options)
}

/// The tool a request names, built with the real loader and `Tool.new`.
pub(super) fn load_tool(request: &ToolRequest) -> Result<std::rc::Rc<Tool>, Json> {
    let options = tool_options(request)?;
    let definition = match &request.source {
        Source::Definition { yaml } => {
            ToolDefinition::from_yaml(yaml).map_err(|e| model_error(&e))?
        }
        #[cfg(not(target_arch = "wasm32"))]
        Source::VersionFile {
            register,
            tool,
            implementation,
            file,
        } => {
            super::native::version_file_definition(register, tool, implementation, file, &options)?
        }
        #[cfg(not(target_arch = "wasm32"))]
        Source::Register { register, tool } => {
            return super::native::register_tool(register, tool, &options)
        }
    };
    Tool::new(definition, &options)
        .map(std::rc::Rc::new)
        .map_err(|e| tool_error(&e))
}

// ------------------------------------------------------------------ build

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    pub tool: ToolRequest,
    /// A command name, or a routed action path (`remote:add`).
    pub command: String,
    #[serde(default)]
    pub params: Option<Typed>,
}

pub(super) fn params(params: &Option<Typed>) -> Value {
    params
        .as_ref()
        .map_or(Value::Hash(Vec::new()), |Typed(v)| v.clone())
}

fn env_pairs(built: &BuiltCommand) -> Json {
    Json::Array(
        built
            .invocation
            .env
            .0
            .iter()
            .map(|(k, v)| json!([k, v]))
            .collect(),
    )
}

/// `#execute` up to the executor: argv from `#build_args`, the command's
/// environment variables, the `stdin` parameter and the executable the
/// command runs. A command whose executable was not discovered reports
/// `executable: null`; the arguments do not depend on it.
pub fn build(request: BuildRequest) -> Json {
    let tool = match load_tool(&request.tool) {
        Ok(tool) => tool,
        Err(error) => return error,
    };
    let params = params(&request.params);
    let (built, executable) = match tool.build(&request.command, &params) {
        Ok(built) => {
            let executable = Json::String(built.invocation.executable.clone());
            (built, executable)
        }
        Err(ToolError::ExecutableNotFound { .. }) => {
            match tool.build_args(&request.command, &params) {
                Ok(built) => (built, Json::Null),
                Err(error) => return tool_error(&error),
            }
        }
        Err(error) => return tool_error(&error),
    };
    json!({"ok": true, "value": {
        "executable": executable,
        "args": built.invocation.args,
        "env": env_pairs(&built),
        "stdin": built.stdin.as_deref().map(value::bytes),
        "command_name": built.command_name,
    }})
}

// ---------------------------------------------------------------- resolve

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolveRequest {
    /// `Tool::Loader.extract_version_from_pattern(output, pattern)`.
    ExtractVersion {
        output: Option<String>,
        pattern: Option<String>,
    },
    /// `Versionian::VersionRange.new(range, scheme, **bounds).matches?(version)`
    /// with the scheme from `VersionSchemeResolver.resolve(scheme)`.
    Range {
        scheme: Typed,
        range: String,
        bounds: Typed,
        version: String,
    },
    /// `Tool::Loader.find_matching_version_spec` for one implementation of
    /// an implementation index (`index` is the index YAML): the matching
    /// spec's file, or null.
    VersionSpec {
        index: String,
        implementation: String,
        version: String,
    },
    /// `Tool.get`: the loaded tool's identity, profile and executable.
    Tool(ToolRequest),
}

/// A Ruby value as a version-range input.
fn spec_value(value: &Value) -> Result<SpecValue, Json> {
    Ok(match value {
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
                .map(|(k, v)| Ok((spec_value(k)?, spec_value(v)?)))
                .collect::<Result<_, Json>>()?,
        ),
        other => {
            return Err(transport(
                "INVALID_ARGUMENT",
                format!(
                    "a version spec cannot hold an instance of {}",
                    other.class_name()
                ),
            ))
        }
    })
}

fn pattern_error(error: &PatternError) -> Json {
    match error {
        PatternError::RubyInvalid { reason, .. } => ruby("RegexpError", reason.clone()),
        PatternError::Unsupported { .. } => transport("NOT_PORTED", error.to_string()),
    }
}

fn range(scheme: &Value, range: &str, bounds: &Value, version: &str) -> Result<Json, Json> {
    let scheme = version_ranges::resolve(&spec_value(scheme)?).map_err(|e| range_error(&e))?;
    let kind = RangeKind::from_name(range).map_err(|e| range_error(&e))?;
    let Value::Hash(pairs) = bounds else {
        return Err(transport("INVALID_ARGUMENT", "bounds must be a Hash"));
    };
    let mut keywords = Bounds::new();
    for (key, value) in pairs {
        let (Value::String(key) | Value::Symbol(key)) = key else {
            return Err(transport("INVALID_ARGUMENT", "bounds keys are names"));
        };
        keywords = keywords.with(key, spec_value(value)?);
    }
    let matched = VersionRange::new(kind, &scheme, keywords)
        .and_then(|r| r.matches(version))
        .map_err(|e| range_error(&e))?;
    Ok(Json::Bool(matched))
}

fn version_spec(index: &str, implementation: &str, version: &str) -> Result<Json, Json> {
    let index = ImplementationIndex::from_register_yaml(index)
        .map_err(|e| model_error(&e))?
        .ok_or_else(|| transport("INVALID_ARGUMENT", "the index document is empty"))?;
    let wanted = Value::str(implementation);
    let entry = index
        .implementations
        .iter()
        .find(|m| m.get_sym_or_str("name") == Some(&wanted))
        .ok_or_else(|| {
            transport(
                "INVALID_ARGUMENT",
                format!("no implementation {implementation}"),
            )
        })?;
    let scheme_spec = entry
        .get_sym_or_str("version_scheme")
        .cloned()
        .unwrap_or(Value::Nil);
    let scheme =
        version_ranges::resolve(&spec_value(&scheme_spec)?).map_err(|e| range_error(&e))?;
    let versions = entry
        .get_sym_or_str("versions")
        .cloned()
        .unwrap_or(Value::Nil);
    let SpecValue::Array(specs) = spec_value(&versions)? else {
        return Err(transport(
            "NOT_PORTED",
            "versions that are not an Array are not ported",
        ));
    };
    let found = version_ranges::find_matching_version_spec(&specs, version, &scheme)
        .map_err(|e| range_error(&e))?;
    let Some(i) = found else {
        return Ok(Json::Null);
    };
    let spec = match &versions {
        Value::Array(items) => &items[i],
        _ => unreachable!("checked above"),
    };
    Ok(spec
        .get_sym_or_str("file")
        .map_or(Json::Null, value::encode))
}

/// What `Tool.get` resolved.
pub(super) fn tool_identity(tool: &Tool) -> Json {
    json!({
        "name": tool.name(),
        "version": tool.version(),
        "platform": tool.platform().as_str(),
        "shell": tool.shell().as_str(),
        "profile_index": tool.profile_index(),
        "profile_name": tool.command_profile().name.as_ref().and_then(crate::tool::text),
        "executable": tool.executable(),
        "available": tool.is_available(),
        "unavailability_reason": tool.unavailability_reason(),
        "discovery": tool.discovery_description(),
        "routing": tool.has_routing(),
        "commands": tool.commands().iter().filter_map(|c| c.name.as_ref().and_then(crate::tool::text)).collect::<Vec<_>>(),
    })
}

pub fn resolve(request: ResolveRequest) -> Json {
    let result = match request {
        ResolveRequest::ExtractVersion { output, pattern } => {
            extract_version_from_pattern(output.as_deref(), pattern.as_deref())
                .map(|v| json!(v))
                .map_err(|e| pattern_error(&e))
        }
        ResolveRequest::Range {
            scheme: Typed(scheme),
            range: name,
            bounds: Typed(bounds),
            version,
        } => range(&scheme, &name, &bounds, &version),
        ResolveRequest::VersionSpec {
            index,
            implementation,
            version,
        } => version_spec(&index, &implementation, &version),
        ResolveRequest::Tool(request) => load_tool(&request).map(|tool| tool_identity(&tool)),
    };
    match result {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => error,
    }
}

// --------------------------------------------------------------- validate

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValidateRequest {
    /// `Ukiryu::Type.validate(value, type, options)`.
    Type {
        value: Typed,
        r#type: Typed,
        #[serde(default)]
        options: Option<Typed>,
    },
    /// `DefinitionValidator.validate_string(yaml, schema_path:)` through the
    /// core validator. With no `schema_path`, `UKIRYU_SCHEMA_PATH` is read
    /// from the process environment as the original does; the remote schema
    /// download is never performed (reported as the schema stage).
    Definition {
        yaml: String,
        #[serde(default)]
        schema_path: Option<String>,
    },
}

fn stage(stage: &Option<SchemaStage>) -> Json {
    match stage {
        None => Json::Null,
        Some(SchemaStage::Given { path }) => json!({"kind": "given", "path": path}),
        Some(SchemaStage::GivenNotLoaded { path }) => {
            json!({"kind": "given_not_loaded", "path": path})
        }
        Some(SchemaStage::Env { path }) => json!({"kind": "env", "path": path}),
        Some(SchemaStage::RemoteFallbackNotPerformed) => {
            json!({"kind": "remote_fallback_not_performed"})
        }
    }
}

/// `ValidationResult#to_h` plus how the schema stage was resolved.
fn report(report: &ValidationReport) -> Json {
    let result = &report.result;
    json!({
        "valid": result.is_valid(),
        "errors": result.errors,
        "warnings": result.warnings,
        "schema_path": result.schema_path,
        "error_count": result.error_count(),
        "warning_count": result.warning_count(),
        "schema_stage": stage(&report.schema_stage),
    })
}

pub fn validate(request: ValidateRequest) -> Json {
    match request {
        ValidateRequest::Type {
            value: Typed(value),
            r#type: Typed(spec),
            options,
        } => {
            let options = options.map_or(Value::Hash(Vec::new()), |Typed(o)| o);
            match types::validate(&value, &spec, &options, &StdHost) {
                Ok(value) => json!({"ok": true, "value": value::encode(&value)}),
                Err(error) => types_error(&error),
            }
        }
        ValidateRequest::Definition { yaml, schema_path } => {
            match definition_validator::validate_string_report(
                &yaml,
                schema_path.as_deref(),
                &ValidatorEnv::from_process(),
            ) {
                Ok(r) => json!({"ok": true, "value": report(&r)}),
                Err(error) => schema_error(&error),
            }
        }
    }
}

pub(super) fn types_error(error: &types::Error) -> Json {
    if error.class == types::ErrorClass::Unsupported {
        return transport("NOT_PORTED", error.message.clone());
    }
    ruby(error.class.ruby_name(), error.message.clone())
}

// ----------------------------------------------------------------- schema

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum SchemaRequest {
    /// `SchemaValidator.validate_profile(YAML.safe_load(profile), schema_path:,
    /// strict:)`. Without `schema_path`, `UKIRYU_SCHEMA_PATH` is read from the
    /// process environment as the original does.
    ValidateProfile {
        profile: String,
        #[serde(default)]
        schema_path: Option<String>,
        #[serde(default)]
        strict: bool,
    },
    /// `JSON::Validator.fully_validate(schema, profile, strict:)` with both
    /// documents supplied as YAML (`YAML.safe_load`).
    FullyValidate {
        profile: String,
        schema: String,
        #[serde(default)]
        strict: bool,
    },
}

pub fn schema(request: SchemaRequest) -> Json {
    let result = match request {
        SchemaRequest::ValidateProfile {
            profile,
            schema_path,
            strict,
        } => schema::safe_load(&profile, Permitted::DOCUMENT).and_then(|profile| {
            schema::validate_profile(
                &profile,
                &ValidateOptions {
                    schema_path: schema_path.map(Into::into),
                    strict,
                },
            )
        }),
        SchemaRequest::FullyValidate {
            profile,
            schema,
            strict,
        } => schema::safe_load(&profile, Permitted::DOCUMENT).and_then(|profile| {
            let schema = schema::safe_load(&schema, Permitted::NONE)?;
            schema::fully_validate(&schema, &profile, strict)
        }),
    };
    match result {
        Ok(messages) => json!({"ok": true, "value": messages}),
        Err(error) => schema_error(&error),
    }
}

// ---------------------------------------------------------------- results

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultsRequest {
    /// The meaning of an exit status for a command of a tool: its own exit
    /// code table, else the profile's.
    ExitCodeMeaning {
        tool: ToolRequest,
        command: String,
        /// An Integer (`Process::Status#exitstatus`).
        status: Typed,
    },
}

pub fn results(request: ResultsRequest) -> Json {
    match request {
        ResultsRequest::ExitCodeMeaning {
            tool,
            command,
            status,
        } => match load_tool(&tool) {
            Ok(tool) => {
                let Value::Integer(status) = status.0 else {
                    return transport("INVALID_ARGUMENT", "status must be an Integer");
                };
                let Ok(status) = i64::try_from(&status) else {
                    return transport("INVALID_ARGUMENT", "status is out of range");
                };
                json!({"ok": true, "value": tool.exit_code_meaning(&command, status)})
            }
            Err(error) => error,
        },
    }
}
