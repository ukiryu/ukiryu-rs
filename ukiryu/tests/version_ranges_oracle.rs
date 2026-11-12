//! Oracle parity for `ukiryu::version_ranges`.
//!
//! version_ranges_oracle/golden.json is generated, not hand-written: capture.rb runs the original
//! ukiryu at ab6df353 (Ruby 4.0.1, rubygems 4.0.3, versionian 0.1.0 from the
//! locked bundle) and records each outcome; pack.py interns repeated specs.
//! Regenerate with the K18-RANGES capture scripts; never edit the data by hand.
use num_bigint::BigInt;
use serde_json::Value;
use ukiryu::version_ranges::{
    find_matching_version_spec, is_inline, is_reference, resolve, Bounds, ErrorClass, RangeError,
    RangeKind, Scheme, SpecValue, VersionRange,
};

/// Rows the port reports as `Unsupported` (inline pattern and composite schemes).
const EXPECTED_UNSUPPORTED: usize = 2;

fn decode(v: &Value) -> SpecValue {
    match v {
        Value::Null => SpecValue::Nil,
        Value::Bool(b) => SpecValue::Bool(*b),
        Value::Number(n) => SpecValue::Int(BigInt::from(n.as_i64().expect("integer"))),
        Value::String(s) => SpecValue::Str(s.clone()),
        Value::Array(a) => SpecValue::Array(a.iter().map(decode).collect()),
        Value::Object(o) => {
            let (tag, inner) = o.iter().next().expect("tagged value");
            match tag.as_str() {
                "$sym" => SpecValue::Sym(inner.as_str().unwrap().into()),
                "$float" => SpecValue::Float(inner.as_str().unwrap().parse().unwrap()),
                "$int" => SpecValue::Int(inner.as_str().unwrap().parse().unwrap()),
                "$map" => SpecValue::Map(
                    inner
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|kv| (decode(&kv[0]), decode(&kv[1])))
                        .collect(),
                ),
                other => panic!("unknown tag {other}"),
            }
        }
    }
}

fn sym_or_nil(s: Option<&str>) -> SpecValue {
    s.map_or(SpecValue::Nil, |s| SpecValue::Sym(s.into()))
}

fn meta(s: &Scheme) -> SpecValue {
    let mut m = vec![
        (
            SpecValue::Str("class".into()),
            SpecValue::Str(s.class_name().into()),
        ),
        (SpecValue::Str("name".into()), sym_or_nil(s.name())),
        (
            SpecValue::Str("description".into()),
            s.description().clone(),
        ),
    ];
    if let Some(f) = s.format() {
        m.push((SpecValue::Str("format".into()), SpecValue::Str(f.into())));
    }
    SpecValue::Map(m)
}

fn bounds(v: &SpecValue) -> Bounds {
    let SpecValue::Map(pairs) = v else {
        panic!("bounds map")
    };
    pairs.iter().fold(Bounds::new(), |b, (k, v)| match k {
        SpecValue::Sym(k) => b.with(k, v.clone()),
        _ => panic!("keyword bounds"),
    })
}

fn run(row: &[Value], specs: &[SpecValue]) -> (Result<SpecValue, RangeError>, usize) {
    let spec = &specs[row[1].as_u64().unwrap() as usize];
    let exp_ix = match row[0].as_str().unwrap() {
        "range" => 5,
        "valid" => 3,
        "version_spec" => 4,
        _ => 2,
    };
    let got = match row[0].as_str().unwrap() {
        "range" => resolve(spec).and_then(|scheme| {
            let kind = RangeKind::from_name(row[2].as_str().unwrap())?;
            VersionRange::new(kind, &scheme, bounds(&decode(&row[3])))?
                .matches(row[4].as_str().unwrap())
                .map(SpecValue::Bool)
        }),
        "valid" => resolve(spec)
            .and_then(|s| s.is_valid(row[2].as_str().unwrap()))
            .map(SpecValue::Bool),
        "resolve" => resolve(spec).map(|s| meta(&s)),
        "inline?" => Ok(SpecValue::Bool(is_inline(spec))),
        "reference?" => Ok(SpecValue::Bool(is_reference(spec))),
        "version_spec" => {
            let SpecValue::Array(versions) = &specs[row[2].as_u64().unwrap() as usize] else {
                panic!("versions array")
            };
            resolve(spec).and_then(|scheme| {
                find_matching_version_spec(versions, row[3].as_str().unwrap(), &scheme)
                    .map(|ix| ix.map_or(SpecValue::Nil, |i| SpecValue::Int(BigInt::from(i))))
            })
        }
        other => panic!("unknown op {other}"),
    };
    (got, exp_ix)
}

#[test]
fn version_ranges_match_the_ruby_oracle() {
    let golden: Value = serde_json::from_str(GOLDEN).expect("golden json");
    let specs: Vec<SpecValue> = golden["specs"]
        .as_array()
        .unwrap()
        .iter()
        .map(decode)
        .collect();
    let rows = golden["rows"].as_array().unwrap();
    let (mut pass, mut unsupported, mut failures) = (0usize, Vec::new(), Vec::new());
    for row in rows {
        let row = row.as_array().unwrap();
        let (got, ix) = run(row, &specs);
        let expect = &row[ix];
        let ok = match (&got, expect) {
            (Err(e), _) if e.class == ErrorClass::Unsupported => {
                unsupported.push(format!("{row:?} -> {e}"));
                continue;
            }
            (Ok(v), Value::Object(o)) => o.get("ok").map(decode).as_ref() == Some(v),
            (Err(e), Value::Array(pair)) => {
                pair[0].as_str() == Some(e.class.ruby_name())
                    && pair[1].as_str() == Some(&e.message)
            }
            _ => false,
        };
        if ok {
            pass += 1;
        } else {
            failures.push(format!("{row:?}\n    got {got:?}"));
        }
    }
    for f in failures.iter().take(40) {
        eprintln!("MISMATCH {f}");
    }
    for u in &unsupported {
        eprintln!("UNSUPPORTED {u}");
    }
    eprintln!(
        "rows={} pass={pass} mismatch={} unsupported={}",
        rows.len(),
        failures.len(),
        unsupported.len()
    );
    assert!(failures.is_empty(), "{} oracle mismatches", failures.len());
    assert_eq!(unsupported.len(), EXPECTED_UNSUPPORTED);
    assert_eq!(pass + unsupported.len(), rows.len());
}

/// Generated by capture.rb + pack.py; do not edit.
const GOLDEN: &str = include_str!("version_ranges_oracle/golden.json");
