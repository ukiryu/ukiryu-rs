//! Port of the parts of Ruby's `date` extension (gem 3.6.0, `date_parse.c`
//! and `date_core.c`) that `Ukiryu::ManPageParser` reaches: `Date._parse`
//! in its default (non-`TIGHT_PARSER`) build, `Date.parse` (fragment
//! completion and validation with the default `Date::ITALY` start),
//! `Date.new(y, m, d)` and `Date#strftime('%Y-%m-%d')`.
//!
//! The `offset` fragment (`date_zone_to_diff`) is not computed: `Date`
//! results never read it. The `wnum0`/`wnum1`/`jd`/`seconds` completion
//! branches are not ported because `Date._parse` never produces those keys.

use super::onig::{Match, Regex};
use num_bigint::BigInt;
use std::fmt;
use std::sync::OnceLock;

/// The calendar day `Date.today` returned for the parse. `Date.parse` uses
/// it to fill fragments the text does not give (for example the year of
/// `"September 21"`). Ruby reads the process-local clock; the port takes it
/// explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Today {
    pub year: i32,
    pub mon: i32,
    pub mday: i32,
}

/// `Date._parse` fragments that influence a `Date`. Integers are kept
/// unbounded, as Ruby's are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fragments {
    pub year: Option<BigInt>,
    pub mon: Option<BigInt>,
    pub mday: Option<BigInt>,
    pub yday: Option<BigInt>,
    pub wday: Option<BigInt>,
    pub cwyear: Option<BigInt>,
    pub cweek: Option<BigInt>,
    pub cwday: Option<BigInt>,
    pub hour: Option<BigInt>,
    pub min: Option<BigInt>,
    pub sec: Option<BigInt>,
    /// `sec_fraction` as a reduced `(numerator, denominator)`.
    pub sec_fraction: Option<(BigInt, BigInt)>,
    pub zone: Option<String>,
    jd: Option<BigInt>,
    bc: bool,
    comp: Option<bool>,
}

/// A failure raised by the ported Ruby code, with its Ruby class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DateFailure {
    /// `ArgumentError` (for example the 128-byte `Date.parse` limit).
    Argument(String),
    /// `Date::Error` (a subclass of `ArgumentError`): always `"invalid date"`.
    Invalid,
    /// `RangeError` from `NUM2INT`.
    Range(String),
}

impl DateFailure {
    /// Whether Ruby's `rescue ArgumentError, Date::Error` catches it.
    pub fn is_argument_error(&self) -> bool {
        !matches!(self, DateFailure::Range(_))
    }
}

impl fmt::Display for DateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DateFailure::Argument(m) | DateFailure::Range(m) => f.write_str(m),
            DateFailure::Invalid => f.write_str("invalid date"),
        }
    }
}

/// A proleptic calendar date as `Date` reports it (`year` may be any size).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CivilDate {
    pub year: BigInt,
    pub mon: i32,
    pub mday: i32,
}

impl CivilDate {
    /// `Date#strftime('%Y-%m-%d')`.
    pub fn iso8601(&self) -> String {
        let digits = self.year.magnitude().to_string();
        let sign = if self.year.sign() == num_bigint::Sign::Minus {
            "-"
        } else {
            ""
        };
        format!("{sign}{digits:0>4}-{:02}-{:02}", self.mon, self.mday)
    }
}

// ---------------------------------------------------------------------------
// Date._parse
// ---------------------------------------------------------------------------

const MONTHS: &str = "jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec";

struct Patterns {
    day: Regex,
    time: Regex,
    time_inner: Regex,
    eu: Regex,
    us: Regex,
    iso: Regex,
    iso21: Regex,
    iso22: Regex,
    iso23: Regex,
    iso24: Regex,
    iso25_0: Regex,
    iso25: Regex,
    iso26_0: Regex,
    iso26: Regex,
    jis: Regex,
    vms11: Regex,
    vms12: Regex,
    sla: Regex,
    dot: Regex,
    year: Regex,
    mon: Regex,
    mday: Regex,
    ddd: Regex,
    bc: Regex,
    frag: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let era = r"(c(?:e|\.e\.)|b(?:ce|\.c\.e\.)|a(?:d|\.d\.)|b(?:c|\.c\.))";
        Patterns {
            day: Regex::new(r"\b(sun|mon|tue|wed|thu|fri|sat)[^-/\d\s]*", true),
            time: Regex::new(
                concat!(
                    r"((?<!\d)\d+\s*(?:(?::\s*\d+(?:\s*:\s*\d+(?:[,.]\d*)?)?",
                    r"|h(?:\s*\d+m?(?:\s*\d+s?)?)?)(?:\s*[ap](?:m\b|\.m\.))?",
                    r"|[ap](?:m\b|\.m\.)))",
                    r"(?:\s*((?:gmt|utc?)?[-+]\d+(?:[,.:]\d+(?::\d+)?)?",
                    r"|(?-i:[[:alpha:].\s]+)(?:standard|daylight)\stime\b",
                    r"|(?-i:[[:alpha:]]+)(?:\sdst)?\b))?"
                ),
                true,
            ),
            time_inner: Regex::new(
                r"\A(\d+)h?(?:\s*:?\s*(\d+)m?(?:\s*:?\s*(\d+)(?:[,.](\d+))?s?)?)?(?:\s*([ap])(?:m\b|\.m\.))?",
                true,
            ),
            eu: Regex::new(
                &format!(
                    r"('?(?<!\d)\d+)[^-\d\s]*\s*({MONTHS})[^-\d\s']*(?:\s*(?:\b{era}(?!(?<!\.)[a-z]))?\s*('?-?\d+(?:(?:st|nd|rd|th)\b)?))?"
                ),
                true,
            ),
            us: Regex::new(
                &format!(
                    r"\b({MONTHS})[^-\d\s']*\s*('?\d+)[^-\d\s']*(?:\s*+,?\s*+{era}?\s*('?-?\d+))?"
                ),
                true,
            ),
            iso: Regex::new(r"('?[-+]?(?<!\d)\d+)-(\d+)-('?-?\d+)", false),
            iso21: Regex::new(r"\b(\d{2}|\d{4})?-?w(\d{2})(?:-?(\d))?\b", true),
            iso22: Regex::new(r"-w-(\d)\b", true),
            iso23: Regex::new(r"--(\d{2})?-(\d{2})\b", false),
            iso24: Regex::new(r"--(\d{2})(\d{2})?\b", false),
            iso25_0: Regex::new(r"[,.](\d{2}|\d{4})-\d{3}\b", false),
            iso25: Regex::new(r"\b(\d{2}|\d{4})-(\d{3})\b", false),
            iso26_0: Regex::new(r"\d-\d{3}\b", false),
            iso26: Regex::new(r"\b-(\d{3})\b", false),
            jis: Regex::new(r"\b([mtshr])(\d+)\.(\d+)\.(\d+)", true),
            vms11: Regex::new(
                &format!(r"('?-?(?<!\d)\d+)-({MONTHS})[^-/.]*-('?-?\d+)"),
                true,
            ),
            vms12: Regex::new(
                &format!(r"\b({MONTHS})[^-/.]*-('?-?\d+)(?:-('?-?\d+))?"),
                true,
            ),
            sla: Regex::new(r"('?-?(?<!\d)\d+)/\s*('?\d+)(?:\D\s*('?-?\d+))?", true),
            dot: Regex::new(r"('?-?(?<!\d)\d+)\.\s*('?\d+)\.\s*('?-?\d+)", true),
            year: Regex::new(r"'(\d+)\b", false),
            mon: Regex::new(&format!(r"\b({MONTHS})\S*"), true),
            mday: Regex::new(r"((?<!\d)\d+)(st|nd|rd|th)\b", true),
            ddd: Regex::new(
                r"([-+]?)((?<!\d)\d{2,14})(?:\s*t?\s*(\d{2,6})?(?:[,.](\d*))?)?(?:\s*(z\b|[-+]\d{1,4}\b|\[[-+]?\d[^\]]*\]))?",
                true,
            ),
            bc: Regex::new(r"\b(bc\b|bce\b|b\.c\.|b\.c\.e\.)", true),
            frag: Regex::new(r"\A\s*(\d{1,2})\s*\z", true),
        }
    })
}

struct Subject {
    chars: Vec<char>,
}

impl Subject {
    /// `subx`: replace the first match with one space, then run the callback.
    fn subx(&mut self, re: &Regex) -> Option<(Match, Vec<char>)> {
        let found = re.find(&self.chars)?;
        let original = self.chars.clone();
        let (b, e) = found.range(0).expect("group 0");
        self.chars.splice(b..e, [' ']);
        Some((found, original))
    }

    fn class(&self) -> u8 {
        let mut flags = 0;
        for c in &self.chars {
            // `check_class` inspects bytes with C-locale `isalpha`/`isdigit`.
            if c.is_ascii_alphabetic() {
                flags |= ALPHA;
            }
            if c.is_ascii_digit() {
                flags |= DIGIT;
            }
            match c {
                '-' => flags |= DASH,
                '.' => flags |= DOT,
                '/' => flags |= SLASH,
                _ => {}
            }
        }
        flags
    }

    fn has(&self, want: u8) -> bool {
        self.class() & want == want
    }
}

const ALPHA: u8 = 1;
const DIGIT: u8 = 2;
const DASH: u8 = 4;
const DOT: u8 = 8;
const SLASH: u8 = 16;

fn num(s: &str) -> BigInt {
    // `rb_str_to_inum(s, 10, 0)` on the digit-only (optionally signed)
    // captures these callers pass. An empty string is 0.
    let t = s.strip_prefix('+').unwrap_or(s);
    if t.is_empty() || t == "-" {
        return BigInt::from(0);
    }
    t.parse::<BigInt>().unwrap_or_default()
}

fn group(m: &Match, chars: &[char], i: usize) -> Option<String> {
    m.text(chars, i)
}

fn num2int(v: &BigInt) -> Result<i32, DateFailure> {
    // `NUM2INT`: Fixnums outside `int` and Bignums raise different messages.
    if let Ok(i) = i32::try_from(v) {
        return Ok(i);
    }
    let fixnum_max = BigInt::from((1i64 << 62) - 1);
    let fixnum_min = BigInt::from(-(1i64 << 62));
    if *v > fixnum_max || *v < fixnum_min {
        return Err(DateFailure::Range(
            "bignum too big to convert into 'long'".to_string(),
        ));
    }
    let dir = if v.sign() == num_bigint::Sign::Minus {
        "small"
    } else {
        "big"
    };
    Err(DateFailure::Range(format!(
        "integer {v} too {dir} to convert to 'int'"
    )))
}

fn mon_num(s: &str) -> BigInt {
    let b = s.as_bytes();
    let idx = MONTHS
        .split('|')
        .position(|m| b.len() >= 3 && b[..3].eq_ignore_ascii_case(m.as_bytes()))
        .unwrap_or(12);
    BigInt::from(idx + 1)
}

fn day_num(s: &str) -> BigInt {
    let b = s.as_bytes();
    let idx = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
        .iter()
        .position(|d| b.len() >= 3 && b[..3].eq_ignore_ascii_case(d.as_bytes()))
        .unwrap_or(7);
    BigInt::from(idx)
}

/// `s3e`: assign year/month/day from loosely ordered strings.
fn s3e(h: &mut Fragments, y: Option<String>, m: Option<String>, d: Option<String>, bc: bool) {
    let mut y = y.map(String::into_bytes);
    let mut m = m.map(String::into_bytes);
    let mut d = d.map(String::into_bytes);
    let mut comp: Option<bool> = None;
    let issign = |c: u8| c == b'-' || c == b'+';

    if y.is_some() && m.is_some() && d.is_none() {
        let (oy, om, od) = (y.take(), m.take(), d.take());
        y = od;
        m = oy;
        d = om;
    }

    if y.is_none() {
        if d.as_ref().is_some_and(|d| d.len() > 2) {
            y = d.take();
        }
        if d.as_ref().is_some_and(|d| d.first() == Some(&b'\'')) {
            y = d.take();
        }
    }

    if let Some(ys) = y.clone() {
        let mut s = 0;
        while s < ys.len() && !issign(ys[s]) && !ys[s].is_ascii_digit() {
            s += 1;
        }
        if s < ys.len() {
            let bp = s;
            if issign(ys[s]) {
                s += 1;
            }
            let mut ep = s;
            while ep < ys.len() && ys[ep].is_ascii_digit() {
                ep += 1;
            }
            if ep < ys.len() {
                let old_d = d.take();
                d = Some(ys[bp..ep].to_vec());
                y = old_d;
            }
        }
    }

    if let Some(ms) = m.as_ref() {
        if ms.first() == Some(&b'\'') || ms.len() > 2 {
            // us -> be
            let (oy, om, od) = (y.take(), m.take(), d.take());
            y = om;
            m = od;
            d = oy;
        }
    }

    if let Some(ds) = d.as_ref() {
        if ds.first() == Some(&b'\'') || ds.len() > 2 {
            let (oy, od) = (y.take(), d.take());
            y = od;
            d = oy;
        }
    }

    if let Some(ys) = y.as_ref() {
        let mut s = 0;
        while s < ys.len() && !issign(ys[s]) && !ys[s].is_ascii_digit() {
            s += 1;
        }
        if s < ys.len() {
            let bp = s;
            let mut sign = false;
            if issign(ys[s]) {
                s += 1;
                sign = true;
            }
            if sign {
                comp = Some(false);
            }
            let mut ep = s;
            while ep < ys.len() && ys[ep].is_ascii_digit() {
                ep += 1;
            }
            if ep - s > 2 {
                comp = Some(false);
            }
            let text = String::from_utf8_lossy(&ys[bp..ep]).into_owned();
            h.year = Some(cstr2num(&text));
        }
    }

    if bc {
        h.bc = true;
    }

    if let Some(ms) = m.as_ref() {
        if let Some(v) = digits_after(ms) {
            h.mon = Some(v);
        }
    }

    if let Some(ds) = d.as_ref() {
        if let Some(v) = digits_after(ds) {
            h.mday = Some(v);
        }
    }

    if let Some(c) = comp {
        h.comp = Some(c);
    }
}

fn cstr2num(s: &str) -> BigInt {
    // `rb_cstr_to_inum(s, 10, 0)`: a lone sign is 0.
    num(s)
}

fn digits_after(s: &[u8]) -> Option<BigInt> {
    let mut i = 0;
    while i < s.len() && !s[i].is_ascii_digit() {
        i += 1;
    }
    if i >= s.len() {
        return None;
    }
    let mut e = i;
    while e < s.len() && s[e].is_ascii_digit() {
        e += 1;
    }
    Some(num(std::str::from_utf8(&s[i..e]).expect("ascii digits")))
}

fn first_byte(s: &Option<String>) -> Option<u8> {
    s.as_ref().and_then(|v| v.as_bytes().first().copied())
}

fn n2i(s: &[u8], f: usize, w: usize) -> BigInt {
    let mut v: i64 = 0;
    for b in &s[f..f + w] {
        v = v * 10 + i64::from(b - b'0');
    }
    BigInt::from(v)
}

fn rational(digits: &str) -> (BigInt, BigInt) {
    let mut n = num(digits);
    let mut d = BigInt::from(10).pow(digits.chars().count() as u32);
    let g = gcd(n.clone(), d.clone());
    if g != BigInt::from(0) {
        n /= &g;
        d /= &g;
    }
    (n, d)
}

fn gcd(mut a: BigInt, mut b: BigInt) -> BigInt {
    let zero = BigInt::from(0);
    while b != zero {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}

/// `Date._parse(str)` with `comp = true`, without the `offset` key.
pub fn date_parse_fragments(input: &str) -> Result<Fragments, DateFailure> {
    check_limit(input)?;
    let p = patterns();
    let mut h = Fragments {
        comp: Some(true),
        ..Fragments::default()
    };
    let mut chars: Vec<char> = input.chars().collect();
    // gsub!(/[^-+',./:@[:alnum:]\[\]]+/, ' '): every maximal run of other
    // characters becomes one space.
    {
        let keep = |c: char| c.is_alphanumeric() || "-+',./:@[]".contains(c);
        let mut out = Vec::with_capacity(chars.len());
        let mut in_run = false;
        for c in chars {
            if keep(c) {
                out.push(c);
                in_run = false;
            } else if !in_run {
                out.push(' ');
                in_run = true;
            }
        }
        chars = out;
    }
    let mut s = Subject { chars };

    if s.has(ALPHA) {
        if let Some((m, c)) = s.subx(&p.day) {
            h.wday = Some(day_num(&group(&m, &c, 1).expect("day")));
        }
    }
    if s.has(DIGIT) {
        if let Some((m, c)) = s.subx(&p.time) {
            parse_time_cb(&mut h, &m, &c, p)?;
        }
    }

    let ok = (s.has(ALPHA | DIGIT) && (parse_eu(&mut s, &mut h, p) || parse_us(&mut s, &mut h, p)))
        || (s.has(DIGIT | DASH) && parse_iso(&mut s, &mut h, p))
        || (s.has(DIGIT | DOT) && parse_jis(&mut s, &mut h, p))
        || (s.has(ALPHA | DIGIT | DASH) && parse_vms(&mut s, &mut h, p))
        || (s.has(DIGIT | SLASH) && parse_sla(&mut s, &mut h, p))
        || (s.has(DIGIT | DOT) && parse_dot(&mut s, &mut h, p))
        || (s.has(DIGIT) && parse_iso2(&mut s, &mut h, p))
        || (s.has(DIGIT) && parse_year(&mut s, &mut h, p))
        || (s.has(ALPHA) && parse_mon(&mut s, &mut h, p))
        || (s.has(DIGIT) && parse_mday(&mut s, &mut h, p))
        || (s.has(DIGIT) && parse_ddd(&mut s, &mut h, p));
    let _ = ok;

    if s.has(ALPHA) && s.subx(&p.bc).is_some() {
        h.bc = true;
    }
    if s.has(DIGIT) {
        if let Some((m, c)) = s.subx(&p.frag) {
            let v = num(&group(&m, &c, 1).expect("frag"));
            if h.hour.is_some() && h.mday.is_none() && v >= 1.into() && v <= 31.into() {
                h.mday = Some(v.clone());
            }
            if h.mday.is_some() && h.hour.is_none() && v >= 0.into() && v <= 24.into() {
                h.hour = Some(v);
            }
        }
    }

    if std::mem::take(&mut h.bc) {
        if let Some(y) = h.cwyear.take() {
            h.cwyear = Some(-y + 1);
        }
        if let Some(y) = h.year.take() {
            h.year = Some(-y + 1);
        }
    }
    if h.comp.take() == Some(true) {
        for y in [&mut h.cwyear, &mut h.year].into_iter().flatten() {
            if *y >= 0.into() && *y <= 99.into() {
                *y += if *y >= 69.into() { 1900 } else { 2000 };
            }
        }
    }
    Ok(h)
}

fn check_limit(input: &str) -> Result<(), DateFailure> {
    let len = input.len();
    if len > 128 {
        return Err(DateFailure::Argument(format!(
            "string length ({len}) exceeds the limit 128"
        )));
    }
    Ok(())
}

fn parse_time_cb(
    h: &mut Fragments,
    m: &Match,
    c: &[char],
    p: &Patterns,
) -> Result<(), DateFailure> {
    let s1: Vec<char> = group(m, c, 1).expect("time").chars().collect();
    if let Some(zone) = group(m, c, 2) {
        h.zone = Some(zone);
    }
    let Some(t) = p.time_inner.find(&s1) else {
        return Ok(());
    };
    let mut hour = num(&group(&t, &s1, 1).expect("hour"));
    let min = group(&t, &s1, 2).map(|v| num(&v));
    let sec = group(&t, &s1, 3).map(|v| num(&v));
    let frac = group(&t, &s1, 4).map(|v| rational(&v));
    if let Some(ap) = group(&t, &s1, 5) {
        let mut ih = num2int(&hour)? % 12;
        if ap.starts_with(['P', 'p']) {
            ih += 12;
        }
        hour = BigInt::from(ih);
    }
    h.hour = Some(hour);
    if min.is_some() {
        h.min = min;
    }
    if sec.is_some() {
        h.sec = sec;
    }
    if frac.is_some() {
        h.sec_fraction = frac;
    }
    Ok(())
}

fn era_bc(b: &Option<String>) -> bool {
    matches!(first_byte(b), Some(b'B' | b'b'))
}

fn parse_eu(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.eu) else {
        return false;
    };
    let d = group(&m, &c, 1);
    let mon = mon_num(&group(&m, &c, 2).expect("mon")).to_string();
    let b = group(&m, &c, 3);
    let y = group(&m, &c, 4);
    s3e(h, y, Some(mon), d, era_bc(&b));
    true
}

fn parse_us(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.us) else {
        return false;
    };
    let mon = mon_num(&group(&m, &c, 1).expect("mon")).to_string();
    let d = group(&m, &c, 2);
    let b = group(&m, &c, 3);
    let y = group(&m, &c, 4);
    s3e(h, y, Some(mon), d, era_bc(&b));
    true
}

fn parse_iso(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.iso) else {
        return false;
    };
    s3e(
        h,
        group(&m, &c, 1),
        group(&m, &c, 2),
        group(&m, &c, 3),
        false,
    );
    true
}

fn parse_iso2(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    if let Some((m, c)) = s.subx(&p.iso21) {
        if let Some(y) = group(&m, &c, 1) {
            h.cwyear = Some(num(&y));
        }
        h.cweek = Some(num(&group(&m, &c, 2).expect("week")));
        if let Some(d) = group(&m, &c, 3) {
            h.cwday = Some(num(&d));
        }
        return true;
    }
    if let Some((m, c)) = s.subx(&p.iso22) {
        h.cwday = Some(num(&group(&m, &c, 1).expect("cwday")));
        return true;
    }
    if let Some((m, c)) = s.subx(&p.iso23) {
        if let Some(mon) = group(&m, &c, 1) {
            h.mon = Some(num(&mon));
        }
        h.mday = Some(num(&group(&m, &c, 2).expect("mday")));
        return true;
    }
    if let Some((m, c)) = s.subx(&p.iso24) {
        h.mon = Some(num(&group(&m, &c, 1).expect("mon")));
        if let Some(d) = group(&m, &c, 2) {
            h.mday = Some(num(&d));
        }
        return true;
    }
    if p.iso25_0.find(&s.chars).is_none() {
        if let Some((m, c)) = s.subx(&p.iso25) {
            h.year = Some(num(&group(&m, &c, 1).expect("year")));
            h.yday = Some(num(&group(&m, &c, 2).expect("yday")));
            return true;
        }
    }
    if p.iso26_0.find(&s.chars).is_none() {
        if let Some((m, c)) = s.subx(&p.iso26) {
            h.yday = Some(num(&group(&m, &c, 1).expect("yday")));
            return true;
        }
    }
    false
}

fn parse_jis(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.jis) else {
        return false;
    };
    let era = match first_byte(&group(&m, &c, 1)) {
        Some(b'M' | b'm') => 1867,
        Some(b'T' | b't') => 1911,
        Some(b'S' | b's') => 1925,
        Some(b'H' | b'h') => 1988,
        Some(b'R' | b'r') => 2018,
        _ => 0,
    };
    h.year = Some(num(&group(&m, &c, 2).expect("y")) + era);
    h.mon = Some(num(&group(&m, &c, 3).expect("mon")));
    h.mday = Some(num(&group(&m, &c, 4).expect("mday")));
    true
}

fn parse_vms(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    if let Some((m, c)) = s.subx(&p.vms11) {
        let d = group(&m, &c, 1);
        let mon = mon_num(&group(&m, &c, 2).expect("mon")).to_string();
        let y = group(&m, &c, 3);
        s3e(h, y, Some(mon), d, false);
        return true;
    }
    if let Some((m, c)) = s.subx(&p.vms12) {
        let mon = mon_num(&group(&m, &c, 1).expect("mon")).to_string();
        let d = group(&m, &c, 2);
        let y = group(&m, &c, 3);
        s3e(h, y, Some(mon), d, false);
        return true;
    }
    false
}

fn parse_sla(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.sla) else {
        return false;
    };
    s3e(
        h,
        group(&m, &c, 1),
        group(&m, &c, 2),
        group(&m, &c, 3),
        false,
    );
    true
}

fn parse_dot(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.dot) else {
        return false;
    };
    s3e(
        h,
        group(&m, &c, 1),
        group(&m, &c, 2),
        group(&m, &c, 3),
        false,
    );
    true
}

fn parse_year(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.year) else {
        return false;
    };
    h.year = Some(num(&group(&m, &c, 1).expect("year")));
    true
}

fn parse_mon(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.mon) else {
        return false;
    };
    h.mon = Some(mon_num(&group(&m, &c, 1).expect("mon")));
    true
}

fn parse_mday(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.mday) else {
        return false;
    };
    h.mday = Some(num(&group(&m, &c, 1).expect("mday")));
    true
}

fn parse_ddd(s: &mut Subject, h: &mut Fragments, p: &Patterns) -> bool {
    let Some((m, c)) = s.subx(&p.ddd) else {
        return false;
    };
    let s1 = group(&m, &c, 1);
    let s2 = group(&m, &c, 2).expect("digits");
    let s3 = group(&m, &c, 3);
    let s4 = group(&m, &c, 4);
    let s5 = group(&m, &c, 5);
    let neg = first_byte(&s1) == Some(b'-');
    let signed = |v: BigInt| if neg { -v } else { v };
    let cs2 = s2.as_bytes();
    let l2 = cs2.len();
    let time_only = s3.is_none() && s4.is_some();
    match l2 {
        2 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
            } else {
                h.mday = Some(n2i(cs2, 0, 2));
            }
        }
        4 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
                h.min = Some(n2i(cs2, l2 - 4, 2));
            } else {
                h.mon = Some(n2i(cs2, 0, 2));
                h.mday = Some(n2i(cs2, 2, 2));
            }
        }
        6 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
                h.min = Some(n2i(cs2, l2 - 4, 2));
                h.hour = Some(n2i(cs2, l2 - 6, 2));
            } else {
                h.year = Some(signed(n2i(cs2, 0, 2)));
                h.mon = Some(n2i(cs2, 2, 2));
                h.mday = Some(n2i(cs2, 4, 2));
            }
        }
        8 | 10 | 12 | 14 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
                h.min = Some(n2i(cs2, l2 - 4, 2));
                h.hour = Some(n2i(cs2, l2 - 6, 2));
                h.mday = Some(n2i(cs2, l2 - 8, 2));
                if l2 >= 10 {
                    h.mon = Some(n2i(cs2, l2 - 10, 2));
                }
                if l2 == 12 {
                    h.year = Some(signed(n2i(cs2, l2 - 12, 2)));
                }
                if l2 == 14 {
                    h.year = Some(signed(n2i(cs2, l2 - 14, 4)));
                    h.comp = Some(false);
                }
            } else {
                h.year = Some(signed(n2i(cs2, 0, 4)));
                h.mon = Some(n2i(cs2, 4, 2));
                h.mday = Some(n2i(cs2, 6, 2));
                if l2 >= 10 {
                    h.hour = Some(n2i(cs2, 8, 2));
                }
                if l2 >= 12 {
                    h.min = Some(n2i(cs2, 10, 2));
                }
                if l2 >= 14 {
                    h.sec = Some(n2i(cs2, 12, 2));
                }
                h.comp = Some(false);
            }
        }
        3 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
                h.min = Some(n2i(cs2, l2 - 3, 1));
            } else {
                h.yday = Some(n2i(cs2, 0, 3));
            }
        }
        5 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
                h.min = Some(n2i(cs2, l2 - 4, 2));
                h.hour = Some(n2i(cs2, l2 - 5, 1));
            } else {
                h.year = Some(signed(n2i(cs2, 0, 2)));
                h.yday = Some(n2i(cs2, 2, 3));
            }
        }
        7 => {
            if time_only {
                h.sec = Some(n2i(cs2, l2 - 2, 2));
                h.min = Some(n2i(cs2, l2 - 4, 2));
                h.hour = Some(n2i(cs2, l2 - 6, 2));
                h.mday = Some(n2i(cs2, l2 - 7, 1));
            } else {
                h.year = Some(signed(n2i(cs2, 0, 4)));
                h.yday = Some(n2i(cs2, 4, 3));
            }
        }
        _ => {}
    }
    if let Some(s3) = s3 {
        let cs3 = s3.as_bytes();
        let l3 = cs3.len();
        if matches!(l3, 2 | 4 | 6) {
            if s4.is_some() {
                h.sec = Some(n2i(cs3, l3 - 2, 2));
                if l3 >= 4 {
                    h.min = Some(n2i(cs3, l3 - 4, 2));
                }
                if l3 >= 6 {
                    h.hour = Some(n2i(cs3, l3 - 6, 2));
                }
            } else {
                h.hour = Some(n2i(cs3, 0, 2));
                if l3 >= 4 {
                    h.min = Some(n2i(cs3, 2, 2));
                }
                if l3 >= 6 {
                    h.sec = Some(n2i(cs3, 4, 2));
                }
            }
        }
    }
    if let Some(s4) = s4 {
        h.sec_fraction = Some(rational(&s4));
    }
    if let Some(s5) = s5 {
        let mut zone = s5.clone();
        if let Some(inner) = s5.strip_prefix('[') {
            let inner = inner.strip_suffix(']').unwrap_or(inner);
            zone = match inner.find(':') {
                Some(i) => inner[i + 1..].to_string(),
                None => inner.to_string(),
            };
        }
        h.zone = Some(zone);
    }
    true
}

// ---------------------------------------------------------------------------
// Calendar arithmetic (date_core.c) with the default start Date::ITALY
// ---------------------------------------------------------------------------

const ITALY: i64 = 2_299_161;
const CM_PERIOD: i64 = 213_447_717;
const CM_PERIOD_GCY: i64 = 584_400;
const CM_PERIOD_JCY: i64 = 584_388;

/// The calendar-reform start: `ITALY`, or pure Julian/Gregorian (`+inf`/`-inf`).
#[derive(Clone, Copy, PartialEq)]
enum Sg {
    Reform,
    Julian,
    Gregorian,
}

impl Sg {
    fn before(self, jd: i64) -> bool {
        // `jd < sg`
        match self {
            Sg::Reform => jd < ITALY,
            Sg::Julian => true,
            Sg::Gregorian => false,
        }
    }
}

fn div(n: i64, d: i64) -> i64 {
    n.div_euclid(d)
}

fn modulo(n: i64, d: i64) -> i64 {
    n.rem_euclid(d)
}

fn gregorian_civil_to_jd(y: i64, m: i64, d: i64) -> i64 {
    let (y0, m0) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
    let q1 = div(y0, 100);
    let yc = div(1461 * y0, 4) - q1 + div(q1, 4);
    let mc = (979 * m0 - 2919) / 32;
    yc + mc + (d - 1) + 1_721_120
}

fn gregorian_jd_to_civil(jd: i64) -> (i64, i64, i64) {
    let a = jd + 32044;
    let b = div(4 * a + 3, 146_097);
    let c = a - div(146_097 * b, 4);
    let d = div(4 * c + 3, 1461);
    let e = c - div(1461 * d, 4);
    let m = div(5 * e + 2, 153);
    let day = e - div(153 * m + 2, 5) + 1;
    let month = m + 3 - 12 * div(m, 10);
    let year = 100 * b + d - 4800 + div(m, 10);
    (year, month, day)
}

fn civil_to_jd(y: i64, m: i64, d: i64, sg: Sg) -> i64 {
    if sg == Sg::Gregorian {
        return gregorian_civil_to_jd(y, m, d);
    }
    let jd = gregorian_civil_to_jd(y, m, d);
    if sg.before(jd) {
        let (y2, m2) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
        let v =
            (365.25 * (y2 + 4716) as f64).floor() + (30.6001 * (m2 + 1) as f64).floor() + d as f64
                - 1524.0;
        v as i64
    } else {
        jd
    }
}

fn jd_to_civil(jd: i64, sg: Sg) -> (i64, i64, i64) {
    if sg == Sg::Gregorian || !sg.before(jd) {
        return gregorian_jd_to_civil(jd);
    }
    let a = jd as f64;
    let b = a + 1524.0;
    let c = ((b - 122.1) / 365.25).floor();
    let d = (365.25 * c).floor();
    let e = ((b - d) / 30.6001).floor();
    let dom = b - d - (30.6001 * e).floor();
    let (m, y) = if e <= 13.0 {
        (e - 1.0, c - 4716.0)
    } else {
        (e - 13.0, c - 4715.0)
    };
    (y as i64, m as i64, dom as i64)
}

fn valid_civil_c(y: i64, mut m: i64, mut d: i64, sg: Sg) -> Option<i64> {
    if m < 0 {
        m += 13;
    }
    if !(1..=12).contains(&m) {
        return None;
    }
    if d < 0 {
        let ldom = find_ldom(y, m, sg)?;
        let (ry, rm, rd) = jd_to_civil(ldom + d + 1, sg);
        if ry != y || rm != m {
            return None;
        }
        d = rd;
    }
    let jd = civil_to_jd(y, m, d, sg);
    let (ry, rm, rd) = jd_to_civil(jd, sg);
    (ry == y && rm == m && rd == d).then_some(jd)
}

fn find_ldom(y: i64, m: i64, sg: Sg) -> Option<i64> {
    if sg == Sg::Gregorian {
        let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        return Some(gregorian_civil_to_jd(ny, nm, 1) - 1);
    }
    (0..30).find_map(|i| valid_civil_c(y, m, 31 - i, sg))
}

fn find_fdoy(y: i64, sg: Sg) -> Option<i64> {
    if sg == Sg::Gregorian {
        return Some(gregorian_civil_to_jd(y, 1, 1));
    }
    (1..31).find_map(|d| valid_civil_c(y, 1, d, sg))
}

fn find_ldoy(y: i64, sg: Sg) -> Option<i64> {
    if sg == Sg::Gregorian {
        return Some(gregorian_civil_to_jd(y + 1, 1, 1) - 1);
    }
    (0..30).find_map(|i| valid_civil_c(y, 12, 31 - i, sg))
}

fn julian_leap(y: i64) -> bool {
    modulo(y, 4) == 0
}

fn gregorian_leap(y: i64) -> bool {
    (modulo(y, 4) == 0 && y % 100 != 0) || modulo(y, 400) == 0
}

fn last_day(leap: bool, m: i64) -> i64 {
    const T: [[i64; 13]; 2] = [
        [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
        [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
    ];
    T[usize::from(leap)][m as usize]
}

fn valid_pure_civil(y: i64, mut m: i64, mut d: i64, gregorian: bool) -> Option<(i64, i64)> {
    if m < 0 {
        m += 13;
    }
    if !(1..=12).contains(&m) {
        return None;
    }
    let leap = if gregorian {
        gregorian_leap(y)
    } else {
        julian_leap(y)
    };
    let last = last_day(leap, m);
    if d < 0 {
        d = last + d + 1;
    }
    if d < 1 || d > last {
        return None;
    }
    Some((m, d))
}

fn ordinal_to_jd(y: i64, d: i64, sg: Sg) -> i64 {
    find_fdoy(y, sg).unwrap_or(0) + d - 1
}

fn jd_to_ordinal(jd: i64, sg: Sg) -> (i64, i64) {
    let (ry, _, _) = jd_to_civil(jd, sg);
    let first = find_fdoy(ry, sg).unwrap_or(0);
    (ry, jd - first + 1)
}

fn valid_ordinal_c(y: i64, mut d: i64, sg: Sg) -> Option<i64> {
    if d < 0 {
        let ldoy = find_ldoy(y, sg)?;
        let (ry2, rd2) = jd_to_ordinal(ldoy + d + 1, sg);
        if ry2 != y {
            return None;
        }
        d = rd2;
    }
    let jd = ordinal_to_jd(y, d, sg);
    let (ry2, rd2) = jd_to_ordinal(jd, sg);
    (ry2 == y && rd2 == d).then_some(jd)
}

fn commercial_to_jd(y: i64, w: i64, d: i64, sg: Sg) -> i64 {
    let rjd2 = find_fdoy(y, sg).unwrap_or(0) + 3;
    (rjd2 - modulo(rjd2, 7)) + 7 * (w - 1) + (d - 1)
}

fn jd_to_commercial(jd: i64, sg: Sg) -> (i64, i64, i64) {
    let (a, _, _) = jd_to_civil(jd - 3, sg);
    let mut rjd2 = commercial_to_jd(a + 1, 1, 1, sg);
    let ry = if jd >= rjd2 {
        a + 1
    } else {
        rjd2 = commercial_to_jd(a, 1, 1, sg);
        a
    };
    let rw = 1 + div(jd - rjd2, 7);
    let mut rd = modulo(jd + 1, 7);
    if rd == 0 {
        rd = 7;
    }
    (ry, rw, rd)
}

fn valid_commercial_c(y: i64, mut w: i64, mut d: i64, sg: Sg) -> Option<i64> {
    if d < 0 {
        d += 8;
    }
    if w < 0 {
        if w < -53 {
            return None;
        }
        let rjd2 = commercial_to_jd(y + 1, 1, 1, sg);
        let (ry2, rw2, _) = jd_to_commercial(rjd2 + w * 7, sg);
        if ry2 != y {
            return None;
        }
        w = rw2;
    } else if !(1..=53).contains(&w) {
        return None;
    }
    let jd = commercial_to_jd(y, w, d, sg);
    let (ry2, rw, rd) = jd_to_commercial(jd, sg);
    (y == ry2 && w == rw && d == rd).then_some(jd)
}

/// `guess_style`: years before 1582 are Julian, after 1930 Gregorian.
fn guess_style(y: &BigInt) -> Sg {
    if *y < BigInt::from(1582) {
        Sg::Julian
    } else if *y > BigInt::from(1930) {
        Sg::Gregorian
    } else {
        Sg::Reform
    }
}

fn decode_year(y: &BigInt, sg: Sg) -> (BigInt, i64) {
    let period = if sg == Sg::Gregorian {
        CM_PERIOD_GCY
    } else {
        CM_PERIOD_JCY
    };
    let t = y + 4712;
    let p = BigInt::from(period);
    let nth = floor_div(&t, &p);
    let r = &t - &nth * &p;
    let ry = i64::try_from(&r).expect("bounded by period") - 4712;
    (nth, ry)
}

fn floor_div(a: &BigInt, b: &BigInt) -> BigInt {
    let q = a / b;
    if (a % b != BigInt::from(0))
        && ((a.sign() == num_bigint::Sign::Minus) != (b.sign() == num_bigint::Sign::Minus))
    {
        q - 1
    } else {
        q
    }
}

fn encode_jd(nth: &BigInt, rjd: i64) -> BigInt {
    nth * CM_PERIOD + rjd
}

/// Runs one of the `c_valid_*` checks the way `valid_*_p` does: directly for
/// reform-era years, otherwise on the year reduced into its cycle.
fn with_style(y: &BigInt, check: impl Fn(i64, Sg) -> Option<i64>) -> Option<BigInt> {
    match guess_style(y) {
        Sg::Reform => {
            let yi = i64::try_from(y).expect("reform-era year");
            check(yi, Sg::Reform).map(BigInt::from)
        }
        style => {
            let (nth, ry) = decode_year(y, style);
            check(ry, style).map(|rjd| encode_jd(&nth, rjd))
        }
    }
}

fn valid_civil(y: &BigInt, m: i32, d: i32) -> Option<BigInt> {
    let (m, d) = (i64::from(m), i64::from(d));
    with_style(y, |yy, sg| match sg {
        Sg::Reform => valid_civil_c(yy, m, d, sg),
        _ => {
            let (rm, rd) = valid_pure_civil(yy, m, d, sg == Sg::Gregorian)?;
            Some(civil_to_jd(yy, rm, rd, sg))
        }
    })
}

fn valid_ordinal(y: &BigInt, d: i32) -> Option<BigInt> {
    with_style(y, |yy, sg| valid_ordinal_c(yy, i64::from(d), sg))
}

fn valid_commercial(y: &BigInt, w: i32, d: i32) -> Option<BigInt> {
    with_style(y, |yy, sg| {
        valid_commercial_c(yy, i64::from(w), i64::from(d), sg)
    })
}

/// The civil date a `Date` with this Julian Day Number (and start `ITALY`)
/// reports.
fn jd_to_date(jd: &BigInt) -> CivilDate {
    let p = BigInt::from(CM_PERIOD);
    let nth = floor_div(jd, &p);
    let rjd = i64::try_from(&(jd - &nth * &p)).expect("bounded by period");
    let zero = BigInt::from(0);
    let vsg = if nth == zero {
        Sg::Reform
    } else if nth < zero {
        Sg::Julian
    } else {
        Sg::Gregorian
    };
    let (ry, m, d) = jd_to_civil(rjd, vsg);
    let gregorian = !vsg.before(rjd);
    let year = if nth == zero {
        BigInt::from(ry)
    } else {
        let period = if gregorian {
            CM_PERIOD_GCY
        } else {
            CM_PERIOD_JCY
        };
        nth * period + ry
    };
    CivilDate {
        year,
        mon: m as i32,
        mday: d as i32,
    }
}

impl Today {
    fn jd(&self) -> i64 {
        gregorian_civil_to_jd(
            i64::from(self.year),
            i64::from(self.mon),
            i64::from(self.mday),
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Key {
    Year,
    Mon,
    Mday,
    Yday,
    Wday,
    Cwyear,
    Cweek,
    Cwday,
    Hour,
    Min,
    Sec,
    /// Keys `Date._parse` never produces (`jd`, `wnum0`, `wnum1`).
    Never,
}

impl Fragments {
    fn has(&self, k: Key) -> bool {
        match k {
            Key::Year => self.year.is_some(),
            Key::Mon => self.mon.is_some(),
            Key::Mday => self.mday.is_some(),
            Key::Yday => self.yday.is_some(),
            Key::Wday => self.wday.is_some(),
            Key::Cwyear => self.cwyear.is_some(),
            Key::Cweek => self.cweek.is_some(),
            Key::Cwday => self.cwday.is_some(),
            Key::Hour => self.hour.is_some(),
            Key::Min => self.min.is_some(),
            Key::Sec => self.sec.is_some(),
            Key::Never => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Time,
    Nil,
    Ordinal,
    Civil,
    Commercial,
    Wday,
    Wnum,
}

/// `rt_complete_frags` for `klass == Date`.
fn complete(h: &mut Fragments, today: &Today) {
    use Key::*;
    let tab: [(Kind, &[Key]); 11] = [
        (Kind::Time, &[Hour, Min, Sec]),
        (Kind::Nil, &[Never]),
        (Kind::Ordinal, &[Year, Yday, Hour, Min, Sec]),
        (Kind::Civil, &[Year, Mon, Mday, Hour, Min, Sec]),
        (Kind::Commercial, &[Cwyear, Cweek, Cwday, Hour, Min, Sec]),
        (Kind::Wday, &[Wday, Hour, Min, Sec]),
        (Kind::Wnum, &[Year, Never, Wday, Hour, Min, Sec]),
        (Kind::Wnum, &[Year, Never, Wday, Hour, Min, Sec]),
        (Kind::Nil, &[Cwyear, Cweek, Wday, Hour, Min, Sec]),
        (Kind::Nil, &[Year, Never, Cwday, Hour, Min, Sec]),
        (Kind::Nil, &[Year, Never, Cwday, Hour, Min, Sec]),
    ];
    let mut best: Option<(Kind, &[Key])> = None;
    let mut eno = 0;
    for (kind, keys) in tab {
        let n = keys.iter().filter(|k| h.has(**k)).count();
        if n > eno {
            eno = n;
            best = Some((kind, keys));
        }
    }
    let Some((kind, keys)) = best else {
        return;
    };
    if keys.len() <= eno {
        return;
    }
    let jd = today.jd();
    match kind {
        Kind::Ordinal => {
            if h.year.is_none() {
                h.year = Some(today.year.into());
            }
            if h.yday.is_none() {
                h.yday = Some(1.into());
            }
        }
        Kind::Civil => {
            for k in keys {
                if h.has(*k) {
                    break;
                }
                match k {
                    Year => h.year = Some(today.year.into()),
                    Mon => h.mon = Some(today.mon.into()),
                    Mday => h.mday = Some(today.mday.into()),
                    _ => unreachable!("civil completion stops at a date key"),
                }
            }
            if h.mon.is_none() {
                h.mon = Some(1.into());
            }
            if h.mday.is_none() {
                h.mday = Some(1.into());
            }
        }
        Kind::Commercial => {
            let (cy, cw, _) = jd_to_commercial(jd, Sg::Reform);
            for k in keys {
                if h.has(*k) {
                    break;
                }
                match k {
                    Cwyear => h.cwyear = Some(cy.into()),
                    Cweek => h.cweek = Some(cw.into()),
                    _ => unreachable!("commercial completion stops at a date key"),
                }
            }
            if h.cweek.is_none() {
                h.cweek = Some(1.into());
            }
            if h.cwday.is_none() {
                h.cwday = Some(1.into());
            }
        }
        Kind::Wday => {
            let wday = modulo(jd + 1, 7);
            let target = BigInt::from(jd - wday) + h.wday.clone().expect("wday");
            h.jd = Some(target);
        }
        Kind::Time | Kind::Nil | Kind::Wnum => {}
    }
}

/// `Date.parse(str)`: `Date._parse`, completion and validation.
pub fn date_parse(input: &str, today: &Today) -> Result<CivilDate, DateFailure> {
    let mut h = date_parse_fragments(input)?;
    let jd = if h.yday.is_none() && h.year.is_some() && h.mon.is_some() && h.mday.is_some() {
        let (y, m, d) = (h.year.as_ref(), h.mon.as_ref(), h.mday.as_ref());
        let d = num2int(d.expect("mday"))?;
        let m = num2int(m.expect("mon"))?;
        valid_civil(y.expect("year"), m, d)
    } else {
        complete(&mut h, today);
        valid_frags(&h)?
    };
    jd.map(|jd| jd_to_date(&jd)).ok_or(DateFailure::Invalid)
}

/// `rt__valid_date_frags_p` without the never-produced `wnum` keys.
fn valid_frags(h: &Fragments) -> Result<Option<BigInt>, DateFailure> {
    if let Some(jd) = &h.jd {
        return Ok(Some(jd.clone()));
    }
    if let (Some(yday), Some(year)) = (&h.yday, &h.year) {
        if let Some(jd) = valid_ordinal(year, num2int(yday)?) {
            return Ok(Some(jd));
        }
    }
    if let (Some(mday), Some(mon), Some(year)) = (&h.mday, &h.mon, &h.year) {
        let d = num2int(mday)?;
        let m = num2int(mon)?;
        if let Some(jd) = valid_civil(year, m, d) {
            return Ok(Some(jd));
        }
    }
    let wday = match &h.cwday {
        Some(v) => Some(v.clone()),
        None => h.wday.as_ref().map(|w| {
            if *w == BigInt::from(0) {
                BigInt::from(7)
            } else {
                w.clone()
            }
        }),
    };
    if let (Some(wday), Some(week), Some(year)) = (wday, &h.cweek, &h.cwyear) {
        let d = num2int(&wday)?;
        let w = num2int(week)?;
        if let Some(jd) = valid_commercial(year, w, d) {
            return Ok(Some(jd));
        }
    }
    Ok(None)
}

/// `Date.new(y, m, d)` for Integer arguments.
pub fn date_new(y: &BigInt, m: &BigInt, d: &BigInt) -> Result<CivilDate, DateFailure> {
    let d = num2int(d)?;
    let m = num2int(m)?;
    valid_civil(y, m, d)
        .map(|jd| jd_to_date(&jd))
        .ok_or(DateFailure::Invalid)
}
