//! Ruby core semantics the models depend on (numeric formatting, String
//! parsing, Array#sum), each pinned by oracle captures in the model tests.
use num_bigint::BigInt;

/// `Float#to_s`.
pub fn float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    // Shortest round-trip digits and decimal exponent, as Ruby's dtoa mode 0.
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').expect("exponent form");
    let exp: i32 = exp.parse().expect("exponent digits");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt = exp + 1;
    let sign = if f < 0.0 { "-" } else { "" };
    let len = digits.len() as i32;
    if decpt > 0 && decpt <= 16 {
        if decpt >= len {
            let zeros = "0".repeat((decpt - len) as usize);
            format!("{sign}{digits}{zeros}.0")
        } else {
            let (a, b) = digits.split_at(decpt as usize);
            format!("{sign}{a}.{b}")
        }
    } else if decpt <= 0 && decpt > -4 {
        let zeros = "0".repeat((-decpt) as usize);
        format!("{sign}0.{zeros}{digits}")
    } else {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() { "0" } else { rest };
        let e = decpt - 1;
        let esign = if e < 0 { '-' } else { '+' };
        format!("{sign}{first}.{rest}e{esign}{:02}", e.abs())
    }
}

fn frexp_exponent(x: f64) -> i32 {
    if x == 0.0 || !x.is_finite() {
        return 0;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // subnormal: scale into the normal range first
        frexp_exponent(x * 2f64.powi(64)) - 64
    } else {
        exp - 1022
    }
}

fn round_half_up(x: f64, s: f64) -> f64 {
    let xs = x * s;
    let mut f = xs.round();
    if s == 1.0 {
        return f;
    }
    if x > 0.0 {
        if (f + 0.5) / s <= x {
            f += 1.0;
        }
    } else if (f - 0.5) / s >= x {
        f -= 1.0;
    }
    f
}

/// `Float#round(ndigits)` for 0 < ndigits <= 14 (the double-based path).
pub fn float_round(x: f64, ndigits: i32) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let binexp = frexp_exponent(x);
    // float_round_overflow
    let over = if binexp > 0 {
        binexp / 4
    } else {
        binexp / 3 - 1
    };
    if ndigits >= 16 - over {
        return x;
    }
    // float_round_underflow
    let under = if binexp > 0 {
        binexp / 3 + 1
    } else {
        binexp / 4
    };
    if ndigits < -under {
        return 0.0;
    }
    let s = 10f64.powi(ndigits);
    round_half_up(x, s) / s
}

/// `Array#sum` over Floats (Kahan-Babuska compensated, as MRI does),
/// starting from the Integer 0.
pub fn float_sum(values: &[f64]) -> f64 {
    let mut f = 0.0f64;
    let mut c = 0.0f64;
    for &x in values {
        if f.is_nan() {
            continue;
        }
        if x.is_nan() {
            f = x;
            continue;
        }
        if x.is_infinite() {
            if f.is_infinite() && x.is_sign_negative() != f.is_sign_negative() {
                f = f64::NAN;
            } else {
                f = x;
            }
            continue;
        }
        if f.is_infinite() {
            continue;
        }
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    f + c
}

/// `String#inspect` for UTF-8 strings.
pub fn string_inspect(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
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
            '#' => {
                // `#{`, `#$`, `#@` are escaped so the result re-evaluates literally.
                if matches!(chars.get(i + 1), Some('{') | Some('$') | Some('@')) {
                    out.push_str("\\#");
                } else {
                    out.push('#');
                }
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `String#to_i` (base 10): optional leading whitespace, sign, digits with
/// single underscores; stops at the first invalid character.
pub fn str_to_i(s: &str) -> i128 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut n: i128 = 0;
    let mut prev_digit = false;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(i128::from(b - b'0'));
            prev_digit = true;
        } else if b == b'_' && prev_digit && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
            prev_digit = false;
        } else {
            break;
        }
        i += 1;
    }
    if neg {
        -n
    } else {
        n
    }
}

/// `String#to_i`: leading whitespace, an optional sign, then decimal digits
/// with single underscores between them. Exact for any length (Bignum).
pub fn str_to_bigint(s: &str) -> BigInt {
    let t = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let bytes = t.as_bytes();
    let mut digits = String::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            digits.push(char::from(b));
        } else if b == b'_'
            && i > 0
            && bytes[i - 1].is_ascii_digit()
            && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
        {
            continue;
        } else {
            break;
        }
    }
    let n: BigInt = if digits.is_empty() {
        BigInt::from(0)
    } else {
        digits.parse().expect("ASCII digits")
    };
    if neg {
        -n
    } else {
        n
    }
}

/// `String#to_f`: the longest leading decimal float literal, else 0.0.
pub fn str_to_f(s: &str) -> f64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let b = t.as_bytes();
    let mut i = 0;
    let mut out = String::new();
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        out.push(b[i] as char);
        i += 1;
    }
    let digits = |i: &mut usize, out: &mut String| {
        let start = *i;
        while *i < b.len() {
            if b[*i].is_ascii_digit() {
                out.push(b[*i] as char);
            } else if b[*i] == b'_'
                && *i > start
                && b[*i - 1].is_ascii_digit()
                && b.get(*i + 1).is_some_and(u8::is_ascii_digit)
            {
            } else {
                break;
            }
            *i += 1;
        }
        *i > start
    };
    if !digits(&mut i, &mut out) {
        return 0.0;
    }
    if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
        out.push('.');
        i += 1;
        digits(&mut i, &mut out);
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        let mut exp = String::from("e");
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            exp.push(b[j] as char);
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            exp.push(b[j] as char);
            j += 1;
        }
        if j > start {
            out.push_str(&exp);
        }
    }
    out.parse().unwrap_or(0.0)
}

fn kernel_integer_base10(s: &str) -> Option<i128> {
    // Kernel#Integer(str, 10): surrounding whitespace, sign, digits with
    // single underscores between digits; anything else is an error.
    let t = s.trim_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if body.is_empty() || body.starts_with('_') || body.ends_with('_') || body.contains("__") {
        return None;
    }
    if !body.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
        return None;
    }
    let digits: String = body.chars().filter(|c| *c != '_').collect();
    let n: i128 = digits.parse().ok()?;
    Some(if neg { -n } else { n })
}

/// `Lutaml::Model::Type::Integer.cast` for a String.
pub fn lutaml_integer_from_str(s: &str) -> Option<i128> {
    let lines_match = |pred: &dyn Fn(&str) -> bool| s.split('\n').any(pred);
    // /^0[0-7]+$/ (line anchors)
    if lines_match(&|l: &str| {
        l.len() > 1 && l.starts_with('0') && l[1..].bytes().all(|b| (b'0'..=b'7').contains(&b))
    }) {
        return Some(str_to_i_radix8(s));
    }
    if lines_match(&is_float_like) {
        let f: f64 = s.trim().parse().ok()?;
        return float_to_integer(f);
    }
    kernel_integer_base10(s)
}

fn str_to_i_radix8(s: &str) -> i128 {
    let t = s.trim_start();
    let mut n: i128 = 0;
    for b in t.bytes() {
        if (b'0'..=b'7').contains(&b) {
            n = n * 8 + i128::from(b - b'0');
        } else if b == b'_' {
            continue;
        } else {
            break;
        }
    }
    n
}

/// /^-?\d+(\.\d+)?(e-?\d+)?$/i
fn is_float_like(l: &str) -> bool {
    let b = l.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') {
        i += 1;
    }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        let s2 = i + 1;
        let mut j = s2;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == s2 {
            return false;
        }
        i = j;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && b[j] == b'-' {
            j += 1;
        }
        let s3 = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == s3 {
            return false;
        }
        i = j;
    }
    i == b.len()
}

/// `Integer(float)` (truncation; NaN/Infinity raise, which lutaml maps to nil).
pub fn float_to_integer(f: f64) -> Option<i128> {
    if !f.is_finite() {
        return None;
    }
    let t = f.trunc();
    if t.abs() < 1.7e38 {
        Some(t as i128)
    } else {
        None
    }
}

/// `String#capitalize` (simple case mapping).
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.extend(chars.flat_map(char::to_lowercase));
            out
        }
    }
}

/// Numeric argument for the duration formatters (`Integer` or `Float`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i128),
    Float(f64),
}

impl Number {
    fn is_zero(self) -> bool {
        match self {
            Number::Int(i) => i == 0,
            Number::Float(f) => f == 0.0,
        }
    }
}

/// The private `format_duration` shared by StageMetrics and ExecutionReport:
/// `'0ms'` for zero, `"#{(s * 1000).round(2)}ms"` below one second,
/// else `"#{s.round(2)}s"`.
pub fn format_duration(seconds: Number) -> String {
    if seconds.is_zero() {
        return "0ms".into();
    }
    match seconds {
        Number::Int(i) => {
            if i < 1 {
                format!("{}ms", i * 1000)
            } else {
                format!("{i}s")
            }
        }
        Number::Float(f) => {
            if f < 1.0 {
                format!("{}ms", float_to_s(float_round(f * 1000.0, 2)))
            } else {
                format!("{}s", float_to_s(float_round(f, 2)))
            }
        }
    }
}
