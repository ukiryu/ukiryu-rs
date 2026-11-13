//! Port of the parts of Ruby's `date` extension (gem 3.5.1, the default gem
//! of Ruby 4.0.1, `date_core.c`) that `Ukiryu::ManPageParser` reaches on top
//! of `Date._parse`: `Date.parse` (the 128-byte limit, fragment completion and
//! validation with the default `Date::ITALY` start), `Date.new(y, m, d)` and
//! `Date#strftime('%Y-%m-%d')`.
//!
//! `Date._parse` itself is the shared port in [`crate::types::date_parse`]
//! (also used by `Time.parse`); this module only maps its complete fragment
//! set, commercial-week keys included, into [`Fragments`].
//!
//! The `offset` fragment (`date_zone_to_diff`) is not computed: `Date`
//! results never read it. The `wnum0`/`wnum1`/`jd`/`seconds` completion
//! branches are not ported because `Date._parse` never produces those keys.

use crate::types::date_parse::{self as shared, Parsed};
use crate::types::error::ErrorClass;
use num_bigint::BigInt;
use std::fmt;

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
    /// The shared Onigmo path could not run a pattern. Never a Ruby class:
    /// it is not rescued as `ArgumentError` and means "not decided".
    Unsupported(String),
}

impl DateFailure {
    /// Whether Ruby's `rescue ArgumentError, Date::Error` catches it.
    pub fn is_argument_error(&self) -> bool {
        matches!(self, DateFailure::Argument(_) | DateFailure::Invalid)
    }
}

impl fmt::Display for DateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DateFailure::Argument(m) | DateFailure::Range(m) | DateFailure::Unsupported(m) => {
                f.write_str(m)
            }
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

impl From<Parsed> for Fragments {
    fn from(p: Parsed) -> Self {
        Fragments {
            year: p.year,
            mon: p.mon,
            mday: p.mday,
            yday: p.yday,
            wday: p.wday,
            cwyear: p.cwyear,
            cweek: p.cweek,
            cwday: p.cwday,
            hour: p.hour,
            min: p.min,
            sec: p.sec,
            sec_fraction: p.sec_fraction.map(|f| rational(f.numerator, f.digits)),
            zone: p.zone,
            jd: None,
        }
    }
}

/// `Rational(numerator, 10**digits)`, reduced.
fn rational(mut n: BigInt, digits: usize) -> (BigInt, BigInt) {
    let mut d = BigInt::from(10).pow(u32::try_from(digits).expect("at most 128 digits"));
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

/// `Date._parse(str)` with `comp = true`, without the `offset` key: the
/// 128-byte limit, then the shared [`crate::types::date_parse::parse`].
pub fn date_parse_fragments(input: &str) -> Result<Fragments, DateFailure> {
    check_limit(input)?;
    match shared::parse(input) {
        Ok(parsed) => Ok(parsed.into()),
        Err(e) if e.class == ErrorClass::Range => Err(DateFailure::Range(e.message)),
        Err(e) => Err(DateFailure::Unsupported(e.message)),
    }
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
    let vsg = match nth.cmp(&zero) {
        std::cmp::Ordering::Equal => Sg::Reform,
        std::cmp::Ordering::Less => Sg::Julian,
        std::cmp::Ordering::Greater => Sg::Gregorian,
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
