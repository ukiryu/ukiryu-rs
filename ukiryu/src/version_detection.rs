//! Version detection and extraction strategies.
//!
//! Ports `Ukiryu::VersionDetector` (lib/ukiryu/version_detector.rb),
//! `Ukiryu::Tool::VersionDetection#detect_version` (lib/ukiryu/tool/version_detection.rb)
//! and the detection step of implementation-index loading (lib/ukiryu/tool/loader.rb)
//! from the pinned Ruby source.
//!
//! Process execution, man-page date parsing and platform/shell detection are
//! injected ([`CommandRunner`], [`ManPageDates`], explicit platform and
//! optional shell values), so this module never starts a process or decides
//! the owner-pending shell policy.
mod detector;
mod index;
mod pattern;
mod profile;
mod runner;
pub mod text;

pub use detector::{
    detect_info, detect_with_methods, DetectedVersion, DetectionMethod, InfoRequest, MethodSpec,
    MethodsRequest, PatternSpec, MAN_TAIL_CHARS,
};
pub use index::{
    detection_output, extract_version_from_pattern, probe_index_detection, IndexDetection,
    IndexProbe, INDEX_DETECTION_TIMEOUT_SECS,
};
pub use pattern::{PatternError, PatternMatch, RubyPattern, DEFAULT_PATTERN_SOURCE};
pub use profile::{
    detect_profile_version, DetectionMethodView, ToolContext, VersionDetectionView,
    PROFILE_DETECTION_TIMEOUT_SECS,
};
pub use runner::{CommandOutput, CommandRequest, CommandRunner, ManPageDates, RunnerError};
