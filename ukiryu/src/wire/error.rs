//! Error replies. The wire owns the stable `code`; `source_class` is the
//! Ruby exception class the original raises (the public error contract of
//! `docs/reference/error-codes`), present whenever the failure has one.
//!
//! | code | meaning |
//! |---|---|
//! | `INVALID_REQUEST` | the request is not valid JSON or not a known operation shape |
//! | `INVALID_ARGUMENT` | a request field is out of range for the operation |
//! | `UNSUPPORTED_CAPABILITY` | the operation is not available in this build (WebAssembly) |
//! | `INTERNAL_ERROR` | a core defect (a panic or an impossible state) |
//! | `NOT_PORTED` | the input reaches source behaviour this port does not reproduce |
//! | `PROCESS_ERROR` | the process could not be started or its output collected |
//! | one code per `Ukiryu::Errors` class (`TOOL_NOT_FOUND`, `VALIDATION_ERROR`, ...) | that class |
//! | `RUBY_EXCEPTION` | any other Ruby exception class (`ArgumentError`, `TypeError`, ...) |
use serde_json::{json, Value as Json};

use crate::errors::ErrorKind;
use crate::models::ModelError;
use crate::schema::SchemaError;
use crate::tool::ToolError;
use crate::version_ranges::{self, RangeError};

/// The stable code of a `Ukiryu::Errors` class.
pub fn kind_code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Error => "UKIRYU_ERROR",
        ErrorKind::UnknownShell => "UNKNOWN_SHELL",
        ErrorKind::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
        ErrorKind::Validation => "VALIDATION_ERROR",
        ErrorKind::ProfileNotFound => "PROFILE_NOT_FOUND",
        ErrorKind::ProfileLoad => "PROFILE_LOAD_ERROR",
        ErrorKind::Definition => "DEFINITION_ERROR",
        ErrorKind::DefinitionNotFound => "DEFINITION_NOT_FOUND",
        ErrorKind::DefinitionLoad => "DEFINITION_LOAD_ERROR",
        ErrorKind::DefinitionValidation => "DEFINITION_VALIDATION_ERROR",
        ErrorKind::Load => "LOAD_ERROR",
        ErrorKind::ToolNotFound => "TOOL_NOT_FOUND",
        ErrorKind::ExecutableNotFound => "EXECUTABLE_NOT_FOUND",
        ErrorKind::Execution => "EXECUTION_FAILED",
        ErrorKind::Timeout => "TIMEOUT",
        ErrorKind::VersionDetection => "VERSION_DETECTION_ERROR",
    }
}

/// The stable code of a Ruby exception class.
pub fn class_code(class: &str) -> &'static str {
    ErrorKind::from_ruby_class(class).map_or("RUBY_EXCEPTION", kind_code)
}

/// A failure of the transport or the request itself (no Ruby class).
pub fn transport(code: &str, message: impl ToString) -> Json {
    json!({"ok": false, "error": {"code": code, "message": message.to_string()}})
}

/// A failure the original raises as `class`.
pub fn ruby(class: &str, message: impl ToString) -> Json {
    json!({"ok": false, "error": {
        "code": class_code(class),
        "source_class": class,
        "message": message.to_string(),
    }})
}

pub fn model_error(error: &ModelError) -> Json {
    if matches!(error.kind, crate::models::ModelErrorKind::Unsupported) {
        return transport("NOT_PORTED", error.message.clone());
    }
    ruby(error.ruby_class(), error.message.clone())
}

pub fn range_error(error: &RangeError) -> Json {
    if error.class == version_ranges::ErrorClass::Unsupported {
        return transport("NOT_PORTED", error.message.clone());
    }
    ruby(error.class.ruby_name(), error.message.clone())
}

pub fn schema_error(error: &SchemaError) -> Json {
    match error.ruby_class() {
        Some(class) => ruby(&class, error.message()),
        None => transport("NOT_PORTED", error.message()),
    }
}

/// The message Ruby's `Tool::CommandResolution` raises when several
/// profiles exist and none matches: it names a constant it cannot resolve.
pub const PROFILE_NOT_FOUND_NAME_ERROR: &str =
    "uninitialized constant Ukiryu::Tool::CommandResolution::ProfileNotFoundError";

pub fn tool_error(error: &ToolError) -> Json {
    match error {
        ToolError::NotFound { .. } => ruby(ErrorKind::ToolNotFound.ruby_class(), error),
        // With several profiles and none matching, the core names the
        // profile it could not find ("... Current: platform/shell") where the
        // original raises NameError for an unresolvable constant. Keep the
        // original. Without profiles the original raises the real class.
        ToolError::ProfileNotFound { message } if message.contains(". Current: ") => {
            ruby("NameError", PROFILE_NOT_FOUND_NAME_ERROR)
        }
        ToolError::ProfileNotFound { message } => {
            ruby(ErrorKind::ProfileNotFound.ruby_class(), message)
        }
        ToolError::Argument { message } => ruby("ArgumentError", message),
        // Callers of the wire never see this: build reports a missing
        // executable as null and execute follows the original (see native).
        ToolError::ExecutableNotFound { .. } => transport("INTERNAL_ERROR", error),
        ToolError::Build(e) => super::ops::types_error(e),
        ToolError::Ruby { class, message } => ruby(class, message),
        ToolError::Source { message } => transport("NOT_PORTED", message),
        ToolError::Execute(e) => execute_error(e),
    }
}

/// An executor failure. A finished run that failed keeps its result.
pub fn execute_error(error: &crate::executor::ExecuteError) -> Json {
    use crate::executor::ExecuteError as E;
    match error {
        E::InvalidTimeout(_) => transport("INVALID_ARGUMENT", error),
        E::ShellNotFound { message, .. } => ruby("RuntimeError", message),
        E::Spawn { .. } | E::Stdin(_) | E::Io(_) => transport("PROCESS_ERROR", error),
        E::TimedOut(e) => {
            let mut reply = ruby(e.ruby_class(), e.message());
            reply["error"]["timeout"] = e.timeout().map_or(Json::Null, |t| match t {
                crate::execution::Seconds::Int(i) => json!(i),
                crate::execution::Seconds::Float(f) => super::value::float(f),
            });
            reply
        }
        E::Failed {
            error: e,
            execution,
        } => {
            let mut reply = ruby(e.ruby_class(), e.message());
            reply["error"]["result"] = super::results::execution(execution);
            reply
        }
        E::InvalidByteSequence {
            error: e,
            execution,
        } => {
            let mut reply = ruby(e.ruby_class(), e.message());
            reply["error"]["result"] = super::results::execution(execution);
            reply
        }
    }
}
