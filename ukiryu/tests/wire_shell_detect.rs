//! All 17 frozen shell rows through the wire, applying the traced D2
//! expectations from migrated.json. Virtual platforms make this portable
//! across every CI host, including Windows; no host env is inherited.
use serde_json::{json, Value};

fn wire(platform: &str, env: Value) -> Value {
    ukiryu::wire::process(
        json!({"op": "resolve", "method": "shell_detect", "platform": platform, "env": env})
            .to_string()
            .as_bytes(),
    )
}

#[test]
fn all_shell_rows_match_frozen_or_traced_d2_expectations() {
    let rows: Vec<Value> = include_str!("../../corpus/conformance/cases/resolve.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["op"] == "resolve.shell_detect")
        .collect();
    assert_eq!(rows.len(), 17);
    let rules: Value = serde_json::from_str(include_str!("conformance/migrated.json")).unwrap();
    let mut migrated = 0;
    for row in rows {
        let migration = rules["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == row["id"]);
        let want = migration.map_or(&row["expect"], |m| &m["expect"]);
        migrated += usize::from(migration.is_some());
        for platform in row["applicability"]["platforms"].as_array().unwrap() {
            let reply = wire(platform.as_str().unwrap(), row["input"]["env"].clone());
            let have = if reply["ok"] == true {
                json!({"ok": reply["value"]})
            } else {
                json!({"error": {"class": reply["error"]["source_class"], "message": reply["error"]["message"]}})
            };
            assert_eq!(&have, want, "{} ({platform}): {reply}", row["id"]);
            if let Some(detail) = migration.and_then(|m| m.get("detail")) {
                assert_eq!(&reply["error"]["detail"], detail, "{}", row["id"]);
            }
        }
    }
    assert_eq!(migrated, 10);
}

#[test]
fn explicit_snapshot_covers_windows_precedence_and_missing_values() {
    for (env, shell) in [
        (
            json!({"psmodulepath": "", "MSYSTEM": "MINGW64"}),
            "powershell",
        ),
        (json!({"MSYSTEM": ""}), "bash"),
        (json!({"mingw_prefix": "/mingw64"}), "bash"),
        (json!({"WSL_DISTRO": "Ubuntu"}), "bash"),
        (
            json!({"UKIRYU_TEST_SHELL": "cmd", "PSModulePath": "x"}),
            "cmd",
        ),
    ] {
        assert_eq!(
            wire("windows", env)["value"]["detect"],
            json!({"$sym": shell})
        );
    }
    for (platform, env, message) in [
        ("linux", json!({}), "Unable to detect shell: SHELL not set"),
        ("linux", json!({"SHELL": null}), "Unable to detect shell: SHELL not set"),
        ("windows", json!({"PSModulePath": null, "SHELL": "/bin/bash"}),
         "Unable to detect shell: none of PSModulePath, MSYSTEM, MINGW_PREFIX or WSL_DISTRO is set"),
    ] {
        let reply = wire(platform, env);
        assert_eq!(reply["error"]["code"], "UNKNOWN_SHELL");
        assert_eq!(reply["error"]["message"], message);
    }
    let reply = wire("linux", json!({"UKIRYU_TEST_SHELL": "unix"}));
    assert_eq!(reply["error"]["detail"]["variant"], "PlatformGroup");
}

#[test]
fn invalid_snapshots_are_rejected() {
    assert_eq!(
        wire("plan9", json!({}))["error"]["code"],
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        wire("linux", json!({"SHELL": false}))["error"]["code"],
        "INVALID_ARGUMENT"
    );
    for request in [
        json!({"op": "resolve", "method": "shell_detect", "platform": "linux"}),
        json!({"op": "resolve", "method": "shell_detect", "env": {}}),
        json!({"op": "resolve", "method": "shell_detect", "platform": "linux", "env": []}),
    ] {
        let reply = ukiryu::wire::process(request.to_string().as_bytes());
        assert_eq!(reply["error"]["code"], "INVALID_REQUEST");
    }
}
