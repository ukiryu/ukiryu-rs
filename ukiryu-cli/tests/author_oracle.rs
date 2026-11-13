//! Golden tests for the authoring handlers (`ukiryu_cli::commands::author`).
//!
//! `author_data/cases.json` is captured from the pinned Ruby oracle
//! (ukiryu/ukiryu ab6df35) by the K22-CLI-AUTHOR capture scripts, against a
//! copy of `author_data/tree` with every file mtime set to
//! 2026-01-02 03:04:05 UTC and an isolated environment. Each case is replayed
//! here in a fresh copy of the same tree, with the process working directory
//! set to it (one test function, so the cases run sequentially).
//!
//! The captures use POSIX paths and Linux errno texts, so the replay is Unix
//! only.
#![cfg(unix)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ukiryu::definition::DiscoveryEnv;
use ukiryu::environment::Environment;
use ukiryu::extractors::base::ProcessHost;
use ukiryu::render::OutputWrite;
use ukiryu::runtime::ShellName;
use ukiryu_cli::commands::author::{
    self, AuthorContext, AuthorError, Confirm, DefinitionsInput, DocsInput, DocsOptions,
    DocsTarget, ExtractInput, LintInput, LintOptions, LintTarget, ValidateInput, ValidateOptions,
    ValidateTarget,
};
use ukiryu_cli::commands::CommandOutcome;

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

fn files_under(dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        if dir.is_file() {
            out.push(dir.to_string_lossy().into_owned());
        }
        return;
    };
    for entry in rd {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            files_under(&entry.path(), out);
        } else {
            out.push(entry.path().to_string_lossy().into_owned());
        }
    }
}

/// Options after the positionals: `--name value` pairs and boolean flags.
struct Args {
    positional: Vec<String>,
    options: Vec<(String, Option<String>)>,
}

const FLAGS: [&str; 7] = [
    "--strict",
    "--executable",
    "--all",
    "--verbose",
    "-v",
    "-f",
    "--force",
];

fn split(args: &[String], value_flags_v: bool) -> Args {
    let mut positional = Vec::new();
    let mut options = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let is_flag = FLAGS.contains(&a.as_str()) && !(a == "-v" && value_flags_v);
        if is_flag {
            options.push((a.clone(), None));
            i += 1;
        } else if a.starts_with('-') && a.len() > 1 {
            options.push((a.clone(), args.get(i + 1).cloned()));
            i += 2;
        } else {
            positional.push(a.clone());
            i += 1;
        }
    }
    Args {
        positional,
        options,
    }
}

impl Args {
    fn get(&self, names: &[&str]) -> Option<String> {
        self.options
            .iter()
            .rev()
            .find(|(n, _)| names.contains(&n.as_str()))
            .and_then(|(_, v)| v.clone())
    }

    fn has(&self, names: &[&str]) -> bool {
        self.options
            .iter()
            .any(|(n, _)| names.contains(&n.as_str()))
    }
}

struct Answer {
    line: Option<String>,
    shown: Vec<OutputWrite>,
}

impl Confirm for Answer {
    fn ask(&mut self, shown: Vec<OutputWrite>) -> Option<String> {
        self.shown = shown;
        self.line.take()
    }
}

enum Run {
    Done(CommandOutcome),
    Err(AuthorError),
}

fn run_case(case: &Value, ctx: &AuthorContext, answer: &mut Answer) -> Run {
    let args: Vec<String> = case["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    let runner = case["runner"].as_str().unwrap();
    let (verb, rest) = match runner {
        "exe" => (args[0].as_str(), &args[1..]),
        other => (other, &args[..]),
    };
    let result = match verb {
        "validate" => {
            let a = split(rest, false);
            let target = match a.positional[0].as_str() {
                "file" => ValidateTarget::File {
                    path: a.positional[1].clone(),
                },
                "string" => ValidateTarget::String {
                    yaml: a.positional[1].clone(),
                },
                _ => ValidateTarget::All,
            };
            author::validate(
                ValidateInput {
                    target,
                    options: ValidateOptions {
                        verbose: a.has(&["--verbose"]),
                        format: a.get(&["--format"]),
                        schema: a.get(&["--schema"]),
                        strict: a.has(&["--strict"]),
                        executable: a.has(&["--executable"]),
                        all: a.has(&["--all"]),
                        register: a.get(&["--register"]),
                    },
                },
                ctx,
            )
        }
        "lint" => {
            let a = split(rest, false);
            let target = match a.positional[0].as_str() {
                "file" => LintTarget::File {
                    path: a.positional[1].clone(),
                },
                "all" => LintTarget::All {
                    register: a.get(&["--register"]),
                },
                _ => LintTarget::Rules,
            };
            author::lint(
                LintInput {
                    target,
                    options: LintOptions {
                        verbose: a.has(&["--verbose"]),
                        format: a.get(&["--format"]),
                        severity: a.get(&["--severity"]),
                    },
                },
                ctx,
            )
        }
        "docs" => {
            let a = split(rest, false);
            let target = match a.positional[0].as_str() {
                "generate" => DocsTarget::Generate {
                    tool: a.positional[1].clone(),
                    register: a.get(&["--register"]),
                },
                "generate-all" => DocsTarget::GenerateAll {
                    register: a.get(&["--register"]),
                    output_dir: a.get(&["--output-dir"]),
                },
                _ => DocsTarget::Serve {
                    port: a.get(&["--port"]),
                },
            };
            author::docs(
                DocsInput {
                    target,
                    options: DocsOptions {
                        verbose: a.has(&["--verbose"]),
                        format: a.get(&["--format"]),
                        output: a.get(&["--output"]),
                    },
                },
                ctx,
            )
        }
        "definitions" => {
            let sub = rest.first().map(String::as_str).unwrap_or("list");
            let tail = if rest.is_empty() { rest } else { &rest[1..] };
            // `-v` is --verbose for list, --version for info/remove.
            let a = split(tail, sub != "list");
            let input = match sub {
                "list" => DefinitionsInput::List {
                    verbose: a.has(&["-v", "--verbose"]),
                },
                "path" => DefinitionsInput::Path,
                "add" => DefinitionsInput::Add {
                    source: a.positional[0].clone(),
                    name: a.get(&["-n", "--name"]),
                    version: a.get(&["-V", "--version"]),
                },
                "remove" => DefinitionsInput::Remove {
                    tool: a.positional[0].clone(),
                    version: a.get(&["-v", "--version"]),
                    force: a.has(&["-f", "--force"]),
                },
                _ => DefinitionsInput::Info {
                    tool: a.positional[0].clone(),
                    version: a.get(&["-v", "--version"]),
                },
            };
            author::definitions(input, ctx, answer)
        }
        "extract" => {
            let a = split(rest, false);
            let root = std::env::current_dir().unwrap();
            let bin = root.join("xbin");
            let environment = Environment::from_process()
                .expect("UTF-8 process environment")
                .prepend_path(&[bin.to_str().unwrap()]);
            let mut host = ProcessHost::new(environment, ShellName::Bash);
            author::extract(
                ExtractInput {
                    tool: a.positional[0].clone(),
                    output: a.get(&["-o", "--output"]),
                    method: a.get(&["-m", "--method"]),
                    verbose: a.has(&["-v", "--verbose"]),
                },
                ctx,
                &mut host,
            )
        }
        other => panic!("unexpected verb {other}"),
    };
    match result {
        Ok(o) => Run::Done(o),
        Err(e) => Run::Err(e),
    }
}

/// Perform the writes the way the binary would; returns (stdout, stderr).
fn perform(writes: &[OutputWrite]) -> (String, String) {
    let (mut stdout, mut stderr) = (String::new(), String::new());
    for w in writes {
        match w {
            OutputWrite::Stdout(s) => stdout.push_str(s),
            OutputWrite::Stderr(s) => stderr.push_str(s),
            OutputWrite::File { path, contents } => std::fs::write(path, contents).unwrap(),
        }
    }
    (stdout, stderr)
}

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

fn context(case: &Value, root: &str) -> AuthorContext {
    let env = |key: &str| {
        case["env"]
            .get(key)
            .and_then(Value::as_str)
            .map(|v| v.replace("{root}", root))
    };
    AuthorContext {
        thor_colors: case["pty"].as_bool().unwrap_or(false),
        discovery: DiscoveryEnv {
            home: Some(format!("{root}/home")),
            xdg_data_home: Some(env("XDG_DATA_HOME").unwrap_or(format!("{root}/xdg"))),
            xdg_data_dirs: Some(format!("{root}/sysdata")),
            path: Some(env("PATH").unwrap_or(format!("{root}/bin"))),
            opt_root: format!("{root}/opt"),
        },
        schema_path_env: env("UKIRYU_SCHEMA_PATH"),
        default_register: Some(env("UKIRYU_REGISTER").unwrap_or(format!("{root}/register"))),
        config_colors_disabled: false,
    }
}

/// The YAML part (before the first ESC) as data, the coloured trailer as bytes.
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

/// Compare up to `marker` (the YAML engine message after it differs).
fn same_prefix(actual: &str, expected: &str, marker: &str) -> bool {
    match expected.find(marker) {
        Some(i) => {
            actual.len() >= i + marker.len()
                && actual[..i + marker.len()] == expected[..i + marker.len()]
        }
        None => actual == expected,
    }
}

#[test]
fn authoring_handlers_match_the_oracle_captures() {
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir().join("cases.json")).unwrap())
            .unwrap();
    let mtime = UNIX_EPOCH + Duration::from_secs(fixture["fixed_mtime_unix"].as_u64().unwrap());
    let base = std::env::temp_dir().join(format!("ukiryu-author-{}", std::process::id()));
    let original_dir = std::env::current_dir().unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (n, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let id = case["id"].as_str().unwrap();
        let mode = case["mode"].as_str().unwrap();
        if mode == "frozen" {
            continue;
        }
        let root_path = base.join(n.to_string());
        copy_tree(&data_dir().join("tree"), &root_path, mtime);
        std::fs::create_dir_all(root_path.join("home")).unwrap();
        std::fs::create_dir_all(root_path.join("bin")).unwrap();
        let root_path = std::fs::canonicalize(&root_path).unwrap();
        let root = root_path.to_string_lossy().into_owned();
        setup(id, &root_path, mtime);
        std::env::set_current_dir(&root_path).unwrap();
        let ctx = context(case, &root);
        let mut answer = Answer {
            line: case["stdin"].as_str().map(str::to_owned),
            shown: Vec::new(),
        };
        let run = run_case(case, &ctx, &mut answer);
        let expect = &case["expect"];
        let exp = |k: &str| expect[k].as_str().unwrap_or("").replace("{root}", &root);
        checked += 1;
        let mut fail = |what: String| failures.push(format!("{id}: {what}"));
        match (mode, run) {
            ("gated", Run::Err(AuthorError::Gated { partial, .. })) => {
                let (stdout, _) = perform(&partial);
                if stdout != case["partial_stdout"].as_str().unwrap() {
                    fail(format!("gated partial stdout {stdout:?}"));
                }
            }
            ("gated", other) => fail(format!("expected Gated, got {}", describe(&other))),
            (_, Run::Err(e)) => fail(format!("handler error {e:?}")),
            (_, Run::Done(outcome)) => {
                let mut writes = answer.shown.clone();
                writes.extend(outcome.writes.iter().cloned());
                let (stdout, stderr) = perform(&writes);
                if mode == "tty" {
                    let tty = case["tty_output"].as_str().unwrap().replace("\r\n", "\n");
                    let tty = tty.replace("{root}", &root);
                    // An uncaught Ruby exception starts a line with its
                    // source path ({gems}/... or {oracle}/... once scrubbed).
                    let starts_raise =
                        |t: &str| t.starts_with("{gems}/") || t.starts_with("{oracle}/");
                    let raise_at = std::iter::once(0)
                        .chain(tty.match_indices('\n').map(|(i, _)| i + 1))
                        .find(|&i| starts_raise(&tty[i..]));
                    match raise_at {
                        Some(0) => {
                            if !stdout.is_empty() || !stderr.starts_with("\x1b[31m[") {
                                fail(format!("tty raise at start: {stdout:?} {stderr:?}"));
                            }
                        }
                        Some(i) => {
                            if stdout != tty[..i] || !stderr.starts_with("\x1b[31m[NameError] ") {
                                fail(format!("tty raise: {stdout:?} {stderr:?}"));
                            }
                        }
                        None => {
                            let both = format!("{stdout}{stderr}");
                            if both != tty {
                                fail(format!("tty output\n{both:?}\nexpected\n{tty:?}"));
                            }
                        }
                    }
                } else {
                    let (es, ee) = (exp("stdout"), exp("stderr"));
                    let ok_out = match mode {
                        "lines" => sorted_lines(&stdout) == sorted_lines(&es),
                        "prefix" => same_prefix(&stdout, &es, case["prefix"].as_str().unwrap()),
                        "yaml_data" => same_yaml_data(&stdout, &es),
                        _ => stdout == es,
                    };
                    let ok_err = match mode {
                        "prefix" => same_prefix(&stderr, &ee, case["prefix"].as_str().unwrap()),
                        _ => stderr == ee,
                    };
                    if !ok_out {
                        fail(format!("stdout\n{stdout:?}\nexpected\n{es:?}"));
                    }
                    if !ok_err {
                        fail(format!("stderr\n{stderr:?}\nexpected\n{ee:?}"));
                    }
                }
                let exit = expect["exit"].as_i64().unwrap() as i32;
                if outcome.exit_code != exit {
                    fail(format!("exit {} expected {exit}", outcome.exit_code));
                }
                if let Some(files) = expect["files"].as_array() {
                    let mut want: Vec<String> = files
                        .iter()
                        .map(|f| f.as_str().unwrap().replace("{root}", &root))
                        .collect();
                    want.sort();
                    let dir = case_files_dir(case, &root_path);
                    let mut got = Vec::new();
                    files_under(&dir, &mut got);
                    got.sort();
                    if got != want {
                        fail(format!("files {got:?} expected {want:?}"));
                    }
                }
                if let Some(contents) = expect["file_contents"].as_object() {
                    for (rel, text) in contents {
                        let got = std::fs::read_to_string(root_path.join(rel)).unwrap_or_default();
                        if got != text.as_str().unwrap() {
                            fail(format!("contents of {rel}\n{got:?}"));
                        }
                    }
                }
            }
        }
    }
    std::env::set_current_dir(original_dir).unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert!(checked > 100, "only {checked} cases checked");
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

fn case_files_dir(case: &Value, root: &Path) -> PathBuf {
    let args: Vec<&str> = case["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    if args.contains(&"--output-dir") {
        root.join("outdocs")
    } else if args.contains(&"--output") {
        root.join("out.md")
    } else if args.contains(&"-o") {
        root.join("out.yaml")
    } else {
        root.join("xdg")
    }
}

fn describe(run: &Run) -> String {
    match run {
        Run::Done(o) => format!("outcome exit {}", o.exit_code),
        Run::Err(e) => format!("{e:?}"),
    }
}
