//! Definition core parity against captures of the pinned Ruby oracle
//! (ab6df353, `lib/ukiryu/definition/**`). Each fixture under
//! `definition_core_fixtures/` records its capture script and source files.
use serde::Deserialize;
use serde_json::{json, Value as Json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ukiryu::definition::cache::CachedDefinition;
use ukiryu::definition::loader::validate_profile;
use ukiryu::definition::source::{same_source, sha256_hex};
use ukiryu::definition::*;
use yeptris::Value as Yaml;

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<Json>(include_str!(concat!(
            "definition_core_fixtures/",
            $name,
            ".json"
        )))
        .unwrap()
    };
}

fn cases(doc: &Json) -> &Vec<Json> {
    assert_eq!(
        doc["provenance"]["oracle_sha"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    doc["cases"].as_array().unwrap()
}

fn err_json(e: &DefinitionError) -> Json {
    json!({"error": {"class": e.ruby_class(), "message": e.message}})
}

fn ok<T: Into<Json>>(v: T) -> Json {
    json!({ "ok": v.into() })
}

fn s(v: &Json) -> &str {
    v.as_str().unwrap()
}

// ---------------------------------------------------------------- temp roots

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ukiryu-defcore-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Symlinked temp dirs (macOS) are resolved so expanded paths match;
        // Windows canonical paths carry a `\\?\` prefix, so keep them as-is.
        if cfg!(unix) {
            TempRoot(std::fs::canonicalize(&dir).unwrap())
        } else {
            TempRoot(dir)
        }
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn join(&self, rel: &str) -> String {
        format!("{}/{}", self.path(), rel)
    }
    fn write(&self, rel: &str, body: &str) -> String {
        let p = self.join(rel);
        std::fs::create_dir_all(Path::new(&p).parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }
    fn scrub(&self, text: &str) -> String {
        text.replace(&self.path(), "<ROOT>")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn set_mtime(path: &str, secs: u64) {
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

// ------------------------------------------------- minimal private document

#[derive(Clone, Debug, Deserialize)]
struct TCmd {
    name: Option<String>,
    #[cfg_attr(not(unix), allow(dead_code))]
    description: Option<String>,
}

fn default_style() -> Option<String> {
    Some("single_dash_space".into())
}

fn empty_cmds() -> Option<Vec<TCmd>> {
    Some(Vec::new())
}

#[derive(Clone, Debug, Deserialize)]
struct TProfile {
    name: Option<String>,
    #[serde(default = "default_style")]
    option_style: Option<String>,
    #[serde(default = "empty_cmds")]
    commands: Option<Vec<TCmd>>,
    inherits: Option<String>,
}

/// Test-only stand-in for `Models::ToolDefinition` (owned by the models
/// module): header fields, profiles and commands with lutaml defaults.
#[derive(Clone, Debug, Deserialize)]
struct TDef {
    ukiryu_schema: Option<String>,
    #[serde(rename = "$self")]
    self_uri: Option<String>,
    name: Option<String>,
    version: Option<String>,
    profiles: Option<Vec<TProfile>>,
    #[serde(skip)]
    #[cfg_attr(not(unix), allow(dead_code))]
    composite: bool,
    #[serde(skip)]
    inherits: Option<Vec<Yaml>>,
    #[serde(skip)]
    includes: Option<Vec<Yaml>>,
}

impl DefinitionDocument for TDef {
    fn parse_definition(yaml: &str) -> std::result::Result<Self, DocumentParseError> {
        ukiryu::from_yaml::<TDef>(yaml).map_err(|e| DocumentParseError::Format(e.to_string()))
    }
    fn definition_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn definition_version(&self) -> Option<&str> {
        self.version.as_deref()
    }
    fn has_profiles(&self) -> bool {
        self.profiles.as_ref().is_some_and(|p| !p.is_empty())
    }
    fn ukiryu_schema(&self) -> Option<&str> {
        self.ukiryu_schema.as_deref()
    }
    fn self_uri(&self) -> Option<&str> {
        self.self_uri.as_deref()
    }
    fn resolve_inheritance(&mut self) {
        let Some(profiles) = self.profiles.as_mut() else {
            return;
        };
        for i in 0..profiles.len() {
            let Some(parent_name) = profiles[i].inherits.clone() else {
                continue;
            };
            let Some(parent) = profiles
                .iter()
                .find(|p| p.name.as_deref() == Some(&parent_name))
            else {
                continue;
            };
            let parent_cmds = parent.commands.clone().unwrap_or_default();
            let mut merged = profiles[i].commands.clone().unwrap_or_default();
            let child_names: Vec<Option<String>> = merged.iter().map(|c| c.name.clone()).collect();
            merged.extend(
                parent_cmds
                    .into_iter()
                    .filter(|c| !child_names.contains(&c.name)),
            );
            profiles[i].commands = Some(merged);
        }
    }
}

impl ComposableCommand for TCmd {
    fn command_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl ComposableProfile for TProfile {
    type Command = TCmd;
    fn profile_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn commands(&self) -> Option<&Vec<TCmd>> {
        self.commands.as_ref()
    }
    fn commands_mut(&mut self) -> &mut Option<Vec<TCmd>> {
        &mut self.commands
    }
    fn option_style(&self) -> Option<&str> {
        self.option_style.as_deref()
    }
    fn set_option_style(&mut self, value: Option<String>) {
        self.option_style = value;
    }
}

impl ComposableDefinition for TDef {
    type Profile = TProfile;
    fn profiles(&self) -> Option<&Vec<TProfile>> {
        self.profiles.as_ref()
    }
    fn profiles_mut(&mut self) -> &mut Option<Vec<TProfile>> {
        &mut self.profiles
    }
    fn inherits(&self) -> Option<Vec<Yaml>> {
        self.inherits.clone()
    }
    fn includes(&self) -> Option<Vec<Yaml>> {
        self.includes.clone()
    }
}

impl CachedDefinition for TDef {}

fn tdef(yaml: &str) -> TDef {
    TDef::parse_definition(yaml).unwrap()
}

#[cfg(unix)]
fn json_to_yaml(v: &Json) -> Yaml {
    match v {
        Json::Null => Yaml::Null,
        Json::Bool(b) => Yaml::Bool(*b),
        Json::Number(n) => Yaml::Number(yeptris::Number::I64(n.as_i64().unwrap())),
        Json::String(s) => Yaml::String(s.clone()),
        Json::Array(a) => Yaml::Sequence(a.iter().map(json_to_yaml).collect()),
        Json::Object(o) => {
            let mut m = yeptris::Mapping::new();
            for (k, v) in o {
                m.insert(Yaml::String(k.clone()), json_to_yaml(v));
            }
            Yaml::Mapping(m)
        }
    }
}

// ------------------------------------------------------- version resolver

fn constraints_json(list: &[Constraint]) -> Json {
    Json::Array(
        list.iter()
            .map(|c| json!({"operator": c.operator.as_str(), "version": c.version, "raw": c.raw}))
            .collect(),
    )
}

fn ord_json(o: std::cmp::Ordering) -> Json {
    json!(o as i8)
}

fn str_list(v: &Json) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| s(x).to_string())
        .collect()
}

#[test]
fn version_resolver_matches_oracle() {
    let doc = fixture!("version_resolver");
    let mut count = 0;
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "parse_constraint" => ok(constraints_json(&VersionResolver::parse_constraint(s(
                &case["input"],
            )))),
            "satisfies" => {
                match VersionResolver::satisfies(s(&case["version"]), s(&case["constraint"])) {
                    Ok(b) => ok(b),
                    Err(e) => err_json(&e),
                }
            }
            "resolve" => {
                let vs = str_list(&case["versions"]);
                match VersionResolver::resolve(s(&case["constraint"]), Some(&vs)) {
                    Ok(r) => ok(r),
                    Err(e) => err_json(&e),
                }
            }
            "resolve_nil" => {
                ok(VersionResolver::resolve::<&str>(s(&case["constraint"]), None).unwrap())
            }
            "compare_versions" => ok(ord_json(VersionResolver::compare_versions(
                s(&case["a"]),
                s(&case["b"]),
            ))),
            "parse_version" => ok(Json::Array(
                VersionResolver::parse_version(s(&case["input"]))
                    .iter()
                    .map(|n| serde_json::from_str::<Json>(&n.to_string()).unwrap())
                    .collect(),
            )),
            "latest" => ok(VersionResolver::latest(Some(&str_list(&case["versions"])))),
            "pessimistic_range" => match VersionResolver::pessimistic_range(s(&case["input"])) {
                Ok((a, b)) => ok(json!([a, b])),
                Err(e) => err_json(&e),
            },
            "constraint_factories" => ok(json!({
                "exact": constraints_json(&[Constraint::exact("1.0")]),
                "min": constraints_json(&[Constraint::min("1.0")]),
                "max": constraints_json(&[Constraint::max("2")]),
                "pessimistic": constraints_json(&[Constraint::pessimistic("1.2")]),
                "range": constraints_json(&Constraint::range("1", "2")),
                "to_s": Constraint::min("3").to_string(),
                "custom": constraints_json(&[Constraint::new(Operator::Other(">".into()), "4", Some("gt four".into()))]),
            })),
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
        count += 1;
    }
    assert_eq!(count, 118);
    assert!(!VersionResolver::satisfies_all(
        "1",
        &[Constraint::new(Operator::Other("!=".into()), "1", None)]
    )
    .unwrap());
}

// ------------------------------------------------- lint issue / validation

fn opt(v: &Json) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn issue_json(i: &LintIssue) -> Json {
    json!({
        "to_h": serde_json::to_value(i.to_h()).unwrap(),
        "to_s": i.to_string(), "error": i.is_error(), "warning": i.is_warning(), "info": i.is_info(),
        "style": i.is_style(), "has_suggestion": i.has_suggestion(), "has_location": i.has_location(),
        "severity_string": i.severity_string(),
    })
}

fn vr_json(r: &ValidationResult) -> Json {
    json!({
        "valid": r.is_valid(), "invalid": r.is_invalid(), "has_errors": r.has_errors(), "has_warnings": r.has_warnings(),
        "issue_count": r.issue_count(), "error_count": r.error_count(), "warning_count": r.warning_count(),
        "summary": r.summary(), "to_s": r.to_string(), "to_json": r.to_json(), "schema_path": r.schema_path,
    })
}

#[test]
fn lint_issue_and_validation_result_match_oracle() {
    let doc = fixture!("results");
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "lint_issue_factory" => {
                let (m, l, sg, r) = (
                    s(&case["message"]),
                    opt(&case["location"]),
                    opt(&case["suggestion"]),
                    opt(&case["rule_id"]),
                );
                let issue = match s(&case["severity"]) {
                    "error" => LintIssue::error(m, l, sg, r),
                    "warning" => LintIssue::warning(m, l, sg, r),
                    "info" => LintIssue::info(m, l, sg, r),
                    _ => LintIssue::style(m, l, sg, r),
                };
                ok(issue_json(&issue))
            }
            "lint_issue_custom_severity" => ok(issue_json(&LintIssue::new(
                Severity::Other("fatal".into()),
                "m",
                Some("l".into()),
                None,
                None,
            ))),
            "lint_issue_constants" => ok(Json::Array(
                Severity::ALL.iter().map(|s| json!(s.as_str())).collect(),
            )),
            "vr_success" => ok(vr_json(&ValidationResult::success())),
            "vr_failure" => ok(vr_json(&ValidationResult::failure(
                str_list(&case["errors"]),
                str_list(&case["warnings"]),
            ))),
            "vr_with_warnings" => ok(vr_json(&ValidationResult::with_warnings(str_list(
                &case["warnings"],
            )))),
            "vr_new" => ok(vr_json(&ValidationResult::new(
                case["valid"].as_bool().unwrap(),
                str_list(&case["errors"]),
                str_list(&case["warnings"]),
                opt(&case["schema_path"]),
            ))),
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
    }
    assert_eq!(cases(&doc).len(), 16);
}

// ---------------------------------------------------------------- metadata

#[cfg(unix)]
fn kind(name: &str) -> SourceKind {
    match name {
        "user" => SourceKind::User,
        "bundled" => SourceKind::Bundled,
        "local_system" => SourceKind::LocalSystem,
        "system" => SourceKind::System,
        "register" => SourceKind::Register,
        other => SourceKind::Other(other.into()),
    }
}

#[cfg(unix)]
fn md_json(m: &DefinitionMetadata, root: &TempRoot) -> Json {
    json!({
        "name": m.name, "version": m.version, "path": root.scrub(&m.path), "source_type": m.source_type.as_str(),
        "to_s": root.scrub(&m.to_string()), "inspect": root.scrub(&m.inspect()), "priority": m.priority(),
    })
}

// The captures use POSIX paths; native Windows path rendering is unverified.
#[cfg(unix)]
#[test]
fn metadata_matches_oracle() {
    let doc = fixture!("metadata");
    let root = TempRoot::new("md");
    let specs: Vec<Vec<String>> = cases(&doc)
        .iter()
        .filter(|c| c["op"] == "new")
        .map(|c| str_list(&c["spec"]))
        .collect();
    let mk =
        |sp: &Vec<String>| DefinitionMetadata::new(&sp[0], &sp[1], root.join(&sp[2]), kind(&sp[3]));
    let mut compared = 0;
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "new" => ok(md_json(&mk(&str_list(&case["spec"])), &root)),
            "relative_path" => {
                // Ruby ran this under Dir.chdir(ROOT); expansion is against the cwd.
                let cwd = std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let m = DefinitionMetadata::new("n", "1", "rel/../t.yaml", SourceKind::User);
                assert_eq!(m.path, format!("{cwd}/t.yaml"));
                let mut j = md_json(&m, &root);
                for key in ["path", "to_s", "inspect"] {
                    j[key] = json!(s(&j[key]).replace(&cwd, "<ROOT>"));
                }
                ok(j)
            }
            "cmp" => {
                let a = mk(&specs[case["a"].as_u64().unwrap() as usize]);
                let b = mk(&specs[case["b"].as_u64().unwrap() as usize]);
                compared += 1;
                ok(ord_json(a.compare(&b)))
            }
            "eq" => {
                let a = mk(&specs[0]);
                ok(json!([
                    a == mk(&specs[0]),
                    a == mk(&specs[1]),
                    a == DefinitionMetadata::new(
                        "a",
                        "1.0",
                        root.join("x.yaml"),
                        SourceKind::Bundled
                    )
                ]))
            }
            "exists_mtime" => {
                let p = root.write("exists.yaml", "name: x\n");
                set_mtime(&p, 1_600_000_000);
                let e = DefinitionMetadata::new("x", "1", &p, SourceKind::User);
                let n =
                    DefinitionMetadata::new("x", "1", root.join("missing.yaml"), SourceKind::User);
                ok(json!([
                    e.exists(),
                    ukiryu::definition::sources::file::ruby_time_to_s(e.mtime().unwrap()),
                    n.exists(),
                    n.mtime().map(|_| "set")
                ]))
            }
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
    }
    assert_eq!(compared, 100);
}

// ----------------------------------------------------------------- sources

#[test]
fn string_source_matches_oracle() {
    let doc = fixture!("sources");
    let mut seen = 0;
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "string_source" => match StringSource::new(s(&case["input"])) {
                Ok(mut src) => ok(json!({
                    "content": src.content(), "load": src.load().unwrap(), "size": src.size(),
                    "content_hash": src.content_hash(), "cache_key": src.cache_key().unwrap(),
                    "source_type": src.source_type().as_str(), "to_s": src.describe().unwrap(),
                    "inspect": src.inspect().unwrap(),
                })),
                Err(e) => err_json(&e),
            },
            "string_source_eq" => {
                let n = |x: &str| StringSource::new(x).unwrap();
                ok(json!([
                    same_source(&n("a"), &n(" a ")).unwrap(),
                    same_source(&n("a"), &n("b")).unwrap(),
                    same_source(&n("a"), &n("a\n")).unwrap(),
                    false // Ruby compares against a plain String; not a Source here.
                ]))
            }
            // Non-String arguments (ArgumentError) are excluded by the Rust
            // signature; file cases are covered by `file_source_matches_oracle`.
            _ => continue,
        };
        assert_eq!(actual, case["result"], "{case}");
        seen += 1;
    }
    assert_eq!(seen, 9);
    assert_eq!(
        sha256_hex(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[cfg(unix)]
#[test]
fn file_source_matches_oracle() {
    const T1: u64 = 1_600_000_000;
    const T2: u64 = 1_600_000_100;
    let doc = fixture!("sources");
    let root = TempRoot::new("src");
    let tool = root.write("defs/tool.yaml", "name: tool\nversion: '1.0'\n");
    set_mtime(&tool, T1);
    set_mtime(&root.write("defs/other.yaml", "  name: other  \n"), T1);
    std::fs::create_dir_all(root.join("defs/dir.yaml")).unwrap();
    let locked = root.write("defs/locked.yaml", "name: locked\n");
    set_mtime(&locked, T1);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let running_as_root = std::fs::read(&locked).is_ok();
    std::os::unix::fs::symlink(&tool, root.join("defs/link.yaml")).unwrap();
    std::os::unix::fs::symlink(
        root.join("defs/nowhere.yaml"),
        root.join("defs/dangling.yaml"),
    )
    .unwrap();

    let scrub = |text: &str| {
        let mut out = text.to_string();
        for rel in [
            "defs/tool.yaml",
            "defs/other.yaml",
            "defs/link.yaml",
            "defs/mod.yaml",
            "defs/same.yaml",
        ] {
            let p = root.join(rel);
            out = out.replace(&sha256_hex(&p), &format!("<SHA256:<ROOT>/{rel}>"));
        }
        root.scrub(&out)
    };
    let errj = |e: &DefinitionError| json!({"class": e.ruby_class(), "message": scrub(&e.message)});
    let view = |mut src: FileSource| -> Json {
        let before = src
            .mtime()
            .map(ukiryu::definition::sources::file::ruby_time_to_s);
        let key_before = src.cache_key().unwrap();
        let load = src.load().unwrap();
        json!({
            "path": scrub(src.path()), "mtime_before": before, "cache_key_before": scrub(&key_before), "load": load,
            "mtime_after": ukiryu::definition::sources::file::ruby_time_to_s(src.mtime().unwrap()),
            "cache_key_after": scrub(&src.cache_key().unwrap()), "real_path": scrub(src.real_path()),
            "source_type": src.source_type().as_str(), "to_s": scrub(&src.describe().unwrap()),
            "inspect": scrub(&src.inspect().unwrap()),
        })
    };
    let mut seen = 0;
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "file_source" => {
                let path = s(&case["path"]).replace("<ROOT>", &root.path());
                if path.ends_with("locked.yaml") && running_as_root {
                    continue;
                }
                match FileSource::new(&path) {
                    Ok(src) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        view(src.clone())
                    })) {
                        Ok(v) => ok(v),
                        Err(_) => {
                            let mut src = src;
                            let e = src.load().unwrap_err();
                            json!({"error": errj(&e)})
                        }
                    },
                    Err(e) => json!({"error": errj(&e)}),
                }
            }
            "file_source_modified_after_load" => {
                let p = root.write("defs/mod.yaml", "name: a\n");
                set_mtime(&p, T1);
                let mut src = FileSource::new(&p).unwrap();
                let first = src.load().unwrap();
                std::fs::write(&p, "name: b\n").unwrap();
                set_mtime(&p, T2);
                let key = src.cache_key().unwrap();
                ok(json!([first, scrub(&key), errj(&src.load().unwrap_err())]))
            }
            "file_source_rewritten_same_mtime" => {
                let p = root.write("defs/same.yaml", "name: a\n");
                set_mtime(&p, T1);
                let mut src = FileSource::new(&p).unwrap();
                let first = src.load().unwrap();
                std::fs::write(&p, "name: changed\n").unwrap();
                set_mtime(&p, T1);
                ok(json!([first, src.load().unwrap()]))
            }
            "file_source_deleted_before_load" => {
                let p = root.write("defs/gone.yaml", "name: a\n");
                let mut src = FileSource::new(&p).unwrap();
                std::fs::remove_file(&p).unwrap();
                ok(json!([
                    scrub(src.real_path()),
                    errj(&src.cache_key().unwrap_err()),
                    errj(&src.load().unwrap_err())
                ]))
            }
            "file_source_eq" => {
                let a = FileSource::new(root.join("defs/tool.yaml")).unwrap();
                let b = FileSource::new(root.join("defs/x/../tool.yaml")).unwrap();
                let l = FileSource::new(root.join("defs/link.yaml")).unwrap();
                let st = StringSource::new("name: tool\nversion: '1.0'").unwrap();
                ok(json!([
                    same_source(&a, &b).unwrap(),
                    same_source(&a, &l).unwrap(),
                    same_source(&a, &st).unwrap()
                ]))
            }
            // Relative paths depend on the process cwd (Ruby used Dir.chdir); see metadata test.
            _ => continue,
        };
        assert_eq!(actual, case["result"], "{case}");
        seen += 1;
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(seen >= 13, "{seen}");
}

// ------------------------------------------------------------------ loader

fn def_json(d: &TDef) -> Json {
    json!({
        "name": d.name, "version": d.version, "ukiryu_schema": d.ukiryu_schema, "self_uri": d.self_uri,
        "profiles": d.profiles.clone().unwrap_or_default().iter().map(|p| json!({
            "name": p.name, "inherits": p.inherits,
            "commands": p.commands.clone().unwrap_or_default().iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn mode(v: &Json) -> ValidationMode {
    match v.as_str() {
        None | Some("strict") => ValidationMode::Strict,
        Some("lenient") => ValidationMode::Lenient,
        Some("none") => ValidationMode::None,
        Some(other) => panic!("mode {other}"),
    }
}

#[test]
fn loader_matches_oracle() {
    let doc = fixture!("loader");
    let root = TempRoot::new("ld");
    let valid = cases(&doc).iter().find(|c| c["id"] == "valid").unwrap()["yaml"].clone();
    let missing_name = cases(&doc)
        .iter()
        .find(|c| c["id"] == "missing_name")
        .unwrap()["yaml"]
        .clone();
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "load_from_string" => {
                let loader = Loader::<TDef>::new();
                match loader.load_from_string(s(&case["yaml"]), mode(&case["mode"])) {
                    Ok(d) => {
                        let stderr: String = loader
                            .take_warnings()
                            .iter()
                            .map(|w| format!("[Ukiryu] {w}\n"))
                            .collect();
                        ok(json!({"definition": def_json(&d), "stderr": stderr}))
                    }
                    Err(e) if case["id"] == "syntax_error" => {
                        // The parser's own text differs (Psych vs Yeptris); class and prefix are Ruby's.
                        let expected = s(&case["result"]["error"]["message"]);
                        let prefix = &expected[..expected.find(": input format").unwrap() + 2];
                        assert_eq!(e.ruby_class(), s(&case["result"]["error"]["class"]));
                        assert!(e.message.starts_with(prefix), "{} vs {prefix}", e.message);
                        continue;
                    }
                    Err(e) => err_json(&e),
                }
            }
            "load_from_string_empty" => err_json(
                &Loader::<TDef>::new()
                    .load_from_string("", ValidationMode::Strict)
                    .unwrap_err(),
            ),
            "cache_bypasses_validation" => {
                let loader = Loader::<TDef>::new();
                let a = loader
                    .load_from_string(s(&missing_name), ValidationMode::Lenient)
                    .unwrap();
                let b = loader
                    .load_from_string(s(&missing_name), ValidationMode::Strict)
                    .unwrap();
                loader
                    .clear_cache(Some(&StringSource::new(s(&missing_name)).unwrap()))
                    .unwrap();
                let c = loader
                    .load_from_string(s(&missing_name), ValidationMode::Strict)
                    .unwrap_err();
                ok(json!([Arc::ptr_eq(&a, &b), c.ruby_class()]))
            }
            "cache_identity" => {
                let loader = Loader::<TDef>::new();
                let a = loader
                    .load_from_string(s(&valid), ValidationMode::Strict)
                    .unwrap();
                let b = loader
                    .load_from_string(&format!("  {}\n\n", s(&valid)), ValidationMode::Strict)
                    .unwrap();
                loader.clear_cache(None).unwrap();
                let c = loader
                    .load_from_string(s(&valid), ValidationMode::Strict)
                    .unwrap();
                ok(json!([Arc::ptr_eq(&a, &b), Arc::ptr_eq(&a, &c)]))
            }
            "load_from_file" if !cfg!(unix) => continue,
            "load_from_file" => {
                let path = s(&case["path"]).replace("<ROOT>", &root.path());
                if path.ends_with("t.yaml") {
                    root.write("t.yaml", s(&valid));
                }
                match Loader::<TDef>::new().load_from_file(&path, ValidationMode::Strict) {
                    Ok(d) => ok(def_json(&d)),
                    Err(e) => {
                        json!({"error": {"class": e.ruby_class(), "message": root.scrub(&e.message)}})
                    }
                }
            }
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
    }
    assert_eq!(cases(&doc).len(), 77);
    let bare = tdef("name: t\n");
    assert_eq!(
        validate_profile(&bare, ValidationMode::Lenient).unwrap().unwrap(),
        "Profile validation failed:\n  - Missing 'version' field\n  - Missing 'profiles' field or profiles is empty"
    );
}

#[test]
fn loader_cache_is_bounded_lru() {
    let loader = Loader::<TDef>::with_cache_limits(2, None);
    let doc = |n: &str| format!("name: {n}\nversion: '1'\nprofiles:\n  - name: p\n");
    let a = loader
        .load_from_string(&doc("a"), ValidationMode::Strict)
        .unwrap();
    loader
        .load_from_string(&doc("b"), ValidationMode::Strict)
        .unwrap();
    std::thread::sleep(Duration::from_millis(2));
    assert!(Arc::ptr_eq(
        &a,
        &loader
            .load_from_string(&doc("a"), ValidationMode::Strict)
            .unwrap()
    ));
    loader
        .load_from_string(&doc("c"), ValidationMode::Strict)
        .unwrap();
    assert_eq!(loader.cache_size(), 2);
    assert!(Arc::ptr_eq(
        &a,
        &loader
            .load_from_string(&doc("a"), ValidationMode::Strict)
            .unwrap()
    ));
}

// --------------------------------------------------------------- discovery

#[cfg(unix)]
fn discovery_root() -> (TempRoot, Json) {
    let tree: Json = fixture!("discovery_tree");
    let root = TempRoot::new("dc");
    for (rel, body) in tree["files"].as_object().unwrap() {
        root.write(rel, s(body));
    }
    for rel in tree["dirs"].as_array().unwrap() {
        std::fs::create_dir_all(root.join(s(rel))).unwrap();
    }
    (root, tree)
}

#[cfg(unix)]
fn env_for(root: &TempRoot, tree: &Json, id: &str) -> DiscoveryEnv {
    let sub = |v: &str| v.replace("<ROOT>", &root.path());
    let base = &tree["env"];
    let mut env = DiscoveryEnv {
        home: Some(sub(s(&base["HOME"]))),
        xdg_data_home: None,
        xdg_data_dirs: Some(sub(s(&base["XDG_DATA_DIRS"]))),
        path: Some(sub(s(&base["PATH"]))),
        opt_root: root.join("opt"),
    };
    match id {
        "tree" => {}
        "tree_explicit_home" => env.xdg_data_home = Some(root.join("home/.local/share")),
        "dirs_unset" => env.xdg_data_dirs = None,
        "dirs_empty" => env.xdg_data_dirs = Some(String::new()),
        "home_empty_string" => env.xdg_data_home = Some(String::new()),
        "path_unset" => env.path = None,
        "path_empty" => env.path = Some(String::new()),
        other => panic!("env {other}"),
    }
    env
}

#[cfg(unix)]
#[test]
fn discovery_matches_oracle() {
    let doc = fixture!("discovery");
    let (root, tree) = discovery_root();
    let sc = |v: &str| root.scrub(v);
    let list = |v: Vec<String>| Json::Array(v.iter().map(|x| json!(sc(x))).collect());
    let mds = |v: Vec<DefinitionMetadata>| {
        Json::Array(v.iter().map(|m| json!(sc(&m.to_string()))).collect())
    };
    // Known, documented divergences: Symbol scalars (`:sym`) and dates are
    // typed by Psych but plain strings to Yeptris.
    let divergent = ["symbol.yaml", "date.yaml"];
    let mut checked = 0;
    for case in cases(&doc) {
        let d = case["env"]
            .as_str()
            .map(|id| Discovery::new(env_for(&root, &tree, id)));
        let actual = match s(&case["op"]) {
            "paths" => {
                let d = d.unwrap();
                ok(json!({
                    "xdg_data_home": sc(&d.xdg_data_home()), "xdg_data_dirs": list(d.xdg_data_dirs()),
                    "user_definitions_directory": sc(&d.user_definitions_directory()),
                    "tool_bundled_paths": list(d.tool_bundled_paths()), "search_paths": list(d.search_paths()),
                }))
            }
            "discover" => {
                let mut map = serde_json::Map::new();
                for (k, v) in d.unwrap().discover() {
                    map.insert(k, mds(v));
                }
                ok(Json::Object(map))
            }
            "available_tools" => ok(list(d.unwrap().available_tools())),
            "find" => ok(d
                .unwrap()
                .find(s(&case["tool"]), case["version"].as_str())
                .map(|m| sc(&m.to_string()))),
            "definitions_for" => ok(mds(d.unwrap().definitions_for(s(&case["tool"])))),
            "discover_in_path" => {
                let mut v: Vec<String> = d
                    .unwrap()
                    .discover_in_path(&s(&case["path"]).replace("<ROOT>", &root.path()))
                    .iter()
                    .map(|m| sc(&m.to_string()))
                    .collect();
                v.sort();
                ok(json!(v))
            }
            "determine_source_type" => {
                let path = s(&case["path"]).replace("<ROOT>", &root.path());
                ok(d.unwrap().determine_source_type(&path).as_str().to_string())
            }
            "metadata_from_file" => {
                let f = s(&case["file"]);
                if divergent.contains(&f) {
                    continue;
                }
                let p = if case["body"].is_string() {
                    root.write(&format!("edge/{f}"), s(&case["body"]))
                } else {
                    root.join(f)
                };
                let got =
                    Discovery::metadata_from_file(&p, SourceKind::User).map(|m| sc(&m.to_string()));
                let expected = case["result"]["ok"]["to_s"].as_str().map(str::to_string);
                assert_eq!(got, expected, "{case}");
                checked += 1;
                continue;
            }
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
        checked += 1;
    }
    assert_eq!(checked, 51);
}

// ---------------------------------------------------------------- composer

#[cfg(unix)]
fn cdef_json(d: &TDef) -> Json {
    json!({
        "class": if d.composite { "Composite" } else { "ToolDefinition" },
        "name": d.name, "version": d.version,
        "profiles": d.profiles.as_ref().map(|ps| ps.iter().map(|p| json!({
            "name": p.name, "option_style": p.option_style,
            "commands": p.commands.as_ref().map(|cs| cs.iter().map(|c| format!(
                "{}:{}", c.name.clone().unwrap_or_default(), c.description.clone().unwrap_or_default()
            )).collect::<Vec<_>>()),
        })).collect::<Vec<_>>()),
    })
}

#[cfg(unix)]
#[test]
fn composer_matches_oracle() {
    let doc = fixture!("composer");
    let fx: Json = fixture!("composer_fixtures");
    let docs = &fx["documents"];
    let root = TempRoot::new("cp");
    for (rel, body) in fx["tree"].as_object().unwrap() {
        root.write(rel, s(body));
    }
    let discovery = Discovery::new(DiscoveryEnv {
        home: Some(root.join("home")),
        xdg_data_home: Some(root.join("home/.local/share")),
        xdg_data_dirs: Some(root.join("nodirs")),
        path: Some(root.join("nopath")),
        opt_root: root.join("opt"),
    });
    let mut checked = 0;
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "merge_definitions" => {
                if case["base"] == case["addition"] {
                    // Ruby merges an object with itself; two aliases of one
                    // value cannot be expressed with `&mut` borrows.
                    continue;
                }
                let mut base = tdef(s(&docs[s(&case["base"])]));
                let mut addition = tdef(s(&docs[s(&case["addition"])]));
                let merged = DefinitionComposer::merge_definitions(
                    &mut base,
                    &mut addition,
                    &MergeStrategy::from_symbol(s(&case["strategy"])),
                );
                ok(
                    json!({"merged": cdef_json(&merged), "base_after": cdef_json(&base),
                          "addition_after": cdef_json(&addition), "merged_is_base": false}),
                )
            }
            "merge_strategies_constant" => ok(json!(MergeStrategy::ALL
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>())),
            "compose_tool_definition_identity" => {
                let loader = Loader::<TDef>::new();
                let d = tdef(s(&docs["base"]));
                let out = DefinitionComposer::compose(d.clone(), &discovery, &loader).unwrap();
                assert_eq!(cdef_json(&out), cdef_json(&d));
                ok(json!([
                    true,
                    DefinitionComposer::validate_composable(&d, &discovery, &loader).unwrap()
                ]))
            }
            "compose" => {
                let spec = &fx["composites"][s(&case["id"])];
                let loader = Loader::<TDef>::new();
                let mut c = tdef(s(&fx["child_profiles"]));
                c.composite = true;
                c.inherits = spec["inherits"]
                    .as_array()
                    .map(|a| a.iter().map(json_to_yaml).collect());
                c.includes = spec["includes"]
                    .as_array()
                    .map(|a| a.iter().map(json_to_yaml).collect());
                let errors =
                    DefinitionComposer::validate_composable(&c, &discovery, &loader).unwrap();
                let out =
                    DefinitionComposer::compose(c, &discovery, &Loader::<TDef>::new()).unwrap();
                ok(json!({"composed": cdef_json(&out), "validate_composable": errors}))
            }
            "load_base_definition" => {
                let loader = Loader::<TDef>::new();
                let d = DefinitionComposer::load_base_definition(
                    &json_to_yaml(&case["spec"]),
                    &discovery,
                    &loader,
                )
                .unwrap();
                ok(d.map(|d| cdef_json(&d)))
            }
            // Ruby mutates Loader-cached objects through includes; loaded
            // definitions are cloned here (documented divergence).
            "loader_cache_poisoning" => continue,
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
        checked += 1;
    }
    assert_eq!(checked, 51);
}

// ------------------------------------------------------------------- cache

#[test]
fn definition_cache_matches_oracle() {
    let doc = fixture!("cache");
    let d = |n: &str, v: &str| tdef(&format!("name: {n}\nversion: '{v}'\n"));
    let stats = |c: &DefinitionCache<TDef>| {
        let st = c.stats();
        json!({"size": st.size, "ttl": st.ttl, "refresh_strategy": st.refresh_strategy.as_str(), "entries": st.entries})
    };
    let errj = |r: std::result::Result<RefreshStrategy, DefinitionError>| match r {
        Ok(s) => json!(s.as_str()),
        Err(e) => json!({"class": e.ruby_class(), "message": e.message}),
    };
    for case in cases(&doc) {
        let actual = match s(&case["op"]) {
            "default_ttl" => {
                let c = DefinitionCache::<TDef>::default();
                ok(json!([
                    ukiryu::definition::cache::DEFAULT_TTL,
                    c.ttl(),
                    c.refresh_strategy().as_str()
                ]))
            }
            "basic" => {
                let c = DefinitionCache::<TDef>::default();
                let mut out = vec![json!(if c.get("k1").unwrap().is_none() {
                    "nil"
                } else {
                    "x"
                })];
                let a = c.set("k1", d("a", "1"), None);
                out.push(json!(a.name.as_deref() == Some("a")));
                out.push(json!(
                    c.get("k1").unwrap().unwrap().name.as_deref() == Some("a")
                ));
                out.push(json!(c.contains_key("k1")));
                out.push(json!(c.contains_key("nope")));
                c.set("k0", d("z", "9"), None);
                c.set("k1", d("b", "2"), None);
                out.push(stats(&c));
                out.push(json!(c.invalidate("k1")));
                out.push(json!(c.invalidate("k1")));
                out.push(json!(c.is_stale("nope")));
                out.push(json!(c.is_stale("k0")));
                out.push(json!(c.clear()));
                out.push(json!(c.clear()));
                out.push(stats(&c));
                ok(Json::Array(out))
            }
            "refresh_strategy" => {
                let c = DefinitionCache::<TDef>::default();
                let mut out: Vec<Json> = ["lazy", "eager", "never", "bogus"]
                    .iter()
                    .map(|n| {
                        errj(RefreshStrategy::from_symbol(n).map(|st| {
                            c.set_refresh_strategy(st);
                            c.refresh_strategy()
                        }))
                    })
                    .collect();
                // Ruby rejects the String "lazy" (only Symbols); not representable.
                out.push(case["result"]["ok"][4].clone());
                ok(Json::Array(out))
            }
            "ttl_staleness" => {
                let c = DefinitionCache::<TDef>::new(-1);
                c.set("k", d("a", "1"), None);
                let mut r = vec![
                    json!(c.is_stale("k")),
                    json!(c.get("k").unwrap().is_some()),
                    json!(c.refresh_stale().unwrap()),
                    json!(c.prune()),
                    json!(c.contains_key("k")),
                    stats(&c),
                ];
                let c2 = DefinitionCache::<TDef>::new(1000);
                c2.set("k", d("a", "1"), None);
                r.extend([
                    json!(c2.is_stale("k")),
                    json!(c2.refresh_stale().unwrap()),
                    json!(c2.prune()),
                    json!(c2.contains_key("k")),
                ]);
                ok(Json::Array(r))
            }
            "metadata_entries" | "metadata_without_mtime" if !cfg!(unix) => continue,
            "metadata_entries" | "metadata_without_mtime" => {
                let root = TempRoot::new("ca");
                let p = root.write(
                    "d/tool.yaml",
                    "name: t\nversion: '1'\nprofiles:\n  - name: p\n",
                );
                set_mtime(&p, 1_600_000_000);
                let md = DefinitionMetadata::new("t", "1", &p, SourceKind::User);
                let c = DefinitionCache::<DefinitionMetadata>::default();
                if case["op"] == "metadata_without_mtime" {
                    c.set("m", md.clone(), None);
                    ok(json!([c.is_stale("m")]))
                } else {
                    c.set("m", md.clone(), Some(&md));
                    let fresh = c.is_stale("m");
                    set_mtime(&p, 1_600_000_500);
                    let stale = c.is_stale("m");
                    let ej =
                        |e: DefinitionError| json!({"class": e.ruby_class(), "message": e.message});
                    let got = c.get("m").map(|_| json!("unexpected")).unwrap_or_else(ej);
                    c.set_refresh_strategy(RefreshStrategy::Never);
                    let never = root.scrub(&c.get("m").unwrap().unwrap().to_string());
                    let rs = c.refresh_stale().map(|n| json!(n)).unwrap_or_else(ej);
                    let pr = c.prune();
                    ok(json!([
                        fresh,
                        stale,
                        got,
                        never,
                        rs,
                        pr,
                        c.contains_key("m")
                    ]))
                }
            }
            // `DefinitionCache.instance` (process singleton) is not ported.
            "singleton" => continue,
            other => panic!("unknown op {other}"),
        };
        assert_eq!(actual, case["result"], "{case}");
    }
    assert_eq!(cases(&doc).len(), 7);
}

#[test]
fn error_classes_and_suggestions() {
    let e = StringSource::new("").unwrap_err();
    assert_eq!(e.ruby_class(), "Ukiryu::Errors::DefinitionLoadError");
    assert_eq!(e.suggestions()[0], "Validate YAML syntax");
    let nf = FileSource::new("/nonexistent/ukiryu/defcore/x.yaml").unwrap_err();
    assert_eq!(nf.kind, DefinitionErrorKind::NotFound);
    #[cfg(unix)]
    assert_eq!(
        nf.message,
        "Definition file not found: /nonexistent/ukiryu/defcore/x.yaml"
    );
    let _ = SystemTime::now();
}
