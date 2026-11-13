//! Browser-side entry points for the Onigmo regexp checks.
use serde_json::{json, Value};
use ukiryu::types::date_parse;
use ukiryu::types::onig::OnigRegex;
use wasm_bindgen::prelude::*;

/// `Regexp.new(pattern, options).match(string)` as the JSON shape of
/// `tests/regexp_onigmo_data/cases.jsonl` expectations.
#[wasm_bindgen]
pub fn regexp_match(pattern: &str, options: u32, string: &str) -> String {
    let result = match OnigRegex::compile(pattern, options) {
        Err(_) => json!({"error": "RegexpError"}),
        Ok(re) => match re.captures(string) {
            None => json!({"engine_error": true}),
            Some(None) => json!({"match": Value::Null}),
            Some(Some(m)) => json!({"match": m
                .groups
                .iter()
                .map(|g| g.map(|(s, e)| json!([s, e])).unwrap_or(Value::Null))
                .collect::<Vec<_>>()}),
        },
    };
    result.to_string()
}

/// `Date._parse(input, true)`: the year, month and day `Time.parse` reads.
#[wasm_bindgen]
pub fn date_parse(input: &str) -> String {
    match date_parse::parse(input) {
        Ok(h) => json!({
            "year": h.year.map(|v| v.to_string()),
            "mon": h.mon.map(|v| v.to_string()),
            "mday": h.mday.map(|v| v.to_string()),
        })
        .to_string(),
        Err(e) => json!({"error": format!("{e:?}")}).to_string(),
    }
}
