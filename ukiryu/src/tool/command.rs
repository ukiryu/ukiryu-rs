//! Building and running a command: `Tool#execute`, `#execute_simple`,
//! `#execute_action`, `#normalize_params` and `#execute_with_config`.
use crate::command_builder::{
    assemble, ArgumentSpec, BuildContext, CommandSpec, EnvVarSets, EnvVarSpec, FlagSpec,
    Invocation, OptionSpec,
};
use crate::environment::Environment;
use crate::execution::Seconds;
use crate::executor::{self, ExecuteRequest, Execution};
use crate::models::{
    ArgumentDefinition, Attr, CommandDefinition, EnvVarDefinition, FlagDefinition,
    OptionDefinition, Text,
};
use crate::types::{StdHost, Value};

use super::resolution::{inspect_path, split_root_path};
use super::{text, Tool, ToolError};

/// A command ready to run: what `#execute` hands to the executor.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltCommand {
    /// Executable, argv and the command's environment variables.
    pub invocation: Invocation,
    /// `command_def.name`, recorded as the command info's command name.
    pub command_name: Option<String>,
    /// The `stdin` parameter, removed from the parameters before building.
    pub stdin: Option<Vec<u8>>,
}

/// The executor settings `#execute` takes besides the parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecuteSettings {
    /// `execution_timeout:` (required, as in the source).
    pub timeout: Seconds,
    /// Variables the command's own variables are applied over. Empty means
    /// the inherited process environment plus the command's variables, which
    /// is what the source passes.
    pub environment: Environment,
}

impl ExecuteSettings {
    pub fn new(timeout: Seconds) -> ExecuteSettings {
        ExecuteSettings {
            timeout,
            environment: Environment::new(),
        }
    }
}

fn opt_text(t: &Option<Text>) -> Option<String> {
    t.as_ref().and_then(text)
}

fn attr_text(a: &Attr<Text>) -> Option<String> {
    a.get().and_then(text)
}

fn texts(list: &Option<Vec<Text>>) -> Option<Vec<String>> {
    list.as_ref()
        .map(|items| items.iter().map(Text::to_ruby_s).collect())
}

fn ints(list: &Option<Vec<Option<i64>>>) -> Option<Vec<i64>> {
    list.as_ref()
        .map(|items| items.iter().flatten().copied().collect())
}

fn option_spec(o: &OptionDefinition) -> OptionSpec {
    OptionSpec {
        name: opt_text(&o.name),
        cli: opt_text(&o.cli),
        r#type: attr_text(&o.r#type),
        assignment_delimiter: attr_text(&o.assignment_delimiter),
        separator: opt_text(&o.separator),
        range: ints(&o.range),
        values: texts(&o.values),
        of: opt_text(&o.of),
    }
}

fn flag_spec(f: &FlagDefinition) -> FlagSpec {
    FlagSpec {
        name: opt_text(&f.name),
        cli: opt_text(&f.cli),
        default: f.default.get().copied(),
        position_constraint: opt_text(&f.position_constraint),
    }
}

fn argument_spec(a: &ArgumentDefinition) -> ArgumentSpec {
    ArgumentSpec {
        name: opt_text(&a.name),
        r#type: attr_text(&a.r#type),
        position: attr_text(&a.position),
        variadic: a.variadic.get().copied(),
        min: a.min,
        max: a.max,
        size: ints(&a.size),
        of: opt_text(&a.of),
        range: ints(&a.range),
        values: texts(&a.values),
    }
}

fn env_var_spec(e: &EnvVarDefinition) -> EnvVarSpec {
    EnvVarSpec {
        name: opt_text(&e.name),
        value: opt_text(&e.value),
        from: opt_text(&e.from),
        platforms: Some(texts(&e.platforms).unwrap_or_default()),
    }
}

/// The builder's view of a typed command definition.
pub(super) fn command_spec(c: &CommandDefinition) -> CommandSpec {
    let list = |items: &Option<Vec<OptionDefinition>>| {
        items.as_ref().map(|v| v.iter().map(option_spec).collect())
    };
    CommandSpec {
        name: opt_text(&c.name),
        subcommand: opt_text(&c.subcommand),
        use_env_vars: texts(&c.use_env_vars),
        options: list(&c.options),
        post_options: list(&c.post_options),
        flags: c.flags.as_ref().map(|v| v.iter().map(flag_spec).collect()),
        arguments: c
            .arguments
            .as_ref()
            .map(|v| v.iter().map(argument_spec).collect()),
        env_vars: c
            .env_vars
            .as_ref()
            .map(|v| v.iter().map(env_var_spec).collect()),
    }
}

/// `EnvVarDefinition.new(hash)` for one entry of an env var set.
fn env_var_from_hash(h: &Value) -> EnvVarSpec {
    let scalar = |key: &str| {
        h.get_sym_or_str(key)
            .filter(|v| !v.is_nil())
            .map(Value::to_ruby_s)
    };
    let platforms = h
        .get_sym_or_str("platforms")
        .map(|v| v.to_array().iter().map(Value::to_ruby_s).collect())
        .unwrap_or_default();
    EnvVarSpec {
        name: scalar("name"),
        value: scalar("value"),
        from: scalar("from"),
        platforms: Some(platforms),
    }
}

/// `profile.env_var_sets`, keyed by String as `dig(set_name.to_s)` reads it.
fn env_var_sets(value: Option<&Value>) -> EnvVarSets {
    let mut sets = EnvVarSets::new();
    if let Some(Value::Hash(pairs)) = value {
        for (k, v) in pairs {
            if let Value::String(name) = k {
                let items = v
                    .as_array()
                    .unwrap_or_default()
                    .iter()
                    .filter(|e| matches!(e, Value::Hash(_)))
                    .map(env_var_from_hash)
                    .collect();
                sets.entry(name.clone()).or_insert(items);
            }
        }
    }
    sets
}

/// `#normalize_params`: a Hash none of whose keys are Symbols gets its
/// String keys turned into Symbols.
fn normalize_params(params: &Value) -> Value {
    match params {
        Value::Hash(pairs) if !pairs.iter().any(|(k, _)| matches!(k, Value::Symbol(_))) => {
            Value::Hash(
                pairs
                    .iter()
                    .map(|(k, v)| match k {
                        Value::String(s) => (Value::Symbol(s.clone()), v.clone()),
                        other => (other.clone(), v.clone()),
                    })
                    .collect(),
            )
        }
        Value::Hash(_) => params.clone(),
        _ => Value::Hash(Vec::new()),
    }
}

/// `params.delete(:stdin)`.
fn take_stdin(params: &mut Value) -> Result<Option<Vec<u8>>, ToolError> {
    let Value::Hash(pairs) = params else {
        return Ok(None);
    };
    let Some(i) = pairs
        .iter()
        .position(|(k, _)| matches!(k, Value::Symbol(s) if s == "stdin"))
    else {
        return Ok(None);
    };
    match pairs.remove(i).1 {
        Value::Nil => Ok(None),
        Value::String(s) => Ok(Some(s.into_bytes())),
        other => Err(ToolError::Argument {
            message: format!(
                "stdin must be a String here (got {}); attach a reader with Tool::request",
                other.class_name()
            ),
        }),
    }
}

impl Tool {
    /// `#execute(root_path, **params)` up to the executor call: a path with
    /// `:` is a routed action (`remote:add`), anything else a command name.
    /// `params` is the Symbol-keyed Hash `Options#to_params` produces.
    pub fn build(&self, root_path: &str, params: &Value) -> Result<BuiltCommand, ToolError> {
        let (mut built, command) = self.build_parts(root_path, params)?;
        if let Some(command) = command {
            built.invocation.executable = self.command_executable(command)?;
        }
        Ok(built)
    }

    /// `#build_args` and `#build_env_vars` for `root_path`, without the
    /// executable choice [`Tool::build`] makes afterwards: a routed action's
    /// invocation names its routing target, a command's names no executable
    /// (an empty string). Parameter validation and its errors are those of
    /// [`Tool::build`].
    pub fn build_args(&self, root_path: &str, params: &Value) -> Result<BuiltCommand, ToolError> {
        self.build_parts(root_path, params).map(|(built, _)| built)
    }

    fn build_parts(
        &self,
        root_path: &str,
        params: &Value,
    ) -> Result<(BuiltCommand, Option<&CommandDefinition>), ToolError> {
        if root_path.contains(':') {
            let path = split_root_path(root_path);
            let resolution =
                self.resolve_action_path(&path)
                    .ok_or_else(|| ToolError::Argument {
                        message: format!("Cannot resolve action path: {}", inspect_path(&path)),
                    })?;
            let action = resolution.action.ok_or_else(|| ToolError::Argument {
                message: format!("Action not found: {}", inspect_path(&path)),
            })?;
            Ok((
                self.build_command(resolution.executable, action, params)?,
                None,
            ))
        } else {
            let command =
                self.command_definition(root_path)
                    .ok_or_else(|| ToolError::Argument {
                        message: format!("Unknown command: {root_path}"),
                    })?;
            // Arguments are built (and validated) before the executable is
            // chosen, as in the source.
            Ok((
                self.build_command(String::new(), command, params)?,
                Some(command),
            ))
        }
    }

    fn build_command(
        &self,
        executable: String,
        command: &CommandDefinition,
        params: &Value,
    ) -> Result<BuiltCommand, ToolError> {
        let mut params = normalize_params(params);
        let stdin = take_stdin(&mut params)?;
        let shell =
            crate::cache::builtin_shell(self.shell.as_str()).map_err(|e| ToolError::Ruby {
                class: "Ukiryu::Errors::UnknownShellError",
                message: e.to_string(),
            })?;
        let ctx = BuildContext {
            shell: shell.as_ref(),
            platform: self.platform.as_str(),
            host: &StdHost,
        };
        let sets = env_var_sets(self.command_profile().env_var_sets.get());
        let spec = command_spec(command);
        let invocation = assemble(executable, &spec, Some(&sets), &params, &ctx)
            .map_err(|e| ToolError::Build(Box::new(e)))?;
        Ok(BuiltCommand {
            invocation,
            command_name: opt_text(&command.name),
            stdin,
        })
    }

    /// `#execute_with_config` before running: the executor request for a
    /// built command.
    pub fn request(
        &self,
        built: BuiltCommand,
        settings: &ExecuteSettings,
    ) -> Result<ExecuteRequest, ToolError> {
        let BuiltCommand {
            invocation,
            command_name,
            stdin,
        } = built;
        let mut env = Vec::with_capacity(invocation.env.0.len());
        for (name, value) in invocation.env.0 {
            let name = name.ok_or_else(|| ToolError::Ruby {
                class: "TypeError",
                message: "no implicit conversion of nil into String".into(),
            })?;
            env.push((name, value));
        }
        let environment = settings.environment.merge_pairs(env);
        let mut request = ExecuteRequest::new(
            invocation.executable,
            invocation.args,
            environment,
            self.shell,
            settings.timeout,
        );
        if let Some(name) = self.name() {
            request = request.tool_name(name);
        }
        if let Some(name) = command_name {
            request = request.command_name(name);
        }
        if let Some(stdin) = stdin {
            request = request.stdin_bytes(stdin);
        }
        Ok(request)
    }

    /// `#execute(root_path, execution_timeout:, **params)`.
    pub fn execute(
        &self,
        root_path: &str,
        params: &Value,
        settings: &ExecuteSettings,
    ) -> Result<Execution, ToolError> {
        let built = self.build(root_path, params)?;
        let request = self.request(built, settings)?;
        executor::execute(request).map_err(ToolError::from)
    }
}
