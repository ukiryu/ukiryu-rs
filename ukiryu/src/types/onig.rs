//! Ruby (Onigmo) regular expressions on a portable engine.
//!
//! Ruby compiles `Regexp` literals with Onigmo. This module translates a Ruby
//! pattern into the syntax of [`fancy_regex`] (pure Rust, so it also runs on
//! WebAssembly) and matches with Onigmo's semantics for every construct it
//! accepts. Whatever it cannot reproduce exactly is refused, and the caller
//! reports that as `Unsupported` instead of guessing.
//!
//! Translated so the result is Onigmo's, measured against Ruby 4.0.1
//! (Unicode 17.0.0) over every code point:
//!
//! * `\d \s \w \h` (and negations) are ASCII only: `[0-9]`,
//!   `[\t\n\v\f\r ]`, `[0-9A-Za-z_]`, `[0-9A-Fa-f]`, and do not case-fold
//!   under `/i`;
//! * `\b` / `\B` use Onigmo's Unicode word set:
//!   `Alphabetic | M | Nd | Pc | Join_Control` plus `U+00B2 U+00B3 U+00B9
//!   U+00BC..U+00BE`;
//! * `[[:alpha:]]` is `\p{Alphabetic}`, `[[:alnum:]]` is
//!   `[\p{Alphabetic}\p{Nd}]`;
//! * `^` / `$` are always line anchors (`^` not after a final newline), `/m`
//!   is dot-all, `/x` whitespace and comments are dropped outside classes.
//!
//! Refused: back-references, `\k \g \p \G \K \R \X` and other Onigmo-only
//! escapes, POSIX brackets other than `alpha`/`alnum`, absent operators,
//! conditionals, bytes `\x80..\xFF`, non-ASCII pattern text under `/i`, and
//! input that holds a character whose full case folding has several
//! characters while `/i` is active (Onigmo matches `ss` against `ß`; the
//! Rust engine uses simple folding) or a code point the Rust tables
//! (Unicode 16.0.0) leave unassigned.
use std::sync::OnceLock;

use fancy_regex::{Regex as Engine, RegexBuilder};

/// `Regexp::IGNORECASE`, `EXTENDED`, `MULTILINE`.
const IGNORECASE: u32 = 1;
const EXTENDED: u32 = 2;
const MULTILINE: u32 = 4;
/// `FIXEDENCODING` and `NOENCODING` do not change matching of UTF-8 text.
const ENCODING_BITS: u32 = 16 | 32;

/// Onigmo's word characters for `\b` (Ruby 4.0.1, see the module docs).
const WORD: &str =
    r"[\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\p{Join_Control}\x{B2}\x{B3}\x{B9}\x{BC}-\x{BE}]";

/// Code points whose full case folding has more than one character
/// (`String#downcase(:fold)`, Ruby 4.0.1, Unicode 17.0.0).
const MULTI_CHAR_FOLDS: [u32; 104] = [
    0x00DF, 0x0130, 0x0149, 0x01F0, 0x0390, 0x03B0, 0x0587, 0x1E96, 0x1E97, 0x1E98, 0x1E99, 0x1E9A,
    0x1E9E, 0x1F50, 0x1F52, 0x1F54, 0x1F56, 0x1F80, 0x1F81, 0x1F82, 0x1F83, 0x1F84, 0x1F85, 0x1F86,
    0x1F87, 0x1F88, 0x1F89, 0x1F8A, 0x1F8B, 0x1F8C, 0x1F8D, 0x1F8E, 0x1F8F, 0x1F90, 0x1F91, 0x1F92,
    0x1F93, 0x1F94, 0x1F95, 0x1F96, 0x1F97, 0x1F98, 0x1F99, 0x1F9A, 0x1F9B, 0x1F9C, 0x1F9D, 0x1F9E,
    0x1F9F, 0x1FA0, 0x1FA1, 0x1FA2, 0x1FA3, 0x1FA4, 0x1FA5, 0x1FA6, 0x1FA7, 0x1FA8, 0x1FA9, 0x1FAA,
    0x1FAB, 0x1FAC, 0x1FAD, 0x1FAE, 0x1FAF, 0x1FB2, 0x1FB3, 0x1FB4, 0x1FB6, 0x1FB7, 0x1FBC, 0x1FC2,
    0x1FC3, 0x1FC4, 0x1FC6, 0x1FC7, 0x1FCC, 0x1FD2, 0x1FD3, 0x1FD6, 0x1FD7, 0x1FE2, 0x1FE3, 0x1FE4,
    0x1FE6, 0x1FE7, 0x1FF2, 0x1FF3, 0x1FF4, 0x1FF6, 0x1FF7, 0x1FFC, 0xFB00, 0xFB01, 0xFB02, 0xFB03,
    0xFB04, 0xFB05, 0xFB06, 0xFB13, 0xFB14, 0xFB15, 0xFB16, 0xFB17,
];

/// A compiled Ruby regular expression.
#[derive(Debug)]
pub struct OnigRegex {
    engine: Engine,
    ignore_case: bool,
}

/// A successful match: the byte span of each group (`None` when the group
/// did not take part).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captures {
    pub groups: Vec<Option<(usize, usize)>>,
}

impl Captures {
    pub fn span(&self, i: usize) -> Option<(usize, usize)> {
        self.groups.get(i).copied().flatten()
    }

    pub fn text<'t>(&self, i: usize, haystack: &'t str) -> Option<&'t str> {
        self.span(i).map(|(s, e)| &haystack[s..e])
    }
}

fn unassigned() -> &'static Engine {
    static RE: OnceLock<Engine> = OnceLock::new();
    RE.get_or_init(|| Engine::new(r"\p{Cn}").expect("static pattern"))
}

impl OnigRegex {
    /// Compile Ruby `source` with `Regexp#options` bits; `None` when the
    /// pattern uses a construct this module does not reproduce.
    pub fn new(source: &str, options: u32) -> Option<OnigRegex> {
        if options & !(IGNORECASE | EXTENDED | MULTILINE | ENCODING_BITS) != 0 {
            return None;
        }
        let mut tr = Translator {
            chars: source.chars().collect(),
            pos: 0,
            out: String::new(),
            extended: options & EXTENDED != 0,
            ignore_case: options & IGNORECASE != 0,
            scopes: Vec::new(),
        };
        tr.sequence()?;
        if tr.ignore_case && !source.is_ascii() {
            return None;
        }
        let mut pattern = String::from("(?m)");
        if options & IGNORECASE != 0 {
            pattern.push_str("(?i)");
        }
        if options & MULTILINE != 0 {
            pattern.push_str("(?s)");
        }
        pattern.push_str(&tr.out);
        let engine = RegexBuilder::new(&pattern)
            .oniguruma_mode(true)
            .build()
            .ok()?;
        Some(OnigRegex {
            engine,
            ignore_case: tr.ignore_case,
        })
    }

    fn accepts(&self, haystack: &str) -> bool {
        if self.ignore_case
            && haystack
                .chars()
                .any(|c| MULTI_CHAR_FOLDS.binary_search(&(c as u32)).is_ok())
        {
            return false;
        }
        haystack.is_ascii() || !matches!(unassigned().is_match(haystack), Ok(true))
    }

    /// `haystack =~ self` as a boolean; `None` when the answer may differ
    /// from Onigmo's or the engine gave up.
    pub fn is_match(&self, haystack: &str) -> Option<bool> {
        if !self.accepts(haystack) {
            return None;
        }
        self.engine.is_match(haystack).ok()
    }

    /// `self.match(haystack)`; `Some(None)` is no match.
    pub fn captures(&self, haystack: &str) -> Option<Option<Captures>> {
        if !self.accepts(haystack) {
            return None;
        }
        let caps = self.engine.captures(haystack).ok()?;
        Some(caps.map(|c| {
            Captures {
                groups: (0..c.len())
                    .map(|i| c.get(i).map(|m| (m.start(), m.end())))
                    .collect(),
            }
        }))
    }
}

/// `string =~ regexp` for a Ruby `Regexp` value.
pub fn regexp_match(re: &super::value::Regexp, string: &str) -> Option<bool> {
    OnigRegex::new(&re.source, re.options)?.is_match(string)
}

struct Translator {
    chars: Vec<char>,
    pos: usize,
    out: String,
    extended: bool,
    /// Whether `/i` is active anywhere in the pattern.
    ignore_case: bool,
    /// The `/x` state to restore when each open group closes.
    scopes: Vec<bool>,
}

fn class_escape(c: char) -> Option<&'static str> {
    Some(match c {
        'd' => "0-9",
        'w' => "0-9A-Za-z_",
        's' => r"\t\n\x0B\x0C\r ",
        'h' => "0-9A-Fa-f",
        _ => return None,
    })
}

/// Characters that need a backslash to be literal in the target syntax.
fn is_meta(c: char) -> bool {
    r"\.+*?()|[]{}^$#&-~".contains(c)
}

fn push_literal(out: &mut String, c: char) {
    if is_meta(c) {
        out.push('\\');
    }
    out.push(c);
}

impl Translator {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn sequence(&mut self) -> Option<()> {
        while let Some(c) = self.bump() {
            if self.extended && matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C') {
                continue;
            }
            if self.extended && c == '#' {
                while let Some(c) = self.bump() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            match c {
                '\\' => self.escape()?,
                '[' => self.class()?,
                '(' => self.group()?,
                ')' => {
                    self.extended = self.scopes.pop()?;
                    self.out.push(')');
                }
                _ => self.out.push(c),
            }
        }
        if self.scopes.is_empty() {
            Some(())
        } else {
            None
        }
    }

    /// Hex digits after `\x` or `\u`; a code point that is not a byte above
    /// ASCII (`\x80..\xFF` are raw bytes in Ruby).
    fn hex(&mut self, min: usize, max: usize) -> Option<u32> {
        let mut v = 0u32;
        let mut n = 0;
        while n < max {
            match self.peek().and_then(|c| c.to_digit(16)) {
                Some(d) => {
                    v = v * 16 + d;
                    n += 1;
                    self.pos += 1;
                }
                None => break,
            }
        }
        (n >= min).then_some(v)
    }

    /// One escaped character as target syntax, for use inside or outside a
    /// class. `None` refuses the pattern.
    fn escaped_char(&mut self, c: char, out: &mut String) -> Option<()> {
        let code = match c {
            'n' => 0x0A,
            't' => 0x09,
            'r' => 0x0D,
            'f' => 0x0C,
            'v' => 0x0B,
            'e' => 0x1B,
            'a' => 0x07,
            'x' => {
                let v = self.hex(1, 2)?;
                if v >= 0x80 {
                    return None;
                }
                v
            }
            'u' => {
                if self.peek() == Some('{') {
                    self.pos += 1;
                    let v = self.hex(1, 6)?;
                    if self.bump()? != '}' {
                        return None;
                    }
                    v
                } else {
                    self.hex(4, 4)?
                }
            }
            '0' => {
                let mut v = 0u32;
                for _ in 0..2 {
                    match self.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.pos += 1;
                        }
                        None => break,
                    }
                }
                v
            }
            c if c.is_ascii_alphanumeric() => return None,
            c => {
                push_literal(out, c);
                return Some(());
            }
        };
        char::from_u32(code)?;
        out.push_str(&format!("\\x{{{code:X}}}"));
        Some(())
    }

    fn escape(&mut self) -> Option<()> {
        let c = self.bump()?;
        match c {
            'd' | 'w' | 's' | 'h' => {
                let set = class_escape(c)?;
                self.out.push_str(&format!("(?-i:[{set}])"));
            }
            'D' | 'W' | 'S' | 'H' => {
                let set = class_escape(c.to_ascii_lowercase())?;
                self.out.push_str(&format!("(?-i:[^{set}])"));
            }
            'b' => self
                .out
                .push_str(&format!("(?:(?<={WORD})(?!{WORD})|(?<!{WORD})(?={WORD}))")),
            'B' => self
                .out
                .push_str(&format!("(?:(?<={WORD})(?={WORD})|(?<!{WORD})(?!{WORD}))")),
            // Onigmo's `\Z` allows one final newline; the engine's allows several.
            'Z' => self.out.push_str(r"(?=\n?\z)"),
            'A' | 'z' => {
                self.out.push('\\');
                self.out.push(c);
            }
            _ => {
                let mut lit = String::new();
                self.escaped_char(c, &mut lit)?;
                self.out.push_str(&lit);
            }
        }
        Some(())
    }

    fn class(&mut self) -> Option<()> {
        let mut out = String::from("[");
        if self.peek() == Some('^') {
            self.pos += 1;
            out.push('^');
        }
        let mut first = true;
        loop {
            let c = self.bump()?;
            match c {
                ']' if !first => break,
                '[' if self.peek() == Some(':') => {
                    let rest: String = self.chars[self.pos..].iter().collect();
                    let (set, len) = if rest.starts_with(":alpha:]") {
                        (r"\p{Alphabetic}", 8)
                    } else if rest.starts_with(":alnum:]") {
                        (r"\p{Alphabetic}\p{Nd}", 8)
                    } else {
                        return None;
                    };
                    self.pos += len;
                    out.push_str(set);
                }
                '[' => {
                    // A nested class: translate it in place.
                    let saved = std::mem::take(&mut self.out);
                    self.class()?;
                    let nested = std::mem::replace(&mut self.out, saved);
                    out.push_str(&nested);
                }
                '&' if self.peek() == Some('&') => {
                    self.pos += 1;
                    out.push_str("&&");
                }
                '-' if self.peek() == Some('-') => return None,
                '\\' => {
                    let e = self.bump()?;
                    match e {
                        'd' | 'w' | 's' | 'h' => out.push_str(class_escape(e)?),
                        'D' | 'W' | 'S' | 'H' => {
                            out.push_str(&format!("[^{}]", class_escape(e.to_ascii_lowercase())?))
                        }
                        'b' => out.push_str(r"\x{8}"),
                        _ => self.escaped_char(e, &mut out)?,
                    }
                }
                '-' => out.push('-'),
                c => push_literal(&mut out, c),
            }
            first = false;
        }
        if self.ignore_case && (out.contains("0-9A-Za-z_") || out.contains("0-9A-Fa-f")) {
            // Onigmo does not case-fold `\w`/`\h` inside a class under /i.
            return None;
        }
        out.push(']');
        self.out.push_str(&out);
        Some(())
    }

    fn group(&mut self) -> Option<()> {
        self.scopes.push(self.extended);
        if self.peek() != Some('?') {
            self.out.push('(');
            return Some(());
        }
        self.pos += 1;
        match self.bump()? {
            ':' => self.out.push_str("(?:"),
            '=' => self.out.push_str("(?="),
            '!' => self.out.push_str("(?!"),
            '>' => self.out.push_str("(?>"),
            '#' => {
                self.scopes.pop();
                while self.bump()? != ')' {}
            }
            '<' => match self.bump()? {
                '=' => self.out.push_str("(?<="),
                '!' => self.out.push_str("(?<!"),
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let mut name = String::from(c);
                    loop {
                        let c = self.bump()?;
                        if c == '>' {
                            break;
                        }
                        if !(c.is_ascii_alphanumeric() || c == '_') {
                            return None;
                        }
                        name.push(c);
                    }
                    self.out.push_str(&format!("(?P<{name}>"));
                }
                _ => return None,
            },
            c if matches!(c, 'i' | 'm' | 'x' | '-') => {
                self.pos -= 1;
                let mut on = true;
                let mut flags = String::new();
                let mut extended = self.extended;
                loop {
                    let c = self.bump()?;
                    match c {
                        '-' if on => {
                            on = false;
                            flags.push('-');
                        }
                        'i' => {
                            if on {
                                self.ignore_case = true;
                            }
                            flags.push('i');
                        }
                        'm' => flags.push('s'),
                        'x' => extended = on,
                        ':' | ')' => {
                            let flags = flags.trim_end_matches('-');
                            if c == ':' {
                                self.out.push_str(&format!("(?{flags}:"));
                                self.extended = extended;
                            } else {
                                // `(?imx)` applies to the rest of the enclosing group.
                                self.scopes.pop();
                                if !flags.is_empty() {
                                    self.out.push_str(&format!("(?{flags})"));
                                }
                                self.extended = extended;
                            }
                            break;
                        }
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(src: &str, opts: u32, s: &str) -> Option<bool> {
        OnigRegex::new(src, opts)?.is_match(s)
    }

    #[test]
    fn ascii_classes_and_anchors() {
        assert_eq!(m(r"\A[a-z]+\d+\z", 0, "abc123"), Some(true));
        assert_eq!(m(r"\A\d\z", 0, "\u{661}"), Some(false));
        assert_eq!(m(r"\n^", 0, "a\n"), Some(false));
        assert_eq!(m(r"a b", EXTENDED, "ab"), Some(true));
        assert_eq!(m(r"[ ]", EXTENDED, " "), Some(true));
        assert_eq!(m(r"\w", IGNORECASE, "\u{212A}"), Some(false));
        assert_eq!(m(r"[a-z]", IGNORECASE, "\u{212A}"), Some(true));
        assert_eq!(m(r"a\b", 0, "a\u{e9}"), Some(false));
        assert_eq!(m(r"ss", IGNORECASE, "\u{df}"), None);
        assert_eq!(m(r"(\w)\1", 0, "aa"), None);
    }
}
