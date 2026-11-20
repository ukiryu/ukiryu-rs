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
    self, AuthorContext, AuthorError, Confirm, DefinitionsInput, DescribeInput, ExtractInput,
    LintInput, LintOptions, LintTarget, ToolEnv, ValidateInput, ValidateOptions, ValidateTarget,
};
use crate::commands::execute::{
    self, CacheAction, CacheInput, ExecInput, ExecuteContext, ExecuteError, RunFileInput,
};
use crate::commands::inspect::{
    self, CommandsInput, ConfigInput, HandlerContext, InfoInput, InspectError, OptsInput,
    RegisterInput, ResolveInput, SystemInput, VersionInput, WhichInput,
};
use crate::commands::CommandOutcome;
use clap::error::ErrorKind;
use clap::ArgMatches;
use serde_json::Value;
use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Write};
use ukiryu::config::{self, OverrideResolver, Values};
use ukiryu::definition::{DefinitionCache, DefinitionMetadata, DiscoveryEnv};
use ukiryu::environment::Environment;
use ukiryu::extractors::base::ProcessHost;
use ukiryu::render::{OutputWrite, PaintSupport};
use ukiryu::runtime::{Clock, Host, Runtime};

/// Exit status of a verb whose handler is not integrated in this build.
pub const GATED_EXIT: i32 = 2;

/// Run the CLI over `args` (including the program name) and return the exit
/// status. All output goes to the process's stdout, stderr and files.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut io = ProcessIo::default();
    run_with(args, &mut io)
}

/// [`run`] over an explicit output sink (the binary passes the process).
fn run_with<I, T>(args: I, io: &mut ProcessIo) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    // Thor decides help, `-h`, unknown or ambiguous commands, unknown switches
    // and argument counts before any command runs (`crate::thor`); what it
    // cannot model goes on to the clap grammar.
    let args = match args
        .iter()
        .skip(1)
        .map(|a| a.to_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()
    {
        None => args,
        Some(words) => match crate::thor::preempt(&words, io.env("THOR_COLUMNS").as_deref()) {
            crate::thor::Preempt::Output {
                stdout,
                stderr,
                code,
            } => {
                if !stdout.is_empty() {
                    let _ = io.perform(&[OutputWrite::Stdout(stdout)]);
                }
                if !stderr.is_empty() {
                    io.stderr(&stderr);
                }
                return code;
            }
            crate::thor::Preempt::Pass(words) => {
                let mut passed = vec![args[0].clone()];
                passed.extend(words.into_iter().map(OsString::from));
                passed
            }
        },
    };
    let matches = match crate::arguments::command().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(error) => return parse_error(error, io),
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
            let _ = io.perform(&[OutputWrite::Stdout(help)]);
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
fn parse_error(error: clap::Error, io: &mut ProcessIo) -> i32 {
    match error.kind() {
        // Thor reports an unknown (sub)command as one stderr line, status 1.
        ErrorKind::InvalidSubcommand => {
            if let Some(clap::error::ContextValue::String(name)) =
                error.get(clap::error::ContextKind::InvalidSubcommand)
            {
                io.stderr(&format!("Could not find command \"{name}\".\n"));
                return 1;
            }
            io.print_error(&error);
            1
        }
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            io.print_error(&error);
            0
        }
        // Thor prints the command list on stdout and succeeds when no
        // command is given.
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _ = io.perform(&[OutputWrite::Stdout(error.to_string())]);
            0
        }
        _ => {
            io.print_error(&error);
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
        "version" => inspect_outcome(
            inspect::version(VersionInput, &io.handler_context(None)?),
            io,
        ),
        "system" => {
            let ctx = io.handler_context(text(sub, "register")?)?;
            let input = SystemInput {
                subcommand: text(sub, "subcommand")?,
            };
            inspect_outcome(inspect::system(input, &ctx), io)
        }
        "config" => {
            let ctx = io.handler_context(text(sub, "register")?)?;
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
        "info" => {
            // A bare `ukiryu info` prints Thor's command list (not the info
            // command's usage): `ThorExt` turns the missing argument into
            // `help`, whatever other options were given.
            let Some(tool) = text(sub, "tool")? else {
                let help = thor_command_list(io.env("THOR_COLUMNS").as_deref());
                return Ok(io.perform(&[OutputWrite::Stdout(help)]).unwrap_or(0));
            };
            let ctx = io.handler_context(text(sub, "register")?)?;
            let input = InfoInput {
                tool,
                all: flag(sub, "all"),
            };
            inspect_outcome(inspect::info(input, &ctx), io)
        }
        "opts" => {
            let ctx = io.handler_context(text(sub, "register")?)?;
            let input = OptsInput {
                tool: text(sub, "tool")?.unwrap_or_default(),
                command: text(sub, "command")?,
            };
            inspect_outcome(inspect::opts(input, &ctx), io)
        }
        "list" => {
            let ctx = io.handler_context(text(sub, "register")?)?;
            inspect_outcome(inspect::list(&ctx), io)
        }
        "commands" => {
            let ctx = io.handler_context(text(sub, "register")?)?;
            let input = CommandsInput {
                tool: required(sub, "tool")?,
            };
            inspect_outcome(inspect::commands(input, &ctx), io)
        }
        "register" => {
            // `RegisterCommand` takes no `--register` option.
            let ctx = io.handler_context(None)?;
            let input = RegisterInput {
                subcommand: text(sub, "subcommand")?,
                force: flag(sub, "force"),
            };
            inspect_outcome(inspect::register(input, &ctx), io)
        }
        "resolve" => {
            // `ResolveCommand` takes no `--register` option.
            let ctx = io.handler_context(None)?;
            let input = ResolveInput {
                tool: required(sub, "tool")?,
                constraint: text(sub, "version-constraint")?,
            };
            inspect_outcome(inspect::resolve(input, &ctx), io)
        }
        "validate" => validate(sub, io),
        "lint" => lint(sub, io),
        "definitions" => definitions(sub, io),
        "extract" => extract(sub, io),
        "describe" => describe(sub, io),
        "exec" => exec(sub, io),
        "run-file" => run_file(sub, io),
        "cache" => cache(sub, io),
        "which" => {
            let ctx = io.handler_context(text(sub, "register")?)?;
            let input = WhichInput {
                identifier: required(sub, "identifier")?,
                platform: text(sub, "platform")?,
                shell: text(sub, "shell")?,
            };
            inspect_outcome(inspect::which(input, &ctx), io)
        }
        other => Err(Failure::Gated {
            what: other.to_owned(),
            needs: "its handler",
        }),
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
    let ctx = io.author_context(false);
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
    let ctx = io.author_context(false);
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
    let ctx = io.author_context(false);
    let mut confirm = StdinConfirm { io };
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
    let host = io.host()?;
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
    let ctx = io.author_context(colors_disabled);
    author_outcome("extract", author::extract(input, &ctx, &mut process), io)
}

/// `ukiryu describe TOOL [COMMAND]`. `--format` is accepted and not read,
/// as in the source `DescribeCommand`.
fn describe(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let input = DescribeInput {
        tool: required(sub, "tool")?,
        command: text(sub, "command")?,
        register: text(sub, "register")?,
    };
    let host = io.host()?;
    let config = configuration(&host)?;
    let colors_disabled = config.resolve("use_color") == Some(&Value::Bool(false));
    let configured_shell = config
        .resolve("shell")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let env = ToolEnv {
        host,
        configured_shell,
    };
    let ctx = io.author_context(colors_disabled);
    author_outcome("describe", author::describe(input, &ctx, &env), io)
}

/// The process facts `exec`, `run-file` and `cache` read. The `paint` gem is
/// taken as available, as in the captures (the Ruby CLI ran with it
/// installed).
fn execute_context(io: &ProcessIo) -> Result<ExecuteContext, Failure> {
    Ok(ExecuteContext {
        host: io.host()?,
        stdout_tty: io.stdout_tty(),
        paint: PaintSupport::AVAILABLE,
    })
}

fn exec(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let arguments = match sub.try_get_many::<OsString>("arguments") {
        Ok(Some(values)) => values
            .map(|value| {
                value.to_str().map(str::to_owned).ok_or_else(|| {
                    Failure::Message(format!(
                        "ukiryu: argument is not valid UTF-8: {}",
                        value.to_string_lossy()
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    let input = ExecInput {
        tool: required(sub, "tool")?,
        arguments,
        register: text(sub, "register")?,
        definition: text(sub, "definition")?,
        format: text(sub, "format")?,
        output: text(sub, "output")?,
        dry_run: flag(sub, "dry-run"),
        shell: text(sub, "shell")?,
        stdin: flag(sub, "stdin"),
        raw: flag(sub, "raw"),
    };
    let ctx = execute_context(io)?;
    let result = if io.captured.is_some() {
        execute::exec(input, &ctx, &mut io::Cursor::new(Vec::<u8>::new()))
    } else {
        execute::exec(input, &ctx, &mut io::stdin().lock())
    };
    execute_outcome("exec", result, io)
}

fn run_file(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let input = RunFileInput {
        request_file: required(sub, "request-file")?,
        register: text(sub, "register")?,
        output: text(sub, "output")?,
        format: text(sub, "format")?,
        dry_run: flag(sub, "dry-run"),
        shell: text(sub, "shell")?,
    };
    let ctx = execute_context(io)?;
    execute_outcome("run-file", execute::run_file(input, &ctx), io)
}

/// A CLI process starts with an empty definition cache (nothing populates the
/// source's singleton before the command runs).
fn cache(sub: &ArgMatches, io: &mut ProcessIo) -> Result<i32, Failure> {
    let (action, leaf) = match sub.subcommand() {
        Some(("info", leaf)) => (CacheAction::Info, leaf),
        Some(("stats", leaf)) => (CacheAction::Stats, leaf),
        Some(("clear", leaf)) => (CacheAction::Clear, leaf),
        _ => return Err(Failure::Help("cache")),
    };
    let input = CacheInput {
        action,
        dry_run: flag(leaf, "dry-run"),
        verbose: flag(leaf, "verbose"),
    };
    let ctx = execute_context(io)?;
    let definitions: DefinitionCache<DefinitionMetadata> = DefinitionCache::default();
    execute_outcome("cache", execute::cache(input, &ctx, &definitions), io)
}

// ------------------------------------------------------------------ outcomes

/// A handler's outcome: its writes and status; an uncaught source exception
/// is its `[Class] message` line on stderr and status 1; a gated or
/// unsupported path reports the gate after the writes made before it.
fn execute_outcome(
    verb: &str,
    result: Result<CommandOutcome, ExecuteError>,
    io: &mut ProcessIo,
) -> Result<i32, Failure> {
    let error = match result {
        Ok(outcome) => return Ok(io.perform(&outcome.writes).unwrap_or(outcome.exit_code)),
        Err(error) => error,
    };
    if let Some(code) = io.perform(error.partial()) {
        return Ok(code);
    }
    match error {
        ExecuteError::Raised { .. } => {
            let message = error.friendly_message();
            if io.thor_colors() {
                io.stderr(&format!("\x1b[31m{message}\x1b[0m\n"));
            } else {
                io.stderr(&format!("{message}\n"));
            }
            Ok(1)
        }
        ExecuteError::Gated { stage, detail, .. } => Err(Failure::Gated {
            what: format!("{verb} ({stage}: {detail})"),
            needs: "the register behaviour named here",
        }),
        ExecuteError::Unsupported { feature } => Err(Failure::Gated {
            what: format!("{verb} ({feature})"),
            needs: "a core API that reproduces this source behaviour",
        }),
    }
}

fn inspect_outcome(
    result: Result<CommandOutcome, InspectError>,
    io: &mut ProcessIo,
) -> Result<i32, Failure> {
    match result {
        Ok(outcome) => Ok(io.perform(&outcome.writes).unwrap_or(outcome.exit_code)),
        Err(error) => {
            // The output printed before the failure comes first.
            if let Some(code) = io.perform(error.partial()) {
                return Ok(code);
            }
            io.stderr(&format!("{}\n", error.friendly_message()));
            // A path whose component is missing never reports a plain failure.
            Ok(if matches!(error, InspectError::Gated { .. }) {
                GATED_EXIT
            } else {
                1
            })
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
#[derive(Default)]
struct ProcessIo {
    captured: Option<crate::capture::CapturedOutput>,
    host: Option<Host>,
}

/// Run the same dispatcher as the executable, with explicit process facts and
/// captured output. This never changes ENV, cwd, or process file descriptors.
pub fn capture<I, T>(args: I, host: Host) -> crate::capture::CapturedOutput
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut io = ProcessIo {
        captured: Some(crate::capture::CapturedOutput::default()),
        host: Some(host),
    };
    let code = run_with(args, &mut io);
    let mut output = io.captured.expect("capture sink remains installed");
    output.exit = code;
    output
}

impl ProcessIo {
    fn host(&self) -> Result<Host, Failure> {
        self.host
            .clone()
            .map(Ok)
            .unwrap_or_else(|| Host::native().map_err(|e| Failure::Message(format!("ukiryu: {e}"))))
    }

    fn env(&self, key: &str) -> Option<String> {
        match &self.host {
            Some(host) => host.env.get(key).map(str::to_owned),
            None => std::env::var(key).ok(),
        }
    }

    fn stdout_tty(&self) -> bool {
        self.captured.is_none() && io::stdout().is_terminal()
    }

    fn thor_colors(&self) -> bool {
        self.stdout_tty()
            && self.env("TERM").is_none_or(|v| v != "dumb")
            && self.env("NO_COLOR").is_none_or(|v| v.is_empty())
    }

    fn handler_context(&self, register: Option<String>) -> Result<HandlerContext, Failure> {
        Ok(HandlerContext {
            host: self.host()?,
            register,
        })
    }

    fn author_context(&self, config_colors_disabled: bool) -> AuthorContext {
        AuthorContext {
            thor_colors: self.thor_colors(),
            discovery: DiscoveryEnv {
                home: self.env("HOME"),
                xdg_data_home: self.env("XDG_DATA_HOME"),
                xdg_data_dirs: self.env("XDG_DATA_DIRS"),
                path: self.env("PATH"),
                opt_root: "/opt".to_owned(),
            },
            schema_path_env: self.env("UKIRYU_SCHEMA_PATH"),
            default_register: None,
            config_colors_disabled,
        }
    }

    /// Perform `writes` in order. Returns `Some(1)` after reporting the first
    /// write that failed (later writes are not attempted, as the source's
    /// exception stops the command), `None` when all succeeded.
    fn perform(&mut self, writes: &[OutputWrite]) -> Option<i32> {
        for write in writes {
            let result = match write {
                OutputWrite::Stdout(text) => {
                    if let Some(captured) = &mut self.captured {
                        captured.stdout.push_str(text);
                        continue;
                    }
                    let mut out = io::stdout().lock();
                    out.write_all(text.as_bytes()).and_then(|()| out.flush())
                }
                OutputWrite::Stderr(text) => {
                    if let Some(captured) = &mut self.captured {
                        captured.stderr.push_str(text);
                        continue;
                    }
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

    fn print_error(&mut self, error: &clap::Error) {
        if self.captured.is_none() {
            let _ = error.print();
        } else if error.use_stderr() {
            self.stderr(&error.to_string());
        } else {
            let _ = self.perform(&[OutputWrite::Stdout(error.to_string())]);
        }
    }

    fn stderr(&mut self, text: &str) {
        if let Some(captured) = &mut self.captured {
            captured.stderr.push_str(text);
            return;
        }
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
struct StdinConfirm<'a> {
    io: &'a mut ProcessIo,
}

impl Confirm for StdinConfirm<'_> {
    fn ask(&mut self, shown: Vec<OutputWrite>) -> Option<String> {
        if self.io.perform(&shown).is_some() {
            // The prompt could not be written; answer as at end of input.
            return None;
        }
        if self.io.captured.is_some() {
            return None;
        }
        let mut line = String::new();
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
}

/// Thor's `ukiryu help` listing (`Thor.help`: `print_table(list, indent: 2,
/// truncate: true)` then a blank line), captured from the pinned exe/ukiryu.
/// `width` is `THOR_COLUMNS`; without it Thor asks the terminal, and 80 is
/// what a non-terminal reports. A width under 10 also means 80.
pub(crate) fn thor_command_list(width: Option<&str>) -> String {
    const ROWS: [(&str, &str); 18] = [
        ("ukiryu cache [ACTION]", "Manage definition cache"),
        (
            "ukiryu commands TOOL",
            "List all commands available for a tool",
        ),
        (
            "ukiryu config [ACTION] [KEY] [VALUE]",
            "Manage configuration (list, get, set, unset)",
        ),
        ("ukiryu definitions [COMMAND]", "Manage tool definitions"),
        (
            "ukiryu describe TOOL [COMMAND]",
            "Show comprehensive documentation for a tool or specific command",
        ),
        (
            "ukiryu exec TOOL [COMMAND] [KEY=VALUE ...]",
            "Execute a tool command inline",
        ),
        (
            "ukiryu extract TOOL",
            "Extract tool definition from an installed CLI tool",
        ),
        (
            "ukiryu help [COMMAND]",
            "Describe available commands or one specific command",
        ),
        (
            "ukiryu info [TOOL]",
            "Show detailed information about a tool",
        ),
        ("ukiryu list", "List all available tools in the register"),
        (
            "ukiryu opts TOOL [COMMAND]",
            "Show options for a tool or specific command",
        ),
        ("ukiryu register [SUBCOMMAND]", "Manage the tool register"),
        (
            "ukiryu resolve TOOL [VERSION]",
            "Resolve which definition would be used for a tool",
        ),
        (
            "ukiryu run-file REQUEST_FILE",
            "Execute a Ukiryu Structured Execution Request from a YAML file",
        ),
        (
            "ukiryu system [SUBCOMMAND]",
            "Show system information (shells, etc.)",
        ),
        ("ukiryu tree", "Print a tree of all available commands"),
        ("ukiryu validate [COMMAND]", "Validate tool definitions"),
        ("ukiryu version", "Show Ukiryu version"),
    ];
    // `which` is the 19th row and sorts last.
    const WHICH: (&str, &str) = (
        "ukiryu which IDENTIFIER",
        "Show which tool implementation would be selected",
    );
    const INDENT: usize = 2;
    let terminal_width = match width {
        // Ruby `String#to_i`: leading digits, else 0.
        Some(text) => {
            let digits: String = text
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse::<usize>().unwrap_or(0)
        }
        None => 80,
    };
    let terminal_width = if terminal_width < 10 {
        80
    } else {
        terminal_width
    };
    let column = ROWS
        .iter()
        .chain(std::iter::once(&WHICH))
        .map(|(usage, _)| usage.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::from("ukiryu commands:\n");
    for (usage, about) in ROWS.iter().chain(std::iter::once(&WHICH)) {
        // Thor lists each description as a `# ` comment.
        let sentence = format!("{usage:<width$}# {about}", width = column + 2);
        let chars: Vec<char> = sentence.chars().collect();
        let line: String = if chars.len() <= terminal_width {
            sentence
        } else {
            let keep = terminal_width.saturating_sub(3 + INDENT);
            chars[..keep.min(chars.len())].iter().collect::<String>() + "..."
        };
        out.push_str(&" ".repeat(INDENT));
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    out
}
