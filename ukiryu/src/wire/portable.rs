//! Pure shell formatting over the existing implementations; no host detection.
use crate::cache::shell_instances::{builtin_shell, UnknownShellError};
use crate::runtime::{ShellName, ShellSource};
use crate::shell::powershell::PowerShell;
use serde::Deserialize;
use serde_json::{json, Value};

/// Explicit formatter and typed action. Names are concrete registered shells or
/// their slash-delimited basenames, independent of the machine running the core.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscapeRequest {
    pub shell: String,
    pub operation: EscapeOperation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum EscapeOperation {
    Escape {
        string: String,
    },
    Quote {
        string: String,
        for_exe: Option<bool>,
    },
    NeedsQuoting {
        string: String,
    },
    FormatPath {
        path: String,
    },
    EnvVar {
        name: String,
    },
    Join {
        executable: String,
        args: Vec<String>,
    },
    EscapeForDoubleQuotes {
        string: String,
    },
}

pub fn escape(request: EscapeRequest) -> Value {
    // Match the runtime's lexical basename rule; using std::path here would
    // make the same JSON request mean different things on Windows and Unix.
    let trimmed = request.shell.trim_end_matches('/');
    let name = if trimmed.is_empty() && !request.shell.is_empty() {
        "/"
    } else {
        trimmed.rsplit('/').next().unwrap_or(trimmed)
    };
    let shell_name = match ShellName::parse(name, ShellSource::Explicit) {
        Ok(name) => name,
        Err(error) => {
            return json!({"ok":false,"error":{
                "code":"UNKNOWN_SHELL", "source_class":UnknownShellError::RUBY_CLASS,
                "message":error.to_string()
            }})
        }
    };
    // Strict runtime lookup above excludes the legacy unix/windows/pwsh aliases
    // accepted by the source factory. No default formatter can enter this API.
    let shell = match builtin_shell(shell_name.as_str()) {
        Ok(shell) => shell,
        Err(error) => return super::failure("INTERNAL_ERROR", error),
    };
    let value = match request.operation {
        EscapeOperation::Escape { string } => json!(shell.escape(&string)),
        EscapeOperation::Quote { string, for_exe } => {
            if for_exe.is_some() && shell_name != ShellName::PowerShell {
                return super::failure("INVALID_ARGUMENT", "for_exe is a PowerShell quote option");
            }
            // Ruby accepts this keyword but ignores its value; preserve that.
            json!(shell.quote(&string))
        }
        EscapeOperation::NeedsQuoting { string } => json!(shell.needs_quoting(&string)),
        EscapeOperation::FormatPath { path } => json!(shell.format_path(&path)),
        EscapeOperation::EnvVar { name } => json!(shell.env_var(&name)),
        EscapeOperation::Join { executable, args } => json!(shell.join(&executable, &args)),
        EscapeOperation::EscapeForDoubleQuotes { string } => {
            if shell_name != ShellName::PowerShell {
                return super::failure(
                    "INVALID_ARGUMENT",
                    "escape_for_double_quotes requires PowerShell",
                );
            }
            json!(PowerShell.escape_for_double_quotes(&string))
        }
    };
    json!({"ok":true,"value":value})
}
