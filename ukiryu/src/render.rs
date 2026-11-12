//! Response rendering for the CLI output formats.
//!
//! Source: `lib/ukiryu/cli_commands/response_formatter.rb`
//! (`Ukiryu::CliCommands::ResponseFormatter`), the `RunCommand#say_dry_run`
//! override in `run_command.rb` and `BaseCommand#say` in
//! `base_command.rb`, at the pinned Ruby oracle.
//!
//! The core decides every byte and where it goes; a runtime adapter only
//! performs the returned [`OutputWrite`]s in order. Nothing here touches a
//! terminal, a file or the environment: colour and terminal state come in
//! through [`RenderSettings`].
//!
//! YAML goes through the Yeptris emitter with the pinned
//! [`yaml_emit_options`], JSON through `serde_json`. Responses are consumed
//! through the [`Renderable`] trait, so the typed response models (owned by
//! the models module) plug in without a second model stack here.
use serde::Serialize;
use std::fmt;

mod dry_run;
mod emit;

pub use dry_run::{render_dry_run, DryRunStyle, InspectValue};
pub use emit::{to_json_pretty, to_yaml, yaml_emit_options};

/// `ResponseFormatter::OUTPUT_FORMATS`, in source order.
pub const OUTPUT_FORMATS: [&str; 4] = ["yaml", "json", "table", "raw"];

/// A response output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Yaml,
    Json,
    Table,
    Raw,
}

impl OutputFormat {
    /// Validates a format name the way `RunCommand#run` and
    /// `RunFileCommand#run` do before rendering.
    pub fn parse(name: &str) -> Result<OutputFormat, RenderError> {
        match name {
            "yaml" => Ok(OutputFormat::Yaml),
            "json" => Ok(OutputFormat::Json),
            "table" => Ok(OutputFormat::Table),
            "raw" => Ok(OutputFormat::Raw),
            other => Err(RenderError::new(format!(
                "Invalid format: {other}. Must be one of: {}",
                OUTPUT_FORMATS.join(", ")
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Yaml => "yaml",
            OutputFormat::Json => "json",
            OutputFormat::Table => "table",
            OutputFormat::Raw => "raw",
        }
    }
}

/// A rendering failure (an invalid format name or a serialization error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError {
    message: String,
}

impl RenderError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        RenderError {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RenderError {}

/// Availability of the optional `paint` gem the source colours with.
///
/// The YAML colouring calls `require 'paint'` (so it needs the gem to be
/// installed); the table renderer checks `defined?(Paint)` (so it needs the
/// gem to be loaded already, which the CLI does only through the debug
/// logger). `enabled` is false when Paint's colour mode is 0, where it
/// returns text unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintSupport {
    pub available: bool,
    pub loaded: bool,
    pub enabled: bool,
}

impl PaintSupport {
    pub const UNAVAILABLE: PaintSupport = PaintSupport {
        available: false,
        loaded: false,
        enabled: true,
    };
    pub const AVAILABLE: PaintSupport = PaintSupport {
        available: true,
        loaded: false,
        enabled: true,
    };
    pub const LOADED: PaintSupport = PaintSupport {
        available: true,
        loaded: true,
        enabled: true,
    };
}

/// The configuration and terminal state the source reads while rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSettings {
    /// `Config#use_color` (`nil` means auto-detect).
    pub use_color: Option<bool>,
    /// `$stdout.tty?`.
    pub stdout_tty: bool,
    pub paint: PaintSupport,
}

impl RenderSettings {
    /// `Config#colors_disabled?`: only an explicit `use_color: false`.
    pub fn colors_disabled(&self) -> bool {
        self.use_color == Some(false)
    }

    /// The YAML colouring decision of `format_yaml_response`.
    fn yaml_colors(&self) -> bool {
        match self.use_color {
            None => self.stdout_tty && !self.colors_disabled(),
            Some(on) => on,
        }
    }
}

/// One write the adapter performs, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputWrite {
    Stdout(String),
    Stderr(String),
    /// `File.write(path, contents)` (replaces the file).
    File {
        path: String,
        contents: String,
    },
}

/// The request arguments as the table summary reads them
/// (`request.options`, `request.flags`, `request.positional`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestSummary {
    /// `(name, value)` of each option, as Ruby `to_s` text.
    pub options: Vec<(String, String)>,
    pub flags: Vec<String>,
    /// The value of each positional argument.
    pub positional: Vec<String>,
}

impl RequestSummary {
    /// `format_request_summary`.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.options.is_empty() {
            let opts: Vec<String> = self
                .options
                .iter()
                .map(|(n, v)| format!("{n}={v}"))
                .collect();
            parts.push(opts.join(" "));
        }
        if !self.flags.is_empty() {
            let flags: Vec<String> = self.flags.iter().map(|f| format!("--{f}")).collect();
            parts.push(flags.join(" "));
        }
        if !self.positional.is_empty() {
            parts.push(self.positional.join(" "));
        }
        parts.join(" ")
    }
}

/// The fields of a success response the table and raw formats read; each
/// text is the Ruby `to_s` of the attribute (`nil` → empty).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SuccessView {
    pub exit_code: Option<i64>,
    pub formatted_duration: String,
    pub request: RequestSummary,
    pub full_command: String,
    pub stdout: String,
    pub stderr: String,
}

/// The table/raw view of a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseView {
    Success(SuccessView),
    /// `ErrorResponse#error` as `to_s` text.
    Error {
        error: String,
    },
}

/// A response the formats can render: serialized as a whole for YAML and
/// JSON, read through [`ResponseView`] for table and raw output.
pub trait Renderable: Serialize {
    fn view(&self) -> ResponseView;
}

/// `ResponseFormatter#output_response(response, format, output_file, config)`
/// as the ordered writes it performs.
pub fn output_response<R: Renderable + ?Sized>(
    response: &R,
    format: OutputFormat,
    output_file: Option<&str>,
    settings: &RenderSettings,
) -> Result<Vec<OutputWrite>, RenderError> {
    if format == OutputFormat::Raw {
        return Ok(raw_writes(&response.view(), output_file));
    }
    let output = match format {
        OutputFormat::Yaml => format_yaml_text(&to_yaml(response)?, settings),
        OutputFormat::Json => to_json_pretty(response)?,
        OutputFormat::Table => format_table_response(&response.view(), settings),
        OutputFormat::Raw => unreachable!("handled above"),
    };
    Ok(emit_output(output, output_file, settings))
}

/// The non-raw tail of `output_response`: write `output` to the file and
/// announce it, or `say` it on stdout.
pub fn emit_output(
    output: String,
    output_file: Option<&str>,
    settings: &RenderSettings,
) -> Vec<OutputWrite> {
    match output_file {
        Some(path) => vec![
            OutputWrite::File {
                path: path.to_owned(),
                contents: output,
            },
            OutputWrite::Stdout(say(
                &format!("Response written to: {path}"),
                Some(SayColor::Green),
                settings,
            )),
        ],
        None => vec![OutputWrite::Stdout(say(&output, None, settings))],
    }
}

fn raw_writes(view: &ResponseView, output_file: Option<&str>) -> Vec<OutputWrite> {
    let mut writes = Vec::new();
    match view {
        ResponseView::Success(s) => {
            if !s.stdout.is_empty() {
                writes.push(OutputWrite::Stdout(s.stdout.clone()));
            }
            if !s.stderr.is_empty() {
                writes.push(OutputWrite::Stderr(s.stderr.clone()));
            }
            if let Some(path) = output_file {
                writes.push(OutputWrite::File {
                    path: path.to_owned(),
                    contents: s.stdout.clone(),
                });
            }
        }
        ResponseView::Error { error } => writes.push(OutputWrite::Stderr(format!("{error}\n"))),
    }
    writes
}

/// The colouring step of `format_yaml_response` over a YAML document:
/// each line is painted by its key when colours are on and the gem can be
/// required, otherwise the text is returned unchanged.
pub fn format_yaml_text(yaml: &str, settings: &RenderSettings) -> String {
    if !settings.yaml_colors() || !settings.paint.available {
        return yaml.to_owned();
    }
    let paint = Painter(settings.paint.enabled);
    yaml.split_inclusive('\n')
        .map(|line| {
            let codes: &[u8] = if line.starts_with("status:") {
                &[36, 1]
            } else if line.starts_with("exit_code:") {
                &[33]
            } else if line.starts_with("  executable:") {
                &[32]
            } else if line.starts_with("  full_command:") {
                &[34, 1]
            } else if line.starts_with("stderr:") {
                &[31]
            } else if line.starts_with("  duration_seconds:")
                || line.starts_with("  formatted_duration:")
            {
                &[35]
            } else {
                &[]
            };
            if codes.is_empty() {
                line.to_owned()
            } else {
                paint.paint(line, codes)
            }
        })
        .collect()
}

/// `format_table_response`.
fn format_table_response(view: &ResponseView, settings: &RenderSettings) -> String {
    let paint = Painter(settings.paint.enabled);
    match view {
        ResponseView::Success(s) if settings.paint.loaded => {
            let mut out = vec![
                format!(
                    "{} {}\n",
                    paint.paint("✓", &[32]),
                    paint.paint("Command completed successfully", &[32, 1])
                ),
                format!(
                    "{} {}\n",
                    paint.paint("Exit code:", &[37]),
                    exit_code_s(s.exit_code)
                ),
                format!(
                    "{} {}\n",
                    paint.paint("Duration:", &[37]),
                    s.formatted_duration
                ),
                format!(
                    "\n{} {}\n",
                    paint.paint("Request:", &[36, 1]),
                    s.request.summary()
                ),
                format!(
                    "\n{} {}\n",
                    paint.paint("Command:", &[36, 1]),
                    s.full_command
                ),
            ];
            if !s.stdout.is_empty() {
                out.push(format!(
                    "\n{} {}\n",
                    paint.paint("Output:", &[37]),
                    first_line(&s.stdout)
                ));
            }
            if !s.stderr.is_empty() {
                out.push(format!(
                    "\n{} {}\n",
                    paint.paint("Errors:", &[31]),
                    s.stderr
                ));
            }
            out.join("\n")
        }
        ResponseView::Success(s) => format!(
            "✓ Command completed successfully\nExit code: {}\nDuration: {}\n\nRequest: {}\n\nCommand: {}\n",
            exit_code_s(s.exit_code),
            s.formatted_duration,
            s.request.summary(),
            s.full_command
        ),
        ResponseView::Error { error } if settings.paint.loaded => format!(
            "{} {} {error}\n",
            paint.paint("✗", &[31]),
            paint.paint("Command failed:", &[31, 1])
        ),
        ResponseView::Error { error } => format!("✗ Command failed: {error}\n"),
    }
}

fn exit_code_s(code: Option<i64>) -> String {
    code.map(|c| c.to_string()).unwrap_or_default()
}

/// `stdout.split("\n").first` interpolated: Ruby drops trailing empty
/// fields, so an all-newline string has no first line.
fn first_line(s: &str) -> &str {
    let trimmed = s.trim_end_matches('\n');
    trimmed.split('\n').next().unwrap_or("")
}

/// `Paint[text, *styles]` for the basic colours the source uses.
struct Painter(bool);

impl Painter {
    fn paint(&self, text: &str, codes: &[u8]) -> String {
        if !self.0 {
            return text.to_owned();
        }
        let codes: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
        format!("\x1b[{}m{text}\x1b[0m", codes.join(";"))
    }
}

/// The colours `BaseCommand#say` maps; any other name (`:clear`) is 37.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SayColor {
    Green,
    Yellow,
    Cyan,
    White,
    /// `:clear` and other unmapped names.
    Other,
}

impl SayColor {
    fn code(self) -> u8 {
        match self {
            SayColor::Green => 32,
            SayColor::Yellow => 33,
            SayColor::Cyan => 36,
            SayColor::White | SayColor::Other => 37,
        }
    }
}

/// `BaseCommand#say(message, color)`: ANSI-wrapped unless colours are
/// disabled, then `puts` (a newline unless the text already ends in one).
pub(crate) fn say(message: &str, color: Option<SayColor>, settings: &RenderSettings) -> String {
    say_with(message, color, settings.colors_disabled())
}

pub(crate) fn say_with(message: &str, color: Option<SayColor>, colors_disabled: bool) -> String {
    let text = match color {
        Some(c) if !colors_disabled => format!("\x1b[{}m{message}\x1b[0m", c.code()),
        _ => message.to_owned(),
    };
    puts(text)
}

fn puts(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}
