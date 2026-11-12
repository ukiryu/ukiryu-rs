//! The detection step of implementation-index loading
//! (`Ukiryu::Tool::Loader`, lib/ukiryu/tool/loader.rb): `run_detection_command`,
//! `try_execute_command`, `extract_version_from_pattern` and the presence
//! check in `try_implementation`. Choosing a version spec from the result is
//! the version-range module's job; the loader itself stays with the register.
//!
//! `Ukiryu::Tool` carries an older copy of these methods with no internal
//! callers; its presence check differs (a `nil` pattern skips the
//! implementation). The live loader copy is ported.
use super::pattern::{PatternError, RubyPattern};
use super::runner::{CommandOutput, CommandRequest, CommandRunner};
use super::text;

/// Timeout the loader hardcodes for detection commands.
pub const INDEX_DETECTION_TIMEOUT_SECS: u64 = 5;

/// The `detection:` hash of an implementation-index entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexDetection {
    /// `Array(command)`.
    pub command: Option<Vec<String>>,
    /// `Array(executables)`, tried in order.
    pub executables: Option<Vec<String>>,
    pub executable: Option<String>,
    pub pattern: Option<String>,
}

/// Outcome of probing one implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexProbe {
    /// No usable output, or a presence pattern that did not match: the loader
    /// moves on to the next implementation.
    NotDetected,
    /// The implementation is present. `version` is `match[1]`, or `None` for a
    /// presence-only pattern or no pattern (the loader then uses its default spec).
    Detected {
        output: String,
        version: Option<String>,
    },
}

/// `try_execute_command`'s choice of text: scrubbed stdout on success,
/// otherwise stderr when it is not blank. `None` covers every case the source
/// returns `nil` for, including stderr that `strip` rejects as invalid UTF-8.
pub fn detection_output(output: &CommandOutput) -> Option<String> {
    if output.success() {
        return Some(text::scrub_remove(&output.stdout));
    }
    match text::strip_is_empty(&output.stderr) {
        Some(false) => Some(text::scrub_remove(&output.stderr)),
        _ => None,
    }
}

/// `extract_version_from_pattern`: `match[1]`. Unlike profile detection, a
/// pattern Ruby rejects propagates (`RegexpError`), so it is an `Err` here.
pub fn extract_version_from_pattern(
    output: Option<&str>,
    pattern: Option<&str>,
) -> Result<Option<String>, PatternError> {
    let (Some(output), Some(pattern)) = (output, pattern) else {
        return Ok(None);
    };
    let pattern = RubyPattern::new(pattern)?;
    Ok(pattern
        .captures(output)?
        .and_then(|m| m.group1.map(str::to_string)))
}

/// `run_detection_command` + `try_implementation`'s extraction and presence check.
///
/// `option_executable` is the loader's `options[:executable]`; `shell` its
/// `options[:shell]` (`None`: the runner applies the owner-pending default).
pub fn probe_index_detection(
    detection: &IndexDetection,
    tool_name: &str,
    option_executable: Option<&str>,
    shell: Option<&str>,
    runner: &mut dyn CommandRunner,
) -> Result<IndexProbe, PatternError> {
    let Some(command) = detection.command.as_ref() else {
        return Ok(IndexProbe::NotDetected);
    };
    let mut attempt = |executable: &str| -> Option<String> {
        let request = CommandRequest {
            executable: executable.to_string(),
            args: command.clone(),
            shell: shell.map(str::to_string),
            timeout_secs: INDEX_DETECTION_TIMEOUT_SECS,
            allow_failure: true,
        };
        runner
            .run(&request)
            .ok()
            .as_ref()
            .and_then(detection_output)
    };
    let output = match &detection.executables {
        Some(executables) => executables.iter().find_map(|e| attempt(e)),
        None => {
            let executable = detection
                .executable
                .as_deref()
                .or(option_executable)
                .unwrap_or(tool_name);
            attempt(executable)
        }
    };
    let Some(output) = output else {
        return Ok(IndexProbe::NotDetected);
    };

    // Extraction and the presence check share one compiled pattern (the
    // source compiles it twice). It is compiled only once output exists, as
    // in the source.
    let Some(pattern) = detection.pattern.as_deref() else {
        return Ok(IndexProbe::Detected {
            output,
            version: None,
        });
    };
    let pattern = RubyPattern::new(pattern)?;
    let version = match pattern.captures(&output)? {
        None => return Ok(IndexProbe::NotDetected),
        Some(found) => found.group1.map(str::to_string),
    };
    Ok(IndexProbe::Detected { output, version })
}
