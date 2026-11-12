//! Port of `Ukiryu::VersionDetector` (lib/ukiryu/version_detector.rb):
//! `detect_info` and `detect_with_methods`.
use super::pattern::{PatternError, RubyPattern};
use super::runner::{CommandRequest, CommandRunner, ManPageDates};
use super::text;

/// `VersionInfo#method_used` / `available_methods` entries (Ruby symbols).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectionMethod {
    Command,
    ManPage,
    Profile,
    /// A method type the detector does not run; it still appears in
    /// `available_methods` (`methods.map { |m| m[:type] }.uniq`).
    Other(String),
}

impl DetectionMethod {
    /// The Ruby symbol name.
    pub fn name(&self) -> &str {
        match self {
            DetectionMethod::Command => "command",
            DetectionMethod::ManPage => "man_page",
            DetectionMethod::Profile => "profile",
            DetectionMethod::Other(name) => name,
        }
    }
}

/// The fields of the `Models::VersionInfo` the source builds. Kept minimal:
/// the typed model and its helpers belong to the models module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedVersion {
    /// `match[1]`, the man-page date, or the profile version. `None` when
    /// the pattern has no first group (Ruby still reports a detection).
    pub value: Option<String>,
    pub method_used: DetectionMethod,
    pub available_methods: Vec<DetectionMethod>,
}

impl DetectedVersion {
    fn single(value: Option<&str>, method: DetectionMethod) -> Self {
        DetectedVersion {
            value: value.map(str::to_string),
            available_methods: vec![method.clone()],
            method_used: method,
        }
    }
}

/// A pattern argument: the source's `/(\d+\.\d+)/` literal or a profile string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternSpec {
    DefaultLiteral,
    Source(String),
}

/// Compiled form; a pattern Ruby rejects is kept so the command still runs
/// (Ruby raises only at match time and rescues it).
enum Compiled<'p> {
    Ready(std::borrow::Cow<'p, RubyPattern>),
    RubyInvalid,
}

fn compile(spec: &PatternSpec) -> Result<Compiled<'static>, PatternError> {
    match spec {
        PatternSpec::DefaultLiteral => Ok(Compiled::Ready(std::borrow::Cow::Borrowed(RubyPattern::default_literal()))),
        PatternSpec::Source(source) => match RubyPattern::new(source) {
            Ok(pattern) => Ok(Compiled::Ready(std::borrow::Cow::Owned(pattern))),
            Err(PatternError::RubyInvalid { .. }) => Ok(Compiled::RubyInvalid),
            Err(error) => Err(error),
        },
    }
}

/// Arguments of `VersionDetector.detect_info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfoRequest {
    pub executable: Option<String>,
    /// Already normalised: a single string command is a one-element list.
    pub command: Vec<String>,
    pub pattern: PatternSpec,
    pub shell: Option<String>,
    /// `'command'` (default) or `'man'`; any other value behaves like `'command'`.
    pub source: Option<String>,
    pub timeout_secs: u64,
}

impl InfoRequest {
    /// The source defaults: `--version`, `/(\d+\.\d+)/`, `'command'`, 30 seconds.
    pub fn new(executable: impl Into<String>) -> Self {
        InfoRequest {
            executable: Some(executable.into()),
            command: vec!["--version".to_string()],
            pattern: PatternSpec::DefaultLiteral,
            shell: None,
            source: Some("command".to_string()),
            timeout_secs: 30,
        }
    }
}

/// Number of trailing characters searched first for `source: 'man'`.
pub const MAN_TAIL_CHARS: usize = 500;

/// `VersionDetector.detect_info`.
///
/// Every failure the source rescues (runner error, non-zero exit, a pattern
/// Ruby rejects) yields `Ok(None)`. `Err` is reserved for patterns this port
/// cannot reproduce.
pub fn detect_info(request: &InfoRequest, runner: &mut dyn CommandRunner) -> Result<Option<DetectedVersion>, PatternError> {
    let Some(executable) = request.executable.as_deref().filter(|e| !e.is_empty()) else {
        return Ok(None);
    };
    let compiled = compile(&request.pattern)?;
    let command = CommandRequest {
        executable: executable.to_string(),
        args: request.command.clone(),
        shell: request.shell.clone(),
        timeout_secs: request.timeout_secs,
        allow_failure: true,
    };
    let Ok(output) = runner.run(&command) else {
        return Ok(None);
    };
    if !output.success() {
        return Ok(None);
    }
    let Compiled::Ready(pattern) = compiled else {
        return Ok(None);
    };
    let stdout = text::scrub(&output.stdout);
    let stderr = text::scrub(&output.stderr);

    if request.source.as_deref() == Some("man") {
        let joined = format!("{stdout}{stderr}");
        let tail = text::tail_chars(&joined, MAN_TAIL_CHARS);
        if let Some(found) = pattern.captures(tail)? {
            return Ok(Some(DetectedVersion::single(found.group1, DetectionMethod::ManPage)));
        }
    }

    let found = match pattern.captures(&stdout)? {
        Some(found) => Some(found),
        None => pattern.captures(&stderr)?,
    };
    Ok(found.map(|found| DetectedVersion::single(found.group1, DetectionMethod::Command)))
}

/// One entry of the `methods:` array of `detect_with_methods`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MethodSpec {
    /// `{type: :command, command:, pattern:}`; `None` fields take the
    /// source defaults (`'--version'`, `/(\d+\.\d+)/`).
    Command {
        command: Option<Vec<String>>,
        pattern: Option<PatternSpec>,
    },
    /// `{type: :man_page, paths:}` with platform name keys.
    ManPage { paths: Option<Vec<(String, String)>> },
    /// Any other `type`: never run, but listed in `available_methods`.
    Other(String),
}

impl MethodSpec {
    fn method(&self) -> DetectionMethod {
        match self {
            MethodSpec::Command { .. } => DetectionMethod::Command,
            MethodSpec::ManPage { .. } => DetectionMethod::ManPage,
            MethodSpec::Other(name) => DetectionMethod::Other(name.clone()),
        }
    }
}

/// Arguments of `VersionDetector.detect_with_methods` besides the boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodsRequest {
    pub executable: Option<String>,
    pub methods: Vec<MethodSpec>,
    pub shell: Option<String>,
    pub timeout_secs: u64,
    /// The `Ukiryu::Platform.detect` value (`linux`, `macos`, `windows`).
    pub platform: String,
}

/// `VersionDetector.detect_with_methods`: the first method that yields a
/// result wins, in declaration order.
pub fn detect_with_methods(
    request: &MethodsRequest,
    runner: &mut dyn CommandRunner,
    man_pages: &mut dyn ManPageDates,
) -> Result<Option<DetectedVersion>, PatternError> {
    let mut available: Vec<DetectionMethod> = Vec::new();
    for method in &request.methods {
        let m = method.method();
        if !available.contains(&m) {
            available.push(m);
        }
    }

    for method in &request.methods {
        match method {
            MethodSpec::Command { command, pattern } => {
                let info = detect_info(
                    &InfoRequest {
                        executable: request.executable.clone(),
                        command: command.clone().unwrap_or_else(|| vec!["--version".to_string()]),
                        pattern: pattern.clone().unwrap_or(PatternSpec::DefaultLiteral),
                        shell: request.shell.clone(),
                        source: Some("command".to_string()),
                        timeout_secs: request.timeout_secs,
                    },
                    runner,
                )?;
                if info.is_some() {
                    return Ok(info);
                }
            }
            MethodSpec::ManPage { paths } => {
                let man_path = paths
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .find(|(platform, _)| *platform == request.platform)
                    .map(|(_, path)| path.as_str());
                let Some(man_path) = man_path else { continue };
                let Some(date) = man_pages.parse_date(man_path) else { continue };
                return Ok(Some(DetectedVersion {
                    value: Some(date),
                    method_used: DetectionMethod::ManPage,
                    available_methods: available,
                }));
            }
            MethodSpec::Other(_) => {}
        }
    }
    Ok(None)
}
