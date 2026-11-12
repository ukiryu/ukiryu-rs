//! `Time.parse(value.to_s)` for `Ukiryu::Type` `:datetime` values.
//!
//! Ruby's `Time.parse` runs `Date._parse`, a large set of lenient heuristics
//! that also reads the current date and the process time zone. This port
//! reproduces an explicit subset whose results are pinned by oracle rows:
//!
//! * ISO 8601 extended dates `YYYY-M-D`, optionally followed by `T`, `t` or a
//!   space and `h:mm[:ss[(.|,)fraction]]`, then an optional zone;
//! * ISO 8601 basic `YYYYMMDDThhmmss` with an explicit zone;
//! * zones `Z`, `z`, `UTC` (a UTC time) and `±hh:mm` / `±hhmm` (fixed offset);
//!   without a zone the time is local and its offset comes from the [`Host`];
//! * `Date._parse`'s 128-byte input limit;
//! * input without ASCII digits: it yields a date only through a month name
//!   (`/\b(jan|feb|…|dec)\S*/i` after `Date._parse`'s punctuation folding);
//!   without one, Ruby raises `no time information`. A month name alone needs
//!   the current year, which this port does not read (`Unsupported`).
//!
//! Everything else, including Ruby's out-of-range rollover (`24:00`,
//! second 60, `02-30`) and fractions beyond nanoseconds, returns an
//! `Unsupported` error rather than a guessed result.
use super::error::{Error, ErrorClass, Result};
use super::ruby::{days_from_civil, string_inspect};
use super::validate::Host;
use super::value::Time;

struct Cursor<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn digits(&mut self, min: usize, max: usize) -> Option<u32> {
        let n = self.s[self.pos..]
            .iter()
            .take(max)
            .take_while(|b| b.is_ascii_digit())
            .count();
        if n < min {
            return None;
        }
        let v = std::str::from_utf8(&self.s[self.pos..self.pos + n])
            .ok()?
            .parse()
            .ok()?;
        self.pos += n;
        Some(v)
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.s.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn rest(&self) -> &[u8] {
        &self.s[self.pos..]
    }
}

enum Zone {
    Utc,
    Offset(i32),
    Local,
}

struct Parsed {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    nanos: u32,
    zone: Zone,
}

fn unsupported(input: &str) -> Error {
    Error::unsupported(format!(
        "Time.parse({}) is outside the ported datetime subset",
        string_inspect(input)
    ))
}

fn parse_zone(c: &mut Cursor<'_>) -> Option<Zone> {
    while c.eat(b' ') {}
    let rest = c.rest();
    if rest.is_empty() {
        return Some(Zone::Local);
    }
    if rest == b"Z" || rest == b"z" || rest == b"UTC" {
        c.pos = c.s.len();
        return Some(Zone::Utc);
    }
    let sign = match c.peek()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    c.pos += 1;
    let h = c.digits(2, 2)?;
    c.eat(b':');
    let m = c.digits(2, 2)?;
    if m > 59 {
        return None;
    }
    if c.pos != c.s.len() {
        return None;
    }
    Some(Zone::Offset(sign * (h as i32 * 3600 + m as i32 * 60)))
}

fn parse_fraction(c: &mut Cursor<'_>) -> Option<u32> {
    if !(c.eat(b'.') || c.eat(b',')) {
        return Some(0);
    }
    let n = c.rest().iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 || n > 9 {
        return None;
    }
    let padded = format!("{:0<9}", std::str::from_utf8(&c.rest()[..n]).ok()?);
    c.pos += n;
    padded.parse().ok()
}

fn parse_subset(s: &[u8]) -> Option<Parsed> {
    let mut c = Cursor { s, pos: 0 };
    let basic = s.len() >= 15 && s[..8].iter().all(u8::is_ascii_digit) && s[8] == b'T';
    if basic {
        let year = c.digits(4, 4)? as i64;
        let month = c.digits(2, 2)?;
        let day = c.digits(2, 2)?;
        c.eat(b'T');
        let hour = c.digits(2, 2)?;
        let minute = c.digits(2, 2)?;
        let second = c.digits(2, 2)?;
        let zone = parse_zone(&mut c)?;
        if matches!(zone, Zone::Local) {
            return None;
        }
        return Some(Parsed {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanos: 0,
            zone,
        });
    }
    let year = c.digits(4, 4)? as i64;
    if !c.eat(b'-') {
        return None;
    }
    let month = c.digits(1, 2)?;
    if !c.eat(b'-') {
        return None;
    }
    let day = c.digits(1, 2)?;
    let (mut hour, mut minute, mut second, mut nanos) = (0, 0, 0, 0);
    if matches!(c.peek(), Some(b'T' | b't' | b' '))
        && c.s.get(c.pos + 1).is_some_and(u8::is_ascii_digit)
    {
        c.pos += 1;
        hour = c.digits(1, 2)?;
        if !c.eat(b':') {
            return None;
        }
        minute = c.digits(1, 2)?;
        if c.eat(b':') {
            second = c.digits(1, 2)?;
            nanos = parse_fraction(&mut c)?;
        }
    }
    let zone = parse_zone(&mut c)?;
    Some(Parsed {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanos,
        zone,
    })
}

/// `Date._parse` folds runs of other characters to a space
/// (`/[^-+',.\/:@[:alnum:]\[\]]+/`) and then looks for `\b(jan|…|dec)`.
fn has_month_name(input: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let kept = |c: char| c.is_alphanumeric() || "-+',./:@[]".contains(c);
    let chars: Vec<char> = input.chars().map(|c| if kept(c) { c } else { ' ' }).collect();
    (0..chars.len()).any(|i| {
        let boundary = i == 0 || !chars[i - 1].is_alphanumeric();
        boundary
            && chars.len() >= i + 3
            && MONTHS.iter().any(|m| {
                chars[i..i + 3]
                    .iter()
                    .zip(m.chars())
                    .all(|(c, mc)| c.to_ascii_lowercase() == mc)
            })
    })
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// `Time.parse(input)` for the ported subset.
pub fn parse(input: &str, host: &dyn Host) -> Result<Time> {
    if input.len() > 128 {
        return Err(Error::new(
            ErrorClass::Argument,
            format!("string length ({}) exceeds the limit 128", input.len()),
        ));
    }
    if !input.bytes().any(|b| b.is_ascii_digit()) {
        if input.contains('\u{17F}') {
            // U+017F case-folds to `s` under Onigmo's /i (`ſep`).
            return Err(unsupported(input));
        }
        if has_month_name(input) {
            return Err(unsupported(input));
        }
        return Err(Error::new(
            ErrorClass::Argument,
            format!("no time information in {}", string_inspect(input)),
        ));
    }
    let trimmed = input.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r'));
    let p = parse_subset(trimmed.as_bytes()).ok_or_else(|| unsupported(input))?;
    let in_range = (1..=12).contains(&p.month)
        && p.day >= 1
        && p.day <= days_in_month(p.year, p.month)
        && p.hour <= 23
        && p.minute <= 59
        && p.second <= 59;
    if !in_range {
        return Err(unsupported(input));
    }
    let (offset, utc) = match p.zone {
        Zone::Utc => (0, true),
        Zone::Offset(o) if o.abs() < 86_400 => (o, false),
        Zone::Offset(_) => return Err(unsupported(input)),
        Zone::Local => (
            host.local_offset(p.year, p.month, p.day, p.hour, p.minute, p.second)
                .ok_or_else(|| {
                    Error::unsupported(format!(
                        "local time zone offset unavailable for Time.parse({})",
                        string_inspect(input)
                    ))
                })?,
            false,
        ),
    };
    let local = days_from_civil(p.year, p.month, p.day) * 86_400
        + (p.hour * 3600 + p.minute * 60 + p.second) as i64;
    Ok(Time {
        seconds: local - offset as i64,
        nanos: p.nanos,
        utc_offset: offset,
        utc,
    })
}
