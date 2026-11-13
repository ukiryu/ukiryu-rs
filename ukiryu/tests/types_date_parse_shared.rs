//! The shared `Date._parse` port (`ukiryu::types::date_parse`, used by
//! `Time.parse` and by the man-page `Date.parse`) against `Date._parse(str)`
//! captured from Ruby 4.0.1 with date 3.5.1 and the corpus oracle
//! Gemfile.lock fc3b12f2 (`src/extractors/man/testdata/oracle.json`, see its
//! `provenance`). Every key except `offset` is compared, so the Date-only
//! `wday`/`cwyear`/`cweek`/`cwday` keys, the raw `zone` text and non-ASCII
//! inputs are checked as Ruby returns them.

use num_bigint::BigInt;
use serde_json::Value;
use std::collections::BTreeMap;
use ukiryu::extractors::man::{date_parse_fragments, DateFailure, Fragments};
use ukiryu::types::date_parse::{parse, Parsed};
use ukiryu::types::error::ErrorClass;

const ORACLE: &str = include_str!("../src/extractors/man/testdata/oracle.json");

fn cases() -> Vec<Value> {
    let o: Value = serde_json::from_str(ORACLE).expect("oracle json");
    assert_eq!(o["provenance"]["date_gem"], "3.5.1");
    o["date_cases"].as_array().expect("cases").clone()
}

fn int_keys(p: &Parsed) -> [(&'static str, &Option<BigInt>); 11] {
    [
        ("year", &p.year),
        ("mon", &p.mon),
        ("mday", &p.mday),
        ("yday", &p.yday),
        ("wday", &p.wday),
        ("cwyear", &p.cwyear),
        ("cweek", &p.cweek),
        ("cwday", &p.cwday),
        ("hour", &p.hour),
        ("min", &p.min),
        ("sec", &p.sec),
    ]
}

/// The oracle's fragment object without `sec_fraction`, plus the parsed
/// `sec_fraction` checked as an exact rational.
fn compare(p: &Parsed, want: &Value) -> Result<(), String> {
    let want = want.as_object().expect("fragments object");
    let mut got = BTreeMap::new();
    for (k, v) in int_keys(p) {
        if let Some(v) = v {
            got.insert(k.to_string(), Value::String(v.to_string()));
        }
    }
    if let Some(z) = &p.zone {
        got.insert("zone".into(), Value::String(z.clone()));
    }
    let mut want_rest = want.clone();
    let want_frac = want_rest.remove("sec_fraction");
    match (&p.sec_fraction, want_frac) {
        (None, None) => {}
        (Some(f), Some(Value::String(r))) => {
            let (n, d) = r.split_once('/').expect("n/d");
            let n: BigInt = n.parse().expect("numerator");
            let d: BigInt = d.parse().expect("denominator");
            let scale = BigInt::from(10).pow(u32::try_from(f.digits).expect("digits"));
            if &f.numerator * &d != n * scale {
                return Err(format!("sec_fraction {}e-{} != {r}", f.numerator, f.digits));
            }
        }
        (got_f, want_f) => return Err(format!("sec_fraction {got_f:?} != {want_f:?}")),
    }
    let got = Value::Object(got.into_iter().collect());
    if got != Value::Object(want_rest) {
        return Err(format!("{got} != {}", Value::Object(want.clone())));
    }
    Ok(())
}

#[test]
fn shared_parser_matches_every_captured_fragment() {
    let mut fails = Vec::new();
    let (mut ok, mut limit, mut range) = (0, 0, 0);
    for c in cases() {
        let s = c["date_str"].as_str().expect("str");
        let want = &c["fragments"];
        if let Some(w) = want.get("ok") {
            ok += 1;
            match parse(s) {
                Ok(p) => {
                    if let Err(e) = compare(&p, w) {
                        fails.push(format!("{s:?}: {e}"));
                    }
                }
                Err(e) => fails.push(format!("{s:?}: unexpected {e}")),
            }
            continue;
        }
        let class = want["error"]["class"].as_str().expect("class");
        let message = want["error"]["message"].as_str().expect("message");
        match class {
            // The 128-byte limit is checked by the callers, before `parse`.
            "ArgumentError" => {
                limit += 1;
                assert!(s.len() > 128, "{s:?}");
                assert_eq!(
                    message,
                    format!("string length ({}) exceeds the limit 128", s.len())
                );
            }
            "RangeError" => {
                range += 1;
                match parse(s) {
                    Err(e) if e.class == ErrorClass::Range && e.message == message => {}
                    other => fails.push(format!("{s:?}: want RangeError {message}, got {other:?}")),
                }
            }
            other => panic!("unexpected oracle class {other}"),
        }
    }
    assert!(
        fails.is_empty(),
        "{} mismatches:\n{}",
        fails.len(),
        fails.join("\n")
    );
    assert_eq!((ok, limit, range), (257, 19, 2));
}

/// The captured cases keep the Date-only keys and non-ASCII inputs that a
/// truncated `Time.parse` view would lose; counts measured from the data.
#[test]
fn captured_cases_cover_week_unicode_and_zone_keys() {
    let mut count = BTreeMap::new();
    for c in cases() {
        let s = c["date_str"].as_str().expect("str");
        if !s.is_ascii() {
            *count.entry("non_ascii").or_insert(0) += 1;
        }
        let Some(f) = c["fragments"].get("ok") else {
            continue;
        };
        for k in ["wday", "cwyear", "cweek", "cwday", "zone", "sec_fraction"] {
            if f.get(k).is_some() {
                *count.entry(k).or_insert(0) += 1;
            }
        }
    }
    let want: BTreeMap<&str, i32> = [
        ("non_ascii", 22),
        ("wday", 17),
        ("cwyear", 18),
        ("cweek", 24),
        ("cwday", 18),
        ("zone", 23),
        ("sec_fraction", 5),
    ]
    .into_iter()
    .collect();
    assert_eq!(count, want);
}

fn same_keys(f: &Fragments, p: &Parsed) -> bool {
    f.year == p.year
        && f.mon == p.mon
        && f.mday == p.mday
        && f.yday == p.yday
        && f.wday == p.wday
        && f.cwyear == p.cwyear
        && f.cweek == p.cweek
        && f.cwday == p.cwday
        && f.hour == p.hour
        && f.min == p.min
        && f.sec == p.sec
        && f.zone == p.zone
        && f.sec_fraction.is_some() == p.sec_fraction.is_some()
}

/// The man-page `Date._parse` is the shared parser behind the 128-byte limit:
/// one implementation, the same keys and the same errors.
#[test]
fn man_fragments_are_the_shared_parse() {
    for c in cases() {
        let s = c["date_str"].as_str().expect("str");
        match (date_parse_fragments(s), parse(s)) {
            (Ok(f), Ok(p)) => assert!(same_keys(&f, &p), "{s:?}"),
            (Err(DateFailure::Range(m)), Err(e)) => {
                assert_eq!(e.class, ErrorClass::Range, "{s:?}");
                assert_eq!(m, e.message, "{s:?}");
            }
            (Err(DateFailure::Argument(m)), _) => {
                assert!(s.len() > 128, "{s:?}: {m}");
            }
            (f, p) => panic!("{s:?}: man {f:?} shared {p:?}"),
        }
    }
}
