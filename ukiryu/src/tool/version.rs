//! `Tool::VersionDetection` (lib/ukiryu/tool/version_detection.rb):
//! `#detect_version` with the typed `Models::VersionDetection` read into the
//! detector's view, `#version` with its detection fallback,
//! `#check_version_compatibility` and `#probe_flag`.
//!
//! Compatibility results are the core
//! [`VersionCompatibility`](crate::models::VersionCompatibility) model; the
//! requirement check is its `check`.
//!
//! Version detection runs through an injected
//! [`CommandRunner`]/[`ManPageDates`], as in [`crate::version_detection`].
//! `#probe_flag` runs its process through a [`FlagProbe`]; [`ExecutorProbe`]
//! is the real executor. The tool supplies everything the executor requires:
//! its shell, the source's 5 second timeout and the source's empty
//! environment (the inherited process environment).
use crate::environment::Environment;
use crate::execution::results::bytes;
use crate::execution::Seconds;
use crate::executor::{self, ExecuteError, ExecuteRequest, Execution};
use crate::models::{Text, Value, VersionCompatibility, VersionDetection, VersionDetectionMethod};
use crate::runtime::ShellName;
use crate::version_detection::{
    detect_profile_version, CommandRunner, DetectedVersion, DetectionError, DetectionMethod,
    DetectionMethodView, ManPageDates, ToolContext, VersionDetectionView,
};

use super::{text, Tool, ToolError};

/// The timeout `#probe_flag` hardcodes, in seconds.
pub const PROBE_FLAG_TIMEOUT_SECS: i64 = 5;

/// The `mode` argument of `#check_version_compatibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompatibilityMode {
    /// `:strict` (the default): an unknown version is incompatible.
    #[default]
    Strict,
    /// `:lenient`: an unknown or unsatisfying version is compatible with a
    /// warning reason.
    Lenient,
    /// `:probe`: like any non-strict mode, but detects the version again
    /// when it is still unknown.
    Probe,
    /// Any other value. The source only compares against the three symbols,
    /// so an unknown version is compatible with a warning (it is not
    /// `:strict`) while an unsatisfying version stays incompatible (it is
    /// not `:lenient`).
    Other,
}

impl CompatibilityMode {
    /// The mode for a Ruby symbol name.
    pub fn from_name(name: &str) -> CompatibilityMode {
        match name {
            "strict" => CompatibilityMode::Strict,
            "lenient" => CompatibilityMode::Lenient,
            "probe" => CompatibilityMode::Probe,
            _ => CompatibilityMode::Other,
        }
    }
}

/// The `Executor.execute(executable, [flag, '--help'], shell:, timeout: 5)`
/// call `#probe_flag` makes.
#[derive(Debug, Clone, PartialEq)]
pub struct FlagProbeRequest {
    pub executable: String,
    /// `[flag, '--help']`.
    pub arguments: Vec<String>,
    /// Empty: the source passes no `env:`, so the child inherits the process
    /// environment.
    pub environment: Environment,
    pub shell: ShellName,
    /// [`PROBE_FLAG_TIMEOUT_SECS`].
    pub timeout: Seconds,
}

impl FlagProbeRequest {
    /// The executor request for this call (failure not allowed, as in the
    /// source).
    pub fn into_execute_request(self) -> ExecuteRequest {
        ExecuteRequest::new(
            self.executable,
            self.arguments,
            self.environment,
            self.shell,
            self.timeout,
        )
    }
}

/// Runs the `#probe_flag` process.
pub trait FlagProbe {
    fn run(&mut self, request: FlagProbeRequest) -> Result<Execution, ExecuteError>;
}

/// [`FlagProbe`] through [`executor::execute`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutorProbe;

impl FlagProbe for ExecutorProbe {
    fn run(&mut self, request: FlagProbeRequest) -> Result<Execution, ExecuteError> {
        executor::execute(request.into_execute_request())
    }
}

/// `Models::VersionInfo#to_s`: the value, with ` (man page)` appended for a
/// man page (where a missing value interpolates as empty).
pub fn version_info_to_s(info: &DetectedVersion) -> Option<String> {
    match info.method_used {
        DetectionMethod::ManPage => Some(format!(
            "{} (man page)",
            info.value.as_deref().unwrap_or_default()
        )),
        _ => info.value.clone(),
    }
}

fn method_view(m: &VersionDetectionMethod) -> DetectionMethodView {
    let paths = m.paths.as_ref().map(|p| match p {
        Value::Hash(pairs) => pairs
            .iter()
            .map(|(k, v)| (k.to_ruby_s(), v.to_ruby_s()))
            .collect(),
        _ => Vec::new(),
    });
    DetectionMethodView {
        method_type: m.r#type.as_ref().and_then(text),
        command: m.command.as_ref().and_then(text).map(|c| vec![c]),
        pattern: m.pattern.as_ref().and_then(text),
        paths,
    }
}

/// The detector's view of `Models::VersionDetection`.
pub(super) fn detection_view(vd: &VersionDetection) -> VersionDetectionView {
    VersionDetectionView {
        command: vd
            .command
            .as_ref()
            .map(|c| c.iter().map(Text::to_ruby_s).collect()),
        pattern: vd.pattern.as_ref().and_then(text),
        source: vd.source.get().and_then(text),
        detection_methods: vd
            .detection_methods
            .as_ref()
            .map(|ms| ms.iter().map(method_view).collect()),
    }
}

impl Tool {
    /// `#detect_version` (and `#version_info`): runs the definition's
    /// version detection with the tool's executable and shell.
    pub fn detect_version(
        &self,
        runner: &mut dyn CommandRunner,
        man_pages: &mut dyn ManPageDates,
    ) -> Result<Option<DetectedVersion>, DetectionError> {
        let view = self
            .definition
            .version_detection
            .as_deref()
            .map(detection_view);
        let context = ToolContext {
            profile_version: self.definition.version.as_ref().and_then(text),
            executable: self.executable().map(str::to_owned),
            shell: Some(self.shell.as_str().to_owned()),
            platform: self.platform.as_str().to_owned(),
        };
        detect_profile_version(view.as_ref(), &context, runner, man_pages)
    }

    /// `#version`: the `version` option, else the definition's version, else
    /// the detected version as `VersionInfo#to_s` gives it.
    pub fn version_or_detect(
        &self,
        runner: &mut dyn CommandRunner,
        man_pages: &mut dyn ManPageDates,
    ) -> Result<Option<String>, DetectionError> {
        if let Some(version) = self.version() {
            return Ok(Some(version));
        }
        Ok(self
            .detect_version(runner, man_pages)?
            .as_ref()
            .and_then(version_info_to_s))
    }

    /// `#profile_version_requirement`: the selected profile's
    /// `version_requirement`.
    pub fn version_requirement(&self) -> Option<String> {
        self.command_profile()
            .version_requirement
            .as_ref()
            .and_then(text)
    }

    /// `#check_version_compatibility(mode)`.
    ///
    /// The installed version is [`Tool::version_or_detect`], so detection
    /// runs (before the requirement is read) whenever neither the option nor
    /// the definition gives a version; [`CompatibilityMode::Probe`] detects
    /// once more when the version is still unknown and a requirement exists.
    /// Detection errors propagate as in the source.
    pub fn check_version_compatibility(
        &self,
        mode: CompatibilityMode,
        runner: &mut dyn CommandRunner,
        man_pages: &mut dyn ManPageDates,
    ) -> Result<VersionCompatibility, DetectionError> {
        let installed = self.version_or_detect(runner, man_pages)?;
        let requirement = match self.version_requirement() {
            Some(r) if !r.is_empty() => r,
            _ => {
                return Ok(VersionCompatibility::new(
                    installed.as_deref().unwrap_or("unknown"),
                    Some("none"),
                    true,
                    None,
                ));
            }
        };

        let installed = match installed {
            None if mode == CompatibilityMode::Probe => self
                .detect_version(runner, man_pages)?
                .as_ref()
                .and_then(version_info_to_s),
            other => other,
        };

        let Some(installed) = installed else {
            return Ok(if mode == CompatibilityMode::Strict {
                VersionCompatibility::new(
                    "unknown",
                    Some(&requirement),
                    false,
                    Some("Cannot determine installed tool version".to_owned()),
                )
            } else {
                VersionCompatibility::new(
                    "unknown",
                    Some(&requirement),
                    true,
                    Some("Warning: Could not verify version compatibility".to_owned()),
                )
            });
        };

        let result = VersionCompatibility::check(&installed, Some(&requirement));
        if !result.is_compatible() && mode == CompatibilityMode::Lenient {
            return Ok(VersionCompatibility::new(
                &installed,
                Some(&requirement),
                true,
                Some(format!(
                    "Warning: {}",
                    result.reason.as_deref().unwrap_or_default()
                )),
            ));
        }
        Ok(result)
    }

    /// `#probe_flag(flag)` through the real executor.
    pub fn probe_flag(&self, flag: &str) -> Result<bool, ToolError> {
        self.probe_flag_with(flag, &mut ExecutorProbe)
    }

    /// `#probe_flag(flag)`: `false` without an executable; otherwise runs
    /// `<executable> <flag> --help` with the tool's shell and a 5 second
    /// timeout and answers whether it succeeded with no `unknown` in its raw
    /// stderr (case-sensitive). As in the source, failure is not allowed: a
    /// non-zero exit, a timeout or a start failure is an error, not `false`.
    pub fn probe_flag_with(
        &self,
        flag: &str,
        probe: &mut dyn FlagProbe,
    ) -> Result<bool, ToolError> {
        let Some(executable) = self.executable() else {
            return Ok(false);
        };
        let execution = probe.run(FlagProbeRequest {
            executable: executable.to_owned(),
            arguments: vec![flag.to_owned(), "--help".to_owned()],
            environment: Environment::new(),
            shell: self.shell,
            timeout: Seconds::Int(PROBE_FLAG_TIMEOUT_SECS),
        })?;
        Ok(execution.result.is_success() && !bytes::contains(execution.result.stderr(), "unknown"))
    }
}
