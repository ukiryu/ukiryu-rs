//! `DocumentationGenerator` parity against captures of the pinned Ruby
//! oracle (ab6df353, `lib/ukiryu/definition/documentation_generator.rb`).
//!
//! Each case in `src/definition/documentation/oracle_cases.json` holds the
//! YAML text, the typed tree that `YAML.safe_load(yaml, symbolize_names:
//! true)` produced in Ruby, and the exact output (or exception class and
//! message) of every captured call. The test parses the same YAML with
//! yeptris, checks the tree matches what Ruby saw, then compares results
//! byte for byte.
use serde_json::Value as Json;
use std::path::PathBuf;
use ukiryu::definition::documentation::{
    DocumentationError, DocumentationErrorKind, DocumentationGenerator, Format, IoErrno, FORMATS,
};
use yeptris::{Number, Value};

const ORACLE: &str = include_str!("../src/definition/documentation/oracle_cases.json");
const ORACLE_SHA: &str = "ab6df353c6dc2d7faf7817a86aafcafa82579896";

fn oracle() -> Json {
    let doc: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(doc["provenance"]["oracle_sha"], ORACLE_SHA);
    doc
}

fn parse(yaml: &str) -> Value {
    let mut docs = yeptris::parse_stream(yaml.as_bytes()).expect("yeptris parse");
    assert_eq!(docs.len(), 1, "one YAML document");
    docs.remove(0)
}

/// Compare a yeptris value with the Ruby typed tree (`is_key`: the value is
/// a mapping key, so a string stands for a symbol).
fn same_tree(ruby: &Json, ours: &Value, is_key: bool, path: &str) -> Result<(), String> {
    let ours = match ours {
        Value::Tagged(t) => &t.value,
        other => other,
    };
    let t = ruby["t"].as_str().unwrap();
    let fail = || Err(format!("{path}: ruby {ruby} vs yeptris {ours:?}"));
    match (t, ours) {
        ("nil", Value::Null) => Ok(()),
        ("bool", Value::Bool(b)) if ruby["v"].as_bool() == Some(*b) => Ok(()),
        ("str", Value::String(s)) if !is_key && ruby["v"] == s.as_str() => Ok(()),
        ("sym", Value::String(s)) if is_key && ruby["v"] == s.as_str() => Ok(()),
        ("int", Value::Number(Number::I64(i))) if ruby["v"] == i.to_string().as_str() => Ok(()),
        ("int", Value::Number(Number::U64(u))) if ruby["v"] == u.to_string().as_str() => Ok(()),
        ("float", Value::Number(Number::F64(f))) => {
            let r: f64 = ruby["v"].as_str().unwrap().parse().unwrap();
            if r.to_bits() == f.to_bits() {
                Ok(())
            } else {
                fail()
            }
        }
        ("seq", Value::Sequence(items)) => {
            let r = ruby["v"].as_array().unwrap();
            if r.len() != items.len() {
                return fail();
            }
            for (i, (a, b)) in r.iter().zip(items).enumerate() {
                same_tree(a, b, false, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        ("map", Value::Mapping(m)) => {
            let r = ruby["v"].as_array().unwrap();
            if r.len() != m.0.len() {
                return fail();
            }
            for (i, (pair, (k, v))) in r.iter().zip(&m.0).enumerate() {
                same_tree(&pair[0], k, true, &format!("{path}{{key {i}}}"))?;
                same_tree(&pair[1], v, false, &format!("{path}{{value {i}}}"))?;
            }
            Ok(())
        }
        _ => fail(),
    }
}

fn result_json(r: Result<String, DocumentationError>) -> Json {
    match r {
        Ok(text) => serde_json::json!({ "ok": text }),
        Err(e) => {
            serde_json::json!({ "error": { "class": e.ruby_class(), "message": e.message } })
        }
    }
}

#[test]
fn oracle_cases_match_byte_for_byte() {
    let doc = oracle();
    let cases = doc["cases"].as_array().unwrap();
    let mut failures: Vec<String> = Vec::new();
    let (mut ok_count, mut err_count) = (0usize, 0usize);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let input = parse(case["yaml"].as_str().unwrap());
        if let Err(e) = same_tree(&case["input"], &input, false, name) {
            failures.push(format!("input tree: {e}"));
            continue;
        }
        for r in case["results"].as_array().unwrap() {
            let format = r["format"].as_str().unwrap();
            let ours = match r["op"].as_str().unwrap() {
                "generate" => DocumentationGenerator::generate(&input, format),
                "command" => DocumentationGenerator::generate_command_docs(
                    r["command"].as_str().unwrap(),
                    &input,
                    format,
                ),
                other => panic!("unknown op {other}"),
            };
            let ours = result_json(ours);
            if ours == r["result"] {
                if r["result"].get("ok").is_some() {
                    ok_count += 1;
                } else {
                    err_count += 1;
                }
            } else {
                failures.push(format!(
                    "{name} {} {format}:\n  ruby: {}\n  ours: {}",
                    r["op"], r["result"], ours
                ));
            }
        }
    }
    println!(
        "cases={} matched_ok={ok_count} matched_error={err_count} failures={}",
        cases.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures
            .iter()
            .take(25)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(cases.len() >= 120, "oracle case count");
    assert!(ok_count >= 150 && err_count >= 90, "executed counts");
}

#[test]
fn spec_literal_matches_yaml_form() {
    // Ruby's own spec builds the definition with symbol values; the capture
    // checked that it renders identically to the YAML-loaded form used here.
    assert_eq!(oracle()["spec_literal_equals_yaml"].as_bool(), Some(true));
}

fn spec_definition(doc: &Json) -> Value {
    let case = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "spec_full")
        .unwrap();
    parse(case["yaml"].as_str().unwrap())
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ukiryu-docgen-{}-{name}", std::process::id()))
}

#[test]
fn generate_to_file_writes_generated_bytes_unchanged() {
    let doc = oracle();
    let definition = spec_definition(&doc);
    let path = temp_path("docs.md");
    DocumentationGenerator::generate_to_file(&definition, &path, "markdown").unwrap();
    let written = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    // LF line endings on every platform; nothing is appended after the
    // last line (this document's last line is empty, so it ends in LF).
    assert_eq!(
        written,
        doc["generate_to_file_linux_bytes"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
    assert!(!written.contains(&b'\r'));
}

#[test]
fn generate_to_file_reports_unsupported_format_before_writing() {
    let doc = oracle();
    let path = temp_path("never.md");
    let err =
        DocumentationGenerator::generate_to_file(&spec_definition(&doc), &path, "pdf").unwrap_err();
    assert_eq!(err.kind, DocumentationErrorKind::Argument);
    assert_eq!(err.message, "Unsupported format: pdf");
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn generate_to_file_errno_messages_match_linux_capture() {
    let doc = oracle();
    let expected = &doc["generate_to_file_io_errors_linux"];
    let dir = temp_path("io");
    std::fs::create_dir_all(&dir).unwrap();
    let definition = parse("name: x\n");
    let cases = [
        ("enoent", dir.join("missing").join("x.md"), IoErrno::NoEntry),
        ("eisdir", dir.clone(), IoErrno::IsDirectory),
    ];
    for (key, path, errno) in cases {
        let err =
            DocumentationGenerator::generate_to_file(&definition, &path, "markdown").unwrap_err();
        assert_eq!(err.kind, DocumentationErrorKind::Io(errno), "{key}");
        assert_eq!(err.ruby_class(), expected[key]["class"], "{key}");
        let message = err.message.replace(dir.to_str().unwrap(), "<DIR>");
        assert_eq!(message, expected[key]["message"], "{key}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn formats_and_normalization() {
    assert_eq!(FORMATS, ["markdown", "asciidoc", "md"]);
    assert_eq!(Format::from_symbol("md"), Ok(Format::Markdown));
    assert_eq!(Format::from_symbol("markdown"), Ok(Format::Markdown));
    assert_eq!(Format::from_symbol("asciidoc"), Ok(Format::Asciidoc));
    let err = Format::from_symbol("adoc").unwrap_err();
    assert_eq!(err.ruby_class(), "ArgumentError");
    assert_eq!(err.message, "Unsupported format: adoc");
    assert_eq!(err.to_string(), "ArgumentError: Unsupported format: adoc");
}
