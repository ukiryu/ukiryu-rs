//! Bash, Zsh, POSIX sh and Dash formatting, ported from
//! `lib/ukiryu/shell/{unix_base,bash,zsh,sh,dash}.rb` at the pinned oracle
//! commit. All four shells share the same single-quote rules in the source.
//!
//! This module only formats strings. It never runs a shell.

use super::{base, ShellFormatting};

/// Replaces every `'` with `'\''` (close quote, escaped quote, reopen quote).
/// Every other byte, including newlines and NUL, is left as it is.
pub fn escape_single_quotes(value: &str) -> String {
    value.replace('\'', "'\\''")
}

/// Wraps [`escape_single_quotes`] output in single quotes.
pub fn single_quote(value: &str) -> String {
    let escaped = escape_single_quotes(value);
    let mut quoted = String::with_capacity(escaped.len() + 2);
    quoted.push('\'');
    quoted.push_str(&escaped);
    quoted.push('\'');
    quoted
}

/// `$NAME` with the name inserted unchanged; the source does not validate it.
pub fn dollar_env_var(name: &str) -> String {
    format!("${name}")
}

macro_rules! posix_shell {
    ($(#[$doc:meta])* $ty:ident, $name:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $ty;

        impl $ty {
            /// The source `EXECUTABLE` constant, also returned by `shell_command`.
            pub const EXECUTABLE: &'static str = $name;
        }

        impl ShellFormatting for $ty {
            fn name(&self) -> &'static str {
                $name
            }

            fn escape(&self, value: &str) -> String {
                escape_single_quotes(value)
            }

            fn quote(&self, value: &str) -> String {
                single_quote(value)
            }

            fn needs_quoting(&self, value: &str) -> bool {
                base::needs_quoting(value)
            }

            fn format_path(&self, path: &str) -> String {
                base::format_path(path)
            }

            fn env_var(&self, name: &str) -> String {
                dollar_env_var(name)
            }

            fn join(&self, executable: &str, arguments: &[String]) -> String {
                base::join_quoted(executable, arguments, single_quote)
            }
        }
    };
}

posix_shell!(
    /// `Ukiryu::Shell::Bash`.
    Bash,
    "bash"
);
posix_shell!(
    /// `Ukiryu::Shell::Zsh`.
    Zsh,
    "zsh"
);
posix_shell!(
    /// `Ukiryu::Shell::Sh` (POSIX sh).
    Sh,
    "sh"
);
posix_shell!(
    /// `Ukiryu::Shell::Dash` (Debian Almquist shell).
    Dash,
    "dash"
);
