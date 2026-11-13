//! Parity tests for `ukiryu::extractors::man` against golden data captured
//! from the Ruby oracle (`Ukiryu::ManPageParser` at ukiryu ab6df353, Ruby
//! 4.0.1, date gem 3.5.1, the corpus oracle Gemfile.lock fc3b12f2). See
//! `provenance` in the data file. `Date._parse` runs through the shared
//! `ukiryu::types::date_parse` port.

use serde_json::Value;
use std::collections::BTreeMap;
use std::io;
use ukiryu::extractors::man::{
    date_parse, date_parse_fragments, extract_date_from_dd_line, parse_content, DateFailure,
    FileSystem, Fragments, ManPageError, ManPageParser, ManPageSource, Today,
};

const ORACLE: &str = include_str!("../src/extractors/man/testdata/oracle.json");

fn oracle() -> Value {
    serde_json::from_str(ORACLE).expect("oracle json")
}

fn today(o: &Value) -> Today {
    let t = &o["today"];
    let get = |k: &str| i32::try_from(t[k].as_i64().expect("int")).expect("i32");
    Today {
        year: get("year"),
        mon: get("mon"),
        mday: get("mday"),
    }
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn failure_json(f: &DateFailure) -> Value {
    let class = match f {
        DateFailure::Argument(_) => "ArgumentError",
        DateFailure::Invalid => "Date::Error",
        DateFailure::Range(_) => "RangeError",
        DateFailure::Unsupported(_) => "Unsupported",
    };
    serde_json::json!({"error": {"class": class, "message": f.to_string()}})
}

fn error_json(e: &ManPageError) -> Value {
    serde_json::json!({"error": {"class": e.ruby_class(), "message": e.to_string()}})
}

fn result_json(r: Result<Option<String>, ManPageError>) -> Value {
    match r {
        Ok(v) => serde_json::json!({ "ok": v }),
        Err(e) => error_json(&e),
    }
}

fn fragments_json(f: &Fragments) -> Value {
    let mut m = BTreeMap::new();
    let ints = [
        ("year", &f.year),
        ("mon", &f.mon),
        ("mday", &f.mday),
        ("yday", &f.yday),
        ("wday", &f.wday),
        ("cwyear", &f.cwyear),
        ("cweek", &f.cweek),
        ("cwday", &f.cwday),
        ("hour", &f.hour),
        ("min", &f.min),
        ("sec", &f.sec),
    ];
    for (k, v) in ints {
        if let Some(v) = v {
            m.insert(k.to_string(), Value::String(v.to_string()));
        }
    }
    if let Some((n, d)) = &f.sec_fraction {
        m.insert("sec_fraction".into(), Value::String(format!("{n}/{d}")));
    }
    if let Some(z) = &f.zone {
        m.insert("zone".into(), Value::String(z.clone()));
    }
    serde_json::json!({ "ok": m })
}

/// Replays recorded file-system behaviour by path.
#[derive(Default)]
struct Recorded {
    files: BTreeMap<String, Result<Vec<u8>, io::ErrorKind>>,
}

impl Recorded {
    fn with(mut self, path: &str, r: Result<Vec<u8>, io::ErrorKind>) -> Self {
        self.files.insert(path.to_string(), r);
        self
    }
}

impl ManPageSource for Recorded {
    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        match &self.files[path] {
            Ok(b) => Ok(b.clone()),
            Err(k) => Err(io::Error::from(*k)),
        }
    }
}

fn mismatch_report(kind: &str, fails: &[String]) {
    assert!(
        fails.is_empty(),
        "{} {kind} mismatches:\n{}",
        fails.len(),
        fails.join("\n")
    );
}

#[test]
fn date_parse_fragments_match_oracle() {
    let o = oracle();
    let mut fails = Vec::new();
    for c in o["date_cases"].as_array().expect("cases") {
        let s = c["date_str"].as_str().expect("str");
        let got = match date_parse_fragments(s) {
            Ok(f) => fragments_json(&f),
            Err(e) => failure_json(&e),
        };
        if got != c["fragments"] {
            fails.push(format!("{s:?}\n  want {}\n  got  {got}", c["fragments"]));
        }
    }
    mismatch_report("Date._parse", &fails);
}

#[test]
fn date_parse_matches_oracle() {
    let o = oracle();
    let t = today(&o);
    let mut fails = Vec::new();
    for c in o["date_cases"].as_array().expect("cases") {
        let s = c["date_str"].as_str().expect("str");
        let got = match date_parse(s, &t) {
            Ok(d) => serde_json::json!({ "ok": d.iso8601() }),
            Err(e) => failure_json(&e),
        };
        if got != c["date_parse"] {
            fails.push(format!("{s:?}\n  want {}\n  got  {got}", c["date_parse"]));
        }
    }
    mismatch_report("Date.parse", &fails);
}

#[test]
fn extract_date_from_dd_line_matches_oracle() {
    let o = oracle();
    let t = today(&o);
    let mut fails = Vec::new();
    for c in o["date_cases"].as_array().expect("cases") {
        let line = c["dd_line"].as_str().expect("line");
        let got = result_json(extract_date_from_dd_line(line, &t));
        if got != c["extract"] {
            fails.push(format!("{line:?}\n  want {}\n  got  {got}", c["extract"]));
        }
    }
    mismatch_report("extract_date_from_dd_line", &fails);
}

#[test]
fn parse_date_over_recorded_pages_matches_oracle() {
    let o = oracle();
    let t = today(&o);
    let mut fails = Vec::new();
    let cases = o["date_cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|c| (c["date_str"].as_str().expect("s"), c))
        .chain(
            o["file_cases"]
                .as_array()
                .expect("files")
                .iter()
                .map(|c| (c["name"].as_str().expect("name"), c)),
        );
    for (name, c) in cases {
        let bytes = hex(c["content_hex"].as_str().expect("hex"));
        let parser = ManPageParser::new(Recorded::default().with("page.1", Ok(bytes)), t);
        let got = result_json(parser.parse_date(Some("page.1")));
        if got != c["parse_date"] {
            fails.push(format!(
                "{name:?}\n  want {}\n  got  {got}",
                c["parse_date"]
            ));
        }
    }
    mismatch_report("parse_date", &fails);
}

#[test]
fn path_handling_matches_oracle() {
    let o = oracle();
    let t = today(&o);
    let s = &o["special"];
    let parser = ManPageParser::new(
        Recorded::default()
            .with("dir", Err(io::ErrorKind::IsADirectory))
            .with("unreadable", Err(io::ErrorKind::PermissionDenied)),
        t,
    );
    assert_eq!(result_json(parser.parse_date(None)), s["nil_path"]);
    assert_eq!(result_json(parser.parse_date(Some(""))), s["empty_path"]);
    assert_eq!(
        result_json(parser.parse_date(Some("missing"))),
        s["missing_path"]
    );
    assert_eq!(s["unreadable_is_root"], Value::Bool(false));
    assert_eq!(
        result_json(parser.parse_date(Some("unreadable"))),
        s["unreadable"]
    );
    // The oracle message embeds the host path ("Is a directory @ io_fread -
    // <path>"); the class is the parity point.
    let err = parser
        .parse_date(Some("dir"))
        .expect_err("EISDIR propagates");
    assert_eq!(err.ruby_class(), s["directory"]["error"]["class"]);
}

#[test]
fn parse_from_fallback_matches_oracle() {
    let o = oracle();
    let t = today(&o);
    let file = |name: &str| {
        let c = o["file_cases"]
            .as_array()
            .expect("files")
            .iter()
            .find(|c| c["name"] == name)
            .expect("file case");
        hex(c["content_hex"].as_str().expect("hex"))
    };
    let source = Recorded::default()
        .with("good", Ok(file("mdoc_basic")))
        .with("nodate", Ok(file("no_dd")))
        .with("bad", Ok(file("invalid_utf8_before_dd")))
        .with("second", Ok(b".Dd June 2, 2002\n".to_vec()));
    let parser = ManPageParser::new(source, t);
    let cases = o["fallback"].as_object().expect("fallback");
    assert_eq!(cases.len(), 6);
    for (name, c) in cases {
        let paths: Vec<&str> = c["paths"]
            .as_array()
            .expect("paths")
            .iter()
            .map(|p| p.as_str().expect("path"))
            .collect();
        let got = result_json(parser.parse_from_fallback(paths));
        assert_eq!(got, c["result"], "{name}");
    }
}

#[test]
fn resolve_man_page_path_is_recorded_as_broken_in_the_oracle() {
    // Not ported: the pinned source calls the undefined
    // `Ukiryu::Platform.current`, so every call raises.
    let o = oracle();
    for (_, c) in o["resolve_man_page_path"].as_object().expect("resolve") {
        assert_eq!(c["error"]["class"], "NoMethodError");
    }
}

#[test]
fn file_system_source_reads_real_files() {
    let o = oracle();
    let t = today(&o);
    let dir = std::env::temp_dir().join(format!("ukiryu-extract-man-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let parser = ManPageParser::new(FileSystem, t);
    let mut checked = 0;
    for c in o["file_cases"].as_array().expect("files") {
        let name = c["name"].as_str().expect("name");
        let path = dir.join(format!("{name}.1"));
        std::fs::write(&path, hex(c["content_hex"].as_str().expect("hex"))).expect("write");
        let got = result_json(parser.parse_date(path.to_str()));
        assert_eq!(got, c["parse_date"], "{name}");
        checked += 1;
    }
    assert_eq!(checked, o["file_cases"].as_array().expect("files").len());
    let missing = dir.join("missing.1");
    assert_eq!(
        result_json(parser.parse_date(missing.to_str())),
        o["special"]["missing_path"]
    );
    #[cfg(unix)]
    {
        let err = parser
            .parse_date(dir.to_str())
            .expect_err("reading a directory fails");
        assert_eq!(
            err.ruby_class(),
            o["special"]["directory"]["error"]["class"]
        );
    }
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn content_level_entry_point_agrees_with_parse_date() {
    let o = oracle();
    let t = today(&o);
    for c in o["file_cases"].as_array().expect("files") {
        let bytes = hex(c["content_hex"].as_str().expect("hex"));
        assert_eq!(result_json(parse_content(&bytes, &t)), c["parse_date"]);
    }
}
