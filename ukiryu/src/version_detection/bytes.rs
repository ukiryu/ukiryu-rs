//! Loader version extraction preserves the Ruby output string's encoding.
use super::text;
use crate::types::onig::{Encoding, OnigRegex};

/// Borrowed Ruby string bytes with an explicit ASCII-compatible encoding.
#[derive(Debug, Clone, Copy)]
pub struct EncodedText<'a> {
    pub bytes: &'a [u8],
    pub encoding: Encoding,
}

/// A captured version; even an ASCII capture from BINARY remains BINARY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedVersion {
    pub bytes: Vec<u8>,
    pub encoding: Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionStage {
    Compile,
    Match,
}

/// Source exception and the operation at which Ruby raised it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionError {
    pub class: &'static str,
    pub message: String,
    pub stage: ExtractionStage,
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ExtractionError {}

fn regexp_description(source: &str) -> String {
    let mut out = String::from("/");
    let mut chars = source.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                out.push(ch);
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '/' => out.push_str("\\/"),
            _ => out.push(ch),
        }
    }
    out.push('/');
    out
}

/// `Tool::Loader.extract_version_from_pattern`: first `output.scrub('')`,
/// then `Regexp.new(pattern)`, then `match[1]`. A nil argument short-circuits
/// before any of those operations. Pattern text is the existing UTF-8 API.
pub fn extract_version_from_encoded(
    output: Option<EncodedText<'_>>,
    pattern: Option<&str>,
) -> Result<Option<ExtractedVersion>, ExtractionError> {
    let (Some(output), Some(pattern)) = (output, pattern) else {
        return Ok(None);
    };
    // String#scrub('') removes invalid sequences only for encodings that can
    // contain them. Every byte is valid in ASCII-8BIT; never UTF-8-decode it.
    let scrubbed;
    let bytes = match output.encoding {
        Encoding::Binary => output.bytes,
        Encoding::Utf8 => {
            scrubbed = text::scrub_remove(output.bytes).into_bytes();
            &scrubbed
        }
        Encoding::UsAscii => {
            scrubbed = output.bytes.iter().copied().filter(u8::is_ascii).collect();
            &scrubbed
        }
    };
    let regex = OnigRegex::compile(pattern, 0).map_err(|message| ExtractionError {
        class: "RegexpError",
        message: format!("{message}: {}", regexp_description(pattern)),
        stage: ExtractionStage::Compile,
    })?;
    let matched = regex
        .captures_bytes(bytes, output.encoding)
        .map_err(|error| ExtractionError {
            class: error.class,
            message: error.message,
            stage: ExtractionStage::Match,
        })?;
    Ok(matched
        .and_then(|found| found.span(1))
        .map(|(start, end)| ExtractedVersion {
            bytes: bytes[start..end].to_vec(),
            encoding: output.encoding,
        }))
}
