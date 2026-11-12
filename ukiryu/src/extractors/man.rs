//! Man-page date extraction, ported from `Ukiryu::ManPageParser`
//! (`lib/ukiryu/man_page_parser.rb` at ukiryu `ab6df353`).
//!
//! The source reads a man page, finds the first mdoc `.Dd` line and turns its
//! date into ISO 8601 (`YYYY-MM-DD`) with Ruby's `Date.parse`, falling back to
//! three regular expressions. Version detection uses the result for tools
//! without a `--version` flag.
//!
//! File access is injected through [`ManPageSource`]; [`FileSystem`] is the
//! `std::fs` implementation. `Date.parse` fills missing fields from the
//! current day, so the parser takes that day explicitly as [`Today`].
//!
//! `ManPageParser.resolve_man_page_path` is not ported: at the pinned source
//! it calls `Ukiryu::Platform.current`, which does not exist, so every call
//! raises `NoMethodError` and no caller uses it.

mod date;
mod onig;

pub use date::{date_parse, date_parse_fragments, CivilDate, DateFailure, Fragments, Today};

use num_bigint::BigInt;
use onig::Regex;
use std::fmt;
use std::io;
use std::sync::OnceLock;

/// Where man-page bytes come from (`File.exist?` and `File.read`).
pub trait ManPageSource {
    fn exists(&self, path: &str) -> bool;
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
}

/// [`ManPageSource`] over the real file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileSystem;

impl ManPageSource for FileSystem {
    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
}

/// An exception the source lets escape from `parse_date`.
#[derive(Debug)]
pub enum ManPageError {
    /// `ArgumentError: invalid byte sequence in UTF-8`, raised while scanning
    /// lines up to and including the `.Dd` line.
    InvalidByteSequence,
    /// A read failure other than "not found" or "permission denied"
    /// (`Errno::EISDIR` for a directory, for example).
    Io(io::Error),
    /// `Date::Error: invalid date` from `Date.new` in the regex fallback.
    InvalidDate,
    /// `RangeError` from an out-of-range integer.
    Range(String),
}

impl ManPageError {
    /// The Ruby exception class the source raises.
    pub fn ruby_class(&self) -> &'static str {
        match self {
            ManPageError::InvalidByteSequence => "ArgumentError",
            ManPageError::Io(e) if e.kind() == io::ErrorKind::IsADirectory => "Errno::EISDIR",
            ManPageError::Io(_) => "SystemCallError",
            ManPageError::InvalidDate => "Date::Error",
            ManPageError::Range(_) => "RangeError",
        }
    }
}

impl fmt::Display for ManPageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManPageError::InvalidByteSequence => f.write_str("invalid byte sequence in UTF-8"),
            ManPageError::Io(e) => write!(f, "{e}"),
            ManPageError::InvalidDate => f.write_str("invalid date"),
            ManPageError::Range(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ManPageError {}

impl From<DateFailure> for ManPageError {
    fn from(f: DateFailure) -> Self {
        match f {
            DateFailure::Range(m) => ManPageError::Range(m),
            DateFailure::Invalid | DateFailure::Argument(_) => ManPageError::InvalidDate,
        }
    }
}

/// `Ukiryu::ManPageParser` with its file access and clock injected.
#[derive(Clone, Debug)]
pub struct ManPageParser<S> {
    source: S,
    today: Today,
}

impl<S: ManPageSource> ManPageParser<S> {
    pub fn new(source: S, today: Today) -> Self {
        ManPageParser { source, today }
    }

    /// `ManPageParser.parse_date(path)`: the `.Dd` date as `YYYY-MM-DD`, or
    /// `None` when the path is absent, missing, unreadable (not found or
    /// permission denied) or has no parsable `.Dd` line.
    pub fn parse_date(&self, path: Option<&str>) -> Result<Option<String>, ManPageError> {
        let Some(path) = path else {
            return Ok(None);
        };
        if !self.source.exists(path) {
            return Ok(None);
        }
        let content = match self.source.read(path) {
            Ok(bytes) => bytes,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(None)
            }
            Err(e) => return Err(ManPageError::Io(e)),
        };
        parse_content(&content, &self.today)
    }

    /// `ManPageParser.parse_from_fallback(paths)`: the first path that yields
    /// a date. Errors from any path propagate, as in the source.
    pub fn parse_from_fallback<'a, I>(&self, paths: I) -> Result<Option<String>, ManPageError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        for path in paths {
            if let Some(date) = self.parse_date(Some(path))? {
                return Ok(Some(date));
            }
        }
        Ok(None)
    }
}

fn is_ruby_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// The part of `parse_date` after `File.read`: find the first line matching
/// `/^\.Dd\s+/` and parse it.
pub fn parse_content(content: &[u8], today: &Today) -> Result<Option<String>, ManPageError> {
    for line in content.split_inclusive(|b| *b == b'\n') {
        let Ok(text) = std::str::from_utf8(line) else {
            return Err(ManPageError::InvalidByteSequence);
        };
        let b = text.as_bytes();
        if b.starts_with(b".Dd") && b.get(3).copied().is_some_and(is_ruby_space) {
            return extract_date_from_dd_line(text, today);
        }
    }
    Ok(None)
}

/// `extract_date_from_dd_line`: strip the macro, then `Date.parse` with the
/// regex fallback, formatted as `YYYY-MM-DD`.
pub fn extract_date_from_dd_line(
    dd_line: &str,
    today: &Today,
) -> Result<Option<String>, ManPageError> {
    let rest = dd_line
        .strip_prefix(".Dd")
        .map(|r| r.trim_start_matches(|c: char| c.is_ascii() && is_ruby_space(c as u8)))
        .unwrap_or(dd_line);
    let date_str = ruby_strip(rest);
    let parsed = match date::date_parse(date_str, today) {
        Ok(d) => Some(d),
        Err(f) if f.is_argument_error() => extract_date_with_regex(date_str)?,
        Err(f) => return Err(f.into()),
    };
    Ok(parsed.map(|d| d.iso8601()))
}

/// `String#strip`: ASCII whitespace and NUL on both ends.
fn ruby_strip(s: &str) -> &str {
    let ws = |c: char| c == '\0' || (c.is_ascii() && is_ruby_space(c as u8));
    s.trim_matches(ws)
}

/// `String#to_i` for the `\w+`/`\d+` captures: leading digits, allowing
/// single underscores between digits; anything else gives 0.
fn ruby_to_i(s: &str) -> BigInt {
    let b = s.as_bytes();
    let mut digits = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            digits.push(b[i] as char);
            i += 1;
        } else if b[i] == b'_' && !digits.is_empty() && b.get(i + 1).is_some_and(u8::is_ascii_digit)
        {
            i += 1;
        } else {
            break;
        }
    }
    if digits.is_empty() {
        BigInt::from(0)
    } else {
        digits.parse().expect("ascii digits")
    }
}

fn month_name_to_number(name: &str) -> Option<i32> {
    let n = match name.to_lowercase().as_str() {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    };
    Some(n)
}

fn fallback_patterns() -> &'static [Regex; 3] {
    static P: OnceLock<[Regex; 3]> = OnceLock::new();
    P.get_or_init(|| {
        [
            Regex::new(r"(\w+)\s+(\d+),?\s+(\d{4})", false),
            Regex::new(r"(\d+)\s+(\w+)\s+(\d{4})", false),
            Regex::new(r"(\d{4})-(\d{2})-(\d{2})", false),
        ]
    })
}

/// `extract_date_with_regex`. Its `Date.new` errors are not rescued.
fn extract_date_with_regex(date_str: &str) -> Result<Option<CivilDate>, ManPageError> {
    let chars: Vec<char> = date_str.chars().collect();
    for re in fallback_patterns() {
        let Some(m) = re.find(&chars) else {
            continue;
        };
        let c: Vec<String> = (1..=3)
            .map(|g| m.text(&chars, g).expect("all groups participate"))
            .collect();
        let len = |s: &String| s.chars().count();
        if len(&c[0]) == 4 {
            let date = date::date_new(&ruby_to_i(&c[0]), &ruby_to_i(&c[1]), &ruby_to_i(&c[2]))?;
            return Ok(Some(date));
        } else if len(&c[2]) == 4 {
            let (day, month_name) = if c[0].bytes().all(|b| b.is_ascii_digit()) {
                (&c[0], &c[1])
            } else {
                (&c[1], &c[0])
            };
            if let Some(month) = month_name_to_number(month_name) {
                let date =
                    date::date_new(&ruby_to_i(&c[2]), &BigInt::from(month), &ruby_to_i(day))?;
                return Ok(Some(date));
            }
        }
    }
    Ok(None)
}
