//! Port of `Ukiryu::Tool::VersionDetection#detect_version` and
//! `#detect_version_with_detection_methods` (lib/ukiryu/tool/version_detection.rb).
//!
//! The inputs are a minimal view of `Models::VersionDetection` /
//! `Models::VersionDetectionMethod` as the Lutaml model presents them to this
//! code (a scalar `command` is already a one-element list). The typed models
//! are owned by the models module; integration replaces these views with a
//! conversion from them.
use super::detector::{
    detect_info, detect_with_methods, DetectedVersion, DetectionMethod, InfoRequest, MethodSpec, MethodsRequest,
    PatternSpec,
};
use super::pattern::PatternError;
use super::runner::{CommandRunner, ManPageDates};

/// Timeout the source hardcodes for profile version detection.
pub const PROFILE_DETECTION_TIMEOUT_SECS: u64 = 30;

/// The `Models::VersionDetection` fields detection reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionDetectionView {
    pub command: Option<Vec<String>>,
    pub pattern: Option<String>,
    pub source: Option<String>,
    pub detection_methods: Option<Vec<DetectionMethodView>>,
}

/// The `Models::VersionDetectionMethod` fields detection reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetectionMethodView {
    pub method_type: Option<String>,
    pub command: Option<Vec<String>>,
    pub pattern: Option<String>,
    /// Platform name → man page path.
    pub paths: Option<Vec<(String, String)>>,
}

/// The tool state `detect_version` reads besides the profile's detection block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolContext {
    /// `@profile.version`, reported when the pattern is empty.
    pub profile_version: Option<String>,
    /// `@executable`.
    pub executable: Option<String>,
    /// `@shell`; `None` is resolved by the runner (owner-pending D2).
    pub shell: Option<String>,
    /// `Ukiryu::Platform.detect`, used only by `man_page` methods.
    pub platform: String,
}

/// `Tool#detect_version`.
pub fn detect_profile_version(
    detection: Option<&VersionDetectionView>,
    tool: &ToolContext,
    runner: &mut dyn CommandRunner,
    man_pages: &mut dyn ManPageDates,
) -> Result<Option<DetectedVersion>, PatternError> {
    let Some(vd) = detection else {
        return Ok(None);
    };

    if let Some(methods) = vd.detection_methods.as_deref().filter(|m| !m.is_empty()) {
        return detect_with_methods(
            &MethodsRequest {
                executable: tool.executable.clone(),
                methods: methods.iter().map(method_spec).collect(),
                shell: tool.shell.clone(),
                timeout_secs: PROFILE_DETECTION_TIMEOUT_SECS,
                platform: tool.platform.clone(),
            },
            runner,
            man_pages,
        );
    }

    let Some(command) = vd.command.as_deref().filter(|c| !c.is_empty()) else {
        return Ok(None);
    };

    // An empty pattern means "use the profile's own version" (e.g. BusyBox applets).
    let Some(pattern) = vd.pattern.as_deref().filter(|p| !p.is_empty()) else {
        return Ok(Some(DetectedVersion {
            value: tool.profile_version.clone(),
            method_used: DetectionMethod::Profile,
            available_methods: vec![DetectionMethod::Profile],
        }));
    };

    // `source: man` runs `man <rest of command>`; the first element is dropped.
    let (executable, args) = if vd.source.as_deref() == Some("man") {
        (Some("man".to_string()), command[1..].to_vec())
    } else {
        (tool.executable.clone(), command.to_vec())
    };

    detect_info(
        &InfoRequest {
            executable,
            command: args,
            pattern: PatternSpec::Source(pattern.to_string()),
            shell: tool.shell.clone(),
            source: vd.source.clone(),
            timeout_secs: PROFILE_DETECTION_TIMEOUT_SECS,
        },
        runner,
    )
}

/// The `detection_methods` → detector conversion. Every type other than
/// `man_page` (including a missing type and `system_tool`) becomes a
/// `--version` command with the default pattern unless it names its own.
fn method_spec(method: &DetectionMethodView) -> MethodSpec {
    if method.method_type.as_deref() == Some("man_page") {
        MethodSpec::ManPage {
            paths: Some(method.paths.clone().unwrap_or_default()),
        }
    } else {
        MethodSpec::Command {
            command: Some(method.command.clone().unwrap_or_else(|| vec!["--version".to_string()])),
            pattern: Some(match &method.pattern {
                Some(pattern) => PatternSpec::Source(pattern.clone()),
                None => PatternSpec::DefaultLiteral,
            }),
        }
    }
}
