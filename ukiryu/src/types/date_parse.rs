//! `Date._parse(string, true)`, as `Time.parse` uses it.
//!
//! Port of `date__parse` in `ext/date/date_parse.c` of the date gem 3.5.1
//! (the default gem of Ruby 4.0.1), built without `TIGHT_PARSER`. Every
//! sub-parser keeps its Onigmo pattern text verbatim and runs through
//! [`super::onig`]; a match is replaced by one space before the next
//! sub-parser runs, exactly like the C code. Only the keys `Time.parse` reads
//! are kept (`year yday mon mday hour min sec sec_fraction zone`); the
//! commercial-week keys and `offset` are parsed and dropped.
use std::sync::OnceLock;

use num_bigint::BigInt;

use super::error::{Error, ErrorClass, Result};
use super::onig::{Captures, OnigRegex};
use super::ruby::string_inspect;

/// `sec_fraction`: `numerator / 10**digits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fraction {
    pub numerator: BigInt,
    pub digits: usize,
}

/// The `Date._parse` hash keys `Time.parse` reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub year: Option<BigInt>,
    pub yday: Option<BigInt>,
    pub mon: Option<BigInt>,
    pub mday: Option<BigInt>,
    pub hour: Option<BigInt>,
    pub min: Option<BigInt>,
    pub sec: Option<BigInt>,
    pub sec_fraction: Option<Fraction>,
    pub zone: Option<String>,
    bc: bool,
    comp: bool,
}

const ABBR_DAYS: &str = "sun|mon|tue|wed|thu|fri|sat";
const ABBR_MONTHS: &str = "jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec";

fn declined(what: &str, input: &str) -> Error {
    Error::unsupported(format!(
        "Date._parse({}): {what} failed in the Onigmo regexp engine",
        string_inspect(input)
    ))
}

/// `rb_str_to_inum(s, 10, 0)` for the digit strings the patterns capture
/// (an optional sign, then digits; no digits is 0).
fn str2num(s: &str) -> BigInt {
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let v = if digits.is_empty() {
        BigInt::from(0)
    } else {
        digits.parse().expect("ASCII digits")
    };
    if neg {
        -v
    } else {
        v
    }
}

fn n2i(s: &str, from: usize, width: usize) -> BigInt {
    BigInt::from(
        s.as_bytes()[from..from + width]
            .iter()
            .fold(0i64, |v, b| v * 10 + i64::from(b - b'0')),
    )
}

fn index3(table: &str, s: &str) -> usize {
    let s = s.as_bytes();
    table
        .split('|')
        .position(|name| s.len() >= 3 && s[..3].eq_ignore_ascii_case(name.as_bytes()))
        .unwrap_or(table.split('|').count())
}

/// `mon_num`: 1-based index of the three-letter month prefix.
fn mon_num(s: &str) -> i64 {
    index3(ABBR_MONTHS, s) as i64 + 1
}

struct Pat {
    source: String,
    ignore_case: bool,
    cell: OnceLock<Option<OnigRegex>>,
}

impl Pat {
    fn get(&self) -> Option<&OnigRegex> {
        self.cell
            .get_or_init(|| OnigRegex::new(&self.source, u32::from(self.ignore_case)))
            .as_ref()
    }
}

macro_rules! pattern {
    ($name:ident, $icase:expr, $src:expr) => {
        fn $name() -> &'static Pat {
            static P: OnceLock<Pat> = OnceLock::new();
            P.get_or_init(|| Pat {
                source: $src,
                ignore_case: $icase,
                cell: OnceLock::new(),
            })
        }
    };
}

const NUMBER: &str = r"(?<!\d)\d";
const BEGIN_ERA: &str = r"\b";
const END_ERA: &str = r"(?!(?<!\.)[a-z])";

pattern!(pat_gsub, false, r"[^-+',./:@[:alnum:]\[\]]+".to_string());
pattern!(pat_day, true, format!(r"\b({ABBR_DAYS})[^-/\d\s]*"));
pattern!(
    pat_time,
    true,
    format!(
        concat!(
            r"(",
            r"{n}+\s*",
            r"(?:",
            r"(?:",
            r":\s*\d+",
            r"(?:",
            r"\s*:\s*\d+(?:[,.]\d*)?",
            r")?",
            r"|",
            r"h(?:\s*\d+m?(?:\s*\d+s?)?)?",
            r")",
            r"(?:",
            r"\s*",
            r"[ap](?:m\b|\.m\.)",
            r")?",
            r"|",
            r"[ap](?:m\b|\.m\.)",
            r")",
            r")",
            r"(?:",
            r"\s*",
            r"(",
            r"(?:gmt|utc?)?[-+]\d+(?:[,.:]\d+(?::\d+)?)?",
            r"|",
            r"(?-i:[[:alpha:].\s]+)(?:standard|daylight)\stime\b",
            r"|",
            r"(?-i:[[:alpha:]]+)(?:\sdst)?\b",
            r")",
            r")?"
        ),
        n = NUMBER
    )
);
pattern!(
    pat_time2,
    true,
    concat!(
        r"\A(\d+)h?",
        r"(?:\s*:?\s*(\d+)m?",
        r"(?:",
        r"\s*:?\s*(\d+)(?:[,.](\d+))?s?",
        r")?",
        r")?",
        r"(?:\s*([ap])(?:m\b|\.m\.))?"
    )
    .to_string()
);
pattern!(
    pat_eu,
    true,
    format!(
        concat!(
            r"('?{n}+)[^-\d\s]*",
            r"\s*",
            r"({m})[^-\d\s']*",
            r"(?:",
            r"\s*",
            r"(?:",
            r"{be}",
            r"(c(?:e|\.e\.)|b(?:ce|\.c\.e\.)|a(?:d|\.d\.)|b(?:c|\.c\.))",
            r"{ee}",
            r")?",
            r"\s*",
            r"('?-?\d+(?:(?:st|nd|rd|th)\b)?)",
            r")?"
        ),
        n = NUMBER,
        m = ABBR_MONTHS,
        be = BEGIN_ERA,
        ee = END_ERA
    )
);
pattern!(
    pat_us,
    true,
    format!(
        concat!(
            r"\b({m})[^-\d\s']*",
            r"\s*",
            r"('?\d+)[^-\d\s']*",
            r"(?:",
            r"\s*+,?",
            r"\s*+",
            r"(c(?:e|\.e\.)|b(?:ce|\.c\.e\.)|a(?:d|\.d\.)|b(?:c|\.c\.))?",
            r"\s*",
            r"('?-?\d+)",
            r")?"
        ),
        m = ABBR_MONTHS
    )
);
pattern!(
    pat_iso,
    false,
    format!(r"('?[-+]?{NUMBER}+)-(\d+)-('?-?\d+)")
);
pattern!(
    pat_iso21,
    true,
    r"\b(\d{2}|\d{4})?-?w(\d{2})(?:-?(\d))?\b".to_string()
);
pattern!(pat_iso22, true, r"-w-(\d)\b".to_string());
pattern!(pat_iso23, false, r"--(\d{2})?-(\d{2})\b".to_string());
pattern!(pat_iso24, false, r"--(\d{2})(\d{2})?\b".to_string());
pattern!(pat_iso25_0, false, r"[,.](\d{2}|\d{4})-\d{3}\b".to_string());
pattern!(pat_iso25, false, r"\b(\d{2}|\d{4})-(\d{3})\b".to_string());
pattern!(pat_iso26_0, false, r"\d-\d{3}\b".to_string());
pattern!(pat_iso26, false, r"\b-(\d{3})\b".to_string());
pattern!(pat_jis, true, r"\b([mtshr])(\d+)\.(\d+)\.(\d+)".to_string());
pattern!(
    pat_vms11,
    true,
    format!(r"('?-?{NUMBER}+)-({ABBR_MONTHS})[^-/.]*-('?-?\d+)")
);
pattern!(
    pat_vms12,
    true,
    format!(r"\b({ABBR_MONTHS})[^-/.]*-('?-?\d+)(?:-('?-?\d+))?")
);
pattern!(
    pat_sla,
    true,
    format!(r"('?-?{NUMBER}+)/\s*('?\d+)(?:\D\s*('?-?\d+))?")
);
pattern!(
    pat_dot,
    true,
    format!(r"('?-?{NUMBER}+)\.\s*('?\d+)\.\s*('?-?\d+)")
);
pattern!(pat_year, false, r"'(\d+)\b".to_string());
pattern!(pat_mon, true, format!(r"\b({ABBR_MONTHS})\S*"));
pattern!(pat_mday, true, format!(r"({NUMBER}+)(st|nd|rd|th)\b"));
pattern!(
    pat_ddd,
    true,
    format!(
        concat!(
            r"([-+]?)({n}{{2,14}})",
            r"(?:",
            r"\s*",
            r"t?",
            r"\s*",
            r"(\d{{2,6}})?(?:[,.](\d*))?",
            r")?",
            r"(?:",
            r"\s*",
            r"(",
            r"z\b",
            r"|",
            r"[-+]\d{{1,4}}\b",
            r"|",
            r"\[[-+]?\d[^\]]*\]",
            r")",
            r")?"
        ),
        n = NUMBER
    )
);
pattern!(pat_bc, true, r"\b(bc\b|bce\b|b\.c\.|b\.c\.e\.)".to_string());
pattern!(pat_frag, true, r"\A\s*(\d{1,2})\s*\z".to_string());

const HAVE_ALPHA: u8 = 1;
const HAVE_DIGIT: u8 = 2;
const HAVE_DASH: u8 = 4;
const HAVE_DOT: u8 = 8;
const HAVE_SLASH: u8 = 16;

fn check_class(s: &str) -> u8 {
    s.bytes().fold(0, |f, b| {
        f | if b.is_ascii_alphabetic() {
            HAVE_ALPHA
        } else {
            0
        } | if b.is_ascii_digit() { HAVE_DIGIT } else { 0 }
            | match b {
                b'-' => HAVE_DASH,
                b'.' => HAVE_DOT,
                b'/' => HAVE_SLASH,
                _ => 0,
            }
    })
}

struct State<'a> {
    input: &'a str,
    str: String,
    h: Parsed,
}

type Cb = fn(&mut State<'_>, &Captures, &str) -> Result<bool>;

impl State<'_> {
    fn have(&self, flags: u8) -> bool {
        check_class(&self.str) & flags == flags
    }

    fn matches(&self, pat: &Pat) -> Result<Option<Captures>> {
        let re = pat.get().ok_or_else(|| declined("a pattern", self.input))?;
        re.captures(&self.str)
            .ok_or_else(|| declined("this text", self.input))
    }

    /// `subx`: on a match, replace it with one space and run the callback.
    fn subs(&mut self, pat: &Pat, cb: Cb) -> Result<bool> {
        let Some(m) = self.matches(pat)? else {
            return Ok(false);
        };
        let text = self.str.clone();
        let (b, e) = m.span(0).expect("group 0");
        self.str.replace_range(b..e, " ");
        cb(self, &m, &text)?;
        Ok(true)
    }

    /// `s3e(hash, y, m, d, bc)`.
    fn s3e(&mut self, y: Option<String>, m: Option<String>, d: Option<String>, bc: bool) {
        let (mut y, mut m, mut d) = (y, m, d);
        let mut c: Option<bool> = None;

        if y.is_some() && m.is_some() && d.is_none() {
            let (oy, om, od) = (y, m, d);
            y = od;
            m = oy;
            d = om;
        }

        if y.is_none() {
            if d.as_ref().is_some_and(|d| d.len() > 2) {
                y = d.take();
            }
            if d.as_ref().is_some_and(|d| d.starts_with('\'')) {
                y = d.take();
            }
        }

        if let Some(ys) = y.clone() {
            let b = ys.as_bytes();
            let mut s = 0;
            while s < b.len() && !matches!(b[s], b'-' | b'+') && !b[s].is_ascii_digit() {
                s += 1;
            }
            if s < b.len() {
                let bp = s;
                if matches!(b[s], b'-' | b'+') {
                    s += 1;
                }
                let l = b[s..].iter().take_while(|c| c.is_ascii_digit()).count();
                let ep = s + l;
                if ep < b.len() {
                    y = d.take();
                    d = Some(ys[bp..ep].to_string());
                }
            }
        }

        if let Some(ms) = &m {
            if ms.starts_with('\'') || ms.len() > 2 {
                let (oy, om, od) = (y, m, d);
                y = om;
                m = od;
                d = oy;
            }
        }

        if let Some(ds) = &d {
            if ds.starts_with('\'') || ds.len() > 2 {
                std::mem::swap(&mut y, &mut d);
            }
        }

        if let Some(ys) = &y {
            let b = ys.as_bytes();
            let mut s = 0;
            while s < b.len() && !matches!(b[s], b'-' | b'+') && !b[s].is_ascii_digit() {
                s += 1;
            }
            if s < b.len() {
                let bp = s;
                let sign = matches!(b[s], b'-' | b'+');
                if sign {
                    s += 1;
                    c = Some(false);
                }
                let l = b[s..].iter().take_while(|c| c.is_ascii_digit()).count();
                if l > 2 {
                    c = Some(false);
                }
                self.h.year = Some(str2num(&ys[bp..s + l]));
            }
        }

        if bc {
            self.h.bc = true;
        }

        if let Some(ms) = &m {
            let digits: String = ms
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(char::is_ascii_digit)
                .collect();
            if !digits.is_empty() {
                self.h.mon = Some(str2num(&digits));
            }
        }

        if let Some(ds) = &d {
            let digits: String = ds
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(char::is_ascii_digit)
                .collect();
            if !digits.is_empty() {
                self.h.mday = Some(str2num(&digits));
            }
        }

        if let Some(c) = c {
            self.h.comp = c;
        }
    }
}

fn group(m: &Captures, i: usize, text: &str) -> Option<String> {
    m.text(i, text).map(str::to_string)
}

fn parse_day_cb(_: &mut State<'_>, _: &Captures, _: &str) -> Result<bool> {
    // `wday` is not read by Time.parse.
    Ok(true)
}

fn parse_time2_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let mut h = str2num(m.text(1, text).expect("hour"));
    let min = m.text(2, text).map(str2num);
    let s = m.text(3, text).map(str2num);
    let f = m.text(4, text).map(|f| Fraction {
        numerator: str2num(f),
        digits: f.len(),
    });
    if let Some(p) = m.text(5, text) {
        // NUM2INT(h): RangeError beyond a C int.
        let ih: i32 = match i32::try_from(&h) {
            Ok(i) => i,
            Err(_) => {
                let fixnum = BigInt::from(1i64 << 62);
                let message = if h >= fixnum || h < -fixnum {
                    "bignum too big to convert into 'long'".to_string()
                } else {
                    format!("integer {h} too big to convert to 'int'")
                };
                return Err(Error::new(ErrorClass::Range, message));
            }
        };
        let mut ih = ih % 12;
        if p.starts_with(['P', 'p']) {
            ih += 12;
        }
        h = BigInt::from(ih);
    }
    st.h.hour = Some(h);
    if min.is_some() {
        st.h.min = min;
    }
    if s.is_some() {
        st.h.sec = s;
    }
    if f.is_some() {
        st.h.sec_fraction = f;
    }
    Ok(true)
}

fn parse_time_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let s1 = m.text(1, text).expect("time").to_string();
    if let Some(s2) = m.text(2, text) {
        st.h.zone = Some(s2.to_string());
    }
    match st.matches_in(pat_time2(), &s1)? {
        Some(m2) => parse_time2_cb(st, &m2, &s1),
        None => Ok(false),
    }
}

impl State<'_> {
    fn matches_in(&self, pat: &Pat, text: &str) -> Result<Option<Captures>> {
        let re = pat.get().ok_or_else(|| declined("a pattern", self.input))?;
        re.captures(text)
            .ok_or_else(|| declined("this text", self.input))
    }
}

fn era_bc(m: &Captures, i: usize, text: &str) -> bool {
    m.text(i, text).is_some_and(|b| b.starts_with(['B', 'b']))
}

fn parse_eu_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let d = group(m, 1, text);
    let mon = mon_num(m.text(2, text).expect("month")).to_string();
    let bc = era_bc(m, 3, text);
    let y = group(m, 4, text);
    st.s3e(y, Some(mon), d, bc);
    Ok(true)
}

fn parse_us_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let mon = mon_num(m.text(1, text).expect("month")).to_string();
    let d = group(m, 2, text);
    let bc = era_bc(m, 3, text);
    let y = group(m, 4, text);
    st.s3e(y, Some(mon), d, bc);
    Ok(true)
}

fn parse_iso_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.s3e(
        group(m, 1, text),
        group(m, 2, text),
        group(m, 3, text),
        false,
    );
    Ok(true)
}

fn ignore_cb(_: &mut State<'_>, _: &Captures, _: &str) -> Result<bool> {
    // Commercial-week keys (`cwyear`, `cweek`, `cwday`) are not read by Time.parse.
    Ok(true)
}

fn parse_iso23_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    if let Some(mon) = m.text(1, text) {
        st.h.mon = Some(str2num(mon));
    }
    st.h.mday = Some(str2num(m.text(2, text).expect("mday")));
    Ok(true)
}

fn parse_iso24_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.h.mon = Some(str2num(m.text(1, text).expect("mon")));
    if let Some(d) = m.text(2, text) {
        st.h.mday = Some(str2num(d));
    }
    Ok(true)
}

fn parse_iso25_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.h.year = Some(str2num(m.text(1, text).expect("year")));
    st.h.yday = Some(str2num(m.text(2, text).expect("yday")));
    Ok(true)
}

fn parse_iso26_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.h.yday = Some(str2num(m.text(1, text).expect("yday")));
    Ok(true)
}

fn gengo(c: u8) -> i64 {
    match c.to_ascii_lowercase() {
        b'm' => 1867,
        b't' => 1911,
        b's' => 1925,
        b'h' => 1988,
        b'r' => 2018,
        _ => 0,
    }
}

fn parse_jis_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let ep = gengo(m.text(1, text).expect("era").as_bytes()[0]);
    st.h.year = Some(str2num(m.text(2, text).expect("year")) + ep);
    st.h.mon = Some(str2num(m.text(3, text).expect("mon")));
    st.h.mday = Some(str2num(m.text(4, text).expect("mday")));
    Ok(true)
}

fn parse_vms11_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let d = group(m, 1, text);
    let mon = mon_num(m.text(2, text).expect("month")).to_string();
    let y = group(m, 3, text);
    st.s3e(y, Some(mon), d, false);
    Ok(true)
}

fn parse_vms12_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let mon = mon_num(m.text(1, text).expect("month")).to_string();
    let d = group(m, 2, text);
    let y = group(m, 3, text);
    st.s3e(y, Some(mon), d, false);
    Ok(true)
}

fn parse_year_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.h.year = Some(str2num(m.text(1, text).expect("year")));
    Ok(true)
}

fn parse_mon_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.h.mon = Some(BigInt::from(mon_num(m.text(1, text).expect("month"))));
    Ok(true)
}

fn parse_mday_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    st.h.mday = Some(str2num(m.text(1, text).expect("mday")));
    Ok(true)
}

fn parse_ddd_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let s1 = m.text(1, text);
    let s2 = m.text(2, text).expect("digits");
    let s3 = m.text(3, text);
    let s4 = m.text(4, text);
    let s5 = m.text(5, text);
    let neg = s1.is_some_and(|s| s.starts_with('-'));
    let signed = |v: BigInt| if neg { -v } else { v };
    let l2 = s2.len();
    let time_only = s3.is_none() && s4.is_some();
    let h = &mut st.h;
    match l2 {
        2 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
            } else {
                h.mday = Some(n2i(s2, 0, 2));
            }
        }
        4 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
                h.min = Some(n2i(s2, l2 - 4, 2));
            } else {
                h.mon = Some(n2i(s2, 0, 2));
                h.mday = Some(n2i(s2, 2, 2));
            }
        }
        6 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
                h.min = Some(n2i(s2, l2 - 4, 2));
                h.hour = Some(n2i(s2, l2 - 6, 2));
            } else {
                h.year = Some(signed(n2i(s2, 0, 2)));
                h.mon = Some(n2i(s2, 2, 2));
                h.mday = Some(n2i(s2, 4, 2));
            }
        }
        8 | 10 | 12 | 14 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
                h.min = Some(n2i(s2, l2 - 4, 2));
                h.hour = Some(n2i(s2, l2 - 6, 2));
                h.mday = Some(n2i(s2, l2 - 8, 2));
                if l2 >= 10 {
                    h.mon = Some(n2i(s2, l2 - 10, 2));
                }
                if l2 == 12 {
                    h.year = Some(signed(n2i(s2, l2 - 12, 2)));
                }
                if l2 == 14 {
                    h.year = Some(signed(n2i(s2, l2 - 14, 4)));
                    h.comp = false;
                }
            } else {
                h.year = Some(signed(n2i(s2, 0, 4)));
                h.mon = Some(n2i(s2, 4, 2));
                h.mday = Some(n2i(s2, 6, 2));
                if l2 >= 10 {
                    h.hour = Some(n2i(s2, 8, 2));
                }
                if l2 >= 12 {
                    h.min = Some(n2i(s2, 10, 2));
                }
                if l2 >= 14 {
                    h.sec = Some(n2i(s2, 12, 2));
                }
                h.comp = false;
            }
        }
        3 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
                h.min = Some(n2i(s2, l2 - 3, 1));
            } else {
                h.yday = Some(n2i(s2, 0, 3));
            }
        }
        5 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
                h.min = Some(n2i(s2, l2 - 4, 2));
                h.hour = Some(n2i(s2, l2 - 5, 1));
            } else {
                h.year = Some(signed(n2i(s2, 0, 2)));
                h.yday = Some(n2i(s2, 2, 3));
            }
        }
        7 => {
            if time_only {
                h.sec = Some(n2i(s2, l2 - 2, 2));
                h.min = Some(n2i(s2, l2 - 4, 2));
                h.hour = Some(n2i(s2, l2 - 6, 2));
                h.mday = Some(n2i(s2, l2 - 7, 1));
            } else {
                h.year = Some(signed(n2i(s2, 0, 4)));
                h.yday = Some(n2i(s2, 4, 3));
            }
        }
        _ => {}
    }
    if let Some(s3) = s3 {
        let l3 = s3.len();
        if matches!(l3, 2 | 4 | 6) {
            if s4.is_some() {
                h.sec = Some(n2i(s3, l3 - 2, 2));
                if l3 >= 4 {
                    h.min = Some(n2i(s3, l3 - 4, 2));
                }
                if l3 >= 6 {
                    h.hour = Some(n2i(s3, l3 - 6, 2));
                }
            } else {
                h.hour = Some(n2i(s3, 0, 2));
                if l3 >= 4 {
                    h.min = Some(n2i(s3, 2, 2));
                }
                if l3 >= 6 {
                    h.sec = Some(n2i(s3, 4, 2));
                }
            }
        }
    }
    if let Some(s4) = s4 {
        h.sec_fraction = Some(Fraction {
            numerator: str2num(s4),
            digits: s4.len(),
        });
    }
    if let Some(s5) = s5 {
        h.zone = Some(if let Some(inner) = s5.strip_prefix('[') {
            let inner = &inner[..inner.len() - 1];
            match inner.find(':') {
                Some(i) => inner[i + 1..].to_string(),
                None => inner.to_string(),
            }
        } else {
            s5.to_string()
        });
    }
    Ok(true)
}

fn parse_bc_cb(st: &mut State<'_>, _: &Captures, _: &str) -> Result<bool> {
    st.h.bc = true;
    Ok(true)
}

fn parse_frag_cb(st: &mut State<'_>, m: &Captures, text: &str) -> Result<bool> {
    let s = m.text(1, text).expect("digits");
    let h = &mut st.h;
    if h.hour.is_some() && h.mday.is_none() {
        let n = str2num(s);
        if n >= BigInt::from(1) && n <= BigInt::from(31) {
            h.mday = Some(n);
        }
    }
    if h.mday.is_some() && h.hour.is_none() {
        let n = str2num(s);
        if n >= BigInt::from(0) && n <= BigInt::from(24) {
            h.hour = Some(n);
        }
    }
    Ok(true)
}

impl State<'_> {
    fn iso2(&mut self) -> Result<bool> {
        if self.subs(pat_iso21(), ignore_cb)? || self.subs(pat_iso22(), ignore_cb)? {
            return Ok(true);
        }
        if self.subs(pat_iso23(), parse_iso23_cb)? || self.subs(pat_iso24(), parse_iso24_cb)? {
            return Ok(true);
        }
        if self.matches(pat_iso25_0())?.is_none() && self.subs(pat_iso25(), parse_iso25_cb)? {
            return Ok(true);
        }
        if self.matches(pat_iso26_0())?.is_none() && self.subs(pat_iso26(), parse_iso26_cb)? {
            return Ok(true);
        }
        Ok(false)
    }

    fn date_part(&mut self) -> Result<()> {
        if self.have(HAVE_ALPHA | HAVE_DIGIT)
            && (self.subs(pat_eu(), parse_eu_cb)? || self.subs(pat_us(), parse_us_cb)?)
        {
            return Ok(());
        }
        if self.have(HAVE_DIGIT | HAVE_DASH) && self.subs(pat_iso(), parse_iso_cb)? {
            return Ok(());
        }
        if self.have(HAVE_DIGIT | HAVE_DOT) && self.subs(pat_jis(), parse_jis_cb)? {
            return Ok(());
        }
        if self.have(HAVE_ALPHA | HAVE_DIGIT | HAVE_DASH)
            && (self.subs(pat_vms11(), parse_vms11_cb)?
                || self.subs(pat_vms12(), parse_vms12_cb)?)
        {
            return Ok(());
        }
        if self.have(HAVE_DIGIT | HAVE_SLASH) && self.subs(pat_sla(), parse_iso_cb)? {
            return Ok(());
        }
        if self.have(HAVE_DIGIT | HAVE_DOT) && self.subs(pat_dot(), parse_iso_cb)? {
            return Ok(());
        }
        if self.have(HAVE_DIGIT) && self.iso2()? {
            return Ok(());
        }
        if self.have(HAVE_DIGIT) && self.subs(pat_year(), parse_year_cb)? {
            return Ok(());
        }
        if self.have(HAVE_ALPHA) && self.subs(pat_mon(), parse_mon_cb)? {
            return Ok(());
        }
        if self.have(HAVE_DIGIT) && self.subs(pat_mday(), parse_mday_cb)? {
            return Ok(());
        }
        if self.have(HAVE_DIGIT) {
            self.subs(pat_ddd(), parse_ddd_cb)?;
        }
        Ok(())
    }
}

/// `Date._parse(input, true)` without the length check (the caller does it).
pub fn parse(input: &str) -> Result<Parsed> {
    let gsub = pat_gsub()
        .get()
        .ok_or_else(|| declined("a pattern", input))?;
    // gsub!(/[^-+',.\/:@[:alnum:]\[\]]+/, " ")
    let mut folded = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let m = gsub
            .captures(rest)
            .ok_or_else(|| declined("this text", input))?;
        let Some((b, e)) = m.and_then(|m| m.span(0)) else {
            folded.push_str(rest);
            break;
        };
        folded.push_str(&rest[..b]);
        folded.push(' ');
        rest = &rest[e..];
    }
    let mut st = State {
        input,
        str: folded,
        h: Parsed {
            comp: true,
            ..Parsed::default()
        },
    };
    if st.have(HAVE_ALPHA) {
        st.subs(pat_day(), parse_day_cb)?;
    }
    if st.have(HAVE_DIGIT) {
        st.subs(pat_time(), parse_time_cb)?;
    }
    st.date_part()?;
    if st.have(HAVE_ALPHA) {
        st.subs(pat_bc(), parse_bc_cb)?;
    }
    if st.have(HAVE_DIGIT) {
        st.subs(pat_frag(), parse_frag_cb)?;
    }
    let mut h = st.h;
    if std::mem::take(&mut h.bc) {
        if let Some(y) = h.year.take() {
            h.year = Some(-y + 1);
        }
    }
    if h.comp {
        if let Some(y) = &h.year {
            if *y >= BigInt::from(0) && *y <= BigInt::from(99) {
                let add = if *y >= BigInt::from(69) { 1900 } else { 2000 };
                h.year = Some(y + add);
            }
        }
    }
    Ok(h)
}
