//! The loader's typed model construction (`ukiryu::model_loading`) against
//! lutaml `Model.new(**kwargs)` as called by `Ukiryu::Tool::Loader`.
//!
//! Golden data: `model_loading_data/adapters.json`, producer
//! `capture/adapters.rb` run against ukiryu ab6df353 (Ruby 4.0.1,
//! lutaml-model 0.8.175). Each row replays one
//! `convert_to_tool_definition(tool, interface, version, impl, detected,
//! platform:, shell:)` over the oracle's register fixtures (byte copies in
//! `models_data/fixtures`): every recorded `CommandDefinition.new`,
//! `PlatformProfile.new` and `ToolDefinition.new` call is rebuilt from its
//! captured keywords, then the converted ToolDefinition is compared after
//! `resolve_inheritance!`, through `to_yaml`, and after a
//! `ToolDefinition.from_yaml(to_yaml)` roundtrip. `probes` pin `nil` versus
//! absent keywords, nested Hash casting and attribute-name keywords.
#[path = "models_data/support.rs"]
mod support;

use serde_json::{json, Map, Value as Json};
use support::*;
use ukiryu::model_loading::{command_definition_new, platform_profile_new, tool_definition_new};
use ukiryu::models::*;

fn golden_data() -> Json {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/model_loading_data/adapters.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden")).expect("json")
}

/// Inverse of the oracle encoder (`$sym`, `$float`, `$map`).
fn dec(j: &Json) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::int(n.as_i64().expect("integer")),
        Json::String(s) => Value::str(s.clone()),
        Json::Array(items) => Value::Array(items.iter().map(dec).collect()),
        Json::Object(m) => {
            if let Some(s) = m.get("$sym") {
                return Value::sym(s.as_str().unwrap());
            }
            if let Some(f) = m.get("$float") {
                return Value::Float(f.as_str().unwrap().parse().unwrap());
            }
            if let Some(pairs) = m.get("$map") {
                let mut h = Value::Hash(Vec::new());
                for p in pairs.as_array().unwrap() {
                    h.insert(dec(&p[0]), dec(&p[1]));
                }
                return h;
            }
            assert!(!m.keys().any(|k| k.starts_with('$')), "unexpected {j}");
            let mut h = Value::Hash(Vec::new());
            for (k, v) in m {
                h.insert(Value::str(k.clone()), dec(v));
            }
            h
        }
    }
}

fn hash_of<T: Model>(m: &T) -> Json {
    enc(&m.to_hash().expect("to_hash"))
}

fn profile_readers(p: &PlatformProfile) -> Json {
    let mut routing = Map::new();
    for (k, v) in p.routing().table() {
        routing.insert(k, enc(&v));
    }
    json!({
        "option_style": p.option_style.get().map(|t| t.to_ruby_s()),
        "routing": Json::Object(routing),
    })
}

fn command_readers(c: &CommandDefinition) -> Json {
    json!({ "standalone_executable": c.standalone_executable.get().copied() })
}

struct Check {
    failures: Vec<String>,
    passed: usize,
}

impl Check {
    fn eq(&mut self, what: impl Fn() -> String, expected: &Json, actual: &Json) {
        if same(expected, actual) {
            self.passed += 1;
        } else {
            self.failures.push(format!(
                "{}\n  expected {expected}\n  actual   {actual}",
                what()
            ));
        }
    }

    fn yaml(&mut self, what: impl Fn() -> String, expected: &str, actual: &str) {
        if yaml_equiv(expected, actual) {
            self.passed += 1;
        } else {
            self.failures.push(format!(
                "{}\n  expected {expected:?}\n  actual   {actual:?}",
                what()
            ));
        }
    }

    fn finish(self, name: &str) {
        let n = self.failures.len();
        assert!(
            n == 0,
            "{name}: {n} mismatches ({} passed)\n{}",
            self.passed,
            self.failures
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        eprintln!("{name}: {} checks passed", self.passed);
    }
}

#[test]
fn constructor_probes() {
    let g = golden_data();
    let probes = g["probes"].as_array().unwrap();
    assert_eq!(probes.len(), 10);
    let mut c = Check {
        failures: Vec::new(),
        passed: 0,
    };
    for (i, p) in probes.iter().enumerate() {
        let kwargs = dec(&p["kwargs"]);
        let ok = &p["result"]["ok"];
        let what = |part: &str| format!("probe {i} {} {part}", p["class"]);
        let (hash, yaml, readers) = match p["class"].as_str().unwrap() {
            "PlatformProfile" => {
                let m = platform_profile_new(&kwargs, Vec::new()).expect("profile");
                (hash_of(&m), m.to_yaml().unwrap(), profile_readers(&m))
            }
            "CommandDefinition" => {
                let m = command_definition_new(&kwargs).expect("command");
                (hash_of(&m), m.to_yaml().unwrap(), command_readers(&m))
            }
            "ToolDefinition" => {
                let mut m = tool_definition_new(&kwargs, Vec::new(), None).expect("tool");
                // `profiles` was not passed: the source attribute stays nil.
                m.profiles = None;
                (hash_of(&m), m.to_yaml().unwrap(), json!({}))
            }
            other => panic!("class {other}"),
        };
        c.eq(|| what("to_hash"), &ok["hash"], &hash);
        c.yaml(|| what("to_yaml"), ok["yaml"].as_str().unwrap(), &yaml);
        for (k, v) in readers.as_object().unwrap() {
            c.eq(|| what(k.as_str()), &ok[k], v);
        }
    }
    c.finish("constructor_probes");
}

#[test]
fn loader_conversion_over_register_fixtures() {
    let g = golden_data();
    assert_eq!(
        g["oracle"].as_str(),
        Some("ab6df353c6dc2d7faf7817a86aafcafa82579896")
    );
    let rows = g["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 96);
    let mut c = Check {
        failures: Vec::new(),
        passed: 0,
    };
    let mut converted = 0;
    let mut no_profile = 0;
    let mut calls = 0;
    let mut last_profile: Option<PlatformProfile> = None;
    for row in rows {
        let file = row["file"].as_str().unwrap();
        let platform = row["platform"].as_str().unwrap();
        let shell = row["shell"].as_str().unwrap();
        let tag = format!(
            "{file} {platform}/{shell} detected={}",
            row["detected_version"]
        );
        let text = std::fs::read_to_string(data_dir().join("fixtures").join(file)).unwrap();
        let version = ImplementationVersion::from_register_yaml(&text)
            .expect("version loads")
            .expect("version document");
        let selected = version
            .compatible_profile(platform, shell)
            .expect("compatible_profile");
        let result = &row["result"]["ok"];
        if result.is_null() {
            no_profile += 1;
            c.eq(
                || format!("{tag} no profile"),
                &json!(false),
                &json!(selected.is_some()),
            );
            continue;
        }
        let name = selected
            .as_ref()
            .and_then(|p| p.get_sym_or_str("name"))
            .map(|n| n.to_ruby_s());
        c.eq(|| format!("{tag} profile"), &row["profile"], &json!(name));

        let mut commands = Vec::new();
        for call in row["calls"].as_array().unwrap() {
            calls += 1;
            let kwargs = dec(&call["kwargs"]);
            match call["class"].as_str().unwrap() {
                "CommandDefinition" => {
                    let m = command_definition_new(&kwargs).expect("command");
                    c.eq(
                        || format!("{tag} command hash"),
                        &call["hash"],
                        &hash_of(&m),
                    );
                    c.eq(
                        || format!("{tag} command readers"),
                        &json!({ "standalone_executable": call["standalone_executable"] }),
                        &command_readers(&m),
                    );
                    commands.push(m);
                }
                "PlatformProfile" => {
                    let m = platform_profile_new(&kwargs, std::mem::take(&mut commands))
                        .expect("profile");
                    c.eq(
                        || format!("{tag} profile hash"),
                        &call["hash"],
                        &hash_of(&m),
                    );
                    c.eq(
                        || format!("{tag} profile readers"),
                        &json!({ "option_style": call["option_style"], "routing": call["routing"] }),
                        &profile_readers(&m),
                    );
                    last_profile = Some(m);
                }
                "ToolDefinition" => {
                    let profile = last_profile.clone().expect("profile built first");
                    let mut tool = tool_definition_new(
                        &kwargs,
                        vec![profile],
                        version.version_detection.clone(),
                    )
                    .expect("tool");
                    c.eq(
                        || format!("{tag} tool hash"),
                        &call["hash"],
                        &hash_of(&tool),
                    );
                    tool.resolve_inheritance();
                    converted += 1;
                    c.eq(
                        || format!("{tag} result hash"),
                        &result["hash"],
                        &hash_of(&tool),
                    );
                    let yaml = tool.to_yaml().expect("to_yaml");
                    c.yaml(
                        || format!("{tag} result yaml"),
                        result["yaml"].as_str().unwrap(),
                        &yaml,
                    );
                    let back = ToolDefinition::from_yaml(&yaml).expect("roundtrip");
                    c.eq(
                        || format!("{tag} roundtrip"),
                        &result["roundtrip"],
                        &hash_of(&back),
                    );
                }
                other => panic!("class {other}"),
            }
        }
    }
    assert_eq!((converted, no_profile, calls), (64, 32, 148));
    c.finish("loader_conversion_over_register_fixtures");
}
