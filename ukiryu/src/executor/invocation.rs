//! The process each shell runs and the environment it receives, following
//! the `execute_command` / `execute_command_with_stdin` methods of
//! lib/ukiryu/shell/{unix_base,tcsh,cmd,powershell}.rb and
//! `Executor.prepare_environment`.

use std::ffi::OsStr;

use crate::environment::Environment;
use crate::executable::is_executable;
use crate::runtime::ShellName;
use crate::shell::cmd::Cmd;
use crate::shell::posix::{Bash, Dash, Sh, Zsh};
use crate::shell::powershell::{InvocationHost, PowerShell};
use crate::shell::unix_special::{Fish, Tcsh};
use crate::shell::ShellFormatting;

/// The formatter whose `join` builds the command line for `shell`.
pub(super) fn formatter(shell: ShellName) -> &'static dyn ShellFormatting {
    match shell {
        ShellName::Bash => &Bash,
        ShellName::Zsh => &Zsh,
        ShellName::Fish => &Fish,
        ShellName::Sh => &Sh,
        ShellName::Dash => &Dash,
        ShellName::Tcsh => &Tcsh,
        ShellName::PowerShell => &PowerShell,
        ShellName::Cmd => &Cmd,
    }
}

/// The host facts the source reads while building an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Host {
    pub windows: bool,
    pub macos: bool,
}

impl Host {
    pub(super) fn current() -> Host {
        Host {
            windows: cfg!(windows),
            macos: cfg!(target_os = "macos"),
        }
    }
}

/// `Executor.find_executable(command)` without additional paths: each
/// `PATH` entry, trying every `PATHEXT` extension when that variable is set
/// and the bare name otherwise. `search_path` and `pathext` are the values to
/// search; the process environment is not read here.
pub fn find_executable_in(
    command: &str,
    search_path: Option<&OsStr>,
    pathext: Option<&str>,
    windows: bool,
) -> Option<String> {
    let exts: Vec<&str> = match pathext {
        Some(value) => value.split(';').collect(),
        None => vec![""],
    };
    let separator = if windows { ';' } else { ':' };
    let path = search_path?.to_str()?;
    // Ruby `String#split` drops trailing empty fields.
    let mut dirs: Vec<&str> = path.split(separator).collect();
    while dirs.last() == Some(&"") {
        dirs.pop();
    }
    let mut seen: Vec<&str> = Vec::new();
    for dir in dirs {
        // `search_paths.uniq!` keeps the first occurrence of each entry.
        if seen.contains(&dir) {
            continue;
        }
        seen.push(dir);
        for ext in &exts {
            let candidate = format!("{dir}/{command}{ext}");
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// The `RuntimeError` text the source raises when a Unix shell is missing.
pub(super) fn shell_not_found_message(shell: ShellName) -> String {
    match shell {
        ShellName::Tcsh => "tcsh not found in system PATH".to_owned(),
        other => format!("Shell '{}' not found in system PATH", other.as_str()),
    }
}

/// Why no argv could be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellMissing(pub String);

/// The argv for one run. Unix shells run `<shell path> -c <join>` with the
/// shell found on `search_path`; cmd runs `cmd /c <join>`; PowerShell uses
/// the merged `command_argv` / `stdin_command_argv` builders.
pub(super) fn argv(
    shell: ShellName,
    executable: &str,
    arguments: &[String],
    stdin_path: Option<&str>,
    host: Host,
    search_path: Option<&OsStr>,
    pathext: Option<&str>,
) -> Result<Vec<String>, ShellMissing> {
    let line = || formatter(shell).join(executable, arguments);
    match shell {
        ShellName::Cmd => Ok(vec!["cmd".to_owned(), "/c".to_owned(), line()]),
        ShellName::PowerShell => {
            let ps_host = if host.windows {
                InvocationHost::Windows
            } else {
                InvocationHost::Unix
            };
            Ok(match stdin_path {
                Some(path) => PowerShell.stdin_command_argv(executable, arguments, path, ps_host),
                None => PowerShell.command_argv(executable, arguments, ps_host),
            })
        }
        unix => {
            let program = find_executable_in(unix.as_str(), search_path, pathext, host.windows)
                .ok_or_else(|| ShellMissing(shell_not_found_message(unix)))?;
            Ok(vec![program, "-c".to_owned(), line()])
        }
    }
}

/// Environment changes applied over the inherited process environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct EnvPlan {
    /// Variables set for the child, in order.
    pub set: Vec<(String, String)>,
    /// Variables removed from the child.
    pub remove: Vec<String>,
}

/// `prepare_environment`: the caller's variables, then the shell's headless
/// variables, with `DISPLAY` removed.
///
/// The source deletes `DISPLAY` from the hash it passes to `Open3`, which
/// only stops it from being set: a `DISPLAY` inherited from the parent
/// process still reaches the child. The target removes it from the child,
/// which is what the source's comments say is intended.
pub(super) fn env_plan(environment: &Environment, shell: ShellName, host: Host) -> EnvPlan {
    let mut env = environment.delete("DISPLAY");
    if shell == ShellName::Bash && host.macos {
        for (key, value) in [
            ("NSAppleEventsSuppressStartupAlert", "true"),
            ("NSUIElement", "1"),
            ("GDK_BACKEND", "x11"),
        ] {
            env = env.set(key, value);
        }
    }
    EnvPlan {
        set: env.to_pairs(),
        remove: vec!["DISPLAY".to_owned()],
    }
}
