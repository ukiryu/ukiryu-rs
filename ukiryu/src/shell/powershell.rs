//! PowerShell formatting, ported from `lib/ukiryu/shell/powershell.rb` and the
//! `needs_quoting?` checks it inherits from `lib/ukiryu/shell/base.rb`.
//!
//! Arguments are wrapped in double quotes with backtick escapes, while
//! [`PowerShell::escape`] doubles single quotes for single-quoted literals.
//! The invocation helpers build the `-Command` argv that the Ruby executor
//! hands to the process layer; they do not spawn anything.

use super::ShellFormatting;

/// The PowerShell shell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PowerShell;

/// Host family that selects the PowerShell executable and the argument
/// quoting rules of the invocation helpers. Detecting the host is the
/// caller's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationHost {
    /// Windows: runs `powershell`.
    Windows,
    /// Every other platform: runs `pwsh` (PowerShell Core).
    Unix,
}

impl PowerShell {
    /// Default executable name (Ruby `EXECUTABLE`).
    pub const EXECUTABLE: &'static str = "pwsh";

    /// Escape a value for a double-quoted PowerShell string: backtick,
    /// double quote and dollar each get a backtick prefix. Nothing else
    /// changes, including newlines and single quotes.
    pub fn escape_for_double_quotes(&self, value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            if matches!(ch, '`' | '"' | '$') {
                out.push('`');
            }
            out.push(ch);
        }
        out
    }

    /// The PowerShell executable for a host: `powershell` on Windows, `pwsh`
    /// elsewhere.
    pub fn powershell_command(&self, host: InvocationHost) -> &'static str {
        match host {
            InvocationHost::Windows => "powershell",
            InvocationHost::Unix => "pwsh",
        }
    }

    /// The argv the Ruby `execute_command` passes to the process layer:
    /// `[powershell_command, "-NoLogo", "-Command", "& <command>; exit $LASTEXITCODE"]`.
    pub fn command_argv(
        &self,
        executable: &str,
        arguments: &[String],
        host: InvocationHost,
    ) -> Vec<String> {
        let command = call_command(executable, arguments, host, false);
        self.invocation(format!("& {command}; exit $LASTEXITCODE"), host)
    }

    /// The argv the Ruby `execute_command_with_stdin` passes to the process
    /// layer, piping the file at `stdin_path` into the command through
    /// `Get-Content`. On Windows this variant also turns `/` into `\` inside
    /// arguments, unlike [`PowerShell::command_argv`]; the source does the same.
    pub fn stdin_command_argv(
        &self,
        executable: &str,
        arguments: &[String],
        stdin_path: &str,
        host: InvocationHost,
    ) -> Vec<String> {
        let command = call_command(executable, arguments, host, true);
        self.invocation(
            format!(
                "Get-Content {} | & {command}; exit $LASTEXITCODE",
                single_quoted(stdin_path)
            ),
            host,
        )
    }

    fn invocation(&self, script: String, host: InvocationHost) -> Vec<String> {
        vec![
            self.powershell_command(host).to_owned(),
            "-NoLogo".to_owned(),
            "-Command".to_owned(),
            script,
        ]
    }
}

/// Ruby `\s` on a UTF-8 string: ASCII space, tab, LF, VT, FF and CR only.
fn is_ruby_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r')
}

/// Ruby `Base::SPECIAL_CHARS_PATTERN` (`[\s&*()\[\]{}|;<>?`~!@%"]`).
fn is_special(ch: char) -> bool {
    is_ruby_whitespace(ch)
        || matches!(
            ch,
            '&' | '*'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '|'
                | ';'
                | '<'
                | '>'
                | '?'
                | '`'
                | '~'
                | '!'
                | '@'
                | '%'
                | '"'
        )
}

/// A single-quoted PowerShell literal with embedded single quotes doubled.
fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The `'<exe>' <args...>` part of the Ruby executor's command script.
fn call_command(
    executable: &str,
    arguments: &[String],
    host: InvocationHost,
    stdin_variant: bool,
) -> String {
    let exe = match host {
        InvocationHost::Windows => executable.replace('/', "\\"),
        InvocationHost::Unix => executable.to_owned(),
    };
    let mut parts = Vec::with_capacity(arguments.len() + 1);
    parts.push(single_quoted(&exe));
    for argument in arguments {
        let value = match host {
            InvocationHost::Windows if stdin_variant => argument.replace('/', "\\"),
            _ => argument.clone(),
        };
        let needs_literal = match host {
            InvocationHost::Windows => {
                value.starts_with('-')
                    || value.contains('$')
                    || value.contains('`')
                    || value.contains(' ')
                    || value.contains('"')
            }
            InvocationHost::Unix => {
                value.contains(' ')
                    || value.starts_with('-')
                    || value.contains('$')
                    || value.contains('`')
                    || value.contains(';')
            }
        };
        parts.push(if needs_literal {
            single_quoted(&value)
        } else {
            value
        });
    }
    parts.join(" ")
}

impl ShellFormatting for PowerShell {
    fn name(&self) -> &'static str {
        "powershell"
    }

    /// Doubles single quotes, for use inside a single-quoted literal.
    fn escape(&self, value: &str) -> String {
        value.replace('\'', "''")
    }

    /// Always double quotes, with [`PowerShell::escape_for_double_quotes`]
    /// applied. The Ruby `for_exe:` keyword does not change the result.
    fn quote(&self, value: &str) -> String {
        format!("\"{}\"", self.escape_for_double_quotes(value))
    }

    /// Base rules (empty, ASCII whitespace, special characters) plus a
    /// leading `-` (parameter binder) or any `$` (variable expansion).
    fn needs_quoting(&self, value: &str) -> bool {
        value.is_empty()
            || value.chars().any(is_special)
            || value.starts_with('-')
            || value.contains('$')
    }

    fn format_path(&self, path: &str) -> String {
        path.to_owned()
    }

    fn env_var(&self, name: &str) -> String {
        format!("$ENV:{name}")
    }

    /// Quotes the executable and each argument only when needed. The argument
    /// right after an exact `-Command` or `-File` is passed through unquoted.
    fn join(&self, executable: &str, arguments: &[String]) -> String {
        let mut parts = Vec::with_capacity(arguments.len() + 1);
        parts.push(if self.needs_quoting(executable) {
            self.quote(executable)
        } else {
            executable.to_owned()
        });
        let mut skip_quote = false;
        for argument in arguments {
            let formatted = if skip_quote {
                skip_quote = false;
                argument.clone()
            } else if argument == "-Command" || argument == "-File" {
                skip_quote = true;
                argument.clone()
            } else if self.needs_quoting(argument) {
                self.quote(argument)
            } else {
                argument.clone()
            };
            parts.push(formatted);
        }
        parts.join(" ")
    }
}
