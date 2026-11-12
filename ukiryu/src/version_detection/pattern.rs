//! Ruby (Onigmo) version patterns on top of the `regex` crate.
//!
//! The pinned source never matches with a Rust-like engine: profile patterns
//! are strings that `String#match` turns into `Regexp.new(pattern)`, and the
//! detectors read `MatchData#[1]`. Ruby and `regex` disagree on several
//! defaults (`\d`/`\w`/`\s` are ASCII-only in Ruby, `^`/`$` are always line
//! anchors, `(?m)` means dot-all, plain groups stop capturing once a named
//! group exists, `{,n}` and a malformed `{` are legal). This module translates
//! the subset whose behaviour was captured from the pinned Ruby and refuses
//! everything else:
//!
//! * [`PatternError::RubyInvalid`]: Ruby itself raises `RegexpError` for the
//!   pattern. Callers reproduce whatever the source does with that error.
//! * [`PatternError::Unsupported`]: Ruby accepts the pattern but this port
//!   cannot guarantee the same matches. This is an explicit gap, never a
//!   silent no-match.
//!
//! A pattern is compiled once and reused for every haystack it is applied to
//! (the source recompiles on each `String#match`).
use std::fmt;
use std::sync::OnceLock;

/// Why a Ruby pattern string cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternError {
    /// The pinned Ruby raises `RegexpError` for this pattern.
    RubyInvalid { pattern: String, reason: String },
    /// Valid in Ruby, but outside the subset this port reproduces exactly.
    Unsupported { pattern: String, construct: String },
}

impl PatternError {
    pub fn pattern(&self) -> &str {
        match self {
            PatternError::RubyInvalid { pattern, .. }
            | PatternError::Unsupported { pattern, .. } => pattern,
        }
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatternError::RubyInvalid { pattern, reason } => {
                write!(f, "Ruby rejects version pattern /{pattern}/: {reason}")
            }
            PatternError::Unsupported { pattern, construct } => {
                write!(f, "version pattern /{pattern}/ uses {construct}, which this port does not reproduce")
            }
        }
    }
}

impl std::error::Error for PatternError {}

/// The result of applying a pattern: `MatchData#[0]` and `MatchData#[1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternMatch<'h> {
    pub whole: &'h str,
    /// `match[1]`: `None` when the pattern has no first group or the group
    /// did not participate.
    pub group1: Option<&'h str>,
}

/// A compiled Ruby pattern.
#[derive(Clone, Debug)]
pub struct RubyPattern {
    source: String,
    regex: regex::Regex,
    /// The translation contains a Ruby `^`; see [`RubyPattern::captures`].
    has_line_start: bool,
}

/// Source of the `/(\d+\.\d+)/` literal used as the source's default pattern.
pub const DEFAULT_PATTERN_SOURCE: &str = r"(\d+\.\d+)";

impl RubyPattern {
    /// `Regexp.new(source)`, restricted to the reproduced subset.
    pub fn new(source: &str) -> Result<Self, PatternError> {
        let first = Translator::new(source, false).run()?;
        let translated = if first.has_named {
            Translator::new(source, true).run()?
        } else {
            first
        };
        let regex = regex::Regex::new(&translated.out).map_err(|e| PatternError::Unsupported {
            pattern: source.to_string(),
            construct: format!(
                "a form the regex crate rejects ({})",
                e.to_string().lines().last().unwrap_or("")
            ),
        })?;
        Ok(RubyPattern {
            source: source.to_string(),
            regex,
            has_line_start: translated.has_line_start,
        })
    }

    /// The source's `/(\d+\.\d+)/` literal, compiled once per process.
    pub fn default_literal() -> &'static RubyPattern {
        static DEFAULT: OnceLock<RubyPattern> = OnceLock::new();
        DEFAULT.get_or_init(|| {
            RubyPattern::new(DEFAULT_PATTERN_SOURCE).expect("default pattern is supported")
        })
    }

    /// The Ruby pattern text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// `haystack.match(pattern)`.
    ///
    /// Ruby's `^` never matches at the very end of a string that ends in
    /// `"\n"`, while the `regex` crate's multi-line `^` does. When that one
    /// position could decide the result, this returns
    /// [`PatternError::Unsupported`] instead of guessing.
    pub fn captures<'h>(
        &self,
        haystack: &'h str,
    ) -> Result<Option<PatternMatch<'h>>, PatternError> {
        let Some(caps) = self.regex.captures(haystack) else {
            return Ok(None);
        };
        let whole = caps.get(0).expect("group 0 always participates");
        if self.has_line_start && haystack.ends_with('\n') && whole.end() == haystack.len() {
            return Err(PatternError::Unsupported {
                pattern: self.source.clone(),
                construct: "`^` that may match after a trailing newline (Ruby never matches there)"
                    .to_string(),
            });
        }
        Ok(Some(PatternMatch {
            whole: whole.as_str(),
            group1: caps.get(1).map(|m| m.as_str()),
        }))
    }

    /// `haystack.match?(Regexp.new(pattern))`.
    pub fn is_match(&self, haystack: &str) -> Result<bool, PatternError> {
        Ok(self.captures(haystack)?.is_some())
    }
}

const WORD: &str = "a-zA-Z0-9_";
const DIGIT: &str = "0-9";
const SPACE: &str = r"\t\n\x0B\x0C\r ";
const HEX: &str = "0-9a-fA-F";

struct Translated {
    out: String,
    has_named: bool,
    has_line_start: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prev {
    /// Start of pattern, after `(` or `|`: a quantifier has no target.
    Nothing,
    /// An anchor or other zero-width assertion.
    Assertion,
    /// Something a quantifier may follow.
    Atom,
    /// `*`, `+`, `?`, `{n,}`, `{,m}`, `{n,m}`: a lazy `?` may follow.
    Greedy,
    /// `{n}`: Ruby reads a following `?` as "optional", not lazy.
    Exact,
    /// A lazy quantifier.
    Lazy,
}

struct Translator<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
    out: String,
    named_mode: bool,
    has_named: bool,
    has_line_start: bool,
    depth: usize,
    names: Vec<String>,
    prev: Prev,
}

impl<'a> Translator<'a> {
    fn new(source: &'a str, named_mode: bool) -> Self {
        Translator {
            source,
            chars: source.chars().collect(),
            pos: 0,
            out: String::with_capacity(source.len() * 2),
            named_mode,
            has_named: false,
            has_line_start: false,
            depth: 0,
            names: Vec::new(),
            prev: Prev::Nothing,
        }
    }

    fn invalid<T>(&self, reason: &str) -> Result<T, PatternError> {
        Err(PatternError::RubyInvalid {
            pattern: self.source.to_string(),
            reason: reason.to_string(),
        })
    }

    fn unsupported<T>(&self, construct: &str) -> Result<T, PatternError> {
        Err(PatternError::Unsupported {
            pattern: self.source.to_string(),
            construct: construct.to_string(),
        })
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn run(mut self) -> Result<Translated, PatternError> {
        while let Some(c) = self.peek(0) {
            self.pos += 1;
            match c {
                '\\' => self.escape()?,
                '.' => self.atom("."),
                '^' => {
                    self.has_line_start = true;
                    self.assertion("(?m:^)");
                }
                '$' => self.assertion("(?m:$)"),
                '[' => {
                    let class = self.class()?;
                    self.atom(&class);
                }
                '(' => self.group()?,
                ')' => {
                    if self.depth == 0 {
                        return self.invalid("unmatched close parenthesis");
                    }
                    self.depth -= 1;
                    self.atom(")");
                }
                '|' => {
                    self.out.push('|');
                    self.prev = Prev::Nothing;
                }
                '*' | '+' | '?' => self.quantifier(c)?,
                '{' => self.brace()?,
                _ => self.literal(c),
            }
        }
        if self.depth > 0 {
            return self.invalid("end pattern with unmatched parenthesis");
        }
        Ok(Translated {
            out: self.out,
            has_named: self.has_named,
            has_line_start: self.has_line_start,
        })
    }

    fn atom(&mut self, text: &str) {
        self.out.push_str(text);
        self.prev = Prev::Atom;
    }

    fn assertion(&mut self, text: &str) {
        self.out.push_str(text);
        self.prev = Prev::Assertion;
    }

    fn literal(&mut self, c: char) {
        let escaped = regex::escape(c.encode_utf8(&mut [0; 4]));
        self.atom(&escaped);
    }

    /// A Ruby shorthand set as an ASCII class that case folding cannot widen.
    fn set(&mut self, members: &str, negated: bool) {
        let class = format!("(?-i:[{}{}])", if negated { "^" } else { "" }, members);
        self.atom(&class);
    }

    fn escape(&mut self) -> Result<(), PatternError> {
        let Some(c) = self.peek(0) else {
            return self.invalid("too short escape sequence");
        };
        self.pos += 1;
        match c {
            'd' => self.set(DIGIT, false),
            'D' => self.set(DIGIT, true),
            'w' => self.set(WORD, false),
            'W' => self.set(WORD, true),
            's' => self.set(SPACE, false),
            'S' => self.set(SPACE, true),
            'h' => self.set(HEX, false),
            'H' => self.set(HEX, true),
            // Ruby's \b and \B use Unicode word characters (captured: `a\b`
            // does not match "aé"), like the regex crate's default.
            'b' => self.assertion(r"\b"),
            'B' => self.assertion(r"\B"),
            'A' => self.assertion(r"\A"),
            'z' => self.assertion(r"\z"),
            _ => {
                let ch = self.escaped_char(c, false)?;
                self.literal(ch);
            }
        }
        Ok(())
    }

    /// An escape that denotes one character (shared by classes).
    fn escaped_char(&mut self, c: char, in_class: bool) -> Result<char, PatternError> {
        Ok(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'f' => '\x0C',
            'v' => '\x0B',
            'a' => '\x07',
            'e' => '\x1B',
            '0' => {
                if self.peek(0).is_some_and(|d| d.is_ascii_digit()) {
                    return self.unsupported("an octal escape");
                }
                '\0'
            }
            'x' => {
                let hi = self.peek(0).and_then(|d| d.to_digit(16));
                let lo = self.peek(1).and_then(|d| d.to_digit(16));
                match (hi, lo) {
                    (Some(hi), Some(lo)) if hi < 8 => {
                        self.pos += 2;
                        char::from_u32(hi * 16 + lo).expect("ASCII")
                    }
                    _ => {
                        return self
                            .unsupported("a \\x escape other than \\x00-\\x7F with two digits")
                    }
                }
            }
            'u' => {
                let digits: Option<Vec<u32>> = (0..4)
                    .map(|i| self.peek(i).and_then(|d| d.to_digit(16)))
                    .collect();
                let Some(digits) = digits else {
                    return self.unsupported("a \\u escape other than \\uHHHH");
                };
                let code = digits.iter().fold(0, |acc, d| acc * 16 + d);
                let Some(ch) = char::from_u32(code) else {
                    return self.unsupported("a \\u escape naming a surrogate");
                };
                self.pos += 4;
                ch
            }
            'b' if in_class => return self.unsupported("\\b inside a character class (backspace)"),
            '1'..='9' => return self.unsupported("a back-reference or octal escape"),
            c if c.is_ascii_alphanumeric() => {
                return self.unsupported(&format!("the escape \\{c}"));
            }
            // Escaped punctuation, space and non-ASCII stand for themselves.
            c => c,
        })
    }

    fn group(&mut self) -> Result<(), PatternError> {
        if self.peek(0) != Some('?') {
            self.depth += 1;
            self.out.push_str(if self.named_mode { "(?:" } else { "(" });
            self.prev = Prev::Nothing;
            return Ok(());
        }
        self.pos += 1;
        match self.peek(0) {
            Some(':') => {
                self.pos += 1;
                self.depth += 1;
                self.out.push_str("(?:");
                self.prev = Prev::Nothing;
                Ok(())
            }
            Some('<') if matches!(self.peek(1), Some('=') | Some('!')) => {
                self.unsupported("a look-behind group")
            }
            Some('<') => self.named_group('>'),
            Some('\'') => self.named_group('\''),
            Some('=') | Some('!') => self.unsupported("a look-ahead group"),
            Some('>') => self.unsupported("an atomic group"),
            Some('~') => self.unsupported("an absent operator"),
            Some('#') => {
                while let Some(c) = self.peek(0) {
                    self.pos += 1;
                    if c == ')' {
                        // A comment is not a quantifier target.
                        self.prev = Prev::Assertion;
                        return Ok(());
                    }
                }
                self.unsupported("an unterminated comment group")
            }
            _ => self.option_group(),
        }
    }

    fn named_group(&mut self, close: char) -> Result<(), PatternError> {
        self.pos += 1;
        let mut name = String::new();
        loop {
            match self.peek(0) {
                None => return self.invalid("invalid group name"),
                Some(c) if c == close => {
                    self.pos += 1;
                    break;
                }
                Some(c) => {
                    name.push(c);
                    self.pos += 1;
                }
            }
        }
        let mut chars = name.chars();
        let valid_start = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if !name.is_ascii() {
            return self.unsupported("a non-ASCII group name");
        }
        if !valid_start || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return self.invalid(&format!("invalid group name <{name}>"));
        }
        if self.names.contains(&name) {
            return self.unsupported("a repeated group name");
        }
        self.names.push(name.clone());
        self.has_named = true;
        self.depth += 1;
        self.out.push_str(&format!("(?P<{name}>"));
        self.prev = Prev::Nothing;
        Ok(())
    }

    fn option_group(&mut self) -> Result<(), PatternError> {
        let mut flags = String::new();
        loop {
            let Some(c) = self.peek(0) else {
                return self.invalid("end pattern in group");
            };
            self.pos += 1;
            match c {
                // Ruby `m` is dot-all, the regex crate's `s`.
                'm' => flags.push('s'),
                '-' => flags.push('-'),
                'i' => {
                    return self.unsupported("case-insensitive matching (Ruby folds multi-character cases such as \"ss\"/\"ß\")")
                }
                'x' => return self.unsupported("extended (?x) syntax"),
                'a' | 'd' | 'u' => return self.unsupported("a character-set option"),
                'P' => return self.invalid("undefined group option"),
                ':' | ')' => {
                    if flags.is_empty() {
                        return self.invalid("undefined group option");
                    }
                    if c == ':' {
                        self.depth += 1;
                        self.out.push_str(&format!("(?{flags}:"));
                        self.prev = Prev::Nothing;
                    } else {
                        // Flags without a group apply to the rest of the enclosing group.
                        self.out.push_str(&format!("(?{flags})"));
                        self.prev = Prev::Assertion;
                    }
                    if flags == "-" || flags.ends_with('-') {
                        return self.unsupported("an empty option switch");
                    }
                    return Ok(());
                }
                _ => return self.unsupported("an unrecognised group option"),
            }
        }
    }

    fn quantifier(&mut self, c: char) -> Result<(), PatternError> {
        match (self.prev, c) {
            (Prev::Nothing, _) => self.invalid("target of repeat operator is not specified"),
            (Prev::Assertion, _) => self.unsupported("a quantifier on an anchor"),
            (Prev::Atom, _) => {
                self.out.push(c);
                self.prev = Prev::Greedy;
                Ok(())
            }
            (Prev::Greedy, '?') => {
                self.out.push('?');
                self.prev = Prev::Lazy;
                Ok(())
            }
            (Prev::Greedy, '+') => self.unsupported("a possessive quantifier"),
            (Prev::Exact, '?') => {
                self.unsupported("`?` after an exact `{n}` (Ruby reads it as optional)")
            }
            _ => self.unsupported("a repeated quantifier"),
        }
    }

    /// `{`: an interval when it parses as one, otherwise a literal brace.
    fn brace(&mut self) -> Result<(), PatternError> {
        let start = self.pos;
        let mut lower = String::new();
        while let Some(d) = self.peek(0).filter(|d| d.is_ascii_digit()) {
            lower.push(d);
            self.pos += 1;
        }
        let mut upper: Option<String> = None;
        if self.peek(0) == Some(',') {
            self.pos += 1;
            let mut u = String::new();
            while let Some(d) = self.peek(0).filter(|d| d.is_ascii_digit()) {
                u.push(d);
                self.pos += 1;
            }
            upper = Some(u);
        }
        let closed = self.peek(0) == Some('}');
        let is_interval =
            closed && !(lower.is_empty() && upper.as_deref().is_none_or(str::is_empty));
        if !is_interval {
            // Ruby reads a brace that is not a complete interval (`x{a}`,
            // `x{`, `x{,}`) as a literal.
            self.pos = start;
            self.literal('{');
            return Ok(());
        }
        self.pos += 1;
        let parse = |s: &str| s.parse::<u32>().ok();
        let (min, max) = match &upper {
            None => {
                let n = parse(&lower);
                (n, n)
            }
            Some(u) => (
                if lower.is_empty() {
                    Some(0)
                } else {
                    parse(&lower)
                },
                if u.is_empty() { None } else { parse(u) },
            ),
        };
        let Some(min) = min else {
            return self.unsupported("an interval bound that does not fit");
        };
        if upper.as_deref().is_some_and(|u| !u.is_empty()) && max.is_none() {
            return self.unsupported("an interval bound that does not fit");
        }
        if let Some(max) = max {
            if max < min {
                return self.invalid("upper is smaller than lower in repeat range");
            }
        }
        match self.prev {
            Prev::Nothing => return self.invalid("target of repeat operator is not specified"),
            Prev::Assertion => return self.unsupported("a quantifier on an anchor"),
            Prev::Atom => {}
            _ => return self.unsupported("a repeated quantifier"),
        }
        let exact = upper.is_none();
        let text = match (upper.is_none(), max) {
            (true, _) => format!("{{{min}}}"),
            (false, Some(max)) => format!("{{{min},{max}}}"),
            (false, None) => format!("{{{min},}}"),
        };
        self.out.push_str(&text);
        self.prev = if exact { Prev::Exact } else { Prev::Greedy };
        Ok(())
    }

    fn class(&mut self) -> Result<String, PatternError> {
        let mut out = String::from("[");
        if self.peek(0) == Some('^') {
            self.pos += 1;
            out.push('^');
        }
        if self.peek(0) == Some(']') {
            if self.peek(1).is_none() {
                return self.invalid("empty char-class");
            }
            // A leading `]` is a literal member in Ruby.
            self.pos += 1;
            out.push_str(r"\]");
            self.class_tail_range(&mut out, ']')?;
        }
        loop {
            let Some(c) = self.peek(0) else {
                return self.invalid("premature end of char-class");
            };
            self.pos += 1;
            match c {
                ']' => {
                    out.push(']');
                    return Ok(out);
                }
                '[' => return self.unsupported("a nested set or POSIX bracket"),
                '&' if self.peek(0) == Some('&') => {
                    return self.unsupported("class intersection (&&)")
                }
                '\\' => {
                    let Some(e) = self.peek(0) else {
                        return self.invalid("premature end of char-class");
                    };
                    self.pos += 1;
                    let set = match e {
                        'd' => Some(DIGIT.to_string()),
                        'D' => Some(format!("[^{DIGIT}]")),
                        'w' => Some(WORD.to_string()),
                        'W' => Some(format!("[^{WORD}]")),
                        's' => Some(SPACE.to_string()),
                        'S' => Some(format!("[^{SPACE}]")),
                        'h' => Some(HEX.to_string()),
                        'H' => Some(format!("[^{HEX}]")),
                        _ => None,
                    };
                    if let Some(set) = set {
                        if self.peek(0) == Some('-') && self.peek(1).is_some_and(|n| n != ']') {
                            return self.invalid("unmatched range specifier in char-class");
                        }
                        out.push_str(&set);
                        continue;
                    }
                    let ch = self.escaped_char(e, true)?;
                    push_class_char(&mut out, ch);
                    self.class_tail_range(&mut out, ch)?;
                }
                c => {
                    push_class_char(&mut out, c);
                    self.class_tail_range(&mut out, c)?;
                }
            }
        }
    }

    /// After a single-character member: an optional `-hi` range.
    fn class_tail_range(&mut self, out: &mut String, lo: char) -> Result<(), PatternError> {
        if self.peek(0) != Some('-') {
            return Ok(());
        }
        match self.peek(1) {
            None | Some(']') => Ok(()), // trailing `-` is literal; handled by the loop
            Some('[') => self.unsupported("a range ending in a nested set"),
            Some(hi) => {
                self.pos += 2;
                let hi = if hi == '\\' {
                    let Some(e) = self.peek(0) else {
                        return self.invalid("premature end of char-class");
                    };
                    self.pos += 1;
                    if matches!(e, 'd' | 'D' | 'w' | 'W' | 's' | 'S' | 'h' | 'H') {
                        return self.unsupported("a range ending in a shorthand set");
                    }
                    self.escaped_char(e, true)?
                } else {
                    hi
                };
                if hi < lo {
                    return self.invalid("empty range in char class");
                }
                out.push('-');
                push_class_char(out, hi);
                Ok(())
            }
        }
    }
}

fn push_class_char(out: &mut String, c: char) {
    if c.is_ascii_punctuation() || c == ' ' {
        out.push('\\');
        out.push(c);
    } else if c.is_ascii_control() {
        out.push_str(&format!("\\x{:02X}", c as u32));
    } else {
        out.push(c);
    }
}
