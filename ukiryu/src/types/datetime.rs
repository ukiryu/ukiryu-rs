//! `Time.parse(value.to_s)` for `Ukiryu::Type` `:datetime` values.
//!
//! Port of `Time.parse` / `Time.make_time` / `Time.zone_offset` /
//! `Time.apply_offset` / `force_zone!` from `lib/time.rb` of Ruby 4.0.1,
//! over [`super::date_parse`] (`Date._parse`) and the argument checks of
//! `Time.utc` / `Time.local` (`time_arg`, `validate_vtm` in Ruby 4.0.1
//! `time.c`), including their out-of-range rollover (`24:00`, second 60,
//! `02-30`) and messages.
//!
//! Ruby reads three facts from the process: the current time (`Time.now`,
//! which fills the fields a text leaves out), the local UTC offset and the
//! local zone abbreviation (`Time.local`, and `zone_offset` for zone names
//! outside its table). They come from the [`Host`]; the current time is the
//! injected [`crate::runtime::Clock`] (see [`Now`]). When a needed fact is
//! missing the result is `Unsupported`, never a guess. Calendar fields
//! beyond what `Time` can hold here (years beyond ±10^11, fields beyond
//! `i64`) are `Unsupported` too. A fraction finer than a nanosecond is kept
//! exactly, as Ruby keeps its `Rational` (see [`Time`]).
use std::time::{SystemTime, UNIX_EPOCH};

use num_bigint::BigInt;

use super::date_parse::{self, Fraction, Parsed};
use super::error::{Error, ErrorClass, Result};
use super::ruby::{civil_from_days, days_from_civil, string_inspect};
use super::validate::Host;
use super::value::Time;
use crate::extractors::man::Today;
use crate::runtime::Clock;

/// Ruby's `Time.now` as `Time.parse` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Now {
    /// Only the local calendar day and the local UTC offset are known
    /// (`Clock::Fixed`). Inputs that need the time of day, or the day in
    /// another zone, are `Unsupported`.
    Day { today: Today, utc_offset: i32 },
    /// The exact instant, in the local zone (`utc_offset`).
    Instant(Time),
}

impl Now {
    /// `Time.now` from the runtime clock. `Clock::System` reads the system
    /// clock in the zone of its own offset; `Clock::Fixed` gives a day in a
    /// zone `utc_offset` seconds east of UTC.
    pub fn from_clock(clock: Clock, utc_offset: i32) -> Now {
        match clock {
            Clock::Fixed(today) => Now::Day { today, utc_offset },
            Clock::System { utc_offset_secs } => {
                let d = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                Now::Instant(Time::new(
                    d.as_secs() as i64,
                    d.subsec_nanos(),
                    utc_offset_secs,
                    false,
                ))
            }
        }
    }
}

fn need(what: &str, input: &str) -> Error {
    Error::unsupported(format!(
        "Time.parse({}) needs {what}",
        string_inspect(input)
    ))
}

fn argument(message: impl Into<String>) -> Error {
    Error::new(ErrorClass::Argument, message)
}

/// The local calendar fields of `now` in a zone; `None` when unknown.
struct NowFields {
    date: Option<(i64, i64, i64)>,
    /// hour, min, sec, usec.
    time: Option<(i64, i64, i64, i64)>,
}

fn now_fields(now: Option<&Now>, off: Option<i64>) -> NowFields {
    match now {
        None => NowFields {
            date: None,
            time: None,
        },
        Some(&Now::Day { today, utc_offset }) => {
            let same = off.is_none_or(|o| o == i64::from(utc_offset));
            NowFields {
                date: same.then_some((
                    i64::from(today.year),
                    i64::from(today.mon),
                    i64::from(today.mday),
                )),
                time: None,
            }
        }
        Some(Now::Instant(t)) => {
            let o = off.unwrap_or(i64::from(t.utc_offset));
            let local = t.seconds + o;
            let (y, m, d) = civil_from_days(local.div_euclid(86_400));
            let sod = local.rem_euclid(86_400);
            NowFields {
                date: Some((y, i64::from(m), i64::from(d))),
                time: Some((
                    sod / 3600,
                    sod % 3600 / 60,
                    sod % 60,
                    i64::from(t.nanos / 1000),
                )),
            }
        }
    }
}

fn now_year(now: Option<&Now>) -> Option<i64> {
    now_fields(now, None).date.map(|d| d.0)
}

/// `Time::ZoneOffset` (hours).
fn zone_table(zone: &str) -> Option<i64> {
    Some(match zone {
        "UTC" | "Z" | "UT" | "GMT" => 0,
        "EST" => -5,
        "EDT" => -4,
        "CST" => -6,
        "CDT" => -5,
        "MST" => -7,
        "MDT" => -6,
        "PST" => -8,
        "PDT" => -7,
        "A" => 1,
        "B" => 2,
        "C" => 3,
        "D" => 4,
        "E" => 5,
        "F" => 6,
        "G" => 7,
        "H" => 8,
        "I" => 9,
        "K" => 10,
        "L" => 11,
        "M" => 12,
        "N" => -1,
        "O" => -2,
        "P" => -3,
        "Q" => -4,
        "R" => -5,
        "S" => -6,
        "T" => -7,
        "U" => -8,
        "V" => -9,
        "W" => -10,
        "X" => -11,
        "Y" => -12,
        _ => return None,
    })
}

fn two_digits(b: &[u8]) -> Option<i64> {
    (b.len() == 2 && b.iter().all(u8::is_ascii_digit))
        .then(|| i64::from(b[0] - b'0') * 10 + i64::from(b[1] - b'0'))
}

/// `Time.zone_offset(zone, year)`.
fn zone_offset(zone: &str, year: i64, host: &dyn Host, input: &str) -> Result<Option<i64>> {
    let z = zone.to_uppercase();
    let b = z.as_bytes();
    if b.len() >= 5 && matches!(b[0], b'+' | b'-') {
        // /\A([+-])(\d\d)(:?)(\d\d)(?:\3(\d\d))?\z/
        let sign = if b[0] == b'-' { -1 } else { 1 };
        let colon = b[3] == b':';
        let rest = &b[if colon { 4 } else { 3 }..];
        if let (Some(h), Some(m)) = (two_digits(&b[1..3]), rest.get(..2).and_then(two_digits)) {
            let tail = &rest[2..];
            let sec = if tail.is_empty() {
                Some(0)
            } else if colon && tail.len() == 3 && tail[0] == b':' {
                two_digits(&tail[1..])
            } else if !colon && tail.len() == 2 {
                two_digits(tail)
            } else {
                None
            };
            if let Some(s) = sec {
                return Ok(Some(sign * ((h * 60 + m) * 60 + s)));
            }
        }
    }
    if b.len() == 3 && matches!(b[0], b'+' | b'-') {
        if let Some(h) = two_digits(&b[1..]) {
            let h = if b[0] == b'-' { -h } else { h };
            return Ok(Some(h * 3600));
        }
    }
    if let Some(h) = zone_table(&z) {
        return Ok(Some(h * 3600));
    }
    for month in [1, 7] {
        let name = host
            .local_zone_abbreviation(year, month, 1)
            .ok_or_else(|| need("the local time zone name", input))?;
        if name.to_uppercase() == z {
            let off = host
                .local_offset(year, month, 1, 0, 0, 0)
                .ok_or_else(|| need("the local time zone offset", input))?;
            return Ok(Some(i64::from(off)));
        }
    }
    Ok(None)
}

/// `zone_utc?`.
fn zone_utc(zone: &str) -> bool {
    ["-00:00", "-0000", "-00", "UTC", "Z", "UT"]
        .iter()
        .any(|z| zone.eq_ignore_ascii_case(z))
}

fn leap(y: i64) -> bool {
    (y.rem_euclid(4) == 0 && y.rem_euclid(100) != 0) || y.rem_euclid(400) == 0
}

/// `month_days(y, m)` with Ruby's Array indexing (`m - 1` may be negative).
fn month_days(y: i64, m: i64) -> Option<i64> {
    const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let i = m - 1;
    let i = if i < 0 { i + 12 } else { i };
    if !(0..12).contains(&i) {
        return None;
    }
    Some(if i == 1 && leap(y) {
        29
    } else {
        DAYS[i as usize]
    })
}

/// `apply_offset`: shifts a local civil time to UTC; `day` may become nil.
// The if-chain mirrors time.rb `apply_offset`.
#[allow(clippy::type_complexity, clippy::comparison_chain)]
fn apply_offset(
    mut year: i64,
    mut mon: i64,
    day: i64,
    mut hour: i64,
    mut min: i64,
    mut sec: i64,
    off: i64,
) -> (i64, i64, Option<i64>, i64, i64, i64) {
    let mut day = Some(day);
    if off < 0 {
        let mut off = -off;
        let o = off.rem_euclid(60);
        off = off.div_euclid(60);
        if o != 0 {
            sec += o;
            off += sec.div_euclid(60);
            sec = sec.rem_euclid(60);
        }
        let o = off.rem_euclid(60);
        off = off.div_euclid(60);
        if o != 0 {
            min += o;
            off += min.div_euclid(60);
            min = min.rem_euclid(60);
        }
        let o = off.rem_euclid(24);
        off = off.div_euclid(24);
        if o != 0 {
            hour += o;
            off += hour.div_euclid(24);
            hour = hour.rem_euclid(24);
        }
        if off != 0 {
            let d = day.expect("day") + off;
            day = Some(d);
            if let Some(days) = month_days(year, mon) {
                if days < d {
                    mon += 1;
                    if 12 < mon {
                        mon = 1;
                        year += 1;
                    }
                    day = Some(1);
                }
            }
        }
    } else if 0 < off {
        let mut off = off;
        let o = off.rem_euclid(60);
        off = off.div_euclid(60);
        if o != 0 {
            sec -= o;
            off -= sec.div_euclid(60);
            sec = sec.rem_euclid(60);
        }
        let o = off.rem_euclid(60);
        off = off.div_euclid(60);
        if o != 0 {
            min -= o;
            off -= min.div_euclid(60);
            min = min.rem_euclid(60);
        }
        let o = off.rem_euclid(24);
        off = off.div_euclid(24);
        if o != 0 {
            hour -= o;
            off -= hour.div_euclid(24);
            hour = hour.rem_euclid(24);
        }
        if off != 0 {
            let d = day.expect("day") - off;
            day = Some(d);
            if d < 1 {
                mon -= 1;
                if mon < 1 {
                    year -= 1;
                    mon = 12;
                }
                day = month_days(year, mon);
            }
        }
    }
    (year, mon, day, hour, min, sec)
}

/// `NUM2INT` then `obj2ubits(v, bits)`.
fn ubits(v: i64, bits: u32) -> Result<i64> {
    if v > i64::from(i32::MAX) {
        return Err(Error::new(
            ErrorClass::Range,
            format!("integer {v} too big to convert to 'int'"),
        ));
    }
    if v < i64::from(i32::MIN) {
        return Err(Error::new(
            ErrorClass::Range,
            format!("integer {v} too small to convert to 'int'"),
        ));
    }
    let rv = v as i32 as u32;
    let mask = (1u32 << bits) - 1;
    if rv & mask != rv {
        return Err(argument("argument out of range"));
    }
    Ok(i64::from(rv))
}

/// Seconds since the epoch of a civil time after `time_arg` checks
/// (`Time.utc(year, mon, day, hour, min, sec, usec)` without the zone).
fn time_arg(year: i64, mon: i64, day: Option<i64>, hour: i64, min: i64, sec: i64) -> Result<i64> {
    let mut mon = ubits(mon, 4)?;
    let mut mday = match day {
        None => 1,
        Some(d) => ubits(d, 5)?,
    };
    match mon {
        2 => {
            let mday2 = if leap(year) { 29 } else { 28 };
            if mday > mday2 {
                mday -= mday2;
                mon += 1;
            }
        }
        4 | 6 | 9 | 11 if mday == 31 => {
            mon += 1;
            mday = 1;
        }
        _ => {}
    }
    let hour = ubits(hour, 5)?;
    let min = ubits(min, 6)?;
    let sec = ubits(sec, 6)?;
    let check = |v: i64, lo: i64, hi: i64, name: &str| {
        if v < lo || v > hi {
            Err(argument(format!("{name} out of range")))
        } else {
            Ok(())
        }
    };
    check(mon, 1, 12, "mon")?;
    check(mday, 1, 31, "mday")?;
    check(hour, 0, 24, "hour")?;
    check(min, 0, if hour == 24 { 0 } else { 59 }, "min")?;
    check(sec, 0, if hour == 24 { 0 } else { 60 }, "sec")?;
    Ok(days_from_civil(year, mon as u32, mday as u32) * 86_400 + hour * 3600 + min * 60 + sec)
}

/// `Time.utc`/`Time.local` with `usec = sec_fraction * 1000000`: the
/// whole `seconds` plus the exact fraction (`Rational(numerator,
/// 10**digits)`), or whole `nanos` when the fraction came from `Time.now`.
fn exact_time(seconds: i64, sub: &SubSecond, utc_offset: i32, utc: bool) -> Time {
    match sub {
        SubSecond::Nanos(n) => Time::new(seconds, *n, utc_offset, utc),
        SubSecond::Fraction(f) => {
            let denom = BigInt::from(10u32).pow(f.digits as u32);
            let numer = BigInt::from(seconds) * &denom + &f.numerator;
            Time::from_rational(&numer, &denom, utc_offset, utc)
                .expect("a fraction below one second keeps the whole seconds")
        }
    }
}

/// Where the sub-second part of a parsed time comes from.
enum SubSecond<'a> {
    Nanos(u32),
    Fraction(&'a Fraction),
}

fn small(v: &Option<BigInt>, input: &str) -> Result<Option<i64>> {
    match v {
        None => Ok(None),
        Some(b) => i64::try_from(b)
            .ok()
            .filter(|v| v.unsigned_abs() < 1 << 40)
            .map(Some)
            .ok_or_else(|| need("a calendar field within ±2^40", input)),
    }
}

#[derive(Clone)]
struct Fields {
    year: Option<i64>,
    yday: Option<i64>,
    mon: Option<i64>,
    day: Option<i64>,
    hour: Option<i64>,
    min: Option<i64>,
    sec: Option<i64>,
}

fn make_time(
    input: &str,
    f: Fields,
    frac: Option<&Fraction>,
    zone: Option<&str>,
    now: Option<&Now>,
    host: &dyn Host,
) -> Result<Time> {
    let Fields {
        mut year,
        yday,
        mut mon,
        mut day,
        mut hour,
        mut min,
        mut sec,
    } = f.clone();
    if year.is_none()
        && yday.is_none()
        && mon.is_none()
        && day.is_none()
        && hour.is_none()
        && min.is_none()
        && sec.is_none()
        && frac.is_none()
    {
        return Err(argument(format!(
            "no time information in {}",
            string_inspect(input)
        )));
    }

    let off_year = year.or_else(|| now_year(now));
    let mut off = None;
    if let Some(zone) = zone {
        let y = off_year.ok_or_else(|| need("the current date", input))?;
        off = zone_offset(zone, y, host, input)?;
    }

    if let Some(yd) = yday {
        if !(1..=366).contains(&yd) {
            return Err(argument(format!("yday {yd} out of range")));
        }
        let mut m = (yd - 1).div_euclid(31) + 1;
        let mut d = (yd - 1).rem_euclid(31) + 1;
        let sub = |m: i64, d: i64| Fields {
            yday: None,
            mon: Some(m),
            day: Some(d),
            ..f.clone()
        };
        let t = make_time(input, sub(m, d), frac, zone, now, host)?;
        let local = t.seconds + i64::from(t.utc_offset);
        let (ty, _, _) = civil_from_days(local.div_euclid(86_400));
        let t_yday = local.div_euclid(86_400) - days_from_civil(ty, 1, 1) + 1;
        let diff = yd - t_yday;
        if diff == 0 {
            return Ok(t);
        }
        d += diff;
        let oy = off_year.ok_or_else(|| need("the current date", input))?;
        if d > 28 {
            if let Some(md) = month_days(oy, m).filter(|&md| d > md) {
                m += 1;
                if m > 12 {
                    return Err(argument(format!("yday {yd} out of range")));
                }
                d -= md;
            }
        }
        return make_time(input, sub(m, d), frac, zone, now, host);
    }

    let nf = now_fields(now, off);
    let mut usec_nanos: Option<u32> = None;
    let mut need_fraction = frac;
    'defaults: {
        if year.is_some() {
            break 'defaults;
        }
        let date = nf.date.ok_or_else(|| need("the current date", input))?;
        year = Some(date.0);
        if mon.is_some() {
            break 'defaults;
        }
        mon = Some(date.1);
        if day.is_some() {
            break 'defaults;
        }
        day = Some(date.2);
        if hour.is_some() {
            break 'defaults;
        }
        let tod = nf
            .time
            .ok_or_else(|| need("the current time of day", input))?;
        hour = Some(tod.0);
        if min.is_some() {
            break 'defaults;
        }
        min = Some(tod.1);
        if sec.is_some() {
            break 'defaults;
        }
        sec = Some(tod.2);
        if frac.is_some() {
            break 'defaults;
        }
        usec_nanos = Some((tod.3 * 1000) as u32);
        need_fraction = None;
    }

    let year = year.unwrap_or(1970);
    let mon = mon.unwrap_or(1);
    let day = day.unwrap_or(1);
    let hour = hour.unwrap_or(0);
    let min = min.unwrap_or(0);
    let sec = sec.unwrap_or(0);
    if year.unsigned_abs() > 100_000_000_000 {
        return Err(need("a year within ±10^11", input));
    }

    if Some(year) != off_year {
        off = match zone {
            Some(zone) => zone_offset(zone, year, host, input)?,
            None => None,
        };
    }

    let subsec = match (usec_nanos, need_fraction) {
        (Some(n), _) => SubSecond::Nanos(n),
        (None, Some(f)) => SubSecond::Fraction(f),
        (None, None) => SubSecond::Nanos(0),
    };

    if let Some(off) = off {
        let (y, mo, d, h, mi, s) = apply_offset(year, mon, day, hour, min, sec, off);
        let seconds = time_arg(y, mo, d, h, mi, s)?;
        if zone.is_some_and(zone_utc) {
            return Ok(exact_time(seconds, &subsec, 0, true));
        }
        if off <= -86_400 || off >= 86_400 {
            return Err(argument("utc_offset out of range"));
        }
        return Ok(exact_time(seconds, &subsec, off as i32, false));
    }

    let local = time_arg(year, mon, Some(day), hour, min, sec)?;
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    let sod = local.rem_euclid(86_400);
    let offset = host
        .local_offset(
            y,
            m,
            d,
            (sod / 3600) as u32,
            (sod % 3600 / 60) as u32,
            (sod % 60) as u32,
        )
        .ok_or_else(|| need("the local time zone offset", input))?;
    Ok(exact_time(
        local - i64::from(offset),
        &subsec,
        offset,
        false,
    ))
}

/// `Time.parse(input)` with `Time.now` from the host.
pub fn parse(input: &str, host: &dyn Host) -> Result<Time> {
    if input.len() > 128 {
        return Err(argument(format!(
            "string length ({}) exceeds the limit 128",
            input.len()
        )));
    }
    let Parsed {
        year,
        yday,
        mon,
        mday,
        hour,
        min,
        sec,
        sec_fraction,
        zone,
        ..
    } = date_parse::parse(input)?;
    let fields = Fields {
        year: small(&year, input)?,
        yday: small(&yday, input)?,
        mon: small(&mon, input)?,
        day: small(&mday, input)?,
        hour: small(&hour, input)?,
        min: small(&min, input)?,
        sec: small(&sec, input)?,
    };
    make_time(
        input,
        fields,
        sec_fraction.as_ref(),
        zone.as_deref(),
        host.now().as_ref(),
        host,
    )
}
