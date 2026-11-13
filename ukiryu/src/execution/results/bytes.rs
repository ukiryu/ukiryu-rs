//! Ruby `String` methods on captured process output.
//!
//! `Open3` returns stdout and stderr as Strings tagged with the default
//! external encoding (UTF-8 in the pinned oracle) without validating them,
//! so a stream can hold any bytes. The executor keeps them as bytes, and the
//! readers of `Ukiryu::Execution::Output` apply the Ruby methods below, which
//! either work on the bytes or raise for an invalid byte sequence. No step
//! replaces invalid bytes with U+FFFD: Ruby never does at these stages.
//!
//! Observed with Ruby 4.0.1 (`tests/stream_values_data/observations.json`):
//!
//! - `strip` raises for any string with an invalid sequence. `ArgumentError`
//!   when the leading NUL/whitespace scan reaches the invalid sequence,
//!   otherwise `Encoding::CompatibilityError` (raised by the trailing scan).
//! - `split(/\r?\n/)` and Regexp matching raise `ArgumentError`.
//! - `include?` with a valid needle and `length` work on the bytes; `length`
//!   counts each byte of an invalid sequence as one character.
//! - Lutaml's `:string` attribute cast (`ensure_utf8`) re-encodes with
//!   `invalid: :replace, replace: ""`: every invalid sequence is removed.
use std::fmt;

use super::ruby::split_lines as split_valid_lines;

/// The Ruby exception class raised for an invalid byte sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidByteSequenceClass {
    /// `ArgumentError`
    Argument,
    /// `Encoding::CompatibilityError`
    EncodingCompatibility,
}

/// `invalid byte sequence in UTF-8`, raised by a String method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidByteSequence {
    pub class: InvalidByteSequenceClass,
}

impl InvalidByteSequence {
    pub const MESSAGE: &'static str = "invalid byte sequence in UTF-8";

    pub fn argument() -> Self {
        InvalidByteSequence {
            class: InvalidByteSequenceClass::Argument,
        }
    }

    pub fn encoding_compatibility() -> Self {
        InvalidByteSequence {
            class: InvalidByteSequenceClass::EncodingCompatibility,
        }
    }

    /// The Ruby class name.
    pub fn ruby_class(&self) -> &'static str {
        match self.class {
            InvalidByteSequenceClass::Argument => "ArgumentError",
            InvalidByteSequenceClass::EncodingCompatibility => "Encoding::CompatibilityError",
        }
    }

    /// The Ruby message.
    pub fn message(&self) -> &'static str {
        Self::MESSAGE
    }
}

impl fmt::Display for InvalidByteSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::MESSAGE)
    }
}

impl std::error::Error for InvalidByteSequence {}

/// The characters `String#strip` removes from both ends.
fn is_strip_char(c: char) -> bool {
    matches!(c, '\0' | ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')
}

/// The bytes as text when they are valid UTF-8; otherwise the error a
/// method that needs valid text (`split`, Regexp matching) raises.
pub fn valid_text(bytes: &[u8]) -> Result<&str, InvalidByteSequence> {
    std::str::from_utf8(bytes).map_err(|_| InvalidByteSequence::argument())
}

/// `String#strip`.
pub fn strip(bytes: &[u8]) -> Result<&str, InvalidByteSequence> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.trim_matches(is_strip_char)),
        Err(error) => {
            let valid = std::str::from_utf8(&bytes[..error.valid_up_to()])
                .expect("the prefix before the first error is valid");
            if valid.chars().all(is_strip_char) {
                Err(InvalidByteSequence::argument())
            } else {
                Err(InvalidByteSequence::encoding_compatibility())
            }
        }
    }
}

/// `string.split(/\r?\n/)`: trailing empty fields are dropped.
pub fn split_lines(bytes: &[u8]) -> Result<Vec<String>, InvalidByteSequence> {
    valid_text(bytes).map(split_valid_lines)
}

/// `String#length`: characters, where each byte of an invalid sequence is
/// one character.
pub fn length(bytes: &[u8]) -> usize {
    let mut count = 0;
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => return count + text.chars().count(),
            Err(error) => {
                let valid = error.valid_up_to();
                count += std::str::from_utf8(&rest[..valid])
                    .expect("valid prefix")
                    .chars()
                    .count();
                // One character for the first byte of the invalid sequence.
                count += 1;
                rest = &rest[valid + 1..];
            }
        }
    }
    count
}

/// `String#include?(needle)` for a valid UTF-8 needle: a byte search. A
/// valid needle starts on a character head, so the match position check
/// Ruby applies never rejects a byte match.
pub fn contains(bytes: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    needle.is_empty() || bytes.windows(needle.len()).any(|w| w == needle)
}

/// Lutaml's `ensure_utf8` cast of a String assigned to a `:string`
/// attribute: `encode("UTF-8", invalid: :replace, undef: :replace,
/// replace: "")`, which for UTF-8 input is `scrub("")`.
pub fn ensure_utf8(bytes: &[u8]) -> String {
    crate::version_detection::text::scrub_remove(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_classes() {
        assert_eq!(strip(b"\0 a \0"), Ok("a"));
        assert_eq!(strip(b" \xffa"), Err(InvalidByteSequence::argument()));
        assert_eq!(
            strip(b"a\xff"),
            Err(InvalidByteSequence::encoding_compatibility())
        );
    }

    #[test]
    fn length_counts_invalid_bytes() {
        assert_eq!(length(b"A\x00\xff\x80B"), 5);
        assert_eq!(length(b"\xe2\x82A"), 3);
        assert_eq!(length("\u{e9}".as_bytes()), 1);
    }

    #[test]
    fn ensure_utf8_drops_invalid_sequences() {
        assert_eq!(ensure_utf8(b"A\x00\xff\x80B"), "A\0B");
    }
}
