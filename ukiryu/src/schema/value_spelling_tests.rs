use super::*;
use serde_json::{json, Value};

fn captured<T: serde::Serialize>(r: Result<T, SchemaError>) -> Value {
    match r {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"error": {"class": e.ruby_class(), "message": e.message()}}),
    }
}

#[test]
fn value_spelling_oracle() {
    // This fixture records Psych's getlocal timestamp under TZ=UTC. The
    // other library unit tests do not read TZ; integration binaries have
    // their own processes. Restore the caller's setting even on failure.
    struct RestoreTz(Option<std::ffi::OsString>);
    impl Drop for RestoreTz {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("TZ", v),
                None => std::env::remove_var("TZ"),
            }
        }
    }
    let _restore = RestoreTz(std::env::var_os("TZ"));
    std::env::set_var("TZ", "UTC");
    let rows: Vec<Value> = serde_json::from_str(include_str!(
        "../../tests/schema_golden/value-spelling/cases.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 320);
    let mut failures = Vec::new();
    let mut refusals = 0;
    for row in &rows {
        let text = row["yaml"].as_str().unwrap();
        let permitted = match row["permitted"].as_str().unwrap() {
            "none" => Permitted::NONE,
            "symbol" => Permitted::SYMBOL,
            "document" => Permitted::DOCUMENT,
            _ => unreachable!(),
        };
        let value = safe_load(text, permitted);
        // Binary strings need an encoding-aware Data string; Encoding is
        // outside this data model. Keep their existing loud refusal pinned.
        if text.starts_with("!!binary")
            || text.starts_with("!ruby/encoding")
            || (row["permitted"] == "document"
                && (text == "!ruby/class Time" || text == "!ruby/module Date"))
            || text.starts_with("!ruby/hash-with-ivars {elements:")
        {
            assert!(
                matches!(value, Err(SchemaError::Unsupported { ref feature, .. }) if feature == "YAML tag")
            );
            refusals += 1;
            continue;
        }
        let got = captured(value.map(|v| {
            json!({
                "to_s": captured(v.ruby_to_s()),
                "inspect": captured(v.ruby_inspect()),
                "json": captured(super::json::dump(&v)),
            })
        }));
        if got != row["expect"] {
            failures.push(format!(
                "{} {text:?}\nwant: {}\ngot: {got}",
                row["id"], row["expect"]
            ));
        }
    }
    assert_eq!(refusals, 20);
    assert!(
        failures.is_empty(),
        "{} divergences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Missing host-local-zone data must keep the Time and fail loudly at
    // formatting, rather than silently resolve a timestamp as a String.
    std::env::set_var("TZ", "Asia/Tokyo");
    let time = safe_load("2001-12-15 02:59:43", Permitted::DOCUMENT).unwrap();
    assert!(matches!(time, Data::Time(_)));
    assert!(
        matches!(time.ruby_to_s(), Err(SchemaError::Unsupported { ref feature, .. }) if feature == "schema local timezone")
    );
}

#[test]
fn malformed_schema_oracle() {
    let rows: Vec<Value> = serde_json::from_str(include_str!(
        "../../tests/schema_golden/value-spelling/diagnostics.json"
    ))
    .unwrap();
    assert!(!rows.is_empty());
    let mut failures = Vec::new();
    for row in rows {
        let want = if row.get("json").is_some() {
            let mut expected = row["parse_expect"].clone();
            if expected["error"]["class"] == "JSON::ParserError" {
                expected["error"]["class"] = json!("JSON::Schema::JsonParseError");
            }
            expected
        } else {
            row["expect"].clone()
        };
        let got = if let Some(text) = row["json"].as_str() {
            captured(super::json::parse_schema(text).map(|_| true))
        } else {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            captured(
                super::malformed::decode(bytes)
                    .and_then(|text| load_schema_text(&text))
                    .map(|_| true),
            )
        };
        if got != want {
            failures.push(format!("input: {row}\ngot: {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} divergences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn malformed_files_reach_the_diagnostic_parsers() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let path = std::env::temp_dir().join(format!(
        "ukiryu-schema-diagnostics-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    let scratch = Scratch(path);
    let file = scratch.0.join("schema.txt");
    let rows: Vec<Value> = serde_json::from_str(include_str!(
        "../../tests/schema_golden/value-spelling/diagnostics.json"
    ))
    .unwrap();
    let mut count = 0;
    for row in rows {
        let got = if let Some(text) = row["json"].as_str() {
            // Valid JSON followed by schema-engine errors is outside this
            // parser check. The captured parser result is tested above.
            if row["expect"]["error"]["class"] != "JSON::Schema::JsonParseError"
                && row["expect"].get("ok").is_none()
            {
                continue;
            }
            std::fs::write(&file, text).unwrap();
            captured(super::resources::read(file.to_str().unwrap()).map(|_| true))
        } else {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            captured(load_schema(Some(&file)).map(|_| true))
        };
        assert_eq!(got, row["expect"], "{row}");
        count += 1;
    }
    assert!(count >= 90, "file boundary probes shrank: {count}");
}

#[cfg(target_os = "linux")]
#[test]
fn schema_file_io_error() {
    // Ruby ab6df353: File.read('/proc/self/mem') reaches read(2), then EIO.
    let path = std::path::Path::new("/proc/self/mem");
    let error = read_text(path).unwrap_err();
    assert_eq!(error.ruby_class().as_deref(), Some("Errno::EIO"));
    assert_eq!(
        error.message(),
        "Input/output error @ io_fread - /proc/self/mem"
    );
}
