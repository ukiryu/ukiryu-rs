//! Oracle replay for `ukiryu::command_builder`.
//!
//! Data in `tests/command_builder_data/` was captured from the pinned Ruby
//! implementation (ukiryu/ukiryu ab6df353, `lib/ukiryu/command_builder.rb`);
//! `provenance.json` names the capture scripts and corpus commit.
//!
//! - `corpus_build.jsonl`: every `build.args` row of the conformance corpus
//!   with the command definition the Ruby loader resolved for it
//!   (`corpus_commands.jsonl`) and every `Shell#format_path` call the build
//!   made. With `UKIRYU_CONFORMANCE_DIR` set, the rows are also checked
//!   against the corpus `cases/build.jsonl` itself.
//! - `synthetic_build.jsonl` / `synthetic_env.jsonl`: authored definitions
//!   covering every delimiter style, separator, value type, flag rule,
//!   positional rule and env composition rule, plus the spec fixture profile.
//!
//! Shell path formatting is replayed from the recorded calls (and the call
//! sequence is checked), so these tests pin the builder, not a shell port.
use num_bigint::BigInt;
use serde_json::Value as Json;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use ukiryu::command_builder::{
    build_args, build_env_vars, ArgumentSpec, BuildContext, CommandSpec, EnvVarSpec, OptionSpec,
};
use ukiryu::shell::ShellFormatting;
use ukiryu::types::{OptionSource, StdHost, Value};

/// Pinned counts. Changing the data requires changing these deliberately.
const CORPUS_ROWS: usize = 485;
const CORPUS_LOADER_ROWS: usize = 3;
const CORPUS_COMMANDS: usize = 40;
const SYNTHETIC_BUILD_ROWS: usize = 657;
const SYNTHETIC_ENV_ROWS: usize = 185;

/// Rows where MRI's unstable `sort_by` emits equal-position arguments in a
/// different order than declaration order; the port keeps declaration order.
const SORT_ORDER_DIVERGENT: &[&str] = &["args.mixed-positions.20"];

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/command_builder_data")
        .join(name)
}

fn jsonl(path: PathBuf) -> Vec<Json> {
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(|l| serde_json::from_str(l).expect("jsonl row"))
        .collect()
}

/// The corpus value encoding (README "Value encoding") to a Ruby value.
fn decode(j: &Json) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Integer(n.to_string().parse::<BigInt>().expect("integer")),
        Json::String(s) => Value::String(s.clone()),
        Json::Array(items) => Value::Array(items.iter().map(decode).collect()),
        Json::Object(map) => {
            if map.len() == 1 {
                let (tag, payload) = map.iter().next().unwrap();
                match tag.as_str() {
                    "$sym" => return Value::Symbol(payload.as_str().unwrap().into()),
                    "$float" => return Value::Float(ruby_float(payload.as_str().unwrap())),
                    "$map" => {
                        return Value::Hash(
                            payload
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|kv| (decode(&kv[0]), decode(&kv[1])))
                                .collect(),
                        )
                    }
                    t if t.starts_with('$') => panic!("unsupported tag {t}"),
                    _ => {}
                }
            }
            Value::Hash(
                map.iter()
                    .map(|(k, v)| (Value::String(k.clone()), decode(v)))
                    .collect(),
            )
        }
    }
}

fn ruby_float(s: &str) -> f64 {
    match s {
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        _ => s.parse().expect("float"),
    }
}

/// Top-level parameter hash (`params.transform_keys(&:to_sym)`).
fn params(j: &Json) -> BTreeMap<String, Value> {
    j.as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), decode(v))).collect())
        .unwrap_or_default()
}

/// A shell whose only used method, `format_path`, answers from the calls
/// recorded while Ruby built the same row, in the same order.
struct RecordedShell {
    shell: String,
    calls: RefCell<VecDeque<(String, String, String)>>,
}

impl RecordedShell {
    fn new(shell: &str, calls: &Json) -> Self {
        let calls = calls
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| {
                        (
                            c[0].as_str().unwrap().to_string(),
                            c[1].as_str().unwrap().to_string(),
                            c[2].as_str().unwrap().to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        RecordedShell {
            shell: shell.to_string(),
            calls: RefCell::new(calls),
        }
    }

    fn remaining(&self) -> usize {
        self.calls.borrow().len()
    }
}

impl ShellFormatting for RecordedShell {
    fn name(&self) -> &'static str {
        "recorded"
    }
    fn escape(&self, _: &str) -> String {
        unreachable!("the builder never escapes")
    }
    fn quote(&self, _: &str) -> String {
        unreachable!("the builder never quotes")
    }
    fn needs_quoting(&self, _: &str) -> bool {
        unreachable!("the builder never quotes")
    }
    fn format_path(&self, path: &str) -> String {
        let (shell, input, output) = self
            .calls
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("unrecorded format_path({path:?})"));
        assert_eq!(shell, self.shell, "format_path shell");
        assert_eq!(input, path, "format_path argument");
        output
    }
    fn env_var(&self, _: &str) -> String {
        unreachable!("the builder never references env vars")
    }
    fn join(&self, _: &str, _: &[String]) -> String {
        unreachable!("the builder never joins")
    }
}

fn spec(j: &Json) -> CommandSpec {
    serde_json::from_value(j.clone()).expect("command spec")
}

fn outcome(r: ukiryu::types::Result<Vec<String>>) -> Json {
    match r {
        Ok(args) => serde_json::json!({ "ok": args }),
        Err(e) => {
            serde_json::json!({ "error": { "class": e.class.ruby_name(), "message": e.message } })
        }
    }
}

/// Replays one build row. Returns the outcome and whether every recorded
/// `format_path` call was consumed.
fn replay(command: &Json, row: &Json, shell: &str, platform: &str) -> (Json, bool) {
    let recorded = RecordedShell::new(shell, &row["format_path"]);
    let ctx = BuildContext {
        shell: &recorded,
        platform,
        host: &StdHost,
    };
    let p = params(&row_params(row));
    let got = outcome(build_args(&spec(command), &p, &ctx));
    (got, recorded.remaining() == 0)
}

fn row_params(row: &Json) -> Json {
    if row.get("input").is_some() {
        row["input"]["params"].clone()
    } else {
        row["params"].clone()
    }
}

/// `Model#[]` answers recorded by the oracle for the keys `Type.validate`
/// reads must equal what the builder's input types return.
fn check_model_index(id: &str, list: &Json, arg: bool, failures: &mut Vec<String>) {
    for def in list.as_array().into_iter().flatten() {
        let Some(index) = def.get("$index").and_then(Json::as_object) else {
            continue;
        };
        for (key, answer) in index {
            let Some(expected) = answer.get("ok") else {
                failures.push(format!("{id}: Model#[]({key}) raised in Ruby: {answer}"));
                continue;
            };
            let got = if arg {
                serde_json::from_value::<ArgumentSpec>(def.clone())
                    .unwrap()
                    .option(key)
            } else {
                serde_json::from_value::<OptionSpec>(def.clone())
                    .unwrap()
                    .option(key)
            };
            if got != decode(expected) {
                failures.push(format!("{id}: Model#[]({key}) = {got:?}, Ruby {expected}"));
            }
        }
    }
}

fn check_index_for(id: &str, command: &Json, failures: &mut Vec<String>) {
    check_model_index(id, &command["options"], false, failures);
    check_model_index(id, &command["post_options"], false, failures);
    check_model_index(id, &command["arguments"], true, failures);
}

fn finish(name: &str, failures: Vec<String>) {
    if !failures.is_empty() {
        let shown: Vec<_> = failures.iter().take(25).collect();
        panic!(
            "{name}: {} failure(s)\n{}",
            failures.len(),
            shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn corpus_build_args_rows() {
    let commands: BTreeMap<String, Json> = jsonl(data("corpus_commands.jsonl"))
        .into_iter()
        .map(|r| (r["key"].as_str().unwrap().to_string(), r["command"].clone()))
        .collect();
    assert_eq!(commands.len(), CORPUS_COMMANDS, "pinned command count");
    let rows = jsonl(data("corpus_build.jsonl"));
    assert_eq!(rows.len(), CORPUS_ROWS, "pinned corpus row count");

    let mut failures = Vec::new();
    let (mut executed, mut loader) = (0, 0);
    for row in &rows {
        let id = row["id"].as_str().unwrap();
        if let Some(err) = row.get("load_error") {
            // The register loader (Tool.get chain) owns these: it ends before
            // build_args. Pinned here so the count cannot drift silently.
            loader += 1;
            if row["expect"]["error"] != *err {
                failures.push(format!("{id}: loader error differs from corpus"));
            }
            continue;
        }
        let key = row["command"].as_str().unwrap();
        let command = &commands[key];
        let input = &row["input"];
        let (got, consumed) = replay(
            command,
            row,
            input["shell"].as_str().unwrap(),
            input["platform"].as_str().unwrap(),
        );
        executed += 1;
        if got != row["expect"] {
            failures.push(format!("{id}: got {got}, expected {}", row["expect"]));
        } else if !consumed {
            failures.push(format!("{id}: recorded format_path calls left over"));
        }
    }
    for (key, command) in &commands {
        check_index_for(key, command, &mut failures);
    }
    assert_eq!(loader, CORPUS_LOADER_ROWS, "pinned loader-owned rows");
    assert_eq!(executed + loader, CORPUS_ROWS);
    finish("corpus build.args", failures);
    eprintln!("corpus build.args: {executed} executed, {loader} loader-owned, 0 failed");
}

/// With `UKIRYU_CONFORMANCE_DIR`, the committed rows must be exactly the
/// corpus rows (ids, inputs, expectations, applicability).
#[test]
fn corpus_rows_match_conformance_dir() {
    let Ok(dir) = std::env::var("UKIRYU_CONFORMANCE_DIR") else {
        eprintln!("UKIRYU_CONFORMANCE_DIR unset: corpus identity check not run");
        return;
    };
    let corpus = jsonl(PathBuf::from(dir).join("cases/build.jsonl"));
    let ours = jsonl(data("corpus_build.jsonl"));
    assert_eq!(corpus.len(), CORPUS_ROWS, "corpus build row count");
    let ours: BTreeMap<&str, &Json> = ours
        .iter()
        .map(|r| (r["id"].as_str().unwrap(), r))
        .collect();
    for row in &corpus {
        let id = row["id"].as_str().unwrap();
        let mine = ours
            .get(id)
            .unwrap_or_else(|| panic!("{id}: missing from fixture"));
        for field in ["input", "expect", "applicability"] {
            assert_eq!(mine[field], row[field], "{id}: {field}");
        }
    }
}

#[test]
fn synthetic_build_args_rows() {
    let rows = jsonl(data("synthetic_build.jsonl"));
    assert_eq!(
        rows.len(),
        SYNTHETIC_BUILD_ROWS,
        "pinned synthetic build rows"
    );
    let mut failures = Vec::new();
    let mut divergent = HashSet::new();
    let (mut ok_rows, mut err_rows) = (0, 0);
    for row in &rows {
        let id = row["id"].as_str().unwrap();
        let (got, consumed) = replay(
            &row["command"],
            row,
            row["shell"].as_str().unwrap(),
            row["platform"].as_str().unwrap(),
        );
        check_index_for(id, &row["command"], &mut failures);
        if row["expect"].get("ok").is_some() {
            ok_rows += 1;
        } else {
            err_rows += 1;
        }
        if SORT_ORDER_DIVERGENT.contains(&id) {
            // Same elements, declaration order instead of MRI's order.
            let mut a: Vec<_> = got["ok"]
                .as_array()
                .unwrap()
                .iter()
                .map(Json::to_string)
                .collect();
            let mut b: Vec<_> = row["expect"]["ok"]
                .as_array()
                .unwrap()
                .iter()
                .map(Json::to_string)
                .collect();
            assert_ne!(a, b, "{id}: no longer divergent; drop it from the list");
            a.sort();
            b.sort();
            assert_eq!(a, b, "{id}: divergent row must be a reordering");
            divergent.insert(id);
            continue;
        }
        if got != row["expect"] {
            failures.push(format!("{id}: got {got}, expected {}", row["expect"]));
        } else if !consumed {
            failures.push(format!("{id}: recorded format_path calls left over"));
        }
    }
    assert_eq!(divergent.len(), SORT_ORDER_DIVERGENT.len());
    finish("synthetic build_args", failures);
    eprintln!("synthetic build_args: {ok_rows} ok rows, {err_rows} error rows");
}

#[test]
fn synthetic_env_rows() {
    let rows = jsonl(data("synthetic_env.jsonl"));
    assert_eq!(rows.len(), SYNTHETIC_ENV_ROWS, "pinned synthetic env rows");
    let mut failures = Vec::new();
    for row in &rows {
        let id = row["id"].as_str().unwrap();
        let sets: BTreeMap<String, Vec<EnvVarSpec>> =
            serde_json::from_value(row["env_var_sets"].clone()).expect("env var sets");
        let p = params(&row["params"]);
        let env = build_env_vars(
            &spec(&row["command"]),
            Some(&sets),
            &p,
            row["platform"].as_str().unwrap(),
        );
        let got = serde_json::json!({ "ok": env.0.iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>() });
        if got != row["expect"] {
            failures.push(format!("{id}: got {got}, expected {}", row["expect"]));
        }
    }
    finish("synthetic build_env_vars", failures);
}

#[test]
fn invocation_joins_only_at_the_shell_boundary() {
    struct Joiner;
    impl ShellFormatting for Joiner {
        fn name(&self) -> &'static str {
            "joiner"
        }
        fn escape(&self, v: &str) -> String {
            v.into()
        }
        fn quote(&self, v: &str) -> String {
            format!("<{v}>")
        }
        fn needs_quoting(&self, _: &str) -> bool {
            true
        }
        fn format_path(&self, p: &str) -> String {
            p.into()
        }
        fn env_var(&self, n: &str) -> String {
            n.into()
        }
        fn join(&self, exe: &str, args: &[String]) -> String {
            std::iter::once(exe.to_string())
                .chain(args.iter().map(|a| self.quote(a)))
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
    let command: CommandSpec = serde_json::from_value(serde_json::json!({
        "name": "c",
        "options": [{"name": "o", "cli": "-o"}],
        "arguments": [{"name": "f"}],
        "env_vars": [{"name": "A", "from": "o"}]
    }))
    .unwrap();
    let p: BTreeMap<String, Value> = [
        ("o".to_string(), Value::str("x y")),
        ("f".to_string(), Value::str("in")),
    ]
    .into();
    let ctx = BuildContext {
        shell: &Joiner,
        platform: "linux",
        host: &StdHost,
    };
    let inv = ukiryu::command_builder::assemble("tool", &command, None, &p, &ctx).unwrap();
    assert_eq!(inv.args, ["-o", "x y", "in"]);
    assert_eq!(inv.env.get("A"), Some("x y"));
    assert_eq!(inv.command_line(&Joiner), "tool <-o> <x y> <in>");
}
