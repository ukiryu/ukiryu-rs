//! Ruby `Regexp` matching with Ruby's own engine.
//!
//! Patterns compile and match with the Onigmo sources of Ruby 4.0.1, built
//! unchanged from `vendor/onigmo` in Ruby's configuration (see `build.rs`,
//! `vendor/onigmo/PROVENANCE` and `NOTICE`). Case folding, properties,
//! back-references, named groups, look-around and every other construct are
//! therefore Onigmo's, with the Unicode 17.0.0 tables Ruby 4.0.1 ships.
//!
//! This module keeps the two steps of `re.c` that run before Onigmo:
//!
//! * `rb_reg_preprocess` (`unescape_nonascii`), ported below: `\u` escapes and
//!   escaped multibyte bytes become literal UTF-8, `(?#...)` and `/x`
//!   comments are dropped, and a non-ASCII or `\p` pattern gets a fixed
//!   encoding;
//! * the encoding choice of `rb_reg_initialize` / `rb_reg_prepare_enc`: a
//!   regexp without a fixed encoding is US-ASCII and matches ASCII-only text
//!   as compiled; any other text recompiles it as UTF-8 from the source,
//!   preprocessed again with no options, exactly like `rb_reg_prepare_re`.
//!
//! The source is taken as a UTF-8 Ruby string. Inputs Ruby would reject when
//! creating the `Regexp` give `None`.
use std::ffi::{c_int, c_longlong, c_uint, c_void};
use std::sync::{Mutex, OnceLock};

/// `Regexp::IGNORECASE`, `EXTENDED`, `MULTILINE` (= Onigmo's option bits).
const IGNORECASE: u32 = 1;
const EXTENDED: u32 = 2;
const MULTILINE: u32 = 4;
/// `ARG_REG_OPTION_MASK`: the bits Ruby hands to Onigmo.
const REG_OPTION_MASK: u32 = IGNORECASE | EXTENDED | MULTILINE;
/// `ARG_ENCODING_FIXED` and `ARG_ENCODING_NONE` (`Regexp::FIXEDENCODING`,
/// `Regexp::NOENCODING`).
const ENCODING_FIXED: u32 = 16;
const ENCODING_NONE: u32 = 32;

/// `ONIG_MISMATCH` and `ONIG_MAX_ERROR_MESSAGE_LEN` (onigmo.h).
const ONIG_MISMATCH: c_longlong = -1;
const ONIG_MAX_ERROR_MESSAGE_LEN: usize = 90;

extern "C" {
    fn ukiryu_onigmo_new(
        out: *mut *mut c_void,
        pattern: *const u8,
        len: usize,
        options: c_uint,
        utf8: c_int,
        message: *mut u8,
    ) -> c_int;
    fn ukiryu_onigmo_regions(reg: *mut c_void) -> c_int;
    fn ukiryu_onigmo_search(
        reg: *mut c_void,
        string: *const u8,
        len: usize,
        spans: *mut c_longlong,
        capacity: c_int,
    ) -> c_longlong;
    fn ukiryu_onigmo_free(reg: *mut c_void);
}

/// Onigmo keeps a little global state while it compiles (its one-time
/// initialisation and the glue's `rb_raise` flag): compile one at a time.
static COMPILE: Mutex<()> = Mutex::new(());

/// One compiled Onigmo `regex_t`.
#[derive(Debug)]
struct Compiled {
    reg: *mut c_void,
    regions: usize,
    /// A search does not change the `regex_t`, but nothing in Onigmo
    /// promises that; searches on one regexp take turns.
    lock: Mutex<()>,
}

// SAFETY: the pointer is owned (freed once, in Drop) and every use is
// serialised by `lock` or happens under `COMPILE` before it is shared.
unsafe impl Send for Compiled {}
unsafe impl Sync for Compiled {}

impl Drop for Compiled {
    fn drop(&mut self) {
        // SAFETY: `reg` came from ukiryu_onigmo_new and is freed only here.
        unsafe { ukiryu_onigmo_free(self.reg) }
    }
}

impl Compiled {
    /// `onig_new` with Ruby's syntax and default case-fold flag.
    fn new(pattern: &[u8], options: u32, utf8: bool) -> Result<Compiled, String> {
        let _guard = COMPILE.lock().unwrap_or_else(|e| e.into_inner());
        let mut reg: *mut c_void = std::ptr::null_mut();
        let mut message = [0u8; ONIG_MAX_ERROR_MESSAGE_LEN];
        // SAFETY: pointer/length pairs describe live buffers; `message` has
        // the size onig_error_code_to_str writes at most.
        let r = unsafe {
            ukiryu_onigmo_new(
                &mut reg,
                pattern.as_ptr(),
                pattern.len(),
                options & REG_OPTION_MASK,
                c_int::from(utf8),
                message.as_mut_ptr(),
            )
        };
        if r != 0 || reg.is_null() {
            let end = message
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(message.len());
            let text = String::from_utf8_lossy(&message[..end]).into_owned();
            return Err(if text.is_empty() {
                format!("Onigmo error {r}")
            } else {
                text
            });
        }
        // SAFETY: `reg` is a valid compiled regexp.
        let regions = unsafe { ukiryu_onigmo_regions(reg) };
        Ok(Compiled {
            reg,
            regions: usize::try_from(regions).unwrap_or(1).max(1),
            lock: Mutex::new(()),
        })
    }

    /// `onig_search` over the whole of `haystack`; `None` on an engine error.
    fn search(&self, haystack: &str) -> Option<Option<Captures>> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut spans: Vec<c_longlong> = vec![-1; self.regions * 2];
        let capacity = c_int::try_from(self.regions).ok()?;
        // SAFETY: `spans` holds 2 * regions slots, as `capacity` says.
        let r = unsafe {
            ukiryu_onigmo_search(
                self.reg,
                haystack.as_ptr(),
                haystack.len(),
                spans.as_mut_ptr(),
                capacity,
            )
        };
        if r == ONIG_MISMATCH {
            return Some(None);
        }
        if r < 0 {
            return None;
        }
        let groups = spans
            .chunks(2)
            .map(|p| match (usize::try_from(p[0]), usize::try_from(p[1])) {
                (Ok(s), Ok(e)) => Some((s, e)),
                _ => None,
            })
            .collect();
        Some(Some(Captures { groups }))
    }
}

/// A compiled Ruby regular expression.
#[derive(Debug)]
pub struct OnigRegex {
    source: String,
    options: u32,
    /// The `Regexp` has a fixed (UTF-8) encoding; otherwise it is US-ASCII.
    fixed: bool,
    /// What `Regexp.new(source, options)` compiled.
    created: Compiled,
    /// `rb_reg_prepare_re` for non-ASCII text when `created` is US-ASCII.
    utf8: OnceLock<Option<Compiled>>,
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

impl OnigRegex {
    /// Compile Ruby `source` with `Regexp#options` bits; `None` when Ruby
    /// would not create this `Regexp`.
    pub fn new(source: &str, options: u32) -> Option<OnigRegex> {
        Self::compile(source, options).ok()
    }

    /// `Regexp.new(source, options)` for a UTF-8 source string; the error is
    /// the preprocessing or Onigmo message.
    pub fn compile(source: &str, options: u32) -> Result<OnigRegex, String> {
        if options & !(REG_OPTION_MASK | ENCODING_FIXED | ENCODING_NONE) != 0 {
            return Err(format!("unknown Regexp option bits {options:#x}"));
        }
        let pre = preprocess(source.as_bytes(), options & REG_OPTION_MASK)?;
        if pre.fixed && options & ENCODING_NONE != 0 {
            // rb_reg_initialize: a /n regexp may not need a non-binary encoding.
            return Err("incompatible character encoding".into());
        }
        let fixed = pre.fixed || options & ENCODING_FIXED != 0;
        let created = Compiled::new(&pre.bytes, options, fixed)?;
        Ok(OnigRegex {
            source: source.to_owned(),
            options,
            fixed,
            created,
            utf8: OnceLock::new(),
        })
    }

    /// The compiled regexp Ruby uses for `haystack`.
    fn prepared(&self, haystack: &str) -> Option<&Compiled> {
        if self.fixed || haystack.is_ascii() {
            return Some(&self.created);
        }
        self.utf8
            .get_or_init(|| {
                let pre = preprocess(self.source.as_bytes(), 0).ok()?;
                Compiled::new(&pre.bytes, self.options, true).ok()
            })
            .as_ref()
    }

    /// `haystack =~ self` as a boolean; `None` when Onigmo reports an error.
    pub fn is_match(&self, haystack: &str) -> Option<bool> {
        Some(self.captures(haystack)?.is_some())
    }

    /// `self.match(haystack)`; `Some(None)` is no match.
    pub fn captures(&self, haystack: &str) -> Option<Option<Captures>> {
        self.prepared(haystack)?.search(haystack)
    }
}

/// `string =~ regexp` for a Ruby `Regexp` value.
pub fn regexp_match(re: &super::value::Regexp, string: &str) -> Option<bool> {
    OnigRegex::new(&re.source, re.options)?.is_match(string)
}

/// The output of `rb_reg_preprocess`.
struct Preprocessed {
    bytes: Vec<u8>,
    /// `fixed_enc` was set: the pattern needs its (UTF-8) encoding.
    fixed: bool,
}

/// `rb_reg_preprocess(p, end, utf8, &fixed_enc, err, options)` (re.c).
fn preprocess(source: &[u8], options: u32) -> Result<Preprocessed, String> {
    let mut pre = Preprocessor {
        s: source,
        buf: Vec::with_capacity(source.len()),
        fixed: false,
        has_property: false,
    };
    let mut p = 0;
    pre.scan(&mut p, options, false)?;
    let fixed = pre.fixed || pre.has_property;
    Ok(Preprocessed {
        bytes: pre.buf,
        fixed,
    })
}

/// `rb_enc_precise_mbclen` for UTF-8.
enum MbLen {
    Found,
    NeedMore,
    Invalid,
}

fn precise_mbclen(bytes: &[u8]) -> MbLen {
    let Some(&lead) = bytes.first() else {
        return MbLen::NeedMore;
    };
    let width = match lead {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return MbLen::Invalid,
    };
    let have = &bytes[..bytes.len().min(width)];
    match std::str::from_utf8(have) {
        Ok(_) if have.len() == width => MbLen::Found,
        Ok(_) => MbLen::NeedMore,
        Err(e) if e.error_len().is_none() => MbLen::NeedMore,
        Err(_) => MbLen::Invalid,
    }
}

/// `ruby_scan_digits(start, len, base)` for bases 8 and 16: the value and
/// the number of digits read.
fn scan_digits(s: &[u8], len: usize, base: u32) -> (u64, usize) {
    let mut value: u64 = 0;
    let mut n = 0;
    while n < len && n < s.len() {
        match char::from(s[n]).to_digit(base) {
            Some(d) => {
                value = value
                    .wrapping_mul(u64::from(base))
                    .wrapping_add(u64::from(d));
                n += 1;
            }
            None => break,
        }
    }
    (value, n)
}

/// `rb_isspace`.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

struct Preprocessor<'a> {
    s: &'a [u8],
    buf: Vec<u8>,
    /// `*encp != 0`.
    fixed: bool,
    has_property: bool,
}

impl Preprocessor<'_> {
    /// Width of the character at `p` (the source is valid UTF-8).
    fn width(&self, p: usize) -> usize {
        match self.s[p] {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        }
    }

    fn escape_hex(&mut self, byte: u8) {
        self.buf
            .extend_from_slice(format!("\\x{byte:02X}").as_bytes());
    }

    /// `read_escaped_byte`: `p` is at a backslash.
    fn read_escaped_byte(&self, p: &mut usize) -> Result<u8, String> {
        let s = self.s;
        let end = s.len();
        let mut q = *p;
        if q == end || s[q] != b'\\' {
            return Err("too short escaped multibyte character".into());
        }
        q += 1;
        let mut meta = false;
        let mut ctrl = false;
        let code: u64 = 'again: loop {
            if q == end {
                return Err("too short escape sequence".into());
            }
            let c = s[q];
            q += 1;
            match c {
                b'\\' => break 'again u64::from(b'\\'),
                b'n' => break 'again 0x0A,
                b't' => break 'again 0x09,
                b'r' => break 'again 0x0D,
                b'f' => break 'again 0x0C,
                b'v' => break 'again 0x0B,
                b'a' => break 'again 0x07,
                b'e' => break 'again 0x1B,
                b'0'..=b'7' => {
                    q -= 1;
                    let (v, len) = scan_digits(&s[q..], (end - q).min(3), 8);
                    q += len;
                    break 'again v;
                }
                b'x' => {
                    let (v, len) = scan_digits(&s[q..], (end - q).min(2), 16);
                    if len < 1 {
                        return Err("invalid hex escape".into());
                    }
                    q += len;
                    break 'again v;
                }
                b'M' => {
                    if meta {
                        return Err("duplicate meta escape".into());
                    }
                    meta = true;
                    if q + 1 < end {
                        let dash = s[q];
                        q += 1;
                        if dash == b'-' && s[q] & 0x80 == 0 {
                            if s[q] == b'\\' {
                                q += 1;
                                continue 'again;
                            }
                            q += 1;
                            break 'again u64::from(s[q - 1]);
                        }
                    }
                    return Err("too short meta escape".into());
                }
                b'C' => {
                    if q == end || s[q] != b'-' {
                        return Err("too short control escape".into());
                    }
                    q += 1;
                }
                b'c' => {}
                _ => return Err("unexpected escape sequence".into()),
            }
            // Only \C- and \c get here.
            if ctrl {
                return Err("duplicate control escape".into());
            }
            ctrl = true;
            if q < end && s[q] & 0x80 == 0 {
                if s[q] == b'\\' {
                    q += 1;
                    continue 'again;
                }
                q += 1;
                break 'again u64::from(s[q - 1]);
            }
            return Err("too short control escape".into());
        };
        if code > 0xFF {
            return Err("invalid escape code".into());
        }
        let mut code = code as u8;
        if ctrl {
            code &= 0x1F;
        }
        if meta {
            code |= 0x80;
        }
        *p = q;
        Ok(code)
    }

    /// `unescape_escaped_nonascii`: `p` is at a backslash.
    fn escaped_nonascii(&mut self, p: &mut usize) -> Result<(), String> {
        let mut q = *p;
        let mut area = vec![self.read_escaped_byte(&mut q)?];
        // rb_enc_mbmaxlen(UTF-8) is 4.
        while area.len() < 4 && matches!(precise_mbclen(&area), MbLen::NeedMore) {
            area.push(self.read_escaped_byte(&mut q)?);
        }
        if matches!(precise_mbclen(&area), MbLen::Invalid) {
            return Err("invalid multibyte escape".into());
        }
        if area.len() > 1 || area[0] & 0x80 != 0 {
            self.buf.extend_from_slice(&area);
            self.fixed = true;
        } else {
            self.escape_hex(area[0]);
        }
        *p = q;
        Ok(())
    }

    /// `append_utf8`.
    fn append_utf8(&mut self, code: u64) -> Result<(), String> {
        if (0xD800..=0xDFFF).contains(&code) || code > 0x10FFFF {
            return Err("invalid Unicode range".into());
        }
        if code < 0x80 {
            self.escape_hex(code as u8);
        } else {
            let c = char::from_u32(code as u32).ok_or("invalid Unicode range")?;
            let mut b = [0u8; 4];
            self.buf.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            self.fixed = true;
        }
        Ok(())
    }

    /// `unescape_unicode_list`: after `\u{`.
    fn unicode_list(&mut self, p: &mut usize) -> Result<(), String> {
        let s = self.s;
        let end = s.len();
        let mut q = *p;
        let mut any = false;
        while q < end && is_space(s[q]) {
            q += 1;
        }
        loop {
            let (code, len) = scan_digits(&s[q..], end - q, 16);
            if len == 0 {
                break;
            }
            if len > 6 {
                return Err("invalid Unicode range".into());
            }
            q += len;
            self.append_utf8(code)?;
            any = true;
            while q < end && is_space(s[q]) {
                q += 1;
            }
        }
        if !any {
            return Err("invalid Unicode list".into());
        }
        *p = q;
        Ok(())
    }

    /// `unescape_unicode_bmp`: after `\u`.
    fn unicode_bmp(&mut self, p: &mut usize) -> Result<(), String> {
        let s = self.s;
        if s.len() < *p + 4 {
            return Err("invalid Unicode escape".into());
        }
        let (code, len) = scan_digits(&s[*p..], 4, 16);
        if len != 4 {
            return Err("invalid Unicode escape".into());
        }
        self.append_utf8(code)?;
        *p += 4;
        Ok(())
    }

    /// `unescape_nonascii0`.
    fn scan(&mut self, pp: &mut usize, options: u32, recurse: bool) -> Result<(), String> {
        let s = self.s;
        let end = s.len();
        let mut p = *pp;
        let mut in_char_class = 0u32;
        let mut parens = 1u32;
        let mut extended_mode = options & EXTENDED != 0;

        while p < end {
            let chlen = self.width(p);
            if chlen > 1 || s[p] & 0x80 != 0 {
                self.buf.extend_from_slice(&s[p..p + chlen]);
                p += chlen;
                self.fixed = true;
                continue;
            }
            let c = s[p];
            p += 1;
            match c {
                b'\\' => {
                    if p == end {
                        return Err("too short escape sequence".into());
                    }
                    let chlen = self.width(p);
                    if chlen > 1 {
                        // The backslash and the multibyte character, as is.
                        self.buf.extend_from_slice(&s[p - 1..p + chlen]);
                        p += chlen;
                        self.fixed = true;
                        continue;
                    }
                    let c = s[p];
                    p += 1;
                    match c {
                        b'1'..=b'7' => {
                            let len = end - (p - 1);
                            let (v, _) = scan_digits(&s[p - 1..], len.min(3), 8);
                            if v <= 0o177 {
                                // A back-reference or 7-bit octal: kept as is.
                                self.buf.extend_from_slice(&[b'\\', c]);
                            } else {
                                p -= 2;
                                self.escaped_nonascii(&mut p)?;
                            }
                        }
                        b'0' | b'x' | b'c' | b'C' | b'M' => {
                            p -= 2;
                            self.escaped_nonascii(&mut p)?;
                        }
                        b'u' => {
                            if p == end {
                                return Err("too short escape sequence".into());
                            }
                            if s[p] == b'{' {
                                p += 1;
                                self.unicode_list(&mut p)?;
                                if p == end || s[p] != b'}' {
                                    return Err("invalid Unicode list".into());
                                }
                                p += 1;
                            } else {
                                self.unicode_bmp(&mut p)?;
                            }
                        }
                        b'p' | b'P' => {
                            if !self.fixed {
                                self.has_property = true;
                            }
                            self.buf.extend_from_slice(&[b'\\', c]);
                        }
                        _ => self.buf.extend_from_slice(&[b'\\', c]),
                    }
                }
                b'#' if extended_mode && in_char_class == 0 => {
                    // A comment in an extended regexp is dropped.
                    while p < end {
                        let c = s[p];
                        p += 1;
                        if c == b'\n' {
                            break;
                        }
                        if c & 0x80 != 0 {
                            self.fixed = true;
                        }
                    }
                }
                b'[' => {
                    in_char_class += 1;
                    self.buf.push(c);
                }
                b']' => {
                    in_char_class = in_char_class.saturating_sub(1);
                    self.buf.push(c);
                }
                b')' => {
                    self.buf.push(c);
                    if in_char_class == 0 && recurse {
                        parens -= 1;
                        if parens == 0 {
                            *pp = p;
                            return Ok(());
                        }
                    }
                }
                b'(' if in_char_class == 0 && p + 1 < end && s[p] == b'?' => {
                    if s[p + 1] == b'#' {
                        // (?#...) is dropped wherever it appears.
                        let orig = p;
                        let mut cont = true;
                        while cont && p < end {
                            let c = s[p];
                            p += 1;
                            match c {
                                b')' => cont = false,
                                b'\\' => {
                                    if p >= end {
                                        return Err("invalid multibyte character".into());
                                    }
                                    p += self.width(p);
                                }
                                c if c & 0x80 == 0 => {}
                                _ => {
                                    self.fixed = true;
                                    p -= 1;
                                    p += self.width(p);
                                }
                            }
                        }
                        if cont {
                            // Unterminated: left for Onigmo to reject.
                            p = orig;
                            self.buf.push(b'(');
                        }
                        continue;
                    }
                    if recurse {
                        parens += 1;
                    }
                    let mut invert = false;
                    let mut local_extend = 0i8;
                    let mut recursed = false;
                    let mut i = p + 1;
                    while i < end {
                        match s[i] {
                            b'x' => local_extend = if invert { -1 } else { 1 },
                            b'-' => invert = true,
                            b':' | b')' => {
                                if local_extend == 0
                                    || (local_extend == -1 && !extended_mode)
                                    || (local_extend == 1 && extended_mode)
                                {
                                    break;
                                }
                                if s[i] == b':' {
                                    // The extended flag changes until the ')'.
                                    let local = if local_extend == 1 {
                                        options | EXTENDED
                                    } else {
                                        options & !EXTENDED
                                    };
                                    self.buf.push(b'(');
                                    self.scan(&mut p, local, true)?;
                                    recursed = true;
                                } else {
                                    // The flag changes for the rest of the pattern.
                                    extended_mode = local_extend == 1;
                                }
                                break;
                            }
                            b'i' | b'm' | b'a' | b'd' | b'u' => {}
                            _ => break,
                        }
                        i += 1;
                    }
                    if recursed {
                        continue;
                    }
                    self.buf.push(b'(');
                }
                b'(' => {
                    if in_char_class == 0 && recurse {
                        parens += 1;
                    }
                    self.buf.push(c);
                }
                _ => self.buf.push(c),
            }
        }
        if recurse {
            *pp = p;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(src: &str, opts: u32, s: &str) -> Option<bool> {
        OnigRegex::new(src, opts)?.is_match(s)
    }

    #[test]
    fn onigmo_semantics() {
        assert_eq!(m(r"\A[a-z]+\d+\z", 0, "abc123"), Some(true));
        assert_eq!(m(r"\A\d\z", 0, "\u{661}"), Some(false));
        assert_eq!(m(r"\n^", 0, "a\n"), Some(false));
        assert_eq!(m(r"a b", EXTENDED, "ab"), Some(true));
        assert_eq!(m(r"\w", IGNORECASE, "\u{212A}"), Some(false));
        assert_eq!(m(r"[a-z]", IGNORECASE, "\u{212A}"), Some(true));
        assert_eq!(m(r"ss", IGNORECASE, "\u{df}"), Some(true));
        assert_eq!(m(r"(\w)\1", 0, "aa"), Some(true));
        assert_eq!(m(r"(?<y>\d)-\k<y>", 0, "1-2"), Some(false));
        assert_eq!(m(r"\p{Greek}", 0, "\u{3b1}"), Some(true));
        assert_eq!(m(r"\u{e9}", 0, "caf\u{e9}"), Some(true));
        assert_eq!(m(r"(", 0, ""), None);
    }

    #[test]
    fn preprocess_matches_re_c() {
        let p = |s: &str, o| preprocess(s.as_bytes(), o).map(|p| (p.bytes, p.fixed));
        assert_eq!(p(r"a\u0041", 0), Ok((br"a\x41".to_vec(), false)));
        assert_eq!(p(r"\u{e9 41}", 0), Ok(("\u{e9}\\x41".into(), true)));
        assert_eq!(p(r"\xC3\xA9", 0), Ok(("\u{e9}".into(), true)));
        assert_eq!(p(r"\x41(?#c)\1", 0), Ok((br"\x41\1".to_vec(), false)));
        assert_eq!(p("a # \u{e9}\nb", EXTENDED), Ok((b"a b".to_vec(), true)));
        assert_eq!(p(r"\p{Alpha}", 0), Ok((br"\p{Alpha}".to_vec(), true)));
        assert!(p(r"\uD800", 0).is_err());
        assert!(p(r"\xFF", 0).is_err());
    }

    #[test]
    fn captures_are_byte_spans() {
        let re = OnigRegex::new(r"(\d+)(st|nd|rd|th)?", IGNORECASE).unwrap();
        let text = "1\u{fb06} Jan";
        let c = re.captures(text).unwrap().unwrap();
        assert_eq!(c.span(0), Some((0, 4)));
        assert_eq!(c.text(1, text), Some("1"));
        assert_eq!(c.text(2, text), Some("\u{fb06}"));
    }
}
