//! `ukiryu exec`, `ukiryu run-file` and `ukiryu cache info|stats|clear`.
//!
//! Source: ukiryu `lib/ukiryu/cli_commands/{run,run_file,cache,base}_command.rb`,
//! `response_formatter.rb` and `cli.rb` at ab6df353c6dc2d7faf7817a86aafcafa82579896,
//! with Thor 1.5.0 output rules and `FriendlyCLI` error handling.
//!
//! Each handler is a plain function over a typed input that returns a
//! [`CommandOutcome`]: the ordered writes and the exit status. Rendering goes
//! through [`ukiryu::render`], tools through the typed [`Tool`] runtime,
//! requests through [`ukiryu::request`] and processes through
//! [`ukiryu::executor`]. Nothing here reads process state except the stdin
//! reader the caller passes, files the command names (`stdin=@FILE`,
//! `--definition`, the request file) and the process the tool runs.
//!
//! # How the outcome maps to the source
//!
//! - A rendered response (success or error) is exit status 0: `Cli` sets
//!   `exit_on_failure?` to false and the commands return normally.
//! - `error!` (`Thor::Error`) is the message on stderr and exit status 1, as
//!   `FriendlyCLI.start` prints it (uncoloured; Thor colours it only on a
//!   terminal stderr).
//! - Any other exception escapes the command; the handler returns
//!   [`ExecuteError::Raised`], and [`ExecuteError::friendly_message`] is the
//!   `[Class] message` line `FriendlyCLI` prints before exit status 1.
//!
//! # Register
//!
//! Tools come from the register through [`RegisterSource`] and
//! [`tool::get`]: `exec` without `--definition` and `run-file` resolve the
//! tool there, and `exec --definition` still checks the command against the
//! register's tool (`OptionsBuilder.for` runs `Tool.get(name)`). The register
//! is `config.register` (`--register` or `UKIRYU_REGISTER`) when that
//! directory exists, else the default register found without cloning. When
//! no register can be found without the source's auto-clone, the handler
//! returns [`ExecuteError::Gated`].
//!
//! `Tool.find_by` (interface and alias discovery through `ToolFinder`) is
//! not ported: the tool is looked up by name with `Tool.get`, which is the
//! source's fallback when `find_by` finds nothing.
//!
//! # stdin
//!
//! `--stdin` and `stdin=-` read the whole of the reader the caller passes
//! (the process stdin in the binary); `stdin=@FILE` reads the file. The data
//! is handed to the tool as its `stdin` parameter. This handling is local to
//! this module until a shared stdin module exists.
//!
//! # Not covered (returns [`ExecuteError::Unsupported`])
//!
//! - `UKIRYU_DEBUG` (the `Logger` debug sections) and `UKIRYU_METRICS` (the
//!   execution report).
//! - stdin bytes that are not UTF-8.
//! - A parameter value the shared YAML loader does not reproduce.
//! - `setup_register` / `default_register_path` side effects (setting
//!   `UKIRYU_REGISTER`, cloning the default register) are not performed;
//!   they do not change the output of the covered paths.

mod cache;
mod run_file;

pub use cache::{cache, CacheAction, CacheInput};
pub use run_file::{run_file, RunFileInput};

use super::CommandOutcome;
use serde_json::Value as Json;
use std::fmt;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;
use ukiryu::cache::ToolCache;
use ukiryu::config::{self, OverrideResolver, Values};
use ukiryu::definition::{
    DefinitionDocument, DefinitionError, DocumentParseError, Loader, ValidationMode,
};
use ukiryu::environment::Environment;
use ukiryu::executable::DiscoveryContext;
use ukiryu::execution::{InvalidByteSequence, ResultPartsError, Seconds};
use ukiryu::executor::{self, ExecuteError as ProcessError};
use ukiryu::extractors::base::ProcessHost;
use ukiryu::models::attr::Model;
use ukiryu::models::error::ModelErrorKind;
use ukiryu::models::tool_definition::ToolDefinition;
use ukiryu::models::{CommandDefinition, ErrorResponse, ExecutionResultParts, SuccessResponse};
use ukiryu::register::{self, Register};
use ukiryu::render::{
    output_response, render_dry_run, DryRunStyle, OutputFormat, OutputWrite, PaintSupport,
    RenderError, RenderSettings,
};
use ukiryu::runtime::{Clock, Host, Runtime};
use ukiryu::schema::{safe_load, Data, Permitted, SchemaError};
use ukiryu::tool::{self, ExecuteSettings, Tool, ToolError, ToolOptions};
use ukiryu::tool_loader::RegisterSource;
use ukiryu::types::{ErrorClass, Value};

/// `execute_request`'s `config.timeout || 90`.
const DEFAULT_TIMEOUT: i64 = 90;

/// The process facts the handlers read, captured once by the caller.
#[derive(Clone, Debug)]
pub struct ExecuteContext {
    /// `Platform.detect` and a snapshot of `ENV` (configuration, shell
    /// detection, executable search and the executor's shell search path).
    pub host: Host,
    /// `$stdout.tty?` (YAML colouring with `use_color` unset).
    pub stdout_tty: bool,
    /// Availability of the `paint` gem's behaviour.
    pub paint: PaintSupport,
}

impl ExecuteContext {
    fn render_settings(&self, config: &OverrideResolver) -> RenderSettings {
        RenderSettings {
            use_color: match config.resolve("use_color") {
                Some(Json::Bool(b)) => Some(*b),
                _ => None,
            },
            stdout_tty: self.stdout_tty,
            paint: self.paint,
        }
    }
}

/// Why a handler produced no outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecuteError {
    /// The step needs register behaviour that is not ported (default-command
    /// metadata, `Tool.find_by`, the default register's auto-clone).
    /// `partial` holds the writes made before the step.
    Gated {
        stage: &'static str,
        detail: String,
        partial: Vec<OutputWrite>,
    },
    /// An exception the source does not rescue; `FriendlyCLI` prints
    /// [`ExecuteError::friendly_message`] and exits with status 1.
    Raised {
        class: String,
        message: String,
        partial: Vec<OutputWrite>,
    },
    /// Behaviour this port does not reproduce (see the module docs).
    Unsupported { feature: String },
}

impl ExecuteError {
    fn gated(stage: &'static str, detail: impl Into<String>, partial: Vec<OutputWrite>) -> Self {
        ExecuteError::Gated {
            stage,
            detail: detail.into(),
            partial,
        }
    }

    fn raised(class: impl Into<String>, message: impl Into<String>) -> Self {
        ExecuteError::Raised {
            class: class.into(),
            message: message.into(),
            partial: Vec::new(),
        }
    }

    fn unsupported(feature: impl Into<String>) -> Self {
        ExecuteError::Unsupported {
            feature: feature.into(),
        }
    }

    /// The stderr line `FriendlyCLI.start` prints for [`ExecuteError::Raised`]
    /// (`[Class] message`); for the other variants a plain description.
    pub fn friendly_message(&self) -> String {
        match self {
            ExecuteError::Raised { class, message, .. } => format!("[{class}] {message}"),
            other => other.to_string(),
        }
    }

    /// Writes performed before the failure.
    pub fn partial(&self) -> &[OutputWrite] {
        match self {
            ExecuteError::Gated { partial, .. } | ExecuteError::Raised { partial, .. } => partial,
            ExecuteError::Unsupported { .. } => &[],
        }
    }
}

impl fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecuteError::Gated { stage, detail, .. } => {
                write!(f, "not available: {stage} ({detail})")
            }
            ExecuteError::Raised { class, message, .. } => write!(f, "{class}: {message}"),
            ExecuteError::Unsupported { feature } => write!(f, "not supported: {feature}"),
        }
    }
}

impl std::error::Error for ExecuteError {}

impl From<RenderError> for ExecuteError {
    fn from(e: RenderError) -> Self {
        // The json gem's GeneratorError, or a model that cannot be emitted.
        match e.kind() {
            ukiryu::render::RenderErrorKind::Generator => {
                ExecuteError::raised("JSON::GeneratorError", e.message())
            }
            _ => ExecuteError::unsupported(format!("response rendering: {}", e.message())),
        }
    }
}

const REGISTER_DETAIL: &str =
    "Register.load_tool_metadata and Tool.find_by (ToolFinder) are not ported";

// ------------------------------------------------------------- configuration

/// `Ukiryu::Config.instance` after `apply_cli_options_to_config`.
///
/// `format` is a CLI value only when it differs from Thor's default `yaml`;
/// `output`, `register` and `shell` when present and not empty; `dry_run`
/// always (Thor passes its default `false`, which masks `UKIRYU_DRY_RUN`).
pub(crate) fn configuration(
    host: &Host,
    format: Option<&str>,
    output: Option<&str>,
    register: Option<&str>,
    shell: Option<&str>,
    dry_run: Option<bool>,
) -> Result<OverrideResolver, ExecuteError> {
    let environment: config::Environment = host
        .env
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let env = config::environment_values(&environment)
        .map_err(|e| ExecuteError::raised("ArgumentError", e.to_string()))?;
    let mut cli = Values::new();
    if let Some(f) = format.filter(|f| *f != "yaml") {
        cli.insert("format".to_owned(), Json::String(f.to_owned()));
    }
    for (key, value) in [("output", output), ("register", register), ("shell", shell)] {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            cli.insert(key.to_owned(), Json::String(v.to_owned()));
        }
    }
    if let Some(d) = dry_run {
        cli.insert("dry_run".to_owned(), Json::Bool(d));
    }
    Ok(OverrideResolver {
        defaults: config::defaults(),
        programmatic: Values::new(),
        env,
        cli,
    })
}

fn truthy(v: Option<&Json>) -> bool {
    !matches!(v, None | Some(Json::Null) | Some(Json::Bool(false)))
}

fn config_string(config: &OverrideResolver, key: &str) -> Option<String> {
    match config.resolve(key) {
        Some(Json::String(s)) => Some(s.clone()),
        Some(Json::Null) | None => None,
        Some(other) => Some(other.to_string()),
    }
}

/// `config.format.to_sym` validated against `OUTPUT_FORMATS`; `Err` is the
/// `error!` message.
fn config_format(config: &OverrideResolver) -> Result<Result<OutputFormat, String>, ExecuteError> {
    let Some(name) = config_string(config, "format") else {
        return Err(ExecuteError::raised(
            "NoMethodError",
            "undefined method 'to_sym' for nil",
        ));
    };
    Ok(OutputFormat::parse(&name).map_err(|e| e.message().to_owned()))
}

/// `config.timeout || 90`.
fn config_timeout(config: &OverrideResolver) -> Seconds {
    match config.resolve("timeout") {
        Some(Json::Number(n)) => match n.as_i64() {
            Some(i) => Seconds::Int(i),
            None => Seconds::Float(n.as_f64().unwrap_or(DEFAULT_TIMEOUT as f64)),
        },
        _ => Seconds::Int(DEFAULT_TIMEOUT),
    }
}

/// Fails for the configuration switches whose output this port does not
/// produce (`UKIRYU_DEBUG` logger sections).
fn reject_debug(config: &OverrideResolver) -> Result<(), ExecuteError> {
    if truthy(config.resolve("debug")) {
        return Err(ExecuteError::unsupported(
            "debug logging (UKIRYU_DEBUG): Logger sections are not ported",
        ));
    }
    Ok(())
}

fn reject_metrics(config: &OverrideResolver) -> Result<(), ExecuteError> {
    if truthy(config.resolve("metrics")) {
        return Err(ExecuteError::unsupported(
            "execution metrics (UKIRYU_METRICS): the execution report is not ported",
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------ output

/// `BaseCommand#say` and `error!` writes in order.
pub(crate) struct Out {
    writes: Vec<OutputWrite>,
    colors_disabled: bool,
}

/// `BaseCommand#say` colours (`:clear` and other names are 37).
#[derive(Clone, Copy)]
enum Color {
    Cyan,
    White,
    Dim,
    Clear,
}

impl Color {
    fn code(self) -> u8 {
        match self {
            Color::Cyan => 36,
            Color::White | Color::Clear => 37,
            Color::Dim => 2,
        }
    }
}

impl Out {
    pub(crate) fn new(colors_disabled: bool) -> Out {
        Out {
            writes: Vec::new(),
            colors_disabled,
        }
    }

    fn say(&mut self, message: &str, color: Color) {
        let mut text = if self.colors_disabled {
            message.to_owned()
        } else {
            format!("\x1b[{}m{message}\x1b[0m", color.code())
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.writes.push(OutputWrite::Stdout(text));
    }

    pub(crate) fn push(&mut self, write: OutputWrite) {
        self.writes.push(write);
    }

    pub(crate) fn done(self) -> CommandOutcome {
        CommandOutcome {
            writes: self.writes,
            exit_code: 0,
        }
    }

    /// `error!`: `Thor::Error`, printed by `FriendlyCLI` on stderr, status 1.
    pub(crate) fn error(mut self, message: &str) -> CommandOutcome {
        let mut text = message.to_owned();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.writes.push(OutputWrite::Stderr(text));
        CommandOutcome {
            writes: self.writes,
            exit_code: 1,
        }
    }

    pub(crate) fn take(self) -> Vec<OutputWrite> {
        self.writes
    }
}

fn colors_disabled(config: &OverrideResolver) -> bool {
    config.resolve("use_color") == Some(&Json::Bool(false))
}

// -------------------------------------------------------------- definition

/// `Models::ToolDefinition` as the definition loader's document, for
/// `Tool.load(path, validation: :strict)`.
pub(crate) struct ToolDocument {
    definition: ToolDefinition,
    name: Option<String>,
    version: Option<String>,
    ukiryu_schema: Option<String>,
    self_uri: Option<String>,
}

impl DefinitionDocument for ToolDocument {
    fn parse_definition(yaml: &str) -> Result<Self, DocumentParseError> {
        let definition = ToolDefinition::from_yaml(yaml).map_err(|e| match e.kind {
            ModelErrorKind::Syntax | ModelErrorKind::InvalidFormat => {
                DocumentParseError::Format(e.message)
            }
            _ => DocumentParseError::Other(e.message),
        })?;
        let text = |t: &Option<ukiryu::models::value::Text>| t.as_ref().map(|t| t.to_ruby_s());
        Ok(ToolDocument {
            name: text(&definition.name),
            version: text(&definition.version),
            ukiryu_schema: text(&definition.ukiryu_schema),
            self_uri: text(&definition.self_uri),
            definition,
        })
    }

    fn definition_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn definition_version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    fn has_profiles(&self) -> bool {
        self.definition
            .profiles
            .as_ref()
            .is_some_and(|p| !p.is_empty())
    }

    fn ukiryu_schema(&self) -> Option<&str> {
        self.ukiryu_schema.as_deref()
    }

    fn self_uri(&self) -> Option<&str> {
        self.self_uri.as_deref()
    }

    fn resolve_inheritance(&mut self) {
        self.definition.resolve_inheritance();
    }
}

/// `Definition::Loader.load_from_source(FileSource.new(path), validation: :strict)`.
pub fn load_definition(path: &str) -> Result<ToolDefinition, DefinitionError> {
    let doc: Arc<ToolDocument> =
        Loader::<ToolDocument>::new().load_from_file(path, ValidationMode::Strict)?;
    Ok(doc.definition.clone())
}

/// `Tool.new(profile, options)` with the platform and shell of
/// `Runtime.instance` (`Config.shell`, else detection) and the host's `PATH`.
pub fn tool_options(
    host: &Host,
    configured_shell: Option<String>,
) -> Result<ToolOptions, ExecuteError> {
    let runtime = Runtime::new(host.clone(), configured_shell, Clock::default());
    let options = ToolOptions::from_runtime(&runtime)
        .map_err(|e| ExecuteError::raised("Ukiryu::Errors::UnknownShellError", e.to_string()))?;
    let discovery = DiscoveryContext::new(
        options.platform,
        options.shell.as_str(),
        host.env.get("PATH"),
        host.env.get("PATHEXT"),
    );
    Ok(options.discovery(discovery))
}

// -------------------------------------------------------------------- exec

/// `ukiryu exec TOOL [COMMAND] [KEY=VALUE ...]` options, as given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecInput {
    /// `TOOL`.
    pub tool: String,
    /// The positional words after `TOOL`: `COMMAND` (or a first `KEY=VALUE`)
    /// and the parameters, in order.
    pub arguments: Vec<String>,
    /// `--register` / `-r`.
    pub register: Option<String>,
    /// `--definition` / `-d`.
    pub definition: Option<String>,
    /// `--format` / `-f` (`None` when not given).
    pub format: Option<String>,
    /// `--output` / `-o`.
    pub output: Option<String>,
    /// `--dry-run` / `-D` (Thor default false).
    pub dry_run: bool,
    /// `--shell` / `-s`.
    pub shell: Option<String>,
    /// `--stdin`.
    pub stdin: bool,
    /// `--raw`.
    pub raw: bool,
}

/// The request arguments: Symbol names with parsed values, in Hash order.
pub type Arguments = Vec<(String, Value)>;

fn upsert(arguments: &mut Arguments, key: &str, value: Value) {
    match arguments.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => arguments.push((key.to_owned(), value)),
    }
}

/// A loaded YAML value as the Ruby object it is. Date and Time cannot occur
/// (not permitted), so they are reported as the loader rejecting the text.
fn ruby_value(data: Data) -> Option<Value> {
    Some(match data {
        Data::Null => Value::Nil,
        Data::Bool(b) => Value::Bool(b),
        Data::Integer(n) => Value::Integer(n),
        Data::Float(f) => Value::Float(f),
        Data::String(s) => Value::String(s),
        Data::Symbol(s) => Value::Symbol(s),
        Data::Date { .. } | Data::Time(_) => return None,
        Data::Array(items) => Value::Array(
            items
                .into_iter()
                .map(ruby_value)
                .collect::<Option<Vec<_>>>()?,
        ),
        Data::Hash(pairs) => Value::Hash(
            pairs
                .into_iter()
                .map(|(k, v)| Some((ruby_value(k)?, ruby_value(v)?)))
                .collect::<Option<Vec<_>>>()?,
        ),
    })
}

/// Why `parse_inline_params` stopped.
enum ParamError {
    /// `error!` message.
    Thor(String),
    Port(ExecuteError),
}

/// `RunCommand#parse_inline_params`: `key=value` pairs, values loaded with
/// `YAML.safe_load(value, permitted_classes: [Symbol])` and kept as the
/// String when loading raises; `stdin=-` and `stdin=@...` stay literal.
fn parse_inline_params(params: &[String]) -> Result<Arguments, ParamError> {
    let mut arguments = Arguments::new();
    for param in params {
        let Some((key, raw)) = param.split_once('=') else {
            return Err(ParamError::Thor(format!(
                "Invalid parameter format: {param}. Use key=value"
            )));
        };
        let skip = key == "stdin" && (raw == "-" || raw.starts_with('@'));
        let value = if skip {
            Value::str(raw)
        } else {
            match safe_load(raw, Permitted::SYMBOL) {
                Ok(data) => ruby_value(data).unwrap_or_else(|| Value::str(raw)),
                Err(SchemaError::Unsupported { feature, detail }) => {
                    return Err(ParamError::Port(ExecuteError::unsupported(format!(
                        "parameter value {raw:?} needs YAML behaviour that is not reproduced ({feature}: {detail})"
                    ))))
                }
                Err(_) => Value::str(raw),
            }
        };
        upsert(&mut arguments, key, value);
    }
    Ok(arguments)
}

fn stdin_string(bytes: Vec<u8>) -> Result<Value, ExecuteError> {
    String::from_utf8(bytes)
        .map(Value::String)
        .map_err(|_| ExecuteError::unsupported("stdin data that is not UTF-8"))
}

/// `File.read(path)` failures other than `ENOENT`, as the `Errno` the source
/// lets escape.
fn file_read_error(path: &str, error: &std::io::Error) -> ExecuteError {
    if std::path::Path::new(path).is_dir() {
        return ExecuteError::raised(
            "Errno::EISDIR",
            format!("Is a directory @ io_fread - {path}"),
        );
    }
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ExecuteError::raised(
            "Errno::EACCES",
            format!("Permission denied @ rb_sysopen - {path}"),
        ),
        _ => ExecuteError::unsupported(format!("reading {path}: {error}")),
    }
}

/// The stdin step of `RunCommand#run`. `Ok(Err(message))` is `error!`.
fn resolve_stdin(
    input_stdin: bool,
    arguments: &mut Arguments,
    stdin: &mut dyn Read,
) -> Result<Result<(), String>, ExecuteError> {
    let current = arguments
        .iter()
        .find(|(k, _)| k == "stdin")
        .map(|(_, v)| v.clone());
    if input_stdin || current.as_ref().and_then(Value::as_str) == Some("-") {
        let mut bytes = Vec::new();
        stdin
            .read_to_end(&mut bytes)
            .map_err(|e| ExecuteError::unsupported(format!("reading stdin: {e}")))?;
        upsert(arguments, "stdin", stdin_string(bytes)?);
    } else if let Some(path) = current
        .as_ref()
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix('@'))
    {
        match std::fs::read(path) {
            Ok(bytes) => upsert(arguments, "stdin", stdin_string(bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Err(format!("File not found: {path}")))
            }
            Err(e) => return Err(file_read_error(path, &e)),
        }
    }
    Ok(Ok(()))
}

/// `RunCommand#run(tool_name, command_name = nil, *params)`.
///
/// `stdin` is the reader `--stdin` / `stdin=-` consume (the process stdin in
/// the binary).
pub fn exec(
    input: ExecInput,
    ctx: &ExecuteContext,
    stdin: &mut dyn Read,
) -> Result<CommandOutcome, ExecuteError> {
    let config = configuration(
        &ctx.host,
        input.format.as_deref(),
        input.output.as_deref(),
        input.register.as_deref(),
        input.shell.as_deref(),
        Some(input.dry_run),
    )?;
    let out = Out::new(colors_disabled(&config));

    let mut words = input.arguments.iter();
    let mut command = words.next().cloned();
    let mut params: Vec<String> = words.cloned().collect();
    if command.as_deref().is_some_and(|c| c.contains('=')) {
        params.insert(0, command.take().unwrap_or_default());
    }

    if command.as_deref() == Some("help") {
        return show_tool_help(&input, ctx, &config, out);
    }

    let command = match command {
        Some(c) => c,
        None => resolve_default_command(&input, ctx, &config)?,
    };

    let mut arguments = match parse_inline_params(&params) {
        Ok(a) => a,
        Err(ParamError::Thor(m)) => return Ok(out.error(&m)),
        Err(ParamError::Port(e)) => return Err(e),
    };
    if let Err(message) = resolve_stdin(input.stdin, &mut arguments, stdin)? {
        return Ok(out.error(&message));
    }
    reject_debug(&config)?;

    let format = if input.raw {
        OutputFormat::Raw
    } else {
        match config_format(&config)? {
            Ok(f) => f,
            Err(message) => return Ok(out.error(&message)),
        }
    };

    if truthy(config.resolve("dry_run")) {
        let text = render_dry_run(
            DryRunStyle::Exec,
            &input.tool,
            &command,
            &arguments,
            render_use_color(&config),
        );
        let mut out = out;
        out.push(OutputWrite::Stdout(text));
        return Ok(out.done());
    }

    reject_metrics(&config)?;
    let options = tool_options(&ctx.host, config_string(&config, "shell"))?;
    let registry = Registry::new(ctx, &config, options.shell);
    let tool = match input.definition.as_deref() {
        Some(path) => match load_definition(path) {
            Err(e) => Err(rescued_definition_error(&e)),
            Ok(definition) => match Tool::new(definition, &options) {
                Err(e) => Err(tool_error_response(e)?),
                Ok(tool) => match tool.name() {
                    None => Err(ErrorResponse::from_message(
                        "Unexpected error: NoMethodError: undefined method 'to_sym' for nil",
                        1,
                    )),
                    Some(name) if name != input.tool => Err(ErrorResponse::from_message(
                        &format!(
                            "Ukiryu::Tool name mismatch: definition file contains '{name}' but command specified '{}'",
                            input.tool
                        ),
                        1,
                    )),
                    Some(_) => Ok(Rc::new(tool)),
                },
            },
        },
        None => registry.get(&input.tool, &options, &out)?,
    };
    let response = match tool {
        Err(e) => Response::Error(e),
        Ok(tool) => {
            let without_stdin: Vec<&str> = arguments
                .iter()
                .map(|(k, _)| k.as_str())
                .filter(|k| *k != "stdin")
                .collect();
            match options_check(
                &tool,
                &input.tool,
                &command,
                &without_stdin,
                &registry,
                &options,
                &out,
            )? {
                Some(error) => Response::Error(error),
                None => {
                    let settings = ExecuteSettings::new(config_timeout(&config));
                    respond(
                        &tool,
                        &command,
                        &arguments,
                        &settings,
                        ctx.host.env.get("PATH"),
                    )?
                }
            }
        }
    };
    let settings = ctx.render_settings(&config);
    let output = config_string(&config, "output");
    render(&response, format, output.as_deref(), &settings)
}

/// The register `setup_register` and `Register.default` select, and the
/// tool cache (`ToolCache`) of this invocation.
pub(crate) struct Registry {
    path: Option<std::path::PathBuf>,
    platform: ukiryu::executable::Platform,
    shell: ukiryu::runtime::ShellName,
    cache: ToolCache<Rc<Tool>>,
}

impl Registry {
    /// `config.register` when that directory exists, else the default
    /// register resolved without cloning (`UKIRYU_REGISTER`, then
    /// `~/.ukiryu/register` when it has a valid structure).
    pub(crate) fn new(
        ctx: &ExecuteContext,
        config: &OverrideResolver,
        shell: ukiryu::runtime::ShellName,
    ) -> Registry {
        let configured = config_string(config, "register")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir());
        let path = configured.or_else(|| {
            register::DiscoveryContext {
                env_register: ctx.host.env.get("UKIRYU_REGISTER").map(Into::into),
                dev_path: None,
                home: ctx
                    .host
                    .env
                    .get("HOME")
                    .filter(|h| !h.is_empty())
                    .map(std::path::PathBuf::from),
            }
            .resolve_path_without_clone()
        });
        Registry {
            path,
            platform: ctx.host.platform,
            shell,
            cache: ToolCache::new(),
        }
    }

    /// `Tool.get(name)`: the tool, or the `ErrorResponse` of the `rescue`
    /// that catches its failure.
    pub(crate) fn get(
        &self,
        name: &str,
        options: &ToolOptions,
        out: &Out,
    ) -> Result<Result<Rc<Tool>, ErrorResponse>, ExecuteError> {
        let Some(path) = self.path.as_ref() else {
            return Err(ExecuteError::gated(
                "Register.default (auto-clone)",
                "no register exists and cloning the default register is not performed",
                out.writes.clone(),
            ));
        };
        let source = RegisterSource::new(
            Register::at(path),
            self.platform,
            ProcessHost::new(Environment::new(), self.shell),
        );
        match tool::get(&source, &self.cache, name, options) {
            Ok(t) => Ok(Ok(t)),
            Err(e) => Ok(Err(tool_error_response(e)?)),
        }
    }
}

/// `tool.available?`, then `OptionsBuilder.for(tool.name, command)` (the
/// register's tool must define the command) and the options object's
/// setters. `Some` is the response of the `rescue` that catches a failure.
#[allow(clippy::too_many_arguments)]
fn options_check(
    tool: &Tool,
    tool_name: &str,
    command: &str,
    setter_keys: &[&str],
    registry: &Registry,
    options: &ToolOptions,
    out: &Out,
) -> Result<Option<ErrorResponse>, ExecuteError> {
    if !tool.is_available() {
        return Ok(Some(ErrorResponse::from_message(
            &format!("Ukiryu::Tool found but not executable: {tool_name}"),
            1,
        )));
    }
    let profile_name = tool.name().unwrap_or_default();
    let registered = match registry.get(&profile_name, options, out)? {
        Ok(t) => t,
        Err(response) => return Ok(Some(response)),
    };
    let Some(definition) = registered.command_definition(command) else {
        return Ok(Some(ErrorResponse::from_message(
            &format!("Invalid arguments: Unknown command: {command} for tool: {profile_name}"),
            1,
        )));
    };
    let names = setter_names(definition);
    if let Some(key) = setter_keys.iter().find(|k| !names.iter().any(|n| n == *k)) {
        return Err(ExecuteError::unsupported(format!(
            "parameter '{key}' has no setter on the options class: the source's NoMethodError names an anonymous class by address"
        )));
    }
    Ok(None)
}

/// The accessors `OptionsBuilder.define_accessors` defines.
fn setter_names(c: &CommandDefinition) -> Vec<String> {
    let text = |t: &Option<ukiryu::models::value::Text>| t.as_ref().map(|t| t.to_ruby_s());
    let mut names = Vec::new();
    names.extend(c.arguments.iter().flatten().filter_map(|a| text(&a.name)));
    names.extend(c.options.iter().flatten().filter_map(|o| text(&o.name)));
    names.extend(c.flags.iter().flatten().filter_map(|f| text(&f.name)));
    names.extend(
        c.post_options
            .iter()
            .flatten()
            .filter_map(|o| text(&o.name)),
    );
    names
}

/// The `use_color` `say_dry_run` reads (`Config#use_color`).
fn render_use_color(config: &OverrideResolver) -> Option<bool> {
    match config.resolve("use_color") {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// `resolve_default_command`: with `--definition` the loaded
/// `ToolDefinition` has no `default_command`, so the source always raises
/// `NoMethodError` (after any load error); without it the register is read.
fn resolve_default_command(
    input: &ExecInput,
    ctx: &ExecuteContext,
    config: &OverrideResolver,
) -> Result<String, ExecuteError> {
    let Some(path) = input.definition.as_deref() else {
        return Err(ExecuteError::gated(
            "Register.load_tool_metadata (default command)",
            REGISTER_DETAIL,
            Vec::new(),
        ));
    };
    let definition = load_definition(path).map_err(raised_definition_error)?;
    let options = tool_options(&ctx.host, config_string(config, "shell"))?;
    Tool::new(definition, &options).map_err(raised_tool_error)?;
    Err(ExecuteError::raised(
        "NoMethodError",
        "undefined method 'default_command' for an instance of Ukiryu::Models::ToolDefinition",
    ))
}

fn raised_definition_error(e: DefinitionError) -> ExecuteError {
    ExecuteError::raised(e.ruby_class(), e.message)
}

fn raised_tool_error(e: ToolError) -> ExecuteError {
    let class = match &e {
        ToolError::Build(b) => b.class.ruby_name().to_owned(),
        ToolError::Ruby { class, .. } => (*class).to_owned(),
        ToolError::Argument { .. } => "ArgumentError".to_owned(),
        other => other
            .kind()
            .map(|k| k.ruby_class().to_owned())
            .unwrap_or_else(|| "StandardError".to_owned()),
    };
    ExecuteError::raised(class, e.to_string())
}

/// A definition load failure inside `execute_request`'s `begin`: none of
/// the definition errors has its own `rescue`, so they reach the
/// `StandardError` branch (`ArgumentError` its own).
fn rescued_definition_error(e: &DefinitionError) -> ErrorResponse {
    let message = match e.ruby_class() {
        "ArgumentError" => format!("Invalid arguments: {}", e.message),
        class => format!("Unexpected error: {class}: {}", e.message),
    };
    ErrorResponse::from_message(&message, 1)
}

/// `show_tool_help(tool_name)`: with `--definition` the tool is loaded and
/// described; without it the source reads the register.
fn show_tool_help(
    input: &ExecInput,
    ctx: &ExecuteContext,
    config: &OverrideResolver,
    mut out: Out,
) -> Result<CommandOutcome, ExecuteError> {
    let Some(path) = input.definition.as_deref() else {
        return Err(ExecuteError::gated(
            "Tool.find_by (exec help)",
            REGISTER_DETAIL,
            out.take(),
        ));
    };
    let definition = load_definition(path).map_err(raised_definition_error)?;
    let options = tool_options(&ctx.host, config_string(config, "shell"))?;
    let tool = Tool::new(definition, &options).map_err(raised_tool_error)?;
    let text = |t: &Option<ukiryu::models::value::Text>| t.as_ref().map(|t| t.to_ruby_s());
    let profile = tool.definition();
    let name = tool.name().unwrap_or_default();
    out.say("", Color::Clear);
    out.say(&format!("Ukiryu::Tool: {name}"), Color::Cyan);
    for (label, value) in [
        ("Display Name", text(&profile.display_name)),
        ("Version", text(&profile.version)),
        ("Homepage", text(&profile.homepage)),
    ] {
        out.say(
            &format!("{label}: {}", value.unwrap_or_else(|| "N/A".to_owned())),
            Color::White,
        );
    }
    out.say("", Color::Clear);
    let commands = tool.commands();
    if commands.is_empty() {
        out.say(
            "This tool has no defined commands (it may be a simple wrapper)",
            Color::Dim,
        );
        out.say(
            &format!("Usage: ukiryu exec {name} [KEY=VALUE ...]"),
            Color::Dim,
        );
        return Ok(out.done());
    }
    out.say("Available commands:", Color::Cyan);
    for cmd in commands {
        let cmd_name = text(&cmd.name).unwrap_or_else(|| "unnamed".to_owned());
        let description = text(&cmd.description).unwrap_or_default();
        out.say(&format!("  {cmd_name:<20} {description}"), Color::White);
        if let Some(usage) = text(&cmd.usage) {
            out.say(&format!("    Usage: {usage}"), Color::Dim);
        }
        if let Some(sub) = text(&cmd.subcommand) {
            out.say(&format!("    Subcommand: {sub}"), Color::Dim);
        }
    }
    out.say("", Color::Clear);
    for line in [
        format!("Usage: ukiryu exec {name} <command> [KEY=VALUE ...]"),
        format!("   or: ukiryu exec {name} help"),
        format!("   or: ukiryu describe {name} <command>"),
    ] {
        out.say(&line, Color::Dim);
    }
    out.say("", Color::Clear);
    out.say("For more information on a specific command:", Color::Dim);
    out.say(&format!("  ukiryu opts {name} <command>"), Color::Dim);
    out.say(&format!("  ukiryu describe {name} <command>"), Color::Dim);
    Ok(out.done())
}

// ---------------------------------------------------- execution + rendering

/// A structured execution response.
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Success(SuccessResponse),
    Error(ErrorResponse),
}

/// `execute_request` after tool resolution and the options object (the
/// register steps): `tool.execute(command.to_sym, execution_timeout:,
/// **arguments)` and `SuccessResponse.from_result`, or the `ErrorResponse`
/// of the matching `rescue`.
///
/// `shell_search_path` is the `PATH` searched for a Unix shell (the host's
/// `PATH` in the CLI).
pub fn respond(
    tool: &Tool,
    command: &str,
    arguments: &[(String, Value)],
    settings: &ExecuteSettings,
    shell_search_path: Option<&str>,
) -> Result<Response, ExecuteError> {
    // The source passes a Symbol, so `execute` never routes an action path:
    // `execute_simple` looks the name up as is.
    if command.contains(':') {
        if tool.command_definition(command).is_none() {
            return Ok(Response::Error(ErrorResponse::from_message(
                &format!("Invalid arguments: Unknown command: {command}"),
                1,
            )));
        }
        return Err(ExecuteError::unsupported(
            "a command name containing ':' (the typed runtime routes it as an action path)",
        ));
    }
    let params = Value::Hash(
        arguments
            .iter()
            .map(|(k, v)| (Value::Symbol(k.clone()), v.clone()))
            .collect(),
    );
    let built = match tool.build(command, &params) {
        Ok(b) => b,
        Err(e) => return tool_error_response(e).map(Response::Error),
    };
    let mut request = match tool.request(built, settings) {
        Ok(r) => r,
        Err(e) => return tool_error_response(e).map(Response::Error),
    };
    if let Some(path) = shell_search_path {
        request = request.shell_search_path(path);
    }
    let execution = match executor::execute(request) {
        Ok(x) => x,
        Err(e) => return tool_error_response(ToolError::from(e)).map(Response::Error),
    };
    let parts = match ExecutionResultParts::try_from(&execution.result) {
        Ok(p) => p,
        Err(ResultPartsError::InvalidByteSequence(e)) => {
            return Ok(Response::Error(ErrorResponse::from_message(
                &invalid_bytes_message(&e),
                1,
            )))
        }
        Err(ResultPartsError::Model(e)) => {
            return Ok(Response::Error(ErrorResponse::from_message(
                &format!("Unexpected error: {}: {}", e.kind.ruby_class(), e.message),
                1,
            )))
        }
    };
    let response =
        SuccessResponse::from_result(&parts, arguments, tool.command_definition(command), None)
            .map_err(|e| ExecuteError::unsupported(format!("response building: {}", e.message)))?;
    Ok(Response::Success(response))
}

/// The `rescue` clauses of `execute_request` for a tool failure.
fn tool_error_response(e: ToolError) -> Result<ErrorResponse, ExecuteError> {
    let message = match e {
        ToolError::NotFound { .. } => format!("Ukiryu::Tool not found: {e}"),
        ToolError::ProfileNotFound { message } => format!("Profile not found: {message}"),
        ToolError::Argument { message } => format!("Invalid arguments: {message}"),
        ToolError::ExecutableNotFound { .. } => {
            format!("Unexpected error: Ukiryu::Errors::ExecutableNotFoundError: {e}")
        }
        ToolError::Build(b) => match b.class {
            ErrorClass::Unsupported => {
                return Err(ExecuteError::unsupported(format!(
                    "argument building: {}",
                    b.message
                )))
            }
            ErrorClass::Argument => format!("Invalid arguments: {}", b.message),
            class => format!("Unexpected error: {}: {}", class.ruby_name(), b.message),
        },
        ToolError::Ruby { class, message } => match class {
            "ArgumentError" => format!("Invalid arguments: {message}"),
            class => format!("Unexpected error: {class}: {message}"),
        },
        ToolError::Source { message } => {
            return Err(ExecuteError::unsupported(format!(
                "definition source failure: {message}"
            )))
        }
        ToolError::Execute(x) => match *x {
            ProcessError::Failed { error, .. } => error.message().to_owned(),
            ProcessError::TimedOut(error) => format!("Command timed out: {}", error.message()),
            ProcessError::InvalidByteSequence { error, .. } => invalid_bytes_message(&error),
            ProcessError::ShellNotFound { message, .. } => {
                format!("Unexpected error: RuntimeError: {message}")
            }
            other => {
                return Err(ExecuteError::unsupported(format!(
                    "executor failure without a Ruby counterpart: {other}"
                )))
            }
        },
    };
    Ok(ErrorResponse::from_message(&message, 1))
}

/// A stream that is not valid UTF-8 where the source reads it as text: its
/// `ArgumentError` reaches the `ArgumentError` rescue, any other class the
/// `StandardError` one.
fn invalid_bytes_message(e: &InvalidByteSequence) -> String {
    match e.ruby_class() {
        "ArgumentError" => format!("Invalid arguments: {}", e.message()),
        class => format!("Unexpected error: {class}: {}", e.message()),
    }
}

/// `output_response(response, format, output_file, config)`; the command
/// then returns normally (exit status 0).
pub fn render(
    response: &Response,
    format: OutputFormat,
    output_file: Option<&str>,
    settings: &RenderSettings,
) -> Result<CommandOutcome, ExecuteError> {
    let writes = match response {
        Response::Success(r) => output_response(r, format, output_file, settings)?,
        Response::Error(r) => output_response(r, format, output_file, settings)?,
    };
    Ok(CommandOutcome {
        writes,
        exit_code: 0,
    })
}

/// The execution settings `execute_request` passes for this invocation:
/// `config.timeout || 90` and an explicit (empty) environment applied over
/// the inherited one (D1).
pub fn exec_settings(
    input: &ExecInput,
    ctx: &ExecuteContext,
) -> Result<ExecuteSettings, ExecuteError> {
    let config = configuration(
        &ctx.host,
        input.format.as_deref(),
        input.output.as_deref(),
        input.register.as_deref(),
        input.shell.as_deref(),
        Some(input.dry_run),
    )?;
    Ok(ExecuteSettings::new(config_timeout(&config)))
}
