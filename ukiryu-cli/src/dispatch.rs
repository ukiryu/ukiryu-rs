//! The binary's dispatcher: parse the arguments with [`crate::arguments`],
//! build each handler's typed input and context from the process, perform the
//! handler's ordered writes on the real stdout / stderr / files, and return the
//! process exit status.
//!
//! Exit statuses:
//! - a handler's own status (`CommandOutcome::exit_code`);
//! - 1 for an argument error and for an uncaught source exception, as
//!   `FriendlyCLI.start` exits after printing the error;
//! - 0 for `--help` / `--version`;
//! - [`GATED_EXIT`] for a verb (or a path of a verb) whose handler is not
//!   integrated yet. It prints which part is missing on stderr and never
//!   pretends to succeed.
//!
//! Text is written as given: the handlers produce UTF-8 strings, and nothing
//! here re-encodes, appends or strips bytes (no newline translation).
use crate::commands::author::{
    self, AuthorContext, AuthorError, Confirm, DefinitionsInput, ExtractInput, LintInput,
    LintOptions, LintTarget, ValidateInput, ValidateOptions, ValidateTarget,
};
use crate::commands::inspect::{
    self, ConfigInput, HandlerContext, InspectError, SystemInput, VersionInput,
};
use crate::commands::CommandOutcome;
use clap::error::ErrorKind;
use clap::ArgMatches;
use serde_json::Value;
use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Write};
use ukiryu::config::{self, OverrideResolver, Values};
use ukiryu::definition::DiscoveryEnv;
use ukiryu::environment::Environment;
use ukiryu::extractors::base::ProcessHost;
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{Clock, Host, Runtime};

/// Exit status of a verb whose handler is not integrated in this build.
pub const GATED_EXIT: i32 = 2;

/// Verbs of the source CLI whose handlers are not merged yet, with the
/// component they wait for.
const GATED_VERBS: [(&str, &str); 11] = [
    ("run-file", "the run-file handler (K22-CLI-EXECUTE)"),
    ("exec", "the exec handler (K22-CLI-EXECUTE)"),
    ("list", "the register and tool APIs (K22-CLI-INSPECT)"),
    ("info", "the register and tool APIs (K22-CLI-INSPECT)"),
    ("commands", "the register and tool APIs (K22-CLI-INSPECT)"),
    ("opts", "the register and tool APIs (K22-CLI-INSPECT)"),
    ("describe", "the tool API (K22-CLI-AUTHOR)"),
    ("which", "the register and tool APIs (K22-CLI-INSPECT)"),
    ("cache", "the cache handler (K22-CLI-EXECUTE)"),
    ("resolve", "the register and tool APIs (K22-CLI-INSPECT)"),
    ("register", "the register API (K22-CLI-INSPECT)"),
];

/// Run the CLI over `args` (including the program name) and return the exit
/// status. All output goes to the process's stdout, stderr and files.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut io = ProcessIo;
    run_with(args, &mut io)
}

/// [`run`] over an explicit output sink (the binary passes the process).
fn run_with<I, T>(args: I, io: &mut ProcessIo) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let matches = match crate::arguments::command().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(error) => return parse_error(error),
    };
    let Some((verb, sub)) = matches.subcommand() else {
        // `subcommand_required` makes this unreachable through clap.
        return gated(io, "ukiryu", "a subcommand");
    };
    match dispatch(verb, sub, io) {
        Ok(code) => code,
        Err(Failure::Gated { what, needs }) => gated(io, &what, needs),
        Err(Failure::Help(verb)) => {
            let mut command = crate::arguments::command();
            let help = command
                .find_subcommand_mut(verb)
                .map(|sub| sub.render_help().to_string())
                .unwrap_or_default();
            let mut out = io::stdout().lock();
            let _ = out.write_all(help.as_bytes());
            let _ = out.flush();
            0
        }
        Err(Failure::Message(message)) => {
            io.stderr(&format!("{message}\n"));
            1
        }
    }
}

/// clap's help and version requests print to stdout and succeed; every other
/// parse error goes to stderr with the source's failure status 1.
fn parse_error(error: clap::Error) -> i32 {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let _ = error.print();
            0
        }
        // Thor prints the command list on stdout and succeeds when no
        // command is given.
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let mut out = io::stdout().lock();
            let _ = write!(out, "{}", error.render());
            let _ = out.flush();
            0
        }
        _ => {
            let _ = error.print();
            1
        }
    }
}

enum Failure {
    /// The verb (or this path of it) has no integrated handler yet.
    Gated { what: String, needs: &'static str },
    /// A Thor subcommand group given no subcommand: its help on stdout,
    /// status 0 (as `ukiryu validate` does in the source).
    Help(&'static str),
    /// A failure before any handler ran, printed as one stderr line.
    Message(String),
}

fn gated(io: &mut ProcessIo, what: &str, needs: &str) -> i32 {
    io.stderr(&format!(
        "ukiryu: '{what}' is not available in this build yet; it needs {needs}\n"
    ));
    GATED_EXIT
}

fn dispatch(verb: &str, sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    match verb {
        "version" => inspect_outcome(inspect::version(VersionInput, &handler_context(None)?), io),
        "system" => {
            let ctx = handler_context(text(sub, "register")?)?;
            let input = SystemInput {
                subcommand: text(sub, "subcommand")?,
            };
            inspect_outcome(inspect::system(input, &ctx), io)
        }
        "config" => {
            let ctx = handler_context(text(sub, "register")?)?;
            let input = ConfigInput::from_arguments(
                text(sub, "action")?.as_deref(),
                text(sub, "key")?.as_deref(),
                text(sub, "value")?.as_deref(),
            )
            .ok_or(Failure::Gated {
                what: "config list".to_owned(),
                needs: "the register API (Register.default)",
            })?;
            inspect_outcome(inspect::config(input, &ctx), io)
        }
        "validate" => validate(sub, io),
        "lint" => lint(sub, io),
        "definitions" => definitions(sub, io),
        "extract" => extract(sub, io),
        other => {
            let needs = GATED_VERBS
                .iter()
                .find(|(name, _)| *name == other)
                .map_or("its handler", |(_, needs)| *needs);
            Err(Failure::Gated {
                what: other.to_owned(),
                needs,
            })
        }
    }
}

// ------------------------------------------------------------------ inputs

/// A string option or positional. Handlers take UTF-8 text, so a non-UTF-8
/// argument is reported instead of being altered.
fn text(matches: &ArgMatches, id: &str) -> Result<Option<String>, Failure> {
    match matches.try_get_one::<OsString>(id) {
        Ok(Some(value)) => value.to_str().map(|v| Some(v.to_owned())).ok_or_else(|| {
            Failure::Message(format!(
                "ukiryu: argument is not valid UTF-8: {}",
                value.to_string_lossy()
            ))
        }),
        _ => Ok(None),
    }
}

fn required(matches: &ArgMatches, id: &str) -> Result<String, Failure> {
    // clap enforces required positionals before dispatch.
    Ok(text(matches, id)?.unwrap_or_default())
}

/// A Thor boolean option (`--x`, `--x=false`, `--no-x`; last one wins).
fn flag(matches: &ArgMatches, id: &str) -> bool {
    matches
        .try_get_one::<bool>(id)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

fn handler_context(register: Option<String>) -> Result<HandlerContext, Failure> {
    HandlerContext::native(register).map_err(|e| Failure::Message(format!("ukiryu: {e}")))
}

/// `Thor::Shell::Color#can_display_colors?`: stdout is a terminal, `TERM` is
/// not `dumb`, and `NO_COLOR` is unset or empty.
fn thor_colors() -> bool {
    io::stdout().is_terminal()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

/// The process facts the authoring verbs read. `Register.default` is resolved
/// by the register owner and is not available yet, so the verbs that need it
/// report [`AuthorError::RegisterUnavailable`].
fn author_context(config_colors_disabled: bool) -> AuthorContext {
    AuthorContext {
        thor_colors: thor_colors(),
        discovery: DiscoveryEnv::from_process(),
        schema_path_env: std::env::var("UKIRYU_SCHEMA_PATH").ok(),
        default_register: None,
        config_colors_disabled,
    }
}

/// `Ukiryu::Config.instance` with `--register` applied (BaseCommand), for the
/// verbs that read it outside the inspect handlers. A `UKIRYU_*` value that
/// does not convert raises `ArgumentError` in the source.
fn configuration(host: &Host) -> Result<OverrideResolver, Failure> {
    let environment: config::Environment = host
        .env
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let env = config::environment_values(&environment)
        .map_err(|e| Failure::Message(format!("[ArgumentError] {e}")))?;
    Ok(OverrideResolver {
        defaults: config::defaults(),
        programmatic: Values::new(),
        env,
        cli: Values::new(),
    })
}

// ------------------------------------------------------------------ verbs

fn validate(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let (target, leaf) = match sub.subcommand() {
        Some(("file", leaf)) => (
            ValidateTarget::File {
                path: required(leaf, "path")?,
            },
            leaf,
        ),
        Some(("string", leaf)) => (
            ValidateTarget::String {
                yaml: required(leaf, "yaml")?,
            },
            leaf,
        ),
        Some(("all", leaf)) => (ValidateTarget::All, leaf),
        _ => return Err(Failure::Help("validate")),
    };
    let options = ValidateOptions {
        verbose: flag(leaf, "verbose"),
        format: text(leaf, "format")?,
        schema: text(leaf, "schema")?,
        strict: flag(leaf, "strict"),
        executable: flag(leaf, "executable"),
        all: flag(leaf, "all"),
        register: text(leaf, "register")?,
    };
    let ctx = author_context(false);
    author_outcome(
        "validate",
        author::validate(ValidateInput { target, options }, &ctx),
        io,
    )
}

/// `lint` is a target addition (not registered in the source cli.rb).
fn lint(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let (target, leaf) = match sub.subcommand() {
        Some(("file", leaf)) => (
            LintTarget::File {
                path: required(leaf, "path")?,
            },
            leaf,
        ),
        Some(("all", leaf)) => (
            LintTarget::All {
                register: text(leaf, "register")?,
            },
            leaf,
        ),
        Some(("rules", leaf)) => (LintTarget::Rules, leaf),
        _ => return Err(Failure::Help("lint")),
    };
    let options = LintOptions {
        verbose: flag(leaf, "verbose"),
        format: text(leaf, "format")?,
        severity: text(leaf, "severity")?,
    };
    let ctx = author_context(false);
    author_outcome(
        "lint",
        author::lint(LintInput { target, options }, &ctx),
        io,
    )
}

fn definitions(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let input = match sub.subcommand() {
        None => DefinitionsInput::List { verbose: false },
        Some(("list", leaf)) => DefinitionsInput::List {
            verbose: flag(leaf, "verbose"),
        },
        Some(("path", _)) => DefinitionsInput::Path,
        Some(("add", leaf)) => DefinitionsInput::Add {
            source: required(leaf, "source")?,
            name: text(leaf, "name")?,
            version: text(leaf, "version")?,
        },
        Some(("remove", leaf)) => DefinitionsInput::Remove {
            tool: required(leaf, "tool")?,
            version: text(leaf, "version")?,
            force: flag(leaf, "force"),
        },
        Some(("info", leaf)) => DefinitionsInput::Info {
            tool: required(leaf, "tool")?,
            version: text(leaf, "version")?,
        },
        Some((other, _)) => {
            return Err(Failure::Gated {
                what: format!("definitions {other}"),
                needs: "its handler",
            })
        }
    };
    let ctx = author_context(false);
    let mut confirm = StdinConfirm { io: ProcessIo };
    let result = author::definitions(input, &ctx, &mut confirm);
    author_outcome("definitions", result, io)
}

fn extract(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let input = ExtractInput {
        tool: required(sub, "tool")?,
        output: text(sub, "output")?,
        method: text(sub, "method")?,
        verbose: flag(sub, "verbose"),
    };
    let host = Host::native().map_err(|e| Failure::Message(format!("ukiryu: {e}")))?;
    let config = configuration(&host)?;
    let colors_disabled = config.resolve("use_color") == Some(&Value::Bool(false));
    let configured_shell = config
        .resolve("shell")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // Extractors run their commands through the detected shell
    // (`Shell.detect`, strict per DECISIONS.md D2) with the inherited process
    // environment (`Environment.from_env`; an empty overlay inherits it).
    let shell = Runtime::new(host, configured_shell, Clock::default())
        .shell()
        .map_err(|e| Failure::Message(format!("[Ukiryu::Errors::UnknownShellError] {e}")))?;
    let mut process = ProcessHost::new(Environment::new(), shell);
    let ctx = author_context(colors_disabled);
    author_outcome("extract", author::extract(input, &ctx, &mut process), io)
}

// ------------------------------------------------------------------ outcomes

fn inspect_outcome(
    result: Result<CommandOutcome, InspectError>,
    io: &mut ProcessIo,
) -> Result<i32, Failure> {
    match result {
        Ok(outcome) => Ok(io.perform(&outcome.writes).unwrap_or(outcome.exit_code)),
        Err(error) => {
            if let InspectError::Shell { partial, .. } = &error {
                if let Some(code) = io.perform(partial) {
                    return Ok(code);
                }
            }
            io.stderr(&format!("{}\n", error.friendly_message()));
            Ok(1)
        }
    }
}

fn author_outcome(
    verb: &str,
    result: Result<CommandOutcome, AuthorError>,
    io: &mut ProcessIo,
) -> Result<i32, Failure> {
    let (partial, what, needs) = match result {
        Ok(outcome) => return Ok(io.perform(&outcome.writes).unwrap_or(outcome.exit_code)),
        Err(AuthorError::Gated { feature, partial }) => (partial, format!("{verb} ({feature})"), {
            if feature.contains("describe") {
                "the tool API"
            } else {
                "the tool and executor APIs"
            }
        }),
        Err(AuthorError::RegisterUnavailable { partial }) => (
            partial,
            verb.to_owned(),
            "the register API (Register.default)",
        ),
        Err(AuthorError::Unsupported { detail, partial }) => {
            if let Some(code) = io.perform(&partial) {
                return Ok(code);
            }
            return Err(Failure::Gated {
                what: format!("{verb} ({detail})"),
                needs: "a core API that reproduces this source behaviour",
            });
        }
    };
    if let Some(code) = io.perform(&partial) {
        return Ok(code);
    }
    Err(Failure::Gated { what, needs })
}

// ------------------------------------------------------------------ output

/// The process's stdout, stderr and filesystem.
struct ProcessIo;

impl ProcessIo {
    /// Perform `writes` in order. Returns `Some(1)` after reporting the first
    /// write that failed (later writes are not attempted, as the source's
    /// exception stops the command), `None` when all succeeded.
    fn perform(&mut self, writes: &[OutputWrite]) -> Option<i32> {
        for write in writes {
            let result = match write {
                OutputWrite::Stdout(text) => {
                    let mut out = io::stdout().lock();
                    out.write_all(text.as_bytes()).and_then(|()| out.flush())
                }
                OutputWrite::Stderr(text) => {
                    let mut err = io::stderr().lock();
                    err.write_all(text.as_bytes()).and_then(|()| err.flush())
                }
                OutputWrite::File { path, contents } => std::fs::write(path, contents)
                    .map_err(|e| io::Error::new(e.kind(), file_error(&e, path))),
            };
            if let Err(error) = result {
                if error.kind() == io::ErrorKind::BrokenPipe {
                    return Some(1);
                }
                self.stderr(&format!("{error}\n"));
                return Some(1);
            }
        }
        None
    }

    fn stderr(&mut self, text: &str) {
        let mut err = io::stderr().lock();
        let _ = err.write_all(text.as_bytes());
        let _ = err.flush();
    }
}

/// `File.write` failure as `FriendlyCLI` prints the uncaught `Errno` error.
fn file_error(error: &io::Error, path: &str) -> String {
    let (class, text) = match error.kind() {
        io::ErrorKind::NotFound => ("Errno::ENOENT", "No such file or directory"),
        io::ErrorKind::PermissionDenied => ("Errno::EACCES", "Permission denied"),
        io::ErrorKind::IsADirectory => ("Errno::EISDIR", "Is a directory"),
        _ => return format!("[IOError] {error} - {path}"),
    };
    format!("[{class}] {text} @ rb_sysopen - {path}")
}

/// `definitions remove` confirmation: show the prompt, then `$stdin.gets`.
struct StdinConfirm {
    io: ProcessIo,
}

impl Confirm for StdinConfirm {
    fn ask(&mut self, shown: Vec<OutputWrite>) -> Option<String> {
        if self.io.perform(&shown).is_some() {
            // The prompt could not be written; answer as at end of input.
            return None;
        }
        let mut line = String::new();
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
}
