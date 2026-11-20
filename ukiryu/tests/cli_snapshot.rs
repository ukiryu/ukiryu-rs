//! Focused parity and side-effect boundary tests for portable CLI snapshots.
use serde_json::{json, Value};
use std::path::Path;

fn snapshot() -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance");
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap())
            .unwrap();
    let mut files = serde_json::Map::new();
    for entry in manifest["inputs"].as_array().unwrap() {
        let name = entry["path"].as_str().unwrap();
        files.insert(
            format!("${{INPUTS}}/{name}"),
            Value::String(std::fs::read_to_string(root.join("inputs").join(name)).unwrap()),
        );
    }
    json!({"files":files,"executables":{},"probes":[],"definitions":[]})
}
fn request(argv: Value, snapshot: Value) -> Value {
    ukiryu::wire::process(&serde_json::to_vec(&json!({"op":"cli.run", "argv":argv, "platform":"linux", "env":{
        "HOME":"${HOME}","SHELL":"/bin/bash","PATH":"","TZ":"UTC","LANG":"C.UTF-8","LC_ALL":"C.UTF-8","TERM":"dumb","COLUMNS":"100","THOR_COLUMNS":"100","UKIRYU_REGISTER":"${INPUTS}/register"
    },"snapshot":snapshot})).unwrap())
}
#[test]
fn all_21_frozen_cli_rows_run_from_text_without_host_io() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance");
    let snapshot = snapshot();
    let mut failures = Vec::new();
    let mut count = 0;
    for line in std::fs::read_to_string(root.join("cases/cli-output.jsonl"))
        .unwrap()
        .lines()
    {
        let row: Value = serde_json::from_str(line).unwrap();
        let actual = request(row["input"]["argv"].clone(), snapshot.clone());
        if actual["ok"] != true || actual["value"] != row["expect"]["ok"] {
            failures.push(format!(
                "{}: actual {actual}, expected {}",
                row["id"], row["expect"]
            ));
        }
        count += 1;
    }
    assert_eq!(count, 21);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn snapshot_is_not_a_transcript_and_never_falls_back_to_host_files() {
    let original = snapshot();
    let mut changed = original.clone();
    let key = "${INPUTS}/synthetic-register/tools/absenttool/default/1.0.yaml";
    changed["files"][key] = Value::String(
        changed["files"][key]
            .as_str()
            .unwrap()
            .replace("Absent Tool (corpus control)", "Snapshot mutation"),
    );
    let argv = json!([
        "info",
        "absenttool",
        "--register",
        "${INPUTS}/synthetic-register"
    ]);
    let actual = request(argv.clone(), changed);
    assert_eq!(actual["ok"], true);
    assert!(actual["value"]["stdout"]
        .as_str()
        .unwrap()
        .contains("Snapshot mutation"));
    assert_ne!(actual, request(argv, original.clone()));
    assert_ne!(
        request(json!(["help"]), original.clone()),
        request(json!(["help", "exec"]), original.clone())
    );
    let mut changed = original.clone();
    changed["files"]["${INPUTS}/external-schema/tool.schema.yaml"] =
        Value::String("type: object\n".into());
    let argv = json!([
        "validate",
        "file",
        "${INPUTS}/profiles/inkscape_1.0.yaml",
        "--schema",
        "${INPUTS}/external-schema/tool.schema.yaml"
    ]);
    assert_ne!(request(argv.clone(), original), request(argv, changed));
    let mut external = snapshot();
    external["files"]["${INPUTS}/external-schema/tool.schema.yaml"] = Value::String(format!(
        "$ref: file://{}/Cargo.toml\n",
        env!("CARGO_MANIFEST_DIR")
    ));
    let refused = request(
        json!([
            "validate",
            "file",
            "${INPUTS}/profiles/inkscape_1.0.yaml",
            "--schema",
            "${INPUTS}/external-schema/tool.schema.yaml"
        ]),
        external,
    );
    assert_eq!(refused["error"]["code"], "UNSUPPORTED_CAPABILITY");
    let empty = json!({"files":{},"executables":{},"probes":[],"definitions":[]});
    let actual = request(
        json!([
            "validate",
            "file",
            format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"))
        ]),
        empty,
    );
    assert_eq!(actual["error"]["code"], "UNSUPPORTED_CAPABILITY");
}
#[test]
fn external_schema_paths_are_refused_independently_of_host_platform() {
    for reference in [
        r"file://D:\a\ukiryu-rs\ukiryu-rs\ukiryu/Cargo.toml",
        "file:///C:/host/Cargo.toml",
        "file://localhost/C:/host/Cargo.toml",
        r"C:\host\Cargo.toml",
        r"\\server\share\Cargo.toml",
        "file:///host/Cargo.toml",
        "../host/Cargo.toml",
        "https://example.invalid/schema.json",
    ] {
        let mut external = snapshot();
        external["files"]["${INPUTS}/external-schema/tool.schema.yaml"] =
            Value::String(format!("$ref: {reference}\n"));
        let refused = request(
            json!([
                "validate",
                "file",
                "${INPUTS}/profiles/inkscape_1.0.yaml",
                "--schema",
                "${INPUTS}/external-schema/tool.schema.yaml"
            ]),
            external,
        );
        assert_eq!(
            refused["error"]["code"], "UNSUPPORTED_CAPABILITY",
            "{reference}: {refused}"
        );
    }
}

#[test]
fn side_effecting_commands_and_missing_snapshots_are_refused_loudly() {
    for argv in [
        json!(["exec", "jq", "process", "filter=."]),
        json!(["exec", "jq", "--dry-run"]),
        json!(["exec", "jq", "help", "--dry-run"]),
        json!(["exec", "jq", "process", "--dry-run", "--stdin"]),
        json!([
            "exec",
            "jq",
            "process",
            "--dry-run",
            "--output",
            "/tmp/s303-forbidden"
        ]),
        json!(["register", "update"]),
        json!(["definitions", "remove", "jq"]),
        json!(["config", "set", "shell", "sh"]),
        json!(["extract", "jq"]),
        json!(["validate", "file", "x", "--executable"]),
    ] {
        let actual = request(argv.clone(), snapshot());
        assert_eq!(
            actual["error"]["code"], "UNSUPPORTED_CAPABILITY",
            "{argv}: {actual}"
        );
    }
    let actual = ukiryu::wire::process(
        br#"{"op":"cli.run","argv":["version"],"platform":"linux","env":{}}"#,
    );
    assert_eq!(actual["error"]["code"], "INVALID_REQUEST");
}
