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
//! current day, so the parser reads that day from an injected
//! [`Clock`] (the runtime / execution context clock:
//! the system clock at an explicit UTC offset natively, a fixed day in tests).
//!
//! [`ManPageParser`] implements
//! [`ManPageDates`], so version
//! detection's `man_page` method gets the same errors the source raises.
//!
//! `ManPageParser.resolve_man_page_path` is not ported: at the pinned source
//! it calls `Ukiryu::Platform.current`, which does not exist, so every call
//! raises `NoMethodError` and no caller uses it.

mod date;

pub use date::{date_parse, date_parse_fragments, CivilDate, DateFailure, Fragments, Today};

use crate::runtime::Clock;
use crate::types::onig::OnigRegex;
use crate::version_detection::ManPageDates;
use num_bigint::BigInt;
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
    /// The shared Onigmo path could not run a pattern. Never a Ruby class:
    /// the port did not decide the input.
    Unsupported(String),
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
            ManPageError::Unsupported(_) => "Unsupported",
        }
    }
}

impl fmt::Display for ManPageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManPageError::InvalidByteSequence => f.write_str("invalid byte sequence in UTF-8"),
            ManPageError::Io(e) => write!(f, "{e}"),
            ManPageError::InvalidDate => f.write_str("invalid date"),
            ManPageError::Range(m) | ManPageError::Unsupported(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ManPageError {}

impl From<DateFailure> for ManPageError {
    fn from(f: DateFailure) -> Self {
        match f {
            DateFailure::Range(m) => ManPageError::Range(m),
            DateFailure::Invalid | DateFailure::Argument(_) => ManPageError::InvalidDate,
            DateFailure::Unsupported(m) => ManPageError::Unsupported(m),
        }
    }
}

/// `Ukiryu::ManPageParser` with its file access and clock injected.
#[derive(Clone, Debug)]
pub struct ManPageParser<S> {
    source: S,
    clock: Clock,
}

impl<S: ManPageSource> ManPageParser<S> {
    /// A parser whose "today" is always `today`.
    pub fn new(source: S, today: Today) -> Self {
        Self::with_clock(source, Clock::Fixed(today))
    }

    /// A parser that reads "today" from `clock` on every parse, as the
    /// source's `Date.parse` does. Pass the runtime or execution context
    /// clock (`Runtime::clock`, `ExecutionContext::clock`).
    pub fn with_clock(source: S, clock: Clock) -> Self {
        ManPageParser { source, clock }
    }

    /// The injected clock.
    pub fn clock(&self) -> Clock {
        self.clock
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
        parse_content(&content, &self.clock.today())
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

/// The man-page boundary of `VersionDetector.detect_with_methods`. Errors
/// propagate unchanged; the source does not rescue them there.
impl<S: ManPageSource> ManPageDates for ManPageParser<S> {
    fn parse_date(&mut self, path: &str) -> Result<Option<String>, ManPageError> {
        ManPageParser::parse_date(self, Some(path))
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

/// The three source regexp literals, compiled by the shared Onigmo path
/// ([`crate::types::onig`]) with no options.
const FALLBACK_PATTERNS: [&str; 3] = [
    r"(\w+)\s+(\d+),?\s+(\d{4})",
    r"(\d+)\s+(\w+)\s+(\d{4})",
    r"(\d{4})-(\d{2})-(\d{2})",
];

fn fallback_patterns() -> &'static [Option<OnigRegex>; 3] {
    static P: OnceLock<[Option<OnigRegex>; 3]> = OnceLock::new();
    P.get_or_init(|| FALLBACK_PATTERNS.map(|src| OnigRegex::new(src, 0)))
}

/// `extract_date_with_regex`. Its `Date.new` errors are not rescued.
fn extract_date_with_regex(date_str: &str) -> Result<Option<CivilDate>, ManPageError> {
    let engine = |what: &str| {
        ManPageError::Unsupported(format!(
            "extract_date_with_regex: {what} failed in the Onigmo regexp engine"
        ))
    };
    for re in fallback_patterns() {
        let re = re.as_ref().ok_or_else(|| engine("a pattern"))?;
        let Some(m) = re.captures(date_str).ok_or_else(|| engine("this text"))? else {
            continue;
        };
        let c: Vec<&str> = (1..=3)
            .map(|g| m.text(g, date_str).expect("all groups participate"))
            .collect();
        let len = |s: &str| s.chars().count();
        if len(c[0]) == 4 {
            let date = date::date_new(&ruby_to_i(c[0]), &ruby_to_i(c[1]), &ruby_to_i(c[2]))?;
            return Ok(Some(date));
        } else if len(c[2]) == 4 {
            let (day, month_name) = if c[0].bytes().all(|b| b.is_ascii_digit()) {
                (c[0], c[1])
            } else {
                (c[1], c[0])
            };
            if let Some(month) = month_name_to_number(month_name) {
                let date = date::date_new(&ruby_to_i(c[2]), &BigInt::from(month), &ruby_to_i(day))?;
                return Ok(Some(date));
            }
        }
    }
    Ok(None)
}
