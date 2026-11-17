//! The result of a finished run in the wire encoding.
//!
//! Streams are the exact bytes the child wrote: a UTF-8 stream is a string,
//! any other stream is `{"$bytes": hex}` (never replaced characters). Start
//! and finish times are exact `$time` values; the duration is the Ruby
//! Float the result computes, as `$float`.
use serde_json::{json, Value as Json};

use super::value;
use crate::execution::Seconds;
use crate::executor::Execution;

fn seconds(s: Seconds) -> Json {
    match s {
        Seconds::Int(i) => json!(i),
        Seconds::Float(f) => value::float(f),
    }
}

/// `Execution::Result` with its raw output.
pub fn execution(run: &Execution) -> Json {
    let result = &run.result;
    let command = &result.command_info;
    let metadata = &result.metadata;
    json!({
        "command": {
            "executable": command.executable,
            "executable_name": command.executable_name(),
            "arguments": command.arguments,
            "full_command": command.full_command,
            "shell": command.shell,
            "tool_name": command.tool_name,
            "command_name": command.command_name,
        },
        "stdout": value::bytes(&run.stdout),
        "stderr": value::bytes(&run.stderr),
        "status": result.status(),
        "success": result.is_success(),
        "started_at": metadata.started_at.as_ref().map(value::time),
        "finished_at": metadata.finished_at.as_ref().map(value::time),
        "duration": metadata.duration().map(value::float),
        "timeout": metadata.timeout.map(seconds),
        "timed_out": metadata.is_timed_out(),
    })
}
