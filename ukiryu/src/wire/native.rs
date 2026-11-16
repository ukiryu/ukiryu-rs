//! Native-only operations: register directories on the host filesystem and
//! process execution. This module is not compiled for WebAssembly, so a
//! browser build cannot reach the executor through the wire.
use std::collections::BTreeMap;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::{json, Value as Json};

use super::error::{execute_error, ruby, tool_error, transport};
use super::ops::{params, ToolRequest};
use super::value::Typed;
use crate::cache::ToolCache;
use crate::environment::Environment;
use crate::execution::Seconds;
use crate::extractors::base::ProcessHost;
use crate::models::ToolDefinition;
use crate::register::Register;
use crate::runtime::native_platform;
use crate::tool::{self, profile_is_modern, ExecuteSettings, Tool, ToolError, ToolOptions};
use crate::tool_loader::{self, RegisterSource};
use crate::types::Value;

fn not_found(tool: &str) -> Json {
    tool_error(&ToolError::NotFound {
        name: tool.to_owned(),
    })
}

/// The corpus `build.args` loader chain: index, interface and the given
/// implementation version file, converted for the requested platform and
/// shell. Each missing step ends in `ToolNotFoundError`, as `Tool.get` does.
pub(super) fn version_file_definition(
    register: &str,
    tool: &str,
    implementation: &str,
    file: &str,
    options: &ToolOptions,
) -> Result<ToolDefinition, Json> {
    let register = Register::at(register);
    let fail = |e: tool_loader::LoadError| tool_error(&ToolError::from(e));
    let index = tool_loader::load_implementation_index(&register, tool)
        .map_err(fail)?
        .ok_or_else(|| not_found(tool))?;
    let interface = tool_loader::load_interface(&register, &index.interface.to_ruby_s())
        .map_err(fail)?
        .ok_or_else(|| not_found(tool))?;
    let version = tool_loader::load_implementation_version(&register, tool, implementation, file)
        .map_err(fail)?
        .ok_or_else(|| not_found(tool))?;
    tool_loader::convert_to_tool_definition(
        tool,
        &interface,
        &version,
        Some(implementation),
        None,
        options.platform.as_str(),
        options.shell.as_str(),
    )
    .map_err(fail)?
    .ok_or_else(|| not_found(tool))
}

/// `Tool.get(name, platform:, shell:)` over a register directory. Detection
/// commands run through the real executor with the requested shell.
pub(super) fn register_tool(
    register: &str,
    name: &str,
    options: &ToolOptions,
) -> Result<Rc<Tool>, Json> {
    let host =
        native_platform().map_err(|e| ruby("Ukiryu::Errors::UnsupportedPlatformError", e))?;
    let runner = ProcessHost::new(Environment::new(), options.shell);
    let source = RegisterSource::new(Register::at(register), host, runner);
    let cache = ToolCache::new();
    tool::get(&source, &cache, name, options).map_err(|e| tool_error(&e))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub tool: ToolRequest,
    pub command: String,
    #[serde(default)]
    pub params: Option<Typed>,
    /// Bytes written to the child's stdin when they are not a UTF-8 String
    /// parameter: a string or `{"$bytes": hex}`.
    #[serde(default)]
    pub stdin: Option<StdinBytes>,
    /// The environment the command's variables are applied over (required).
    pub env: BTreeMap<String, String>,
    /// Seconds: an Integer or a `$float` (required).
    pub timeout: Typed,
}

/// Lossless stdin: a JSON string or `{"$bytes": hex}`.
#[derive(Debug)]
pub struct StdinBytes(pub Vec<u8>);

impl<'de> Deserialize<'de> for StdinBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Bytes {
                #[serde(rename = "$bytes")]
                bytes: String,
            },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Text(text) => Ok(StdinBytes(text.into_bytes())),
            Raw::Bytes { bytes } => super::value::decode_hex(&bytes)
                .map(StdinBytes)
                .map_err(serde::de::Error::custom),
        }
    }
}

fn timeout(value: &Value) -> Result<Seconds, Json> {
    match value {
        Value::Integer(i) => i64::try_from(i)
            .map(Seconds::Int)
            .map_err(|_| transport("INVALID_ARGUMENT", "timeout is out of range")),
        Value::Float(f) => Ok(Seconds::Float(*f)),
        other => Err(transport(
            "INVALID_ARGUMENT",
            format!(
                "timeout must be an Integer or a Float, not {}",
                other.class_name()
            ),
        )),
    }
}

/// `Tool#execute(command, execution_timeout:, **params)`.
///
/// A command whose executable was not discovered behaves as the original:
/// with a subcommand or a modern profile the original runs a `nil`
/// executable (the shell reports it), otherwise `File.dirname(nil)` raises
/// `TypeError`.
pub fn execute(request: ExecuteRequest) -> Json {
    match run(request) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => error,
    }
}

fn run(request: ExecuteRequest) -> Result<Json, Json> {
    let timeout = timeout(&request.timeout.0)?;
    let tool = super::ops::load_tool(&request.tool)?;
    let params = params(&request.params);
    let mut built = match tool.build(&request.command, &params) {
        Ok(built) => built,
        Err(ToolError::ExecutableNotFound { .. }) => {
            let command = tool
                .command_definition(&request.command)
                .ok_or_else(|| transport("INTERNAL_ERROR", "built command has no definition"))?;
            let definition = tool.definition();
            let modern = profile_is_modern(
                definition.version.as_ref(),
                definition.version_detection.as_deref(),
            )
            .map_err(|e| tool_error(&e))?;
            if !(command.has_subcommand() || modern == Some(true)) {
                return Err(ruby(
                    "TypeError",
                    "no implicit conversion of nil into String",
                ));
            }
            tool.build_args(&request.command, &params)
                .map_err(|e| tool_error(&e))?
        }
        Err(error) => return Err(tool_error(&error)),
    };
    if let Some(StdinBytes(bytes)) = request.stdin {
        if built.stdin.is_some() {
            return Err(transport(
                "INVALID_ARGUMENT",
                "stdin is given both as a parameter and as bytes",
            ));
        }
        built.stdin = Some(bytes);
    }
    let settings = ExecuteSettings {
        timeout,
        environment: Environment::from_pairs(request.env),
    };
    let execute = tool.request(built, &settings).map_err(|e| tool_error(&e))?;
    match crate::executor::execute(execute) {
        Ok(run) => Ok(super::results::execution(&run)),
        Err(error) => Err(execute_error(&error)),
    }
}
