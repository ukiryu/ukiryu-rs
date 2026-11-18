//! `escape` kind: `shell.*` ops through the core's per-shell formatters
//! (`ukiryu::shell`, the `ShellFormatting` trait every engine boundary uses).
use ukiryu::shell::cmd::Cmd;
use ukiryu::shell::posix::{Bash, Dash, Sh, Zsh};
use ukiryu::shell::powershell::PowerShell;
use ukiryu::shell::unix_special::{Fish, Tcsh};
use ukiryu::shell::ShellFormatting;

use super::harness::{input_str, Ctx, Outcome, RowError};
use super::json::Oj;

/// The oracle's `SHELLS` table: corpus shell name to formatter.
pub fn shell(name: &str) -> Result<&'static dyn ShellFormatting, RowError> {
    Ok(match name {
        "bash" => &Bash,
        "zsh" => &Zsh,
        "fish" => &Fish,
        "sh" => &Sh,
        "dash" => &Dash,
        "tcsh" => &Tcsh,
        "powershell" => &PowerShell,
        "cmd" => &Cmd,
        other => return Err(RowError::harness(format!("unknown shell {other:?}"))),
    })
}

/// Runs one `shell.*` op on the given formatter. Shared with the negative
/// control that substitutes a wrong formatter.
pub fn run_with(op: &str, sh: &dyn ShellFormatting, input: &Oj) -> Outcome {
    Ok(match op {
        "shell.escape" => Oj::Str(sh.escape(input_str(input, "string")?)),
        "shell.quote" => Oj::Str(sh.quote(input_str(input, "string")?)),
        "shell.needs_quoting" => Oj::Bool(sh.needs_quoting(input_str(input, "string")?)),
        "shell.env_var" => Oj::Str(sh.env_var(input_str(input, "name")?)),
        "shell.format_path" => Oj::Str(sh.format_path(input_str(input, "path")?)),
        "shell.join" => {
            let args = input
                .get("args")
                .and_then(Oj::arr)
                .ok_or_else(|| RowError::harness("shell.join needs args"))?
                .iter()
                .map(|a| {
                    a.str()
                        .map(str::to_string)
                        .ok_or_else(|| RowError::harness("shell.join arg must be a string"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Oj::Str(sh.join(input_str(input, "executable")?, &args))
        }
        other => return Err(RowError::harness(format!("not a shell op {other}"))),
    })
}

pub fn run(op: &str, input: &Oj, _ctx: &Ctx) -> Outcome {
    run_with(op, shell(input_str(input, "shell")?)?, input)
}
