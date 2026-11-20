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
use crate::executable::DiscoveryContext as PathContext;
use crate::execution::Seconds;
use crate::extractors::base::ProcessHost;
use crate::models::{Model, ToolDefinition};
use crate::register::{DiscoveryContext, Git, Register};
use crate::register_lookup::{find_by_input, load_tool_index, Identifier, LookupError, Target};
use crate::runtime::{native_platform, ShellName};
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

/// Definition used by the classic generated OOP classes. Keep the raw YAML
/// loader distinct from Tool.get: it lists only equals-version entries.
pub(super) fn tool_class(
    register: &str,
    name: &str,
    platform: &str,
    shell: &str,
) -> Result<Json, Json> {
    let yaml = Register::at(register)
        .load_tool_yaml(name, None)
        .map_err(|e| ruby(e.class_name(), e.to_string()))?
        .ok_or_else(|| not_found(name))?;
    let mut definition =
        ToolDefinition::from_yaml(&yaml).map_err(|e| super::error::model_error(&e))?;
    definition
        .resolve_inheritance()
        .map_err(|e| super::error::model_error(&e))?;
    let selected = definition
        .compatible_profile(platform, shell)
        .map_err(|e| super::error::model_error(&e))?;
    let index = selected.and_then(|selected| {
        definition.profiles.as_ref().and_then(|profiles| {
            profiles
                .iter()
                .position(|p| p.as_ref().is_some_and(|p| std::ptr::eq(p, selected)))
        })
    });
    let fields = definition
        .to_hash()
        .map_err(|e| super::error::model_error(&e))?;
    let fields =
        super::value::encode(&fields).map_err(|e| ruby(super::value::EncodeError::CLASS, e))?;
    let yaml = definition
        .to_yaml()
        .map_err(|e| super::error::model_error(&e))?;
    Ok(json!({"definition": fields, "profile_index": index, "yaml": yaml}))
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

// ---------------------------------------------------------------- executor

/// `Executor.execute(executable, args, env:, shell:, timeout:, ...)`: one
/// command run through a shell, with no tool definition.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRequest {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The environment the run's variables are applied over (required, D1).
    pub env: BTreeMap<String, String>,
    /// A concrete shell name. Absent, the source's `ArgumentError`.
    #[serde(default)]
    pub shell: Option<String>,
    /// Seconds: an Integer or a `$float`. Absent, the source's
    /// `ArgumentError`.
    #[serde(default)]
    pub timeout: Option<Typed>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// A string or `{"$bytes": hex}`.
    #[serde(default)]
    pub stdin: Option<StdinBytes>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub command_name: Option<String>,
    #[serde(default)]
    pub allow_failure: bool,
}

/// The `executor` op. Checks run in the source's order: shell given,
/// timeout given, shell known, then the run.
pub fn executor(request: ExecutorRequest) -> Json {
    match run_executor(request) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => error,
    }
}

fn run_executor(request: ExecutorRequest) -> Result<Json, Json> {
    use crate::execution_context::MissingSetting;
    use crate::runtime::ShellSource;
    let shell = request
        .shell
        .ok_or_else(|| ruby("ArgumentError", MissingSetting::Shell))?;
    let timeout = request
        .timeout
        .ok_or_else(|| ruby("ArgumentError", MissingSetting::Timeout))?;
    let timeout = self::timeout(&timeout.0)?;
    let shell = ShellName::parse(&shell, ShellSource::Explicit)
        .map_err(|e| ruby("Ukiryu::Errors::UnknownShellError", e))?;
    let mut execute = crate::executor::ExecuteRequest::new(
        request.executable,
        request.args,
        Environment::from_pairs(request.env),
        shell,
        timeout,
    )
    .allow_failure(request.allow_failure);
    if let Some(cwd) = request.cwd {
        execute = execute.cwd(cwd);
    }
    if let Some(StdinBytes(bytes)) = request.stdin {
        execute = execute.stdin_bytes(bytes);
    }
    if let Some(name) = request.tool_name {
        execute = execute.tool_name(name);
    }
    if let Some(name) = request.command_name {
        execute = execute.command_name(name);
    }
    match crate::executor::execute(execute) {
        Ok(run) => Ok(super::results::execution(&run)),
        Err(error) => Err(execute_error(&error)),
    }
}

// ---------------------------------------------------------------- find_by

/// Where the register comes from.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FindRegister {
    /// `Register.new(path)`.
    Path { path: String },
    /// `Register.discover` over explicit inputs, auto-clone included:
    /// `UKIRYU_REGISTER`, the development path, the home directory and the
    /// git program. Nothing is read from the process environment.
    Discover {
        #[serde(default)]
        env_register: Option<String>,
        #[serde(default)]
        dev_path: Option<String>,
        #[serde(default)]
        home: Option<String>,
        /// The git program; `git` through `PATH` when absent.
        #[serde(default)]
        git: Option<String>,
    },
}

/// The `identifier` argument before `identifier.to_s`: `nil`, a String or an
/// Integer. Symbols are Strings on the wire (`Symbol#to_s`).
#[derive(Debug)]
pub enum FindIdentifier {
    Nil,
    Str(String),
    Integer(i64),
}

impl<'de> Deserialize<'de> for FindIdentifier {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Nil(()),
            Int(i64),
            Str(String),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Nil(()) => FindIdentifier::Nil,
            Raw::Int(i) => FindIdentifier::Integer(i),
            Raw::Str(s) => FindIdentifier::Str(s),
        })
    }
}

/// `Tool.find_by(identifier, platform:, shell:)` / `ToolFinder.find_by`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindByRequest {
    pub register: FindRegister,
    #[serde(default = "nil_identifier")]
    pub identifier: FindIdentifier,
    /// The `options[:platform]` / `options[:shell]` the caller passed, as
    /// names (they may be outside the typed set).
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    /// The `PATH` searched for executables; the process `PATH` when absent.
    #[serde(default)]
    pub path: Option<String>,
    /// Call twice against one cache and report whether both calls returned
    /// the same tool object (`same_object` in the reply).
    #[serde(default)]
    pub repeat: bool,
}

fn nil_identifier() -> FindIdentifier {
    FindIdentifier::Nil
}

/// Reply: `value` is `null` when no tool matches, else the tool's identity
/// (as `resolve`). With `repeat`, the reply also carries `same_object`.
pub fn find_by(request: FindByRequest) -> Json {
    match run_find_by(request) {
        Ok((value, same)) => {
            let mut reply = json!({"ok": true, "value": value});
            if let Some(same) = same {
                reply["same_object"] = json!(same);
            }
            reply
        }
        Err(error) => error,
    }
}

fn run_find_by(request: FindByRequest) -> Result<(Json, Option<bool>), Json> {
    let platform =
        native_platform().map_err(|e| ruby("Ukiryu::Errors::UnsupportedPlatformError", e))?;
    let shell = request
        .shell
        .as_deref()
        .and_then(ShellName::lookup)
        .unwrap_or(ShellName::Bash);
    let path = match &request.path {
        Some(path) => Some(path.clone()),
        None => std::env::var("PATH").ok(),
    };
    let register = match &request.register {
        FindRegister::Path { path } => Register::at(path),
        FindRegister::Discover {
            env_register,
            dev_path,
            home,
            git,
        } => {
            let context = DiscoveryContext {
                env_register: env_register.as_ref().map(Into::into),
                dev_path: dev_path.as_ref().map(Into::into),
                home: home.as_ref().map(Into::into),
            };
            let mut git_runner = Git::default();
            if let Some(program) = git {
                git_runner.program = program.into();
            }
            context
                .discover(&git_runner)
                .map_err(|e| ruby(e.class_name(), e.to_string()))?
        }
    };
    let options = ToolOptions::new(platform, shell).discovery(PathContext::new(
        platform,
        request.shell.as_deref().unwrap_or("bash"),
        path.as_deref(),
        None,
    ));
    let runtime = ToolOptions::new(platform, shell);
    let index = load_tool_index(&register);
    let mut env = Environment::new();
    if let Some(path) = &path {
        env = env.set("PATH", path);
    }
    let source = RegisterSource::new(register.clone(), platform, ProcessHost::new(env, shell));
    let cache = ToolCache::new();
    let target = Target {
        platform: request.platform.as_deref(),
        shell: request.shell.as_deref(),
    };
    let call = || {
        let identifier = match &request.identifier {
            FindIdentifier::Nil => Identifier::Nil,
            FindIdentifier::Str(s) => Identifier::Str(s),
            FindIdentifier::Integer(i) => Identifier::Integer(*i),
        };
        find_by_input(
            &register, &index, &source, &cache, identifier, target, &options, &runtime,
        )
        .map_err(|e| match e {
            LookupError::UnrepresentableTarget { .. } => transport("NOT_PORTED", e.to_string()),
            other => ruby(&other.class_name(), other.to_string()),
        })
    };
    let first = call()?;
    let same = if request.repeat {
        Some(match (&first, &call()?) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        })
    } else {
        None
    };
    let value = first.map_or(Json::Null, |tool| super::ops::tool_identity(&tool));
    Ok((value, same))
}
