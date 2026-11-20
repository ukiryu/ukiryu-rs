//! Exact invalid-YAML diagnostics through validate's binary and standalone lint.
//! Regenerate with scripts/capture-definition-diagnostics.py and the pinned Ruby oracle.
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use ukiryu::cli::commands::author::{self, AuthorContext, LintInput, LintOptions, LintTarget};
use ukiryu::definition::definition_validator::RemoteSchemaSource;
use ukiryu::render::OutputWrite;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct RestoreDir(PathBuf);

impl Drop for RestoreDir {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).unwrap();
    }
}

#[test]
fn malformed_definition_diagnostics_match_ruby_cli() {
    let fixture: Value =
        serde_json::from_str(include_str!("definition_diagnostics/cases.json")).unwrap();
    assert_eq!(
        fixture["oracle"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 24);
    let root = Scratch(std::env::temp_dir().join(format!(
        "ukiryu-definition-diagnostics-{}",
        std::process::id()
    )));
    std::fs::create_dir_all(root.0.join("defs")).unwrap();
    std::fs::create_dir_all(root.0.join("home")).unwrap();
    // This test binary has one test. Restore cwd before dropping Scratch.
    let _restore = RestoreDir(std::env::current_dir().unwrap());
    std::env::set_current_dir(&root.0).unwrap();
    let mut failures = Vec::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        std::fs::write(
            root.0.join("defs/diagnostic.yaml"),
            case["yaml"].as_str().unwrap(),
        )
        .unwrap();
        // Ruby's LintCommand is standalone, not registered by exe/ukiryu.
        // Exercise the same shared handler without inventing a new CLI verb.
        if case["args"][0] == "lint" {
            let outcome = author::lint(
                LintInput {
                    target: LintTarget::File {
                        path: "defs/diagnostic.yaml".into(),
                    },
                    options: LintOptions {
                        format: Some(
                            case["args"]
                                .as_array()
                                .unwrap()
                                .last()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .into(),
                        ),
                        ..LintOptions::default()
                    },
                },
                &AuthorContext {
                    thor_colors: false,
                    discovery: ukiryu::definition::DiscoveryEnv {
                        home: None,
                        xdg_data_home: None,
                        xdg_data_dirs: None,
                        path: None,
                        opt_root: root.0.join("opt").to_string_lossy().into_owned(),
                    },
                    schema_path_env: None,
                    remote_schema: RemoteSchemaSource::Unavailable,
                    default_register: None,
                    config_colors_disabled: false,
                },
            )
            .expect("standalone lint handler");
            let (mut stdout, mut stderr) = (String::new(), String::new());
            for write in outcome.writes {
                match write {
                    OutputWrite::Stdout(text) => stdout.push_str(&text),
                    OutputWrite::Stderr(text) => stderr.push_str(&text),
                    OutputWrite::File { .. } => panic!("lint unexpectedly wrote a file"),
                }
            }
            let got =
                serde_json::json!({"stdout": stdout, "stderr": stderr, "exit": outcome.exit_code});
            if got != case["expect"] {
                failures.push(format!("{id}\ngot: {got}\nwant: {}", case["expect"]));
            }
            continue;
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_ukiryu"));
        command
            .args(
                case["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap()),
            )
            .current_dir(&root.0)
            .env_clear()
            .env("HOME", root.0.join("home"))
            .env("XDG_DATA_HOME", root.0.join("xdg"))
            .env("XDG_DATA_DIRS", root.0.join("sysdata"))
            .env("UKIRYU_REGISTER", root.0.join("register"))
            .env("PATH", root.0.join("bin"))
            .env("TERM", "dumb")
            .env("TZ", "UTC")
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null());
        // Windows runtime/system discovery retains its required OS root.
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        let output = command.output().expect("diagnostic CLI subprocess");
        let got = serde_json::json!({
            "stdout": String::from_utf8(output.stdout).unwrap(),
            "stderr": String::from_utf8(output.stderr).unwrap(),
            "exit": output.status.code().expect("exit status"),
        });
        if got != case["expect"] {
            failures.push(format!("{id}\ngot: {got}\nwant: {}", case["expect"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} diagnostic CLI cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
}
