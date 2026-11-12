//! Ruby pattern semantics against rows captured from the pinned Ruby
//! (ukiryu ab6df353, Ruby 4.0.1). Regenerate with the capture script named in
//! the golden file's provenance; never edit expectations by hand.
use serde_json::Value;
use ukiryu::version_detection::{PatternError, RubyPattern, DEFAULT_PATTERN_SOURCE};

fn golden() -> Value {
    serde_json::from_str(include_str!("version_detection_golden/regex.json")).unwrap()
}

/// Ruby-valid patterns this port refuses on purpose. A pattern listed here
/// must fail with `Unsupported`; a supported pattern must match Ruby exactly.
const KNOWN_GAPS: &[&str] = &[
    r"(\d+)\Z",
    r"(?i)version (\d+)",
    r"(?i:ver)sion (\d+)",
    "(?i)(\u{e9}+)",
    "(?i)(k)",
    "(?<n>x)(?<n>y)",
    "(\\d*+)1",
    r"(\d++)",
    r"(?=v)(v\d)",
    r"(?<=v)(\d)",
    r"(?!x)(\d)",
    r"(?<!x)(\d)",
    r"(?>a+)(b)",
    r"(a)\1",
    r"(?<n>a)\k<n>",
    r"\p{Alpha}(\d)",
    r"[[:alpha:]](\d)",
    "[[:digit:]]+",
    r"[a-z&&[^aeiou]](\d)",
    "[a[0-9]]+",
    r"\y(\d)",
    r"\Gv(\d)",
    r"v\K(\d)",
    r"\R(\d)",
    r"\X(\d)",
    r"(?x) v (\d) ",
    r"(?i-m)v(\d)",
    "(?~abc)",
    r"(?u)\w",
    r"(?a)\w",
    "a**",
    "a{1}{2}",
    "a{1}?",
    r"\cA",
    r"\o{101}",
    r"\101(\d)",
    "^*a",
    "(?i)ss",
    r"(?i)(\w)",
    r"(?i)([\w])",
    "(?i)([a-z])",
    "x{1}+",
    "(?<\u{e9}>v)",
    r"\x4",
    r"\u{41}",
];

/// Rows (pattern, input) where a `^` may have matched after a trailing
/// newline: refused at match time instead of guessed.
const LINE_START_ROWS: &[(&str, &str)] = &[(r"\n(^)", "a\n"), (r"^(\d*)$", "x\n"), (r"\n^", "a\n")];

/// Ruby-invalid patterns the port reports as `Unsupported` rather than
/// `RubyInvalid` because the exact Ruby classification was not needed.
const INVALID_REPORTED_UNSUPPORTED: &[&str] = &[r"\8"];

fn outcome(pattern: &RubyPattern, input: &str) -> Result<Value, PatternError> {
    Ok(match pattern.captures(input)? {
        None => Value::Null,
        Some(m) => serde_json::json!({"m0": m.whole, "m1": m.group1}),
    })
}

fn expected(row: &Value) -> Value {
    match &row["match"] {
        Value::Null => Value::Null,
        m => serde_json::json!({"m0": m["m0"], "m1": m["m1"]}),
    }
}

#[test]
fn captured_rows_match_or_are_listed_gaps() {
    let golden = golden();
    let rows = golden["rows"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        997,
        "row count changed: regenerate deliberately"
    );
    let (mut matched, mut invalid, mut gaps) = (0, 0, 0);
    let mut line_start_rows: Vec<(String, String)> = Vec::new();
    for row in rows {
        let source = row["pattern"].as_str().unwrap();
        let input = row["input"].as_str().unwrap();
        let ruby_ok = row["compile"]["ok"].as_bool().unwrap();
        let compiled = RubyPattern::new(source);
        if !ruby_ok {
            match compiled {
                Err(PatternError::RubyInvalid { .. }) => invalid += 1,
                Err(PatternError::Unsupported { .. })
                    if INVALID_REPORTED_UNSUPPORTED.contains(&source) =>
                {
                    gaps += 1
                }
                other => panic!(
                    "/{source}/: Ruby raises {}, port gave {other:?}",
                    row["compile"]["message"]
                ),
            }
            continue;
        }
        let actual = compiled.and_then(|p| {
            let found = outcome(&p, input)?;
            assert_eq!(
                p.is_match(input)?,
                row["match_p"].as_bool().unwrap(),
                "match? /{source}/ on {input:?}"
            );
            Ok(found)
        });
        match actual {
            Ok(found) => {
                assert!(
                    !KNOWN_GAPS.contains(&source),
                    "/{source}/ is listed as a gap but is supported"
                );
                assert_eq!(found, expected(row), "/{source}/ on {input:?}");
                matched += 1;
            }
            Err(PatternError::Unsupported { .. }) if KNOWN_GAPS.contains(&source) => gaps += 1,
            Err(PatternError::Unsupported { construct, .. })
                if construct.contains("trailing newline") =>
            {
                line_start_rows.push((source.to_string(), input.to_string()));
                gaps += 1;
            }
            Err(error) => panic!("/{source}/ on {input:?}: Ruby accepts it, port gave {error:?}"),
        }
    }
    let listed: Vec<(String, String)> = LINE_START_ROWS
        .iter()
        .map(|(p, i)| (p.to_string(), i.to_string()))
        .collect();
    assert_eq!(
        line_start_rows, listed,
        "rows refused for `^` after a trailing newline"
    );
    eprintln!("regex rows: {matched} matched, {invalid} Ruby-invalid, {gaps} explicit gaps");
    assert!(matched > 900);
}

#[test]
fn every_fixture_pattern_is_supported() {
    let golden = golden();
    let sites = golden["fixture_patterns"].as_array().unwrap();
    assert_eq!(sites.len(), 48);
    for site in sites {
        let source = site["pattern"].as_str().unwrap();
        if let Err(error) = RubyPattern::new(source) {
            panic!("{} {}: {error}", site["file"], site["key"]);
        }
    }
}

#[test]
fn default_literal_matches_ruby() {
    let golden = golden();
    let default = &golden["default_literal"];
    assert_eq!(default["source"], DEFAULT_PATTERN_SOURCE);
    for row in default["rows"].as_array().unwrap() {
        let input = row["input"].as_str().unwrap();
        let found = outcome(RubyPattern::default_literal(), input).unwrap();
        assert_eq!(found, expected(row), "default on {input:?}");
    }
}

#[test]
fn line_start_after_trailing_newline_is_refused_not_guessed() {
    // Ruby: `^` never matches at the end of "a\n" (captured: /\n^/ =~ "a\n" is nil).
    let p = RubyPattern::new(r"\n^").unwrap();
    assert!(matches!(
        p.captures("a\n"),
        Err(PatternError::Unsupported { .. })
    ));
    assert_eq!(p.captures("a\nb").unwrap().unwrap().whole, "\n");
}
