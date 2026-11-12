//! Pure helpers shared by every shell, ported from `Ukiryu::Shell::Base`
//! (`lib/ukiryu/shell/base.rb` at the pinned oracle commit).
//!
//! Only the side-effect-free parts of the Ruby base class live here. Alias
//! detection, capabilities and process execution have separate contracts.

/// Ruby's `/\s/` character class: space, tab, line feed, vertical tab,
/// form feed and carriage return. Unicode spaces such as U+00A0 do not match.
pub fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r')
}

/// Characters from Ruby's `SPECIAL_CHARS_PATTERN`
/// (`/[\s&*()\[\]{}|;<>?`~!@%"]/`), excluding the whitespace part.
///
/// `$`, `'`, `\`, `^`, `#` and `=` are deliberately absent, as in the source.
pub const SPECIAL_CHARS: &[char] = &[
    '&', '*', '(', ')', '[', ']', '{', '}', '|', ';', '<', '>', '?', '`', '~', '!', '@', '%', '"',
];

/// `Base#needs_quoting?`: true for the empty string, for any Ruby `\s`
/// whitespace, and for any character in [`SPECIAL_CHARS`].
pub fn needs_quoting(value: &str) -> bool {
    value.is_empty()
        || value
            .chars()
            .any(|c| is_whitespace(c) || SPECIAL_CHARS.contains(&c))
}

/// `Base#format_path`: the path unchanged.
pub fn format_path(path: &str) -> String {
    path.to_owned()
}

/// Quotes the executable and every argument with `quote` and joins them with
/// single spaces, the shape every Unix shell's `join(executable, *args)` uses.
/// The executable is always quoted, even when it has no special characters.
pub fn join_quoted<F>(executable: &str, arguments: &[String], quote: F) -> String
where
    F: Fn(&str) -> String,
{
    let mut line = quote(executable);
    for argument in arguments {
        line.push(' ');
        line.push_str(&quote(argument));
    }
    line
}
