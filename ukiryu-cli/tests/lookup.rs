//! Measured CLI lookup goldens from the pinned Ruby oracle. Regenerate with
//! `scripts/capture-cli-lookup.py`; all streams and exit codes compare exactly.
#![cfg(target_os = "linux")]
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn lookup_cases_match_ruby_captures() {
    let data: Value = serde_json::from_str(include_str!("lookup_data/cases.json")).unwrap();
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let base = std::env::temp_dir().join(format!("ukiryu-cli-lookup-{}", std::process::id()));
    let mut count = 0;
    for case in data["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let root = base.join(id);
        if id.starts_with("exec_") {
            copy_tree(&tests.join("execute_data/fixture"), &root);
        } else {
            copy_tree(&tests.join("inspect_data/register"), &root.join("register"));
        }
        for directory in ["home", "bin", "xdg", "sysdata"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        for name in case["executables"].as_array().unwrap() {
            let path = root.join("bin").join(name.as_str().unwrap());
            fs::write(&path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for (name, interface) in case["implements"].as_object().unwrap() {
            let directory = root.join("register/tools").join(name).join("default");
            let path = fs::read_dir(directory)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let mut yaml = fs::read_to_string(&path).unwrap();
            yaml.push_str(&format!("\nimplements: {}\n", interface.as_str().unwrap()));
            fs::write(path, yaml).unwrap();
        }
        for name in case["profiles"].as_array().unwrap() {
            let directory = root
                .join("register/tools")
                .join(name.as_str().unwrap())
                .join("default");
            let path = fs::read_dir(directory)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let yaml = fs::read_to_string(&path).unwrap();
            let profiles = yaml.split_once("execution_profiles:").unwrap().1;
            fs::write(path, format!("{yaml}\nprofiles:{profiles}")).unwrap();
        }
        if id.starts_with("list_") {
            fs::write(
                root.join("register/tools/zeta/index.yaml"),
                "---\nukiryu_schema: '1.0'\nname: zeta\ninterface: zeta\nimplementations: []\n",
            )
            .unwrap();
        }
        let argv: Vec<&str> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let result = Command::new(env!("CARGO_BIN_EXE_ukiryu"))
            .args(argv)
            .current_dir(&root)
            .env_clear()
            .env("HOME", root.join("home"))
            .env("XDG_DATA_HOME", root.join("xdg"))
            .env("XDG_DATA_DIRS", root.join("sysdata"))
            .env("PATH", format!("{}/bin:/usr/bin:/bin", root.display()))
            .env("SHELL", "/bin/bash")
            .env("UKIRYU_REGISTER", root.join("register"))
            .env("TERM", "dumb")
            .env("TZ", "UTC")
            .env("LANG", "C.UTF-8")
            .output()
            .unwrap();
        let normalize = |bytes: Vec<u8>| {
            String::from_utf8(bytes)
                .unwrap()
                .replace(root.to_str().unwrap(), "{root}")
        };
        assert_eq!(
            i64::from(result.status.code().unwrap()),
            case["status"].as_i64().unwrap(),
            "{id}"
        );
        assert_eq!(
            normalize(result.stdout),
            case["stdout"].as_str().unwrap(),
            "{id}: stdout"
        );
        assert_eq!(
            normalize(result.stderr),
            case["stderr"].as_str().unwrap(),
            "{id}: stderr"
        );
        count += 1;
    }
    assert_eq!(count, 17);
    fs::remove_dir_all(base).unwrap();
}
