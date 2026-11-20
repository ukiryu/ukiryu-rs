//! Every frozen cli-output row through the binding entrypoint, with an explicit
//! Linux host on every CI platform. This tests the dispatcher, not a transcript
//! lookup: the help-input mutation below must change the captured output.
use serde_json::{json, Value};
use std::path::Path;
use ukiryu_cli::capture::process;

#[test]
fn every_cli_corpus_row_is_captured_byte_exact() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance");
    let inputs = std::fs::canonicalize(corpus.join("inputs")).unwrap();
    let inputs = inputs.to_str().unwrap();
    // Windows canonicalize uses a verbatim drive prefix; ordinary paths keep
    // the frozen Linux host independent of Windows' verbatim-path grammar.
    let inputs = inputs
        .strip_prefix(r"\\?\")
        .unwrap_or(inputs)
        .replace('\\', "/");
    let home = std::env::temp_dir().join(format!("ukiryu-capture-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let home = home.to_str().unwrap().replace('\\', "/");
    let env = json!({
        "HOME": home, "SHELL": "/bin/bash", "PATH": "", "TZ": "UTC",
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TERM": "dumb",
        "COLUMNS": "100", "THOR_COLUMNS": "100",
        "UKIRYU_REGISTER": format!("{inputs}/register")
    });
    let mut seen = 0;
    let mut failures = Vec::new();
    for line in std::fs::read_to_string(corpus.join("cases/cli-output.jsonl"))
        .unwrap()
        .lines()
    {
        let row: Value = serde_json::from_str(line).unwrap();
        let argv: Vec<String> = row["input"]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().replace("${INPUTS}", &inputs))
            .collect();
        let reply = process(
            &serde_json::to_vec(&json!({"argv": argv, "platform": "linux", "env": env})).unwrap(),
        );
        let actual = reply
            .to_string()
            .replace(&inputs, "${INPUTS}")
            .replace(&home, "${HOME}");
        // Replace paths on the parsed strings, not escaped JSON strings.
        let scrub = |v: &Value| {
            json!({
                "stdout": v["stdout"].as_str().unwrap_or("").replace(&inputs, "${INPUTS}").replace(&home, "${HOME}"),
                "stderr": v["stderr"].as_str().unwrap_or("").replace(&inputs, "${INPUTS}").replace(&home, "${HOME}"),
                "exit": v["exit"]
            })
        };
        if reply["ok"] != true || scrub(&reply["value"]) != row["expect"]["ok"] {
            failures.push(format!(
                "{}: got {actual}; expected {}",
                row["id"], row["expect"]
            ));
        }
        seen += 1;
    }
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(seen, 21, "all CLI corpus rows are pinned, with zero skips");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn captures_parse_errors_and_changed_input_without_leaking_output() {
    let capture = |argv: Value| {
        process(
            &serde_json::to_vec(
                &json!({"argv": argv, "platform": "linux", "env": {"THOR_COLUMNS": "100"}}),
            )
            .unwrap(),
        )
    };
    let help = capture(json!(["help"]));
    let exec_help = capture(json!(["help", "exec"]));
    assert_eq!(help["ok"], true);
    assert_ne!(help["value"]["stdout"], exec_help["value"]["stdout"]);
    let unknown = capture(json!(["frobnicate"]));
    assert_eq!(
        unknown["value"],
        json!({"stdout": "", "stderr": "Could not find command \"frobnicate\".\n", "exit": 1})
    );
    assert_eq!(
        process(br#"{"argv": [], "platform": "unknown", "env": {}}"#)["ok"],
        false
    );
    for platform in [
        "linux",
        "macos",
        "windows",
        "arm64-darwin",
        "x64-mingw-ucrt",
    ] {
        let request = json!({"argv": ["help"], "platform": platform, "env": {}});
        assert_eq!(
            process(&serde_json::to_vec(&request).unwrap())["ok"],
            true,
            "{platform}"
        );
    }
    assert_eq!(process(b"{}")["ok"], false);
    // The C boundary accepts null/zero and rejects null/nonzero without dereferencing.
    for length in [0, 1] {
        let reply = unsafe { ukiryu_cli::capture::ukiryu_cli_json(std::ptr::null(), length) };
        let value: Value =
            serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(reply) }.to_bytes()).unwrap();
        assert_eq!(value["ok"], false);
        unsafe {
            drop(std::ffi::CString::from_raw(reply));
        }
    }
}

/// Merge coverage: author commands must use the capture request's register,
/// while retaining the unmodified Ruby author output.
#[cfg(target_os = "linux")]
#[test]
fn author_commands_use_the_captured_register() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/author_data");
    let root = std::fs::canonicalize(data.join("tree")).unwrap();
    let env = json!({
        "HOME": root.join("home"),
        "XDG_DATA_HOME": root.join("xdg"),
        "XDG_DATA_DIRS": root.join("sysdata"),
        "PATH": "/usr/bin:/bin", "SHELL": "/bin/bash", "TERM": "dumb",
        "TZ": "UTC", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"
    });
    for (file, id, register) in [
        ("cases.json", "validate_all_text", "register"),
        ("tools_cases.json", "describe_unknown_tool", "regx"),
    ] {
        let rows: Value = serde_json::from_slice(&std::fs::read(data.join(file)).unwrap()).unwrap();
        let case = rows["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap();
        let mut env = env.clone();
        env["UKIRYU_REGISTER"] = json!(root.join(register));
        let reply = process(
            &serde_json::to_vec(&json!({
                "argv": case["args"], "platform": "linux", "env": env
            }))
            .unwrap(),
        );
        assert_eq!(reply["ok"], true, "{id}: {reply}");
        for field in ["stdout", "stderr", "exit"] {
            assert_eq!(
                reply["value"][field], case["expect"][field],
                "{id}: {field}"
            );
        }
    }
}
