//! Oracle parity for `ukiryu::types` (Ukiryu::Type and Ukiryu::Validation).
//!
//! Every expectation under `tests/types_corpus/` was captured by running the
//! pinned Ruby implementation (ukiryu/ukiryu ab6df353, Ruby 4.0.1, TZ=UTC);
//! see `tests/types_corpus/README.md`. Rows are compared on result type,
//! value, error class and exact message. A row the port answers with
//! `Unsupported` is a documented gap and must be listed in
//! `tests/types_corpus/gaps.txt`; an unlisted gap, a listed row that now
//! passes, or any mismatch fails the test.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use num_bigint::BigInt;
use serde_json::{json, Value as Json};
use ukiryu::types::constraints::Constraint;
use ukiryu::types::ruby;
use ukiryu::types::{
    build_constraints, valid_type, validate, Error, ErrorClass, Host, Regexp, Time, Validator,
    Value,
};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/types_corpus")
}

fn rows(file: &str) -> Vec<Json> {
    let text = std::fs::read_to_string(corpus_dir().join(file)).expect("corpus file");
    text.lines()
        .map(|l| serde_json::from_str(l).expect("JSON row"))
        .collect()
}

fn gaps() -> BTreeSet<String> {
    let text = std::fs::read_to_string(corpus_dir().join("gaps.txt")).expect("gaps.txt");
    text.lines()
        .map(|l| l.split('#').next().unwrap().trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

// ------------------------------------------------------------------ codec

fn parse_float(s: &str) -> f64 {
    match s {
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        other => f64::from_str(other).expect("Ruby float text"),
    }
}

fn parse_time(iso: &str) -> Time {
    // YYYY-MM-DDTHH:MM:SS.fffffffffZ
    let y: i64 = iso[0..4].parse().unwrap();
    let mo: u32 = iso[5..7].parse().unwrap();
    let d: u32 = iso[8..10].parse().unwrap();
    let h: i64 = iso[11..13].parse().unwrap();
    let mi: i64 = iso[14..16].parse().unwrap();
    let s: i64 = iso[17..19].parse().unwrap();
    let nanos: u32 = iso[20..29].parse().unwrap();
    let days = days_from_civil(y, mo, d);
    Time {
        seconds: days * 86_400 + h * 3600 + mi * 60 + s,
        nanos,
        utc_offset: 0,
        utc: true,
    }
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn decode(j: &Json) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Integer(BigInt::from_str(&n.to_string()).expect("integer")),
        Json::String(s) => Value::String(s.clone()),
        Json::Array(items) => Value::Array(items.iter().map(decode).collect()),
        Json::Object(map) => {
            assert_eq!(map.len(), 1, "tagged value expected: {j}");
            let (tag, payload) = map.iter().next().unwrap();
            match tag.as_str() {
                "$sym" => Value::Symbol(payload.as_str().unwrap().into()),
                "$float" => Value::Float(parse_float(payload.as_str().unwrap())),
                "$time" => Value::Time(parse_time(payload.as_str().unwrap())),
                "$re" => Value::Regexp(Regexp {
                    source: payload[0].as_str().unwrap().into(),
                    options: payload[1].as_u64().unwrap() as u32,
                }),
                "$map" => Value::Hash(
                    payload
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| (decode(&p[0]), decode(&p[1])))
                        .collect(),
                ),
                other => panic!("unknown tag {other}"),
            }
        }
    }
}

fn encode(v: &Value) -> Json {
    match v {
        Value::Nil => Json::Null,
        Value::Bool(b) => json!(b),
        Value::Integer(i) => serde_json::from_str(&i.to_string()).unwrap(),
        Value::Float(f) => json!({ "$float": ruby::float_to_s(*f) }),
        Value::String(s) => json!(s),
        Value::Symbol(s) => json!({ "$sym": s }),
        Value::Time(t) => {
            let (y, mo, d) = civil_from_days(t.seconds.div_euclid(86_400));
            let sod = t.seconds.rem_euclid(86_400);
            json!({ "$time": format!(
                "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:09}Z",
                sod / 3600, sod % 3600 / 60, sod % 60, t.nanos
            ) })
        }
        Value::Regexp(r) => json!({ "$re": [r.source, r.options] }),
        Value::Array(items) => Json::Array(items.iter().map(encode).collect()),
        Value::Hash(pairs) => json!({
            "$map": pairs.iter().map(|(k, x)| json!([encode(k), encode(x)])).collect::<Vec<_>>()
        }),
    }
}

fn encode_error(e: &Error) -> Json {
    let mut out = json!({ "class": e.class.ruby_name(), "message": e.message });
    if let Some(a) = &e.attribute {
        out["attribute"] = encode(a);
        out["validation_type"] = json!({ "$sym": e.validation_type.unwrap() });
    }
    out
}

fn outcome(r: &Result<Value, Error>) -> Json {
    match r {
        Ok(v) => {
            let mut out = json!({ "ok": encode(v) });
            if let Value::Time(t) = v {
                out["time"] = json!({
                    "utc_offset": t.utc_offset,
                    "inspect": ruby::time_inspect(t),
                    "to_s": ruby::time_to_s(t),
                    "utc": t.utc,
                });
            }
            out
        }
        Err(e) => json!({ "error": encode_error(e) }),
    }
}

// ------------------------------------------------------------------ hosts

/// The oracle environment: the real filesystem, TZ=UTC, no regexp engine.
struct OracleHost;

impl Host for OracleHost {
    fn file_exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn local_offset(&self, _: i64, _: u32, _: u32, _: u32, _: u32, _: u32) -> Option<i32> {
        Some(0)
    }
}

// ------------------------------------------------------------------ checks

#[derive(Default)]
struct Tally {
    checked: usize,
    seen: BTreeSet<String>,
    gaps: Vec<String>,
    failures: Vec<String>,
}

impl Tally {
    fn record(&mut self, id: &str, expected: &Json, actual: &Json, unsupported: bool) {
        self.checked += 1;
        self.seen.insert(id.to_string());
        if unsupported {
            self.gaps.push(id.to_string());
        } else if expected != actual {
            self.failures
                .push(format!("{id}\n  expected {expected}\n  actual   {actual}"));
        }
    }

    fn finish(self, name: &str) {
        let listed = gaps();
        let prefix_rows: BTreeSet<String> = self.gaps.iter().cloned().collect();
        let mut problems = self.failures;
        for g in &self.gaps {
            if !listed.contains(g) {
                problems.push(format!("{g}: Unsupported but not listed in gaps.txt"));
            }
        }
        for g in &listed {
            if self.seen.contains(g) && !prefix_rows.contains(g) {
                problems.push(format!("{g}: listed in gaps.txt but no longer Unsupported"));
            }
        }
        println!(
            "{name}: {} rows, {} documented gaps, {} problems",
            self.checked,
            prefix_rows.len(),
            problems.len()
        );
        assert!(problems.is_empty(), "{name}:\n{}", problems.join("\n"));
    }
}

fn is_unsupported(r: &Result<Value, Error>) -> bool {
    matches!(r, Err(e) if e.class == ErrorClass::Unsupported)
}

#[test]
fn type_validate_matrix() {
    let mut tally = Tally::default();
    for row in rows("type_validate.jsonl") {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        let r = validate(
            &decode(&input["value"]),
            &decode(&input["type"]),
            &decode(&input["options"]),
            &OracleHost,
        );
        tally.record(id, &row["expect"], &outcome(&r), is_unsupported(&r));
    }
    tally.finish("type_validate");
}

#[test]
fn type_valid_type_matrix() {
    let mut tally = Tally::default();
    for row in rows("type_valid_type.jsonl") {
        let id = row["id"].as_str().unwrap();
        let r = Ok(Value::Bool(valid_type(&decode(&row["input"]["type"]))));
        tally.record(id, &row["expect"], &outcome(&r), false);
    }
    tally.finish("type_valid_type");
}

fn current_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

#[test]
fn type_file_real_filesystem() {
    let dir = std::env::temp_dir().join(format!("ukiryu-types-fs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut tally = Tally::default();
    let mut skipped = 0;
    let all = rows("type_file_fs.jsonl");
    let fixture = &all[0]["input"]["fixture"];
    for f in fixture["files"].as_array().unwrap() {
        std::fs::write(dir.join(f.as_str().unwrap()), "x").unwrap();
    }
    for d in fixture["dirs"].as_array().unwrap() {
        std::fs::create_dir(dir.join(d.as_str().unwrap())).unwrap();
    }
    #[cfg(unix)]
    for (link, target) in fixture["symlinks"].as_object().unwrap() {
        std::os::unix::fs::symlink(dir.join(target.as_str().unwrap()), dir.join(link)).unwrap();
    }
    let root = dir.to_str().unwrap().to_string();
    for row in &all {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        let platforms: Vec<&str> = input["platforms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        if !platforms.contains(&current_platform()) {
            skipped += 1;
            continue;
        }
        let path = input["value"].as_str().unwrap().replace("${TMP}", &root);
        let r = validate(
            &Value::String(path),
            &decode(&input["type"]),
            &decode(&input["options"]),
            &OracleHost,
        );
        let actual: Json =
            serde_json::from_str(&outcome(&r).to_string().replace(&root, "${TMP}")).unwrap();
        tally.record(id, &row["expect"], &actual, is_unsupported(&r));
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("type_file_fs: {skipped} rows not applicable on {}", current_platform());
    tally.finish("type_file_fs");
}

/// Production callers pass `ArgumentDefinition`/`OptionDefinition` models as
/// the options; `Model#[]` returns the attribute or nil. None of the keys
/// `Type` reads has a model default, so until the models module is wired in
/// these rows run with a Hash holding exactly the assigned attributes.
#[test]
fn type_model_shaped_options() {
    let mut tally = Tally::default();
    for row in rows("type_model_options.jsonl") {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        let attrs = match decode(&input["attrs"]) {
            Value::Hash(pairs) => Value::Hash(
                pairs
                    .into_iter()
                    .map(|(k, v)| (Value::Symbol(ruby::to_s(&k)), v))
                    .collect(),
            ),
            other => panic!("attrs {other:?}"),
        };
        let r = validate(
            &decode(&input["value"]),
            &decode(&input["type"]),
            &attrs,
            &OracleHost,
        );
        tally.record(id, &row["expect"], &outcome(&r), is_unsupported(&r));
    }
    tally.finish("type_model_options");
}

fn ctor_get(ctor: &Value, key: &str) -> Value {
    ctor.sym_get(key)
}

fn build_constraint(kind: &str, ctor: &Value) -> Constraint {
    let name = ctor_get(ctor, "name");
    match kind {
        "required" => Constraint::Required {
            attribute: name,
            min: if ctor.sym_key("min") {
                ctor_get(ctor, "min")
            } else {
                Value::int(1)
            },
        },
        "range" => Constraint::Range {
            attribute: name,
            min: ctor_get(ctor, "min"),
            max: ctor_get(ctor, "max"),
        },
        "enum" => Constraint::Enum {
            attribute: name,
            allowed_values: ctor_get(ctor, "allowed_values"),
        },
        "type" => Constraint::Type {
            attribute: name,
            type_spec: ctor_get(ctor, "type"),
            options: match ctor_get(ctor, "options") {
                Value::Nil => Value::Hash(vec![]),
                o => o,
            },
        },
        "cardinality" => Constraint::Cardinality {
            attribute: name,
            min: ctor_get(ctor, "min"),
            max: ctor_get(ctor, "max"),
        },
        "boolean_flag" => Constraint::BooleanFlag { attribute: name },
        "dependency" => Constraint::Dependency {
            option: name,
            requires: ctor_get(ctor, "requires"),
            conflicts: ctor_get(ctor, "conflicts"),
            implies: ctor_get(ctor, "implies"),
        },
        other => panic!("unknown constraint {other}"),
    }
}

#[test]
fn constraint_matrix() {
    let mut tally = Tally::default();
    for row in rows("constraints.jsonl") {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        let c = build_constraint(input["constraint"].as_str().unwrap(), &decode(&input["ctor"]));
        let ctx = input.get("context_values").map(decode);
        let accessor = |name: &Value| -> Result<Value, Error> {
            Ok(ctx.as_ref().unwrap().hash_get(name).cloned().unwrap_or(Value::Nil))
        };
        let acc: Option<&dyn Fn(&Value) -> Result<Value, Error>> =
            if ctx.is_some() { Some(&accessor) } else { None };
        let r = c
            .validate(&decode(&input["value"]), acc, &OracleHost)
            .map(|()| Value::sym("passed"));
        tally.record(id, &row["expect"], &outcome(&r), is_unsupported(&r));
    }
    tally.finish("constraints");
}

#[test]
fn validator_matrix() {
    let mut tally = Tally::default();
    let mut not_applicable = vec![];
    for row in rows("validator.jsonl") {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        let Some(cmd) = input.get("command_def") else {
            // Source-only rows: a Models::CommandDefinition (the Rust
            // validator takes the Hash shape the Ruby code indexes) and the
            // module-loading finding.
            not_applicable.push(id.to_string());
            continue;
        };
        let cmd = decode(cmd);
        let ivars: Vec<(String, Value)> = match decode(&input["ivars"]) {
            Value::Hash(pairs) => pairs
                .into_iter()
                .map(|(k, v)| (ruby::to_s(&k), v))
                .collect(),
            other => panic!("ivars {other:?}"),
        };
        let expect = &row["expect"];
        let built = build_constraints(&cmd).map(|cs| {
            Value::Array(
                cs.iter()
                    .map(|c| Value::String(c.class_name().into()))
                    .collect(),
            )
        });
        let mut actual = json!({ "constraints": outcome(&built) });
        let mut unsupported = is_unsupported(&built);
        if built.is_ok() {
            let v = Validator::with_host(&ivars, &cmd, &OracleHost).unwrap();
            let validate = v.validate().map(Value::Bool);
            let valid = v.is_valid().map(Value::Bool);
            let errors = v
                .errors()
                .map(|es| Value::Array(es.into_iter().map(Value::String).collect()));
            unsupported |= is_unsupported(&validate) || is_unsupported(&valid) || is_unsupported(&errors);
            actual["validate"] = outcome(&validate);
            actual["valid"] = outcome(&valid);
            actual["errors"] = outcome(&errors);
        }
        tally.record(id, expect, &actual, unsupported);
    }
    assert_eq!(not_applicable, ["validator.037", "validator.reachability"]);
    tally.finish("validator");
}

#[test]
fn ruby_formatting_matrix() {
    let mut tally = Tally::default();
    for row in rows("ruby_format.jsonl") {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        if id == "ruby.nonprintable_ranges" {
            continue; // compiled into src/types/nonprintable.rs; checked below
        }
        let op = input["op"].as_str().unwrap();
        let value = match &input["value"] {
            Json::String(s) if op != "inspect" => Value::String(s.clone()),
            other => {
                let mut v = decode(other);
                if let (Value::Time(t), Some(detail)) = (&mut v, input.get("time")) {
                    t.utc_offset = detail["utc_offset"].as_i64().unwrap() as i32;
                    t.utc = detail["utc"].as_bool().unwrap();
                }
                v
            }
        };
        let text = |s: String| Ok(Value::String(s));
        let r = match (op, &value) {
            ("float_to_s", Value::Float(f)) => text(ruby::float_to_s(*f)),
            ("inspect", v) => text(ruby::inspect(v)),
            ("downcase", Value::String(s)) => text(ruby::downcase(s)),
            ("dump", Value::String(s)) => text(ruby::dump(s)),
            other => panic!("unknown op {other:?}"),
        };
        let mut actual = outcome(&r);
        if row["expect"].get("to_s").is_some() {
            actual["to_s"] = json!(ruby::to_s(&value));
        }
        tally.record(id, &row["expect"], &actual, false);
    }
    tally.finish("ruby_format");
}

#[test]
fn nonprintable_table_matches_capture() {
    let row = rows("ruby_format.jsonl")
        .into_iter()
        .find(|r| r["id"] == "ruby.nonprintable_ranges")
        .unwrap();
    let ranges = row["expect"]["ok"].as_array().unwrap();
    let mut by_cp = BTreeMap::new();
    for r in ranges {
        by_cp.insert(r[0].as_u64().unwrap(), r[1].as_u64().unwrap());
    }
    // Spot-check every range boundary through String#inspect.
    for (&lo, &hi) in &by_cp {
        for cp in [lo, hi] {
            if let Some(c) = char::from_u32(cp as u32) {
                let s = c.to_string();
                assert_ne!(ruby::string_inspect(&s), format!("\"{s}\""), "U+{cp:X} must be escaped");
            }
        }
        for cp in [lo.wrapping_sub(1), hi + 1] {
            if let Some(c) = char::from_u32(cp as u32) {
                if by_cp.iter().any(|(&a, &b)| a <= cp && cp <= b) {
                    continue;
                }
                let s = c.to_string();
                if !matches!(c, '"' | '\\' | '\n' | '\r' | '\t' | '\x0C' | '\x0B' | '\x08' | '\x07' | '\x1B' | '#') {
                    assert_eq!(ruby::string_inspect(&s), format!("\"{s}\""), "U+{cp:X} is printable");
                }
            }
        }
    }
}

#[test]
fn every_gap_is_a_real_row() {
    let mut ids = BTreeSet::new();
    for f in [
        "type_validate.jsonl",
        "type_valid_type.jsonl",
        "type_file_fs.jsonl",
        "type_model_options.jsonl",
        "constraints.jsonl",
        "validator.jsonl",
        "ruby_format.jsonl",
    ] {
        for r in rows(f) {
            ids.insert(r["id"].as_str().unwrap().to_string());
        }
    }
    for g in gaps() {
        assert!(ids.contains(&g), "gaps.txt lists unknown row {g}");
    }
}
