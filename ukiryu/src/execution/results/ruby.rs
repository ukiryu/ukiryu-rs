//! Ruby core behaviour that the execution result views embed: `Float#round`,
//! the json gem's float output, `Time#-`, `Time#iso8601`, `File.basename`
//! and `String#split(/\r?\n/)`.
//!
//! Each function reproduces Ruby 4.0.1 with json 3.1.0 (the pinned bundle) as
//! captured in `tests/execution_result_golden/`.
use std::io;

use serde::Serialize;

use crate::types::ruby::{civil_from_days, float_to_s};
use crate::types::value::Time;

/// `Float#round(ndigits)` for `ndigits > 0` (float.c `rb_float_round` with
/// `round_half_up`).
pub fn float_round(number: f64, ndigits: i32) -> f64 {
    debug_assert!(ndigits > 0);
    if number == 0.0 || !number.is_finite() {
        return number;
    }
    let binexp = frexp_exp(number);
    // float_round_overflow: the value already has no digits beyond ndigits.
    let float_dig = f64::DIGITS as i32 + 2;
    let limit = float_dig
        - if binexp > 0 {
            binexp / 4
        } else {
            binexp / 3 - 1
        };
    if ndigits >= limit {
        return number;
    }
    // float_round_underflow (checked for positive values only, as in Ruby).
    if number > 0.0
        && ndigits
            < -(if binexp > 0 {
                binexp / 3 + 1
            } else {
                binexp / 4
            })
    {
        return 0.0;
    }
    if ndigits > 14 {
        // Ruby switches to rational rounding here; durations never need it.
        return number;
    }
    let s = 10f64.powi(ndigits);
    round_half_up(number, s) / s
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

/// The binary exponent `frexp` reports: `x = m * 2^e` with `0.5 <= |m| < 1`.
fn frexp_exp(x: f64) -> i32 {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // Subnormal: normalise by scaling with 2^64.
        return frexp_exp(x * f64::from_bits(0x43f0_0000_0000_0000)) - 64;
    }
    exp - 1022
}

/// The json gem's `generate` output for a finite Float (vendored fpconv
/// `fpconv_dtoa`: Grisu2 digits, then `emit_digits`). Non-finite values are
/// rejected before this point.
pub fn json_float(value: f64) -> String {
    let mut out = String::new();
    if value.is_sign_negative() {
        out.push('-');
    }
    if value == 0.0 {
        out.push_str("0.0");
        return out;
    }
    let (digits, k) = grisu2(value);
    let n = digits.len() as i32;
    let e = k + n - 1;
    let abs_exp = e.abs();
    if k >= 0 && abs_exp < 15 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', k as usize));
        out.push_str(".0");
    } else if k < 0 && (k > -7 || abs_exp < 10) {
        let offset = n + k;
        if offset <= 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-offset) as usize));
            out.push_str(&digits);
        } else {
            out.push_str(&digits[..offset as usize]);
            out.push('.');
            out.push_str(&digits[offset as usize..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if n > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&abs_exp.to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Grisu2, transcribed from the json gem's vendored fpconv.c (json 3.1.0). The
// gem does not print the shortest digits in every case, so the port keeps
// its digit generator rather than Rust's shortest formatting.

const POWERS_TEN: [(u64, i32); 87] = [
    (18054884314459144840, -1220),
    (13451937075301367670, -1193),
    (10022474136428063862, -1166),
    (14934650266808366570, -1140),
    (11127181549972568877, -1113),
    (16580792590934885855, -1087),
    (12353653155963782858, -1060),
    (18408377700990114895, -1034),
    (13715310171984221708, -1007),
    (10218702384817765436, -980),
    (15227053142812498563, -954),
    (11345038669416679861, -927),
    (16905424996341287883, -901),
    (12595523146049147757, -874),
    (9384396036005875287, -847),
    (13983839803942852151, -821),
    (10418772551374772303, -794),
    (15525180923007089351, -768),
    (11567161174868858868, -741),
    (17236413322193710309, -715),
    (12842128665889583758, -688),
    (9568131466127621947, -661),
    (14257626930069360058, -635),
    (10622759856335341974, -608),
    (15829145694278690180, -582),
    (11793632577567316726, -555),
    (17573882009934360870, -529),
    (13093562431584567480, -502),
    (9755464219737475723, -475),
    (14536774485912137811, -449),
    (10830740992659433045, -422),
    (16139061738043178685, -396),
    (12024538023802026127, -369),
    (17917957937422433684, -343),
    (13349918974505688015, -316),
    (9946464728195732843, -289),
    (14821387422376473014, -263),
    (11042794154864902060, -236),
    (16455045573212060422, -210),
    (12259964326927110867, -183),
    (18268770466636286478, -157),
    (13611294676837538539, -130),
    (10141204801825835212, -103),
    (15111572745182864684, -77),
    (11258999068426240000, -50),
    (16777216000000000000, -24),
    (12500000000000000000, 3),
    (9313225746154785156, 30),
    (13877787807814456755, 56),
    (10339757656912845936, 83),
    (15407439555097886824, 109),
    (11479437019748901445, 136),
    (17105694144590052135, 162),
    (12744735289059618216, 189),
    (9495567745759798747, 216),
    (14149498560666738074, 242),
    (10542197943230523224, 269),
    (15709099088952724970, 295),
    (11704190886730495818, 322),
    (17440603504673385349, 348),
    (12994262207056124023, 375),
    (9681479787123295682, 402),
    (14426529090290212157, 428),
    (10748601772107342003, 455),
    (16016664761464807395, 481),
    (11933345169920330789, 508),
    (17782069995880619868, 534),
    (13248674568444952270, 561),
    (9871031767461413346, 588),
    (14708983551653345445, 614),
    (10959046745042015199, 641),
    (16330252207878254650, 667),
    (12166986024289022870, 694),
    (18130221999122236476, 720),
    (13508068024458167312, 747),
    (10064294952495520794, 774),
    (14996968138956309548, 800),
    (11173611982879273257, 827),
    (16649979327439178909, 853),
    (12405201291620119593, 880),
    (9242595204427927429, 907),
    (13772540099066387757, 933),
    (10261342003245940623, 960),
    (15290591125556738113, 986),
    (11392378155556871081, 1013),
    (16975966327722178521, 1039),
    (12648080533535911531, 1066),
];

const TENS: [u64; 20] = [
    10_000_000_000_000_000_000,
    1_000_000_000_000_000_000,
    100_000_000_000_000_000,
    10_000_000_000_000_000,
    1_000_000_000_000_000,
    100_000_000_000_000,
    10_000_000_000_000,
    1_000_000_000_000,
    100_000_000_000,
    10_000_000_000,
    1_000_000_000,
    100_000_000,
    10_000_000,
    1_000_000,
    100_000,
    10_000,
    1_000,
    100,
    10,
    1,
];

const HIDDEN_BIT: u64 = 0x0010_0000_0000_0000;

#[derive(Clone, Copy)]
struct Fp {
    frac: u64,
    exp: i32,
}

fn build_fp(d: f64) -> Fp {
    let bits = d.to_bits();
    let frac = bits & 0x000F_FFFF_FFFF_FFFF;
    let exp = ((bits & 0x7FF0_0000_0000_0000) >> 52) as i32;
    if exp != 0 {
        Fp {
            frac: frac + HIDDEN_BIT,
            exp: exp - 1075,
        }
    } else {
        Fp {
            frac,
            exp: -1075 + 1,
        }
    }
}

fn normalize(fp: &mut Fp) {
    while fp.frac & HIDDEN_BIT == 0 {
        fp.frac <<= 1;
        fp.exp -= 1;
    }
    fp.frac <<= 11;
    fp.exp -= 11;
}

fn normalized_boundaries(fp: &Fp) -> (Fp, Fp) {
    let mut upper = Fp {
        frac: (fp.frac << 1) + 1,
        exp: fp.exp - 1,
    };
    while upper.frac & (HIDDEN_BIT << 1) == 0 {
        upper.frac <<= 1;
        upper.exp -= 1;
    }
    upper.frac <<= 10;
    upper.exp -= 10;
    let l_shift = if fp.frac == HIDDEN_BIT { 2 } else { 1 };
    let mut lower = Fp {
        frac: (fp.frac << l_shift) - 1,
        exp: fp.exp - l_shift,
    };
    lower.frac <<= lower.exp - upper.exp;
    lower.exp = upper.exp;
    (lower, upper)
}

fn cached_pow10(exp: i32) -> (Fp, i32) {
    let approx = (-(exp + 87) as f64 * 0.301_029_995_663_981_14) as i32;
    let mut idx = ((approx + 348) / 8) as usize;
    loop {
        let current = exp + POWERS_TEN[idx].1 + 64;
        if current < -60 {
            idx += 1;
            continue;
        }
        if current > -32 {
            idx -= 1;
            continue;
        }
        let (frac, exp) = POWERS_TEN[idx];
        return (Fp { frac, exp }, -348 + idx as i32 * 8);
    }
}

fn multiply(a: &Fp, b: &Fp) -> Fp {
    const LO: u64 = 0xFFFF_FFFF;
    let ah_bl = (a.frac >> 32) * (b.frac & LO);
    let al_bh = (a.frac & LO) * (b.frac >> 32);
    let al_bl = (a.frac & LO) * (b.frac & LO);
    let ah_bh = (a.frac >> 32) * (b.frac >> 32);
    let mut tmp = (ah_bl & LO) + (al_bh & LO) + (al_bl >> 32);
    tmp = tmp.wrapping_add(1 << 31);
    Fp {
        frac: ah_bh
            .wrapping_add(ah_bl >> 32)
            .wrapping_add(al_bh >> 32)
            .wrapping_add(tmp >> 32),
        exp: a.exp + b.exp + 64,
    }
}

fn round_digit(digits: &mut [u8], delta: u64, mut rem: u64, kappa: u64, frac: u64) {
    let last = digits.len() - 1;
    while rem < frac
        && delta.wrapping_sub(rem) >= kappa
        && (rem.wrapping_add(kappa) < frac
            || frac.wrapping_sub(rem) > rem.wrapping_add(kappa).wrapping_sub(frac))
    {
        digits[last] -= 1;
        rem = rem.wrapping_add(kappa);
    }
}

fn generate_digits(fp: &Fp, upper: &Fp, lower: &Fp, k: &mut i32) -> Vec<u8> {
    let wfrac = upper.frac.wrapping_sub(fp.frac);
    let mut delta = upper.frac.wrapping_sub(lower.frac);
    let shift = (-upper.exp) as u32;
    let one = 1u64 << shift;
    let mut part1 = upper.frac >> shift;
    let mut part2 = upper.frac & (one - 1);
    let mut digits: Vec<u8> = Vec::with_capacity(18);
    let mut kappa = 10;
    for &div in &TENS[10..] {
        if kappa == 0 {
            break;
        }
        let digit = part1 / div;
        if digit != 0 || !digits.is_empty() {
            digits.push(b'0' + digit as u8);
        }
        part1 -= digit * div;
        kappa -= 1;
        let tmp = (part1 << shift).wrapping_add(part2);
        if tmp <= delta {
            *k += kappa;
            round_digit(&mut digits, delta, tmp, div << shift, wfrac);
            return digits;
        }
    }
    let mut unit = 18;
    loop {
        part2 = part2.wrapping_mul(10);
        delta = delta.wrapping_mul(10);
        kappa -= 1;
        let digit = part2 >> shift;
        if digit != 0 || !digits.is_empty() {
            digits.push(b'0' + digit as u8);
        }
        part2 &= one - 1;
        if part2 < delta {
            *k += kappa;
            round_digit(
                &mut digits,
                delta,
                part2,
                one,
                wfrac.wrapping_mul(TENS[unit]),
            );
            return digits;
        }
        unit -= 1;
    }
}

/// fpconv `grisu2`: the decimal digits of `d` and the power of ten of the last digit.
fn grisu2(d: f64) -> (String, i32) {
    let mut w = build_fp(d);
    let (mut lower, mut upper) = normalized_boundaries(&w);
    normalize(&mut w);
    let (cp, k) = cached_pow10(upper.exp);
    w = multiply(&w, &cp);
    upper = multiply(&upper, &cp);
    lower = multiply(&lower, &cp);
    lower.frac = lower.frac.wrapping_add(1);
    upper.frac = upper.frac.wrapping_sub(1);
    let mut big_k = -k;
    let digits = generate_digits(&w, &upper, &lower, &mut big_k);
    (String::from_utf8(digits).expect("ASCII digits"), big_k)
}

/// A non-finite Float reached the JSON generator
/// (`JSON::GeneratorError`, "`<value>` not allowed in JSON").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonGeneratorError {
    pub message: String,
}

impl JsonGeneratorError {
    pub fn ruby_class(&self) -> &'static str {
        "JSON::GeneratorError"
    }
}

impl std::fmt::Display for JsonGeneratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsonGeneratorError {}

/// serde_json compact output whose floats are written as the json gem writes them.
struct RubyJsonFormatter;

impl serde_json::ser::Formatter for RubyJsonFormatter {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(json_float(value).as_bytes())
    }

    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        self.write_f64(writer, value as f64)
    }
}

/// `JSON.generate(value)` for a serde view. `non_finite` lists the Floats the
/// view contains, in output order, so a NaN/Infinity raises like the gem
/// instead of serde_json's silent `null`.
pub(crate) fn to_ruby_json<T: Serialize>(
    value: &T,
    floats: &[f64],
) -> Result<String, JsonGeneratorError> {
    if let Some(bad) = floats.iter().find(|f| !f.is_finite()) {
        return Err(JsonGeneratorError {
            message: format!("{} not allowed in JSON", float_to_s(*bad)),
        });
    }
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, RubyJsonFormatter);
    value
        .serialize(&mut ser)
        .expect("in-memory JSON serialisation cannot fail");
    Ok(String::from_utf8(buf).expect("serde_json writes UTF-8"))
}

/// `later - earlier` for two Ruby Times: the exact nanosecond difference
/// divided as a Float (`rb_time_unmagnify_to_float`).
pub fn time_sub(later: &Time, earlier: &Time) -> f64 {
    let ns = |t: &Time| t.seconds as i128 * 1_000_000_000 + t.nanos as i128;
    (ns(later) - ns(earlier)) as f64 / 1e9
}

/// `Time#iso8601` (`xmlschema` without fraction digits).
pub fn time_iso8601(t: &Time) -> String {
    let local = t.seconds + t.utc_offset as i64;
    let (y, mo, d) = civil_from_days(local.div_euclid(86_400));
    let sod = local.rem_euclid(86_400);
    let zone = if t.utc {
        "Z".to_owned()
    } else {
        let o = t.utc_offset;
        let a = o.abs();
        format!(
            "{}{:02}:{:02}",
            if o < 0 { '-' } else { '+' },
            a / 3600,
            a % 3600 / 60
        )
    };
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}{zone}",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

/// The path separators `File.basename` honours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// `/` only (Ruby on Linux and macOS).
    Posix,
    /// `/` and `\` (Ruby's `File::ALT_SEPARATOR` on Windows). Not
    /// oracle-captured: no Windows Ruby run backs this variant yet.
    Windows,
}

impl PathStyle {
    /// The style of the running host, as Ruby would pick it.
    pub fn host() -> Self {
        if cfg!(windows) {
            PathStyle::Windows
        } else {
            PathStyle::Posix
        }
    }

    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == PathStyle::Windows && c == '\\')
    }
}

/// `File.basename(path)`.
pub fn basename(path: &str, style: PathStyle) -> String {
    if path.is_empty() {
        return String::new();
    }
    let trimmed = path.trim_end_matches(|c| style.is_sep(c));
    if trimmed.is_empty() {
        // Only separators: Ruby keeps one.
        return path[..path.chars().next().map_or(0, char::len_utf8)].to_owned();
    }
    match trimmed.rfind(|c| style.is_sep(c)) {
        Some(i) => trimmed[i + 1..].to_owned(),
        None => trimmed.to_owned(),
    }
}

/// `string.split(/\r?\n/)`: trailing empty fields are dropped.
pub fn split_lines(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('\n') {
        let piece = &rest[..i];
        out.push(piece.strip_suffix('\r').unwrap_or(piece).to_owned());
        rest = &rest[i + 1..];
    }
    out.push(rest.to_owned());
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}
