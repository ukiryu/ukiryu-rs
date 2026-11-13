//! Ruby `Regexp` parity for `ukiryu::types::onig` (Onigmo of Ruby 4.0.1).
//!
//! `tests/regexp_onigmo_data/cases.jsonl` was captured from Ruby 4.0.1
//! (revision e04267a1): each row is `Regexp.new(pattern, options)` and the
//! `MatchData#byteoffset` of every group of `match(string)`, or the error
//! class when Ruby rejects the pattern. The input `options` are the
//! resulting `Regexp#options`, as a Ruby `Regexp` value carries them. Rows
//! cover multi-character case folding, back-references and named
//! references, Unicode properties, non-ASCII patterns, the escapes Ruby
//! rewrites before Onigmo sees them, and the US-ASCII/UTF-8 compile choice.
use std::path::Path;

use serde_json::Value as Json;
use ukiryu::types::onig::OnigRegex;

fn rows() -> Vec<Json> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regexp_onigmo_data/cases.jsonl");
    std::fs::read_to_string(path)
        .expect("cases.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).expect("JSON row"))
        .collect()
}

fn spans(c: &ukiryu::types::onig::Captures) -> Json {
    Json::Array(
        c.groups
            .iter()
            .map(|g| match g {
                Some((s, e)) => serde_json::json!([s, e]),
                None => Json::Null,
            })
            .collect(),
    )
}

#[test]
fn regexp_onigmo_matrix() {
    let mut problems = Vec::new();
    let all = rows();
    assert!(
        all.len() >= 90,
        "expected the full capture, got {}",
        all.len()
    );
    for row in &all {
        let id = row["id"].as_str().unwrap();
        let input = &row["input"];
        let pattern = input["pattern"].as_str().unwrap();
        let options = u32::try_from(input["options"].as_u64().unwrap()).unwrap();
        let string = input["string"].as_str().unwrap();
        let expect = &row["expect"];
        let actual = match OnigRegex::compile(pattern, options) {
            Err(_) => serde_json::json!({"error": "RegexpError"}),
            Ok(re) => match re.captures(string) {
                None => serde_json::json!({"engine_error": true}),
                Some(m) => serde_json::json!({"match": m.as_ref().map(spans)}),
            },
        };
        if actual != *expect {
            problems.push(format!("{id}: expected {expect}, got {actual}"));
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} rows differ:\n{}",
        problems.len(),
        all.len(),
        problems.join("\n")
    );
}

/// The compiled regexp is shared by threads (date parsing keeps them in
/// statics); concurrent searches must agree.
#[test]
fn regexp_onigmo_shared_across_threads() {
    let re = std::sync::Arc::new(OnigRegex::new(r"(\d+)(st|nd|rd|th)?", 1).unwrap());
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let re = re.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    let text = if i % 2 == 0 { "1\u{fb06} Jan" } else { "22nd" };
                    let m = re.captures(text).unwrap().unwrap();
                    assert!(m.span(2).is_some());
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}
