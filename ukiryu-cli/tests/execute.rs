//! `exec`, `run-file` and `cache` against Ruby CLI captures.
//!
//! `execute_data/cases.json` holds stdout, stderr, exit status, the stdin
//! given and the files written for each case, captured from the pinned Ruby
//! CLI (ukiryu ab6df353, Ruby 4.0.1) by running `exe/ukiryu` in a fresh copy
//! of `execute_data/fixture` (definition files, request files, an input file
//! and a register holding `tools/cat` and `interfaces/cat.yaml` from
//! ukiryu/register 63c0997) with a controlled environment. Each case is
//! replayed here: the arguments go through the real parser, the handler runs
//! over an explicit context with the same variables (in a fresh copy of the
//! fixture, as the working directory), real processes run through the
//! executor, and the writes are compared.
//!
//! Timing fields (`started_at`, `finished_at`, `duration_seconds`,
//! `formatted_duration`, the table's `Duration:`) are masked on both sides.
//!
//! Cases listed in [`OPEN`] must end in `Gated`/`Unsupported`; cases in
//! [`UNQUALIFIED`] are recorded deviations from the source and only have to
//! run. Every other case must match exactly.
#![cfg(target_os = "linux")]

use serde_json::Value as Json;
use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use ukiryu::definition::{DefinitionCache, DefinitionMetadata};
use ukiryu::environment::Environment;
use ukiryu::executable::Platform;
use ukiryu::render::{OutputWrite, PaintSupport};
use ukiryu::runtime::Host;
use ukiryu_cli::commands::execute::{
    self, CacheAction, CacheInput, ExecInput, ExecuteContext, ExecuteError, RunFileInput,
};
use ukiryu_cli::commands::CommandOutcome;

/// Cases that need behaviour this port does not reproduce yet; each must
/// end in `ExecuteError::Gated` or `ExecuteError::Unsupported`.
const OPEN: &[(&str, &str)] = &[
    (
        "exec_register_no_command",
        "default command from Register.load_tool_metadata (ToolMetadata#default_command)",
    ),
    (
        "exec_help_register",
        "exec help resolves the tool with Tool.find_by",
    ),
    (
        "exec_register_unknown_param",
        "NoMethodError names an anonymous options class by address",
    ),
    (
        "runfile_execute",
        "NoMethodError for stdin= names an anonymous options class by address",
    ),
    (
        "runfile_execute_json",
        "NoMethodError for stdin= names an anonymous options class by address",
    ),
];

/// Recorded deviations from the source (they must run, not match).
const UNQUALIFIED: &[(&str, &str)] = &[(
    "runfile_invalid_yaml",
    "the YAML diagnostic is the Yeptris parser's, not libyaml's",
)];

fn cases() -> Json {
    let text = include_str!("execute_data/cases.json");
    serde_json::from_str(text).expect("cases.json parses")
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/execute_data/fixture")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// The environment `capture/cli.rb` gives the Ruby CLI (`base_env`), plus
/// the case's own variables (`null` removes one).
fn environment(root: &str, case_env: &Json) -> Environment {
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
        .fold(Environment::new(), |env, (k, v)| env.set(&k, v))
}

/// The parsed command line, mapped to a handler call.
enum Call {
    Exec(ExecInput),
    RunFile(RunFileInput),
    Cache(CacheInput),
}

fn string(m: &clap::ArgMatches, id: &str) -> Option<String> {
    m.get_one::<OsString>(id)
        .map(|s| s.to_str().expect("UTF-8 argument").to_owned())
}

fn flag(m: &clap::ArgMatches, id: &str) -> bool {
    m.get_one::<bool>(id).copied().unwrap_or(false)
}

/// The parser-field mapping CLI dispatch uses (see the handoff).
fn parse(args: &[String]) -> Call {
    let argv = std::iter::once("ukiryu".to_owned()).chain(args.iter().cloned());
    let matches = ukiryu_cli::arguments::command()
        .try_get_matches_from(argv)
        .expect("arguments parse");
    match matches.subcommand() {
        Some(("exec", m)) => Call::Exec(ExecInput {
            tool: string(m, "tool").unwrap(),
            arguments: m
                .get_many::<OsString>("arguments")
                .map(|v| v.map(|s| s.to_str().unwrap().to_owned()).collect())
                .unwrap_or_default(),
            register: string(m, "register"),
            definition: string(m, "definition"),
            format: string(m, "format"),
            output: string(m, "output"),
            dry_run: flag(m, "dry-run"),
            shell: string(m, "shell"),
            stdin: flag(m, "stdin"),
            raw: flag(m, "raw"),
        }),
        Some(("run-file", m)) => Call::RunFile(RunFileInput {
            request_file: string(m, "request-file").unwrap(),
            register: string(m, "register"),
            output: string(m, "output"),
            format: string(m, "format"),
            dry_run: flag(m, "dry-run"),
            shell: string(m, "shell"),
        }),
        Some(("cache", m)) => {
            let action = match m.subcommand_name() {
                Some("info") => CacheAction::Info,
                Some("stats") => CacheAction::Stats,
                Some("clear") => CacheAction::Clear,
                other => panic!("cache action {other:?}"),
            };
            Call::Cache(CacheInput {
                action,
                dry_run: flag(m, "dry-run"),
                verbose: flag(m, "verbose"),
            })
        }
        other => panic!("unexpected command {other:?}"),
    }
}

/// What the binary would show: stdout, stderr, exit status and files.
#[derive(Debug, PartialEq)]
struct Shown {
    stdout: String,
    stderr: String,
    status: i64,
    files: Vec<(String, Option<String>)>,
}

fn apply(writes: &[OutputWrite], pty: bool, shown: &mut Shown) {
    for w in writes {
        match w {
            OutputWrite::Stdout(s) => shown.stdout.push_str(s),
            OutputWrite::Stderr(s) if pty => shown.stdout.push_str(s),
            OutputWrite::Stderr(s) => shown.stderr.push_str(s),
            OutputWrite::File { path, contents } => fs::write(path, contents).unwrap(),
        }
    }
}

/// Masks the values of timing fields, whose values differ per run.
fn mask(text: &str) -> String {
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

/// `value` with the timing fields replaced, at any depth.
fn mask_value(value: &mut Json) {
    match value {
        Json::Object(map) => {
            for (k, v) in map.iter_mut() {
                if matches!(
                    k.as_str(),
                    "started_at" | "finished_at" | "duration_seconds" | "formatted_duration"
                ) {
                    *v = Json::String("<masked>".into());
                } else {
                    mask_value(v);
                }
            }
        }
        Json::Array(items) => items.iter_mut().for_each(mask_value),
        _ => {}
    }
}

/// A YAML response document as loaded values (Psych's and the pinned
/// emitter's scalar styles differ; the root rendering contract compares
/// YAML responses by value).
fn yaml_values(text: &str) -> Option<Json> {
    if !text.starts_with("---") {
        return None;
    }
    let unmasked = text.replace(": <masked>", ": '<masked>'");
    let mut value: Json = ukiryu::from_yaml(&unmasked).ok()?;
    mask_value(&mut value);
    Some(value)
}

fn json_values(text: &str) -> Option<Json> {
    let mut value: Json = serde_json::from_str(text).ok()?;
    mask_value(&mut value);
    Some(value)
}

/// Equal output: bytes, except YAML responses (by value) and JSON files
/// (timing masked).
fn same(want: &Shown, got: &Shown) -> bool {
    if want.status != got.status || want.stderr != got.stderr {
        return false;
    }
    let stdout = match (yaml_values(&want.stdout), yaml_values(&got.stdout)) {
        (Some(a), Some(b)) => a == b,
        _ => want.stdout == got.stdout,
    };
    stdout
        && want.files.len() == got.files.len()
        && want
            .files
            .iter()
            .zip(&got.files)
            .all(|((wn, wc), (gn, gc))| {
                wn == gn
                    && match (wc, gc) {
                        (Some(a), Some(b)) => match (json_values(a), json_values(b)) {
                            (Some(x), Some(y)) => x == y,
                            _ => a == b,
                        },
                        (a, b) => a == b,
                    }
            })
}

enum Outcome {
    Shown(Shown),
    Open(String),
}

fn run_case(case: &Json, root: &Path) -> Outcome {
    let root_s = root.to_str().unwrap();
    let args: Vec<String> = case["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    let pty = case["pty"].as_bool().unwrap_or(false);
    let ctx = ExecuteContext {
        host: Host {
            platform: Platform::Linux,
            env: environment(root_s, &case["env"]),
        },
        stdout_tty: pty,
        paint: PaintSupport::AVAILABLE,
    };
    let stdin_text = case["stdin"].as_str().unwrap_or("").to_owned();
    let mut stdin = Cursor::new(stdin_text.into_bytes());
    std::env::set_current_dir(root).unwrap();
    let result: Result<CommandOutcome, ExecuteError> = match parse(&args) {
        Call::Exec(input) => execute::exec(input, &ctx, &mut stdin),
        Call::RunFile(input) => execute::run_file(input, &ctx),
        Call::Cache(input) => {
            let cache: DefinitionCache<DefinitionMetadata> = DefinitionCache::default();
            execute::cache(input, &ctx, &cache)
        }
    };
    let mut shown = Shown {
        stdout: String::new(),
        stderr: String::new(),
        status: 0,
        files: Vec::new(),
    };
    match result {
        Ok(outcome) => {
            apply(&outcome.writes, pty, &mut shown);
            shown.status = i64::from(outcome.exit_code);
        }
        Err(e @ ExecuteError::Raised { .. }) => {
            apply(e.partial(), pty, &mut shown);
            let line = format!("{}\n", e.friendly_message());
            if pty {
                // FriendlyCLI prints in red when Thor colours display on the
                // terminal (TERM not dumb, NO_COLOR unset or empty).
                let env = &ctx.host.env;
                let colors = env.get("TERM") != Some("dumb")
                    && env.get("NO_COLOR").is_none_or(str::is_empty);
                if colors {
                    shown
                        .stdout
                        .push_str(&format!("\x1b[31m{}\x1b[0m\n", e.friendly_message()));
                } else {
                    shown.stdout.push_str(&line);
                }
            } else {
                shown.stderr.push_str(&line);
            }
            shown.status = 1;
        }
        Err(e) => return Outcome::Open(e.to_string()),
    }
    if let Some(files) = case["files"].as_object() {
        for name in files.keys() {
            let path = root.join(name);
            shown
                .files
                .push((name.clone(), fs::read_to_string(path).ok()));
        }
    }
    shown.stdout = mask(&shown.stdout.replace(root_s, "{root}"));
    shown.stderr = mask(&shown.stderr.replace(root_s, "{root}"));
    Outcome::Shown(shown)
}

fn expected(case: &Json) -> Shown {
    let files = case["files"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned)))
                .collect()
        })
        .unwrap_or_default();
    Shown {
        stdout: mask(case["stdout"].as_str().unwrap_or("")),
        stderr: mask(case["stderr"].as_str().unwrap_or("")),
        status: case["status"].as_i64().unwrap(),
        files,
    }
}

#[test]
fn execute_cases_match_ruby_captures() {
    let data = cases();
    let all = data["cases"].as_array().unwrap();
    let base = std::env::temp_dir().join(format!("ukiryu-cli-execute-{}", std::process::id()));
    let mut failures = Vec::new();
    let mut counts = [0usize; 3];
    for case in all {
        let id = case["id"].as_str().unwrap();
        let root = base.join(id);
        let _ = fs::remove_dir_all(&root);
        copy_tree(&fixture_dir(), &root);
        for d in ["home", "xdg", "sysdata"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let open = OPEN.iter().any(|(c, _)| *c == id);
        let unqualified = UNQUALIFIED.iter().any(|(c, _)| *c == id);
        match run_case(case, &root) {
            Outcome::Open(reason) if open => {
                counts[1] += 1;
                eprintln!("open {id}: {reason}");
            }
            Outcome::Open(reason) => failures.push(format!("{id}: not handled: {reason}")),
            Outcome::Shown(_) if open => {
                failures.push(format!("{id}: listed as open but produced an outcome"))
            }
            Outcome::Shown(_) if unqualified => counts[2] += 1,
            Outcome::Shown(shown) => {
                let want = expected(case);
                if same(&want, &shown) {
                    counts[0] += 1;
                } else {
                    failures.push(format!(
                        "{id}:\n--- expected\n{want:#?}\n--- actual\n{shown:#?}"
                    ));
                }
            }
        }
    }
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR")).unwrap();
    let _ = fs::remove_dir_all(&base);
    eprintln!(
        "execute cases: {} exact, {} open, {} unqualified, {} failing of {}",
        counts[0],
        counts[1],
        counts[2],
        failures.len(),
        all.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
