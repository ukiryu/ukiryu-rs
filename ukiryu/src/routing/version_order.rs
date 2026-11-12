//! The `Models::SemanticVersion` ordering that `ToolIndex` uses to pick the
//! newest version file (lib/ukiryu/models/semantic_version.rb).
//!
//! This is a private copy of the few lines the index needs; the model itself
//! belongs to the typed-models owner.

use num_bigint::BigInt;
use std::cmp::Ordering;

/// `SemanticVersion.parse`: `split('.')` (trailing empty fields dropped), each
/// field through `String#to_i`; `nil` or `""` is `[0]`.
pub(crate) fn segments(version: &str) -> Vec<BigInt> {
    if version.is_empty() {
        return vec![BigInt::from(0)];
    }
    let mut fields: Vec<&str> = version.split('.').collect();
    while fields.last() == Some(&"") {
        fields.pop();
    }
    fields.into_iter().map(ruby_to_i).collect()
}

/// `SemanticVersion#<=>`: compare field by field, missing fields are 0.
pub(crate) fn compare(a: &[BigInt], b: &[BigInt]) -> Ordering {
    let zero = BigInt::from(0);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).unwrap_or(&zero);
        let y = b.get(i).unwrap_or(&zero);
        match x.cmp(y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// Ruby `String#to_i`: optional leading whitespace, an optional sign, then
/// decimal digits with single underscores between them; anything else ends
/// the number. No digits is 0.
fn ruby_to_i(field: &str) -> BigInt {
    let bytes = field.as_bytes();
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let negative = match bytes.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut digits = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            digits.push(b as char);
            i += 1;
        } else if b == b'_'
            && !digits.is_empty()
            && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
        {
            i += 1;
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return BigInt::from(0);
    }
    let value: BigInt = digits.parse().unwrap_or_default();
    if negative {
        -value
    } else {
        value
    }
}

// Expected values captured from the pinned Ruby `SemanticVersion.parse` /
// `.compare` (K18-ROUTING evidence probe_semver.rb).
#[cfg(test)]
mod tests {
    use super::*;

    fn seg(v: &str) -> Vec<i64> {
        segments(v)
            .into_iter()
            .map(|x| i64::try_from(x).unwrap())
            .collect()
    }

    #[test]
    fn parses_like_ruby_to_i() {
        assert_eq!(seg(""), vec![0]);
        assert_eq!(seg("1.10.0"), vec![1, 10, 0]);
        assert_eq!(seg("1.0."), vec![1, 0]);
        assert_eq!(seg("."), Vec::<i64>::new());
        assert_eq!(seg("1..2"), vec![1, 0, 2]);
        assert_eq!(seg(" 0.2"), vec![0, 2]);
        assert_eq!(seg("-1.5"), vec![-1, 5]);
        assert_eq!(seg("1_000.x9"), vec![1000, 0]);
        assert_eq!(seg("2a.b3"), vec![2, 0]);
    }

    #[test]
    fn compares_with_zero_padding() {
        assert_eq!(compare(&segments("2"), &segments("2.0")), Ordering::Equal);
        assert_eq!(
            compare(&segments("1.10"), &segments("1.9.9")),
            Ordering::Greater
        );
    }
}
