//! Golden parity for `ukiryu::schema` against the original Ruby
//! implementation.
//!
//! `tests/schema_golden/cases.json` was produced by running the pinned
//! oracle (ukiryu/ukiryu ab6df353 with json-schema 6.2.1, see its
//! `provenance` block) over the inputs in `tests/schema_golden/inputs` and
//! the bundled schemas. Every case must match exactly, except the ones
//! listed in `DIVERGENCES`, which must diverge exactly as declared.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use ukiryu::schema::{
    self, definition_schema_report, load_definition_schema, safe_load, Permitted, SchemaError,
    ValidateOptions,
};

/// How a case is allowed to differ from the oracle.
#[derive(Clone, Copy, Debug)]
enum Divergence {
    /// Same Ruby error class; the message is the YAML engine's, not Psych's.
    YamlMessage,
    /// The port refuses the input (`SchemaError::Unsupported`) instead of
    /// emulating what the oracle does.
    Unsupported,
}

/// Every declared divergence, by case-id prefix.
const DIVERGENCES: &[(&str, Divergence, &str)] = &[
    (
        "bundled.tool_original.",
        Divergence::YamlMessage,
        "invalid original tool.schema.yaml: Psych and yeptris word the syntax error differently",
    ),
    (
        "load.invalid.",
        Divergence::YamlMessage,
        "invalid YAML schema: message wording",
    ),
    (
        "load.scalar.",
        Divergence::Unsupported,
        "string schema: the gem reads a file named by the string",
    ),
    (
        "dialect.draft3.",
        Divergence::Unsupported,
        "draft-03 dialect is not ported",
    ),
    (
        "dialect.with_id.",
        Divergence::Unsupported,
        "schema id rebasing is not ported",
    ),
    (
        "dialect.external_ref.",
        Divergence::Unsupported,
        "external $ref: the gem reads a file",
    ),
    (
        "dialect.pattern.present.",
        Divergence::Unsupported,
        "Ruby regular expressions are not ported",
    ),
    (
        "draft6.stamp-",
        Divergence::Unsupported,
        "format date-time (Ruby DateTime.rfc3339) is not ported",
    ),
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn golden() -> Value {
    let path = crate_dir().join("tests/schema_golden/cases.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden file")).expect("golden json")
}

fn abs(rel: &str) -> PathBuf {
    if rel.is_empty() {
        PathBuf::new()
    } else {
        crate_dir().join(rel)
    }
}

fn fill(s: &str) -> String {
    s.replace("${CRATE}", &crate_dir().to_string_lossy())
}

fn expected(case: &Value) -> Value {
    fn walk(v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(fill(s)),
            Value::Array(a) => Value::Array(a.iter().map(walk).collect()),
            Value::Object(o) => {
                Value::Object(o.iter().map(|(k, v)| (k.clone(), walk(v))).collect())
            }
            other => other.clone(),
        }
    }
    walk(&case["expect"])
}

fn err_value(e: &SchemaError) -> Value {
    serde_json::json!({ "error": { "class": e.ruby_class(), "message": e.message() } })
}

fn document_text(input: &Value) -> String {
    match input.get("yaml").and_then(Value::as_str) {
        Some(y) => y.to_owned(),
        None => std::fs::read_to_string(abs(input["file"].as_str().expect("file"))).expect("input"),
    }
}

fn run_case(case: &Value) -> Value {
    let input = &case["input"];
    let schema_rel = input["schema"].as_str();
    match case["op"].as_str().expect("op") {
        "validate_profile" => {
            let env = input.get("env").and_then(Value::as_str);
            match env {
                Some(e) => std::env::set_var(schema::SCHEMA_PATH_ENV, abs(e)),
                None => std::env::remove_var(schema::SCHEMA_PATH_ENV),
            }
            let result = (|| {
                let profile = safe_load(&document_text(input), Permitted::DOCUMENT)?;
                schema::validate_profile(
                    &profile,
                    &ValidateOptions {
                        schema_path: schema_rel.map(abs),
                        strict: input["strict"].as_bool().unwrap_or(false),
                    },
                )
            })();
            std::env::remove_var(schema::SCHEMA_PATH_ENV);
            match result {
                Ok(messages) => serde_json::json!({ "ok": messages }),
                Err(e) => err_value(&e),
            }
        }
        "definition_schema" => {
            let result = (|| {
                let Some(schema) = load_definition_schema(&abs(schema_rel.expect("schema")))?
                else {
                    return Ok(Value::String("skipped".into()));
                };
                if !schema.truthy() {
                    return Ok(Value::String("skipped".into()));
                }
                let definition = safe_load(&document_text(input), Permitted::DOCUMENT)?;
                let report = definition_schema_report(&definition, &schema)?;
                Ok(serde_json::json!({ "errors": report.errors, "warnings": report.warnings }))
            })();
            match result {
                Ok(v) => serde_json::json!({ "ok": v }),
                Err(e) => err_value(&e),
            }
        }
        other => panic!("unknown op {other}"),
    }
}

fn divergence(id: &str) -> Option<Divergence> {
    DIVERGENCES
        .iter()
        .find(|(prefix, _, _)| id.starts_with(prefix))
        .map(|(_, d, _)| *d)
}

/// Platform-specific cases: the oracle ran on Linux.
fn applicable(id: &str) -> bool {
    !id.starts_with("path.directory.") || cfg!(unix)
}

#[test]
fn golden_cases_match_the_oracle() {
    let golden = golden();
    let cases = golden["cases"].as_array().expect("cases");
    assert!(cases.len() >= 370, "golden set shrank: {}", cases.len());
    let mut failures = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for case in cases {
        let id = case["id"].as_str().expect("id");
        if !applicable(id) {
            *counts.entry("not-applicable").or_default() += 1;
            continue;
        }
        let want = expected(case);
        let got = run_case(case);
        let ok = match divergence(id) {
            None => {
                *counts.entry("exact").or_default() += 1;
                got == want
            }
            Some(Divergence::YamlMessage) => {
                *counts.entry("yaml-message").or_default() += 1;
                want["error"]["class"] == "Psych::SyntaxError"
                    && got["error"]["class"] == "Psych::SyntaxError"
            }
            Some(Divergence::Unsupported) => {
                *counts.entry("unsupported").or_default() += 1;
                got["error"]["class"].is_null()
                    && got["error"]["message"]
                        .as_str()
                        .is_some_and(|m| m.starts_with("unsupported ("))
            }
        };
        if !ok {
            failures.push(format!(
                "{id}\n  want: {}\n  got:  {}",
                serde_json::to_string(&want).unwrap(),
                serde_json::to_string(&got).unwrap()
            ));
        }
    }
    eprintln!("schema golden cases: {counts:?}");
    assert!(
        failures.is_empty(),
        "{} of {} cases diverge:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn every_declared_divergence_is_used() {
    let golden = golden();
    let ids: Vec<&str> = golden["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    for (prefix, _, why) in DIVERGENCES {
        assert!(
            ids.iter().any(|id| id.starts_with(prefix)),
            "stale divergence {prefix} ({why})"
        );
    }
}

#[test]
fn golden_inputs_are_the_captured_bytes() {
    // The capture recorded a SHA-256 for every input; a changed input
    // invalidates the golden data. Hashing is done by the test with a small
    // local SHA-256 to avoid a dependency.
    let golden = golden();
    let sums = golden["provenance"]["inputs_sha256"].as_object().unwrap();
    assert!(!sums.is_empty());
    for (rel, sum) in sums {
        let bytes = std::fs::read(crate_dir().join(rel)).unwrap_or_else(|_| panic!("{rel}"));
        assert_eq!(sha256_hex(&bytes), sum.as_str().unwrap(), "{rel}");
    }
}

#[test]
fn bundled_schemas_are_the_sourced_files() {
    for s in schema::bundled::SCHEMAS {
        let on_disk = std::fs::read_to_string(crate_dir().join(format!(
            "schemas/v1/{}",
            Path::new(s.source_path).file_name().unwrap().to_str().unwrap()
        )))
        .unwrap();
        assert_eq!(on_disk, s.contents, "{}", s.name);
    }
    let original = std::fs::read_to_string(
        crate_dir().join("tests/schema_golden/inputs/oracle/v1/tool.schema.yaml"),
    )
    .unwrap();
    let repaired = schema::bundled::get("tool").unwrap().contents;
    let diff: Vec<(&str, &str)> = original
        .lines()
        .zip(repaired.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(original.lines().count(), repaired.lines().count());
    assert_eq!(
        diff,
        vec![(
            "        description: DEPRECATED: Use invocation.type instead",
            "        description: 'DEPRECATED: Use invocation.type instead'"
        )]
    );
}

#[test]
fn bundled_schemas_load_and_are_rejected_as_draft_07() {
    for s in schema::bundled::SCHEMAS {
        let loaded = safe_load(s.contents, Permitted::NONE).expect(s.name);
        let err = schema::fully_validate(&loaded, &schema::Data::Hash(vec![]), false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "JSON::Schema::SchemaError: Schema not found: http://json-schema.org/draft-07/schema#",
            "{}",
            s.name
        );
    }
}

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}
