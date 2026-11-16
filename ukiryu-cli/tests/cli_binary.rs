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
//!
//! Rows are split into three contracts and counted separately:
//! - source rows: captured from exe/ukiryu and expected byte for byte (or with
//!   the recorded comparison mode), apart from the recorded deviations below;
//! - target-trace rows (`__target`, `__fileutils` ids): the designed output
//!   where the Ruby command crashes on a defect of its own source
//!   (`ORACLE_DEFECTS`), and every `lint` row (lint is a target addition:
//!   cli.rb never registers it). These are UNQUALIFIED: they prove the binary
//!   performs the handler's designed output, not source parity, and the owner
//!   has not accepted them as #21/#22 success;
//! - gated rows: paths whose component is not merged (Register.default,
//!   `validate --executable`, the execute rows the handlers gate), which must
//!   fail with the gated status and never print a success;
//! - terminal rows (`pty`): captured with a terminal on stdout. The test
//!   spawns the binary over pipes, so they cannot run here; they are counted
//!   and asserted by number, and the handler test `execute.rs` replays them.
//!
//! Recorded deviations (all decided outside this file):
//! - the version line carries `ukiryu::VERSION` (D3), not the gem's 0.3.1;
//! - shell detection is strict (DECISIONS.md D2): inspect rows with a `note`
//!   expect the shell error after the same partial output, and extract rows
//!   set `SHELL=/bin/bash` where Ruby searched PATH for bash;
//! - argument errors print clap's wording; only the status (1) is compared;
//! - the persistent config.yml is compared as YAML data, and YAML engine
//!   messages by their recorded prefix.
#![cfg(unix)]

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ukiryu_cli::dispatch::GATED_EXIT;

const BIN: &str = env!("CARGO_BIN_EXE_ukiryu");
#[cfg(target_os = "linux")]
const RUBY_VERSION_LINE: &str = "Ukiryu version 0.3.1";

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
            pipe.write_all(text.as_bytes()).unwrap();
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
                .replace(
                    RUBY_VERSION_LINE,
                    &format!("Ukiryu version {}", ukiryu::VERSION),
                )
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Contract {
    Source,
    TargetTrace,
}

/// The binary's argv for a captured row: exe rows as captured; standalone
/// LintCommand rows behind the `lint` verb.
fn argv(case: &Value) -> Option<Vec<String>> {
    let args = strings(&case["args"]);
    match case["runner"].as_str().unwrap() {
        "exe" => Some(args),
        "lint" => Some(std::iter::once("lint".to_owned()).chain(args).collect()),
        _ => None, // docs stays unregistered (root ruling); library tests only.
    }
}

fn contract(case: &Value) -> Contract {
    let id = case["id"].as_str().unwrap();
    if id.contains("__") || case["runner"] == "lint" {
        Contract::TargetTrace
    } else {
        Contract::Source
    }
}

/// Rows whose path reads `Register.default`, which waits for the register
/// API: the binary must stop with the gated status after the same partial
/// stdout.
fn needs_default_register(args: &[String]) -> bool {
    let all = |verb: &str| {
        args.first().map(String::as_str) == Some(verb)
            && args.get(1).map(String::as_str) == Some("all")
    };
    all("validate") || (all("lint") && !args.iter().any(|a| a == "--register"))
}

fn author_env(case: &Value, root: &str) -> Vec<(String, String)> {
    let mut env = vec![
        ("HOME".to_owned(), format!("{root}/home")),
        ("XDG_DATA_HOME".to_owned(), format!("{root}/xdg")),
        ("XDG_DATA_DIRS".to_owned(), format!("{root}/sysdata")),
        ("PATH".to_owned(), format!("{root}/bin")),
        ("UKIRYU_REGISTER".to_owned(), format!("{root}/register")),
        ("TERM".to_owned(), "dumb".to_owned()),
        ("TZ".to_owned(), "UTC".to_owned()),
        ("LANG".to_owned(), "C.UTF-8".to_owned()),
    ];
    if case["args"][0] == "extract" {
        // D2: Ruby found bash on PATH; the strict contract names the shell.
        env.push(("SHELL".to_owned(), "/bin/bash".to_owned()));
    }
    for (k, v) in case["env"].as_object().unwrap() {
        if k == "RUBYOPT" {
            continue; // capture-only: loads fileutils for the target trace
        }
        let v = v.as_str().unwrap().replace("{root}", root);
        env.retain(|(name, _)| name != k);
        env.push((k.clone(), v));
    }
    env
}

/// Replays the author captures of one contract; returns the replayed count.
fn replay_author(which: Contract) -> usize {
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join("cases.json")).unwrap())
            .unwrap();
    let mtime = UNIX_EPOCH + Duration::from_secs(fixture["fixed_mtime_unix"].as_u64().unwrap());
    let mut failures = Vec::new();
    let mut replayed = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let mode = case["mode"].as_str().unwrap();
        if contract(case) != which || matches!(mode, "frozen" | "tty") {
            continue;
        }
        let Some(args) = argv(case) else { continue };
        let root_path = scratch(&format!("author-{id}"));
        copy_tree(&data_dir().join("tree"), &root_path, mtime);
        std::fs::create_dir_all(root_path.join("home")).unwrap();
        std::fs::create_dir_all(root_path.join("bin")).unwrap();
        setup(id, &root_path, mtime);
        let root = root_path.to_string_lossy().into_owned();
        let env = author_env(case, &root);
        let got = run(&args, &env, &root_path, case["stdin"].as_str());
        replayed += 1;
        let expect = &case["expect"];
        let exp = |k: &str| expect[k].as_str().unwrap_or("").replace("{root}", &root);
        let mut fail = |what: String| failures.push(format!("{id}: {what}"));

        if mode == "gated" || needs_default_register(&args) {
            let want = case["partial_stdout"].as_str().map(str::to_owned);
            let want = want.unwrap_or_default();
            if got.code != GATED_EXIT
                || got.stdout != want
                || !got.stderr.contains("is not available in this build yet")
            {
                fail(format!(
                    "gated: exit {} out {:?} err {:?}",
                    got.code, got.stdout, got.stderr
                ));
            }
            let _ = std::fs::remove_dir_all(&root_path);
            continue;
        }

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
        "{} of {replayed} {which:?} rows differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    replayed
}

#[test]
fn author_source_rows_through_the_binary() {
    let replayed = replay_author(Contract::Source);
    assert!(replayed >= 70, "only {replayed} source rows replayed");
}

/// UNQUALIFIED: designed outputs where the Ruby source crashes, and the lint
/// verb (a target addition). Not source parity; the owner decides later.
#[test]
fn author_target_trace_rows_through_the_binary_unqualified() {
    let replayed = replay_author(Contract::TargetTrace);
    assert!(replayed >= 40, "only {replayed} target-trace rows replayed");
}

// ------------------------------------------------------------------ execute

/// Execute rows the handlers gate (the handler test's `OPEN`): the binary
/// must exit with the gated status and report it, never succeed.
#[cfg(target_os = "linux")]
const EXECUTE_GATED: [&str; 5] = [
    "exec_register_no_command",
    "exec_help_register",
    "exec_register_unknown_param",
    "runfile_execute",
    "runfile_execute_json",
];

/// Recorded deviation: the YAML diagnostic is the Yeptris parser's, not
/// libyaml's. The row must run, not match.
#[cfg(target_os = "linux")]
const EXECUTE_UNQUALIFIED: [&str; 1] = ["runfile_invalid_yaml"];

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
    match (response_yaml(want), response_yaml(got)) {
        (Some(a), Some(b)) => a == b,
        _ => want == got,
    }
}

/// Equal written file: bytes, except JSON (timing masked).
#[cfg(target_os = "linux")]
fn same_file(want: &Option<String>, got: &Option<String>) -> bool {
    match (want, got) {
        (Some(a), Some(b)) => match (response_json(a), response_json(b)) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
        (a, b) => a == b,
    }
}

/// Counts of replayed execute rows by contract.
#[cfg(target_os = "linux")]
#[derive(Default, Debug, PartialEq)]
struct ExecuteCounts {
    exact: usize,
    gated: usize,
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

        if EXECUTE_GATED.contains(&id) {
            if got.code == GATED_EXIT && stderr.contains("not available") {
                counts.gated += 1;
            } else {
                failures.push(format!(
                    "{id}: expected the gated status {GATED_EXIT}, got {} with stderr {stderr:?}",
                    got.code
                ));
            }
            continue;
        }
        if EXECUTE_UNQUALIFIED.contains(&id) {
            if got.code == GATED_EXIT {
                failures.push(format!("{id}: gated, but it must run"));
            } else {
                counts.unqualified += 1;
            }
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
            && stderr == want_stderr
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
/// and cache. Gated rows must be gated (counted), terminal rows are counted
/// (they need a pty), every other row must match the Ruby capture.
#[cfg(target_os = "linux")]
#[test]
fn execute_cases_through_the_binary() {
    let (counts, failures) = replay_execute();
    eprintln!("execute rows through the binary: {counts:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert_eq!(counts.gated, EXECUTE_GATED.len(), "{counts:?}");
    assert_eq!(counts.unqualified, EXECUTE_UNQUALIFIED.len(), "{counts:?}");
    assert_eq!(counts.terminal, 12, "{counts:?}");
    assert_eq!(counts.exact, 79, "{counts:?}");
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
fn verbs_without_a_merged_handler_fail_gated() {
    for args in [
        &["list"][..],
        &["info", "ls"],
        &["commands", "ls"],
        &["opts", "ls"],
        &["describe", "ls"],
        &["which", "ls"],
        &["resolve", "ls"],
        &["register"],
        &["config"],
        &["config", "list"],
    ] {
        let got = simple(args);
        assert_eq!(got.code, GATED_EXIT, "{args:?}: {}", got.stderr);
        assert_eq!(got.stdout, "", "{args:?}");
        assert!(
            got.stderr.starts_with("ukiryu: '") && got.stderr.contains("not available"),
            "{args:?}: {:?}",
            got.stderr
        );
    }
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
    for args in [
        &["--help"][..],
        &["help"],
        &["validate", "--help"],
        &["lint", "--help"],
    ] {
        let got = simple(args);
        assert_eq!(got.code, 0, "{args:?}: {}", got.stderr);
        assert!(got.stdout.contains("Usage"), "{args:?}: {:?}", got.stdout);
    }
    // Thor prints the command list (stdout, status 0) when no command or no
    // subcommand of a group is given (captured: logs/grammar-3.log).
    let cases: [(&[&str], &str); 3] = [
        (&[], "validate"),
        (&["validate"], "file"),
        (&["lint"], "rules"),
    ];
    for (args, word) in cases {
        let got = simple(args);
        assert_eq!(got.code, 0, "{args:?}: {}", got.stderr);
        assert!(got.stdout.contains(word), "{args:?}: {:?}", got.stdout);
        assert_eq!(got.stderr, "", "{args:?}");
    }
}

#[test]
fn version_verb_prints_the_crate_version() {
    let got = simple(&["version"]);
    assert_eq!(got.code, 0, "{}", got.stderr);
    assert_eq!(
        got.stdout,
        format!("\x1b[36mUkiryu version {}\x1b[0m\n", ukiryu::VERSION)
    );
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
