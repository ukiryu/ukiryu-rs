//! Fish and Tcsh formatting, ported from the pinned Ruby sources
//! `lib/ukiryu/shell/{base,fish,tcsh}.rb`.
//!
//! Only pure formatting lives here. Alias detection, shell detection and
//! process execution are separate contracts.

use super::ShellFormatting;

/// The fish shell (`Ukiryu::Shell::Fish`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fish;

/// The TENEX C shell (`Ukiryu::Shell::Tcsh`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tcsh;

/// Ruby `/\s/` on a UTF-8 string: ASCII space, `\t`, `\n`, `\v`, `\f`, `\r` only.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r')
}

/// `Base#needs_quoting?`: empty, whitespace, or one of
/// `SPECIAL_CHARS_PATTERN = /[\s&*()\[\]{}|;<>?`~!@%"]/`.
fn base_needs_quoting(value: &str) -> bool {
    value.is_empty()
        || value.chars().any(|c| {
            is_ruby_space(c)
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
        })
}

/// `"'" -> "'\''"`, the POSIX single-quote break-out used by both shells.
fn escape_single_quotes(value: &str) -> String {
    value.replace('\'', "'\\''")
}

/// `[quote(executable), *args.map { quote(a) }].join(' ')`.
fn join_quoted(shell: &impl ShellFormatting, executable: &str, arguments: &[String]) -> String {
    let mut out = shell.quote(executable);
    for argument in arguments {
        out.push(' ');
        out.push_str(&shell.quote(argument));
    }
    out
}

impl ShellFormatting for Fish {
    fn name(&self) -> &'static str {
        "fish"
    }

    /// Fish reuses the Bash rule: only `'` is rewritten.
    fn escape(&self, value: &str) -> String {
        escape_single_quotes(value)
    }

    fn quote(&self, value: &str) -> String {
        format!("'{}'", self.escape(value))
    }

    fn needs_quoting(&self, value: &str) -> bool {
        base_needs_quoting(value)
    }

    fn format_path(&self, path: &str) -> String {
        path.to_owned()
    }

    fn env_var(&self, name: &str) -> String {
        format!("${name}")
    }

    fn join(&self, executable: &str, arguments: &[String]) -> String {
        join_quoted(self, executable, arguments)
    }
}

impl ShellFormatting for Tcsh {
    fn name(&self) -> &'static str {
        "tcsh"
    }

    /// Backslash-escapes each of `` ! $ ` " \ `` (`/[!$`"\\]/`).
    fn escape(&self, value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for c in value.chars() {
            if matches!(c, '!' | '$' | '`' | '"' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }

    /// Single-quotes after rewriting `!` to `\!` and then `'` to `'\''`.
    /// Unlike `escape`, other characters (including `\`) are left as-is.
    fn quote(&self, value: &str) -> String {
        format!("'{}'", escape_single_quotes(&value.replace('!', "\\!")))
    }

    fn needs_quoting(&self, value: &str) -> bool {
        base_needs_quoting(value)
    }

    fn format_path(&self, path: &str) -> String {
        path.to_owned()
    }

    fn env_var(&self, name: &str) -> String {
        format!("${name}")
    }

    fn join(&self, executable: &str, arguments: &[String]) -> String {
        join_quoted(self, executable, arguments)
    }
}
