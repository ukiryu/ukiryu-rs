//! Authoring verbs: `validate`, `lint`, `docs`, `definitions` and `extract`.
//!
//! Source: `lib/ukiryu/cli_commands/{validate,lint,docs,definitions,extract}_command.rb`
//! and `lib/ukiryu/definition/definition_validator.rb` at the pinned Ruby
//! oracle (ukiryu/ukiryu ab6df35), with Thor 1.5.0 output rules.
//!
//! Each handler takes typed options (the raw option strings, so the Thor
//! `enum`/`numeric` checks keep their exact messages) and returns the ordered
//! writes plus the exit status. Uncaught Ruby exceptions are rendered the way
//! `FriendlyCLI#handle_exception_on_start` prints them under `exe/ukiryu`:
//! `"[Class] message"` on stderr (red when colours are on), exit status 1.
//!
//! Output conventions: these four commands are Thor classes, so they print
//! with Thor's `say` (colour only when [`AuthorContext::thor_colors`] is set;
//! Thor has no `:dim`, so the rows that use it raise `NameError` when colours
//! are on, as in Ruby), or with plain `puts`/`warn`/`print` (definitions).
//!
//! Where the Ruby command crashes on a defect of its own source before it can
//! produce its designed output, the port implements that designed output and
//! the deviation is listed in [`ORACLE_DEFECTS`]; every other path matches the
//! captured oracle bytes.
//!
//! `extract` is a `BaseCommand` (it prints with `BaseCommand#say`, coloured
//! unless `Config#colors_disabled?`) and runs the merged
//! `extractors::help::extract` on a caller-supplied [`CommandHost`]
//! (`extractors::base::ProcessHost` in the binary; its environment and shell
//! are the owner-pending D1/D2 choices, so the caller makes them).
//!
//! `describe` and the `validate --executable`/`--all` smoke tests run on the
//! typed Tool runtime: the caller supplies a [`ToolEnv`] (host platform and
//! environment, configured shell) and the handlers load tools from the
//! default register through `RegisterSource` and `tool::get`.
//! [`validate`] without a [`ToolEnv`] returns [`AuthorError::Gated`] where the
//! executable test starts; [`validate_with_tools`] runs it (the source loads
//! each tool with `Tool.get`, so the smoke tests take the exact name only).
//! `describe` runs `Tool.find_by` through the core
//! `ukiryu::register_lookup::find_by`: exact name, then the interface, alias
//! and register-wide fallbacks over the `ToolIndex` of the same register.
//!
//! The source crashes on a few of its own paths and the handlers reproduce
//! the crash (see [`REPRODUCED_DEFECTS`]); the owner-pending corrections
//! are not applied here.
//! `DefinitionValidator` is the core `ukiryu::definition::definition_validator`
//! module; with no usable local schema it performs Ruby's remote fallback.
//! Hosts can supply an unavailable response to reproduce offline captures.
use super::CommandOutcome;
use std::cmp::Ordering;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use ukiryu::cache::ToolCache;
use ukiryu::definition::documentation::{DocumentationErrorKind, DocumentationGenerator};
use ukiryu::definition::linter::{DefinitionLinter, LintError, LintResult};
use ukiryu::definition::sources::file::ruby_time_to_s;
use ukiryu::definition::{
    definition_validator, DefinitionDocument, DefinitionError, DefinitionErrorKind,
    DefinitionMetadata, Discovery, DiscoveryEnv, DocumentParseError, Loader, Severity,
    ValidationMode, ValidationResult, ValidatorEnv,
};
use ukiryu::executable::DiscoveryContext;
use ukiryu::execution::Seconds;
use ukiryu::executor::{self, ExecuteError as ProcessError, ExecuteRequest};
use ukiryu::extractors::base::ProcessHost;
use ukiryu::extractors::help::{self as extractors, CommandHost, ExtractionMethod};
use ukiryu::extractors::man::{FileSystem, ManPageParser};
use ukiryu::models::attr::Model;
use ukiryu::models::error::ModelErrorKind;
use ukiryu::models::tool_definition::ToolDefinition;
use ukiryu::models::{CommandDefinition, Text};
use ukiryu::register::Register;
use ukiryu::register_lookup::{self, LookupError};
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{Clock, Host, Runtime};
use ukiryu::schema::{safe_load, Data, Permitted, SchemaError};
use ukiryu::tool::{self, version_info_to_s, Tool, ToolError, ToolOptions};
use ukiryu::tool_loader::RegisterSource;
use ukiryu::types::Value;
use ukiryu::version_detection::{DetectedVersion, DetectionError, RubyPattern};

/// Ruby defects of the oracle commands that the port does not reproduce,
/// as `(command path, Ruby observation, port behaviour)`. Each is captured in
/// the test fixture both as the frozen oracle row and as a target-trace row.
pub const ORACLE_DEFECTS: [(&str, &str, &str); 5] = [
    (
        "lint all",
        "NoMethodError: undefined method 'error_count' for LintResult once a file was linted",
        "error_count = number of error-severity issues",
    ),
    (
        "lint file --format json",
        "NoMethodError: Hash#to_json when json is not loaded (standalone Thor class)",
        "LintResult#to_h as JSON",
    ),
    (
        "docs generate / generate-all",
        "NoMethodError: '[]' for Ukiryu::Models::ToolDefinition (model passed to the Hash generator)",
        "strict Loader validation, then generation from YAML.safe_load(symbolize_names: true)",
    ),
    (
        "docs generate-all",
        "NameError: FileUtils not loaded",
        "creates the output directory",
    ),
    (
        "definitions add / remove",
        "NameError: FileUtils not loaded by exe/ukiryu",
        "copies / removes the files",
    ),
];

/// Ruby defects of the oracle commands that the handlers reproduce, as
/// `(command path, Ruby observation)`. Each has a captured oracle row.
pub const REPRODUCED_DEFECTS: [(&str, &str); 6] = [
    (
        "describe (command with exit codes)",
        "NoMethodError: undefined method 'to_i' for an instance of Symbol (exit code keys are Symbols)",
    ),
    (
        "describe (command with env_vars)",
        "NoMethodError: undefined method 'env_var' for an instance of Ukiryu::Models::EnvVarDefinition",
    ),
    (
        "validate --executable (smoke test without timeout)",
        "NoMethodError: undefined method 'timeout' for an instance of Ukiryu::Models::ToolDefinition",
    ),
    (
        "validate --executable (smoke test with a String command)",
        "NoMethodError: undefined method 'shellsplit' for an instance of String",
    ),
    (
        "validate --executable (no smoke tests)",
        "ArgumentError: Tool#execute is called with a positional hash (given 2, expected 1)",
    ),
    (
        "validate (colours on)",
        "NameError: Thor has no :dim colour; every :dim row raises, inside the rescues of the executable tests",
    ),
];

/// Process state the original commands read, chosen by the caller.
#[derive(Clone, Debug)]
pub struct AuthorContext {
    /// Thor colour support: `$stdout.tty? && ENV["TERM"] != "dumb"` and
    /// `NO_COLOR` unset or empty (`Thor::Shell::Color#can_display_colors?`).
    pub thor_colors: bool,
    /// Environment of `Definition::Discovery` (definitions verbs).
    pub discovery: DiscoveryEnv,
    /// `ENV['UKIRYU_SCHEMA_PATH']` (`DefinitionValidator.find_schema`).
    pub schema_path_env: Option<String>,
    /// Native URL fallback, or an explicitly unavailable offline response.
    pub remote_schema: definition_validator::RemoteSchemaSource,
    /// `Register.default.path`, resolved by the register owner (discovery,
    /// `UKIRYU_REGISTER`, user clone). `None` when it is not resolved; the
    /// verbs that need it then return [`AuthorError::RegisterUnavailable`].
    pub default_register: Option<String>,
    /// `Config#colors_disabled?` (`use_color: false` or `NO_COLOR`), read by
    /// `BaseCommand#say` in `extract`.
    pub config_colors_disabled: bool,
}

/// A path the handlers cannot complete with the merged core APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorError {
    /// The command needs `Register.default` and the caller did not resolve it.
    /// `partial` holds the writes performed before that point.
    RegisterUnavailable { partial: Vec<OutputWrite> },
    /// A path that waits on a core API that is not merged yet.
    Gated {
        feature: &'static str,
        partial: Vec<OutputWrite>,
    },
    /// Ruby behaviour a merged core API reports as not reproduced.
    Unsupported {
        detail: String,
        partial: Vec<OutputWrite>,
    },
}

impl std::fmt::Display for AuthorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorError::RegisterUnavailable { .. } => f.write_str("default register not resolved"),
            AuthorError::Gated { feature, .. } => write!(f, "not available yet: {feature}"),
            AuthorError::Unsupported { detail, .. } => write!(f, "unsupported: {detail}"),
        }
    }
}

impl std::error::Error for AuthorError {}

// ------------------------------------------------------------------ output

/// Thor colour names used by these commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    Red,
    Green,
    Yellow,
    Cyan,
    White,
    Clear,
    /// Not a Thor colour: `lookup_color(:dim)` raises NameError.
    Dim,
}

/// Why a handler stops early.
enum Stop {
    /// `exit status` after the writes so far.
    Exit(i32),
    /// An uncaught Ruby exception.
    Raise {
        class: String,
        message: String,
    },
    Error(AuthorErrorKind),
}

enum AuthorErrorKind {
    Register,
    Gated(&'static str),
    Unsupported(String),
}

type Step<T = ()> = Result<T, Stop>;

fn raise(class: &str, message: impl Into<String>) -> Stop {
    Stop::Raise {
        class: class.to_owned(),
        message: message.into(),
    }
}

struct Out {
    writes: Vec<OutputWrite>,
    colors: bool,
}

impl Out {
    fn new(ctx: &AuthorContext) -> Self {
        Out {
            writes: Vec::new(),
            colors: ctx.thor_colors,
        }
    }

    fn stdout(&mut self, text: String) {
        if let Some(OutputWrite::Stdout(last)) = self.writes.last_mut() {
            last.push_str(&text);
        } else {
            self.writes.push(OutputWrite::Stdout(text));
        }
    }

    fn stderr(&mut self, text: String) {
        if let Some(OutputWrite::Stderr(last)) = self.writes.last_mut() {
            last.push_str(&text);
        } else {
            self.writes.push(OutputWrite::Stderr(text));
        }
    }

    fn paint(&self, message: &str, color: Color) -> Step<String> {
        if !self.colors {
            return Ok(message.to_owned());
        }
        let code = match color {
            Color::Red => "\x1b[31m",
            Color::Green => "\x1b[32m",
            Color::Yellow => "\x1b[33m",
            Color::Cyan => "\x1b[36m",
            Color::White => "\x1b[37m",
            Color::Clear => "\x1b[0m",
            Color::Dim => {
                return Err(raise(
                    "NameError",
                    "uninitialized constant Thor::Shell::Color::DIM",
                ))
            }
        };
        Ok(format!("{code}{message}\x1b[0m"))
    }

    /// `Thor::Shell::Basic#say(message, color)`.
    fn say(&mut self, message: &str, color: Color) -> Step {
        let mut buffer = self.paint(message, color)?;
        let body = message.strip_suffix('\n').unwrap_or(message);
        let force_new_line = !(body.ends_with(' ') || body.ends_with('\t'));
        if force_new_line && !message.ends_with('\n') {
            buffer.push('\n');
        }
        self.stdout(buffer);
        Ok(())
    }

    /// `say_error(message)` of these commands: `say message, :red; exit 1`.
    fn say_error<T>(&mut self, message: &str) -> Step<T> {
        self.say(message, Color::Red)?;
        Err(Stop::Exit(1))
    }

    /// `Kernel#puts`.
    fn puts(&mut self, message: &str) {
        let mut text = message.to_owned();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.stdout(text);
    }

    /// `Kernel#warn`.
    fn warn(&mut self, message: &str) {
        let mut text = message.to_owned();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.stderr(text);
    }

    fn finish(mut self, result: Step<i32>) -> Result<CommandOutcome, AuthorError> {
        let exit_code = match result {
            Ok(code) => code,
            Err(Stop::Exit(code)) => code,
            Err(Stop::Raise { class, message }) => {
                // FriendlyCLI#format_error_message for a non-Thor error.
                let text = format!("[{class}] {message}");
                let painted = if self.colors {
                    format!("\x1b[31m{text}\x1b[0m")
                } else {
                    text.clone()
                };
                let body = text.strip_suffix('\n').unwrap_or(&text);
                let mut line = painted;
                if !(body.ends_with(' ') || body.ends_with('\t') || text.ends_with('\n')) {
                    line.push('\n');
                }
                self.stderr(line);
                1
            }
            Err(Stop::Error(kind)) => {
                let partial = self.writes;
                return Err(match kind {
                    AuthorErrorKind::Register => AuthorError::RegisterUnavailable { partial },
                    AuthorErrorKind::Gated(feature) => AuthorError::Gated { feature, partial },
                    AuthorErrorKind::Unsupported(detail) => {
                        AuthorError::Unsupported { detail, partial }
                    }
                });
            }
        };
        Ok(CommandOutcome {
            writes: self.writes,
            exit_code,
        })
    }

    /// A Thor option error (`Thor::MalformattedArgumentError`) under
    /// FriendlyCLI: the message on stderr in red, exit status 1.
    fn thor_error<T>(&mut self, message: &str) -> Step<T> {
        let text = if self.colors {
            format!("\x1b[31m{message}\x1b[0m\n")
        } else {
            format!("{message}\n")
        };
        self.stderr(text);
        Err(Stop::Exit(1))
    }
}

/// Thor's `enum` check: `Expected '--name' to be one of a, b; got x`.
fn check_enum(out: &mut Out, name: &str, value: &str, allowed: &[&str]) -> Step {
    if allowed.contains(&value) {
        return Ok(());
    }
    out.thor_error(&format!(
        "Expected '--{name}' to be one of {}; got {value}",
        allowed.join(", ")
    ))
}

fn unsupported(detail: impl Into<String>) -> Stop {
    Stop::Error(AuthorErrorKind::Unsupported(detail.into()))
}

fn schema_stop(e: SchemaError) -> Stop {
    match e.ruby_class() {
        Some(class) => raise(&class, e.message()),
        None => unsupported(e.message()),
    }
}

fn lint_stop(e: LintError) -> Stop {
    match e {
        LintError::Raised { class, message } => raise(&class, message),
        LintError::Unsupported { .. } => unsupported(e.message()),
    }
}

fn definition_stop(e: DefinitionError) -> Stop {
    raise(e.ruby_class(), e.message)
}

/// Ruby `Errno` raise of `File.read(path)`.
fn read_error(e: &std::io::Error, path: &str) -> Stop {
    match e.kind() {
        std::io::ErrorKind::IsADirectory => raise(
            "Errno::EISDIR",
            format!("Is a directory @ io_fread - {path}"),
        ),
        std::io::ErrorKind::NotADirectory => raise(
            "Errno::ENOTDIR",
            format!("Not a directory @ rb_sysopen - {path}"),
        ),
        std::io::ErrorKind::PermissionDenied => raise(
            "Errno::EACCES",
            format!("Permission denied @ rb_sysopen - {path}"),
        ),
        _ => unsupported(format!("reading {path}: {e}")),
    }
}

/// Ruby `File.join(a, b)` for two non-empty segments.
fn join(a: &str, b: &str) -> String {
    let a = a.trim_end_matches('/');
    let b = b.trim_start_matches('/');
    format!("{a}/{b}")
}

fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Names of non-hidden entries of `dir`, byte-sorted (Ruby `Dir.glob` order).
fn sorted_names(dir: &str) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names
}

/// `Dir.glob(File.join(dir, *dirs, '*.yaml'))` with `depth` directory
/// wildcards (`*`) before the file pattern, sorted.
fn glob_yaml(dir: &str, depth: usize) -> Vec<String> {
    let mut paths = vec![dir.to_owned()];
    for _ in 0..depth {
        paths = paths
            .iter()
            .flat_map(|p| {
                sorted_names(p)
                    .into_iter()
                    .map(move |n| join(p, &n))
                    .filter(|c| is_dir(c))
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    let mut out: Vec<String> = paths
        .iter()
        .flat_map(|p| {
            sorted_names(p)
                .into_iter()
                .filter(|n| n.ends_with(".yaml"))
                .map(move |n| join(p, &n))
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    out
}

// ------------------------------------------------------------------ validate

/// `ukiryu validate` class options (raw strings as given).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidateOptions {
    pub verbose: bool,
    /// `--format` (`text` when absent; must be `text` or `json`).
    pub format: Option<String>,
    pub schema: Option<String>,
    pub strict: bool,
    pub executable: bool,
    pub all: bool,
    /// `--register`: the register `Tool.get` loads tools from in the
    /// executable tests (`register_path:`). `validate all` still reads the
    /// files from `Register.default.path`.
    pub register: Option<String>,
}

/// The `validate` subcommand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidateTarget {
    File { path: String },
    All,
    String { yaml: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidateInput {
    pub target: ValidateTarget,
    pub options: ValidateOptions,
}

/// `ukiryu validate file|all|string`.
pub fn validate(input: ValidateInput, ctx: &AuthorContext) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    let result = run_validate(&input, ctx, None, &mut out);
    out.finish(result)
}

/// `ukiryu validate file|all|string` with the Tool runtime available, so
/// `--executable` and `--all` run the executable tests.
pub fn validate_with_tools(
    input: ValidateInput,
    ctx: &AuthorContext,
    env: &ToolEnv,
) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    let result = run_validate(&input, ctx, Some(env), &mut out);
    out.finish(result)
}

fn run_validate(
    input: &ValidateInput,
    ctx: &AuthorContext,
    env: Option<&ToolEnv>,
    out: &mut Out,
) -> Step<i32> {
    let o = &input.options;
    let format = o.format.as_deref().unwrap_or("text");
    check_enum(out, "format", format, &["text", "json"])?;
    let schema = o.schema.as_deref();
    match &input.target {
        ValidateTarget::File { path } => {
            let result = validate_file(path, schema, ctx)?;
            output_validation(out, &result, path, format)?;
            if (o.executable || o.all) && result.is_valid() {
                match env {
                    Some(env) => test_executable(out, path, o, ctx, env)?,
                    None => {
                        return Err(Stop::Error(AuthorErrorKind::Gated(
                            "validate --executable/--all executable test (needs a ToolEnv)",
                        )))
                    }
                }
            }
            Ok(validation_exit(&result, o.strict))
        }
        ValidateTarget::String { yaml } => {
            let result = validate_string(yaml, schema, ctx)?;
            output_validation(out, &result, "<string>", format)?;
            Ok(validation_exit(&result, o.strict))
        }
        ValidateTarget::All => {
            validate_all(out, schema, ctx)?;
            if o.all || o.executable {
                match env {
                    Some(env) => test_all_executables(out, o, ctx, env)?,
                    None => {
                        return Err(Stop::Error(AuthorErrorKind::Gated(
                            "validate all --executable/--all executable tests (needs a ToolEnv)",
                        )))
                    }
                }
            }
            Ok(0)
        }
    }
}

fn validation_exit(result: &ValidationResult, strict: bool) -> i32 {
    if result.is_invalid() || (result.has_warnings() && strict) {
        1
    } else {
        0
    }
}

fn output_validation(out: &mut Out, result: &ValidationResult, source: &str, format: &str) -> Step {
    if format == "json" {
        return out.say(&result.to_json(), Color::White);
    }
    out.say(&format!("Validating: {source}"), Color::Cyan)?;
    if result.is_valid() {
        if result.has_warnings() {
            out.say("✓ Valid with warnings", Color::Yellow)?;
            out.say("", Color::Clear)?;
            for w in &result.warnings {
                out.say(&format!("  Warning: {w}"), Color::Yellow)?;
            }
        } else {
            out.say("✓ Valid", Color::Green)?;
        }
    } else {
        out.say("✗ Invalid", Color::Red)?;
        out.say("", Color::Clear)?;
        for e in &result.errors {
            out.say(&format!("  Error: {e}"), Color::Red)?;
        }
        if result.has_warnings() {
            out.say("", Color::Clear)?;
            for w in &result.warnings {
                out.say(&format!("  Warning: {w}"), Color::Yellow)?;
            }
        }
    }
    Ok(())
}

fn validate_all(out: &mut Out, schema: Option<&str>, ctx: &AuthorContext) -> Step {
    let register = ctx
        .default_register
        .clone()
        .ok_or(Stop::Error(AuthorErrorKind::Register))?;
    if !is_dir(&register) {
        return out.say_error(&format!("Register not found: {register}"));
    }
    let tools_dir = join(&register, "tools");
    if !is_dir(&tools_dir) {
        return out.say_error(&format!("Tools directory not found: {tools_dir}"));
    }
    let files: Vec<String> = glob_yaml(&tools_dir, 1)
        .into_iter()
        .filter(|f| !f.ends_with("/index.yaml"))
        .collect();
    out.say(
        &format!("Validating {} tool definitions...", files.len()),
        Color::Cyan,
    )?;
    out.say("", Color::Clear)?;
    let prefix = format!("{tools_dir}/");
    let mut valid = 0;
    let mut invalid = 0;
    for file in &files {
        let relative = file.replacen(&prefix, "", 1);
        let result = validate_file(file, schema, ctx)?;
        let padded = ljust(&relative, 35);
        if result.is_valid() {
            valid += 1;
            let (status, color) = if result.has_warnings() {
                ("✓ VALID (with warnings)", Color::Yellow)
            } else {
                ("✓ VALID", Color::Green)
            };
            out.say(&format!("  {padded} {status}"), color)?;
        } else {
            invalid += 1;
            out.say(&format!("  {padded} ✗ INVALID"), Color::Red)?;
            for e in &result.errors {
                out.say(&format!("    - {e}"), Color::Red)?;
            }
        }
        for w in &result.warnings {
            out.say(&format!("    ⚠ {w}"), Color::Yellow)?;
        }
    }
    out.say("", Color::Clear)?;
    out.say("Validation Summary:", Color::Cyan)?;
    out.say(&format!("  Total:   {}", files.len()), Color::White)?;
    out.say(&format!("  Valid:   {valid}"), Color::Green)?;
    out.say(
        &format!("  Invalid: {invalid}"),
        if invalid == 0 {
            Color::Green
        } else {
            Color::Red
        },
    )?;
    if invalid > 0 {
        return Err(Stop::Exit(1));
    }
    Ok(())
}

/// Ruby `String#ljust(width)` (character count).
fn ljust(text: &str, width: usize) -> String {
    let n = text.chars().count();
    if n >= width {
        text.to_owned()
    } else {
        format!("{text}{}", " ".repeat(width - n))
    }
}

/// `ENV['UKIRYU_SCHEMA_PATH']` as the core validator reads it.
fn validator_env(ctx: &AuthorContext) -> ValidatorEnv {
    ValidatorEnv {
        schema_path: ctx.schema_path_env.clone(),
        remote_schema: ctx.remote_schema.clone(),
    }
}

/// `DefinitionValidator.validate_file(path, schema_path:)`.
fn validate_file(path: &str, schema: Option<&str>, ctx: &AuthorContext) -> Step<ValidationResult> {
    definition_validator::validate_file(path, schema, &validator_env(ctx)).map_err(schema_stop)
}

/// `DefinitionValidator.validate_string(yaml, schema_path:)`.
fn validate_string(
    yaml: &str,
    schema: Option<&str>,
    ctx: &AuthorContext,
) -> Step<ValidationResult> {
    definition_validator::validate_string(yaml, schema, &validator_env(ctx)).map_err(schema_stop)
}

/// Ruby regexp `\s`.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

// ------------------------------------------------------------------ lint

/// `ukiryu lint` class options (raw strings as given).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintOptions {
    pub verbose: bool,
    /// `--format` (`text` when absent; `text` or `json`).
    pub format: Option<String>,
    /// `--severity` (checked against Thor's enum; not used by the command).
    pub severity: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LintTarget {
    File {
        path: String,
    },
    /// `--register` (falls back to `Register.default_register_path`).
    All {
        register: Option<String>,
    },
    Rules,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintInput {
    pub target: LintTarget,
    pub options: LintOptions,
}

/// `ukiryu lint file|all|rules` (`LintCommand`; not registered in the Ruby
/// `cli.rb`, see the handoff notes).
pub fn lint(input: LintInput, ctx: &AuthorContext) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    let result = run_lint(&input, ctx, &mut out);
    out.finish(result)
}

fn run_lint(input: &LintInput, ctx: &AuthorContext, out: &mut Out) -> Step<i32> {
    let format = input.options.format.as_deref().unwrap_or("text");
    check_enum(out, "format", format, &["text", "json"])?;
    if let Some(severity) = input.options.severity.as_deref() {
        check_enum(
            out,
            "severity",
            severity,
            &["error", "warning", "info", "style"],
        )?;
    }
    match &input.target {
        LintTarget::File { path } => {
            let result = DefinitionLinter::lint_file(path, None).map_err(lint_stop)?;
            if format == "json" {
                out.say(&result.to_json(), Color::White)?;
            } else {
                lint_text(out, &result, path)?;
            }
            Ok(if result.has_errors() { 1 } else { 0 })
        }
        LintTarget::All { register } => lint_all(out, register.as_deref(), ctx),
        LintTarget::Rules => {
            lint_rules(out)?;
            Ok(0)
        }
    }
}

fn lint_text(out: &mut Out, result: &LintResult, source: &str) -> Step {
    out.say(&format!("Linting: {source}"), Color::Cyan)?;
    if !result.has_issues() {
        return out.say("\n✓ No issues found", Color::Green);
    }
    out.say(
        &format!("\nFound {} issue(s):", result.count()),
        Color::Yellow,
    )?;
    for (severity, label) in [
        (Severity::Error, "Errors"),
        (Severity::Warning, "Warnings"),
        (Severity::Info, "Info"),
        (Severity::Style, "Style"),
    ] {
        let issues = result.by_severity(&severity);
        if issues.is_empty() {
            continue;
        }
        out.say("", Color::Clear)?;
        let color = if severity == Severity::Error {
            Color::Red
        } else {
            Color::White
        };
        out.say(&format!("{label}:"), color)?;
        for issue in issues {
            out.say(&format!("  {issue}"), Color::White)?;
        }
    }
    Ok(())
}

fn register_path(explicit: Option<&str>, ctx: &AuthorContext) -> Step<String> {
    match explicit {
        Some(p) => Ok(p.to_owned()),
        None => ctx
            .default_register
            .clone()
            .ok_or(Stop::Error(AuthorErrorKind::Register)),
    }
}

fn lint_all(out: &mut Out, register: Option<&str>, ctx: &AuthorContext) -> Step<i32> {
    let register = register_path(register, ctx)?;
    if !is_dir(&register) {
        return out.say_error(&format!("Register not found: {register}"));
    }
    let tools_dir = join(&register, "tools");
    if !is_dir(&tools_dir) {
        return out.say_error(&format!("Tools directory not found: {tools_dir}"));
    }
    let mut results = Vec::new();
    let mut total_issues = 0;
    for file in glob_yaml(&tools_dir, 2) {
        let result = DefinitionLinter::lint_file(&file, None).map_err(lint_stop)?;
        total_issues += result.count();
        results.push((file, result));
    }
    // Target for the oracle's missing LintResult#error_count (ORACLE_DEFECTS).
    let total_errors: usize = results.iter().map(|(_, r)| r.errors().len()).sum();
    let total_warnings: usize = results
        .iter()
        .map(|(_, r)| r.count_by_severity(&Severity::Warning))
        .sum();
    out.say("\nLint Summary:", Color::Cyan)?;
    out.say(&format!("  Files: {}", results.len()), Color::White)?;
    let zero = |n: usize, other: Color| if n == 0 { Color::Green } else { other };
    out.say(
        &format!("  Issues: {total_issues}"),
        zero(total_issues, Color::White),
    )?;
    out.say(
        &format!("  Errors: {total_errors}"),
        zero(total_errors, Color::Red),
    )?;
    out.say(&format!("  Warnings: {total_warnings}"), Color::White)?;
    if total_issues > 0 {
        out.say("", Color::Clear)?;
        out.say("Files with Issues:", Color::Yellow)?;
        for (file, result) in &results {
            if !result.has_issues() {
                continue;
            }
            let status = if result.has_errors() { '✗' } else { '⚠' };
            out.say(
                &format!("  {status} {file} ({} issues)", result.count()),
                Color::White,
            )?;
        }
    }
    Ok(if total_errors > 0 { 1 } else { 0 })
}

fn lint_rules(out: &mut Out) -> Step {
    out.say("Linting Rules:", Color::Cyan)?;
    out.say("", Color::Clear)?;
    let groups: [(&str, &[&str], bool); 4] = [
        (
            "Naming Convention Rules:",
            &[
                "naming_tool_name_format - Tool name format",
                "naming_command_name_format - Command name format",
            ],
            true,
        ),
        (
            "Completeness Rules:",
            &[
                "complete_missing_description - Missing description",
                "complete_missing_homepage - Missing homepage",
                "complete_missing_version_detection - Missing version detection",
            ],
            true,
        ),
        (
            "Security Rules:",
            &[
                "security_suspicious_subcommand - Dangerous shell commands",
                "security_unvalidated_input - Arguments without type validation",
            ],
            true,
        ),
        (
            "Best Practice Rules:",
            &[
                "style_redundant_default_profile - Redundant default profile",
                "complete_missing_platforms - Missing platforms specification",
            ],
            false,
        ),
    ];
    for (title, rules, blank_after) in groups {
        out.say(title, Color::White)?;
        for rule in rules {
            out.say(&format!("  {rule}"), Color::Dim)?;
        }
        if blank_after {
            out.say("", Color::Clear)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ docs

/// `ukiryu docs` class options (raw strings as given).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocsOptions {
    pub verbose: bool,
    /// `--format` (`markdown` when absent; markdown, md, asciidoc, adoc).
    pub format: Option<String>,
    /// `--output` (generate only).
    pub output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocsTarget {
    Generate {
        tool: String,
        register: Option<String>,
    },
    GenerateAll {
        register: Option<String>,
        /// `--output-dir` (`docs` when absent).
        output_dir: Option<String>,
    },
    /// `--port` as given (Thor numeric; `8000` when absent).
    Serve { port: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocsInput {
    pub target: DocsTarget,
    pub options: DocsOptions,
}

/// `ukiryu docs generate|generate-all|serve` (`DocsCommand`; not registered
/// in the Ruby `cli.rb`, see the handoff notes).
pub fn docs(input: DocsInput, ctx: &AuthorContext) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    let result = run_docs(&input, ctx, &mut out);
    out.finish(result)
}

fn run_docs(input: &DocsInput, ctx: &AuthorContext, out: &mut Out) -> Step<i32> {
    let format = input.options.format.as_deref().unwrap_or("markdown");
    check_enum(
        out,
        "format",
        format,
        &["markdown", "md", "asciidoc", "adoc"],
    )?;
    let format = match format {
        "adoc" => "asciidoc",
        "md" => "markdown",
        other => other,
    };
    match &input.target {
        DocsTarget::Generate { tool, register } => {
            docs_generate(out, tool, register.as_deref(), format, &input.options, ctx)?
        }
        DocsTarget::GenerateAll {
            register,
            output_dir,
        } => docs_generate_all(
            out,
            register.as_deref(),
            output_dir.as_deref().unwrap_or("docs"),
            format,
            ctx,
        )?,
        DocsTarget::Serve { port } => {
            let port = match port {
                None => "8000".to_owned(),
                Some(raw) => match thor_numeric(raw) {
                    Some(p) => p,
                    None => {
                        return out.thor_error(&format!(
                            "Expected numeric value for '--port'; got {}",
                            ruby_inspect_str(raw)
                        ))
                    }
                },
            };
            out.say(
                "Documentation server is not yet implemented.",
                Color::Yellow,
            )?;
            out.say("\nFor now, you can use a simple HTTP server:", Color::White)?;
            out.say(&format!("  python3 -m http.server {port}"), Color::Dim)?;
            out.say(&format!("  ruby -run -e httpd . -p {port}"), Color::Dim)?;
        }
    }
    Ok(0)
}

/// Thor `parse_numeric`: `/[-+]?(\d*\.\d+|\d+)/` matching the whole value,
/// then `to_f` (with a `.`) or `to_i`, printed with Ruby `to_s`.
fn thor_numeric(raw: &str) -> Option<String> {
    let body = raw.strip_prefix(['-', '+']).unwrap_or(raw);
    let negative = raw.starts_with('-');
    if body.is_empty() {
        return None;
    }
    if let Some((int, frac)) = body.split_once('.') {
        if frac.is_empty()
            || !int.bytes().all(|c| c.is_ascii_digit())
            || !frac.bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let value: f64 = format!("{}0{int}.{frac}", if negative { "-" } else { "" })
            .parse()
            .ok()?;
        return Some(if value.fract() == 0.0 && value.abs() < 1e16 {
            format!("{value:.1}")
        } else {
            format!("{value}")
        });
    }
    if !body.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits = body.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    Some(if negative && digits != "0" {
        format!("-{digits}")
    } else {
        digits.to_owned()
    })
}

/// Ruby `String#inspect` for plain printable text.
fn ruby_inspect_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '#' => out.push('#'),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The docs commands' definition input: strict `Loader.load_from_file`
/// (as Ruby), then the YAML document the generator reads.
fn load_for_docs(file: &str) -> Step<String> {
    Loader::<ToolDocument>::new()
        .load_from_file(file, ValidationMode::Strict)
        .map_err(definition_stop)?;
    let bytes = std::fs::read(file).map_err(|e| read_error(&e, file))?;
    String::from_utf8(bytes).map_err(|_| unsupported(format!("{file} is not UTF-8")))
}

/// `DocumentationGenerator.generate(definition, format:)` on the definition
/// text; `Ok(Err(message))` is the rescued ArgumentError.
fn generate_docs(text: &str, format: &str) -> Step<Result<String, String>> {
    let definition =
        ukiryu::from_yaml(text).map_err(|e| raise("Psych::SyntaxError", e.to_string()))?;
    match DocumentationGenerator::generate(&definition, format) {
        Ok(docs) => Ok(Ok(docs)),
        Err(e) if e.kind == DocumentationErrorKind::Argument => Ok(Err(e.message)),
        Err(e) => Err(raise(e.ruby_class(), e.message)),
    }
}

fn docs_generate(
    out: &mut Out,
    tool: &str,
    register: Option<&str>,
    format: &str,
    options: &DocsOptions,
    ctx: &AuthorContext,
) -> Step {
    let register = register_path(register, ctx)?;
    let tool_path = join(&join(&register, "tools"), tool);
    if !is_dir(&tool_path) {
        return out.say_error(&format!("Tool not found: {tool}"));
    }
    // Dir.entries order (readdir), then a descending Gem::Version sort.
    let mut entries: Vec<String> = std::fs::read_dir(&tool_path)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.') && is_dir(&join(&tool_path, n)))
                .collect()
        })
        .unwrap_or_default();
    if entries.len() > 1 {
        let mut versions = Vec::with_capacity(entries.len());
        for e in &entries {
            versions.push((
                e.clone(),
                GemVersion::new(e).map_err(|m| raise("ArgumentError", m))?,
            ));
        }
        versions.sort_by(|a, b| b.1.compare(&a.1));
        entries = versions.into_iter().map(|(e, _)| e).collect();
    }
    let Some(latest) = entries.first() else {
        return out.say_error(&format!("No versions found for tool: {tool}"));
    };
    let Some(file) = glob_yaml(&join(&tool_path, latest), 0).into_iter().next() else {
        return out.say_error(&format!("No definition file found for {tool} {latest}"));
    };
    let text = load_for_docs(&file)?;
    let docs = match generate_docs(&text, format)? {
        Ok(d) => d,
        Err(message) => return out.say_error(&format!("Error: {message}")),
    };
    match options.output.as_deref() {
        Some(path) => {
            out.writes.push(OutputWrite::File {
                path: path.to_owned(),
                contents: docs,
            });
            out.say(&format!("✓ Documentation written to: {path}"), Color::Green)
        }
        None => out.say(&docs, Color::White),
    }
}

fn docs_generate_all(
    out: &mut Out,
    register: Option<&str>,
    output_dir: &str,
    format: &str,
    ctx: &AuthorContext,
) -> Step {
    let register = register_path(register, ctx)?;
    let tools_dir = join(&register, "tools");
    if !is_dir(&tools_dir) {
        return out.say_error(&format!("Tools directory not found: {tools_dir}"));
    }
    std::fs::create_dir_all(output_dir)
        .map_err(|e| unsupported(format!("creating {output_dir}: {e}")))?;
    let mut count = 0;
    for file in glob_yaml(&tools_dir, 2) {
        let parts: Vec<&str> = file.split('/').collect();
        let tool = parts[parts.len() - 3];
        let version = parts[parts.len() - 2];
        let text = load_for_docs(&file)?;
        let docs = match generate_docs(&text, format)? {
            Ok(d) => d,
            Err(message) => {
                out.say(
                    &format!("Warning: Could not generate docs for {file}: {message}"),
                    Color::Yellow,
                )?;
                continue;
            }
        };
        let ext = if format == "asciidoc" { "adoc" } else { "md" };
        let output_file = join(output_dir, &format!("{tool}-{version}.{ext}"));
        out.writes.push(OutputWrite::File {
            path: output_file.clone(),
            contents: docs,
        });
        count += 1;
        out.say(&format!("✓ Generated: {output_file}"), Color::Green)?;
    }
    out.say("", Color::Clear)?;
    out.say(
        &format!("Generated {count} documentation file(s) in: {output_dir}"),
        Color::Cyan,
    )
}

/// `Gem::Version` (rubygems 4.0.3) parsing and `<=>`, for the docs version
/// directory sort.
#[derive(Clone, Debug)]
struct GemVersion {
    version: String,
    canonical: Vec<Segment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Num(String),
    Str(String),
}

impl GemVersion {
    fn new(input: &str) -> Result<GemVersion, String> {
        if !gem_correct(input) {
            return Err(format!("Malformed version number string {input}"));
        }
        let stripped = input.trim_matches(|c: char| is_ruby_space(c) || c == '\0');
        let version = if stripped.is_empty() {
            "0".to_owned()
        } else {
            stripped.replace('-', ".pre.")
        };
        let prerelease = version.bytes().any(|c| c.is_ascii_alphabetic());
        let mut canonical = gem_drop_trailing_zeros(&version);
        if prerelease {
            canonical = gem_drop_zeros_before_letters(&canonical);
        }
        Ok(GemVersion {
            canonical: gem_partition(&canonical),
            version,
        })
    }

    fn compare(&self, other: &GemVersion) -> Ordering {
        if self.version == other.version || self.canonical == other.canonical {
            return Ordering::Equal;
        }
        let (l, r) = (&self.canonical, &other.canonical);
        let limit = l.len().min(r.len());
        for i in 0..limit {
            match (&l[i], &r[i]) {
                (a, b) if a == b => continue,
                (Segment::Str(_), Segment::Num(_)) => return Ordering::Less,
                (Segment::Num(_), Segment::Str(_)) => return Ordering::Greater,
                (Segment::Num(a), Segment::Num(b)) => {
                    return a.len().cmp(&b.len()).then_with(|| a.cmp(b))
                }
                (Segment::Str(a), Segment::Str(b)) => return a.as_bytes().cmp(b.as_bytes()),
            }
        }
        let (rest, longer_is_left) = if l.len() > limit {
            (&l[limit..], true)
        } else {
            (&r[limit..], false)
        };
        for s in rest {
            let ord = match s {
                Segment::Str(_) => Ordering::Less,
                Segment::Num(d) if !d.is_empty() => Ordering::Greater,
                _ => continue,
            };
            return if longer_is_left { ord } else { ord.reverse() };
        }
        Ordering::Equal
    }
}

/// `Gem::Version.correct?`.
fn gem_correct(s: &str) -> bool {
    let core = s.trim_matches(is_ruby_space);
    if core.is_empty() {
        return true;
    }
    let b = core.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    while i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_alphanumeric() {
        i += 1;
        while i < b.len() && b[i].is_ascii_alphanumeric() {
            i += 1;
        }
    }
    if i == b.len() {
        return true;
    }
    if b[i] != b'-' {
        return false;
    }
    let pre = |c: u8| c.is_ascii_alphanumeric() || c == b'-';
    i += 1;
    loop {
        let st = i;
        while i < b.len() && pre(b[i]) {
            i += 1;
        }
        if i == st {
            return false;
        }
        if i == b.len() {
            return true;
        }
        if b[i] != b'.' {
            return false;
        }
        i += 1;
    }
}

fn gem_partition(v: &str) -> Vec<Segment> {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let st = i;
        if b[i].is_ascii_digit() {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Segment::Num(v[st..i].trim_start_matches('0').to_owned()));
        } else if b[i].is_ascii_alphabetic() {
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            out.push(Segment::Str(v[st..i].to_owned()));
        } else {
            i += 1;
        }
    }
    out
}

/// `@version.sub(/(?<=[a-zA-Z.])[.0]+\z/, "")`.
fn gem_drop_trailing_zeros(v: &str) -> String {
    let b = v.as_bytes();
    let mut start = b.len();
    while start > 0 && (b[start - 1] == b'.' || b[start - 1] == b'0') {
        start -= 1;
    }
    for p in start..b.len() {
        if p > 0 && (b[p - 1].is_ascii_alphabetic() || b[p - 1] == b'.') {
            return v[..p].to_owned();
        }
    }
    v.to_owned()
}

/// `sub!(/(?<=\.|\A)[0.]+(?=[a-zA-Z])/, "")`.
fn gem_drop_zeros_before_letters(v: &str) -> String {
    let b = v.as_bytes();
    for p in 0..b.len() {
        if !(p == 0 || b[p - 1] == b'.') || !(b[p] == b'0' || b[p] == b'.') {
            continue;
        }
        let mut e = p;
        while e < b.len() && (b[e] == b'0' || b[e] == b'.') {
            e += 1;
        }
        if e < b.len() && b[e].is_ascii_alphabetic() {
            return format!("{}{}", &v[..p], &v[e..]);
        }
    }
    v.to_owned()
}

// ------------------------------------------------------------------ definitions

/// `ukiryu definitions` subcommands (`list` is the default command).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefinitionsInput {
    List {
        verbose: bool,
    },
    Path,
    Add {
        source: String,
        name: Option<String>,
        version: Option<String>,
    },
    Remove {
        tool: String,
        version: Option<String>,
        force: bool,
    },
    Info {
        tool: String,
        version: Option<String>,
    },
}

/// The confirmation of `definitions remove` without `--force`.
pub trait Confirm {
    /// Perform `shown` (the listing and the `Are you sure? [y/N] ` prompt)
    /// now, then read one line from stdin (`$stdin.gets`): `None` at EOF.
    fn ask(&mut self, shown: Vec<OutputWrite>) -> Option<String>;
}

/// `ukiryu definitions list|path|add|remove|info`. Filesystem changes of
/// `add` and `remove` happen inside the handler, in Ruby order.
pub fn definitions(
    input: DefinitionsInput,
    ctx: &AuthorContext,
    confirm: &mut dyn Confirm,
) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    let discovery = Discovery::new(ctx.discovery.clone());
    let result = match &input {
        DefinitionsInput::List { verbose } => {
            definitions_list(&mut out, &discovery, *verbose);
            Ok(0)
        }
        DefinitionsInput::Path => {
            definitions_path(&mut out, &discovery);
            Ok(0)
        }
        DefinitionsInput::Add {
            source,
            name,
            version,
        } => definitions_add(
            &mut out,
            &discovery,
            source,
            name.as_deref(),
            version.as_deref(),
        ),
        DefinitionsInput::Remove {
            tool,
            version,
            force,
        } => definitions_remove(
            &mut out,
            &discovery,
            tool,
            version.as_deref(),
            *force,
            confirm,
        ),
        DefinitionsInput::Info { tool, version } => {
            definitions_info(&mut out, &discovery, tool, version.as_deref())
        }
    };
    out.finish(result)
}

fn definitions_list(out: &mut Out, discovery: &Discovery, verbose: bool) {
    let definitions = discovery.discover();
    if definitions.is_empty() {
        out.puts("No definitions found.");
        out.puts("");
        out.puts("Search paths:");
        for path in discovery.search_paths() {
            out.puts(&format!("  - {path}"));
        }
        return;
    }
    for (tool, defs) in &definitions {
        if verbose {
            out.puts(&format!("{tool}:"));
            for md in defs {
                // `defs.index(metadata).zero?` (DefinitionMetadata#==).
                let first = defs.iter().position(|d| d == md) == Some(0);
                let indicator = if first { '*' } else { ' ' };
                out.puts(&format!(
                    "  {indicator} {} ({})",
                    md.version,
                    md.source_type.as_str()
                ));
                out.puts(&format!("    Path: {}", md.path));
            }
            out.puts("");
        } else {
            let versions: Vec<&str> = defs.iter().map(|d| d.version.as_str()).collect();
            out.puts(&format!(
                "{tool}: {} [{}]",
                versions.join(", "),
                defs[0].source_type.as_str()
            ));
        }
    }
    out.puts("\nLegend: * = highest priority (will be used by default)");
}

fn definitions_path(out: &mut Out, discovery: &Discovery) {
    out.puts("Definition search paths (in priority order):");
    out.puts("");
    for (index, path) in discovery.search_paths().iter().enumerate() {
        let prefix = if index == 0 {
            "(1) [Highest Priority]".to_owned()
        } else {
            format!("({})", index + 1)
        };
        let exists = if is_dir(path) { '✓' } else { '✗' };
        out.puts(&format!("{prefix} {exists} {path}"));
    }
    out.puts("");
    out.puts(&format!(
        "User directory: {}",
        discovery.user_definitions_directory()
    ));
    out.puts(&format!("XDG_DATA_HOME: {}", discovery.xdg_data_home()));
    out.puts(&format!(
        "XDG_DATA_DIRS: {}",
        discovery.xdg_data_dirs().join(":")
    ));
}

/// Ruby `File.expand_path(path)`: `~` via `HOME`, relative paths against the
/// current directory, `.`/`..` removed lexically.
fn expand_path(path: &str, home: Option<&str>) -> String {
    let path: PathBuf = if path == "~" || path.starts_with("~/") {
        PathBuf::from(format!("{}{}", home.unwrap_or(""), &path[1..]))
    } else {
        PathBuf::from(path)
    };
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

fn definitions_add(
    out: &mut Out,
    discovery: &Discovery,
    source: &str,
    name: Option<&str>,
    version: Option<&str>,
) -> Step<i32> {
    let source_path = expand_path(source, discovery.env.home.as_deref());
    if !Path::new(&source_path).exists() {
        out.warn(&format!("Error: Source file not found: {source_path}"));
        return Err(Stop::Exit(1));
    }
    let loaded =
        match Loader::<ToolDocument>::new().load_from_file(&source_path, ValidationMode::Strict) {
            Ok(doc) => doc,
            Err(e)
                if matches!(
                    e.kind,
                    DefinitionErrorKind::Load | DefinitionErrorKind::Validation
                ) =>
            {
                out.warn(&format!("Error: Failed to load definition: {}", e.message));
                return Err(Stop::Exit(1));
            }
            Err(e) => return Err(definition_stop(e)),
        };
    let tool_name = match name {
        Some(n) => n.to_owned(),
        None => loaded.name.clone().unwrap_or_default(),
    };
    let version = match version {
        Some(v) => v.to_owned(),
        None => loaded.version.clone().unwrap_or_else(|| "1.0".to_owned()),
    };
    let tool_dir = join(&discovery.user_definitions_directory(), &tool_name);
    std::fs::create_dir_all(&tool_dir)
        .map_err(|e| unsupported(format!("creating {tool_dir}: {e}")))?;
    let target = join(&tool_dir, &format!("{version}.yaml"));
    if Path::new(&target).exists() {
        out.warn(&format!("Error: Definition already exists: {target}"));
        out.warn("Use --name and --version to specify a different tool/version.");
        return Err(Stop::Exit(1));
    }
    std::fs::copy(&source_path, &target)
        .map_err(|e| unsupported(format!("copying to {target}: {e}")))?;
    out.puts("Definition added successfully:");
    out.puts(&format!("  Tool: {tool_name}"));
    out.puts(&format!("  Version: {version}"));
    out.puts(&format!("  Location: {target}"));
    Ok(0)
}

fn definitions_remove(
    out: &mut Out,
    discovery: &Discovery,
    tool: &str,
    version: Option<&str>,
    force: bool,
    confirm: &mut dyn Confirm,
) -> Step<i32> {
    let user_dir = discovery.user_definitions_directory();
    let tool_dir = join(&user_dir, tool);
    if !is_dir(&tool_dir) {
        out.warn(&format!("Error: No definitions found for tool '{tool}'"));
        out.warn(&format!("User definitions directory: {user_dir}"));
        return Err(Stop::Exit(1));
    }
    let versions: Vec<String> = match version {
        Some(v) => {
            if Path::new(&join(&tool_dir, &format!("{v}.yaml"))).exists() {
                vec![v.to_owned()]
            } else {
                out.warn(&format!("Error: Version {v} not found for tool '{tool}'"));
                return Err(Stop::Exit(1));
            }
        }
        None => glob_yaml(&tool_dir, 0)
            .iter()
            .map(|f| {
                let base = f.rsplit('/').next().unwrap_or(f);
                base.strip_suffix(".yaml").unwrap_or(base).to_owned()
            })
            .collect(),
    };
    if !force {
        out.puts("This will remove the following definitions:");
        for v in &versions {
            out.puts(&format!("  - {tool}/{v}"));
        }
        out.stdout("Are you sure? [y/N] ".to_owned());
        let shown = std::mem::take(&mut out.writes);
        let Some(line) = confirm.ask(shown) else {
            return Err(raise("NoMethodError", "undefined method 'chomp' for nil"));
        };
        let response = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .or_else(|| line.strip_suffix('\r'))
            .unwrap_or(&line);
        if response.to_lowercase() != "y" {
            out.puts("Cancelled.");
            return Ok(0);
        }
    }
    for v in &versions {
        let file = join(&tool_dir, &format!("{v}.yaml"));
        std::fs::remove_file(&file).map_err(|e| unsupported(format!("removing {file}: {e}")))?;
        out.puts(&format!("Removed: {tool}/{v}"));
    }
    let empty = std::fs::read_dir(&tool_dir)
        .map(|mut rd| rd.next().is_none())
        .unwrap_or(false);
    if empty {
        std::fs::remove_dir(&tool_dir)
            .map_err(|e| unsupported(format!("removing {tool_dir}: {e}")))?;
        out.puts(&format!("Removed empty directory: {tool_dir}"));
    }
    out.puts("Done.");
    Ok(0)
}

fn definitions_info(
    out: &mut Out,
    discovery: &Discovery,
    tool: &str,
    version: Option<&str>,
) -> Step<i32> {
    let definitions = discovery.definitions_for(tool);
    if definitions.is_empty() {
        out.warn(&format!("Error: No definitions found for tool '{tool}'"));
        return Err(Stop::Exit(1));
    }
    let metadata: Option<&DefinitionMetadata> = match version {
        Some(v) => definitions.iter().find(|d| d.version == v),
        None => definitions.first(),
    };
    let Some(metadata) = metadata else {
        out.warn(&format!(
            "Error: Version '{}' not found for tool '{tool}'",
            version.unwrap_or("")
        ));
        let versions: Vec<&str> = definitions.iter().map(|d| d.version.as_str()).collect();
        out.warn(&format!("Available versions: {}", versions.join(", ")));
        return Err(Stop::Exit(1));
    };
    out.puts(&format!("Tool: {}", metadata.name));
    out.puts(&format!("Version: {}", metadata.version));
    out.puts(&format!("Source: {}", metadata.source_type.as_str()));
    out.puts(&format!("Path: {}", metadata.path));
    let exists = metadata.exists();
    out.puts(&format!("Exists: {}", if exists { "Yes" } else { "No" }));
    if exists {
        // Time#to_s; the merged helper renders UTC (`+0000`).
        let modified = metadata.mtime().map(ruby_time_to_s).unwrap_or_default();
        out.puts(&format!("Modified: {modified}"));
    }
    if definitions.len() > 1 {
        out.puts("");
        out.puts("Available versions:");
        for defn in &definitions {
            let indicator = if defn.version == metadata.version {
                '*'
            } else {
                ' '
            };
            let note = if defn == &definitions[0] {
                " [default]"
            } else {
                ""
            };
            out.puts(&format!(
                "  {indicator} {} ({}){note}",
                defn.version,
                defn.source_type.as_str()
            ));
        }
    }
    out.puts("");
    match metadata.load_definition(&Loader::<ToolDocument>::new()) {
        Ok(doc) => {
            out.puts("Validation: ✓ Valid");
            if let Some(display_name) = &doc.inner.display_name {
                out.puts(&format!("Display Name: {}", display_name.to_ruby_s()));
            }
            if let Some(homepage) = &doc.inner.homepage {
                out.puts(&format!("Homepage: {}", homepage.to_ruby_s()));
            }
        }
        Err(e)
            if matches!(
                e.kind,
                DefinitionErrorKind::Load | DefinitionErrorKind::Validation
            ) =>
        {
            out.puts("Validation: ✗ Invalid");
            out.puts(&format!("Error: {}", e.message));
        }
        Err(e) => return Err(definition_stop(e)),
    }
    Ok(0)
}

/// `Models::ToolDefinition` as the definition loader's document
/// (`ToolDefinition.from_yaml`, then the reader values the loader checks).
struct ToolDocument {
    inner: ToolDefinition,
    name: Option<String>,
    version: Option<String>,
    ukiryu_schema: Option<String>,
    self_uri: Option<String>,
}

impl DefinitionDocument for ToolDocument {
    fn parse_definition(yaml: &str) -> Result<Self, DocumentParseError> {
        let inner = ToolDefinition::from_yaml(yaml).map_err(|e| match e.kind {
            ModelErrorKind::Syntax | ModelErrorKind::InvalidFormat => {
                DocumentParseError::Format(e.message)
            }
            _ => DocumentParseError::Other(e.message),
        })?;
        let text = |t: &Option<ukiryu::models::value::Text>| t.as_ref().map(|t| t.to_ruby_s());
        Ok(ToolDocument {
            name: text(&inner.name),
            version: text(&inner.version),
            ukiryu_schema: text(&inner.ukiryu_schema),
            self_uri: text(&inner.self_uri),
            inner,
        })
    }

    fn definition_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn definition_version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    fn has_profiles(&self) -> bool {
        self.inner.profiles.as_ref().is_some_and(|p| !p.is_empty())
    }

    fn ukiryu_schema(&self) -> Option<&str> {
        self.ukiryu_schema.as_deref()
    }

    fn self_uri(&self) -> Option<&str> {
        self.self_uri.as_deref()
    }

    fn resolve_inheritance(&mut self) -> ukiryu::definition::Result<()> {
        self.inner.resolve_inheritance().map(|_| ()).map_err(|e| {
            ukiryu::definition::DefinitionError::new(
                ukiryu::definition::DefinitionErrorKind::Load,
                e.message,
            )
        })
    }
}

// ------------------------------------------------------------------ extract

/// `ukiryu extract TOOL` options (raw strings as given).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractInput {
    pub tool: String,
    /// `--output`.
    pub output: Option<String>,
    /// `--method` (`auto` when absent; any other value is passed through as
    /// a Symbol, so unknown names reach the extractor's own error).
    pub method: Option<String>,
    pub verbose: bool,
}

/// `BaseCommand#say(message, color)`.
fn base_say(out: &mut Out, message: &str, code: u8, disabled: bool) {
    if disabled {
        out.puts(message);
    } else {
        out.puts(&format!("\x1b[{code}m{message}\x1b[0m"));
    }
}

/// `ukiryu extract TOOL` (`ExtractCommand#run`). Commands run through
/// `host`; host failures that the source does not rescue are printed the
/// FriendlyCLI way (`[Class] message`, exit 1).
pub fn extract(
    input: ExtractInput,
    ctx: &AuthorContext,
    host: &mut dyn CommandHost,
) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    // FriendlyCLI's error colour follows Thor, not Config.
    let result = run_extract(&input, ctx, host, &mut out);
    out.finish(result)
}

fn run_extract(
    input: &ExtractInput,
    ctx: &AuthorContext,
    host: &mut dyn CommandHost,
    out: &mut Out,
) -> Step<i32> {
    let off = ctx.config_colors_disabled;
    let method_option = input.method.as_deref().unwrap_or("auto");
    let method = match method_option {
        "auto" => ExtractionMethod::Auto,
        "native" => ExtractionMethod::Native,
        "help" => ExtractionMethod::Help,
        other => ExtractionMethod::Other(other.to_owned()),
    };
    let result = extractors::extract(&input.tool, Some(method), host)
        .map_err(|e| raise(e.class_name(), e.message()))?;
    let used = result.method.map(|m| m.as_str()).unwrap_or("");
    if result.success {
        let yaml = result.yaml.unwrap_or_default();
        match input.output.as_deref() {
            Some(path) => {
                out.writes.push(OutputWrite::File {
                    path: path.to_owned(),
                    contents: yaml,
                });
                base_say(out, &format!("Definition extracted to: {path}"), 32, off);
                if input.verbose {
                    base_say(out, &format!("Method: {used}"), 36, off);
                }
            }
            None => {
                out.puts(&yaml);
                if input.verbose {
                    base_say(out, &format!("\n# Extracted using: {used}"), 36, off);
                }
            }
        }
        return Ok(0);
    }
    // `@tool_name` is never assigned in the source, so it interpolates as ''.
    base_say(out, "Failed to extract definition from ''", 31, off);
    let error = result.error.unwrap_or_default();
    base_say(out, &format!("Error: {error}"), 31, off);
    base_say(out, "", 37, off);
    base_say(
        out,
        "The tool may not be installed or may not support extraction.",
        33,
        off,
    );
    base_say(out, "", 37, off);
    base_say(out, "Extraction methods tried:", 33, off);
    if method_option != "auto" {
        base_say(
            out,
            &format!("  - {method_option} (explicitly selected)"),
            37,
            off,
        );
    } else {
        base_say(out, "  - native (try --ukiryu-definition flag)", 37, off);
        base_say(out, "  - help (parse --help output)", 37, off);
    }
    base_say(out, "", 37, off);
    base_say(out, "You can specify a method with --method:", 33, off);
    base_say(out, "  ukiryu extract TOOL --method native", 37, off);
    base_say(out, "  ukiryu extract TOOL --method help", 37, off);
    Err(Stop::Exit(1))
}

// ------------------------------------------------------------------ tools

/// Process facts the Tool-dependent verbs (`describe`, `validate
/// --executable`/`--all`) read, chosen by the caller: the host platform and
/// environment snapshot (executable search `PATH`/`PATHEXT`, shell detection,
/// the executor's shell search path) and the configured shell (`Config.shell`,
/// else detected).
#[derive(Clone, Debug)]
pub struct ToolEnv {
    pub host: Host,
    pub configured_shell: Option<String>,
}

/// Tools of one register, with the tool cache of one invocation.
struct Tools<'a> {
    env: &'a ToolEnv,
    register: String,
    options: ToolOptions,
    cache: ToolCache<Rc<Tool>>,
}

impl<'a> Tools<'a> {
    /// `Runtime.instance` platform and shell with the host's `PATH`.
    fn new(env: &'a ToolEnv, register: String) -> Step<Tools<'a>> {
        let runtime = Runtime::new(
            env.host.clone(),
            env.configured_shell.clone(),
            Clock::default(),
        );
        let options = ToolOptions::from_runtime(&runtime)
            .map_err(|e| raise("Ukiryu::Errors::UnknownShellError", e.to_string()))?;
        let discovery = DiscoveryContext::new(
            options.platform,
            options.shell.as_str(),
            env.host.env.get("PATH"),
            env.host.env.get("PATHEXT"),
        );
        Ok(Tools {
            env,
            register,
            options: options.discovery(discovery),
            cache: ToolCache::new(),
        })
    }

    fn source(&self) -> RegisterSource<ProcessHost> {
        RegisterSource::new(
            Register::at(&self.register),
            self.env.host.platform,
            ProcessHost::new(self.env.host.env.clone(), self.options.shell),
        )
    }

    /// `Tool.get(name)`.
    fn get(&self, name: &str) -> Result<Rc<Tool>, ToolError> {
        tool::get(&self.source(), &self.cache, name, &self.options)
    }

    /// `Tool.find_by(name)` (`ToolFinder.find_by` with no options).
    ///
    /// `ToolIndex.instance` is built only when the exact name is not found,
    /// as in the source, and a tool directory it fails to scan prints the
    /// source's `warn` line (`Warning: Failed to parse TOOL: MESSAGE`).
    fn find_by(&self, name: &str, out: &mut Out) -> Step<Option<Rc<Tool>>> {
        match self.get(name) {
            Ok(tool) => return Ok(Some(tool)),
            Err(ToolError::NotFound { .. } | ToolError::ProfileNotFound { .. }) => {}
            Err(other) => return Err(tool_stop(other)),
        }
        let register = Register::at(&self.register);
        let index = register_lookup::load_tool_index(&register);
        for warning in index.warnings() {
            out.warn(&format!(
                "Warning: Failed to parse {}: {}",
                warning.tool, warning.fault.message
            ));
        }
        register_lookup::find_by(
            &register,
            &index,
            &self.source(),
            &self.cache,
            name,
            &self.options,
            &self.options,
        )
        .map_err(lookup_stop)
    }

    /// `tool.detect_version`.
    fn detect(&self, tool: &Tool) -> Step<Option<DetectedVersion>> {
        let mut runner = ProcessHost::new(self.env.host.env.clone(), tool.shell());
        let mut man = ManPageParser::with_clock(FileSystem, Clock::default());
        tool.detect_version(&mut runner, &mut man)
            .map_err(detection_stop)
    }

    /// `tool.version`: the option, the definition's version, else detection.
    fn version(&self, tool: &Tool) -> Step<Option<String>> {
        let mut runner = ProcessHost::new(self.env.host.env.clone(), tool.shell());
        let mut man = ManPageParser::with_clock(FileSystem, Clock::default());
        tool.version_or_detect(&mut runner, &mut man)
            .map_err(detection_stop)
    }
}

fn detection_stop(e: DetectionError) -> Stop {
    match e.ruby_class() {
        Some(class) => raise(class, e.to_string()),
        None => unsupported(e.to_string()),
    }
}

/// A tool failure as the Ruby exception the source would let escape.
fn tool_stop(e: ToolError) -> Stop {
    match e {
        ToolError::NotFound { .. } => raise("Ukiryu::Errors::ToolNotFoundError", e.to_string()),
        ToolError::ProfileNotFound { message } => {
            raise("Ukiryu::Errors::ProfileNotFoundError", message)
        }
        ToolError::Ruby { class, message } => raise(class, message),
        other => unsupported(other.to_string()),
    }
}

/// A `ToolFinder.find_by` failure as the Ruby exception it lets escape.
fn lookup_stop(e: LookupError) -> Stop {
    match e {
        LookupError::Tool(e) => tool_stop(e),
        LookupError::UnrepresentableTarget { .. } => unsupported(e.to_string()),
        other => raise(&other.class_name(), other.to_string()),
    }
}

/// An executor failure the source's `rescue StandardError` reports by message.
fn process_stop(e: ProcessError) -> Stop {
    match &e {
        ProcessError::Failed { .. }
        | ProcessError::TimedOut(_)
        | ProcessError::ShellNotFound { .. }
        | ProcessError::InvalidByteSequence { .. } => {
            raise("Ukiryu::Errors::ExecutionError", e.to_string())
        }
        _ => unsupported(e.to_string()),
    }
}

/// `"#{info}"` of a detected version (`VersionInfo#to_s`).
fn detected_to_s(info: &DetectedVersion) -> Step<String> {
    version_info_to_s(info).ok_or_else(|| {
        unsupported("VersionInfo#to_s is nil: Ruby interpolates the default object description")
    })
}

/// The receiver part of a Ruby 3.4+ `NoMethodError` message for a YAML value.
fn data_receiver(v: Option<&Data>) -> String {
    match v {
        None | Some(Data::Null) => "nil".to_owned(),
        Some(Data::Bool(b)) => b.to_string(),
        Some(Data::Integer(_)) => "an instance of Integer".to_owned(),
        Some(Data::Float(_)) => "an instance of Float".to_owned(),
        Some(Data::String(_)) => "an instance of String".to_owned(),
        Some(Data::Symbol(_)) => "an instance of Symbol".to_owned(),
        Some(Data::Date { .. }) => "an instance of Date".to_owned(),
        Some(Data::Time(_)) => "an instance of Time".to_owned(),
        Some(Data::Array(_)) => "an instance of Array".to_owned(),
        Some(Data::Hash(_)) => "an instance of Hash".to_owned(),
    }
}

/// The receiver part of a `NoMethodError` message for a model value.
fn value_receiver(v: &Value) -> String {
    match v {
        Value::Nil => "nil".to_owned(),
        Value::Bool(b) => b.to_string(),
        other => format!("an instance of {}", other.class_name()),
    }
}

fn lookup(pairs: &[(Data, Data)], key: Data) -> Option<&Data> {
    pairs.iter().find(|(k, _)| k.eql(&key)).map(|(_, v)| v)
}

/// `data[:key] || data['key']`: the Symbol key when truthy, else the String
/// key's value (which may be falsy; a missing key is `nil`, i.e. `None`).
fn pick<'a>(pairs: &'a [(Data, Data)], key: &str) -> Option<&'a Data> {
    lookup(pairs, Data::Symbol(key.to_owned()))
        .filter(|v| v.truthy())
        .or_else(|| lookup(pairs, Data::String(key.to_owned())))
}

/// `"#{value}"` of a YAML value (`nil` is empty).
fn data_to_s(v: Option<&Data>) -> Step<String> {
    match v {
        None => Ok(String::new()),
        Some(d) => d.ruby_to_s().map_err(schema_stop),
    }
}

/// Ruby `File.dirname` for the paths the validate verbs take.
fn ruby_dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.starts_with('/') { "/" } else { "." }.to_owned();
    }
    match trimmed.rfind('/') {
        None => ".".to_owned(),
        Some(i) => {
            let dir = trimmed[..i].trim_end_matches('/');
            if dir.is_empty() {
                "/".to_owned()
            } else {
                dir.to_owned()
            }
        }
    }
}

/// Ruby `File.basename`.
fn ruby_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.starts_with('/') { "/" } else { "" }.to_owned();
    }
    trimmed.rsplit('/').next().unwrap_or_default().to_owned()
}

/// Ruby `File.basename(path, '.yaml')`.
fn ruby_basename_yaml(path: &str) -> String {
    let base = ruby_basename(path);
    match base.strip_suffix(".yaml") {
        Some(stem) if !stem.is_empty() => stem.to_owned(),
        _ => base,
    }
}

/// The tool name the executable tests derive from a definition path: the
/// directory under `tools/`, else the file name.
fn tool_name_for(path: &str) -> String {
    let tool_dir = ruby_basename(&ruby_dirname(path));
    let tools_dir = ruby_basename(&ruby_dirname(&ruby_dirname(path)));
    if tools_dir == "tools" {
        tool_dir
    } else {
        ruby_basename_yaml(path)
    }
}

/// `YAML.safe_load(File.read(path), permitted_classes: [Symbol, Date, Time])`.
fn load_document(path: &str) -> Step<Data> {
    let bytes = std::fs::read(path).map_err(|e| read_error(&e, path))?;
    let text =
        String::from_utf8(bytes).map_err(|_| unsupported(format!("{path} is not valid UTF-8")))?;
    safe_load(&text, Permitted::DOCUMENT).map_err(schema_stop)
}

// ------------------------------------------------------------------ validate --executable

/// `ValidateCommand#test_executable`.
fn test_executable(
    out: &mut Out,
    path: &str,
    o: &ValidateOptions,
    ctx: &AuthorContext,
    env: &ToolEnv,
) -> Step {
    out.say("", Color::Clear)?;
    out.say("Testing executable...", Color::Cyan)?;
    match executable_body(out, path, o, ctx, env) {
        Err(Stop::Raise { class, message }) => {
            if class == "Ukiryu::Errors::ToolNotFoundError" {
                out.say(&format!("✗ Tool not found: {message}"), Color::Red)?;
                return Err(Stop::Exit(1));
            }
            out.say(&format!("✗ Executable test failed: {message}"), Color::Red)?;
            if o.strict {
                return Err(Stop::Exit(1));
            }
            Ok(())
        }
        other => other,
    }
}

fn executable_body(
    out: &mut Out,
    path: &str,
    o: &ValidateOptions,
    ctx: &AuthorContext,
    env: &ToolEnv,
) -> Step {
    let profile_data = load_document(path)?;
    let Data::Hash(pairs) = &profile_data else {
        return Err(unsupported(
            "a definition that is not a mapping reaches the executable test",
        ));
    };
    // `Tool.get(tool_name, register_path: options[:register])`: `--register`
    // names the register the tool is loaded from, else the default one.
    let register = o
        .register
        .clone()
        .or_else(|| ctx.default_register.clone())
        .ok_or(Stop::Error(AuthorErrorKind::Register))?;
    let session = Tools::new(env, register)?;
    let tool = session.get(&tool_name_for(path)).map_err(tool_stop)?;
    match tool.executable() {
        Some(exe) => out.say(&format!("✓ Tool found at: {exe}"), Color::Green)?,
        None => {
            out.say("✗ Tool not found", Color::Red)?;
            out.say("  Searched in: system PATH", Color::Dim)?;
            return Err(Stop::Exit(1));
        }
    }
    if tool.definition().version_detection.is_some() {
        out.say("", Color::Clear)?;
        out.say("Testing version detection...", Color::Cyan)?;
        match session.detect(&tool) {
            Ok(Some(info)) => out.say(
                &format!("✓ Version detected: {}", detected_to_s(&info)?),
                Color::Green,
            )?,
            Ok(None) => out.say(
                "⚠ Version detection failed - could not extract version",
                Color::Yellow,
            )?,
            Err(Stop::Raise { message, .. }) => {
                out.say(&format!("⚠ Version check failed: {message}"), Color::Yellow)?
            }
            Err(other) => return Err(other),
        }
    }
    let smoke = pick(pairs, "smoke_tests").filter(|v| v.truthy());
    let tests: Option<&Vec<Data>> = match smoke {
        Some(Data::Array(items)) if !items.is_empty() => Some(items),
        Some(Data::Array(_)) | None => None,
        Some(Data::Hash(p)) if p.is_empty() => None,
        Some(Data::String(s)) if s.is_empty() => None,
        Some(_) => {
            return Err(unsupported(
                "smoke_tests that is not a list: Ruby iterates it as that value",
            ))
        }
    };
    if let Some(tests) = tests {
        out.say("", Color::Clear)?;
        out.say(
            &format!("Running {} smoke test(s)...", tests.len()),
            Color::Cyan,
        )?;
        return run_smoke_tests(out, o, &session, &tool, tests);
    }
    let profiles = tool.definition().profiles.as_deref().unwrap_or_default();
    if let Some(commands) = profiles
        .first()
        .and_then(Option::as_ref)
        .and_then(|p| p.commands.as_deref())
    {
        if let Some(first) = commands.first().and_then(Option::as_ref) {
            out.say("", Color::Clear)?;
            out.say("Testing command execution (smoke test)...", Color::Cyan)?;
            let name = first.name.as_ref().map(Text::to_ruby_s).unwrap_or_default();
            // `execute_smoke_test` calls `tool.execute(cmd_name, {})`, but
            // `Tool#execute` takes `(root_path, execution_timeout:, **params)`:
            // the positional hash is a second argument, so every call raises
            // this ArgumentError before anything runs, and the rescue reports it.
            out.say(
                &format!(
                    "⚠ Command '{name}' test error: wrong number of arguments (given 2, expected 1; required keyword: execution_timeout)"
                ),
                Color::Yellow,
            )?;
        }
    }
    Ok(())
}

/// `ValidateCommand#run_smoke_tests`.
fn run_smoke_tests(
    out: &mut Out,
    o: &ValidateOptions,
    session: &Tools<'_>,
    tool: &Tool,
    tests: &[Data],
) -> Step {
    let platform = session.env.host.platform.as_str();
    for (index, test) in tests.iter().enumerate() {
        let Data::Hash(pairs) = test else {
            return Err(unsupported("a smoke test that is not a mapping"));
        };
        let name = pick(pairs, "name");
        let description = pick(pairs, "description").filter(|v| v.truthy()).or(name);
        out.say("", Color::Clear)?;
        out.say(
            &format!(
                "[{}/{}] {}: {}",
                index + 1,
                tests.len(),
                data_to_s(name)?,
                data_to_s(description)?
            ),
            Color::Cyan,
        )?;
        if let Some(platforms) = pick(pairs, "platforms").filter(|v| v.truthy()) {
            let included = match platforms {
                Data::Array(items) => items
                    .iter()
                    .any(|i| i.ruby_eq(&Data::String(platform.to_owned()))),
                Data::String(s) => s.contains(platform),
                _ => return Err(unsupported("platforms that is neither a list nor a string")),
            };
            if !included {
                out.say(
                    &format!("  ⊘ Skipped (not for this platform: {platform})"),
                    Color::Dim,
                )?;
                continue;
            }
        }
        // `skip_if` is evaluated by `evaluate_condition`, which always
        // answers false in the source: no test is ever skipped by it.
        let timeout =
            match pick(pairs, "timeout").filter(|v| v.truthy()) {
                Some(t) => t,
                // `|| tool.profile.timeout`: ToolDefinition has no such reader.
                None => return Err(raise(
                    "NoMethodError",
                    "undefined method 'timeout' for an instance of Ukiryu::Models::ToolDefinition",
                )),
            };
        match smoke_test(
            out,
            o,
            session,
            tool,
            pairs,
            pick(pairs, "command"),
            timeout,
        ) {
            Err(Stop::Raise { message, .. }) => {
                out.say(&format!("  ✗ ERROR: {message}"), Color::Red)?;
                if o.strict {
                    return Err(Stop::Exit(1));
                }
            }
            other => other?,
        }
    }
    Ok(())
}

/// The `begin` body of one smoke test: run the command, check the result,
/// report it.
fn smoke_test(
    out: &mut Out,
    o: &ValidateOptions,
    session: &Tools<'_>,
    tool: &Tool,
    test: &[(Data, Data)],
    command: Option<&Data>,
    timeout: &Data,
) -> Step {
    let arguments = match command {
        Some(Data::Array(items)) => items
            .iter()
            .map(|i| match i {
                Data::String(s) => Ok(s.clone()),
                _ => Err(unsupported(
                    "a smoke test command element that is not a string",
                )),
            })
            .collect::<Step<Vec<String>>>()?,
        // `test_command.shellsplit`: `shellwords` is never loaded.
        other => {
            return Err(raise(
                "NoMethodError",
                format!("undefined method 'shellsplit' for {}", data_receiver(other)),
            ))
        }
    };
    let seconds = match timeout {
        Data::Integer(n) => i64::try_from(n).ok().filter(|v| *v > 0).map(Seconds::Int),
        Data::Float(f) if f.is_finite() && *f > 0.0 => Some(Seconds::Float(*f)),
        _ => None,
    }
    .ok_or_else(|| unsupported("a smoke test timeout that is not a positive number"))?;
    let mut request = ExecuteRequest::new(
        tool.executable().unwrap_or_default(),
        arguments,
        session.env.host.env.clone(),
        tool.shell(),
        seconds,
    )
    .tool_name(tool.name().unwrap_or_default());
    if let Some(path) = session.env.host.env.get("PATH") {
        request = request.shell_search_path(path);
    }
    let execution = executor::execute(request).map_err(process_stop)?;
    let (errors, details) = check_expectations(
        test,
        execution.result.exit_status(),
        &execution.stdout,
        &execution.stderr,
    )?;
    if errors.is_empty() {
        out.say("  ✓ PASSED", Color::Green)?;
        if o.verbose {
            for detail in &details {
                out.say(&format!("    {detail}"), Color::Dim)?;
            }
        }
    } else {
        out.say("  ✗ FAILED", Color::Red)?;
        for error in &errors {
            out.say(&format!("    ✗ {error}"), Color::Red)?;
        }
        if o.strict {
            return Err(Stop::Exit(1));
        }
    }
    Ok(())
}

fn utf8(bytes: &[u8]) -> Step<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| unsupported("a smoke test stream that is not valid UTF-8"))
}

/// `ValidateCommand#validate_test_result`: `(errors, details)`.
fn check_expectations(
    test: &[(Data, Data)],
    status: i64,
    stdout: &[u8],
    stderr: &[u8],
) -> Step<(Vec<String>, Vec<String>)> {
    let (stdout, stderr) = (utf8(stdout)?, utf8(stderr)?);
    let mut errors = Vec::new();
    let mut details = Vec::new();
    let expect: &[(Data, Data)] = match pick(test, "expect").filter(|v| v.truthy()) {
        None => &[],
        Some(Data::Hash(p)) => p,
        Some(_) => return Err(unsupported("expect that is not a mapping")),
    };
    let (matched, shown) = match pick(expect, "exit_code").filter(|v| v.truthy()) {
        None => (status == 0, "0".to_owned()),
        Some(Data::Integer(n)) => (
            i64::try_from(n).map(|v| v == status).unwrap_or(false),
            n.to_string(),
        ),
        Some(Data::Float(f)) => (*f == status as f64, data_to_s(Some(&Data::Float(*f)))?),
        Some(other) => (false, data_to_s(Some(other))?),
    };
    if matched {
        details.push(format!("Exit code: {status} (as expected)"));
    } else {
        errors.push(format!(
            "Exit code mismatch: expected {shown}, got {status}"
        ));
    }
    pattern_check(
        expect,
        "output_match",
        "Output",
        stdout,
        &mut errors,
        &mut details,
    )?;
    contains_check(expect, stdout, &mut errors, &mut details)?;
    pattern_check(
        expect,
        "stderr_match",
        "Stderr",
        stderr,
        &mut errors,
        &mut details,
    )?;
    Ok((errors, details))
}

/// `Regexp.new(expect[key])` against a stream.
fn pattern_check(
    expect: &[(Data, Data)],
    key: &str,
    label: &str,
    stream: &str,
    errors: &mut Vec<String>,
    details: &mut Vec<String>,
) -> Step {
    match pick(expect, key).filter(|v| v.truthy()) {
        None => Ok(()),
        Some(Data::String(source)) => {
            let pattern = RubyPattern::new(source).map_err(|e| unsupported(e.to_string()))?;
            if pattern
                .is_match(stream)
                .map_err(|e| unsupported(e.to_string()))?
            {
                details.push(format!("{label} matches pattern: {source}"));
            } else {
                errors.push(format!("{label} does not match pattern: {source}"));
            }
            Ok(())
        }
        Some(_) => Err(unsupported("a match pattern that is not a string")),
    }
}

/// `expect[:output_contains]`: every string must be in stdout.
fn contains_check(
    expect: &[(Data, Data)],
    stdout: &str,
    errors: &mut Vec<String>,
    details: &mut Vec<String>,
) -> Step {
    match pick(expect, "output_contains").filter(|v| v.truthy()) {
        None => Ok(()),
        Some(Data::Array(items)) => {
            for item in items {
                let Data::String(s) = item else {
                    return Err(unsupported("an output_contains entry that is not a string"));
                };
                if stdout.contains(s.as_str()) {
                    details.push(format!("Output contains: {s}"));
                } else {
                    errors.push(format!("Output missing string: {s}"));
                }
            }
            Ok(())
        }
        Some(_) => Err(unsupported("output_contains that is not a list")),
    }
}

/// `ValidateCommand#test_all_executables`.
fn test_all_executables(
    out: &mut Out,
    o: &ValidateOptions,
    ctx: &AuthorContext,
    env: &ToolEnv,
) -> Step {
    let register = ctx
        .default_register
        .clone()
        .ok_or(Stop::Error(AuthorErrorKind::Register))?;
    if !is_dir(&register) {
        return out.say_error(&format!("Register not found: {register}"));
    }
    let tools_dir = join(&register, "tools");
    if !is_dir(&tools_dir) {
        return out.say_error(&format!("Tools directory not found: {tools_dir}"));
    }
    let files: Vec<String> = glob_yaml(&tools_dir, 1)
        .into_iter()
        .filter(|f| !f.ends_with("/index.yaml"))
        .collect();
    out.say("", Color::Clear)?;
    out.say(
        &format!("Testing {} tool executable(s)...", files.len()),
        Color::Cyan,
    )?;
    // The files come from the default register; `Tool.get` takes `--register`.
    let tools_register = o.register.clone().unwrap_or(register);
    let session = Tools::new(env, tools_register).map_err(|e| match e {
        // Ruby raises this inside each row's `rescue StandardError`.
        Stop::Raise { class, message } => unsupported(format!("{class}: {message}")),
        other => other,
    })?;
    let prefix = format!("{tools_dir}/");
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for file in &files {
        let relative = file.replacen(&prefix, "", 1);
        let padded = ljust(&relative, 35);
        match executable_row(out, &session, file, &padded) {
            Ok(Row::Passed) => passed += 1,
            Ok(Row::Skipped) => skipped += 1,
            Err(Stop::Raise { class, message }) => {
                if class == "Ukiryu::Errors::ToolNotFoundError" {
                    out.say(&format!("  {padded} ✗ (tool not found)"), Color::Yellow)?;
                    skipped += 1;
                } else {
                    out.say(&format!("  {padded} ✗ Error: {message}"), Color::Red)?;
                    if o.strict {
                        failed += 1;
                    }
                }
            }
            Err(other) => return Err(other),
        }
    }
    out.say("", Color::Clear)?;
    out.say("Executable Test Summary:", Color::Cyan)?;
    out.say(&format!("  Total:   {}", files.len()), Color::White)?;
    out.say(&format!("  Passed:  {passed}"), Color::Green)?;
    out.say(&format!("  Skipped: {skipped}"), Color::Yellow)?;
    out.say(
        &format!("  Failed:  {failed}"),
        if failed > 0 { Color::Red } else { Color::Green },
    )?;
    if failed > 0 && o.strict {
        return Err(Stop::Exit(1));
    }
    Ok(())
}

enum Row {
    Passed,
    Skipped,
}

/// The `begin` body of one `test_all_executables` row.
fn executable_row(out: &mut Out, session: &Tools<'_>, file: &str, padded: &str) -> Step<Row> {
    load_document(file)?;
    let tool = session.get(&tool_name_for(file)).map_err(tool_stop)?;
    if tool.executable().is_none() {
        out.say(&format!("  {padded} ⊘ (not installed)"), Color::Dim)?;
        return Ok(Row::Skipped);
    }
    if tool.definition().version_detection.is_none() {
        out.say(
            &format!("  {padded} ✓ (no version detection)"),
            Color::Green,
        )?;
        return Ok(Row::Passed);
    }
    match session.detect(&tool) {
        Ok(detected) => {
            let shown = match &detected {
                Some(info) => detected_to_s(info)?,
                None => String::new(),
            };
            out.say(&format!("  {padded} ✓ (v{shown})"), Color::Green)?;
            Ok(Row::Passed)
        }
        Err(Stop::Raise { message, .. }) => {
            out.say(
                &format!("  {padded} ⚠ Version detection failed: {message}"),
                Color::Yellow,
            )?;
            Ok(Row::Skipped)
        }
        Err(other) => Err(other),
    }
}

// ------------------------------------------------------------------ describe

/// `ukiryu describe TOOL [COMMAND]` options, as given. `--format` is accepted
/// by the source but never read by `describe`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescribeInput {
    pub tool: String,
    pub command: Option<String>,
    /// `--register` / `-r`.
    pub register: Option<String>,
}

/// `ukiryu describe TOOL [COMMAND]` (`DescribeCommand#run`).
pub fn describe(
    input: DescribeInput,
    ctx: &AuthorContext,
    env: &ToolEnv,
) -> Result<CommandOutcome, AuthorError> {
    let mut out = Out::new(ctx);
    let result = run_describe(&input, ctx, env, &mut out);
    out.finish(result)
}

const RULE: &str = "============================================================";

/// Thor-free `BaseCommand#say` colour codes (`dim` is 2, `clear` is not in
/// the table and falls back to white).
const CYAN: u8 = 36;
const WHITE: u8 = 37;
const GREEN: u8 = 32;
const RED: u8 = 31;
const YELLOW: u8 = 33;
const DIM: u8 = 2;

fn run_describe(
    input: &DescribeInput,
    ctx: &AuthorContext,
    env: &ToolEnv,
    out: &mut Out,
) -> Step<i32> {
    let off = ctx.config_colors_disabled;
    // `setup_register`: `--register` when that directory exists, else the
    // default register.
    let register = input
        .register
        .as_deref()
        .filter(|p| !p.is_empty() && is_dir(p))
        .map(str::to_owned)
        .or_else(|| ctx.default_register.clone())
        .ok_or(Stop::Error(AuthorErrorKind::Register))?;
    let session = Tools::new(env, register.clone())?;
    let Some(tool) = session.find_by(&input.tool, out)? else {
        // `error!`: `Register.tools` of the register `setup_register` chose.
        let tools = Register::at(&register).tool_names();
        return out.thor_error(&format!(
            "Tool not found: {}\nAvailable tools: {}",
            input.tool,
            tools.join(", ")
        ));
    };
    let Some(commands) = tool.command_profile().commands.as_deref() else {
        return out.thor_error(&format!("No commands defined for {}", input.tool));
    };
    let commands: Vec<&CommandDefinition> = commands.iter().flatten().collect();
    match input.command.as_deref() {
        Some("help") => tool_summary(out, off, &session, &tool, &commands, true)?,
        None => tool_summary(out, off, &session, &tool, &commands, false)?,
        Some(name) => {
            let mut found = None;
            for c in &commands {
                let own = c.name.as_ref().map(Text::to_ruby_s).unwrap_or_default();
                if own == name {
                    found = Some(c);
                    break;
                }
                match &c.name {
                    None => {
                        return Err(raise("NoMethodError", "undefined method 'to_sym' for nil"))
                    }
                    Some(Text::List(_)) => {
                        return Err(raise(
                            "NoMethodError",
                            "undefined method 'to_sym' for an instance of Array",
                        ))
                    }
                    Some(_) => {}
                }
            }
            let Some(cmd) = found else {
                let names: Vec<String> = commands
                    .iter()
                    .map(|c| c.name.as_ref().map(Text::to_ruby_s).unwrap_or_default())
                    .collect();
                return out.thor_error(&format!(
                    "Command '{name}' not found for {}\nAvailable commands: {}",
                    input.tool,
                    names.join(", ")
                ));
            };
            describe_command(out, off, &tool, &input.tool, name, cmd)?;
        }
    }
    Ok(0)
}

fn say_c(out: &mut Out, off: bool, message: &str, code: u8) {
    base_say(out, message, code, off);
}

fn texts_joined(list: &[Text], separator: &str) -> String {
    list.iter()
        .map(Text::to_ruby_s)
        .collect::<Vec<_>>()
        .join(separator)
}

fn ints_joined(list: &[Option<ukiryu::models::Attr<i64>>], separator: &str) -> String {
    list.iter()
        .map(|i| i.as_ref().map(|v| v.to_string()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(separator)
}

/// `Array#inspect` of an integer list (`nil` for a missing element).
fn ints_inspect(list: &[Option<ukiryu::models::Attr<i64>>]) -> String {
    let items: Vec<String> = list
        .iter()
        .map(|i| {
            i.as_ref()
                .map_or_else(|| "nil".to_owned(), |v| v.to_ruby_value().inspect())
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/// `list.any?` of a model's string list: some element is neither nil nor false.
fn any_text(list: &Option<Vec<Text>>) -> bool {
    list.as_ref()
        .is_some_and(|l| l.iter().any(|t| !matches!(t, Text::Nil)))
}

fn non_empty<T>(list: &Option<Vec<T>>) -> Option<&Vec<T>> {
    list.as_ref().filter(|l| !l.is_empty())
}

/// `show_tool_help` (`help` as the command) and `describe_tool_overview`.
fn tool_summary(
    out: &mut Out,
    off: bool,
    session: &Tools<'_>,
    tool: &Tool,
    commands: &[&CommandDefinition],
    help: bool,
) -> Step {
    let definition = tool.definition();
    let shown = |t: &Option<Text>, fallback: &str| {
        t.as_ref()
            .map(Text::to_ruby_s)
            .unwrap_or_else(|| fallback.to_owned())
    };
    let name = tool.name().unwrap_or_default();
    say_c(out, off, "", WHITE);
    say_c(out, off, RULE, CYAN);
    say_c(out, off, &format!("Tool: {name}"), CYAN);
    say_c(out, off, RULE, CYAN);
    say_c(out, off, "", WHITE);
    say_c(
        out,
        off,
        &format!("Display Name: {}", shown(&definition.display_name, "N/A")),
        WHITE,
    );
    say_c(
        out,
        off,
        &format!("Version: {}", shown(&definition.version, "N/A")),
        WHITE,
    );
    say_c(
        out,
        off,
        &format!("Homepage: {}", shown(&definition.homepage, "N/A")),
        WHITE,
    );
    if let Some(aliases) = non_empty(&definition.aliases) {
        say_c(
            out,
            off,
            &format!("Aliases: {}", texts_joined(aliases, ", ")),
            WHITE,
        );
    }
    say_c(out, off, "", WHITE);
    match tool.executable() {
        Some(exe) => {
            say_c(out, off, "Status: INSTALLED", GREEN);
            say_c(out, off, &format!("Executable: {exe}"), WHITE);
            let version = session
                .version(tool)?
                .unwrap_or_else(|| "unknown".to_owned());
            say_c(out, off, &format!("Detected Version: {version}"), WHITE);
        }
        None => say_c(out, off, "Status: NOT FOUND", RED),
    }
    say_c(out, off, "", WHITE);
    say_c(out, off, &format!("Commands ({}):", commands.len()), YELLOW);
    for cmd in commands {
        let cmd_name = cmd
            .name
            .as_ref()
            .map(Text::to_ruby_s)
            .unwrap_or_else(|| "unnamed".to_owned());
        let description = shown(&cmd.description, "No description");
        say_c(
            out,
            off,
            &format!("  {} {description}", ljust(&cmd_name, 20)),
            WHITE,
        );
        if help {
            if let Some(usage) = &cmd.usage {
                say_c(out, off, &format!("    Usage: {}", usage.to_ruby_s()), DIM);
            }
        }
        if let Some(sets) = non_empty(&cmd.use_env_vars) {
            say_c(
                out,
                off,
                &format!("    Env Var Sets: {}", texts_joined(sets, ", ")),
                DIM,
            );
        }
    }
    say_c(out, off, "", WHITE);
    if help {
        say_c(
            out,
            off,
            &format!("Usage: ukiryu exec {name} <command> [KEY=VALUE ...]"),
            DIM,
        );
        say_c(out, off, &format!("   or: ukiryu exec {name} help"), DIM);
        say_c(
            out,
            off,
            &format!("   or: ukiryu describe {name} <command>"),
            DIM,
        );
        say_c(out, off, "", WHITE);
        say_c(out, off, "For more information on a specific command:", DIM);
        say_c(out, off, &format!("  ukiryu opts {name} <command>"), DIM);
        say_c(
            out,
            off,
            &format!("  ukiryu describe {name} <command>"),
            DIM,
        );
    } else {
        say_c(
            out,
            off,
            &format!("Use 'ukiryu describe {name} <command>' for detailed command documentation"),
            DIM,
        );
    }
    Ok(())
}

/// `describe_command`.
fn describe_command(
    out: &mut Out,
    off: bool,
    tool: &Tool,
    tool_name: &str,
    command_name: &str,
    cmd: &CommandDefinition,
) -> Step {
    say_c(out, off, "", WHITE);
    say_c(out, off, RULE, CYAN);
    say_c(
        out,
        off,
        &format!("{} {command_name}", tool.name().unwrap_or_default()),
        CYAN,
    );
    say_c(out, off, RULE, CYAN);
    say_c(out, off, "", WHITE);
    if let Some(description) = &cmd.description {
        out.puts(&description.to_ruby_s());
    }
    say_c(out, off, "", WHITE);
    if let Some(usage) = &cmd.usage {
        say_c(out, off, "Usage:", YELLOW);
        say_c(out, off, &format!("  {}", usage.to_ruby_s()), WHITE);
        say_c(out, off, "", WHITE);
    }
    if let Some(subcommand) = &cmd.subcommand {
        say_c(
            out,
            off,
            &format!("Subcommand: {}", subcommand.to_ruby_s()),
            WHITE,
        );
        say_c(out, off, "", WHITE);
    }
    if let Some(sets) = non_empty(&cmd.use_env_vars) {
        say_c(
            out,
            off,
            &format!("Env Var Sets: {}", texts_joined(sets, ", ")),
            WHITE,
        );
        say_c(out, off, "", WHITE);
    }
    if let Some(arguments) = non_empty(&cmd.arguments) {
        say_c(out, off, "Arguments:", YELLOW);
        for arg in arguments {
            let arg = arg
                .as_ref()
                .ok_or_else(|| raise("NoMethodError", "undefined method 'name' for nil"))?;
            let name = arg
                .name
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "unnamed".to_owned());
            let kind = arg
                .r#type
                .get()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "string".to_owned());
            let required = if arg.required.get().copied().unwrap_or(false) {
                "required"
            } else {
                "optional"
            };
            let variadic = if arg.variadic.get().copied().unwrap_or(false) {
                "(variadic)"
            } else {
                ""
            };
            let position = arg
                .position
                .get()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "default".to_owned());
            say_c(
                out,
                off,
                &format!("  {name} ({kind}, {required}{variadic})"),
                WHITE,
            );
            if position != "default" {
                say_c(out, off, &format!("    Position: {position}"), DIM);
            }
            if let Some(description) = &arg.description {
                say_c(
                    out,
                    off,
                    &format!("    Description: {}", description.to_ruby_s()),
                    DIM,
                );
            }
            if arg.min.is_some() || arg.max.is_some() || arg.size.is_some() {
                let mut constraints = Vec::new();
                if let Some(min) = &arg.min {
                    constraints.push(format!("min: {min}"));
                }
                if let Some(max) = &arg.max {
                    constraints.push(format!("max: {max}"));
                }
                if let Some(size) = &arg.size {
                    constraints.push(format!("size: {}", ints_inspect(size)));
                }
                say_c(
                    out,
                    off,
                    &format!("    Constraints: {}", constraints.join(", ")),
                    DIM,
                );
            }
            if let Some(range) = &arg.range {
                say_c(
                    out,
                    off,
                    &format!("    Range: {}", ints_joined(range, "..")),
                    DIM,
                );
            }
            if let Some(values) = &arg.values {
                say_c(
                    out,
                    off,
                    &format!("    Valid values: {}", texts_joined(values, ", ")),
                    DIM,
                );
            }
            say_c(out, off, "", WHITE);
        }
    }
    if let Some(options) = non_empty(&cmd.options) {
        say_c(out, off, "Options:", YELLOW);
        for opt in options {
            let opt = opt
                .as_ref()
                .ok_or_else(|| raise("NoMethodError", "undefined method 'name' for nil"))?;
            let name = opt
                .name
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "unnamed".to_owned());
            let cli = opt
                .cli
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "N/A".to_owned());
            let kind = opt
                .r#type
                .get()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "string".to_owned());
            let delimiter = opt
                .assignment_delimiter
                .get()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "auto".to_owned());
            say_c(out, off, &format!("  {name} ({kind})"), WHITE);
            say_c(out, off, &format!("    CLI: {cli}"), DIM);
            if delimiter != "auto" {
                say_c(
                    out,
                    off,
                    &format!("    Assignment Delimiter: {delimiter}"),
                    DIM,
                );
            }
            if let Some(default) = &opt.default {
                say_c(
                    out,
                    off,
                    &format!("    Default: {}", default.to_ruby_s()),
                    DIM,
                );
            }
            if let Some(platforms) = non_empty(&opt.platforms).filter(|_| any_text(&opt.platforms))
            {
                say_c(
                    out,
                    off,
                    &format!("    Platforms: {}", texts_joined(platforms, ", ")),
                    DIM,
                );
            }
            if let Some(description) = &opt.description {
                say_c(
                    out,
                    off,
                    &format!("    Description: {}", description.to_ruby_s()),
                    DIM,
                );
            }
            if let Some(range) = &opt.range {
                say_c(
                    out,
                    off,
                    &format!("    Range: {}", ints_joined(range, "..")),
                    DIM,
                );
            }
            if let Some(values) = &opt.values {
                say_c(
                    out,
                    off,
                    &format!("    Valid values: {}", texts_joined(values, ", ")),
                    DIM,
                );
            }
            if let Some(of) = &opt.of {
                say_c(
                    out,
                    off,
                    &format!("    Element type: {}", of.to_ruby_s()),
                    DIM,
                );
            }
            say_c(out, off, "", WHITE);
        }
    }
    if let Some(post) = non_empty(&cmd.post_options) {
        say_c(out, off, "Post-Options (between input and output):", YELLOW);
        for opt in post {
            let opt = opt
                .as_ref()
                .ok_or_else(|| raise("NoMethodError", "undefined method 'name' for nil"))?;
            let name = opt
                .name
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "unnamed".to_owned());
            let cli = opt
                .cli
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "N/A".to_owned());
            let kind = opt
                .r#type
                .get()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "string".to_owned());
            say_c(out, off, &format!("  {name} ({kind})"), WHITE);
            say_c(out, off, &format!("    CLI: {cli}"), DIM);
            if let Some(description) = &opt.description {
                say_c(
                    out,
                    off,
                    &format!("    Description: {}", description.to_ruby_s()),
                    DIM,
                );
            }
            say_c(out, off, "", WHITE);
        }
    }
    if let Some(flags) = non_empty(&cmd.flags) {
        say_c(out, off, "Flags:", YELLOW);
        for flag in flags {
            let flag = flag
                .as_ref()
                .ok_or_else(|| raise("NoMethodError", "undefined method 'name' for nil"))?;
            let name = flag
                .name
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "unnamed".to_owned());
            let cli = flag
                .cli
                .as_ref()
                .map(Text::to_ruby_s)
                .unwrap_or_else(|| "N/A".to_owned());
            say_c(out, off, &format!("  {name} (boolean)"), WHITE);
            say_c(out, off, &format!("    CLI: {cli}"), DIM);
            if let Some(default) = flag.default.get() {
                say_c(out, off, &format!("    Default: {default}"), DIM);
            }
            if let Some(platforms) =
                non_empty(&flag.platforms).filter(|_| any_text(&flag.platforms))
            {
                say_c(
                    out,
                    off,
                    &format!("    Platforms: {}", texts_joined(platforms, ", ")),
                    DIM,
                );
            }
            if let Some(description) = &flag.description {
                say_c(
                    out,
                    off,
                    &format!("    Description: {}", description.to_ruby_s()),
                    DIM,
                );
            }
            say_c(out, off, "", WHITE);
        }
    }
    if non_empty(&cmd.env_vars).is_some() {
        say_c(out, off, "Environment Variables:", YELLOW);
        // The loop reads `ev.env_var`, which EnvVarDefinition does not have
        // (its attribute is `from`), so the first variable raises.
        return Err(raise(
            "NoMethodError",
            "undefined method 'env_var' for an instance of Ukiryu::Models::EnvVarDefinition",
        ));
    }
    say_c(out, off, "Option Sets (common combinations):", YELLOW);
    say_c(out, off, "  --help (show help)", DIM);
    say_c(out, off, "  --version (show version)", DIM);
    if let Some(options) = non_empty(&cmd.options) {
        let named = |words: &[&str]| -> Vec<String> {
            options
                .iter()
                .filter_map(|o| {
                    o.as_ref()
                        .and_then(|o| o.name.as_ref())
                        .map(Text::to_ruby_s)
                })
                .filter(|n| {
                    let lower = n.to_lowercase();
                    words.iter().any(|w| lower.contains(w))
                })
                .collect()
        };
        let output = named(&["output", "out", "file", "format"]);
        if !output.is_empty() {
            say_c(out, off, "", WHITE);
            say_c(
                out,
                off,
                &format!("Output options: {}", output.join(", ")),
                DIM,
            );
        }
        let quality = named(&["quality", "q", "compression"]);
        if !quality.is_empty() {
            say_c(
                out,
                off,
                &format!("Quality options: {}", quality.join(", ")),
                DIM,
            );
        }
    }
    if let Some(codes) = cmd.exit_codes.as_deref() {
        let standard = codes.standard_codes();
        let custom = codes.custom_codes();
        let filled = |v: &Value| matches!(v, Value::Hash(p) if !p.is_empty());
        if filled(&standard) || filled(&custom) {
            say_c(out, off, "", WHITE);
            say_c(out, off, "Exit Codes:", YELLOW);
            if filled(&standard) {
                say_c(out, off, "  Standard:", DIM);
                code_lines(out, off, &standard)?;
            }
            if filled(&custom) {
                say_c(out, off, "  Custom:", DIM);
                code_lines(out, off, &custom)?;
            }
        }
    }
    say_c(out, off, "", WHITE);
    say_c(out, off, RULE, CYAN);
    say_c(out, off, "Example usage:", YELLOW);
    say_c(
        out,
        off,
        &format!("  ukiryu exec {tool_name} {command_name} key=value"),
        WHITE,
    );
    say_c(out, off, RULE, CYAN);
    Ok(())
}

/// `String#to_i` (and `to_i` of the other key classes the source can meet).
fn key_to_i(key: &Value) -> Step<i128> {
    match key {
        Value::String(s) => {
            let t = s.trim_start();
            let (negative, digits) = match t.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, t.strip_prefix('+').unwrap_or(t)),
            };
            let mut value: i128 = 0;
            let mut previous_digit = false;
            for c in digits.chars() {
                match c.to_digit(10) {
                    Some(d) => {
                        value = value.saturating_mul(10).saturating_add(i128::from(d));
                        previous_digit = true;
                    }
                    None if c == '_' && previous_digit => previous_digit = false,
                    None => break,
                }
            }
            Ok(if negative { -value } else { value })
        }
        Value::Nil => Ok(0),
        Value::Integer(n) => Ok(i128::try_from(n).unwrap_or(i128::MAX)),
        other => Err(raise(
            "NoMethodError",
            format!("undefined method 'to_i' for {}", value_receiver(other)),
        )),
    }
}

/// `codes.sort_by { |k, _| k.to_i }.each { |code, meaning| say "    #{code.rjust(3)}: #{meaning}" }`.
fn code_lines(out: &mut Out, off: bool, codes: &Value) -> Step {
    let Value::Hash(pairs) = codes else {
        return Ok(());
    };
    let mut keyed = Vec::new();
    for (key, meaning) in pairs {
        // The oracle's exit-code hash keys are Symbols (measured), so the
        // first `k.to_i` raises; the core model stores the same keys as
        // Strings.
        if matches!(key, Value::String(_)) {
            return Err(raise(
                "NoMethodError",
                "undefined method 'to_i' for an instance of Symbol",
            ));
        }
        keyed.push((key_to_i(key)?, key, meaning));
    }
    keyed.sort_by_key(|(n, _, _)| *n);
    for (_, key, meaning) in keyed {
        let Value::String(code) = key else {
            return Err(raise(
                "NoMethodError",
                format!("undefined method 'rjust' for {}", value_receiver(key)),
            ));
        };
        let width = code.chars().count();
        let padded = if width >= 3 {
            code.clone()
        } else {
            format!("{}{code}", " ".repeat(3 - width))
        };
        say_c(
            out,
            off,
            &format!("    {padded}: {}", meaning.to_ruby_s()),
            WHITE,
        );
    }
    Ok(())
}
