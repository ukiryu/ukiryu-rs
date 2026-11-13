//! Exact Ruby `Time` values, including fractions finer than a nanosecond.
//!
//! Every expectation in `tests/types_corpus/time_exact.jsonl` was captured
//! from Ruby 4.0.1 with the pinned ukiryu/ukiryu ab6df353 under `TZ=UTC`
//! (see `tests/types_corpus/README.md`). Integers are decimal strings and
//! Floats are compared by their IEEE bits.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::str::FromStr;

use num_bigint::BigInt;
use serde_json::Value as Json;
use ukiryu::types::ruby;
use ukiryu::types::value::{eql, ruby_eq};
use ukiryu::types::{validate, Host, SubNanos, Time, Value};

fn rows() -> Vec<Json> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/types_corpus/time_exact.jsonl");
    std::fs::read_to_string(path)
        .expect("time_exact.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).expect("JSON row"))
        .collect()
}

fn big(j: &Json) -> BigInt {
    BigInt::from_str(j.as_str().expect("decimal string")).expect("integer")
}

fn pair(j: &Json) -> (BigInt, BigInt) {
    (big(&j[0]), big(&j[1]))
}

fn time(spec: &Json) -> Time {
    let utc = spec.get("utc").and_then(Json::as_bool).unwrap_or(false);
    let offset = spec.get("utc_offset").and_then(Json::as_i64).unwrap_or(0) as i32;
    let (n, d) = pair(&spec["r"]);
    Time::from_rational(&n, &d, offset, utc).expect("time in range")
}

fn bits(f: f64) -> String {
    format!("{:016x}", f.to_bits())
}

fn hash_of(t: &Time) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// `TZ=UTC`, as the capture ran.
struct UtcHost;

impl Host for UtcHost {
    fn file_exists(&self, _: &str) -> bool {
        false
    }

    fn local_offset(&self, _: i64, _: u32, _: u32, _: u32, _: u32, _: u32) -> Option<i32> {
        Some(0)
    }

    fn local_zone_abbreviation(&self, _: i64, _: u32, _: u32) -> Option<String> {
        Some("UTC".into())
    }
}

fn check_time(id: &str, t: &Time, e: &Json) {
    assert_eq!(t.to_r(), pair(&e["to_r"]), "{id} to_r");
    assert_eq!(t.subsec(), pair(&e["subsec"]), "{id} subsec");
    assert_eq!(BigInt::from(t.to_i()), big(&e["to_i"]), "{id} to_i");
    assert_eq!(
        u64::from(t.nsec()),
        e["nsec"].as_u64().unwrap(),
        "{id} nsec"
    );
    assert_eq!(
        u64::from(t.usec()),
        e["usec"].as_u64().unwrap(),
        "{id} usec"
    );
    assert_eq!(bits(t.to_f()), e["to_f"]["bits"], "{id} to_f");
    assert_eq!(
        ruby::float_to_s(t.to_f()),
        e["to_f"]["to_s"],
        "{id} to_f text"
    );
    assert_eq!(ruby::time_inspect(t), e["inspect"], "{id} inspect");
    assert_eq!(ruby::time_to_s(t), e["to_s"], "{id} to_s");
    assert_eq!(
        i64::from(t.utc_offset),
        e["utc_offset"].as_i64().unwrap(),
        "{id} utc_offset"
    );
    assert_eq!(t.utc, e["utc"].as_bool().unwrap(), "{id} utc");
    let v = Value::Time(t.clone());
    assert_eq!(v.inspect(), e["inspect"], "{id} Value#inspect");
    let json = serde_json::to_string(&Value::Array(vec![v.clone()])).unwrap();
    assert_eq!(json, e["json"], "{id} JSON.generate");

    let options = Value::Hash(vec![]);
    match validate(&v, &Value::sym("float"), &options, &UtcHost) {
        Ok(Value::Float(f)) => assert_eq!(bits(f), e["validate_float"]["ok"]["bits"], "{id}"),
        other => panic!("{id}: validate(:float) gave {other:?}"),
    }
    match validate(&v, &Value::sym("integer"), &options, &UtcHost) {
        Ok(Value::Integer(i)) => assert_eq!(i.to_string(), e["validate_integer"]["ok"], "{id}"),
        other => panic!("{id}: validate(:integer) gave {other:?}"),
    }

    // The exact value survives every conversion helper.
    let (n, d) = t.to_r();
    let back = Time::from_rational(&n, &d, t.utc_offset, t.utc).unwrap();
    assert_eq!(&back, t, "{id} from_rational(to_r)");
    let (n, d) = t.unix_nanos();
    let back = Time::from_unix_nanos(&n, &d, t.utc_offset, t.utc).unwrap();
    assert_eq!(&back, t, "{id} from_unix_nanos(unix_nanos)");
    assert_eq!(t.is_integral_nanos(), t.subnanos.is_zero(), "{id}");
    let sub = SubNanos::new(t.subnanos.numer(), t.subnanos.denom()).unwrap();
    assert_eq!(sub, t.subnanos, "{id} SubNanos round trip");
}

#[test]
fn exact_time_matrix() {
    let (mut times, mut pairs, mut parses) = (0, 0, 0);
    for row in rows() {
        let id = row["id"].as_str().unwrap();
        let e = &row["expect"];
        match row["op"].as_str().unwrap() {
            "time" => {
                check_time(id, &time(&row["input"]["time"]), e);
                times += 1;
            }
            "pair" => {
                let a = time(&row["input"]["a"]);
                let b = time(&row["input"]["b"]);
                let cmp = a.instant_cmp(&b) as i64;
                assert_eq!(cmp, e["cmp"].as_i64().unwrap(), "{id} <=>");
                let (va, vb) = (Value::Time(a.clone()), Value::Time(b.clone()));
                assert_eq!(ruby_eq(&va, &vb), e["eq"].as_bool().unwrap(), "{id} ==");
                assert_eq!(eql(&va, &vb), e["eql"].as_bool().unwrap(), "{id} eql?");
                assert_eq!(a.instant_eql(&b), e["eql"].as_bool().unwrap(), "{id}");
                let same_hash = hash_of(&a) == hash_of(&b);
                assert_eq!(same_hash, e["hash_eq"].as_bool().unwrap(), "{id} hash");
                assert_eq!(ruby::lt(&va, &vb).unwrap(), cmp < 0, "{id} <");
                assert_eq!(ruby::gt(&va, &vb).unwrap(), cmp > 0, "{id} >");
                assert_eq!(bits(a.minus_to_f(&b)), e["sub"]["bits"], "{id} a - b");
                pairs += 1;
            }
            "parse" => {
                let text = row["input"]["value"].as_str().unwrap();
                let r = validate(
                    &Value::str(text),
                    &Value::sym("datetime"),
                    &Value::Hash(vec![]),
                    &UtcHost,
                );
                let ok = &e["ok"];
                let Ok(Value::Time(t)) = &r else {
                    panic!("{id}: {text:?} gave {r:?}, Ruby {e}");
                };
                assert_eq!(t.to_r(), pair(&ok["to_r"]), "{id} to_r");
                assert_eq!(
                    i64::from(t.utc_offset),
                    ok["utc_offset"].as_i64().unwrap(),
                    "{id}"
                );
                assert_eq!(t.utc, ok["utc"].as_bool().unwrap(), "{id} utc");
                assert_eq!(ruby::time_inspect(t), ok["inspect"], "{id} inspect");
                assert_eq!(ruby::time_to_s(t), ok["to_s"], "{id} to_s");
                parses += 1;
            }
            other => panic!("{id}: unknown op {other}"),
        }
    }
    println!("time_exact: {times} times, {pairs} pairs, {parses} parses");
    assert_eq!((times, pairs, parses), (23, 16, 14));
}

/// The integral-nanosecond constructor keeps the old field meaning and has
/// no fraction below a nanosecond.
#[test]
fn whole_nanosecond_times_are_unchanged() {
    let t = Time::new(1_705_486_830, 123_456_789, 19_800, false);
    assert_eq!(
        (t.seconds, t.nanos, t.utc_offset, t.utc),
        (1_705_486_830, 123_456_789, 19_800, false)
    );
    assert!(t.is_integral_nanos());
    assert_eq!(t.subnanos, SubNanos::ZERO);
    assert_eq!(
        Time::new(5, 1_500_000_000, 0, true),
        Time::new(6, 500_000_000, 0, true)
    );
    assert_eq!(SubNanos::new(BigInt::from(1), BigInt::from(1)), None);
    assert_eq!(SubNanos::new(BigInt::from(-1), BigInt::from(3)), None);
    assert_eq!(SubNanos::new(BigInt::from(1), BigInt::from(0)), None);
    assert_eq!(
        Time::from_rational(&BigInt::from(1), &BigInt::from(0), 0, true),
        None
    );
}
