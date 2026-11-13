//! Authoring verbs: `validate`, `lint`, `docs` and `definitions`.
//!
//! Source: `lib/ukiryu/cli_commands/{validate,lint,docs,definitions}_command.rb`
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
//! Not here (gated on APIs that are not merged yet): `validate --executable`
//! and `validate --all` smoke tests (Tool + executor), `extract` (native
//! extractor), `describe` (Tool). Those paths return [`AuthorError::Gated`].
//! The remote schema download of `DefinitionValidator` is not performed: with
//! no `--schema` and no usable `UKIRYU_SCHEMA_PATH`, only the structural
//! checks run (the Ruby behaviour when the download is unavailable).
use super::CommandOutcome;
use std::cmp::Ordering;
use std::path::{Component, Path, PathBuf};
use ukiryu::definition::documentation::{DocumentationErrorKind, DocumentationGenerator};
use ukiryu::definition::linter::{DefinitionLinter, LintError, LintResult};
use ukiryu::definition::sources::file::ruby_time_to_s;
use ukiryu::definition::{
    DefinitionDocument, DefinitionError, DefinitionErrorKind, DefinitionMetadata, Discovery,
    DiscoveryEnv, DocumentParseError, Loader, Severity, ValidationMode, ValidationResult,
};
use ukiryu::models::attr::Model;
use ukiryu::models::error::ModelErrorKind;
use ukiryu::models::tool_definition::ToolDefinition;
use ukiryu::render::OutputWrite;
use ukiryu::schema::{self, Data, Permitted, SchemaError};

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
    /// `Register.default.path`, resolved by the register owner (discovery,
    /// `UKIRYU_REGISTER`, user clone). `None` when it is not resolved; the
    /// verbs that need it then return [`AuthorError::RegisterUnavailable`].
    pub default_register: Option<String>,
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
    /// `--register`: accepted but not read by any validate path (Ruby reads
    /// `Register.default.path` for `validate all`).
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
    let result = run_validate(&input, ctx, &mut out);
    out.finish(result)
}

fn run_validate(input: &ValidateInput, ctx: &AuthorContext, out: &mut Out) -> Step<i32> {
    let o = &input.options;
    let format = o.format.as_deref().unwrap_or("text");
    check_enum(out, "format", format, &["text", "json"])?;
    let schema = o.schema.as_deref();
    match &input.target {
        ValidateTarget::File { path } => {
            let result = validate_file(path, schema, ctx)?;
            output_validation(out, &result, path, format)?;
            if (o.executable || o.all) && result.is_valid() {
                return Err(Stop::Error(AuthorErrorKind::Gated(
                    "validate --executable/--all executable test (Tool and executor API)",
                )));
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
                return Err(Stop::Error(AuthorErrorKind::Gated(
                    "validate all --executable/--all executable tests (Tool and executor API)",
                )));
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

/// `DefinitionValidator.validate_file(path, schema_path:)`.
fn validate_file(path: &str, schema: Option<&str>, ctx: &AuthorContext) -> Step<ValidationResult> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ValidationResult::failure(
                vec![format!("File not found: {path}")],
                Vec::new(),
            ))
        }
        Err(e) => return Err(read_error(&e, path)),
    };
    let text = String::from_utf8(bytes).map_err(|_| unsupported(format!("{path} is not UTF-8")))?;
    validate_string(&text, schema, ctx)
}

/// `DefinitionValidator.validate_string(yaml, schema_path:)`.
fn validate_string(
    yaml: &str,
    schema: Option<&str>,
    ctx: &AuthorContext,
) -> Step<ValidationResult> {
    let definition = match schema::safe_load(yaml, Permitted::DOCUMENT) {
        Ok(d) => d,
        Err(SchemaError::YamlSyntax { message }) => {
            return Ok(ValidationResult::failure(
                vec![format!("Invalid YAML: {message}")],
                Vec::new(),
            ))
        }
        Err(e) => return Err(schema_stop(e)),
    };
    validate_definition(&definition, schema, ctx)
}

/// `DefinitionValidator.validate(definition, schema_path:)`.
fn validate_definition(
    definition: &Data,
    schema_path: Option<&str>,
    ctx: &AuthorContext,
) -> Step<ValidationResult> {
    let (mut errors, mut warnings) = validate_structure(definition)?;
    let schema = match schema_path {
        Some(path) => schema::load_definition_schema(Path::new(path)).map_err(schema_stop)?,
        // find_schema: UKIRYU_SCHEMA_PATH when that file exists. The remote
        // fallback (download_and_load_schema) is not performed.
        None => match ctx.schema_path_env.as_deref() {
            Some(env) if Path::new(env).exists() => {
                schema::load_definition_schema(Path::new(env)).map_err(schema_stop)?
            }
            _ => None,
        },
    };
    if let Some(schema) = schema.filter(Data::truthy) {
        let report = schema::definition_schema_report(definition, &schema).map_err(schema_stop)?;
        errors.extend(report.errors);
        warnings.extend(report.warnings);
    }
    Ok(if errors.is_empty() {
        if warnings.is_empty() {
            ValidationResult::success()
        } else {
            ValidationResult::with_warnings(warnings)
        }
    } else {
        ValidationResult::failure(errors, warnings)
    })
}

/// `definition[:key] || definition['key']` (a truthy value or `None`).
fn hash_get<'a>(pairs: &'a [(Data, Data)], key: &str) -> Option<&'a Data> {
    fn last<'a>(pairs: &'a [(Data, Data)], key: &str, symbol: bool) -> Option<&'a Data> {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| match k {
                Data::Symbol(s) => symbol && s == key,
                Data::String(s) => !symbol && s == key,
                _ => false,
            })
            .map(|(_, v)| v)
            .filter(|v| v.truthy())
    }
    last(pairs, key, true).or_else(|| last(pairs, key, false))
}

fn has_key(pairs: &[(Data, Data)], key: &str) -> bool {
    pairs
        .iter()
        .any(|(k, _)| matches!(k, Data::Symbol(s) | Data::String(s) if s == key))
}

/// Ruby regexp `\s`.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// Ruby `^...$` (multi-line anchors): whether any line satisfies `line_ok`.
fn any_line(s: &str, line_ok: impl Fn(&str) -> bool) -> bool {
    s.split('\n').any(line_ok)
}

/// `DefinitionValidator.validate_structure(definition)`.
fn validate_structure(definition: &Data) -> Step<(Vec<String>, Vec<String>)> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let Data::Hash(pairs) = definition else {
        return Ok((
            vec!["Definition must be a hash/object".to_owned()],
            Vec::new(),
        ));
    };
    if let Some(name) = hash_get(pairs, "name") {
        let name = name.ruby_to_s().map_err(schema_stop)?;
        if name.chars().any(is_ruby_space) {
            errors.push("Tool name must not contain whitespace".to_owned());
        }
        if any_line(&name, |l| l.starts_with(|c: char| c.is_ascii_digit())) {
            warnings.push("Tool name starting with number may cause issues".to_owned());
        }
        let lower_ok = |l: &str| {
            !l.is_empty()
                && l.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        };
        if !any_line(&name, lower_ok) {
            warnings.push(
                "Tool name should contain only lowercase letters, numbers, hyphens, and underscores"
                    .to_owned(),
            );
        }
    }
    if has_key(pairs, "ukiryu_schema") {
        let value = hash_get(pairs, "ukiryu_schema")
            .cloned()
            .unwrap_or_else(|| {
                // `nil || definition['ukiryu_schema']` keeps a falsy String-key value.
                pairs
                    .iter()
                    .rev()
                    .find(|(k, _)| matches!(k, Data::String(s) if s == "ukiryu_schema"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Data::Null)
            });
        let text = value.ruby_to_s().map_err(schema_stop)?;
        let format_ok = |l: &str| {
            let mut parts = l.splitn(2, '.');
            let a = parts.next().unwrap_or("");
            let b = parts.next().unwrap_or("");
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        };
        if !any_line(&text, format_ok) {
            warnings.push(format!(
                "Invalid schema version format: {text} (expected format: '1.0')"
            ));
        }
    }
    Ok((errors, warnings))
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

    fn resolve_inheritance(&mut self) {
        self.inner.resolve_inheritance();
    }
}
