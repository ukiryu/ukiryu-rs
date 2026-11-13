//! Ruby `Time` as an exact instant.
//!
//! Source: `time.c` of Ruby 4.0.1 (ruby e04267a1), the `Time` the pinned
//! ukiryu/ukiryu ab6df353 receives and returns. Ruby keeps the instant as
//! `timew`, an exact count of nanoseconds since the Unix epoch that is an
//! Integer or, when the time has a fraction finer than a nanosecond (for
//! example `Time.parse("...30.1234567891Z")` or `Time.at(Rational(1, 3))`),
//! a Rational. [`Time`] keeps the same exact value: whole seconds,
//! whole nanoseconds, and the exact remaining fraction of a nanosecond
//! ([`SubNanos`], arbitrary precision through `num-rational`).
//!
//! Every method that Ruby computes from `timew` is computed from that exact
//! value: `to_r`, `subsec`, `nsec`, `usec`, `<=>`, `eql?`, `hash`,
//! `inspect`, and the Float conversions `to_f` and `Time#-`, which round
//! exactly as Ruby rounds (no intermediate `f64`).
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

/// Nanoseconds per second (`TIME_SCALE`).
const SCALE: i64 = 1_000_000_000;

fn scale() -> BigInt {
    BigInt::from(SCALE)
}

fn is_zero(r: &BigRational) -> bool {
    r.numer().sign() == Sign::NoSign
}

/// The exact part of a [`Time`] finer than a whole nanosecond: a fraction
/// of one nanosecond in `[0, 1)`, kept reduced. Zero for every time with a
/// whole number of nanoseconds (all clock readings and all decimal
/// fractions of up to nine digits).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubNanos(Option<Box<BigRational>>);

impl SubNanos {
    /// No fraction of a nanosecond.
    pub const ZERO: SubNanos = SubNanos(None);

    /// `numer / denom` of a nanosecond; `None` unless `0 <= numer/denom < 1`.
    pub fn new(numer: BigInt, denom: BigInt) -> Option<SubNanos> {
        if denom.sign() == Sign::NoSign {
            return None;
        }
        SubNanos::from_ratio(BigRational::new(numer, denom))
    }

    fn from_ratio(r: BigRational) -> Option<SubNanos> {
        if r.numer().sign() == Sign::Minus || r >= BigRational::from_integer(BigInt::from(1)) {
            return None;
        }
        Some(if is_zero(&r) {
            SubNanos::ZERO
        } else {
            SubNanos(Some(Box::new(r)))
        })
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_none()
    }

    /// Reduced numerator (`0` when zero).
    pub fn numer(&self) -> BigInt {
        self.0
            .as_ref()
            .map_or_else(|| BigInt::from(0), |r| r.numer().clone())
    }

    /// Reduced, positive denominator (`1` when zero).
    pub fn denom(&self) -> BigInt {
        self.0
            .as_ref()
            .map_or_else(|| BigInt::from(1), |r| r.denom().clone())
    }

    fn ratio(&self) -> BigRational {
        self.0
            .as_deref()
            .cloned()
            .unwrap_or_else(|| BigRational::from_integer(BigInt::from(0)))
    }
}

/// A Ruby `Time`: an exact instant plus the UTC offset it is displayed in.
///
/// The instant is `seconds + (nanos + subnanos) / 10^9` seconds after the
/// Unix epoch: `seconds` is floored Unix time, `nanos` the whole
/// nanoseconds of the non-negative sub-second part (Ruby `Time#nsec`), and
/// `subnanos` the exact rest, below one nanosecond. `utc` distinguishes a
/// UTC time (`2024-01-17 10:20:30 UTC`) from a local time whose offset
/// happens to be zero (`2024-01-17 10:20:30 +0000`).
///
/// Build values with [`Time::new`] (whole nanoseconds) or
/// [`Time::from_rational`] / [`Time::from_unix_nanos`] (exact); read the
/// exact instant with [`Time::to_r`], [`Time::subsec`] or
/// [`Time::unix_nanos`]. `==` on the struct is field equality; Ruby's
/// `==`/`eql?`/`hash` compare the instant only ([`Time::instant_eql`] and
/// the [`Hash`] impl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Time {
    pub seconds: i64,
    pub nanos: u32,
    pub utc_offset: i32,
    pub utc: bool,
    pub subnanos: SubNanos,
}

impl Time {
    /// A time with a whole number of nanoseconds. `nanos` of a second or
    /// more carries into `seconds`.
    pub fn new(seconds: i64, nanos: u32, utc_offset: i32, utc: bool) -> Time {
        let carry = i64::from(nanos / SCALE as u32);
        Time {
            seconds: seconds + carry,
            nanos: nanos % SCALE as u32,
            utc_offset,
            utc,
            subnanos: SubNanos::ZERO,
        }
    }

    /// `Time.at(Rational(numer, denom))` in the given zone: exactly
    /// `numer / denom` seconds after the epoch. `None` when `denom` is zero
    /// or the whole seconds do not fit `i64`.
    pub fn from_rational(
        numer: &BigInt,
        denom: &BigInt,
        utc_offset: i32,
        utc: bool,
    ) -> Option<Time> {
        if denom.sign() == Sign::NoSign {
            return None;
        }
        let ns = BigRational::new(numer * scale(), denom.clone());
        Time::from_ns(&ns, utc_offset, utc)
    }

    /// Exactly `numer / denom` nanoseconds after the epoch (Ruby's `timew`).
    pub fn from_unix_nanos(
        numer: &BigInt,
        denom: &BigInt,
        utc_offset: i32,
        utc: bool,
    ) -> Option<Time> {
        if denom.sign() == Sign::NoSign {
            return None;
        }
        Time::from_ns(
            &BigRational::new(numer.clone(), denom.clone()),
            utc_offset,
            utc,
        )
    }

    fn from_ns(ns: &BigRational, utc_offset: i32, utc: bool) -> Option<Time> {
        let whole = ns.floor().to_integer();
        let rest = ns - BigRational::from_integer(whole.clone());
        let seconds = BigRational::new(whole.clone(), scale())
            .floor()
            .to_integer();
        let nanos = whole - &seconds * scale();
        Some(Time {
            seconds: i64::try_from(&seconds).ok()?,
            nanos: u32::try_from(&nanos).ok()?,
            utc_offset,
            utc,
            subnanos: SubNanos::from_ratio(rest)?,
        })
    }

    /// The exact nanoseconds since the epoch (Ruby's `timew`).
    fn ns(&self) -> BigRational {
        let whole = BigInt::from(self.seconds) * scale() + BigInt::from(self.nanos);
        BigRational::from_integer(whole) + self.subnanos.ratio()
    }

    fn subsec_ns(&self) -> BigRational {
        BigRational::from_integer(BigInt::from(self.nanos)) + self.subnanos.ratio()
    }

    /// Whether the instant is a whole number of nanoseconds (Ruby's `timew`
    /// is an Integer).
    pub fn is_integral_nanos(&self) -> bool {
        self.subnanos.is_zero()
    }

    /// The exact nanoseconds since the epoch as a reduced `(numer, denom)`.
    pub fn unix_nanos(&self) -> (BigInt, BigInt) {
        parts(self.ns())
    }

    /// `Time#to_r`: the exact Unix time in seconds, reduced `(numer, denom)`.
    pub fn to_r(&self) -> (BigInt, BigInt) {
        parts(self.ns() / BigRational::from_integer(scale()))
    }

    /// `Time#subsec`: the exact fraction of the second, reduced
    /// `(numer, denom)`; `(0, 1)` when there is none.
    pub fn subsec(&self) -> (BigInt, BigInt) {
        parts(self.subsec_ns() / BigRational::from_integer(scale()))
    }

    /// `Time#nsec` (truncated).
    pub fn nsec(&self) -> u32 {
        self.nanos
    }

    /// `Time#usec` (truncated).
    pub fn usec(&self) -> u32 {
        self.nanos / 1000
    }

    /// `Time#to_i`.
    pub fn to_i(&self) -> i64 {
        self.seconds
    }

    /// `Time#to_f` (`rb_time_unmagnify_to_float`).
    pub fn to_f(&self) -> f64 {
        unmagnify_to_float(&self.ns())
    }

    /// `self - other` for two Times (`Time#-`): the exact difference as a
    /// Float, rounded the way Ruby rounds it.
    pub fn minus_to_f(&self, other: &Time) -> f64 {
        unmagnify_to_float(&(self.ns() - other.ns()))
    }

    /// `Time#<=>` with another Time: instant order, zone ignored.
    pub fn instant_cmp(&self, other: &Time) -> Ordering {
        (self.seconds, self.nanos, &self.subnanos).cmp(&(
            other.seconds,
            other.nanos,
            &other.subnanos,
        ))
    }

    /// `Time#eql?` (and `Time#==`) with another Time: the same instant.
    pub fn instant_eql(&self, other: &Time) -> bool {
        self.instant_cmp(other) == Ordering::Equal
    }

    /// The sub-second text `Time#inspect` appends after `HH:MM:SS`:
    /// `""`, `".123456789"` style digits without trailing zeros, or, for a
    /// fraction finer than a nanosecond, `" n/d"` (the `Rational#to_s` of
    /// `subsec`).
    pub fn inspect_subsec(&self) -> String {
        if !self.subnanos.is_zero() {
            let (n, d) = self.subsec();
            return format!(" {n}/{d}");
        }
        if self.nanos == 0 {
            return String::new();
        }
        let frac = format!("{:09}", self.nanos);
        format!(".{}", frac.trim_end_matches('0'))
    }
}

/// Ruby `Time#hash` hashes `timew` only, consistent with `eql?`.
impl Hash for Time {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.seconds.hash(state);
        self.nanos.hash(state);
        self.subnanos.hash(state);
    }
}

fn parts(r: BigRational) -> (BigInt, BigInt) {
    r.into_raw()
}

/// `rb_time_unmagnify_to_float` on 64-bit Ruby: an Integer `timew` is
/// divided by the Float `1e9`; a Rational one becomes `Rational(timew,
/// 10^9)` and `rb_Float` converts that as `big2dbl(num) / big2dbl(den)`
/// (`rat2dbl_without_to_f`), each part correctly rounded on its own.
fn unmagnify_to_float(ns: &BigRational) -> f64 {
    use super::ruby::bigint_to_f64;
    if ns.is_integer() {
        return bigint_to_f64(ns.numer()) / 1e9;
    }
    let secs = ns / BigRational::from_integer(scale());
    bigint_to_f64(secs.numer()) / bigint_to_f64(secs.denom())
}
