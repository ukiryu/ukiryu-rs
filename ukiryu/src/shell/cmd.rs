//! Windows `cmd.exe` formatting, ported from `Ukiryu::Shell::Cmd`
//! (`lib/ukiryu/shell/cmd.rb`) and the `needs_quoting?` rule it inherits from
//! `Ukiryu::Shell::Base` (`lib/ukiryu/shell/base.rb`).
//!
//! The Ruby bytes are the contract, including its quirks: `quote` wraps
//! whitespace-bearing values in double quotes without escaping embedded quotes
//! or carets, and `join` passes the arguments after a leading `/c` through
//! `escape` only. All whitespace classes here are ASCII-only, matching Ruby's
//! `\s` on UTF-8 strings.

use super::ShellFormatting;

/// Formatting rules for Windows `cmd.exe`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cmd;

/// `Cmd#escape` regex `/[%^<>&|]/`.
fn is_caret_escaped(c: char) -> bool {
    matches!(c, '%' | '^' | '<' | '>' | '&' | '|')
}

/// `Cmd::WHITESPACE_PATTERN` (`/[ \t]/`), used by `Cmd#quote`.
fn is_cmd_quote_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t')
}

/// `Base::WHITESPACE_PATTERN` (`/\s/`): ASCII space, `\t`, `\n`, `\v`, `\f`, `\r`.
fn is_base_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// `Base::SPECIAL_CHARS_PATTERN` (`/[\s&*()\[\]{}|;<>?`~!@%"]/`).
fn is_base_special(c: char) -> bool {
    is_base_whitespace(c)
        || matches!(
            c,
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

impl ShellFormatting for Cmd {
    fn name(&self) -> &'static str {
        "cmd"
    }

    /// Prefixes each of `% ^ < > & |` with a caret.
    fn escape(&self, value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for c in value.chars() {
            if is_caret_escaped(c) {
                out.push('^');
            }
            out.push(c);
        }
        out
    }

    /// Wraps the raw value in double quotes when it contains a space or tab;
    /// otherwise falls back to [`Cmd::escape`].
    fn quote(&self, value: &str) -> String {
        if value.chars().any(is_cmd_quote_whitespace) {
            let mut out = String::with_capacity(value.len() + 2);
            out.push('"');
            out.push_str(value);
            out.push('"');
            out
        } else {
            self.escape(value)
        }
    }

    /// Inherited `Base#needs_quoting?`: empty, ASCII whitespace, or a shell
    /// special character.
    fn needs_quoting(&self, value: &str) -> bool {
        value.is_empty() || value.chars().any(is_base_special)
    }

    /// Converts every `/` to `\`.
    fn format_path(&self, path: &str) -> String {
        path.replace('/', "\\")
    }

    fn env_var(&self, name: &str) -> String {
        format!("%{name}%")
    }

    /// Joins with single spaces. A leading `/c` followed by at least one more
    /// argument keeps the remainder as one unquoted, caret-escaped command
    /// string; otherwise each argument is quoted when it needs quoting and
    /// escaped when it does not.
    fn join(&self, executable: &str, arguments: &[String]) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(arguments.len() + 1);
        parts.push(self.format_word(executable));
        match arguments {
            [flag, rest @ ..] if flag == "/c" && !rest.is_empty() => {
                parts.push(flag.clone());
                let command: Vec<String> = rest.iter().map(|a| self.escape(a)).collect();
                parts.push(command.join(" "));
            }
            _ => parts.extend(arguments.iter().map(|a| self.format_word(a))),
        }
        parts.join(" ")
    }
}

impl Cmd {
    fn format_word(&self, value: &str) -> String {
        if self.needs_quoting(value) {
            self.quote(value)
        } else {
            self.escape(value)
        }
    }
}
