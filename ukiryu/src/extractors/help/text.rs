//! Ruby string semantics the help path depends on, measured against the
//! pinned oracle (Ruby 4.0.1, Onigmo, Unicode 17.0.0).
//!
//! * `\s` is ASCII-only: tab, LF, VT, FF, CR and space.
//! * `[a-z]` under `/i` also matches U+017F (long s) and U+212A (Kelvin
//!   sign), whose simple case folds are `s` and `k`.
//! * `String#strip` removes NUL as well as `\s` from both ends.
//! * `String#downcase`, `#upcase` and `#capitalize` map every code point
//!   independently (no final-sigma context); the tables live in
//!   `case_table.rs`, generated from the oracle.

use super::case_table::{CAPITALIZE, DOWNCASE, UPCASE};
use super::ExtractError;

/// Ruby regexp `\s` (ASCII only).
pub(crate) fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ')
}

/// Ruby `String#strip` whitespace: `\s` plus NUL.
fn is_strip(c: char) -> bool {
    c == '\0' || is_space(c)
}

/// Ruby `/[a-z]/i` (equivalently `/[A-Z]/i`).
pub(crate) fn is_letter_i(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\u{17F}' || c == '\u{212A}'
}

/// Ruby `String#strip` on valid UTF-8.
pub(crate) fn strip(s: &str) -> &str {
    s.trim_matches(is_strip)
}

/// Ruby `String#strip` on bytes that may not be valid UTF-8.
///
/// Ruby scans leading whitespace first and raises `ArgumentError` when that
/// scan meets an invalid sequence; otherwise a broken string raises
/// `Encoding::CompatibilityError` from the trailing scan.
pub(crate) fn strip_bytes(bytes: &[u8]) -> Result<&str, ExtractError> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(strip(s)),
        Err(_) => {
            let mut pos = 0;
            loop {
                let valid = match std::str::from_utf8(&bytes[pos..]) {
                    Ok(s) => s,
                    Err(e) => std::str::from_utf8(&bytes[pos..pos + e.valid_up_to()]).unwrap_or(""),
                };
                match valid.chars().next() {
                    None => return Err(ExtractError::invalid_byte_sequence()),
                    Some(c) if is_strip(c) => pos += c.len_utf8(),
                    Some(_) => return Err(ExtractError::encoding_compatibility()),
                }
            }
        }
    }
}

/// A Ruby regexp match on a string raises `ArgumentError` when the string is
/// not valid UTF-8.
pub(crate) fn regex_input(bytes: &[u8]) -> Result<&str, ExtractError> {
    std::str::from_utf8(bytes).map_err(|_| ExtractError::invalid_byte_sequence())
}

fn lookup(table: &'static [(u32, &'static str)], c: char) -> Option<&'static str> {
    table
        .binary_search_by_key(&(c as u32), |&(cp, _)| cp)
        .ok()
        .map(|i| table[i].1)
}

fn push_mapped(out: &mut String, table: &'static [(u32, &'static str)], c: char) {
    match lookup(table, c) {
        Some(mapped) => out.push_str(mapped),
        None => out.push(c),
    }
}

/// Ruby `String#downcase` on bytes; a broken string raises
/// `ArgumentError: input string invalid`.
pub(crate) fn downcase_bytes(bytes: &[u8]) -> Result<String, ExtractError> {
    let s = std::str::from_utf8(bytes).map_err(|_| ExtractError::input_string_invalid())?;
    let mut out = String::with_capacity(s.len());
    s.chars().for_each(|c| push_mapped(&mut out, DOWNCASE, c));
    Ok(out)
}

/// Ruby `String#upcase`.
pub(crate) fn upcase(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    s.chars().for_each(|c| push_mapped(&mut out, UPCASE, c));
    out
}

/// Ruby `String#capitalize`: titlecase the first character, downcase the rest.
pub(crate) fn capitalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        push_mapped(&mut out, CAPITALIZE, first);
    }
    chars.for_each(|c| push_mapped(&mut out, DOWNCASE, c));
    out
}
