//! End-to-end tests of the compiled `ukiryu` binary.
//!
//! Every case runs `CARGO_BIN_EXE_ukiryu` as a real process with a cleared,
//! explicit environment and compares its stdout, stderr, exit status and the
//! files it leaves against the Ruby CLI captures that the handler tests use:
//!
//! - `inspect_goldens.json` (version, system, config get|set|unset), captured
//!   by K22-CLI-INSPECT from the pinned exe/ukiryu (ukiryu/ukiryu ab6df35);
//! - `author_data/cases.json` (validate, definitions, extract, lint), captured
//!   by K22-CLI-AUTHOR from the same oracle.
//!
//! - `execute_data/cases.json` (exec, run-file, cache), captured by
//!   K22-CLI-EXECUTE from the same oracle.
//! - `inspect_remainder_goldens.json` (config list, list, commands,
//!   register, resolve), captured by B09-CLI-INSPECT-REMAINDER from the same
//!   oracle, and the corpus `cli-output` rows of those verbs
//!   (`corpus/conformance/cases/cli-output.jsonl`).
//!
//! - the `info` and `opts` rows of the conformance corpus
//!   (`corpus/conformance/cases/cli-output.jsonl`) and
//!   `info_opts_data/cases.json`, captured from the same oracle in the
//!   corpus environment.
//!
//! Author rows come from unmodified pinned Ruby commands. Standalone lint/docs
//! handlers have their own replay; they are not added as CLI verbs. Every
//! pipe-based author row compares source output and status. Terminal captures
//! are replayed by the author handler test.
//!
//! Recorded deviations (all decided outside this file):
//! - shell detection is strict (DECISIONS.md D2): inspect rows with a `note`
//!   expect the shell error after the same partial output, and extract rows
//!   set `SHELL=/bin/bash` where Ruby searched PATH for bash;
//! - argument errors print clap's wording; only the status (1) is compared;
//! - the persistent config.yml is compared as YAML data, and YAML engine
//!   messages by their recorded prefix;
//! - anonymous class addresses use the comparison-only canonicalization
//!   documented in `execute_data/README.md` (shared with `execute.rs`).
#![cfg(unix)]

#[cfg(target_os = "linux")]
#[path = "execute_data/comparison.rs"]
mod comparison;
#[cfg(target_os = "linux")]
use comparison::canonical_addresses;

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_ukiryu");

struct Output {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(args: &[String], env: &[(String, String)], cwd: &Path, stdin: Option<&str>) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the ukiryu binary");
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(text) = stdin {
            // The binary may exit before reading stdin; the closed pipe is
            // then expected, and its status and output are still checked.
            if let Err(err) = pipe.write_all(text.as_bytes()) {
                assert_eq!(
                    err.kind(),
                    std::io::ErrorKind::BrokenPipe,
                    "write stdin: {err}"
                );
            }
        }
    }
    let out = child.wait_with_output().unwrap();
    Output {
        stdout: String::from_utf8(out.stdout).expect("UTF-8 stdout"),
        stderr: String::from_utf8(out.stderr).expect("UTF-8 stderr"),
        code: out.status.code().expect("exit status"),
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ukiryu-cli-bin-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

// ------------------------------------------------------------------ inspect

#[cfg(target_os = "linux")]
fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

#[cfg(target_os = "linux")]
fn yaml_data(text: &str) -> Vec<yeptris::Value> {
    yeptris::parse_stream(text.as_bytes()).expect("YAML parses")
}

#[cfg(target_os = "linux")]
fn install_executable(dir: &Path, name: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The inspect captures were made on a Linux host (rows for other platforms
/// force Ruby's host_os, which the binary cannot), so they replay on Linux.
#[cfg(target_os = "linux")]
#[test]
fn inspect_goldens_through_the_binary() {
    let goldens: Value = serde_json::from_str(include_str!("inspect_goldens.json")).unwrap();
    let mut failures = Vec::new();
    let mut replayed = 0;
    for case in goldens["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if case["platform"] != "linux" {
            continue;
        }
        let root = scratch(&format!("inspect-{name}"));
        let (home, bin, register) = (root.join("home"), root.join("bin"), root.join("register"));
        for dir in [&home, &bin, &register] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for b in strings(&case["bins"]) {
            install_executable(&bin, &b);
        }
        for (rel, body) in case["home_before"].as_object().unwrap() {
            let path = home.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body.as_str().unwrap()).unwrap();
        }
        let s = |p: &Path| p.to_str().unwrap().to_owned();
        let mut env = vec![
            ("HOME".to_owned(), s(&home)),
            ("PATH".to_owned(), s(&bin)),
            ("UKIRYU_REGISTER".to_owned(), s(&register)),
        ];
        for (k, v) in case["env"].as_object().unwrap() {
            env.push((k.clone(), v.as_str().unwrap().to_owned()));
        }
        let subst = |golden: &Value| {
            golden
                .as_str()
                .unwrap()
                .replace("{HOME}", &s(&home))
                .replace("{BIN}", &s(&bin))
                .replace("{REGISTER}", &s(&register))
        };
        let (want_out, want_err) = (subst(&case["stdout"]), subst(&case["stderr"]));
        let want_code = case["exit"].as_i64().unwrap() as i32;
        let args = strings(&case["args"]);
        let got = run(&args, &env, &root, None);
        replayed += 1;
        let mut fail = |what: String| failures.push(format!("{name}: {what}"));
        let parse_error = ukiryu_cli::arguments::command()
            .try_get_matches_from(std::iter::once("ukiryu".to_owned()).chain(args.clone()))
            .is_err();
        if parse_error {
            // Thor's arity wording differs from clap's; the status is kept.
            if got.code != 1 || want_code != 1 || !got.stdout.is_empty() {
                fail(format!(
                    "argument error: exit {} {:?}",
                    got.code, got.stdout
                ));
            }
        } else if !case["note"].is_null() {
            // D2: the strict shell error after the same partial output.
            let cut = want_out.rfind("\x1b[2mCurrent shell: ").unwrap();
            if got.stdout != want_out[..cut]
                || !got
                    .stderr
                    .starts_with("[Ukiryu::Errors::UnknownShellError] ")
                || got.code != 1
            {
                fail(format!(
                    "D2 row: exit {} out {:?} err {:?}",
                    got.code, got.stdout, got.stderr
                ));
            }
        } else {
            const WARNING: &str = "\x1b[31mWarning: Failed to load config file: ";
            let same_out = match (want_out.split_once('\n'), got.stdout.split_once('\n')) {
                (Some((w, wr)), Some((g, gr))) if w.starts_with(WARNING) => {
                    g.starts_with(WARNING) && g.ends_with("\x1b[0m") && wr == gr
                }
                _ => got.stdout == want_out,
            };
            if !same_out {
                fail(format!("stdout {:?}\nexpected {want_out:?}", got.stdout));
            }
            if got.stderr != want_err {
                fail(format!("stderr {:?}\nexpected {want_err:?}", got.stderr));
            }
            if got.code != want_code {
                fail(format!("exit {} expected {want_code}", got.code));
            }
        }
        let want_files = case["home_after"].as_object().unwrap();
        let got_files = files_under(&home);
        let keys: Vec<String> = want_files.keys().cloned().collect();
        if got_files != keys {
            fail(format!("files {got_files:?} expected {keys:?}"));
        } else {
            for (rel, body) in want_files {
                let text = std::fs::read_to_string(home.join(rel)).unwrap();
                if yaml_data(&text) != yaml_data(body.as_str().unwrap()) {
                    fail(format!("{rel} = {text:?}"));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
    assert!(replayed >= 40, "only {replayed} inspect rows replayed");
    assert!(
        failures.is_empty(),
        "{} of {replayed} differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

// ------------------------------------------------------------------ author

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/author_data")
}

fn copy_tree(from: &Path, to: &Path, mtime: SystemTime) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target, mtime);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
            set_mtime(&target, mtime);
        }
    }
}

fn set_mtime(path: &Path, mtime: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}

/// Per-case fixture additions, as the capture made them.
fn setup(id: &str, root: &Path, mtime: SystemTime) {
    if id.starts_with("lint_all_empty_tools") {
        std::fs::create_dir_all(root.join("emptyreg/tools")).unwrap();
    }
    if id.starts_with("definitions_path_bundled") || id.starts_with("definitions_list_bundled") {
        std::fs::create_dir_all(root.join("tooldir/bin")).unwrap();
        let share = root.join("tooldir/share/ukiryu/zip");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::copy(root.join("defs/digit.yaml"), share.join("9.20.yaml")).unwrap();
        set_mtime(&share.join("9.20.yaml"), mtime);
    }
}

fn case_files_dir(args: &[String], root: &Path) -> PathBuf {
    let has = |a: &str| args.iter().any(|x| x == a);
    if has("--output-dir") {
        root.join("outdocs")
    } else if has("--output") {
        root.join("out.md")
    } else if has("-o") {
        root.join("out.yaml")
    } else {
        root.join("xdg")
    }
}

fn all_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        if dir.is_file() {
            out.push(dir.to_string_lossy().into_owned());
        }
        return;
    };
    for entry in rd {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            all_files(&entry.path(), out);
        } else {
            out.push(entry.path().to_string_lossy().into_owned());
        }
    }
}

fn same_yaml_data(actual: &str, expected: &str) -> bool {
    let split = |t: &str| {
        let i = t.find('\x1b').unwrap_or(t.len());
        (t[..i].to_owned(), t[i..].to_owned())
    };
    let ((ay, at), (ey, et)) = (split(actual), split(expected));
    let parse = |t: &str| ukiryu::from_yaml::<Value>(t).ok();
    at == et && parse(&ay).is_some() && parse(&ay) == parse(&ey)
}

fn sorted_lines(s: &str) -> Vec<&str> {
    let mut v: Vec<&str> = s.lines().collect();
    v.sort();
    v
}

fn same_prefix(actual: &str, expected: &str, marker: &str) -> bool {
    match expected.find(marker) {
        Some(i) => {
            actual.len() >= i + marker.len()
                && actual[..i + marker.len()] == expected[..i + marker.len()]
        }
        None => actual == expected,
    }
}

/// Only commands registered by the original CLI run through the binary.
fn argv(case: &Value) -> Option<Vec<String>> {
    (case["runner"] == "exe").then(|| strings(&case["args"]))
}

fn author_env(case: &Value, root: &str) -> Vec<(String, String)> {
    let mut env = vec![
        ("HOME".to_owned(), format!("{root}/home")),
        ("XDG_DATA_HOME".to_owned(), format!("{root}/xdg")),
        ("XDG_DATA_DIRS".to_owned(), format!("{root}/sysdata")),
        ("PATH".to_owned(), format!("{root}/bin")),
        ("UKIRYU_REGISTER".to_owned(), format!("{root}/register")),
        (
            "TERM".to_owned(),
            if case["pty"] == true { "xterm" } else { "dumb" }.to_owned(),
        ),
        ("TZ".to_owned(), "UTC".to_owned()),
        ("LANG".to_owned(), "C.UTF-8".to_owned()),
    ];
    if case["args"][0] == "extract" {
        // D2: Ruby found bash on PATH; the strict contract names the shell.
        env.push(("SHELL".to_owned(), "/bin/bash".to_owned()));
    }
    if case["suite"] == "tools" {
        for (key, value) in [
            ("PATH", format!("{root}/tbin:/usr/bin:/bin")),
            ("UKIRYU_REGISTER", format!("{root}/regx")),
            ("SHELL", "/bin/bash".to_owned()),
        ] {
            env.retain(|(name, _)| name != key);
            env.push((key.to_owned(), value));
        }
    }
    for (k, v) in case["env"].as_object().unwrap() {
        if k == "RUBYOPT" {
            continue; // capture-only: loads fileutils for the target trace
        }
        env.retain(|(name, _)| name != k);
        if let Some(v) = v.as_str() {
            env.push((k.clone(), v.replace("{root}", root)));
        }
    }
    env
}

/// Replays the author captures of one contract; returns the replayed count.
fn replay_author(name: &str) -> usize {
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join(name)).unwrap()).unwrap();
    let mtime = UNIX_EPOCH + Duration::from_secs(fixture["fixed_mtime_unix"].as_u64().unwrap());
    let mut failures = Vec::new();
    let mut replayed = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let mode = case["mode"].as_str().unwrap();
        let Some(args) = argv(case) else { continue };
        let root_path = scratch(&format!("author-{id}"));
        copy_tree(&data_dir().join("tree"), &root_path, mtime);
        std::fs::create_dir_all(root_path.join("home")).unwrap();
        std::fs::create_dir_all(root_path.join("bin")).unwrap();
        setup(id, &root_path, mtime);
        let root = root_path.to_string_lossy().into_owned();
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.replace("{root}", &root))
            .collect();
        let env = author_env(case, &root);
        if mode == "tty" {
            let capture = Command::new("/usr/bin/python3")
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/replay-cli-terminal.py"),
                )
                .arg(BIN)
                .args(&args)
                .env_clear()
                .envs(env.iter().map(|(k, v)| (k, v)))
                .current_dir(&root_path)
                .output()
                .unwrap();
            assert!(
                capture.status.success(),
                "{id}: {}",
                String::from_utf8_lossy(&capture.stderr)
            );
            let terminal: Value = serde_json::from_slice(&capture.stdout).unwrap();
            let expected = case["tty_output"]
                .as_str()
                .unwrap()
                .replace("{root}", &root);
            assert_eq!(
                terminal["output"].as_str().unwrap(),
                expected,
                "{id}: raw PTY bytes"
            );
            assert_eq!(terminal["exit"], case["expect"]["exit"], "{id}: PTY exit");
            replayed += 1;
            std::fs::remove_dir_all(&root_path).unwrap();
            continue;
        }
        let got = run(&args, &env, &root_path, case["stdin"].as_str());
        replayed += 1;
        let expect = &case["expect"];
        let exp = |k: &str| {
            let mut text = expect[k].as_str().unwrap_or("").replace("{root}", &root);
            for relative in ["defs/invalid_yaml.yaml", "defs/list.yaml"] {
                let digest = ukiryu::definition::source::sha256_hex(&format!("{root}/{relative}"));
                text = text.replace(&format!("{{sha256:{relative}}}"), &digest);
            }
            text
        };
        let mut fail = |what: String| failures.push(format!("{id}: {what}"));

        let (es, ee) = (exp("stdout"), exp("stderr"));
        let prefix = case["prefix"].as_str().unwrap_or("");
        let ok_out = match mode {
            "lines" => sorted_lines(&got.stdout) == sorted_lines(&es),
            "prefix" => same_prefix(&got.stdout, &es, prefix),
            "yaml_data" => same_yaml_data(&got.stdout, &es),
            _ => got.stdout == es,
        };
        let ok_err = match mode {
            "prefix" => same_prefix(&got.stderr, &ee, prefix),
            _ => got.stderr == ee,
        };
        if !ok_out {
            fail(format!("stdout\n{:?}\nexpected\n{es:?}", got.stdout));
        }
        if !ok_err {
            fail(format!("stderr\n{:?}\nexpected\n{ee:?}", got.stderr));
        }
        let code = expect["exit"].as_i64().unwrap() as i32;
        if got.code != code {
            fail(format!("exit {} expected {code}", got.code));
        }
        if let Some(files) = expect["files"].as_array() {
            let mut want: Vec<String> = files
                .iter()
                .map(|f| f.as_str().unwrap().replace("{root}", &root))
                .collect();
            want.sort();
            let mut have = Vec::new();
            all_files(&case_files_dir(&args, &root_path), &mut have);
            have.sort();
            if have != want {
                fail(format!("files {have:?} expected {want:?}"));
            }
        }
        if let Some(contents) = expect["file_contents"].as_object() {
            for (rel, text) in contents {
                let have = std::fs::read_to_string(root_path.join(rel)).unwrap_or_default();
                if have != text.as_str().unwrap() {
                    fail(format!("contents of {rel}\n{have:?}"));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&root_path);
    }
    assert!(
        failures.is_empty(),
        "{} of {replayed} source rows differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    replayed
}

#[test]
fn author_source_rows_through_the_binary() {
    let replayed = replay_author("cases.json");
    assert!(replayed >= 70, "only {replayed} source rows replayed");
}

#[cfg(target_os = "linux")]
#[test]
fn describe_and_executable_validation_through_the_binary() {
    assert_eq!(replay_author("tools_cases.json"), 51);
}

// ------------------------------------------------------------------ execute

/// No non-terminal execute rows remain outside the Ruby contract.
#[cfg(target_os = "linux")]
const EXECUTE_UNQUALIFIED: [&str; 0] = [];

/// The environment `capture/cli.rb` gives the Ruby CLI, plus the case's own
/// variables (`null` removes one).
#[cfg(target_os = "linux")]
fn execute_env(root: &str, case_env: &Value) -> Vec<(String, String)> {
    let mut vars: Vec<(String, Option<String>)> = vec![
        ("HOME".into(), Some(format!("{root}/home"))),
        ("XDG_DATA_HOME".into(), Some(format!("{root}/xdg"))),
        ("XDG_DATA_DIRS".into(), Some(format!("{root}/sysdata"))),
        ("PATH".into(), Some("/usr/bin:/bin".into())),
        ("SHELL".into(), Some("/bin/bash".into())),
        ("UKIRYU_REGISTER".into(), Some(format!("{root}/register"))),
        ("TERM".into(), Some("dumb".into())),
        ("TZ".into(), Some("UTC".into())),
        ("LANG".into(), Some("C.UTF-8".into())),
    ];
    if let Some(map) = case_env.as_object() {
        for (k, v) in map {
            vars.retain(|(name, _)| name != k);
            vars.push((k.clone(), v.as_str().map(str::to_owned)));
        }
    }
    vars.into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
}

/// Masks the values of timing fields, whose values differ per run.
#[cfg(target_os = "linux")]
fn mask_timing(text: &str) -> String {
    const KEYS: [&str; 4] = [
        "started_at",
        "finished_at",
        "duration_seconds",
        "formatted_duration",
    ];
    text.split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim_start().trim_start_matches('"');
            let timing = KEYS.iter().any(|k| {
                trimmed.starts_with(&format!("{k}:")) || trimmed.starts_with(&format!("{k}\":"))
            }) || line.starts_with("Duration: ");
            match (timing, line.find(':')) {
                (true, Some(i)) => format!("{}: <masked>\n", &line[..i]),
                _ => line.to_owned(),
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn mask_timing_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if matches!(
                    k.as_str(),
                    "started_at" | "finished_at" | "duration_seconds" | "formatted_duration"
                ) {
                    *v = Value::String("<masked>".into());
                } else {
                    mask_timing_value(v);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask_timing_value),
        _ => {}
    }
}

/// A YAML response document as loaded values (Psych's and the emitter's
/// scalar styles differ; YAML responses are compared by value).
#[cfg(target_os = "linux")]
fn response_yaml(text: &str) -> Option<Value> {
    if !text.starts_with("---") {
        return None;
    }
    let unmasked = text.replace(": <masked>", ": '<masked>'");
    let mut value: Value = ukiryu::from_yaml(&unmasked).ok()?;
    mask_timing_value(&mut value);
    Some(value)
}

#[cfg(target_os = "linux")]
fn response_json(text: &str) -> Option<Value> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    mask_timing_value(&mut value);
    Some(value)
}

/// Equal stdout: bytes, except YAML responses (by value).
#[cfg(target_os = "linux")]
fn same_stdout(want: &str, got: &str) -> bool {
    let want = canonical_addresses(want);
    let got = canonical_addresses(got);
    match (response_yaml(&want), response_yaml(&got)) {
        (Some(a), Some(b)) => a == b,
        _ => want == got,
    }
}

/// Equal written file: bytes, except JSON (timing masked).
#[cfg(target_os = "linux")]
fn same_file(want: &Option<String>, got: &Option<String>) -> bool {
    match (want, got) {
        (Some(a), Some(b)) => {
            let a = canonical_addresses(a);
            let b = canonical_addresses(b);
            match (response_json(&a), response_json(&b)) {
                (Some(x), Some(y)) => x == y,
                _ => a == b,
            }
        }
        (a, b) => a == b,
    }
}

/// Counts of replayed execute rows by contract.
#[cfg(target_os = "linux")]
#[derive(Default, Debug, PartialEq)]
struct ExecuteCounts {
    exact: usize,
    unqualified: usize,
    terminal: usize,
}

#[cfg(target_os = "linux")]
fn replay_execute() -> (ExecuteCounts, Vec<String>) {
    let data: Value = serde_json::from_str(include_str!("execute_data/cases.json")).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/execute_data/fixture");
    let mut counts = ExecuteCounts::default();
    let mut failures = Vec::new();
    for case in data["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        if case["pty"].as_bool().unwrap_or(false) {
            counts.terminal += 1;
            continue;
        }
        let root = scratch(&format!("execute-{id}"));
        copy_tree(&fixture, &root, SystemTime::now());
        for dir in ["home", "xdg", "sysdata"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let r = root.to_str().unwrap().to_owned();
        let got = run(
            &strings(&case["args"]),
            &execute_env(&r, &case["env"]),
            &root,
            Some(case["stdin"].as_str().unwrap_or("")),
        );
        let stdout = mask_timing(&got.stdout.replace(&r, "{root}"));
        let stderr = mask_timing(&got.stderr.replace(&r, "{root}"));
        let files: Vec<(String, Option<String>)> = case["files"]
            .as_object()
            .map(|m| {
                m.keys()
                    .map(|name| (name.clone(), std::fs::read_to_string(root.join(name)).ok()))
                    .collect()
            })
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&root);

        if EXECUTE_UNQUALIFIED.contains(&id) {
            let diagnostics: Value =
                serde_json::from_str(include_str!("execute_data/yeptris_diagnostics.json"))
                    .unwrap();
            assert_eq!(
                i64::from(got.code),
                case["status"].as_i64().unwrap(),
                "{id}"
            );
            assert_eq!(stdout, case["stdout"].as_str().unwrap(), "{id}");
            assert!(case["stderr"]
                .as_str()
                .unwrap()
                .starts_with("Invalid YAML in request file: "));
            assert_eq!(stderr, diagnostics[id].as_str().unwrap(), "{id}");
            counts.unqualified += 1;
            continue;
        }
        let want_stdout = mask_timing(case["stdout"].as_str().unwrap_or(""));
        let want_stderr = mask_timing(case["stderr"].as_str().unwrap_or(""));
        let want_files: Vec<(String, Option<String>)> = case["files"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned)))
                    .collect()
            })
            .unwrap_or_default();
        let files_match = want_files.len() == files.len()
            && want_files
                .iter()
                .zip(&files)
                .all(|((wn, wc), (gn, gc))| wn == gn && same_file(wc, gc));
        if i64::from(got.code) == case["status"].as_i64().unwrap()
            && canonical_addresses(&stderr) == canonical_addresses(&want_stderr)
            && same_stdout(&want_stdout, &stdout)
            && files_match
        {
            counts.exact += 1;
        } else {
            failures.push(format!(
                "{id}: status {} (want {}), stdout {stdout:?} (want {want_stdout:?}), \
                 stderr {stderr:?} (want {want_stderr:?}), files match {files_match}",
                got.code, case["status"]
            ));
        }
    }
    (counts, failures)
}

/// The compiled binary replays the K22 execute captures for exec, run-file
/// and cache. Terminal rows are counted (they need a pty); all other rows
/// match the Ruby capture.
#[cfg(target_os = "linux")]
#[test]
fn execute_cases_through_the_binary() {
    let (counts, failures) = replay_execute();
    eprintln!("execute rows through the binary: {counts:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert_eq!(counts.unqualified, EXECUTE_UNQUALIFIED.len(), "{counts:?}");
    assert_eq!(counts.terminal, 12, "{counts:?}");
    assert_eq!(counts.exact, 92, "{counts:?}");
}

/// The binary comparator permits address variation but keeps every diagnostic
/// and response field exact across YAML, JSON and table responses.
#[cfg(target_os = "linux")]
#[test]
fn anonymous_error_comparison_preserves_response_fields() {
    let data: Value = serde_json::from_str(include_str!("execute_data/cases.json")).unwrap();
    for id in [
        "exec_register_unknown_param",
        "runfile_execute",
        "runfile_execute_json",
        "runfile_execute_table",
    ] {
        let case = data["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap();
        let want = case["stdout"].as_str().unwrap();
        let varied = canonical_addresses(want).replace("0x<address>", "0xABC123");
        assert!(same_stdout(want, &varied), "{id}");
        assert!(
            same_file(&Some(want.to_owned()), &Some(varied.clone())),
            "{id}"
        );
        for (from, to) in [
            ("NoMethodError", "ArgumentError"),
            ("undefined method", "unknown method"),
            ("for an instance", "for a class"),
            ("Class:", "OtherClass:"),
            ("bogus=", "different="),
            ("stdin=", "different="),
            ("exit_code: 1", "exit_code: 2"),
            ("\"exit_code\": 1", "\"exit_code\": 2"),
            ("status: error", "status: success"),
            ("\"status\": \"error\"", "\"status\": \"success\""),
        ] {
            if varied.contains(from) {
                let changed = varied.replace(from, to);
                assert!(!same_stdout(want, &changed), "{id}: {from}");
                assert!(
                    !same_file(&Some(want.to_owned()), &Some(changed)),
                    "{id}: {from}"
                );
            }
        }
    }
}

// ------------------------------------------------------------------ grammar

fn simple(args: &[&str]) -> Output {
    let dir = scratch(&format!("grammar-{}", args.join("_").replace('/', "_")));
    let env = vec![
        ("HOME".to_owned(), dir.to_str().unwrap().to_owned()),
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("SHELL".to_owned(), "/bin/bash".to_owned()),
    ];
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let out = run(&args, &env, &dir, None);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn argument_errors_exit_1_on_stderr() {
    // `docs` stays unregistered, as in the source cli.rb (root ruling).
    for args in [
        &["docs", "generate", "x"][..],
        &["bogus"],
        &["version", "--bogus"],
    ] {
        let got = simple(args);
        assert_eq!(got.code, 1, "{args:?}");
        assert_eq!(got.stdout, "", "{args:?}");
        assert!(!got.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn help_and_version_succeed_on_stdout() {
    for args in [&["list", "--help"][..], &["exec", "-h"]] {
        let got = simple(args);
        assert_eq!(got.code, 0, "{args:?}: {}", got.stderr);
        assert!(got.stdout.contains("Usage"), "{args:?}: {:?}", got.stdout);
    }
    // `help` is Thor's command list (the cli-output.help row pins its bytes).
    let got = simple(&["help"]);
    assert_eq!(got.code, 0, "help: {}", got.stderr);
    assert!(
        got.stdout.starts_with("ukiryu commands:\n"),
        "{:?}",
        got.stdout
    );
    // Thor prints the command list (stdout, status 0) when no command or no
    // subcommand of a group is given (captured: logs/grammar-3.log).
    let cases: [(&[&str], &str); 2] = [(&[], "validate"), (&["validate"], "file")];
    for (args, word) in cases {
        let got = simple(args);
        assert_eq!(got.code, 0, "{args:?}: {}", got.stderr);
        assert!(got.stdout.contains(word), "{args:?}: {:?}", got.stdout);
        assert_eq!(got.stderr, "", "{args:?}");
    }
}

#[test]
fn version_verb_preserves_the_frozen_cli_version() {
    let got = simple(&["version"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(got.stdout, "\x1b[36mUkiryu version 0.3.1\x1b[0m\n");
}

#[test]
fn an_output_file_that_cannot_be_written_is_an_errno_error() {
    let root = scratch("extract-enoent");
    copy_tree(&data_dir().join("tree"), &root, SystemTime::now());
    let r = root.to_str().unwrap();
    let env = vec![
        ("HOME".to_owned(), format!("{r}/home")),
        ("PATH".to_owned(), format!("{r}/xbin:/usr/bin:/bin")),
        ("SHELL".to_owned(), "/bin/bash".to_owned()),
    ];
    let args: Vec<String> = ["extract", "ukx-native", "-o", "nodir/out.yaml"]
        .iter()
        .map(|a| (*a).to_owned())
        .collect();
    let got = run(&args, &env, &root, None);
    assert_eq!(got.code, 1, "{}", got.stderr);
    assert_eq!(got.stdout, "");
    assert_eq!(
        got.stderr,
        "[Errno::ENOENT] No such file or directory @ rb_sysopen - nodir/out.yaml\n"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------- info/opts

#[cfg(target_os = "linux")]
fn conformance_dir() -> PathBuf {
    std::env::var_os("UKIRYU_CONFORMANCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance"))
}

/// Runs `argv` in the corpus `cli-output` environment
/// (`corpus/oracle-ruby/lib/ops.rb`), with `${INPUTS}` substituted in the
/// arguments and substituted back (with `${HOME}`) in the output. `PATH`
/// holds only a `bash` link, so no register tool is installed.
#[cfg(target_os = "linux")]
fn corpus_cli(name: &str, argv: &[String], extra: &[(String, String)]) -> Output {
    let inputs = std::fs::canonicalize(conformance_dir().join("inputs")).unwrap();
    let inputs = inputs.to_str().unwrap();
    let root = scratch(name);
    let home = root.join("home");
    let bin = root.join("bin");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink("/bin/bash", bin.join("bash")).unwrap();
    let h = home.to_str().unwrap();
    let mut env: Vec<(String, String)> = [
        ("HOME", h.to_owned()),
        ("TZ", "UTC".to_owned()),
        ("LANG", "C.UTF-8".to_owned()),
        ("LC_ALL", "C.UTF-8".to_owned()),
        ("SHELL", "/bin/bash".to_owned()),
        ("UKIRYU_REGISTER", format!("{inputs}/register")),
        ("COLUMNS", "100".to_owned()),
        ("THOR_COLUMNS", "100".to_owned()),
        ("TERM", "dumb".to_owned()),
        ("PATH", bin.to_str().unwrap().to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    env.extend(extra.iter().cloned());
    let args: Vec<String> = argv
        .iter()
        .map(|a| a.replace("${INPUTS}", inputs))
        .collect();
    let out = run(&args, &env, &home, Some(""));
    let scrub = |text: &str| text.replace(inputs, "${INPUTS}").replace(h, "${HOME}");
    let out = Output {
        stdout: scrub(&out.stdout),
        stderr: scrub(&out.stderr),
        code: out.code,
    };
    let _ = std::fs::remove_dir_all(&root);
    out
}

/// Every `info` and `opts` row of the conformance corpus, byte for byte.
#[cfg(target_os = "linux")]
#[test]
fn info_and_opts_replay_the_corpus_rows() {
    let cases = std::fs::read_to_string(conformance_dir().join("cases/cli-output.jsonl")).unwrap();
    let mut ids = Vec::new();
    for line in cases.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let id = row["id"].as_str().unwrap();
        let argv = strings(&row["input"]["argv"]);
        if !matches!(argv[0].as_str(), "info" | "opts") {
            continue;
        }
        let want = &row["expect"]["ok"];
        let got = corpus_cli(id, &argv, &[]);
        assert_eq!(got.stdout, want["stdout"].as_str().unwrap(), "{id} stdout");
        assert_eq!(got.stderr, want["stderr"].as_str().unwrap(), "{id} stderr");
        assert_eq!(
            i64::from(got.code),
            want["exit"].as_i64().unwrap(),
            "{id} exit"
        );
        ids.push(id.to_owned());
    }
    assert_eq!(
        ids,
        [
            "cli-output.info-absent",
            "cli-output.info-unknown",
            "cli-output.opts-jq",
            "cli-output.opts-jq-process",
        ]
    );
}

/// The Thor `help`, `help exec`, unknown-verb and `validate lint` rows.
#[cfg(target_os = "linux")]
#[test]
fn thor_help_and_unknown_command_rows_replay_the_corpus() {
    let cases = std::fs::read_to_string(conformance_dir().join("cases/cli-output.jsonl")).unwrap();
    let wanted = [
        "cli-output.help",
        "cli-output.help-exec",
        "cli-output.unknown-verb",
        "cli-output.lint-file",
    ];
    let mut seen = Vec::new();
    for line in cases.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let id = row["id"].as_str().unwrap();
        if !wanted.contains(&id) {
            continue;
        }
        let argv = strings(&row["input"]["argv"]);
        let want = &row["expect"]["ok"];
        let got = corpus_cli(id, &argv, &[]);
        assert_eq!(got.stdout, want["stdout"].as_str().unwrap(), "{id} stdout");
        assert_eq!(got.stderr, want["stderr"].as_str().unwrap(), "{id} stderr");
        assert_eq!(
            i64::from(got.code),
            want["exit"].as_i64().unwrap(),
            "{id} exit"
        );
        seen.push(id.to_owned());
    }
    seen.sort();
    let mut wanted = wanted.to_vec();
    wanted.sort();
    assert_eq!(seen, wanted);
}

/// Thor's `help VERB`, `-h`/`--help`, unknown and ambiguous commands, unknown
/// switches (with `Did you mean?`) and wrong argument counts, captured from
/// the pinned oracle (`tests/thor_data/cases.json`).
#[cfg(target_os = "linux")]
#[test]
fn thor_help_and_parse_errors_match_the_oracle_captures() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/thor_data/cases.json");
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 258);
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let argv = strings(&case["argv"]);
        let got = corpus_cli(&format!("thor-{i}"), &argv, &[]);
        let want_out = case["stdout"].as_str().unwrap();
        let want_err = case["stderr"].as_str().unwrap();
        let want_code = case["exit"].as_i64().unwrap();
        if got.stdout != want_out || got.stderr != want_err || i64::from(got.code) != want_code {
            failures.push(format!(
                "{argv:?}\n  stdout {:?}\n  expected {want_out:?}\n  stderr {:?}\n  expected {want_err:?}\n  exit {} expected {want_code}",
                got.stdout, got.stderr, got.code
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// The interface, `--all`, `NO_COLOR` and error paths of `info` and `opts`
/// the corpus does not cover, captured from the same oracle.
#[cfg(target_os = "linux")]
#[test]
fn info_and_opts_match_the_oracle_captures() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/info_opts_data/cases.json");
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 27);
    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/info_opts_data");
    for (i, case) in cases.iter().enumerate() {
        let argv: Vec<String> = strings(&case["argv"])
            .iter()
            .map(|a| a.replace("${DATA}", data_dir.to_str().unwrap()))
            .collect();
        let extra: Vec<(String, String)> = case["env"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let got = corpus_cli(&format!("info-opts-{i}"), &argv, &extra);
        assert_eq!(
            got.stdout,
            case["stdout"].as_str().unwrap(),
            "{argv:?} stdout"
        );
        assert_eq!(
            got.stderr,
            case["stderr"].as_str().unwrap(),
            "{argv:?} stderr"
        );
        assert_eq!(
            i64::from(got.code),
            case["exit"].as_i64().unwrap(),
            "{argv:?} exit"
        );
    }
}

/// Replays `version_detection_cases.json` (a register of tools whose
/// definition has no version) in the corpus environment, but with `PATH` set
/// to `HOME/bin`, a directory holding a bash link and the case's scripts, so
/// the installed tools run a script the test controls. `only` selects the
/// verbs replayed.
#[cfg(target_os = "linux")]
fn replay_version_detection(only: &str) {
    use std::os::unix::fs::PermissionsExt;
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/info_opts_data");
    let data: Value = serde_json::from_str(
        &std::fs::read_to_string(tests.join("version_detection_cases.json")).unwrap(),
    )
    .unwrap();
    let register = tests.join("vreg");
    let mut replayed = 0;
    for (i, case) in data["cases"].as_array().unwrap().iter().enumerate() {
        let argv = strings(&case["argv"]);
        if argv[0] != only {
            continue;
        }
        let root = scratch(&format!("version-detection-{only}-{i}"));
        let home = root.join("home");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink("/bin/bash", bin.join("bash")).unwrap();
        for (name, body) in data["scripts"].as_object().unwrap() {
            let path = bin.join(name);
            std::fs::write(&path, body.as_str().unwrap()).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let h = home.to_str().unwrap();
        let env: Vec<(String, String)> = [
            ("HOME", h),
            ("TZ", "UTC"),
            ("LANG", "C.UTF-8"),
            ("LC_ALL", "C.UTF-8"),
            ("SHELL", "/bin/bash"),
            ("UKIRYU_REGISTER", register.to_str().unwrap()),
            ("COLUMNS", "100"),
            ("THOR_COLUMNS", "100"),
            ("TERM", "dumb"),
            ("GIT_REDIRECT_STDERR", "/dev/null"),
            ("PATH", bin.to_str().unwrap()),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        let got = run(&argv, &env, &home, Some(""));
        assert_eq!(
            got.stdout.replace(h, "${HOME}"),
            case["stdout"].as_str().unwrap(),
            "{argv:?} stdout"
        );
        assert_eq!(
            got.stderr.replace(h, "${HOME}"),
            case["stderr"].as_str().unwrap(),
            "{argv:?} stderr"
        );
        assert_eq!(
            i64::from(got.code),
            case["exit"].as_i64().unwrap(),
            "{argv:?} exit"
        );
        let _ = std::fs::remove_dir_all(&root);
        replayed += 1;
    }
    assert!(replayed > 0, "no {only} cases replayed");
}

/// `info` of installed tools whose definition has no version: the detected
/// version, and `unknown` when detection finds none.
#[cfg(target_os = "linux")]
#[test]
fn info_detects_the_version_of_an_installed_tool() {
    replay_version_detection("info");
}

/// `list` shows the detected version (`vX`) or `version unknown`.
#[cfg(target_os = "linux")]
#[test]
fn list_detects_the_version_of_installed_tools() {
    replay_version_detection("list");
}

// ------------------------------------------------- inspect remainder (#73)

/// The register-backed inspect verbs this build integrates.
#[cfg(target_os = "linux")]
const REMAINDER_VERBS: [&str; 5] = ["config", "list", "commands", "register", "resolve"];

/// `inspect_remainder_goldens.json` (config list, list, commands, register,
/// resolve), captured from the pinned exe/ukiryu (ukiryu/ukiryu ab6df35)
/// with the per-case environment in its `source.context`, replayed through
/// the binary. The handler test `inspect_remainder.rs` replays the same rows
/// against the handlers; this proves the binary dispatches to them.
#[cfg(target_os = "linux")]
#[test]
fn inspect_remainder_goldens_through_the_binary() {
    let goldens: Value =
        serde_json::from_str(include_str!("inspect_remainder_goldens.json")).unwrap();
    let mtime = UNIX_EPOCH + Duration::from_secs(goldens["fixed_mtime_unix"].as_u64().unwrap());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/inspect_data/register");
    let mut failures = Vec::new();
    let mut replayed = 0;
    for case in goldens["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let root = scratch(&format!("remainder-{name}"));
        let dir = |leaf: &str| {
            let path = root.join(leaf);
            std::fs::create_dir_all(&path).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let (home, bin, xdg, sysdata) = (dir("home"), dir("bin"), dir("xdg"), dir("sysdata"));
        let mode = case["register"].as_str().unwrap();
        let register = if mode == "user" {
            format!("{home}/.ukiryu/register")
        } else {
            root.join("register").to_str().unwrap().to_owned()
        };
        for b in ["bash", "sh", "which"]
            .into_iter()
            .map(str::to_owned)
            .chain(strings(&case["bins"]))
        {
            if b == "git" {
                // The capture links the real git (capture.rb), so `register
                // update` sees a directory that is not a git work tree.
                std::os::unix::fs::symlink("/usr/bin/git", Path::new(&bin).join("git")).unwrap();
            } else {
                install_executable(Path::new(&bin), &b);
            }
        }
        for (rel, body) in case["home_before"].as_object().unwrap() {
            let path = Path::new(&home).join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body.as_str().unwrap()).unwrap();
        }
        for (rel, body) in case["defs"].as_object().unwrap() {
            let (base, rest) = rel.split_once('/').unwrap();
            let base = if base == "user" { &xdg } else { &sysdata };
            let path = Path::new(base).join("ukiryu/definitions").join(rest);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body.as_str().unwrap()).unwrap();
            set_mtime(&path, mtime);
        }
        match mode {
            "env" | "user" => copy_tree(&fixture, Path::new(&register), SystemTime::now()),
            "empty" => {
                let tools = Path::new(&register).join("tools/x");
                std::fs::create_dir_all(&tools).unwrap();
                std::fs::write(tools.join("a.yaml"), "---\nname: x\n").unwrap();
            }
            other => panic!("{name}: register mode {other}"),
        }
        let mut env = vec![
            ("HOME".to_owned(), home.clone()),
            ("PATH".to_owned(), bin.clone()),
            ("TZ".to_owned(), "UTC".to_owned()),
            ("XDG_DATA_HOME".to_owned(), xdg.clone()),
            ("XDG_DATA_DIRS".to_owned(), sysdata.clone()),
        ];
        if mode != "user" {
            env.push(("UKIRYU_REGISTER".to_owned(), register.clone()));
        }
        for (k, v) in case["env"].as_object().unwrap() {
            env.retain(|(name, _)| name != k);
            env.push((
                k.clone(),
                v.as_str().unwrap().replace("{REGISTER}", &register),
            ));
        }
        let args: Vec<String> = strings(&case["args"])
            .into_iter()
            .map(|a| a.replace("{REGISTER}", &register))
            .collect();
        assert!(
            REMAINDER_VERBS.contains(&args[0].as_str()),
            "{name}: verb {}",
            args[0]
        );
        let subst = |golden: &Value| {
            golden
                .as_str()
                .unwrap()
                .replace("{HOME}", &home)
                .replace("{BIN}", &bin)
                .replace("{XDG}", &xdg)
                .replace("{SYSDATA}", &sysdata)
                .replace("{REGISTER}", &register)
        };
        let (want_out, want_err) = (subst(&case["stdout"]), subst(&case["stderr"]));
        let want_code = case["exit"].as_i64().unwrap() as i32;
        let got = run(&args, &env, Path::new(&home), None);
        replayed += 1;
        let mut fail = |what: String| failures.push(format!("{name}: {what}"));
        if !case["note"].is_null() {
            // D2: the strict shell error after the output printed before the
            // first tool lookup.
            let first = want_out.split_inclusive('\n').next().unwrap();
            if got.stdout != first
                || !got
                    .stderr
                    .starts_with("[Ukiryu::Errors::UnknownShellError] ")
                || got.code != 1
            {
                fail(format!(
                    "D2 row: exit {} out {:?} err {:?}",
                    got.code, got.stdout, got.stderr
                ));
            }
        } else {
            const WARNING: &str = "\x1b[31mWarning: Failed to load config file: ";
            let got_lines: Vec<&str> = got.stdout.split('\n').collect();
            let want_lines: Vec<&str> = want_out.split('\n').collect();
            let same_out = got_lines.len() == want_lines.len()
                && got_lines.iter().zip(&want_lines).all(|(g, w)| {
                    if w.starts_with(WARNING) {
                        g.starts_with(WARNING) && g.ends_with("\x1b[0m")
                    } else {
                        g == w
                    }
                });
            if !same_out {
                fail(format!("stdout {:?}\nexpected {want_out:?}", got.stdout));
            }
            if got.stderr != want_err {
                fail(format!("stderr {:?}\nexpected {want_err:?}", got.stderr));
            }
            if got.code != want_code {
                fail(format!("exit {} expected {want_code}", got.code));
            }
        }
        let want_files = case["home_after"].as_object().unwrap();
        let got_files: Vec<String> = files_under(Path::new(&home))
            .into_iter()
            .filter(|rel| !rel.starts_with(".ukiryu/register"))
            .collect();
        let keys: Vec<String> = want_files.keys().cloned().collect();
        if got_files != keys {
            fail(format!("files {got_files:?} expected {keys:?}"));
        } else {
            for (rel, body) in want_files {
                let text = std::fs::read_to_string(Path::new(&home).join(rel)).unwrap();
                if text != body.as_str().unwrap() {
                    fail(format!("{rel} = {text:?}"));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
    assert_eq!(
        replayed,
        goldens["cases"].as_array().unwrap().len(),
        "every remainder row is replayed"
    );
    assert!(
        failures.is_empty(),
        "{} of {replayed} differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// The corpus `cli-output` rows of the verbs above (captured from the Ruby
/// CLI on Linux by corpus/oracle-ruby `op_cli_run`), replayed byte for byte.
/// Every such row must be listed here, so a new row cannot be skipped.
#[cfg(target_os = "linux")]
const CORPUS_REMAINDER_ROWS: [&str; 4] = [
    "cli-output.commands-jq",
    "cli-output.commands-unknown",
    "cli-output.resolve-jq",
    "cli-output.resolve-jq-1-0",
];

#[cfg(target_os = "linux")]
#[test]
fn corpus_cli_output_rows_of_the_remainder_verbs() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance");
    let inputs = corpus.join("inputs");
    let inputs = inputs.to_str().unwrap();
    let cases = std::fs::read_to_string(corpus.join("cases/cli-output.jsonl")).unwrap();
    let mut seen = Vec::new();
    let mut failures = Vec::new();
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line).unwrap();
        let id = case["id"].as_str().unwrap();
        let argv = strings(&case["input"]["argv"]);
        if !REMAINDER_VERBS.contains(&argv[0].as_str()) {
            continue;
        }
        assert!(CORPUS_REMAINDER_ROWS.contains(&id), "unlisted row {id}");
        seen.push(id.to_owned());
        let home = scratch(&format!("corpus-{id}"));
        let h = home.to_str().unwrap();
        // `op_cli_run`'s environment (corpus/oracle-ruby/lib/ops.rb). The
        // oracle inherits the host PATH; a plain system PATH stands in.
        let mut env: Vec<(String, String)> = [
            ("HOME", h),
            ("TZ", "UTC"),
            ("LANG", "C.UTF-8"),
            ("LC_ALL", "C.UTF-8"),
            ("SHELL", "/bin/bash"),
            ("COLUMNS", "100"),
            ("THOR_COLUMNS", "100"),
            ("TERM", "dumb"),
            ("GIT_REDIRECT_STDERR", "/dev/null"),
            ("PATH", "/usr/bin:/bin"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        env.push(("UKIRYU_REGISTER".to_owned(), format!("{inputs}/register")));
        if let Some(extra) = case["input"]["env"].as_object() {
            for (k, v) in extra {
                env.retain(|(name, _)| name != k);
                env.push((k.clone(), v.as_str().unwrap().to_owned()));
            }
        }
        let args: Vec<String> = argv
            .iter()
            .map(|a| a.replace("${INPUTS}", inputs))
            .collect();
        let stdin = case["input"]["stdin"].as_str();
        let got = run(&args, &env, &home, stdin);
        let expect = &case["expect"]["ok"];
        let subst = |v: &Value| {
            v.as_str()
                .unwrap()
                .replace("${INPUTS}", inputs)
                .replace("${HOME}", h)
        };
        let (want_out, want_err) = (subst(&expect["stdout"]), subst(&expect["stderr"]));
        let want_code = expect["exit"].as_i64().unwrap() as i32;
        if got.stdout != want_out || got.stderr != want_err || got.code != want_code {
            failures.push(format!(
                "{id}: exit {} out {:?} err {:?}\nexpected exit {want_code} out {want_out:?} err {want_err:?}",
                got.code, got.stdout, got.stderr
            ));
        }
        let _ = std::fs::remove_dir_all(&home);
    }
    assert_eq!(seen, CORPUS_REMAINDER_ROWS, "corpus rows replayed");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

// ------------------------------------------------------------ exec stdin step

/// `stdin_data/cases.json`: `ukiryu exec jq process <params> -D filter=.`
/// captured from the pinned Ruby CLI (ab6df35, Ruby 4.0.1, Linux) against
/// `stdin_data/register`, with `NO_COLOR=1`. The binary replays each row with
/// the row's stdin pipe and files and must match status, stdout and stderr.
///
/// Every row is counted in a bucket:
/// - `invalid`: the stdin bytes are not valid UTF-8; the row must match;
/// - `closed`: the row closes file descriptor 0 (through `sh`); the row must
///   match;
/// - `unreadable`: a mode-000 file that the test user can still read (root),
///   skipped.
#[cfg(target_os = "linux")]
#[derive(Default, Debug, PartialEq)]
struct StdinCounts {
    exact: usize,
    invalid: usize,
    closed: usize,
    unreadable: usize,
}

#[cfg(target_os = "linux")]
fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[cfg(target_os = "linux")]
fn run_stdin_row(
    args: &[String],
    env: &[(String, String)],
    cwd: &Path,
    stdin: Stdio,
    bytes: &[u8],
) -> (Vec<u8>, Vec<u8>, i32) {
    let mut child = Command::new(BIN)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the ukiryu binary");
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(bytes);
    }
    let out = child.wait_with_output().unwrap();
    (
        out.stdout,
        out.stderr,
        out.status.code().expect("exit status"),
    )
}

/// Runs the binary with file descriptor 0 closed (not `/dev/null`): the
/// shell closes it before `exec`, so the test needs no unsafe code.
#[cfg(target_os = "linux")]
fn closed_stdin_row(
    args: &[String],
    env: &[(String, String)],
    cwd: &Path,
) -> (Vec<u8>, Vec<u8>, i32) {
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"exec "$0" "$@" <&-"#)
        .arg(BIN)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the ukiryu binary through sh");
    (
        out.stdout,
        out.stderr,
        out.status.code().expect("exit status"),
    )
}

#[cfg(target_os = "linux")]
fn replay_stdin() -> (StdinCounts, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;
    let data: Value = serde_json::from_str(include_str!("stdin_data/cases.json")).unwrap();
    let register = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/stdin_data/register");
    let mut counts = StdinCounts::default();
    let mut failures = Vec::new();
    for case in data["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let root = scratch(&format!("stdin-{id}"));
        let dir = root.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        let setup = &case["setup"];
        for (name, hex) in setup["files"].as_object().unwrap() {
            std::fs::write(dir.join(name), unhex(hex.as_str().unwrap())).unwrap();
        }
        for name in setup["dirs"].as_array().unwrap() {
            std::fs::create_dir(dir.join(name.as_str().unwrap())).unwrap();
        }
        for (name, target) in setup["symlinks"].as_object().unwrap() {
            std::os::unix::fs::symlink(dir.join(target.as_str().unwrap()), dir.join(name)).unwrap();
        }
        let mut unreadable = false;
        for name in setup["chmod0"].as_array().unwrap() {
            let path = dir.join(name.as_str().unwrap());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            unreadable |= std::fs::read(&path).is_ok();
        }
        let cleanup = |dir: &Path| {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let _ = std::fs::set_permissions(
                        entry.path(),
                        std::fs::Permissions::from_mode(0o700),
                    );
                }
            }
            let _ = std::fs::remove_dir_all(&root);
        };
        let pipe = &case["pipe"];
        let mode = pipe["mode"].as_str().unwrap();
        if unreadable {
            counts.unreadable += 1;
            cleanup(&dir);
            continue;
        }
        let d = dir.to_str().unwrap();
        let mut args: Vec<String> = ["exec", "jq", "process"].map(String::from).to_vec();
        args.extend(
            strings(&case["params"])
                .iter()
                .map(|p| p.replace("{dir}", d)),
        );
        args.extend(["-D".to_owned(), "filter=.".to_owned()]);
        let env = vec![
            ("HOME".to_owned(), format!("{}/home", root.display())),
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("SHELL".to_owned(), "/bin/bash".to_owned()),
            (
                "UKIRYU_REGISTER".to_owned(),
                register.to_str().unwrap().to_owned(),
            ),
            ("TERM".to_owned(), "dumb".to_owned()),
            ("TZ".to_owned(), "UTC".to_owned()),
            ("LANG".to_owned(), "C.UTF-8".to_owned()),
            ("NO_COLOR".to_owned(), "1".to_owned()),
        ];
        let cwd = if setup["chdir"] == true {
            dir.clone()
        } else {
            root.clone()
        };
        let (stdin, bytes) = match mode {
            "closed" => (Stdio::null(), Vec::new()),
            "bytes" => (Stdio::piped(), unhex(pipe["hex"].as_str().unwrap())),
            "devnull" => (
                Stdio::from(std::fs::File::open("/dev/null").unwrap()),
                Vec::new(),
            ),
            "directory" => (
                Stdio::from(std::fs::File::open(dir.join(pipe["name"].as_str().unwrap())).unwrap()),
                Vec::new(),
            ),
            other => panic!("unknown pipe mode {other}"),
        };
        let (stdout, stderr, code) = if mode == "closed" {
            closed_stdin_row(&args, &env, &cwd)
        } else {
            run_stdin_row(&args, &env, &cwd, stdin, &bytes)
        };
        cleanup(&dir);

        let want_stdout = unhex(case["stdout_hex"].as_str().unwrap());
        let want_stderr = String::from_utf8(unhex(case["stderr_hex"].as_str().unwrap()))
            .unwrap()
            .replace("{dir}", d);
        if i64::from(code) == case["exit_status"].as_i64().unwrap()
            && stdout == want_stdout
            && String::from_utf8_lossy(&stderr) == want_stderr
        {
            if mode == "closed" {
                counts.closed += 1;
            } else if case["resolved"]["stdin"]["valid_encoding"] == false {
                counts.invalid += 1;
            } else {
                counts.exact += 1;
            }
        } else {
            failures.push(format!(
                "{id}: status {code} (want {}), stdout {:?} (want {:?}), stderr {:?} (want {want_stderr:?})",
                case["exit_status"],
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&want_stdout),
                String::from_utf8_lossy(&stderr)
            ));
        }
    }
    (counts, failures)
}

/// `execute_data/stdin_bytes.json`: Ruby captures of `exec` with arbitrary
/// stdin bytes, a closed fd 0 and a child that stops reading early, through
/// the `cat` fixture tool (stdout, stderr, status and written files).
#[cfg(target_os = "linux")]
fn replay_stdin_bytes() -> (usize, Vec<String>) {
    let data: Value = serde_json::from_str(include_str!("execute_data/stdin_bytes.json")).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/execute_data/fixture");
    let mut exact = 0;
    let mut failures = Vec::new();
    for case in data["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let root = scratch(&format!("stdinbytes-{id}"));
        copy_tree(&fixture, &root, SystemTime::now());
        for dir in ["home", "xdg", "sysdata"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (name, hex) in case["files_in"].as_object().unwrap() {
            std::fs::write(root.join(name), unhex(hex.as_str().unwrap())).unwrap();
        }
        let r = root.to_str().unwrap().to_owned();
        let env = execute_env(&r, &case["env"]);
        let args = strings(&case["args"]);
        let pipe = &case["stdin"];
        let (stdout, stderr, code) = match pipe["mode"].as_str().unwrap() {
            "closed" => closed_stdin_row(&args, &env, &root),
            "bytes" => {
                let bytes = match pipe["hex"].as_str() {
                    Some(hex) => unhex(hex),
                    None => unhex(pipe["repeat_hex"].as_str().unwrap())
                        .repeat(pipe["count"].as_u64().unwrap() as usize),
                };
                run_stdin_row(&args, &env, &root, Stdio::piped(), &bytes)
            }
            other => panic!("unknown stdin mode {other}"),
        };
        let written: Vec<(String, Option<String>)> = case["files_out"]
            .as_object()
            .unwrap()
            .keys()
            .map(|name| (name.clone(), std::fs::read_to_string(root.join(name)).ok()))
            .collect();
        let _ = std::fs::remove_dir_all(&root);

        // Masks the timing fields; bytes that are not UTF-8 compare as bytes.
        let mask = |bytes: &[u8]| -> Vec<u8> {
            match std::str::from_utf8(bytes) {
                Ok(text) => mask_timing(text).into_bytes(),
                Err(_) => bytes.to_vec(),
            }
        };
        let want_stdout = mask(&unhex(case["stdout_hex"].as_str().unwrap()));
        let want_stderr = unhex(case["stderr_hex"].as_str().unwrap());
        let got_stdout = mask(&stdout);
        let stdout_ok = match (
            std::str::from_utf8(&want_stdout),
            std::str::from_utf8(&got_stdout),
        ) {
            (Ok(want), Ok(got)) => same_stdout(want, got),
            _ => want_stdout == got_stdout,
        };
        let files_ok = case["files_out"]
            .as_object()
            .unwrap()
            .iter()
            .zip(&written)
            .all(|((name, hex), (got_name, got))| {
                let want = String::from_utf8(unhex(hex.as_str().unwrap())).ok();
                name == got_name && same_file(&want, got)
            });
        if i64::from(code) == case["status"].as_i64().unwrap()
            && stdout_ok
            && stderr == want_stderr
            && files_ok
        {
            exact += 1;
        } else {
            failures.push(format!(
                "{id}: status {code} (want {}), stdout {:?} (want {:?}), stderr {:?} (want {:?}), files match {files_ok}",
                case["status"],
                String::from_utf8_lossy(&got_stdout),
                String::from_utf8_lossy(&want_stdout),
                String::from_utf8_lossy(&stderr),
                String::from_utf8_lossy(&want_stderr),
            ));
        }
    }
    (exact, failures)
}

/// The compiled binary replays the Ruby captures of arbitrary stdin bytes
/// (not UTF-8, NUL, CRLF), a closed fd 0 and a child that exits before it
/// reads its stdin: every one of the 39 rows must match.
#[cfg(target_os = "linux")]
#[test]
fn exec_stdin_bytes_closed_fd_and_early_exit_through_the_binary() {
    let (exact, failures) = replay_stdin_bytes();
    assert!(
        failures.is_empty(),
        "{} rows failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert_eq!(exact, 39);
}

/// The compiled binary replays the shared stdin step's Ruby captures through
/// `exec`: 39 rows, every one counted in a named bucket.
#[cfg(target_os = "linux")]
#[test]
fn exec_stdin_cases_through_the_binary() {
    let (counts, failures) = replay_stdin();
    assert!(
        failures.is_empty(),
        "{} stdin rows failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(
        counts.exact + counts.invalid + counts.closed + counts.unreadable,
        39
    );
    assert!(counts.exact >= 30, "{counts:?}");
}
