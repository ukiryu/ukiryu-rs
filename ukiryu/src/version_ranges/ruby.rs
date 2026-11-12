//! Ruby core behaviours the Versionian port depends on: `String#strip`,
//! `String#inspect`, `Float#to_s`, `Kernel#Integer` and `Kernel#Float` on
//! strings, and the receiver wording of `NoMethodError` messages.
use super::{RangeError, SpecValue};
use num_bigint::BigInt;

/// Characters removed by `String#strip` (NUL, \t, \n, \v, \f, \r, space).
fn is_strip_ws(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ')
}

/// `String#strip`. Ruby removes leading whitespace and trailing whitespace or NUL.
pub(crate) fn strip(s: &str) -> &str {
    s.trim_start_matches(|c: char| c != '\0' && is_strip_ws(c))
        .trim_end_matches(is_strip_ws)
}

/// Regexp `\s` in Ruby: space, \t, \n, \v, \f, \r.
pub(crate) fn is_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r')
}

/// `String#inspect` for a UTF-8 string.
pub(crate) fn inspect_str(s: &str) -> String {
    let mut out = String::from("\"");
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{0c}' => out.push_str("\\f"),
            '\u{0b}' => out.push_str("\\v"),
            '\u{08}' => out.push_str("\\b"),
            '\u{07}' => out.push_str("\\a"),
            '\u{1b}' => out.push_str("\\e"),
            '#' if matches!(chars.get(i + 1), Some('{') | Some('$') | Some('@')) => {
                out.push_str("\\#")
            }
            c if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `Float#to_s`.
pub(crate) fn float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sci = format!("{:e}", f.abs());
    let (mant, exp) = sci.split_once('e').expect("scientific format");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let decpt = exp + 1;
    let sign = if f < 0.0 { "-" } else { "" };
    if !(-3..=16).contains(&decpt) {
        let rest = if digits.len() > 1 { &digits[1..] } else { "0" };
        let e = decpt - 1;
        let es = if e < 0 { "-" } else { "+" };
        format!("{sign}{}.{rest}e{es}{:02}", &digits[..1], e.abs())
    } else if decpt <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if (decpt as usize) >= digits.len() {
        format!(
            "{sign}{digits}{}.0",
            "0".repeat(decpt as usize - digits.len())
        )
    } else {
        let (a, b) = digits.split_at(decpt as usize);
        format!("{sign}{a}.{b}")
    }
}

/// The receiver part of a Ruby 3.4+ `NoMethodError` message.
pub(crate) fn receiver(v: &SpecValue) -> String {
    match v {
        SpecValue::Nil => "nil".into(),
        SpecValue::Bool(true) => "true".into(),
        SpecValue::Bool(false) => "false".into(),
        other => format!("an instance of {}", other.ruby_class()),
    }
}

pub(crate) fn no_method(method: &str, recv: &SpecValue) -> RangeError {
    RangeError::no_method(format!(
        "undefined method '{method}' for {}",
        receiver(recv)
    ))
}

/// `obj.to_sym` for String/Symbol; anything else has no `to_sym`.
pub(crate) fn to_sym(v: &SpecValue) -> Result<String, RangeError> {
    match v {
        SpecValue::Str(s) | SpecValue::Sym(s) => Ok(s.clone()),
        other => Err(no_method("to_sym", other)),
    }
}

/// `x&.to_sym`.
pub(crate) fn opt_to_sym(v: Option<&SpecValue>) -> Result<Option<String>, RangeError> {
    match v {
        None | Some(SpecValue::Nil) => Ok(None),
        Some(v) => to_sym(v).map(Some),
    }
}

/// `Kernel#Integer(string)`; the error is the ArgumentError message.
pub(crate) fn kernel_integer(s: &str) -> Result<BigInt, String> {
    let bad = || format!("invalid value for Integer(): {}", inspect_str(s));
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() && is_regex_space(b[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == '+' || b[i] == '-') {
        neg = b[i] == '-';
        i += 1;
    }
    let mut radix = 10;
    if i + 1 < b.len() && b[i] == '0' {
        match b[i + 1] {
            'x' | 'X' => {
                radix = 16;
                i += 2;
            }
            'b' | 'B' => {
                radix = 2;
                i += 2;
            }
            'o' | 'O' => {
                radix = 8;
                i += 2;
            }
            'd' | 'D' => {
                radix = 10;
                i += 2;
            }
            _ => radix = 8,
        }
    }
    let mut digits = String::new();
    let mut prev_us = true; // a leading underscore is invalid
    while i < b.len() {
        let c = b[i];
        if c == '_' {
            if prev_us {
                return Err(bad());
            }
            prev_us = true;
        } else if c.is_digit(radix) {
            digits.push(c);
            prev_us = false;
        } else {
            break;
        }
        i += 1;
    }
    if digits.is_empty() || prev_us {
        return Err(bad());
    }
    while i < b.len() && is_regex_space(b[i]) {
        i += 1;
    }
    if i != b.len() {
        return Err(bad());
    }
    let mut n = BigInt::parse_bytes(digits.as_bytes(), radix).ok_or_else(bad)?;
    if neg {
        n = -n;
    }
    Ok(n)
}

/// Acceptance test of `Kernel#Float(string)`; the error is the ArgumentError message.
pub(crate) fn kernel_float(s: &str) -> Result<(), String> {
    if s.contains('\0') {
        return Err("string for Float contains null byte".into());
    }
    let bad = || Err(format!("invalid value for Float(): {}", inspect_str(s)));
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() && is_regex_space(b[i]) {
        i += 1;
    }
    if i < b.len() && (b[i] == '+' || b[i] == '-') {
        i += 1;
    }
    // Digit run with single underscores between digits.
    let run = |i: &mut usize, hex: bool| -> Option<usize> {
        let mut n = 0;
        let mut prev_us = false;
        while *i < b.len() {
            let c = b[*i];
            let is_d = if hex {
                c.is_ascii_hexdigit()
            } else {
                c.is_ascii_digit()
            };
            if is_d {
                n += 1;
                prev_us = false;
            } else if c == '_' && n > 0 && !prev_us {
                prev_us = true;
            } else {
                break;
            }
            *i += 1;
        }
        if prev_us {
            None
        } else {
            Some(n)
        }
    };
    let hex = i + 1 < b.len() && b[i] == '0' && (b[i + 1] == 'x' || b[i + 1] == 'X');
    if hex {
        i += 2;
        let Some(int) = run(&mut i, true) else {
            return bad();
        };
        let mut frac = 0;
        if i < b.len() && b[i] == '.' {
            i += 1;
            match run(&mut i, true) {
                Some(n) if n > 0 => frac = n,
                _ => return bad(),
            }
        }
        if int + frac == 0 {
            return bad();
        }
        if i < b.len() && (b[i] == 'p' || b[i] == 'P') {
            i += 1;
            if i < b.len() && (b[i] == '+' || b[i] == '-') {
                i += 1;
            }
            match run(&mut i, false) {
                Some(n) if n > 0 => {}
                _ => return bad(),
            }
        }
    } else {
        // Ruby 3.4+ accepts "1." and ".5"; at least one digit is required.
        let Some(int) = run(&mut i, false) else {
            return bad();
        };
        let mut frac = 0;
        if i < b.len() && b[i] == '.' {
            i += 1;
            let Some(n) = run(&mut i, false) else {
                return bad();
            };
            frac = n;
        }
        if int + frac == 0 {
            return bad();
        }
        if i < b.len() && (b[i] == 'e' || b[i] == 'E') {
            i += 1;
            if i < b.len() && (b[i] == '+' || b[i] == '-') {
                i += 1;
            }
            match run(&mut i, false) {
                Some(n) if n > 0 => {}
                _ => return bad(),
            }
        }
    }
    while i < b.len() && is_regex_space(b[i]) {
        i += 1;
    }
    if i != b.len() {
        return bad();
    }
    Ok(())
}

/// Lines as seen by Ruby's `^`/`$` anchors (`$` matches before every "\n").
pub(crate) fn lines(s: &str) -> impl Iterator<Item = &str> {
    s.split('\n')
}

/// Remove `/\+.*$/` matches (`gsub(/\+.*$/, "")`): from each "+" to the next newline.
pub(crate) fn strip_build(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut skipping = false;
    for c in s.chars() {
        if skipping {
            if c == '\n' {
                skipping = false;
                out.push(c);
            }
        } else if c == '+' {
            skipping = true;
        } else {
            out.push(c);
        }
    }
    out
}
