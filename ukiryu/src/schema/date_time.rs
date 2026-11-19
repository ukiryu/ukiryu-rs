//! Acceptance of Ruby 4.0.1 `DateTime.rfc3339` (date 3.6.0), used by
//! json-schema 6.2.1. This deliberately preserves Ruby's calendar switch,
//! leap seconds, 24:00, permissive numeric offsets and 128-byte limit.

use regex::Regex;
use std::sync::OnceLock;

pub(super) fn valid(text: &str) -> bool {
    // DateTime.rfc3339 raises ArgumentError before parsing longer strings.
    if text.len() > 128 {
        return false;
    }
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(concat!(
            r"\A[ \t\r\n\x0B\x0C]*(-?[0-9]{4})-([0-9]{2})-([0-9]{2})",
            r"[tT \t\r\n\x0B\x0C]([0-9]{2}):([0-9]{2}):([0-9]{2})",
            r"(?:\.[0-9]+)?(?:[zZ]|[-+][0-9]{2}:[0-9]{2})[ \t\r\n\x0B\x0C]*\z"
        ))
        .expect("fixed Ruby RFC3339 pattern")
    });
    let Some(parts) = pattern.captures(text) else {
        return false;
    };
    let number = |i: usize| parts[i].parse::<i32>().expect("matched decimal field");
    let (year, month, day) = (number(1), number(2), number(3));
    let (hour, minute, second) = (number(4), number(5), number(6));
    // DateTime's default is Date::ITALY: Julian before 1582-10-15,
    // with 1582-10-05 through 1582-10-14 omitted.
    if year == 1582 && month == 10 && (5..=14).contains(&day) {
        return false;
    }
    let julian = (year, month, day) < (1582, 10, 15);
    let leap = year % 4 == 0 && (julian || year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    (1..=days).contains(&day)
        && minute < 60
        && second <= 60
        && (hour < 24 || (hour == 24 && minute == 0 && second == 0))
}
