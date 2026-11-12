//! Ruby string operations the detectors apply to raw process output.
//!
//! Process output arrives as bytes in Ruby's default external encoding
//! (UTF-8 in the captured oracle) without validation. The detectors then call
//! `String#scrub`, `String#scrub('')` or `String#strip`, whose handling of
//! invalid sequences is reproduced here.

/// `String#scrub`: each maximal invalid subsequence becomes U+FFFD.
///
/// Ruby and `String::from_utf8_lossy` split invalid input the same way for
/// every captured case (truncated, surrogate, overlong, out-of-range leads).
pub fn scrub(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `String#scrub('')`: each maximal invalid subsequence is removed.
pub fn scrub_remove(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                out.push_str(valid);
                return out;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                out.push_str(std::str::from_utf8(valid).expect("prefix is valid"));
                match error.error_len() {
                    Some(len) => rest = &after[len..],
                    None => return out,
                }
            }
        }
    }
}

/// Characters `String#strip` removes from both ends.
fn is_strip_char(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ')
}

/// `str.strip.empty?`. `None` when Ruby raises: `strip` rejects a string
/// with an invalid byte sequence (`ArgumentError`).
pub fn strip_is_empty(bytes: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.chars().all(is_strip_char))
}

/// `output[-n..] || output`: the last `n` characters, or all of a shorter string.
pub fn tail_chars(text: &str, n: usize) -> &str {
    if n == 0 {
        return &text[text.len()..];
    }
    match text.char_indices().rev().nth(n - 1) {
        Some((index, _)) => &text[index..],
        None => text,
    }
}
