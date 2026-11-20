use super::{ruby_error, SchemaError};

// BigDecimal observes decimal Float spelling. Scientific exponents stay
// symbolic: modular exponentiation avoids allocating a giant power of ten.
pub(super) enum Decimal {
    Finite {
        coefficient: num_bigint::BigInt,
        exponent: i128,
    },
    Infinity,
    Nan,
}

impl Decimal {
    pub(super) fn is_multiple_of(&self, divisor: &Self) -> Result<bool, SchemaError> {
        use num_bigint::BigInt;
        let Self::Finite {
            coefficient: value,
            exponent: value_power,
        } = self
        else {
            return Ok(false);
        };
        let Self::Finite {
            coefficient: factor,
            exponent: factor_power,
        } = divisor
        else {
            return Ok(matches!(divisor, Self::Infinity) && *value == BigInt::from(0));
        };
        if *factor == BigInt::from(0) {
            return Err(ruby_error("ZeroDivisionError", "divided by 0"));
        }
        if *value == BigInt::from(0) {
            return Ok(true);
        }
        let modulus = if *factor < BigInt::from(0) {
            -factor
        } else {
            factor.clone()
        };
        let shift = *value_power - *factor_power;
        // BigDecimal allocates quotient space even for a remainder of zero.
        // Reserve (without touching pages) the same base-10^9 digit storage
        // when the exponent gap is huge. A failed reservation reproduces
        // Ruby's NoMemoryError instead of returning a mathematical answer
        // that the reference cannot compute. Small gaps stay symbolic.
        let value_digits = value.to_string().trim_start_matches('-').len() as i128;
        let factor_digits = factor.to_string().trim_start_matches('-').len() as i128;
        let exponent_gap = value_power + value_digits - factor_power - factor_digits;
        // For opposite signs Ruby also allocates the adjusted remainder
        // (factor + remainder), even when the quotient is zero.
        let adjusted_gap = if value.sign() != factor.sign() {
            exponent_gap.abs()
        } else {
            exponent_gap.max(0)
        };
        let quotient_digits = adjusted_gap + 1;
        if quotient_digits > i128::from(u32::MAX) {
            let words = usize::try_from((quotient_digits + 17 + 8) / 9)
                .map_err(|_| ruby_error("NoMemoryError", "failed to allocate memory"))?;
            let mut storage = Vec::<u32>::new();
            storage
                .try_reserve_exact(words)
                .map_err(|_| ruby_error("NoMemoryError", "failed to allocate memory"))?;
        }
        if shift >= 0 {
            Ok(
                (value * BigInt::from(10).modpow(&BigInt::from(shift), &modulus)) % modulus
                    == BigInt::from(0),
            )
        } else {
            let digits = value.to_string().len();
            if -shift > digits as i128 {
                return Ok(false);
            }
            let power = u32::try_from(-shift).expect("bounded by input digits");
            Ok(value % (modulus * BigInt::from(10).pow(power)) == BigInt::from(0))
        }
    }
}

pub(super) fn decimal(text: &str) -> Result<Option<Decimal>, SchemaError> {
    let text = text.trim_matches(|c: char| c.is_ascii_whitespace());
    if text == "NaN" {
        return Ok(Some(Decimal::Nan));
    }
    if matches!(text, "Infinity" | "+Infinity" | "-Infinity") {
        return Ok(Some(Decimal::Infinity));
    }
    let mut clean = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        if c == '_' {
            if i == 0 || !text.as_bytes()[i - 1].is_ascii_digit() {
                return Ok(None);
            }
        } else {
            clean.push(c);
        }
    }
    static SYNTAX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let syntax = SYNTAX.get_or_init(|| {
        regex::Regex::new(r"\A[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eEdD][+-]?[0-9]+)?\z")
            .expect("fixed decimal syntax")
    });
    if !syntax.is_match(&clean) {
        return Ok(None);
    }
    let (mantissa, raw_exponent) = match clean.find(['e', 'E', 'd', 'D']) {
        Some(i) => (&clean[..i], clean[i + 1..].parse::<i64>()),
        None => (clean.as_str(), Ok(0)),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{integer}{fraction}");
    let coefficient: num_bigint::BigInt = digits.parse().expect("validated decimal digits");
    if coefficient == num_bigint::BigInt::from(0) {
        return Ok(Some(Decimal::Finite {
            coefficient,
            exponent: 0,
        }));
    }
    let raw_exponent = match raw_exponent {
        Ok(n) => i128::from(n),
        Err(_)
            if clean
                .split(['e', 'E', 'd', 'D'])
                .nth(1)
                .unwrap()
                .starts_with('-') =>
        {
            return Ok(Some(Decimal::Finite {
                coefficient: 0.into(),
                exponent: 0,
            }));
        }
        Err(_) => return Ok(Some(Decimal::Infinity)),
    };
    // BigDecimal 4.1.3 uses signed 64-bit exponents in groups of nine
    // decimal digits. Overflow is decided before fractional leading zeros
    // are stripped; preserve that order (including zero/infinity outcomes).
    let significant_integer = integer
        .trim_start_matches(['+', '-'])
        .trim_start_matches('0')
        .len() as i128;
    let exponent = raw_exponent + significant_integer;
    let rounded = exponent + (9 - exponent % 9) % 9;
    let maximum = i128::from(i64::MAX / 9) * 9;
    let minimum = i128::from((i64::MIN + 8) / 9) * 9 - 8;
    if exponent > i128::from(i64::MAX) || rounded > maximum {
        return Ok(Some(Decimal::Infinity));
    }
    if rounded < minimum {
        return Ok(Some(Decimal::Finite {
            coefficient: 0.into(),
            exponent: 0,
        }));
    }
    let power = raw_exponent - fraction.len() as i128;
    let normalized = power + coefficient.to_string().trim_start_matches('-').len() as i128;
    let normalized_group = normalized + (9 - normalized % 9) % 9;
    if normalized_group < minimum + 8 {
        return Ok(Some(Decimal::Finite {
            coefficient: 0.into(),
            exponent: 0,
        }));
    }
    Ok(Some(Decimal::Finite {
        coefficient,
        exponent: power,
    }))
}
