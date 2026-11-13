//! `ukiryu version`, `ukiryu system`, `ukiryu config`, `list`, `commands`, `register`
//! and `resolve`; `info`, `opts` and `which` are gated.
//!
//! Source: ukiryu `lib/ukiryu/cli_commands/{base,version,system,config,list,commands,register,resolve}_command.rb`
//! at ab6df353c6dc2d7faf7817a86aafcafa82579896. These verbs print with
//! `BaseCommand#say`, not through `ResponseFormatter`, so each handler returns
//! the ordered `say` lines as [`OutputWrite`]s.
//!
//! Every handler reads an explicit [`HandlerContext`] (host platform and
//! environment snapshot plus the `--register` option); nothing reads or
//! mutates process state except the persistent configuration file, which
//! `config` reads under `$HOME/.ukiryu` and replaces through an
//! [`OutputWrite::File`], and the register: `register update` and a missing
//! register run `git` and change the register directory, as the source does.
//!
//! `Thor::Error` (the source's `error!`) is rendered as `FriendlyCLI.start`
//! does: the message on stderr and exit status 1. Thor colours that message
//! red only when stderr is a terminal; the handlers write it uncoloured, the
//! form the captured (non-terminal) goldens show.
//!
//! The register-backed verbs (`config list`, `list`, `commands`, `register`,
//! `resolve`) run on the real register and tool APIs: [`Register`],
//! [`RegisterSource`] with [`tool::get`], [`Discovery`] and [`VersionResolver`].
//!
//! Not covered here:
//! - `info`, `opts` and `which` are gated ([`info`], [`opts`], [`which`] return
//!   [`InspectError::Gated`]), and so is the interface path of `list` (a tool
//!   whose own version file is missing). They need core APIs that do not
//!   exist yet: [`FIND_BY_NEEDS`].
//! - `config get|set|unset` run `setup_register` first in the source, which
//!   can clone the default register into `~/.ukiryu/register`. That bootstrap
//!   does not change their output and is not performed here; `config list`,
//!   `list` and `commands` do run it (`setup_register`), including the
//!   clone when no register exists.
//! - The development register (`<gem root>/register`) has no counterpart: the
//!   register search is `UKIRYU_REGISTER`, then `~/.ukiryu/register`.
//! - Rendering a time (`register info`'s last update, `resolve`'s mtimes) needs
//!   the local zone; only a `TZ` naming UTC is rendered, anything else is gated.
//! - `list` needs `Tool#version`; a tool whose definition has no version would
//!   need version detection, which has no man-page date provider yet (gated).
//! - Shell detection follows the strict migrated contract (DECISIONS.md D2):
//!   a missing or unknown `SHELL`, or a Windows host naming no shell, is an
//!   error where the source falls back to `/etc/shells`/`sh`/`cmd`.

use super::CommandOutcome;
use serde_json::Value;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use ukiryu::cache::ToolCache;
use ukiryu::config::{self, ConversionError, OverrideResolver, Values};
use ukiryu::definition::{DefinitionMetadata, Discovery, DiscoveryEnv, VersionResolver};
use ukiryu::environment::Environment;
use ukiryu::executable::{find_in_path, DiscoveryContext, Platform};
use ukiryu::extractors::base::ProcessHost;
use ukiryu::models::{CommandDefinition, ModelError, Text, ToolMetadata, Value as ModelValue};
use ukiryu::register::{self, Git, Register};
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{Clock, Host, HostError, Runtime, ShellError};
use ukiryu::tool::{self, Tool, ToolError, ToolOptions};
use ukiryu::tool_loader::RegisterSource;

/// The process facts a handler reads, captured once by the caller.
#[derive(Clone, Debug)]
pub struct HandlerContext {
    /// `Platform.detect` and a snapshot of `ENV`.
    pub host: Host,
    /// The `--register` / `-r` option, when given. `BaseCommand` applies it as
    /// the CLI configuration layer when it is not empty.
    pub register: Option<String>,
}

impl HandlerContext {
    /// The native host (compiled-for platform, process environment).
    pub fn native(register: Option<String>) -> Result<HandlerContext, HostError> {
        Ok(HandlerContext {
            host: Host::native()?,
            register,
        })
    }

    /// `Ukiryu::Config.instance` with `apply_cli_options_to_config`.
    fn configuration(&self) -> Result<OverrideResolver, InspectError> {
        let environment: config::Environment = self
            .host
            .env
            .iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let env = config::environment_values(&environment).map_err(InspectError::Config)?;
        let mut cli = Values::new();
        if let Some(register) = self.register.as_deref().filter(|r| !r.is_empty()) {
            cli.insert("register".to_owned(), Value::String(register.to_owned()));
        }
        Ok(OverrideResolver {
            defaults: config::defaults(),
            programmatic: Values::new(),
            env,
            cli,
        })
    }

    /// `ENV[name]`, ASCII case-insensitive on Windows as Ruby's Windows `ENV` is.
    fn env(&self, name: &str) -> Option<&str> {
        if self.host.platform == Platform::Windows {
            self.host
                .env
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        } else {
            self.host.env.get(name)
        }
    }
}

/// Why a handler could not produce an outcome. These are the failures the
/// source raises as uncaught exceptions (not `Thor::Error`), so the binary
/// decides how to present them; `partial` holds the writes made before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectError {
    /// A `UKIRYU_*` variable failed `TypeConverter` (raised building `Config`).
    Config(ConversionError),
    /// `Runtime.instance.shell` failed after `system` printed `partial`.
    Shell {
        partial: Vec<OutputWrite>,
        error: ShellError,
    },
    /// `File.expand_path('~')` had no home directory.
    Home,
    /// The persistent configuration file holds a YAML document that is not a
    /// mapping, so the source's `persistent[key] = value` / `key?` raises.
    PersistentNotMapping { path: String },
    /// The persistent configuration could not be written as YAML.
    Emit(String),
    /// An exception the source raises uncaught (`[Class] message`) after
    /// `partial` was printed.
    Raised {
        class: &'static str,
        message: String,
        partial: Vec<OutputWrite>,
    },
    /// The tool definition source failed in a way the port reports as such
    /// (no Ruby counterpart), after `partial` was printed.
    Source {
        message: String,
        partial: Vec<OutputWrite>,
    },
    /// A path whose component does not exist yet. Never a successful stub:
    /// `needs` names the missing API.
    Gated {
        what: String,
        needs: String,
        partial: Vec<OutputWrite>,
    },
}

impl fmt::Display for InspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InspectError::Config(e) => e.fmt(f),
            InspectError::Shell { error, .. } => error.fmt(f),
            InspectError::Home => f.write_str("couldn't find HOME environment -- expanding '~'"),
            InspectError::PersistentNotMapping { path } => {
                write!(f, "persistent configuration is not a mapping: {path}")
            }
            InspectError::Emit(e) => f.write_str(e),
            InspectError::Raised { message, .. } | InspectError::Source { message, .. } => {
                f.write_str(message)
            }
            InspectError::Gated { what, needs, .. } => write!(
                f,
                "ukiryu: '{what}' is not available in this build yet; it needs {needs}"
            ),
        }
    }
}

impl InspectError {
    /// The stderr line `FriendlyCLI.start` prints for an exception that is
    /// not a `Thor::Error` (`[Class] message`, then exit status 1), where the
    /// source raises one. A strict-shell failure (D2) has no source
    /// counterpart and is labelled with the error class the port maps it to.
    pub fn friendly_message(&self) -> String {
        match self {
            InspectError::Config(_) | InspectError::Home => format!("[ArgumentError] {self}"),
            InspectError::Shell { .. } => format!("[Ukiryu::Errors::UnknownShellError] {self}"),
            InspectError::PersistentNotMapping { .. }
            | InspectError::Emit(_)
            | InspectError::Source { .. }
            | InspectError::Gated { .. } => self.to_string(),
            InspectError::Raised { class, message, .. } => format!("[{class}] {message}"),
        }
    }

    /// The writes made before the failure, which the caller performs first.
    pub fn partial(&self) -> &[OutputWrite] {
        match self {
            InspectError::Shell { partial, .. }
            | InspectError::Raised { partial, .. }
            | InspectError::Source { partial, .. }
            | InspectError::Gated { partial, .. } => partial,
            _ => &[],
        }
    }
}

impl std::error::Error for InspectError {}

/// `Config#colors_disabled?`: only an explicit `use_color: false`
/// (`NO_COLOR` sets it).
fn colors_disabled(config: &OverrideResolver) -> bool {
    config.resolve("use_color") == Some(&Value::Bool(false))
}

/// The colours `BaseCommand#say` maps that these commands use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    Red,
    Green,
    Yellow,
    Cyan,
    White,
    Dim,
    /// `say '', :clear`: `:clear` is not in the source's colour table, so it
    /// falls back to white.
    Clear,
}

impl Color {
    fn code(self) -> u8 {
        match self {
            Color::Red => 31,
            Color::Green => 32,
            Color::Yellow => 33,
            Color::Cyan => 36,
            Color::White | Color::Clear => 37,
            Color::Dim => 2,
        }
    }
}

/// Ordered `say` / `error!` writes of one command.
struct Out {
    writes: Vec<OutputWrite>,
    colors_disabled: bool,
}

impl Out {
    fn new(colors_disabled: bool) -> Out {
        Out {
            writes: Vec::new(),
            colors_disabled,
        }
    }

    /// `BaseCommand#say(message, color)`: ANSI-wrapped unless colours are
    /// disabled, then `puts`.
    fn say(&mut self, message: &str, color: Option<Color>) {
        let mut text = match color {
            Some(c) if !self.colors_disabled => format!("\x1b[{}m{message}\x1b[0m", c.code()),
            _ => message.to_owned(),
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.writes.push(OutputWrite::Stdout(text));
    }

    fn done(self) -> CommandOutcome {
        CommandOutcome {
            writes: self.writes,
            exit_code: 0,
        }
    }

    /// `exit n` after the writes so far.
    fn exit(self, code: i32) -> CommandOutcome {
        CommandOutcome {
            writes: self.writes,
            exit_code: code,
        }
    }

    /// `say '', :clear`.
    fn clear(&mut self) {
        self.say("", Some(Color::Clear));
    }

    /// `error!` → `Thor::Error`, which `FriendlyCLI.start` prints to stderr
    /// before `exit(false)`.
    fn error(mut self, message: &str) -> CommandOutcome {
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
}

// ---------------------------------------------------------------- version

/// `ukiryu version` takes no arguments or options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionInput;

/// `VersionCommand#run`: `Ukiryu version <VERSION>` in cyan.
pub fn version(_input: VersionInput, ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let mut out = Out::new(colors_disabled(&config));
    out.say(
        &format!("Ukiryu version {}", ukiryu::VERSION),
        Some(Color::Cyan),
    );
    Ok(out.done())
}

// ----------------------------------------------------------------- system

/// `ukiryu system [SUBCOMMAND]` (`--register` is in [`HandlerContext`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemInput {
    pub subcommand: Option<String>,
}

/// `Shell::VALID_SHELLS`: the platform groups, then the individual shells
/// (`powershell` is in both).
const VALID_SHELLS: [&str; 11] = [
    "unix",
    "windows",
    "powershell",
    "bash",
    "zsh",
    "fish",
    "sh",
    "dash",
    "tcsh",
    "powershell",
    "cmd",
];

/// `Shell::PLATFORM_TO_SHELLS[:unix]`.
const UNIX_SHELLS: [&str; 6] = ["bash", "zsh", "fish", "sh", "dash", "tcsh"];

/// `SystemCommand#run`.
pub fn system(input: SystemInput, ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let out = Out::new(colors_disabled(&config));
    match input.subcommand.as_deref() {
        None | Some("shells") => list_shells(out, ctx, &config),
        Some(other) => Ok(out.error(&format!(
            "Unknown subcommand: {other}. Valid subcommands: shells"
        ))),
    }
}

fn list_shells(
    mut out: Out,
    ctx: &HandlerContext,
    config: &OverrideResolver,
) -> Result<CommandOutcome, InspectError> {
    let platform = ctx.host.platform;
    let platform_shells: &[&str] = if platform == Platform::Windows {
        &["windows", "powershell", "unix"]
    } else {
        &["unix", "powershell"]
    };
    let (available, not_installed): (Vec<&str>, Vec<&str>) = platform_shells
        .iter()
        .copied()
        .partition(|shell| shell_available(ctx, shell));
    let not_supported: Vec<&str> = VALID_SHELLS
        .into_iter()
        .filter(|shell| !platform_shells.contains(shell))
        .collect();

    out.say("Available shells", Some(Color::Cyan));
    out.say("", None);
    out.say(
        "  The following shells are installed and supported on this platform:",
        None,
    );
    out.say("", None);
    if available.is_empty() {
        out.say("    No supported shells detected", Some(Color::Dim));
    } else {
        for shell in &available {
            out.say(&format!("    • {}", describe(shell)), Some(Color::Green));
        }
    }
    if !not_installed.is_empty() {
        out.say("", None);
        out.say("Additional supported shells", Some(Color::Cyan));
        out.say("", None);
        out.say(
            "  These shells are supported but not currently installed:",
            None,
        );
        out.say("", None);
        for shell in &not_installed {
            out.say(&format!("    • {}", describe(shell)), Some(Color::Dim));
        }
    }
    if !not_supported.is_empty() {
        out.say("", None);
        out.say("Not available on this platform", Some(Color::Cyan));
        out.say("", None);
        out.say("  These shells are not supported on this platform:", None);
        out.say("", None);
        for shell in &not_supported {
            out.say(&format!("    • {}", describe(shell)), Some(Color::Dim));
        }
    }
    out.say("", None);
    out.say(
        &format!("Platform: {}", platform.as_str()),
        Some(Color::Dim),
    );

    let configured_shell = config.resolve("shell").and_then(text);
    let runtime = Runtime::new(ctx.host.clone(), configured_shell.clone(), Clock::default());
    let shell = match runtime.shell() {
        Ok(shell) => shell,
        Err(error) => {
            return Err(InspectError::Shell {
                partial: out.writes,
                error,
            })
        }
    };
    out.say(&format!("Current shell: {shell}"), Some(Color::Dim));
    if let Some(shell) = configured_shell {
        out.say(
            &format!("Shell override: {shell} (set via --shell, UKIRYU_SHELL, or config)"),
            Some(Color::Yellow),
        );
    }
    Ok(out.done())
}

/// `Shell.available?` for the platform groups `system` lists.
///
/// The source shells out to `which NAME` for each Unix name and `pwsh`; the
/// port searches the snapshot's `PATH` with the executable locator, so it
/// differs only when `which` itself is missing or behaves unusually.
fn shell_available(ctx: &HandlerContext, group: &str) -> bool {
    let platform = ctx.host.platform;
    let on_path = |name: &str| {
        let context = DiscoveryContext::new(platform, "", ctx.env("PATH"), ctx.env("PATHEXT"));
        find_in_path(&context, name, &[]).is_some()
    };
    match group {
        "unix" => platform != Platform::Windows && UNIX_SHELLS.iter().any(|s| on_path(s)),
        "powershell" => {
            if platform == Platform::Windows {
                ctx.env("PSModulePath").is_some()
            } else {
                on_path("pwsh")
            }
        }
        "windows" => platform == Platform::Windows,
        _ => false,
    }
}

/// `SystemCommand#shell_name_with_description`.
fn describe(shell: &str) -> String {
    let description = match shell {
        "bash" => "GNU Bourne Again SHell",
        "zsh" => "Z shell",
        "fish" => "Friendly interactive shell",
        "sh" => "POSIX shell",
        "powershell" => "PowerShell command-line shell",
        "cmd" => "Windows Command Prompt",
        _ => return shell.to_owned(),
    };
    format!("{shell} - {description}")
}

// ----------------------------------------------------------------- config

/// The `config` actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigAction {
    /// `list`, also the default when no action is given.
    List,
    Get {
        key: Option<String>,
    },
    Set {
        key: Option<String>,
        value: Option<String>,
    },
    Unset {
        key: Option<String>,
    },
    /// Any other action word: the source's "Unknown action" error.
    Unknown {
        action: String,
    },
}

/// `ukiryu config [ACTION] [KEY] [VALUE]` (`--register` is in [`HandlerContext`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigInput {
    pub action: ConfigAction,
}

impl ConfigInput {
    /// Map the parser's three positionals. Every action has a handler, so the
    /// result is always `Some`; the `Option` is the signature the dispatcher
    /// was written against.
    pub fn from_arguments(
        action: Option<&str>,
        key: Option<&str>,
        value: Option<&str>,
    ) -> Option<ConfigInput> {
        let key = key.map(str::to_owned);
        let action = match action.unwrap_or("list") {
            "list" => ConfigAction::List,
            "get" => ConfigAction::Get { key },
            "set" => ConfigAction::Set {
                key,
                value: value.map(str::to_owned),
            },
            "unset" => ConfigAction::Unset { key },
            other => ConfigAction::Unknown {
                action: other.to_owned(),
            },
        };
        Some(ConfigInput { action })
    }
}

const CONFIG_KEYS: [&str; 9] = [
    "register",
    "timeout",
    "debug",
    "dry_run",
    "metrics",
    "shell",
    "format",
    "output",
    "use_color",
];

/// `ConfigCommand#run`.
pub fn config(input: ConfigInput, ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    // CONFIG_FILE is a class constant: loading the command expands `~`.
    let file = config_file(ctx)?;
    let mut out = Out::new(colors_disabled(&config));
    match input.action {
        ConfigAction::List => config_list(out, ctx, &config, &file),
        ConfigAction::Get { key } => {
            let key = key.unwrap_or_default();
            let normalized = key.to_lowercase();
            if !CONFIG_KEYS.contains(&normalized.as_str()) {
                return Ok(out.error(&format!(
                    "Unknown config key: {key}\nValid keys: {}",
                    CONFIG_KEYS.join(", ")
                )));
            }
            let value = config
                .resolve(&normalized)
                .and_then(display)
                .unwrap_or_else(|| "(not set)".to_owned());
            out.say(&format!("{key}: {value}"), Some(Color::White));
            Ok(out.done())
        }
        ConfigAction::Set {
            key: Some(key),
            value: Some(value),
        } => {
            let mut persistent = match load_persistent(&file, &mut out) {
                Some(yeptris::Value::Mapping(map)) => map,
                None | Some(yeptris::Value::Null) | Some(yeptris::Value::Bool(false)) => {
                    yeptris::Mapping::new()
                }
                Some(_) => return Err(InspectError::PersistentNotMapping { path: file.path }),
            };
            persistent.insert(
                yeptris::Value::String(key.clone()),
                yeptris::Value::String(value.clone()),
            );
            ensure_dir(&file.dir);
            out.writes.push(OutputWrite::File {
                path: file.path.clone(),
                contents: dump(persistent)?,
            });
            out.say(&format!("Set {key} = {value}"), Some(Color::Green));
            out.say(&format!("Saved to: {}", file.path), Some(Color::Dim));
            Ok(out.done())
        }
        ConfigAction::Set { .. } => Ok(out.error("Usage: ukiryu config set <key> <value>")),
        ConfigAction::Unset { key: None } => Ok(out.error("Usage: ukiryu config unset <key>")),
        ConfigAction::Unset { key: Some(key) } => {
            let mut persistent = match load_persistent(&file, &mut out) {
                Some(yeptris::Value::Mapping(map)) => map,
                None | Some(yeptris::Value::Null) | Some(yeptris::Value::Bool(false)) => {
                    return Ok(out.error(&format!("Key not set in persistent config: {key}")))
                }
                Some(_) => return Err(InspectError::PersistentNotMapping { path: file.path }),
            };
            let wanted = yeptris::Value::String(key.clone());
            if !persistent.0.iter().any(|(k, _)| *k == wanted) {
                return Ok(out.error(&format!("Key not set in persistent config: {key}")));
            }
            persistent.0.retain(|(k, _)| *k != wanted);
            out.writes.push(OutputWrite::File {
                path: file.path.clone(),
                contents: dump(persistent)?,
            });
            out.say(&format!("Unset {key}"), Some(Color::Green));
            out.say(&format!("Updated: {}", file.path), Some(Color::Dim));
            Ok(out.done())
        }
        ConfigAction::Unknown { action } => Ok(out.error(&format!(
            "Unknown action: {action}\nValid actions: list, get, set, unset"
        ))),
    }
}

struct ConfigFile {
    dir: String,
    path: String,
}

/// `File.expand_path('~/.ukiryu')` and `config.yml` inside it. Ruby takes
/// `HOME`; on Windows it falls back to `HOMEDRIVE`+`HOMEPATH`, then
/// `USERPROFILE`, and writes `/` separators.
fn config_file(ctx: &HandlerContext) -> Result<ConfigFile, InspectError> {
    let windows = ctx.host.platform == Platform::Windows;
    let home = match ctx.env("HOME") {
        Some(home) => home.to_owned(),
        None if windows => match (ctx.env("HOMEDRIVE"), ctx.env("HOMEPATH")) {
            (Some(drive), Some(path)) => format!("{drive}{path}"),
            _ => ctx.env("USERPROFILE").ok_or(InspectError::Home)?.to_owned(),
        },
        None => return Err(InspectError::Home),
    };
    let home = if windows {
        home.replace('\\', "/")
    } else {
        home
    };
    let home = match home.trim_end_matches('/') {
        "" => "/".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    let dir = if home == "/" {
        "/.ukiryu".to_owned()
    } else {
        format!("{home}/.ukiryu")
    };
    let path = format!("{dir}/config.yml");
    Ok(ConfigFile { dir, path })
}

/// `load_persistent_config`: `nil` without a file; a load failure is
/// reported in red and treated as `nil`. The warning carries the YAML
/// engine's message, not Psych's.
fn load_persistent(file: &ConfigFile, out: &mut Out) -> Option<yeptris::Value> {
    if std::fs::metadata(&file.path).is_err() {
        return None;
    }
    let loaded = std::fs::read(&file.path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| yeptris::parse_stream(&bytes).map_err(|e| e.to_string()));
    match loaded {
        // An empty document is `false` from `YAML.load_file`.
        Ok(documents) => Some(
            documents
                .into_iter()
                .next()
                .unwrap_or(yeptris::Value::Bool(false)),
        ),
        Err(message) => {
            out.say(
                &format!("Warning: Failed to load config file: {message}"),
                Some(Color::Red),
            );
            None
        }
    }
}

/// `FileUtils.mkdir_p(CONFIG_DIR)`. A failure surfaces when the file write
/// is applied, as `File.write` would raise in the source.
fn ensure_dir(dir: &str) {
    let _ = std::fs::create_dir_all(dir);
}

/// `data.to_yaml` through the pinned YAML emitter settings.
fn dump(map: yeptris::Mapping) -> Result<String, InspectError> {
    let mut text = yeptris::to_string_with(
        &yeptris::Value::Mapping(map),
        &ukiryu::render::yaml_emit_options(),
    )
    .map_err(|e| InspectError::Emit(e.to_string()))?;
    text.push('\n');
    Ok(text)
}

/// A configured value as text (`Config.shell` is a symbol or `nil`).
fn text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Ruby `"#{value}"` of a resolved configuration value; `nil` is `None`.
fn display(value: &Value) -> Option<String> {
    text(value)
}

// ------------------------------------------------------- register access

/// The register `BaseCommand#setup_register` leaves as `Register.default`.
struct DefaultRegister {
    register: Register,
    /// `ENV['UKIRYU_REGISTER']` when `setup_register` set it (which makes the
    /// default register's source `env`).
    env_register: Option<String>,
}

enum RegisterFailure {
    Home,
    Register(register::Error),
}

impl RegisterFailure {
    fn into_error(self, partial: Vec<OutputWrite>) -> InspectError {
        match self {
            RegisterFailure::Home => InspectError::Home,
            RegisterFailure::Register(error) => raised_register(error, partial),
        }
    }
}

fn raised_register(error: register::Error, partial: Vec<OutputWrite>) -> InspectError {
    InspectError::Raised {
        class: error.class_name(),
        message: error.to_string(),
        partial,
    }
}

/// `Register::Error` and its subclasses, which `register` and
/// `format_register_display` rescue (the others are not rescued).
fn is_register_error(error: &register::Error) -> bool {
    matches!(
        error,
        register::Error::NotFound(_) | register::Error::Clone(_) | register::Error::Update(_)
    )
}

/// The inputs `Register.default` discovers from.
fn register_discovery(ctx: &HandlerContext) -> register::DiscoveryContext {
    let home = ctx
        .env("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| {
            ctx.env("USERPROFILE")
                .filter(|h| !h.is_empty() && ctx.host.platform == Platform::Windows)
        })
        .map(PathBuf::from);
    register::DiscoveryContext {
        env_register: ctx.env(register::ENV_VAR).map(OsString::from),
        dev_path: None,
        home,
    }
}

/// `Register.default`: discover the register, cloning it when it is missing.
fn default_register(ctx: &HandlerContext) -> Result<Register, RegisterFailure> {
    let discovery = register_discovery(ctx);
    if discovery.discover_path().is_none() {
        return Err(RegisterFailure::Home);
    }
    discovery
        .discover(&Git::default())
        .map_err(RegisterFailure::Register)
}

/// `BaseCommand#setup_register` and then `Register.default`.
///
/// `config.register` (the `--register` option or `UKIRYU_REGISTER`) wins when
/// it is an existing directory; the source then exports it as
/// `UKIRYU_REGISTER` and resets the default register, so the register is
/// reported with source `env`. Otherwise the default register is discovered
/// and, when it is a directory, exported the same way.
fn setup_register(
    ctx: &HandlerContext,
    config: &OverrideResolver,
) -> Result<DefaultRegister, RegisterFailure> {
    let configured = config
        .resolve("register")
        .and_then(text)
        .filter(|p| !p.is_empty());
    if let Some(path) = configured.as_deref() {
        if Path::new(path).is_dir() {
            let register = Register::new(path, register::Source::Env);
            register
                .ensure_exists(&Git::default())
                .map_err(RegisterFailure::Register)?;
            return Ok(DefaultRegister {
                register,
                env_register: Some(path.to_owned()),
            });
        }
    }
    let register = default_register(ctx)?;
    if configured.is_some() || !register.path().is_dir() {
        return Ok(DefaultRegister {
            register,
            env_register: None,
        });
    }
    let path = register.path().to_string_lossy().into_owned();
    Ok(DefaultRegister {
        register: Register::new(path.as_str(), register::Source::Env),
        env_register: Some(path),
    })
}

/// `Register.load_tool_metadata(name)` for a name the register lists: the
/// tool's own version file. `None` is the source's interface fallback (the
/// version file is missing or empty), which needs [`FIND_BY_NEEDS`]. The error
/// is the Ruby class and message.
fn tool_metadata(
    register: &Register,
    name: &str,
) -> Result<Option<ToolMetadata>, (&'static str, String)> {
    let yaml = match register.load_tool_yaml(name, None) {
        Ok(Some(yaml)) => yaml,
        Ok(None) => return Ok(None),
        Err(error) => return Err((error.class_name(), error.to_string())),
    };
    let tree: yeptris::Value =
        ukiryu::from_yaml(&yaml).map_err(|e| ("Psych::SyntaxError", e.to_string()))?;
    if matches!(tree, yeptris::Value::Null | yeptris::Value::Bool(false)) {
        return Ok(None);
    }
    let model_error = |e: ModelError| (e.ruby_class(), e.message);
    let hash = ModelValue::from_yaml_value(&tree).map_err(model_error)?;
    ToolMetadata::from_hash(&hash, name, register.path().to_str())
        .map(Some)
        .map_err(model_error)
}

/// The core APIs `info`, `opts`, `which` and the interface path of `list`
/// need and that do not exist in the workspace.
pub const FIND_BY_NEEDS: &str = "core APIs that do not exist yet: the register scanner \
     (routing::RegisterTool values built from a Register's tools directory for routing::ToolIndex), \
     Tool.find_by (ToolFinder.find_by: interface and alias lookup), \
     the Register.load_tool_metadata interface fallback, \
     and ToolDefinition#compatible_profile with Tools::Generator.load_tool_definition (used by which)";

fn gated(what: &str, needs: &str, partial: Vec<OutputWrite>) -> InspectError {
    InspectError::Gated {
        what: what.to_owned(),
        needs: needs.to_owned(),
        partial,
    }
}

/// The runtime's platform and shell, with executable discovery over the
/// snapshot's `PATH`. The shell can fail under the strict contract (D2).
fn tool_options(
    ctx: &HandlerContext,
    config: &OverrideResolver,
) -> Result<ToolOptions, ShellError> {
    let configured = config.resolve("shell").and_then(text);
    let runtime = Runtime::new(ctx.host.clone(), configured, Clock::default());
    let options = ToolOptions::from_runtime(&runtime)?;
    let discovery = DiscoveryContext::new(
        options.platform,
        options.shell.as_str(),
        ctx.env("PATH"),
        ctx.env("PATHEXT"),
    );
    Ok(options.discovery(discovery))
}

/// `Tool.get(name)` over the register.
fn get_tool(
    ctx: &HandlerContext,
    register: &Register,
    cache: &ToolCache<Rc<Tool>>,
    options: &ToolOptions,
    name: &str,
) -> Result<Rc<Tool>, ToolError> {
    let source = RegisterSource::new(
        register.clone(),
        ctx.host.platform,
        ProcessHost::new(Environment::new(), options.shell),
    );
    tool::get(&source, cache, name, options)
}

/// A tool failure the source does not rescue.
fn tool_failure(error: &ToolError, partial: Vec<OutputWrite>) -> InspectError {
    let class: Option<&'static str> = match error {
        ToolError::Argument { .. } => Some("ArgumentError"),
        ToolError::Ruby { class, .. } => Some(*class),
        ToolError::Source { .. } => None,
        other => other.kind().map(|kind| kind.ruby_class()),
    };
    match class {
        Some(class) => InspectError::Raised {
            class,
            message: error.to_string(),
            partial,
        },
        None => InspectError::Source {
            message: error.to_string(),
            partial,
        },
    }
}

/// A `Text` attribute as the String Ruby reads (`nil` is `None`).
fn txt(value: &Option<Text>) -> Option<String> {
    match value {
        None | Some(Text::Nil) => None,
        Some(other) => Some(other.to_ruby_s()),
    }
}

// ------------------------------------------------------------ config list

/// `ConfigCommand#action_list`.
fn config_list(
    mut out: Out,
    ctx: &HandlerContext,
    config: &OverrideResolver,
    file: &ConfigFile,
) -> Result<CommandOutcome, InspectError> {
    // `run` starts with `setup_register`, which rescues `Register::Error`.
    let setup = match setup_register(ctx, config) {
        Ok(setup) => Some(setup),
        Err(RegisterFailure::Register(error)) if is_register_error(&error) => None,
        Err(failure) => return Err(failure.into_error(Vec::new())),
    };
    out.say("Current configuration:", Some(Color::Cyan));
    out.clear();

    let register_text = match &setup {
        Some(setup) => register_display(&setup.register),
        None => NOT_FOUND_DISPLAY.to_owned(),
    };
    let shown_value = |key: &str, default: &str| -> String {
        match config.resolve(key) {
            None | Some(Value::Null) => format!("'{default}'"),
            // `shell` and `format` are Symbols, which are not quoted.
            Some(Value::String(s)) if key == "shell" || key == "format" => s.clone(),
            Some(Value::String(s)) => format!("'{s}'"),
            Some(other) => other.to_string(),
        }
    };
    let rows = [
        ("Register", format!("'{register_text}'")),
        ("Timeout", shown_value("timeout", "(no timeout)")),
        ("Debug", shown_value("debug", "(nil)")),
        ("Dry run", shown_value("dry_run", "(nil)")),
        ("Metrics", shown_value("metrics", "(nil)")),
        ("Shell", shown_value("shell", "(auto-detect)")),
        ("Format", shown_value("format", "(nil)")),
        ("Output", shown_value("output", "(stdout)")),
        ("Use color", shown_value("use_color", "(nil)")),
    ];
    for (key, shown) in rows {
        out.say(&format!("  {key:<20}: {shown}"), Some(Color::White));
    }

    out.clear();
    if std::fs::metadata(&file.path).is_ok() {
        out.say("Persistent configuration:", Some(Color::Cyan));
        out.say(&format!("  File: {}", file.path), Some(Color::Dim));
        match load_persistent(file, &mut out) {
            Some(yeptris::Value::Mapping(map)) if !map.0.is_empty() => {
                for (key, value) in &map.0 {
                    let (Some(key), Some(value)) = (ruby_to_s(key), ruby_to_s(value)) else {
                        return Err(gated(
                            "config list (nested persistent value)",
                            "Ruby inspect formatting of nested YAML values",
                            out.writes,
                        ));
                    };
                    out.say(&format!("  {key}: {value}"), Some(Color::Dim));
                }
            }
            None | Some(yeptris::Value::Null) | Some(yeptris::Value::Bool(false)) => {
                out.say("  (empty)", Some(Color::Dim))
            }
            Some(yeptris::Value::Mapping(_)) => out.say("  (empty)", Some(Color::Dim)),
            Some(_) => {
                return Err(InspectError::PersistentNotMapping {
                    path: file.path.clone(),
                })
            }
        }
    } else {
        out.say("No persistent configuration file found.", Some(Color::Dim));
        out.say(
            &format!("  Config would be saved to: {}", file.path),
            Some(Color::Dim),
        );
    }

    out.clear();
    out.say("Environment variables:", Some(Color::Cyan));
    let mut any = false;
    for name in [
        "UKIRYU_REGISTER",
        "UKIRYU_TIMEOUT",
        "UKIRYU_DEBUG",
        "UKIRYU_DRY_RUN",
        "UKIRYU_METRICS",
        "UKIRYU_SHELL",
        "UKIRYU_FORMAT",
        "UKIRYU_OUTPUT",
        "UKIRYU_USE_COLOR",
    ] {
        let exported = setup
            .as_ref()
            .and_then(|s| s.env_register.as_deref())
            .filter(|_| name == "UKIRYU_REGISTER");
        if let Some(value) = exported.or_else(|| ctx.env(name)) {
            any = true;
            out.say(&format!("  {name:<25}: {value}"), Some(Color::White));
        }
    }
    if !any {
        out.say("  (none set)", Some(Color::Dim));
    }
    Ok(out.done())
}

const NOT_FOUND_DISPLAY: &str = "~/.ukiryu/register (not found - run: ukiryu register update)";

/// `format_register_display` for a register `Register.default` returned.
fn register_display(register: &Register) -> String {
    let info = register.info(&Git::default());
    if info.valid {
        let label = match info.source {
            register::Source::Env => "env",
            register::Source::User => "user",
            other => other.as_str(),
        };
        format!(
            "{} [{label}] ({} tools)",
            info.path.display(),
            info.tools_count
        )
    } else if info.exists {
        format!(
            "{} (invalid - run: ukiryu register update --force)",
            info.path.display()
        )
    } else {
        NOT_FOUND_DISPLAY.to_owned()
    }
}

/// Ruby `to_s` of a scalar YAML value; `None` for a collection (its
/// `inspect` form is not produced here).
fn ruby_to_s(value: &yeptris::Value) -> Option<String> {
    Some(match value {
        yeptris::Value::Null => String::new(),
        yeptris::Value::Bool(b) => b.to_string(),
        yeptris::Value::String(s) => s.clone(),
        yeptris::Value::Number(yeptris::Number::I64(n)) => n.to_string(),
        yeptris::Value::Number(yeptris::Number::U64(n)) => n.to_string(),
        yeptris::Value::Number(yeptris::Number::F64(n)) => register::ruby_float(*n),
        _ => return None,
    })
}

// ------------------------------------------------------------------- list

/// `ukiryu list` (`--register` is in [`HandlerContext`]).
pub fn list(ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let mut out = Out::new(colors_disabled(&config));
    let register = setup_register(ctx, &config)
        .map_err(|f| f.into_error(Vec::new()))?
        .register;
    let tools = register.tool_names();
    if tools.is_empty() {
        return Ok(out.error("No tools found in register"));
    }
    out.say(
        &format!("Available tools ({}):", tools.len()),
        Some(Color::Cyan),
    );

    let mut interfaces: Vec<(String, Vec<String>)> = Vec::new();
    let mut standalone: Vec<String> = Vec::new();
    for name in &tools {
        match tool_metadata(&register, name) {
            Ok(Some(metadata)) => match txt(&metadata.implements) {
                Some(interface) => {
                    match interfaces.iter_mut().find(|(known, _)| *known == interface) {
                        Some((_, impls)) => impls.push(name.clone()),
                        None => interfaces.push((interface, vec![name.clone()])),
                    }
                }
                None => standalone.push(name.clone()),
            },
            Ok(None) => {
                return Err(gated(
                    &format!("list (interface lookup for {name})"),
                    FIND_BY_NEEDS,
                    out.writes,
                ))
            }
            Err((class, message)) => {
                return Err(InspectError::Raised {
                    class,
                    message,
                    partial: out.writes,
                })
            }
        }
    }

    let options = tool_options(ctx, &config).map_err(|error| InspectError::Shell {
        partial: std::mem::take(&mut out.writes),
        error,
    })?;
    let cache = ToolCache::new();
    interfaces.sort_by(|a, b| a.0.cmp(&b.0));
    for (interface, mut impls) in interfaces {
        out.say(&format!("  {interface}:"), Some(Color::Cyan));
        impls.sort();
        for name in &impls {
            list_tool(&mut out, ctx, &register, &cache, &options, name, "    ")?;
        }
    }
    standalone.sort();
    for name in &standalone {
        list_tool(&mut out, ctx, &register, &cache, &options, name, "  ")?;
    }
    Ok(out.done())
}

/// One tool line of `list`: `[✓]`/`[✗]`, the name and `v<version>`, or the
/// `[?]` line for a failure the source rescues (`Ukiryu::Errors::Error`).
fn list_tool(
    out: &mut Out,
    ctx: &HandlerContext,
    register: &Register,
    cache: &ToolCache<Rc<Tool>>,
    options: &ToolOptions,
    name: &str,
    indent: &str,
) -> Result<(), InspectError> {
    match get_tool(ctx, register, cache, options, name) {
        Ok(tool) => {
            let Some(version) = tool.version() else {
                return Err(gated(
                    &format!("list (version of {name})"),
                    "Tool#version detection, which needs a man-page date provider (ManPageDates)",
                    std::mem::take(&mut out.writes),
                ));
            };
            let available = tool.is_available();
            let mark = if available { "[✓]" } else { "[✗]" };
            out.say(
                &format!("{indent}{mark:<4} {name:<20} v{version}"),
                Some(if available { Color::Green } else { Color::Red }),
            );
            Ok(())
        }
        Err(error) if error.kind().is_some() => {
            out.say(
                &format!("{indent}[?] {name:<20} error: {error}"),
                Some(Color::Red),
            );
            Ok(())
        }
        Err(error) => Err(tool_failure(&error, std::mem::take(&mut out.writes))),
    }
}

// --------------------------------------------------------------- commands

/// `ukiryu commands TOOL` (`--register` is in [`HandlerContext`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandsInput {
    pub tool: String,
}

/// `CommandsCommand#run`.
pub fn commands(
    input: CommandsInput,
    ctx: &HandlerContext,
) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let mut out = Out::new(colors_disabled(&config));
    let register = setup_register(ctx, &config)
        .map_err(|f| f.into_error(Vec::new()))?
        .register;
    let options = tool_options(ctx, &config).map_err(|error| InspectError::Shell {
        partial: Vec::new(),
        error,
    })?;
    // `Tool.get` raises `ToolNotFoundError` for a missing tool (the source's
    // `unless tool` check never runs).
    let tool = get_tool(ctx, &register, &ToolCache::new(), &options, &input.tool)
        .map_err(|e| tool_failure(&e, Vec::new()))?;

    out.say(&format!("Commands for {}:", input.tool), Some(Color::Cyan));
    let mut grouped: Vec<(Option<String>, Vec<&CommandDefinition>)> = Vec::new();
    for command in tool.commands() {
        let parent = txt(&command.belongs_to);
        match grouped.iter_mut().find(|(key, _)| *key == parent) {
            Some((_, members)) => members.push(command),
            None => grouped.push((parent, vec![command])),
        }
    }

    if grouped.len() == 1 && grouped[0].0.is_none() {
        for command in &grouped[0].1 {
            display_command(&mut out, command);
        }
        return Ok(out.done());
    }

    let routing = tool.command_profile().routing();
    if tool.has_routing() {
        out.say("  Routing table:", Some(Color::Dim));
        for (key, _) in routing.table() {
            let target = routing.resolve(&key).map(|v| v.to_ruby_s());
            out.say(
                &format!("    {key} => {}", target.unwrap_or_default()),
                Some(Color::Dim),
            );
        }
        out.clear();
    }
    if let Some((_, top)) = grouped.iter().find(|(key, _)| key.is_none()) {
        if !top.is_empty() {
            out.say("  Top-level commands:", Some(Color::Cyan));
            for command in top {
                display_command(&mut out, command);
            }
            out.clear();
        }
    }
    for (parent, members) in &grouped {
        let Some(parent) = parent else { continue };
        let target = if tool.has_routing() {
            routing.resolve(parent).map(|v| v.to_ruby_s())
        } else {
            None
        }
        .unwrap_or_else(|| parent.clone());
        out.say(
            &format!("  {parent} commands (route to: {target}):"),
            Some(Color::Cyan),
        );
        for command in members {
            display_command(&mut out, command);
        }
        out.clear();
    }
    Ok(out.done())
}

/// `CommandsCommand#display_command`.
fn display_command(out: &mut Out, command: &CommandDefinition) {
    let name = txt(&command.name).unwrap_or_else(|| "unnamed".to_owned());
    let description = txt(&command.description).unwrap_or_else(|| "No description".to_owned());
    out.say(&format!("  {name:<20} {description}"), Some(Color::White));
    if let Some(usage) = txt(&command.usage) {
        out.say(&format!("    Usage: {usage}"), Some(Color::Dim));
    }
    if let Some(subcommand) = txt(&command.subcommand) {
        out.say(&format!("    Subcommand: {subcommand}"), Some(Color::Dim));
    }
    if command.is_flag_action() {
        let flag = txt(&command.cli_flag).unwrap_or_default();
        out.say(&format!("    Action flag: {flag}"), Some(Color::Dim));
    }
}

// --------------------------------------------------------- info/opts/which

/// `ukiryu info TOOL [--all]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfoInput {
    pub tool: String,
    pub all: bool,
}

/// `ukiryu opts TOOL [COMMAND]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptsInput {
    pub tool: String,
    pub command: Option<String>,
}

/// `ukiryu which IDENTIFIER [--platform P] [--shell S]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhichInput {
    pub identifier: String,
    pub platform: Option<String>,
    pub shell: Option<String>,
}

/// `info` resolves its tool with `Tool.find_by`: gated, see [`FIND_BY_NEEDS`].
pub fn info(_input: InfoInput, _ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    Err(gated("info", FIND_BY_NEEDS, Vec::new()))
}

/// `opts` resolves its tool with `Tool.find_by`: gated, see [`FIND_BY_NEEDS`].
pub fn opts(_input: OptsInput, _ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    Err(gated("opts", FIND_BY_NEEDS, Vec::new()))
}

/// `which` falls back to interface and alias discovery and reads
/// `compatible_profile`: gated, see [`FIND_BY_NEEDS`].
pub fn which(_input: WhichInput, _ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    Err(gated("which", FIND_BY_NEEDS, Vec::new()))
}

// --------------------------------------------------------------- register

/// `ukiryu register [SUBCOMMAND] [--force]`. `--verbose` only sets the
/// `debug` CLI option, which no `register` output reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegisterInput {
    pub subcommand: Option<String>,
    pub force: bool,
}

/// `RegisterCommand#run`. It does not call `setup_register`, so the default
/// register is discovered from `UKIRYU_REGISTER`, then `~/.ukiryu/register`
/// (cloned when missing).
pub fn register(
    input: RegisterInput,
    ctx: &HandlerContext,
) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let mut out = Out::new(colors_disabled(&config));
    match input.subcommand.as_deref() {
        None | Some("info") => register_step(out, |out| show_info(out, ctx)),
        Some("update") => register_step(out, |out| update_register(out, ctx, input.force)),
        Some("path") => {
            let path = match default_register(ctx) {
                Ok(register) => register.path().to_path_buf(),
                Err(failure) => return register_failure(out, failure),
            };
            if path.is_dir() {
                out.say(&path.to_string_lossy(), None);
                Ok(out.done())
            } else {
                Ok(out.error("Register not available. Run: ukiryu register update"))
            }
        }
        Some(other) => Ok(out.error(&format!(
            "Unknown subcommand: {other}. Valid subcommands: info, update, path"
        ))),
    }
}

/// `rescue Ukiryu::Register::Error => e; error!("Register error: ...")`.
fn register_failure(out: Out, failure: RegisterFailure) -> Result<CommandOutcome, InspectError> {
    match failure {
        RegisterFailure::Register(error) if is_register_error(&error) => {
            Ok(out.error(&format!("Register error: {error}")))
        }
        other => Err(other.into_error(out.writes)),
    }
}

fn register_step(
    mut out: Out,
    step: impl FnOnce(&mut Out) -> Result<(), RegisterStepError>,
) -> Result<CommandOutcome, InspectError> {
    match step(&mut out) {
        Ok(()) => Ok(out.done()),
        Err(RegisterStepError::Register(failure)) => register_failure(out, failure),
        Err(RegisterStepError::Gated(error)) => Err(error(out.writes)),
    }
}

enum RegisterStepError {
    Register(RegisterFailure),
    /// A gate raised once the writes so far are known.
    Gated(Box<dyn FnOnce(Vec<OutputWrite>) -> InspectError>),
}

impl From<RegisterFailure> for RegisterStepError {
    fn from(failure: RegisterFailure) -> Self {
        RegisterStepError::Register(failure)
    }
}

/// `RegisterCommand#show_info`.
fn show_info(out: &mut Out, ctx: &HandlerContext) -> Result<(), RegisterStepError> {
    let register = default_register(ctx)?;
    show_info_of(out, ctx, &register)
}

fn show_info_of(
    out: &mut Out,
    ctx: &HandlerContext,
    register: &Register,
) -> Result<(), RegisterStepError> {
    let info = register.info(&Git::default());
    out.say("Register Information", Some(Color::Cyan));
    out.say("", None);
    if !info.exists {
        out.say("  Status: Not configured", Some(Color::Red));
        out.say("", None);
        out.say(
            "  No register found. Run: ukiryu register update",
            Some(Color::Yellow),
        );
    } else if !info.valid {
        out.say("  Status: Invalid", Some(Color::Red));
        out.say(
            &format!("  Path: {}", info.path.display()),
            Some(Color::Dim),
        );
        out.say("", None);
        out.say(
            "  Register is corrupted. Run: ukiryu register update --force",
            Some(Color::Yellow),
        );
    } else {
        out.say("  Status: OK", Some(Color::Green));
        out.say(
            &format!("  Path: {}", info.path.display()),
            Some(Color::Dim),
        );
        let source = match info.source {
            register::Source::Env => "Environment variable (UKIRYU_REGISTER)",
            register::Source::Dev => "Development mode (local submodule)",
            register::Source::User => "User local clone (~/.ukiryu/register)",
            other => other.as_str(),
        };
        out.say(&format!("  Source: {source}"), Some(Color::Dim));
        out.say(
            &format!("  Tools available: {}", info.tools_count),
            Some(Color::Dim),
        );
        if let Some(git) = &info.git_info {
            if let Some(branch) = &git.branch {
                out.say(&format!("  Branch: {branch}"), Some(Color::Dim));
            }
            if let Some(commit) = &git.commit {
                out.say(&format!("  Commit: {commit}"), Some(Color::Dim));
            }
            if let Some(seconds) = git.last_update {
                if !utc_zone(ctx) {
                    return Err(RegisterStepError::Gated(Box::new(|partial| {
                        gated(
                            "register info (last update time)",
                            LOCAL_ZONE_NEEDS,
                            partial,
                        )
                    })));
                }
                out.say(
                    &format!("  Last updated: {}", civil_time(seconds)),
                    Some(Color::Dim),
                );
            }
        }
    }
    out.say("", None);
    out.say("Environment variable:", Some(Color::Cyan));
    match ctx.env(register::ENV_VAR) {
        Some(path) => out.say(&format!("  UKIRYU_REGISTER={path}"), Some(Color::Dim)),
        None => out.say("  UKIRYU_REGISTER (not set)", Some(Color::Dim)),
    }
    if !info.valid {
        out.say("", None);
        out.say("Manual setup:", Some(Color::Cyan));
        out.say("", None);
        out.say("  1. Clone the register:", None);
        out.say(
            "     git clone https://github.com/ukiryu/register ~/.ukiryu/register",
            None,
        );
        out.say("", None);
        out.say("  2. Or set environment variable:", None);
        out.say("     export UKIRYU_REGISTER=/path/to/register", None);
        out.say("", None);
        out.say("  3. Then run this command again.", None);
    }
    Ok(())
}

/// `RegisterCommand#update_register`.
fn update_register(
    out: &mut Out,
    ctx: &HandlerContext,
    force: bool,
) -> Result<(), RegisterStepError> {
    if force {
        out.say("Force re-cloning register...", Some(Color::Yellow));
        let existing = default_register(ctx)?;
        if existing.path().is_dir() {
            // `FileUtils.rm_rf` ignores failures.
            let _ = std::fs::remove_dir_all(existing.path());
        }
    } else {
        out.say("Updating register...", Some(Color::Cyan));
    }
    let register = default_register(ctx)?;
    register
        .update(&Git::default())
        .map_err(|e| RegisterStepError::Register(RegisterFailure::Register(e)))?;
    out.say("Register updated successfully!", Some(Color::Green));
    show_info_of(out, ctx, &register)
}

const LOCAL_ZONE_NEEDS: &str = "a local time-zone provider; only a TZ naming UTC is rendered";

/// Whether the host's local zone is UTC, the only zone rendered here.
fn utc_zone(ctx: &HandlerContext) -> bool {
    matches!(ctx.env("TZ"), Some("UTC" | "UTC0" | "GMT" | "Etc/UTC" | ""))
}

/// `Time#strftime('%Y-%m-%d %H:%M:%S')` of Unix seconds in UTC.
fn civil_time(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

// ---------------------------------------------------------------- resolve

/// `ukiryu resolve TOOL [VERSION_CONSTRAINT]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveInput {
    pub tool: String,
    pub constraint: Option<String>,
}

/// `ResolveCommand#run`: the definitions found under the user, bundled and
/// system data directories (not the register).
pub fn resolve(input: ResolveInput, ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let mut out = Out::new(colors_disabled(&config));
    let discovery = Discovery::new(DiscoveryEnv {
        home: ctx.env("HOME").map(str::to_owned),
        xdg_data_home: ctx.env("XDG_DATA_HOME").map(str::to_owned),
        xdg_data_dirs: ctx.env("XDG_DATA_DIRS").map(str::to_owned),
        path: ctx.env("PATH").map(str::to_owned),
        opt_root: "/opt".to_owned(),
    });
    let tool = input.tool.as_str();
    let definitions = discovery.definitions_for(tool);

    if definitions.is_empty() {
        out.say(&format!("Resolution for: {tool}"), Some(Color::Cyan));
        out.clear();
        out.say(
            &format!("✗ No definitions found for '{tool}'"),
            Some(Color::Red),
        );
        out.clear();
        out.say("To see available tools:", Some(Color::Cyan));
        out.say("  ukiryu list", Some(Color::White));
        out.clear();
        out.say("To search for definitions:", Some(Color::Cyan));
        out.say("  ukiryu definitions list", Some(Color::White));
        return Ok(out.exit(1));
    }

    out.say(&format!("Resolution for: {tool}"), Some(Color::Cyan));
    out.clear();
    out.say(
        &format!("Available Definitions ({}):", definitions.len()),
        Some(Color::White),
    );
    for metadata in &definitions {
        let mtime = match definition_mtime(ctx, metadata) {
            Ok(mtime) => mtime,
            Err(error) => return Err(error(std::mem::take(&mut out.writes))),
        };
        out.say(
            &format!(
                "  {} {} ({})",
                priority_icon(metadata.priority()),
                metadata.version,
                metadata.source_type.as_str()
            ),
            Some(Color::White),
        );
        out.say(&format!("    Path: {}", metadata.path), Some(Color::Dim));
        out.say(&format!("    Mtime: {mtime}"), Some(Color::Dim));
        out.say(
            &format!("    Priority: {}", metadata.priority()),
            Some(Color::Dim),
        );
        out.clear();
    }

    let selected = match input.constraint.as_deref() {
        Some(constraint) => {
            let versions: Vec<&str> = definitions.iter().map(|d| d.version.as_str()).collect();
            let chosen = match VersionResolver::resolve(constraint, Some(versions.as_slice())) {
                Ok(chosen) => chosen,
                Err(error) => {
                    return Err(InspectError::Raised {
                        class: error.ruby_class(),
                        message: error.message,
                        partial: out.writes,
                    })
                }
            };
            let Some(version) = chosen else {
                out.say(
                    &format!("✗ No version satisfies constraint: {constraint}"),
                    Some(Color::Red),
                );
                out.clear();
                out.say("Available versions:", Some(Color::White));
                for version in &versions {
                    out.say(&format!("  - {version}"), Some(Color::Dim));
                }
                return Ok(out.exit(1));
            };
            let selected = definitions
                .iter()
                .find(|d| d.version == version)
                .unwrap_or(&definitions[0]);
            out.say(
                &format!("Selected Definition (constraint: {constraint}):"),
                Some(Color::Cyan),
            );
            out.say(
                &format!("  ✓ {}/{}", selected.name, selected.version),
                Some(Color::Green),
            );
            out.say(
                &format!("    Source: {}", selected.source_type.as_str()),
                Some(Color::White),
            );
            out.say(&format!("    Path: {}", selected.path), Some(Color::White));
            selected
        }
        None => {
            let selected = &definitions[0];
            out.say("Selected Definition (highest priority):", Some(Color::Cyan));
            out.say(
                &format!("  ✓ {}/{}", selected.name, selected.version),
                Some(Color::Green),
            );
            out.say(
                &format!("    Source: {}", selected.source_type.as_str()),
                Some(Color::White),
            );
            out.say(&format!("    Path: {}", selected.path), Some(Color::White));
            out.say(
                &format!("    Priority: {}", selected.priority()),
                Some(Color::White),
            );
            selected
        }
    };

    if !selected.exists() {
        out.clear();
        out.say(
            "⚠ Warning: Definition file does not exist!",
            Some(Color::Yellow),
        );
    }
    Ok(out.done())
}

/// `priority_icon`.
fn priority_icon(priority: u32) -> &'static str {
    match priority {
        1 => "★",
        2 => "◆",
        3 => "■",
        4 => "□",
        5 => "△",
        _ => "·",
    }
}

/// `Time#to_s` of the definition file's mtime (`nil.to_s` when the file is
/// missing). The error is a gate raised with the writes so far.
fn definition_mtime(
    ctx: &HandlerContext,
    metadata: &DefinitionMetadata,
) -> Result<String, Box<dyn FnOnce(Vec<OutputWrite>) -> InspectError>> {
    let Some(modified) = metadata.mtime() else {
        return Ok(String::new());
    };
    if !utc_zone(ctx) {
        return Err(Box::new(|partial| {
            gated("resolve (definition mtime)", LOCAL_ZONE_NEEDS, partial)
        }));
    }
    let seconds = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => after.as_secs() as i64,
        Err(before) => -(before.duration().as_secs() as i64),
    };
    Ok(format!("{} +0000", civil_time(seconds)))
}
