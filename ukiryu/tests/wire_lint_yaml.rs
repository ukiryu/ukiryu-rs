//! Conformance corpus `lint` and `yaml` rows dispatched through the JSON
//! wire (`ukiryu::wire::process`): `lint.string`, `yaml.load`,
//! `yaml.model_load` and `yaml.lutaml_roundtrip`.
//!
//! Each row's input becomes wire requests; the reply is compared with the
//! row's expectation (captured from the pinned Ruby oracle) and never used to
//! produce it. Files named by a row are read from `corpus/conformance/inputs`
//! and passed as document text, or as a path where the original reads the
//! file (`Ukiryu::Models::<model>.from_yaml(path)`).
//!
//! Root ruling R2 rows are judged by the rule files of the core conformance
//! consumer, read unchanged from `tests/conformance/`: a traced migrated
//! expectation (`migrated.json`), the Lutaml declared-field projection
//! (`projection.json`) or the pinned Yeptris emitter bytes
//! (`emitter.json`). Every other row compares with its frozen expectation.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[path = "conformance/sha256.rs"]
mod sha256;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace")
        .join("corpus/conformance")
}

fn inputs() -> PathBuf {
    corpus().join("inputs")
}

fn input_path(rel: &str) -> String {
    inputs().join(rel).to_string_lossy().into_owned()
}

fn read_input(rel: &str) -> String {
    std::fs::read_to_string(inputs().join(rel)).expect("corpus input")
}

/// The oracle's `read_input`: the row's `yaml` text, else its `file`.
fn document(i: &Value) -> String {
    match i["yaml"].as_str() {
        Some(yaml) => yaml.to_owned(),
        None => read_input(i["file"].as_str().expect("file")),
    }
}

fn rows(kind: &str) -> Vec<Value> {
    std::fs::read_to_string(corpus().join(format!("cases/{kind}.jsonl")))
        .expect("cases")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("row"))
        .collect()
}

fn host() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn wire(request: &Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

/// The corpus host substitution: the absolute `inputs/` path is `${INPUTS}`.
fn scrub(value: &Value) -> Value {
    let base = inputs().to_string_lossy().into_owned();
    match value {
        Value::String(s) => Value::String(s.replace(&base, "${INPUTS}")),
        Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), scrub(v))).collect())
        }
        other => other.clone(),
    }
}

/// A row's result through the wire: a value, or the original's error with
/// the reply's structured `detail` when it carries one.
#[derive(Debug)]
enum Outcome {
    Ok(Value),
    Err {
        class: String,
        message: Value,
        detail: Option<Value>,
    },
}

fn outcome(reply: &Value) -> Outcome {
    if reply["ok"] == true {
        return Outcome::Ok(scrub(&reply["value"]));
    }
    let error = &reply["error"];
    match error.get("source_class").and_then(Value::as_str) {
        Some(class) => Outcome::Err {
            class: class.to_owned(),
            message: scrub(&error["message"]),
            detail: error.get("detail").cloned(),
        },
        None => Outcome::Err {
            class: format!("wire {}", error["code"]),
            message: error["message"].clone(),
            detail: None,
        },
    }
}

fn to_yaml(model: &Value, yaml: &str) -> Value {
    wire(&json!({"op": "yaml", "method": "model_to_yaml", "model": model, "yaml": yaml}))
}

/// The row's outcome through the wire.
fn run(row: &Value) -> Outcome {
    let i = &row["input"];
    match row["op"].as_str().expect("op") {
        "lint.string" => outcome(&wire(&json!({
            "op": "lint", "method": "string", "yaml": document(i),
        }))),
        "yaml.load" => outcome(&wire(&json!({
            "op": "yaml", "method": "safe_load", "yaml": document(i),
        }))),
        "yaml.model_load" => outcome(&wire(&json!({
            "op": "yaml", "method": "model_load", "model": i["model"],
            "path": input_path(i["file"].as_str().expect("file")),
        }))),
        // `first = from_yaml(text).to_yaml; from_yaml(first).to_yaml == first`.
        "yaml.lutaml_roundtrip" => {
            let first = to_yaml(&i["model"], &document(i));
            let Some(emitted) = first["value"].as_str() else {
                return outcome(&first);
            };
            let again = to_yaml(&i["model"], emitted);
            Outcome::Ok(json!({
                "emitted": emitted,
                "reparsed_equal": again["value"].as_str() == Some(emitted),
            }))
        }
        op => panic!("{}: undispatched op {op}", row["id"]),
    }
}

/// Rows each op must dispatch; a change to the corpus changes these.
const PINNED: [(&str, usize); 4] = [
    ("lint.string", 70),
    ("yaml.load", 81),
    ("yaml.model_load", 62),
    ("yaml.lutaml_roundtrip", 2),
];

#[test]
fn corpus_rows_match_through_the_wire() {
    let rules = Rules::load();
    let mut dispatched = BTreeMap::<String, usize>::new();
    let mut failures = Vec::new();
    for kind in ["lint", "yaml"] {
        for row in rows(kind) {
            let id = row["id"].as_str().expect("id");
            let op = row["op"].as_str().expect("op");
            let platforms = row["applicability"]["platforms"]
                .as_array()
                .expect("platforms");
            assert!(
                platforms.iter().any(|p| p == host()),
                "{id}: does not apply on {}",
                host()
            );
            *dispatched.entry(op.to_owned()).or_default() += 1;
            if let Some(reason) = rules.judge(&row, &run(&row)) {
                failures.push(format!("{id}: {reason}"));
            }
        }
    }
    for (op, count) in PINNED {
        assert_eq!(
            dispatched.get(op).copied(),
            Some(count),
            "{op} rows dispatched"
        );
    }
    let total: usize = dispatched.values().sum();
    assert!(
        failures.is_empty(),
        "{} of {total} rows differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The replies keep Ruby types the JSON profile of `parse_yaml` refuses.
#[test]
fn safe_load_keeps_ruby_types() {
    let reply = wire(&json!({
        "op": "yaml", "method": "safe_load",
        "yaml": ":sym: 1.5\n2: [.nan, ~]\nplain: text\n",
    }));
    assert_eq!(
        reply,
        json!({"ok": true, "value": {"$map": [
            [{"$sym": "sym"}, {"$float": "1.5"}],
            [2, [{"$float": "NaN"}, null]],
            ["plain", "text"],
        ]}}),
    );
    let reply = wire(&json!({"op": "yaml", "method": "safe_load", "yaml": "a: [1"}));
    assert_eq!(
        reply["error"]["source_class"], "Psych::SyntaxError",
        "{reply}"
    );
    assert_eq!(
        reply["error"]["detail"],
        json!({"category": "YamlSyntax", "stage": "document"}),
        "{reply}"
    );
}

/// Control: an altered input changes the reply, so a match is not an echo.
#[test]
fn altered_inputs_change_the_reply() {
    let lint = |yaml: &str| wire(&json!({"op": "lint", "method": "string", "yaml": yaml}));
    let row = rows("lint")
        .into_iter()
        .find(|r| r["input"]["yaml"].is_string())
        .expect("inline lint row");
    // The description clears one of the row's issues, so the reply differs.
    let altered = lint("name: other\nversion: '1.0'\ndescription: altered\n");
    assert_eq!(altered["ok"], true, "{altered}");
    let original = lint(row["input"]["yaml"].as_str().expect("yaml"));
    assert_ne!(altered["value"], original["value"]);
    assert!(Rules::frozen().judge(&row, &outcome(&altered)).is_some());
}

#[test]
fn unknown_models_are_invalid_arguments() {
    let reply = wire(&json!({
        "op": "yaml", "method": "model_to_yaml", "model": "Nope", "yaml": "a: 1",
    }));
    assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT", "{reply}");
}

// ------------------------------------------------------------ R2 rules

/// The core conformance consumer's rule files.
fn rules_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance")
}

fn rule_file(name: &str) -> Value {
    let text = std::fs::read_to_string(rules_dir().join(name)).expect("rule file");
    serde_json::from_str(&text).expect("rule json")
}

/// Key-sorted compact JSON, the form `frozen_expect_sha256` digests.
fn canonical(v: &Value) -> String {
    match v {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => {
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            let body: Vec<String> = pairs
                .into_iter()
                .map(|(k, x)| format!("{}:{}", Value::String(k.clone()), canonical(x)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        other => other.to_string(),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

struct Migration {
    expect: Value,
    detail: Option<Value>,
}

struct Projection {
    class: String,
    declared: Vec<String>,
    private: Vec<String>,
}

/// How each row is judged: by its frozen expectation, or by the one R2
/// rule that lists it.
struct Rules {
    migrated: BTreeMap<String, Migration>,
    projections: Vec<Projection>,
    projected: BTreeSet<String>,
    emitter: BTreeMap<String, PathBuf>,
}

impl Rules {
    fn load() -> Rules {
        let migrated = rule_file("migrated.json")["rows"]
            .as_array()
            .expect("migrated rows")
            .iter()
            .map(|m| {
                (
                    m["id"].as_str().expect("id").to_owned(),
                    Migration {
                        expect: m["expect"].clone(),
                        detail: m.get("detail").cloned(),
                    },
                )
            })
            .collect();
        let projection = rule_file("projection.json");
        let projections = projection["classes"]
            .as_array()
            .expect("classes")
            .iter()
            .map(|c| Projection {
                class: c["class"].as_str().unwrap_or("").to_owned(),
                declared: strings(&c["declared"]),
                private: strings(&c["framework_private"]),
            })
            .collect();
        let emitter = rule_file("emitter.json")["rows"]
            .as_array()
            .expect("emitter rows")
            .iter()
            .map(|r| {
                (
                    r["id"].as_str().expect("id").to_owned(),
                    rules_dir().join(r["golden"].as_str().expect("golden")),
                )
            })
            .collect();
        Rules {
            migrated,
            projections,
            projected: strings(&projection["rows"]).into_iter().collect(),
            emitter,
        }
    }

    /// Frozen expectations only.
    fn frozen() -> Rules {
        Rules {
            migrated: BTreeMap::new(),
            projections: Vec::new(),
            projected: BTreeSet::new(),
            emitter: BTreeMap::new(),
        }
    }

    /// The ids each rule lists (an id under two rules appears twice).
    fn listed(&self) -> Vec<&String> {
        self.migrated
            .keys()
            .chain(&self.projected)
            .chain(self.emitter.keys())
            .collect()
    }

    /// None when the outcome satisfies the row, else the reason.
    fn judge(&self, row: &Value, outcome: &Outcome) -> Option<String> {
        let id = row["id"].as_str().expect("id");
        let error_match = row["error_match"].as_str().unwrap_or("");
        if let Some(m) = self.migrated.get(id) {
            return mismatch(&m.expect, error_match, m.detail.as_ref(), outcome);
        }
        if self.projected.contains(id) {
            return match project(&row["expect"], &self.projections) {
                Ok(e) => mismatch(&e, error_match, None, outcome),
                Err(e) => Some(format!("projection: {e}")),
            };
        }
        if let Some(golden) = self.emitter.get(id) {
            let golden = std::fs::read(golden).ok();
            return emitter_mismatch(&row["expect"], golden.as_deref(), outcome);
        }
        mismatch(&row["expect"], error_match, None, outcome)
    }
}

fn mismatch(
    expect: &Value,
    error_match: &str,
    detail: Option<&Value>,
    outcome: &Outcome,
) -> Option<String> {
    if let Some(want) = expect.get("ok") {
        return match outcome {
            Outcome::Ok(got) if got == want => None,
            Outcome::Ok(got) => Some(format!("value differs: expected {want} got {got}")),
            Outcome::Err { class, message, .. } => {
                Some(format!("expected ok, got error {class}: {message}"))
            }
        };
    }
    let want = expect.get("error")?;
    let (class, message, got_detail) = match outcome {
        Outcome::Ok(got) => return Some(format!("expected error, got ok {got}")),
        Outcome::Err {
            class,
            message,
            detail,
        } => (class, message, detail),
    };
    if want["class"] != class.as_str() {
        return Some(format!(
            "error class differs: expected {} got {class} ({message})",
            want["class"]
        ));
    }
    if error_match == "class_and_message" && &want["message"] != message {
        return Some(format!(
            "error message differs: expected {} got {message}",
            want["message"]
        ));
    }
    match (detail, got_detail) {
        (None, _) => None,
        (Some(want), Some(got)) if want == got => None,
        (Some(want), got) => Some(format!(
            "error detail differs: expected {want} got {}",
            got.as_ref().map_or("none".into(), Value::to_string)
        )),
    }
}

/// The frozen snapshot without framework-private ivars, on objects of a
/// projected class whose ivars are exactly declared + framework-private.
fn project(v: &Value, projections: &[Projection]) -> Result<Value, String> {
    match v {
        Value::Array(items) => items
            .iter()
            .map(|x| project(x, projections))
            .collect::<Result<_, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            let class = map.get("class").and_then(Value::as_str);
            let spec = class.and_then(|c| projections.iter().find(|p| p.class == c));
            let mut out = serde_json::Map::new();
            for (k, x) in map {
                let x = project(x, projections)?;
                match (spec, k.as_str(), &x) {
                    (Some(p), "ivars", Value::Object(ivars)) => {
                        let have: BTreeSet<&str> = ivars.keys().map(String::as_str).collect();
                        let want: BTreeSet<&str> = p
                            .declared
                            .iter()
                            .chain(&p.private)
                            .map(String::as_str)
                            .collect();
                        if have != want {
                            return Err(format!(
                                "{} ivars {have:?} are not exactly declared + framework-private {want:?}",
                                p.class
                            ));
                        }
                        let kept = ivars
                            .iter()
                            .filter(|(n, _)| p.declared.contains(n))
                            .map(|(n, x)| (n.clone(), x.clone()))
                            .collect();
                        out.insert(k.clone(), Value::Object(kept));
                    }
                    _ => {
                        out.insert(k.clone(), x);
                    }
                }
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

/// Type-sensitive canonical form of YAML text through the wire reader.
fn canonical_yaml(text: &str) -> Result<Value, String> {
    let reply = wire(&json!({"op": "yaml", "method": "safe_load", "yaml": text}));
    if reply["ok"] == true {
        Ok(reply["value"].clone())
    } else {
        Err(reply["error"]["message"].to_string())
    }
}

/// `{emitted, reparsed_equal}`: `reparsed_equal` as frozen, `emitted`
/// canonically equal to the frozen Psych text and byte-equal to the pinned
/// Yeptris golden.
fn emitter_mismatch(expect: &Value, golden: Option<&[u8]>, outcome: &Outcome) -> Option<String> {
    let Some(frozen) = expect.get("ok") else {
        return Some("emitter rule needs an ok expectation".into());
    };
    let got = match outcome {
        Outcome::Ok(v) => v,
        Outcome::Err { class, message, .. } => {
            return Some(format!("expected ok, got error {class}: {message}"))
        }
    };
    let keys = |v: &Value| -> BTreeSet<String> {
        v.as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };
    if keys(got) != keys(frozen) {
        return Some(format!("emitter outcome keys differ: {got}"));
    }
    if got.get("reparsed_equal") != frozen.get("reparsed_equal") {
        return Some("reparsed_equal differs from the frozen value".into());
    }
    let (Some(emitted), Some(psych)) = (got["emitted"].as_str(), frozen["emitted"].as_str()) else {
        return Some("emitted must be a string".into());
    };
    match (canonical_yaml(emitted), canonical_yaml(psych)) {
        (Ok(a), Ok(b)) if a == b => {}
        (Ok(a), Ok(b)) => {
            return Some(format!(
                "emitted YAML is not canonically equal to the frozen Psych text: got {a} want {b}"
            ))
        }
        (a, b) => {
            return Some(format!(
                "canonical load failed: emitted {:?} frozen {:?}",
                a.err(),
                b.err()
            ))
        }
    }
    match golden {
        None => Some(format!(
            "no pinned Yeptris golden; emitted bytes: {}",
            Value::String(emitted.to_owned())
        )),
        Some(g) if g == emitted.as_bytes() => None,
        Some(_) => Some(format!(
            "emitted bytes differ from the pinned Yeptris golden; got {}",
            Value::String(emitted.to_owned())
        )),
    }
}

/// The R2 rules this test applies are traced, pinned and scoped: each
/// lists an existing `lint`/`yaml` row of the right op, under one rule
/// only, and no rule beyond the 27 rows the rule files list for these
/// kinds. Migrated entries for other kinds are not this test's rows.
#[test]
fn r2_rules_are_traced_and_scoped() {
    let rules = Rules::load();
    let corpus: BTreeMap<String, Value> = ["lint", "yaml"]
        .into_iter()
        .flat_map(rows)
        .map(|r| (r["id"].as_str().expect("id").to_owned(), r))
        .collect();
    let mut problems = Vec::new();

    let mut seen = BTreeSet::new();
    let mut applied = 0;
    for id in rules.listed() {
        if !corpus.contains_key(id) {
            if id.starts_with("lint.") || id.starts_with("yaml.") {
                problems.push(format!("{id}: no such corpus row"));
            }
            continue;
        }
        applied += 1;
        if !seen.insert(id) {
            problems.push(format!("{id}: listed by more than one rule"));
        }
    }
    assert_eq!(applied, 27, "R2 rules over lint and yaml rows");

    let bases = ["D1", "D2", "version", "emitter", "yaml-reader"];
    for m in rule_file("migrated.json")["rows"].as_array().expect("rows") {
        let id = m["id"].as_str().unwrap_or("<no id>");
        let Some(row) = corpus.get(id) else { continue };
        let basis = m["basis"].as_str().unwrap_or("");
        if !bases.contains(&basis) {
            problems.push(format!("migrated {id}: unknown basis {basis:?}"));
        }
        if (basis == "D1" || basis == "D2") && row["decision"].as_str() != Some(basis) {
            problems.push(format!(
                "migrated {id}: basis {basis} not the row's decision"
            ));
        }
        if !m["trace"].as_str().unwrap_or("").contains('#') {
            problems.push(format!(
                "migrated {id}: trace must cite the governing issue"
            ));
        }
        let frozen = sha256::hex(canonical(&row["expect"]).as_bytes());
        if m["frozen_expect_sha256"].as_str() != Some(frozen.as_str()) {
            problems.push(format!(
                "migrated {id}: frozen expectation changed (now sha256 {frozen})"
            ));
        }
        match m["expect"].as_object() {
            Some(e) if e.len() == 1 && (e.contains_key("ok") || e.contains_key("error")) => {}
            _ => problems.push(format!("migrated {id}: expect must be {{ok}} or {{error}}")),
        }
        if m["expect"].get("error").is_some() {
            let required: &[&str] = match basis {
                "D1" | "D2" => &["code", "variant", "stage"],
                "yaml-reader" => &["category", "stage"],
                _ => &[],
            };
            for key in required {
                if !m["detail"][key].is_string() {
                    problems.push(format!("migrated {id}: error must freeze detail.{key}"));
                }
            }
        }
        if m["expect"] == row["expect"] {
            problems.push(format!("migrated {id}: equals the frozen expectation"));
        }
    }

    let projection = rule_file("projection.json");
    for key in ["evidence", "trace"] {
        if projection[key].as_str().is_none_or(str::is_empty) {
            problems.push(format!("projection.json: missing {key}"));
        }
    }
    for p in &rules.projections {
        if p.class.is_empty() || p.declared.is_empty() || p.private.is_empty() {
            problems.push(format!("projection {}: incomplete class entry", p.class));
        }
        if p.private.iter().any(|x| p.declared.contains(x)) {
            problems.push(format!(
                "projection {}: a declared field is private",
                p.class
            ));
        }
    }
    for id in &rules.projected {
        let Some(row) = corpus.get(id) else { continue };
        if row["op"] != "yaml.model_load" {
            problems.push(format!("projection {id}: op is not yaml.model_load"));
        }
        match project(&row["expect"], &rules.projections) {
            Err(e) => problems.push(format!("projection {id}: {e}")),
            Ok(p) if p == row["expect"] => {
                problems.push(format!("projection {id}: removes nothing"))
            }
            Ok(_) => {}
        }
    }

    for r in rule_file("emitter.json")["rows"].as_array().expect("rows") {
        let id = r["id"].as_str().unwrap_or("<no id>");
        match corpus.get(id) {
            Some(row) if row["op"] == "yaml.lutaml_roundtrip" => {}
            _ => problems.push(format!("emitter {id}: not a yaml.lutaml_roundtrip row")),
        }
        if !r["trace"].as_str().unwrap_or("").contains('#') {
            problems.push(format!("emitter {id}: trace must cite the governing issue"));
        }
        let golden = rules_dir().join(r["golden"].as_str().unwrap_or(""));
        let got = std::fs::read(golden).map(|b| sha256::hex(&b));
        if got.as_deref().ok() != r["golden_sha256"].as_str() {
            problems.push(format!(
                "emitter {id}: golden sha256 {got:?} differs from the pin"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
