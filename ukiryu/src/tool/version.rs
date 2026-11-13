//! `VersionDetection#detect_version` for a tool: the typed
//! `Models::VersionDetection` read into the detector's view.
use crate::models::{Text, Value, VersionDetection, VersionDetectionMethod};
use crate::version_detection::{
    detect_profile_version, CommandRunner, DetectedVersion, DetectionError, DetectionMethodView,
    ManPageDates, ToolContext, VersionDetectionView,
};

use super::{text, Tool};

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
}
