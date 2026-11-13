//! `generate_to_file` line endings: the Rust target against the bytes the
//! pinned Ruby oracle's `File.write` actually produced on each CI host.
//!
//! `src/definition/documentation/file_write_record.json` is the measurement
//! from `.github/workflows/definition-reference.yml` (Ruby 4.0.1, locked
//! oracle Gemfile, ukiryu ab6df353). This is a canonical comparison: the
//! Rust port writes the LF target text unchanged on every platform, and the
//! only allowed difference from Ruby is the measured OS text-mode
//! transform. On Windows that transform is every LF byte becoming CR LF;
//! CR bytes already in the text, including CRs embedded in values, are kept,
//! so a value's CR LF is written by Ruby as CR CR LF. Nothing beyond these
//! recorded hashes and bytes is claimed as Windows byte parity.
use serde_json::Value as Json;
use ukiryu::definition::documentation::DocumentationGenerator;
use ukiryu::definition::source::sha256_hex;

const ORACLE: &str = include_str!("../src/definition/documentation/oracle_cases.json");
const RECORD: &str = include_str!("../src/definition/documentation/file_write_record.json");
const ORACLE_SHA: &str = "ab6df353c6dc2d7faf7817a86aafcafa82579896";

fn load() -> (Json, Json) {
    let oracle: Json = serde_json::from_str(ORACLE).unwrap();
    let record: Json = serde_json::from_str(RECORD).unwrap();
    assert_eq!(oracle["provenance"]["oracle_sha"], ORACLE_SHA);
    assert_eq!(record["provenance"]["oracle_sha"], ORACLE_SHA);
    (oracle, record)
}

fn host<'a>(record: &'a Json, host_os: &str) -> &'a Json {
    record["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["host_os"] == host_os)
        .unwrap_or_else(|| panic!("no record for {host_os}"))
}

/// Apply a recorded classification to the Rust LF output. Only LF bytes are
/// rewritten; existing CR bytes are never touched or stripped.
fn transform(text: &str, classification: &str) -> String {
    match classification {
        "identical" => text.to_string(),
        "lf_to_crlf" => text.replace('\n', "\r\n"),
        other => panic!("unknown classification {other}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every successful `generate` call of the oracle, rendered by the port.
fn rust_outputs(oracle: &Json) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for case in oracle["cases"].as_array().unwrap() {
        for r in case["results"].as_array().unwrap() {
            if r["op"] != "generate" || r["result"].get("ok").is_none() {
                continue;
            }
            let format = r["format"].as_str().unwrap();
            let mut docs =
                yeptris::parse_stream(case["yaml"].as_str().unwrap().as_bytes()).unwrap();
            let text = DocumentationGenerator::generate(&docs.remove(0), format).unwrap();
            assert_eq!(text, r["result"]["ok"].as_str().unwrap());
            out.push((format!("{}|{format}", case["name"].as_str().unwrap()), text));
        }
    }
    out
}

#[test]
fn measured_ruby_file_write_classifications() {
    let (_, record) = load();
    // Ruby 4.0.1 File.write (text mode) on windows-2022 rewrote every LF
    // byte as CR LF; on ubuntu-24.04 it wrote the text unchanged.
    assert_eq!(host(&record, "mingw32")["classification"], "lf_to_crlf");
    assert_eq!(host(&record, "linux")["classification"], "identical");
}

#[test]
fn rust_target_matches_ruby_bytes_after_measured_transform_only() {
    let (oracle, record) = load();
    let outputs = rust_outputs(&oracle);
    assert_eq!(outputs.len(), 148, "captured successful generate calls");
    for h in record["hosts"].as_array().unwrap() {
        let classification = h["classification"].as_str().unwrap();
        let hashes = h["written_sha256"].as_object().unwrap();
        assert_eq!(hashes.len(), outputs.len(), "{}", h["runner"]);
        for (key, text) in &outputs {
            let expected = transform(text, classification);
            assert_eq!(
                sha256_hex(&expected),
                hashes[key.as_str()],
                "{} {key}",
                h["runner"]
            );
        }
    }
}

#[test]
fn embedded_value_crlf_is_checked_byte_for_byte() {
    let (oracle, record) = load();
    let outputs = rust_outputs(&oracle);
    for h in record["hosts"].as_array().unwrap() {
        let classification = h["classification"].as_str().unwrap();
        let embedded = h["embedded_crlf_case"].as_array().unwrap();
        assert_eq!(embedded.len(), 2);
        for e in embedded {
            let key = format!("crlf_in_values|{}", e["format"].as_str().unwrap());
            let text = &outputs.iter().find(|(k, _)| *k == key).unwrap().1;
            // The value "a\r\nb" keeps its CR in the LF target.
            assert!(text.contains("a\r\nb"));
            let expected = transform(text, classification);
            assert_eq!(hex(expected.as_bytes()), e["written_hex"], "{key}");
        }
    }
    // On Windows Ruby wrote that value as CR CR LF.
    let windows = host(&record, "mingw32");
    let written = windows["embedded_crlf_case"][0]["written_hex"]
        .as_str()
        .unwrap();
    assert!(written.contains(&hex(b"a\r\r\nb")));
}

#[test]
fn rust_generate_to_file_writes_lf_target_on_every_platform() {
    let (oracle, _) = load();
    let case = oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "crlf_in_values")
        .unwrap();
    let mut docs = yeptris::parse_stream(case["yaml"].as_str().unwrap().as_bytes()).unwrap();
    let definition = docs.remove(0);
    let path = std::env::temp_dir().join(format!(
        "ukiryu-docgen-file-write-{}.md",
        std::process::id()
    ));
    DocumentationGenerator::generate_to_file(&definition, &path, "markdown").unwrap();
    let written = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let target = DocumentationGenerator::generate(&definition, "markdown").unwrap();
    assert_eq!(written, target.as_bytes());
}
