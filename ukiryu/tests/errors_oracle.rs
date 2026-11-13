//! Oracle parity for `ukiryu::errors` (`Ukiryu::Errors::*` and the executor's
//! `ExecutionError`/`TimeoutError` messages), plus every corpus row that
//! expects a `Ukiryu::Errors` class.
//!
//! Expectations come from `tests/execution_result_golden/` (see its README)
//! and the shared conformance corpus.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use ukiryu::errors::{execution_failed_message, Error, ErrorKind};
use ukiryu::execution::Seconds;

fn golden(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/execution_result_golden")
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(|l| serde_json::from_str(l).expect("golden row is JSON"))
        .collect()
}

fn text(v: &Value) -> &str {
    v.as_str()
        .unwrap_or_else(|| panic!("expected a string, got {v}"))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| text(s).to_owned())
        .collect()
}

fn seconds(v: &Value) -> Option<Seconds> {
    if v.is_null() {
        return None;
    }
    if let Some(i) = v.get("int") {
        return Some(Seconds::Int(i.as_i64().unwrap()));
    }
    Some(Seconds::Float(text(&v["float"]).parse().unwrap()))
}

#[test]
fn every_error_class_matches_oracle() {
    let rows = golden("errors");
    assert_eq!(rows.len(), ErrorKind::ALL.len());
    let mut seen = Vec::new();
    for row in &rows {
        let class = text(&row["class"]);
        let kind =
            ErrorKind::from_ruby_class(class).unwrap_or_else(|| panic!("{class} has no ErrorKind"));
        seen.push(kind);
        assert_eq!(
            kind.parent().map_or("StandardError", ErrorKind::ruby_class),
            text(&row["superclass"]),
            "{class}"
        );
        let ruby_ancestors: Vec<String> = strings(&row["ancestors"])
            .into_iter()
            .filter(|a| a.starts_with("Ukiryu::Errors::"))
            .collect();
        let ancestors: Vec<&str> = kind
            .ancestors()
            .into_iter()
            .map(ErrorKind::ruby_class)
            .collect();
        assert_eq!(ancestors, ruby_ancestors, "{class}");
        assert_eq!(
            Error::new(kind, None).message(),
            text(&row["default_message"]),
            "{class}"
        );
        let custom = Error::new(kind, Some("custom message"));
        assert_eq!(custom.message(), text(&row["message"]));
        assert_eq!(custom.to_string(), text(&row["message"]));
        assert_eq!(custom.ruby_class(), class);
        let platform = text(&row["ruby_platform"]);
        assert_eq!(
            custom.suggestions(platform),
            strings(&row["suggestions"]),
            "{class}"
        );
        let mut is_a: Vec<&str> = ErrorKind::ALL
            .into_iter()
            .filter(|o| custom.is_a(*o))
            .map(ErrorKind::ruby_class)
            .collect();
        is_a.sort_unstable();
        assert_eq!(is_a, strings(&row["is_a"]), "{class}");
        if kind == ErrorKind::Timeout {
            assert_eq!(
                Error::new(kind, None).timeout(),
                seconds(&row["timeout_default"])
            );
            for case in row["timeout_kwarg"].as_array().unwrap() {
                let e = Error::timeout_error(Some("m"), seconds(&case["timeout"]));
                assert_eq!(e.timeout(), seconds(&case["got"]));
                assert_eq!(e.message(), text(&case["message"]));
            }
        } else {
            assert_eq!(custom.timeout(), None);
        }
    }
    for kind in ErrorKind::ALL {
        assert!(seen.contains(&kind), "{kind:?} not in the oracle capture");
    }
}

#[test]
fn unknown_class_names_are_not_mapped() {
    for name in [
        "",
        "Error",
        "StandardError",
        "Ukiryu::Errors::NoSuchError",
        "ukiryu::errors::validationerror",
        "Ukiryu::Errors::ValidationError ",
    ] {
        assert_eq!(ErrorKind::from_ruby_class(name), None, "{name:?}");
    }
}

#[test]
fn execution_failed_message_matches_oracle() {
    let rows = golden("format_error");
    assert_eq!(rows.len(), 5);
    for row in &rows {
        let i = &row["input"];
        assert_eq!(
            execution_failed_message(
                text(&i["executable"]),
                text(&i["command"]),
                i["stdout"].as_str(),
                i["stderr"].as_str(),
                i["status"].as_i64().unwrap()
            ),
            text(&row["expect"]["message"]),
            "{i}"
        );
    }
}

fn corpus_dir() -> PathBuf {
    std::env::var_os("UKIRYU_CONFORMANCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance"))
}

/// Every corpus row that expects a `Ukiryu::Errors` exception names a class
/// this taxonomy knows. The per-class counts are pinned to the corpus at
/// ukiryu-rs main 3540a1c; a new producer row of that kind changes them
/// deliberately.
#[test]
fn corpus_error_classes_are_in_the_taxonomy() {
    let cases = corpus_dir().join("cases");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&cases)
        .unwrap_or_else(|e| panic!("{}: {e}", cases.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    let mut total = 0;
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    for file in files {
        for line in std::fs::read_to_string(&file).unwrap().lines() {
            total += 1;
            let row: Value = serde_json::from_str(line).unwrap();
            let Some(class) = row["expect"]["error"]["class"].as_str() else {
                continue;
            };
            if !class.starts_with("Ukiryu::Errors::") {
                continue;
            }
            let kind = ErrorKind::from_ruby_class(class)
                .unwrap_or_else(|| panic!("{}: unknown {class}", row["id"]));
            assert!(kind.is_a(ErrorKind::Error));
            *by_class.entry(class.to_owned()).or_default() += 1;
        }
    }
    assert!(total > 0, "no corpus rows read");
    let expected: BTreeMap<String, usize> = [
        ("Ukiryu::Errors::ToolNotFoundError", 3),
        ("Ukiryu::Errors::UnknownShellError", 2),
        ("Ukiryu::Errors::ValidationError", 31),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(by_class, expected);
}
