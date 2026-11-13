//! Conformance consumer: the shared corpus (`corpus/conformance`, issue #23)
//! executed through the Rust core's own APIs.
//!
//! Every row that applies to this platform and to this engine's declared
//! capabilities runs through the matching op adapter under
//! `tests/conformance/`, and its outcome must equal the row's frozen Ruby
//! expectation under the corpus comparison rules. There is no skip list and
//! no expected-failure list. An op without an adapter fails. The only rows
//! compared against something other than their frozen expectation are those
//! listed in `tests/conformance/migrated.json`, each traced to an owner
//! decision; that file also pins the frozen expectation it replaces, so the
//! original observation stays in the corpus unchanged and any change to it
//! is noticed.
//!
//! The corpus location is `UKIRYU_CONFORMANCE_DIR` (default: the repository
//! copy). Set `UKIRYU_CONFORMANCE_REPORT` to write a JSON report with the
//! per-op, per-platform coverage and every failure.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

// Shared helpers: each op adapter uses a different subset.
#[allow(dead_code)]
mod codec;
#[allow(dead_code)]
mod harness;
#[allow(dead_code)]
mod json;
mod ops_build;
mod ops_lint_schema;
mod ops_resolve;
mod ops_shell;
mod ops_validate;
mod ops_yaml;
mod sha256;

use harness::{Ctx, Outcome, RowError, UNKNOWN_OP};
use json::{canonical, clip, parse, structural_eq, Oj};

/// An engine: runs one op on one row input.
type Runner = dyn Fn(&str, &Oj, &Ctx) -> Outcome + Sync;

// ------------------------------------------------------------ lockstep pins

/// Corpus size, pinned deliberately: when the corpus moves, this file moves
/// with it in the same change.
const PINNED_TOTAL: usize = 14177;
const PINNED_IDS_SHA256: &str = "80dd3593a329ab75b6291374121703c60d9b4fa1b06e7df984c07ffcde5bc34b";
/// Rows applicable per platform for an engine with every capability.
const PINNED_APPLICABLE: [(&str, usize); 3] =
    [("linux", 14177), ("macos", 14156), ("windows", 14139)];
/// Rows this engine executes per platform. The core crate's test engine
/// provides `fs` and `process` but not `cli`: the CLI binary is a separate
/// crate, so the 21 Linux `cli-output` rows are not run here (they remain
/// an acceptance gate for the CLI engine).
const ENGINE_CAPABILITIES: [&str; 2] = ["fs", "process"];
const ENGINE_EXECUTED: [(&str, usize); 3] =
    [("linux", 14156), ("macos", 14156), ("windows", 14139)];

const KINDS: [&str; 8] = [
    "resolve",
    "build",
    "escape",
    "validate",
    "lint",
    "schema",
    "cli-output",
    "yaml",
];
const PLATFORMS: [&str; 3] = ["linux", "macos", "windows"];
const CAPABILITIES: [&str; 3] = ["fs", "process", "cli"];
const OPS: [&str; 19] = [
    "shell.escape",
    "shell.quote",
    "shell.needs_quoting",
    "shell.join",
    "shell.env_var",
    "shell.format_path",
    "type.validate",
    "build.args",
    "resolve.extract_version",
    "resolve.version_spec",
    "resolve.range",
    "resolve.shell_detect",
    "lint.string",
    "schema.validate_profile",
    "schema.definition_validate",
    "yaml.load",
    "yaml.model_load",
    "yaml.lutaml_roundtrip",
    "cli.run",
];
const ROW_KEYS: [&str; 12] = [
    "id",
    "kind",
    "op",
    "api",
    "input",
    "expect",
    "compare",
    "error_match",
    "applicability",
    "sources",
    "decision",
    "note",
];
const REQUIRED_KEYS: [&str; 10] = [
    "id",
    "kind",
    "op",
    "api",
    "input",
    "expect",
    "compare",
    "error_match",
    "applicability",
    "sources",
];

fn host_platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        panic!("the corpus defines no platform for this target")
    }
}

fn pinned(table: &[(&str, usize)], platform: &str) -> usize {
    table
        .iter()
        .find(|(p, _)| *p == platform)
        .map(|(_, n)| *n)
        .expect("platform pin")
}

// ------------------------------------------------------------- corpus data

/// The corpus as bytes, so negative controls can mutate it in memory.
#[derive(Clone)]
struct Corpus {
    dir: PathBuf,
    manifest: Oj,
    /// `cases/<kind>.jsonl` to bytes.
    case_files: BTreeMap<String, Vec<u8>>,
    /// Manifest input path to bytes (None when missing).
    inputs: BTreeMap<String, Option<Vec<u8>>>,
    migrated: Oj,
}

fn corpus_dir() -> PathBuf {
    match std::env::var_os("UKIRYU_CONFORMANCE_DIR") {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance"),
    }
}

fn load_corpus() -> Corpus {
    let dir = corpus_dir();
    let manifest_text = std::fs::read_to_string(dir.join("manifest.json"))
        .unwrap_or_else(|e| panic!("read {}/manifest.json: {e}", dir.display()));
    let manifest = parse(&manifest_text).expect("manifest JSON");
    let mut case_files = BTreeMap::new();
    let mut names: Vec<_> = std::fs::read_dir(dir.join("cases"))
        .expect("cases dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|n| n.ends_with(".jsonl"))
        .collect();
    names.sort();
    for n in names {
        let bytes = std::fs::read(dir.join("cases").join(&n)).expect("case file");
        case_files.insert(format!("cases/{n}"), bytes);
    }
    let mut inputs = BTreeMap::new();
    for inp in manifest.get("inputs").and_then(Oj::arr).expect("inputs") {
        let rel = inp.field_str("path").expect("input path").to_string();
        let bytes = std::fs::read(dir.join("inputs").join(&rel)).ok();
        inputs.insert(rel, bytes);
    }
    let migrated_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/migrated.json");
    let migrated = parse(&std::fs::read_to_string(migrated_path).expect("migrated.json"))
        .expect("migrated.json JSON");
    Corpus {
        dir,
        manifest,
        case_files,
        inputs,
        migrated,
    }
}

#[derive(Clone)]
struct Row {
    id: String,
    op: String,
    input: Oj,
    expect: Oj,
    error_match: String,
    platforms: Vec<String>,
    requires: Vec<String>,
    decision: Option<String>,
}

fn strings(v: Option<&Oj>) -> Option<Vec<String>> {
    v.and_then(Oj::arr)?
        .iter()
        .map(|x| x.str().map(str::to_string))
        .collect()
}

/// Parses every case line and checks the row contract (README, schema).
fn parse_rows(c: &Corpus, problems: &mut Vec<String>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (file, bytes) in &c.case_files {
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                problems.push(format!("{file}: not UTF-8"));
                continue;
            }
        };
        for (n, line) in text.split('\n').enumerate() {
            if line.is_empty() {
                continue;
            }
            let raw = match parse(line) {
                Ok(r) => r,
                Err(e) => {
                    problems.push(format!("{file}:{}: invalid JSON ({e})", n + 1));
                    continue;
                }
            };
            let id = raw
                .get("id")
                .and_then(Oj::str)
                .unwrap_or("<no id>")
                .to_string();
            if let Oj::Obj(pairs) = &raw {
                for (k, _) in pairs {
                    if !ROW_KEYS.contains(&k.as_str()) {
                        problems.push(format!("{id}: unknown key {k:?}"));
                    }
                }
            }
            for k in REQUIRED_KEYS {
                if raw.get(k).is_none() {
                    problems.push(format!("{id}: missing {k}"));
                }
            }
            let kind = raw.get("kind").and_then(Oj::str).unwrap_or("");
            let op = raw.get("op").and_then(Oj::str).unwrap_or("");
            if !KINDS.contains(&kind) {
                problems.push(format!("{id}: unknown kind {kind:?}"));
            }
            if !OPS.contains(&op) {
                problems.push(format!("{id}: unknown op {op:?}"));
            }
            if !id.starts_with(&format!("{kind}.")) {
                problems.push(format!("{id}: id must start with kind"));
            }
            if format!("cases/{kind}.jsonl") != *file {
                problems.push(format!("{id}: kind {kind} stored in {file}"));
            }
            let expect = raw.get("expect").cloned().unwrap_or(Oj::Null);
            let expect_ok = match &expect {
                Oj::Obj(p) if p.len() == 1 && p[0].0 == "ok" => true,
                Oj::Obj(p) if p.len() == 1 && p[0].0 == "error" => {
                    let e = &p[0].1;
                    matches!(e, Oj::Obj(f) if f.len() == 2)
                        && e.get("class").and_then(Oj::str).is_some()
                        && e.get("message").and_then(Oj::str).is_some()
                }
                _ => false,
            };
            if !expect_ok {
                problems.push(format!(
                    "{id}: expect must be {{ok}} or {{error: {{class, message}}}}"
                ));
            }
            if raw.get("compare") != Some(&Oj::s("structural")) {
                problems.push(format!("{id}: compare must be structural"));
            }
            let error_match = raw.get("error_match").and_then(Oj::str).unwrap_or("");
            if !["class", "class_and_message"].contains(&error_match) {
                problems.push(format!("{id}: bad error_match"));
            }
            let app = raw.get("applicability");
            let platforms = strings(app.and_then(|a| a.get("platforms")));
            let requires = strings(app.and_then(|a| a.get("requires")));
            let (platforms, requires) = match (platforms, requires) {
                (Some(p), Some(r))
                    if !p.is_empty()
                        && p.iter().all(|x| PLATFORMS.contains(&x.as_str()))
                        && r.iter().all(|x| CAPABILITIES.contains(&x.as_str())) =>
                {
                    (p, r)
                }
                _ => {
                    problems.push(format!(
                        "{id}: applicability needs non-empty known platforms and known requires"
                    ));
                    (Vec::new(), Vec::new())
                }
            };
            rows.push(Row {
                id,
                op: op.to_string(),
                input: raw.get("input").cloned().unwrap_or(Oj::Null),
                expect,
                error_match: error_match.to_string(),
                platforms,
                requires,
                decision: raw.get("decision").and_then(Oj::str).map(str::to_string),
            });
        }
    }
    rows
}

/// Manifest seal, lockstep pins and row contract. Empty means intact.
fn integrity(c: &Corpus) -> (Vec<Row>, Vec<String>) {
    let mut problems = Vec::new();
    let rows = parse_rows(c, &mut problems);
    let mut seen = BTreeSet::new();
    for r in &rows {
        if !seen.insert(r.id.as_str()) {
            problems.push(format!("duplicate id {}", r.id));
        }
    }
    let mc = c.manifest.get("cases").expect("manifest cases");
    let total = mc.get("total").and_then(|t| match t {
        Oj::Num(n) => n.parse::<usize>().ok(),
        _ => None,
    });
    if total != Some(rows.len()) {
        problems.push(format!("case count {} != manifest {total:?}", rows.len()));
    }
    if rows.len() != PINNED_TOTAL {
        problems.push(format!(
            "case count {} != pinned {PINNED_TOTAL} (move the pin deliberately)",
            rows.len()
        ));
    }
    let mut ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    let ids_sha = sha256::hex(ids.join("\n").as_bytes());
    if mc.get("ids_sha256").and_then(Oj::str) != Some(ids_sha.as_str()) {
        problems.push("case id set differs from manifest ids_sha256".into());
    }
    if ids_sha != PINNED_IDS_SHA256 {
        problems.push(format!(
            "case id set sha256 {ids_sha} differs from the pinned set"
        ));
    }
    let files = mc.get("files");
    for (rel, bytes) in &c.case_files {
        let got = sha256::hex(bytes);
        let want = files.and_then(|f| f.get(rel)).and_then(Oj::str);
        if want != Some(got.as_str()) {
            problems.push(format!("{rel}: sha256 {got} != manifest {want:?}"));
        }
    }
    if let Some(Oj::Obj(listed)) = files {
        for (rel, _) in listed {
            if !c.case_files.contains_key(rel) {
                problems.push(format!("{rel}: listed in manifest but missing"));
            }
        }
    }
    for inp in c.manifest.get("inputs").and_then(Oj::arr).unwrap_or(&[]) {
        let rel = inp.field_str("path").unwrap_or("");
        let want = inp.field_str("sha256").unwrap_or("");
        let got = match c.inputs.get(rel) {
            Some(Some(b)) => sha256::hex(b),
            _ => "missing".into(),
        };
        if got != want {
            problems.push(format!("inputs/{rel}: sha256 {got} != {want}"));
        }
    }
    for (platform, want) in PINNED_APPLICABLE {
        let in_manifest = c
            .manifest
            .get("applicable")
            .and_then(|a| a.get(platform))
            .cloned();
        if in_manifest != Some(Oj::Num(want.to_string())) {
            problems.push(format!(
                "manifest applicable.{platform} {in_manifest:?} != pinned {want}"
            ));
        }
        let all = rows
            .iter()
            .filter(|r| r.platforms.iter().any(|p| p == platform))
            .count();
        if all != want {
            problems.push(format!("{all} rows apply on {platform}, pinned {want}"));
        }
        let engine = rows.iter().filter(|r| applies(r, platform)).count();
        let pin = pinned(&ENGINE_EXECUTED, platform);
        if engine != pin {
            problems.push(format!(
                "{engine} rows apply to this engine on {platform}, pinned {pin}"
            ));
        }
    }
    problems.extend(migration_problems(c, &rows));
    (rows, problems)
}

fn applies(r: &Row, platform: &str) -> bool {
    r.platforms.iter().any(|p| p == platform)
        && r.requires
            .iter()
            .all(|q| ENGINE_CAPABILITIES.contains(&q.as_str()))
}

// ------------------------------------------------------ migrated expectations

/// Bases a migrated expectation may cite. D1/D2 entries are allowed only on
/// rows the corpus itself marks with that decision.
const MIGRATION_BASES: [&str; 4] = ["D1", "D2", "version", "emitter"];

struct Migration {
    expect: Oj,
}

fn migrations(c: &Corpus) -> BTreeMap<String, Migration> {
    c.migrated
        .get("rows")
        .and_then(Oj::arr)
        .unwrap_or(&[])
        .iter()
        .filter_map(|m| {
            Some((
                m.field_str("id").ok()?.to_string(),
                Migration {
                    expect: m.get("expect")?.clone(),
                },
            ))
        })
        .collect()
}

fn migration_problems(c: &Corpus, rows: &[Row]) -> Vec<String> {
    let mut problems = Vec::new();
    let by_id: BTreeMap<&str, &Row> = rows.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut seen = BTreeSet::new();
    let entries = match c.migrated.get("rows").and_then(Oj::arr) {
        Some(e) => e,
        None => return vec!["migrated.json: missing rows array".into()],
    };
    for m in entries {
        let id = m.field_str("id").unwrap_or("<no id>");
        if !seen.insert(id.to_string()) {
            problems.push(format!("migrated {id}: listed twice"));
        }
        let basis = m.field_str("basis").unwrap_or("");
        if !MIGRATION_BASES.contains(&basis) {
            problems.push(format!("migrated {id}: unknown basis {basis:?}"));
        }
        let trace = m.field_str("trace").unwrap_or("");
        if !trace.contains('#') {
            problems.push(format!(
                "migrated {id}: trace must cite the governing issue"
            ));
        }
        let Some(row) = by_id.get(id) else {
            problems.push(format!("migrated {id}: no such corpus row"));
            continue;
        };
        if (basis == "D1" || basis == "D2") && row.decision.as_deref() != Some(basis) {
            problems.push(format!(
                "migrated {id}: basis {basis} but the row's decision is {:?}",
                row.decision
            ));
        }
        let frozen = sha256::hex(canonical(&row.expect).as_bytes());
        if m.field_str("frozen_expect_sha256").ok() != Some(frozen.as_str()) {
            problems.push(format!(
                "migrated {id}: frozen expectation changed (now sha256 {frozen}); re-review"
            ));
        }
        match m.get("expect") {
            Some(Oj::Obj(p)) if p.len() == 1 && (p[0].0 == "ok" || p[0].0 == "error") => {}
            _ => problems.push(format!("migrated {id}: expect must be {{ok}} or {{error}}")),
        }
        if m.get("expect")
            .is_some_and(|e| structural_eq(e, &row.expect))
        {
            problems.push(format!("migrated {id}: equals the frozen expectation"));
        }
    }
    problems
}

// ----------------------------------------------------------------- execution

/// Runs one op through its core adapter.
fn dispatch(op: &str, input: &Oj, ctx: &Ctx) -> Outcome {
    match op {
        "shell.escape"
        | "shell.quote"
        | "shell.needs_quoting"
        | "shell.join"
        | "shell.env_var"
        | "shell.format_path" => ops_shell::run(op, input, ctx),
        "type.validate" => ops_validate::type_validate(input, ctx),
        "build.args" => ops_build::build_args(input, ctx),
        "resolve.version_spec" => ops_build::version_spec(input, ctx),
        "resolve.extract_version" => ops_resolve::extract_version(input, ctx),
        "resolve.range" => ops_resolve::range(input, ctx),
        "resolve.shell_detect" => ops_resolve::shell_detect(input, ctx),
        "lint.string" => ops_lint_schema::lint_string(input, ctx),
        "schema.validate_profile" => ops_lint_schema::validate_profile(input, ctx),
        "schema.definition_validate" => ops_lint_schema::definition_validate(input, ctx),
        "yaml.load" => ops_yaml::load(input, ctx),
        "yaml.model_load" => ops_yaml::model_load(input, ctx),
        "yaml.lutaml_roundtrip" => ops_yaml::lutaml_roundtrip(input, ctx),
        other => Err(RowError::new(
            UNKNOWN_OP,
            format!("no core adapter for op {other}"),
        )),
    }
}

/// Executes with host substitutions applied; a panic is a failed row.
fn execute(op: &str, input: &Oj, ctx: &Ctx, run: &Runner) -> Outcome {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(op, input, ctx)));
    match result {
        Ok(Ok(v)) => Ok(ctx.scrub(v)),
        Ok(Err(e)) => Err(RowError::new(e.class, ctx.scrub_str(&e.message))),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            Err(RowError::new("Conformance::Panic", msg))
        }
    }
}

/// None when the outcome satisfies the expectation, else the reason.
fn mismatch(expect: &Oj, error_match: &str, outcome: &Outcome) -> Option<String> {
    if let Some(want) = expect.get("ok") {
        return match outcome {
            Err(e) => Some(format!(
                "expected ok, got error {}: {}",
                e.class,
                clip(&e.message, 300)
            )),
            Ok(got) if structural_eq(want, got) => None,
            Ok(got) => Some(format!(
                "value differs: expected {} got {}",
                clip(&canonical(want), 300),
                clip(&canonical(got), 300)
            )),
        };
    }
    let want = expect.get("error")?;
    let class = want.get("class").and_then(Oj::str).unwrap_or("");
    let message = want.get("message").and_then(Oj::str).unwrap_or("");
    match outcome {
        Ok(got) => Some(format!(
            "expected error {class}, got ok {}",
            clip(&canonical(got), 300)
        )),
        Err(e) if e.class != class => Some(format!(
            "error class differs: expected {class} got {} ({})",
            e.class,
            clip(&e.message, 200)
        )),
        Err(e)
            if error_match == "class_and_message" && e.message.as_bytes() != message.as_bytes() =>
        {
            Some(format!(
                "error message differs: expected {:?} got {:?}",
                clip(message, 200),
                clip(&e.message, 200)
            ))
        }
        Err(_) => None,
    }
}

fn sandbox_home() -> String {
    let dir = std::env::temp_dir().join(format!("ukiryu-conformance-home-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("sandbox home");
    dir.to_string_lossy().into_owned()
}

fn context(c: &Corpus) -> Ctx {
    let dir_str = c.dir.to_string_lossy().into_owned();
    Ctx {
        dir: c.dir.clone(),
        inputs: format!("{dir_str}/inputs"),
        home: sandbox_home(),
        platform: host_platform(),
    }
}

struct Failure {
    id: String,
    op: String,
    reason: String,
}

struct RunReport {
    executed: Vec<(String, String, bool)>,
    failures: Vec<Failure>,
}

/// Executes every row that applies to this engine on this platform.
fn run_rows(
    rows: &[Row],
    migrated: &BTreeMap<String, Migration>,
    ctx: &Ctx,
    run: &Runner,
) -> RunReport {
    let applicable: Vec<&Row> = rows.iter().filter(|r| applies(r, ctx.platform)).collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .min(8);
    let next = AtomicUsize::new(0);
    let results: Vec<Vec<(usize, Option<String>, bool)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(row) = applicable.get(i) else { break };
                        let outcome = execute(&row.op, &row.input, ctx, run);
                        let (expect, is_migrated) = match migrated.get(&row.id) {
                            Some(m) => (&m.expect, true),
                            None => (&row.expect, false),
                        };
                        out.push((i, mismatch(expect, &row.error_match, &outcome), is_migrated));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker"))
            .collect()
    });
    let mut flat: Vec<_> = results.into_iter().flatten().collect();
    flat.sort_by_key(|(i, _, _)| *i);
    let mut report = RunReport {
        executed: Vec::new(),
        failures: Vec::new(),
    };
    for (i, reason, is_migrated) in flat {
        let row = applicable[i];
        report
            .executed
            .push((row.id.clone(), row.op.clone(), is_migrated));
        if let Some(reason) = reason {
            report.failures.push(Failure {
                id: row.id.clone(),
                op: row.op.clone(),
                reason,
            });
        }
    }
    report
}

fn write_report(platform: &str, rows: &[Row], report: &RunReport) {
    let Some(path) = std::env::var_os("UKIRYU_CONFORMANCE_REPORT") else {
        return;
    };
    let mut by_op: BTreeMap<&str, [usize; 4]> = BTreeMap::new();
    for r in rows {
        by_op.entry(r.op.as_str()).or_default()[0] += 1;
    }
    let failed: BTreeSet<&str> = report.failures.iter().map(|f| f.id.as_str()).collect();
    for (id, op, is_migrated) in &report.executed {
        let e = by_op.entry(op.as_str()).or_default();
        e[1] += 1;
        if failed.contains(id.as_str()) {
            e[2] += 1;
        }
        if *is_migrated {
            e[3] += 1;
        }
    }
    let ops = Oj::Obj(
        by_op
            .iter()
            .map(|(op, [total, executed, failed, migrated])| {
                (
                    op.to_string(),
                    Oj::obj(vec![
                        ("total", Oj::Num(total.to_string())),
                        ("executed", Oj::Num(executed.to_string())),
                        ("failed", Oj::Num(failed.to_string())),
                        ("migrated", Oj::Num(migrated.to_string())),
                    ]),
                )
            })
            .collect(),
    );
    let doc = Oj::obj(vec![
        ("platform", Oj::s(platform)),
        (
            "capabilities",
            Oj::Arr(ENGINE_CAPABILITIES.iter().map(|c| Oj::s(*c)).collect()),
        ),
        ("total", Oj::Num(rows.len().to_string())),
        ("executed", Oj::Num(report.executed.len().to_string())),
        ("failed", Oj::Num(report.failures.len().to_string())),
        ("by_op", ops),
        (
            "failures",
            Oj::Arr(
                report
                    .failures
                    .iter()
                    .map(|f| {
                        Oj::obj(vec![
                            ("id", Oj::s(f.id.clone())),
                            ("op", Oj::s(f.op.clone())),
                            ("reason", Oj::s(f.reason.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    std::fs::write(&path, canonical(&doc)).expect("write conformance report");
}

// --------------------------------------------------------------------- tests

#[test]
fn corpus_is_sealed_and_counts_are_in_lockstep() {
    let c = load_corpus();
    let (_, problems) = integrity(&c);
    assert!(
        problems.is_empty(),
        "{} integrity problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn every_applicable_row_matches_through_the_core() {
    let c = load_corpus();
    let (rows, problems) = integrity(&c);
    assert!(problems.is_empty(), "integrity: {}", problems.join("\n"));
    let ctx = context(&c);
    let migrated = migrations(&c);
    let report = run_rows(&rows, &migrated, &ctx, &dispatch);
    write_report(ctx.platform, &rows, &report);

    let want = pinned(&ENGINE_EXECUTED, ctx.platform);
    let mut by_op: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (_, op, _) in &report.executed {
        by_op.entry(op.as_str()).or_default().0 += 1;
    }
    for f in &report.failures {
        by_op.entry(f.op.as_str()).or_default().1 += 1;
    }
    let table: Vec<String> = by_op
        .iter()
        .map(|(op, (n, bad))| format!("  {op:28} executed {n:6} failed {bad:6}"))
        .collect();
    eprintln!(
        "conformance on {}: executed {} of {} rows (pinned {want}), failed {}, migrated {}\n{}",
        ctx.platform,
        report.executed.len(),
        rows.len(),
        report.failures.len(),
        report.executed.iter().filter(|e| e.2).count(),
        table.join("\n")
    );
    let shown: Vec<String> = report
        .failures
        .iter()
        .take(60)
        .map(|f| format!("{}: {}", f.id, f.reason))
        .collect();
    assert_eq!(
        report.executed.len(),
        want,
        "executed count differs from the pin"
    );
    assert!(
        report.failures.is_empty(),
        "{} rows diverge from the corpus (first {}):\n{}",
        report.failures.len(),
        shown.len(),
        shown.join("\n")
    );
    // Every migrated expectation must apply to a row this engine ran.
    let ran: BTreeSet<&str> = report.executed.iter().map(|e| e.0.as_str()).collect();
    for id in migrated.keys() {
        let row = rows.iter().find(|r| &r.id == id).expect("migrated row");
        if applies(row, ctx.platform) {
            assert!(ran.contains(id.as_str()), "migrated {id} never ran");
        }
    }
}

// ----------------------------------------------------------- negative controls

fn mutate_case_line(c: &mut Corpus, file: &str, f: impl Fn(&str) -> String) {
    let bytes = c.case_files.get_mut(file).expect("case file");
    let text = String::from_utf8(bytes.clone()).unwrap();
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    lines[0] = f(&lines[0]);
    *bytes = lines.join("\n").into_bytes();
}

fn field_mut<'a>(v: &'a mut Oj, key: &str) -> Option<&'a mut Oj> {
    match v {
        Oj::Obj(pairs) => pairs.iter_mut().find(|(k, _)| k == key).map(|(_, x)| x),
        _ => None,
    }
}

/// Re-seal a mutated case file so only the deliberate defect remains.
fn reseal(c: &mut Corpus, file: &str) {
    let sha = sha256::hex(&c.case_files[file]);
    let digest = field_mut(&mut c.manifest, "cases")
        .and_then(|cases| field_mut(cases, "files"))
        .and_then(|files| field_mut(files, file))
        .expect("manifest digest for the case file");
    *digest = Oj::Str(sha);
}

fn expect_problem(c: &Corpus, needle: &str, what: &str) {
    let (_, problems) = integrity(c);
    assert!(
        problems.iter().any(|p| p.contains(needle)),
        "negative control {what:?} was not rejected (wanted {needle:?}); got {problems:?}"
    );
}

#[test]
fn negative_controls_reject_broken_corpora() {
    let base = load_corpus();
    let file = "cases/validate.jsonl";

    let mut c = base.clone();
    mutate_case_line(&mut c, file, |l| l.replacen("\"ok\":", "\"ok\": ", 1));
    expect_problem(&c, "sha256", "unsealed edit");

    let mut c = base.clone();
    mutate_case_line(&mut c, file, |l| format!("{l}\n{l}"));
    reseal(&mut c, file);
    expect_problem(&c, "duplicate id", "duplicate case");

    let mut c = base.clone();
    mutate_case_line(&mut c, file, |_| String::new());
    reseal(&mut c, file);
    expect_problem(&c, "case count", "missing case");

    let mut c = base.clone();
    mutate_case_line(&mut c, file, |l| {
        l.replacen("\"op\":\"type.validate\"", "\"op\":\"type.frobnicate\"", 1)
    });
    reseal(&mut c, file);
    expect_problem(&c, "unknown op", "unknown op");

    let mut c = base.clone();
    mutate_case_line(&mut c, file, |l| {
        l.replacen("{\"id\":", "{\"extra\":1,\"id\":", 1)
    });
    reseal(&mut c, file);
    expect_problem(&c, "unknown key", "unknown field");

    let mut c = base.clone();
    mutate_case_line(&mut c, file, |l| {
        l.replacen(
            "\"platforms\":[\"linux\",\"macos\",\"windows\"]",
            "\"platforms\":[]",
            1,
        )
    });
    reseal(&mut c, file);
    expect_problem(&c, "applicability", "empty applicability");

    let mut c = base.clone();
    let platform = host_platform();
    let elsewhere = PLATFORMS.iter().find(|p| **p != platform).unwrap();
    mutate_case_line(&mut c, file, |l| {
        l.replacen(
            "\"platforms\":[\"linux\",\"macos\",\"windows\"]",
            &format!("\"platforms\":[\"{elsewhere}\"]"),
            1,
        )
    });
    reseal(&mut c, file);
    expect_problem(
        &c,
        &format!("on {platform}"),
        "case moved off this platform",
    );

    let mut c = base.clone();
    let first = c.inputs.keys().next().unwrap().clone();
    if let Some(Some(b)) = c.inputs.get_mut(&first) {
        b.push(b'\n');
    }
    expect_problem(&c, "inputs/", "tampered input");

    // Every line edit above changed the source text; confirm the probes hit
    // the intended line format, otherwise the controls would be vacuous.
    let first_line = String::from_utf8(base.case_files[file].clone()).unwrap();
    let first_line = first_line.split('\n').next().unwrap();
    for needle in [
        "{\"id\":",
        "\"op\":\"type.validate\"",
        "\"platforms\":[\"linux\",\"macos\",\"windows\"]",
    ] {
        assert!(
            first_line.contains(needle),
            "probe {needle:?} not in {first_line}"
        );
    }

    // A migrated entry whose frozen expectation no longer matches the row.
    let mut c = base.clone();
    c.migrated = Oj::obj(vec![(
        "rows",
        Oj::Arr(vec![Oj::obj(vec![
            ("id", Oj::s("validate.type.array-max-bad")),
            ("basis", Oj::s("D2")),
            ("trace", Oj::s("#29")),
            ("frozen_expect_sha256", Oj::s("0".repeat(64))),
            ("expect", Oj::obj(vec![("ok", Oj::Null)])),
        ])]),
    )]);
    expect_problem(&c, "frozen expectation changed", "stale migration");
    expect_problem(
        &c,
        "but the row's decision",
        "migration without a corpus decision",
    );
}

#[test]
fn negative_controls_reject_wrong_outcomes() {
    let c = load_corpus();
    let (rows, _) = integrity(&c);
    // For every op, one row's expectation must reject a different row's
    // expected outcome, a wrong error class and a wrong error message.
    let mut ops_checked = BTreeSet::new();
    for op in OPS {
        let of_op: Vec<&Row> = rows.iter().filter(|r| r.op == op).collect();
        let ok_rows: Vec<&&Row> = of_op
            .iter()
            .filter(|r| r.expect.get("ok").is_some())
            .collect();
        if let (Some(a), Some(b)) = (
            ok_rows.first(),
            ok_rows
                .iter()
                .find(|b| !structural_eq(&b.expect, &ok_rows[0].expect)),
        ) {
            let other = Ok(b.expect.get("ok").unwrap().clone());
            assert!(
                mismatch(&a.expect, &a.error_match, &other).is_some(),
                "{op}: wrong value accepted"
            );
            let err = Err(RowError::new("RuntimeError", "x"));
            assert!(
                mismatch(&a.expect, &a.error_match, &err).is_some(),
                "{op}: error accepted"
            );
            ops_checked.insert(op);
        }
        if let Some(e) = of_op.iter().find(|r| r.expect.get("error").is_some()) {
            let want = e.expect.get("error").unwrap();
            let class = want.field_str("class").unwrap().to_string();
            let message = want.field_str("message").unwrap().to_string();
            let wrong_class = Err(RowError::new(format!("{class}X"), message.clone()));
            assert!(mismatch(&e.expect, &e.error_match, &wrong_class).is_some());
            if e.error_match == "class_and_message" {
                let wrong_message = Err(RowError::new(class.clone(), format!("{message}.")));
                assert!(mismatch(&e.expect, &e.error_match, &wrong_message).is_some());
            }
            assert!(mismatch(&e.expect, &e.error_match, &Ok(Oj::Null)).is_some());
            assert!(mismatch(
                &e.expect,
                &e.error_match,
                &Err(RowError::new(class, message))
            )
            .is_none());
        }
    }
    assert!(
        ops_checked.len() >= 15,
        "only {ops_checked:?} had two distinct ok rows"
    );

    // Structural comparison is type-strict and order-strict for arrays.
    assert!(!structural_eq(
        &Oj::Num("1".into()),
        &Oj::tagged("$float", Oj::s("1.0"))
    ));
    assert!(!structural_eq(
        &Oj::Arr(vec![Oj::s("a"), Oj::s("b")]),
        &Oj::Arr(vec![Oj::s("b"), Oj::s("a")])
    ));
    assert!(structural_eq(
        &Oj::obj(vec![("a", Oj::Null), ("b", Oj::Bool(true))]),
        &Oj::obj(vec![("b", Oj::Bool(true)), ("a", Oj::Null)])
    ));
    assert!(!structural_eq(&Oj::sym("a"), &Oj::s("a")));
}

#[test]
fn negative_controls_fail_broken_engines() {
    let c = load_corpus();
    let (rows, _) = integrity(&c);
    let ctx = context(&c);
    let none = BTreeMap::new();

    // An engine with no adapter for an op fails every row of it.
    let unknown =
        |_: &str, _: &Oj, _: &Ctx| -> Outcome { Err(RowError::new(UNKNOWN_OP, "no adapter")) };
    let sample: Vec<Row> = OPS
        .iter()
        .filter_map(|op| {
            rows.iter()
                .find(|r| r.op == *op && applies(r, ctx.platform))
        })
        .cloned()
        .collect();
    let report = run_rows(&sample, &none, &ctx, &unknown);
    assert_eq!(report.failures.len(), report.executed.len());
    assert!(!report.executed.is_empty());

    // A formatter swapped for another shell's must fail rows of every
    // shell op (bash rows formatted as cmd, every other shell as bash).
    let escape: Vec<Row> = rows
        .iter()
        .filter(|r| r.op.starts_with("shell.") && applies(r, ctx.platform))
        .cloned()
        .collect();
    let swapped = |op: &str, input: &Oj, _: &Ctx| -> Outcome {
        let name = input.field_str("shell").map_err(RowError::harness)?;
        let wrong = if name == "bash" { "cmd" } else { "bash" };
        ops_shell::run_with(op, ops_shell::shell(wrong)?, input)
    };
    let report = run_rows(&escape, &none, &ctx, &swapped);
    let failed_ops: BTreeSet<&str> = report.failures.iter().map(|f| f.op.as_str()).collect();
    for op in [
        "shell.escape",
        "shell.quote",
        "shell.needs_quoting",
        "shell.join",
        "shell.env_var",
        "shell.format_path",
    ] {
        assert!(
            failed_ops.contains(op),
            "swapped formatter passed every {op} row"
        );
    }
}
