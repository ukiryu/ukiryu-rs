//! `ukiryu version`, `ukiryu system [SUBCOMMAND]` and `ukiryu config get|set|unset`.
//!
//! Source: ukiryu `lib/ukiryu/cli_commands/{base,version,system,config}_command.rb`
//! at ab6df353c6dc2d7faf7817a86aafcafa82579896. These verbs print with
//! `BaseCommand#say`, not through `ResponseFormatter`, so each handler returns
//! the ordered `say` lines as [`OutputWrite`]s.
//!
//! Every handler reads an explicit [`HandlerContext`] (host platform and
//! environment snapshot plus the `--register` option); nothing reads or
//! mutates process state except the persistent configuration file, which
//! `config` reads under `$HOME/.ukiryu` and replaces through an
//! [`OutputWrite::File`].
//!
//! `Thor::Error` (the source's `error!`) is rendered as `FriendlyCLI.start`
//! does: the message on stderr and exit status 1. Thor colours that message
//! red only when stderr is a terminal; the handlers write it uncoloured, the
//! form the captured (non-terminal) goldens show.
//!
//! Not covered here:
//! - `config` / `config list` reads `Register.default` (and may clone the
//!   register); it waits for the register API and has no handler yet.
//! - `config get|set|unset` run `setup_register` first in the source, which
//!   can clone the default register into `~/.ukiryu/register`. That bootstrap
//!   does not change their output and is not performed here.
//! - Shell detection follows the strict migrated contract (DECISIONS.md D2):
//!   a missing or unknown `SHELL`, or a Windows host naming no shell, is an
//!   error where the source falls back to `/etc/shells`/`sh`/`cmd`.

use super::CommandOutcome;
use serde_json::Value;
use std::fmt;
use ukiryu::config::{self, ConversionError, OverrideResolver, Values};
use ukiryu::executable::{find_in_path, DiscoveryContext, Platform};
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{Clock, Host, HostError, Runtime, ShellError};

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
            InspectError::PersistentNotMapping { .. } | InspectError::Emit(_) => self.to_string(),
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
}

impl Color {
    fn code(self) -> u8 {
        match self {
            Color::Red => 31,
            Color::Green => 32,
            Color::Yellow => 33,
            Color::Cyan => 36,
            Color::White => 37,
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

/// The `config` actions with a handler. `list` (also the default when no
/// action is given) reads the register and is not represented yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigAction {
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
    /// Map the parser's three positionals. `None` means `list` (the source's
    /// default action), which has no handler in this module.
    pub fn from_arguments(
        action: Option<&str>,
        key: Option<&str>,
        value: Option<&str>,
    ) -> Option<ConfigInput> {
        let key = key.map(str::to_owned);
        let action = match action.unwrap_or("list") {
            "list" => return None,
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

/// `ConfigCommand#run` for `get`, `set`, `unset` and unknown actions.
pub fn config(input: ConfigInput, ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    // CONFIG_FILE is a class constant: loading the command expands `~`.
    let file = config_file(ctx)?;
    let mut out = Out::new(colors_disabled(&config));
    match input.action {
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
