//! Oracle tests for `ukiryu::routing`.
//!
//! `routing_oracle/golden.json` was captured from the original Ruby code
//! (`ukiryu/ukiryu` at ab6df353c6dc2d7faf7817a86aafcafa82579896, see its
//! `oracle` block) by running `ToolIndex`, `Tool::Loader` selection,
//! `ImplementationVersion#compatible_profile` and `Register.load_tool_metadata`
//! over the spec fixture register (ping bsd/gnu/windows, gzip and bzip2
//! default/gnu/busybox and the rest) and synthetic registers. Detection
//! commands were stubbed with canned output per implementation; every call
//! the loader made into scheme resolution, Versionian range matching and
//! version-file loading was recorded in order. The tests replay those calls
//! through `SelectionEnv` and require the same call sequence and result.
use serde_json::{json, Value};
use std::collections::VecDeque;
use ukiryu::routing::{
    build_tool_name, compatible_profile, detect_implementation_and_version, Implementation,
    LoadedVersion, Probe, ProfileView, RegisterFile, RegisterImplementation, RegisterTool,
    RoutingError, SelectionEnv, SelectionRequest, ToolIndex, VersionEntry,
};

fn golden() -> Value {
    serde_json::from_str(include_str!("routing_oracle/golden.json")).unwrap()
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn register(v: &Value) -> Vec<RegisterTool> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| RegisterTool {
            name: s(&t["name"]).unwrap(),
            implementations: t["implementations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| RegisterImplementation {
                    name: s(&i["name"]).unwrap(),
                    files: i["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|f| RegisterFile {
                            file: s(&f["file"]).unwrap(),
                            document: match f.get("doc") {
                                Some(doc) => Ok(doc.clone()),
                                None => Err(s(&f["parse_error"]["message"]).unwrap()),
                            },
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn table(rows: &[(String, Vec<String>)]) -> Value {
    Value::Array(rows.iter().map(|(k, v)| json!([k, v])).collect())
}

/// Ruby outcome encoding: `{"ok": ...}` or `{"error": {"class": ...}}`
/// (messages are not compared).
fn outcome<T: Into<Value>>(r: Result<T, ukiryu::routing::SourceFault>) -> Value {
    match r {
        Ok(v) => json!({ "ok": v.into() }),
        Err(f) => json!({ "error": { "class": f.class } }),
    }
}

fn strip_message(v: &Value) -> Value {
    match v.get("error") {
        Some(e) => json!({ "error": { "class": e["class"] } }),
        None => v.clone(),
    }
}

fn check_index(case: &Value) -> usize {
    let label = case["label"].as_str().unwrap();
    let index = ToolIndex::build(register(&case["register"]));
    let mut n = 0;
    assert_eq!(
        table(index.all_tools()),
        case["all_tools"],
        "{label} all_tools"
    );
    assert_eq!(table(index.aliases()), case["aliases"], "{label} aliases");
    for row in case["primary_file"].as_array().unwrap() {
        let tool = row[0].as_str().unwrap();
        let actual = index
            .primary_file(tool)
            .map(|f| format!("{}/{}/{}", f.tool, f.implementation, f.file));
        assert_eq!(json!({ "ok": actual }), row[1], "{label} primary {tool}");
        n += 1;
    }
    for row in case["find_by_interface"].as_array().unwrap() {
        let key = row[0].as_str().unwrap();
        let actual = index.find_by_interface(key).map(|f| {
            f.map(|f| {
                json!({ "tool": f.tool, "file": format!("{}/{}/{}", f.tool, f.implementation, f.file) })
            })
            .unwrap_or(Value::Null)
        });
        assert_eq!(
            outcome(actual),
            strip_message(&row[1]),
            "{label} find_by_interface {key}"
        );
        assert_eq!(
            json!(index.find_all_by_interface(key)),
            row[2],
            "{label} find_all {key}"
        );
        n += 1;
    }
    for row in case["tool_compatible"].as_array().unwrap() {
        let (tool, platform, shell) = (
            row[0].as_str().unwrap(),
            row[1].as_str().unwrap(),
            row[2].as_str().unwrap(),
        );
        assert_eq!(
            outcome(index.tool_compatible(tool, platform, shell)),
            strip_message(&row[3]),
            "{label} tool_compatible {tool} {platform} {shell}"
        );
        n += 1;
    }
    for row in case["find_by_alias"].as_array().unwrap() {
        let (alias, platform, shell) = (
            row[0].as_str().unwrap(),
            row[1].as_str().unwrap(),
            row[2].as_str().unwrap(),
        );
        let actual = index
            .find_by_alias(alias, platform, shell)
            .map(|t| t.map(str::to_string).map_or(Value::Null, Value::from));
        assert_eq!(
            outcome(actual),
            strip_message(&row[3]),
            "{label} find_by_alias {alias} {platform} {shell}"
        );
        n += 1;
    }
    n
}

#[test]
fn tool_index_fixture_register_matches_ruby() {
    let g = golden();
    let n = check_index(&g["index_fixture"]);
    assert_eq!(n, 22 + 96 + 1056 + 384);
}

#[test]
fn tool_index_synthetic_registers_match_ruby() {
    let g = golden();
    let cases = g["index_synthetic"].as_array().unwrap();
    let labels: Vec<_> = cases.iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["versions", "shapes", "compat"]);
    let n: usize = cases.iter().map(check_index).sum();
    assert_eq!(
        n,
        (10 + 48 + 480 + 240) + (14 + 123 + 672 + 288) + (8 + 44 + 384 + 240)
    );
}

#[test]
fn fixture_index_warnings_are_empty_and_synthetic_shapes_warn() {
    let g = golden();
    let fixture = ToolIndex::build(register(&g["index_fixture"]["register"]));
    assert!(fixture.warnings().is_empty());
    let shapes = ToolIndex::build(register(&g["index_synthetic"][1]["register"]));
    let warned: Vec<_> = shapes
        .warnings()
        .iter()
        .map(|w| (w.tool.as_str(), w.fault.class))
        .collect();
    // The Ruby run printed "Warning: Failed to parse <tool>" for exactly these.
    assert_eq!(
        warned,
        [
            ("eps", "NoMethodError"),
            ("eta", "Psych::SyntaxError"),
            ("kappa", "TypeError"),
            ("mu", "NoMethodError"),
            ("xi", "NoMethodError"),
            ("zeta", "NoMethodError"),
        ]
    );
}

fn profile(v: &Value) -> ProfileView {
    let list = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| s(x).unwrap())
            .collect()
    };
    ProfileView {
        name: s(&v["name"]),
        platforms: list(&v["platforms"]),
        shells: list(&v["shells"]),
        inherits: s(&v["inherits"]),
        has_commands: v["has_commands"].as_bool().unwrap(),
    }
}

fn implementations(v: &Value) -> Vec<Implementation> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| Implementation {
            name: s(&i["name"]),
            default: s(&i["default"]),
            versions: i["versions"].as_array().map(|vs| {
                vs.iter()
                    .map(|v| VersionEntry {
                        file: s(&v["file"]),
                        default: v["default"].as_bool().unwrap(),
                    })
                    .collect()
            }),
        })
        .collect()
}

/// Replays the recorded Ruby call log; any call out of order fails the test.
struct Replay {
    names: Vec<Option<String>>,
    events: VecDeque<Value>,
    row: String,
}

#[derive(Debug, PartialEq)]
struct Recorded(Value);

impl Replay {
    fn next(&mut self, kind: &str) -> Value {
        let event = self
            .events
            .pop_front()
            .unwrap_or_else(|| panic!("{}: unexpected {kind} call", self.row));
        assert!(
            event.get(kind).is_some(),
            "{}: expected {event}, got {kind}",
            self.row
        );
        event
    }
}

impl SelectionEnv for Replay {
    type Error = Recorded;

    fn probe(&mut self, i: usize) -> Result<Probe, Recorded> {
        let e = self.next("probe");
        assert_eq!(
            e["probe"].as_str(),
            self.names[i].as_deref(),
            "{}",
            self.row
        );
        Ok(match &e["result"] {
            Value::String(x) if x == "not_detected" => Probe::NotDetected,
            r => Probe::Detected {
                version: s(&r["version"]),
            },
        })
    }

    fn resolve_scheme(&mut self, _i: usize) -> Result<(), Recorded> {
        let e = self.next("resolve");
        match e["result"].get("error") {
            Some(err) => Err(Recorded(err["class"].clone())),
            None => Ok(()),
        }
    }

    fn match_version(&mut self, _i: usize, version: &str) -> Result<Option<usize>, Recorded> {
        let e = self.next("match");
        assert_eq!(e["match"], version, "{}", self.row);
        match e["result"].get("error") {
            Some(err) => Err(Recorded(err["class"].clone())),
            None => Ok(e["result"]["ok"].as_u64().map(|x| x as usize)),
        }
    }

    fn load_version(&mut self, i: usize, file: &str) -> Result<Option<LoadedVersion>, Recorded> {
        let e = self.next("load");
        assert_eq!(e["load"], json!([self.names[i], file]), "{}", self.row);
        if let Some(err) = e["result"].get("error") {
            return Err(Recorded(err["class"].clone()));
        }
        Ok(e["result"].as_object().map(|r| LoadedVersion {
            version: s(&r["version"]),
            profiles: r["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .map(profile)
                .collect(),
        }))
    }
}

fn check_loader(section: &Value) -> (usize, usize) {
    let mut faults = 0;
    let rows = section["rows"].as_array().unwrap();
    for (n, row) in rows.iter().enumerate() {
        let tool = row["tool"].as_str().unwrap();
        let impls = implementations(&section["indexes"][tool]);
        let mut env = Replay {
            names: impls.iter().map(|i| i.name.clone()).collect(),
            events: row["events"].as_array().unwrap().iter().cloned().collect(),
            row: format!("row {n} {tool} {}", row["outputs"]),
        };
        let request = SelectionRequest {
            current_platform: row["current"].as_str().unwrap(),
            target_platform: row["target"].as_str(),
            shell: row["shell"].as_str().unwrap(),
        };
        let actual = match detect_implementation_and_version(&impls, &request, &mut env) {
            Ok(spec) => json!({ "ok": spec.map(|s| json!({
                "implementation": s.implementation_name,
                "version": s.version,
                "file": s.file,
            })) }),
            Err(RoutingError::Source(f)) => {
                faults += 1;
                json!({ "error": { "class": f.class } })
            }
            Err(RoutingError::Env(Recorded(class))) => {
                faults += 1;
                json!({ "error": { "class": class } })
            }
        };
        assert_eq!(actual, strip_message(&row["result"]), "{}", env.row);
        assert!(
            env.events.is_empty(),
            "{}: calls left {:?}",
            env.row,
            env.events
        );
    }
    (rows.len(), faults)
}

#[test]
fn loader_selection_over_fixture_indexes_matches_ruby() {
    let g = golden();
    let tools: Vec<_> = g["loader_fixture"]["indexes"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        tools,
        [
            "bzip2",
            "bzip2_busybox",
            "bzip2_gnu",
            "exiftool",
            "ffmpeg",
            "ghostscript",
            "gzip",
            "gzip_busybox",
            "gzip_gnu",
            "imagemagick",
            "inkscape",
            "jq",
            "pdf2ps",
            "ping",
            "ping_bsd",
            "ping_gnu",
            "ping_windows",
            "sort",
            "tar",
            "unzip",
            "yq"
        ]
    );
    assert_eq!(check_loader(&g["loader_fixture"]), (1216, 0));
}

#[test]
fn loader_selection_source_faults_stay_faults() {
    // Includes Versionian calver NoMethodError (`after`), calver octal
    // ArgumentError, unknown/missing schemes and NoMethodError on missing or
    // empty `versions`, all recorded from the Ruby run.
    let g = golden();
    assert_eq!(check_loader(&g["loader_synthetic"]), (311, 73));
}

#[test]
fn compatible_profile_matches_ruby() {
    let g = golden();
    let section = &g["compatible_profile"];
    let rows = section["rows"].as_array().unwrap();
    let mut selected = 0;
    for row in rows {
        let label = row[0].as_str().unwrap();
        let profiles: Vec<_> = section["profiles"][label]
            .as_array()
            .unwrap()
            .iter()
            .map(profile)
            .collect();
        let m = compatible_profile(
            &profiles,
            row[1].as_str().unwrap(),
            row[2].as_str().unwrap(),
        );
        let actual = m.map(|m| json!({ "profile": m.profile, "commands_from": m.commands_from }));
        if actual.is_some() {
            selected += 1;
        }
        assert_eq!(json!({ "ok": actual }), row[3], "{row}");
    }
    assert_eq!((rows.len(), selected), (1728, 351));
}

#[test]
fn build_tool_name_matches_ruby() {
    let g = golden();
    for row in g["build_tool_name"].as_array().unwrap() {
        assert_eq!(
            build_tool_name(row[0].as_str().unwrap(), row[1].as_str()),
            row[2].as_str().unwrap(),
            "{row}"
        );
    }
}

#[test]
fn versioned_interface_fallback_matches_register_lookup() {
    let g = golden();
    let index = ToolIndex::build(register(&g["index_fixture"]["register"]));
    let rows = g["load_tool_metadata"].as_array().unwrap();
    let mut via_index = 0;
    for row in rows {
        // Rows found by exact tool name never reach the index (loader-owned step).
        if row[1].as_bool().unwrap() {
            continue;
        }
        via_index += 1;
        let name = row[0].as_str().unwrap();
        let actual = index.find_by_interface_keys(name).unwrap().map(|f| f.tool);
        assert_eq!(actual.as_deref(), row[2]["ok"]["tool"].as_str(), "{name}");
    }
    assert_eq!((rows.len(), via_index), (26, 6));
}
