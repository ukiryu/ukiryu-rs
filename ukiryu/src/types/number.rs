//! `Kernel#Integer` and `Kernel#Float` string conversion (strict mode) as the
//! pinned Ruby performs them for `Ukiryu::Type` integer/float validation.
use std::str::FromStr;

use num_bigint::BigInt;

use super::error::{Error, Result};

/// A digit run with Ruby's underscore rule: digits of `radix`, single `_`
/// only between two digits. Returns the digits without underscores.
fn digit_run(s: &str, radix: u32) -> Option<String> {
    if s.is_empty() || s.starts_with('_') || s.ends_with('_') || s.contains("__") {
        return None;
    }
    let digits: String = s.chars().filter(|c| *c != '_').collect();
    if digits.chars().all(|c| c.is_digit(radix)) {
        Some(digits)
    } else {
        None
    }
}

/// `Integer(str)` with base auto-detection (`0x`, `0b`, `0o`, `0d`, leading
/// `0` octal). The input is already stripped. `None` is Ruby's ArgumentError.
pub fn parse_integer(s: &str) -> Option<BigInt> {
    let (negative, body) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let lower = body.get(..2).map(str::to_ascii_lowercase);
    let (radix, digits) = match lower.as_deref() {
        Some("0x") => (16, digit_run(&body[2..], 16)?),
        Some("0b") => (2, digit_run(&body[2..], 2)?),
        Some("0o") => (8, digit_run(&body[2..], 8)?),
        Some("0d") => (10, digit_run(&body[2..], 10)?),
        _ if body.starts_with('0') => (8, digit_run(body, 8)?),
        _ => (10, digit_run(body, 10)?),
    };
    let magnitude = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// `Float(str)`. The input is already stripped. `Ok(None)` is Ruby's
/// ArgumentError; `Err` marks a hexadecimal form this port does not evaluate.
pub fn parse_float(s: &str) -> Result<Option<f64>> {
    let (negative, body) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let value = if body.len() >= 2 && body.as_bytes()[..2].eq_ignore_ascii_case(b"0x") {
        match parse_hex_float(&body[2..])? {
            Some(v) => v,
            None => return Ok(None),
        }
    } else {
        match parse_decimal_float(body) {
            Some(v) => v,
            None => return Ok(None),
        }
    };
    Ok(Some(if negative { -value } else { value }))
}

fn parse_decimal_float(body: &str) -> Option<f64> {
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], Some(&mantissa[i + 1..])),
        None => (mantissa, None),
    };
    let int_digits = if int_part.is_empty() {
        String::new()
    } else {
        digit_run(int_part, 10)?
    };
    let frac_digits = match frac_part {
        None => None,
        Some("") => Some(String::new()),
        Some(f) => Some(digit_run(f, 10)?),
    };
    if int_digits.is_empty() && frac_digits.as_deref().is_none_or(str::is_empty) {
        return None;
    }
    let mut text = int_digits;
    if let Some(f) = frac_digits {
        text.push('.');
        text.push_str(&f);
    }
    if let Some(e) = exponent {
        let (sign, digits) = match e.as_bytes().first() {
            Some(b'+') => ("", &e[1..]),
            Some(b'-') => ("-", &e[1..]),
            _ => ("", e),
        };
        text.push('e');
        text.push_str(sign);
        text.push_str(&digit_run(digits, 10)?);
    }
    if text.starts_with('.') {
        text.insert(0, '0');
    }
    f64::from_str(&text).ok()
}

fn parse_hex_float(body: &str) -> Result<Option<f64>> {
    if body.contains('_') {
        return Err(Error::unsupported(
            "hexadecimal Float with underscores is not ported",
        ));
    }
    let (mantissa, exponent) = match body.find(['p', 'P']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
        None => (mantissa, ""),
    };
    let all = format!("{int_part}{frac_part}");
    if all.is_empty() || !all.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let mut exp: i64 = match exponent {
        None => 0,
        Some(e) => {
            let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return Ok(None);
            }
            match e.parse() {
                Ok(v) => v,
                Err(_) => return Err(Error::unsupported("hexadecimal Float exponent overflow")),
            }
        }
    };
    exp -= 4 * frac_part.len() as i64;
    let trimmed = all.trim_start_matches('0');
    if trimmed.len() > 13 && u64::from_str_radix(trimmed, 16).map_or(true, |m| m >= 1 << 53) {
        return Err(Error::unsupported(
            "hexadecimal Float mantissa wider than 53 bits is not ported",
        ));
    }
    let m = if trimmed.is_empty() {
        0
    } else {
        u64::from_str_radix(trimmed, 16).expect("validated hex digits")
    };
    if m == 0 {
        return Ok(Some(0.0));
    }
    let m = m as f64; // exact: below 2^53
    let value = if exp >= -1022 {
        if exp > 1100 {
            f64::INFINITY
        } else {
            m * 2f64.powi(exp as i32)
        }
    } else if exp < -1074 - 64 {
        0.0
    } else {
        // Scale exactly first, then round once at the subnormal boundary.
        (m * 2f64.powi((exp + 1074) as i32)) * f64::from_bits(1)
    };
    Ok(Some(value))
}
