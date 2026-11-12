//! Help extraction against data captured from the pinned Ruby source
//! (ukiryu/ukiryu ab6df353, lib/ukiryu/extractors/*.rb, Ruby 4.0.1). The
//! capture ran the original methods with only `Executor.execute` and
//! `Shell.detect` stubbed; see `provenance` in the data file.

use std::collections::HashMap;

use serde_json::{json, Value as Json};
use ukiryu::extractors::help::{
    self, CommandHost, CommandOutput, ExtractError, ExtractionMethod, ExtractionResult, HelpParser,
    HostError, NativeExtractor,
};
use yeptris::Value;

fn oracle() -> Json {
    serde_json::from_str(include_str!("../src/extractors/help/testdata/oracle.json")).unwrap()
}

fn bytes(entry: &Json, key: &str) -> Vec<u8> {
    if let Some(text) = entry.get(key).and_then(Json::as_str) {
        return text.as_bytes().to_vec();
    }
    let hex = entry[format!("{key}_hex")].as_str().unwrap_or("");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn text(entry: &Json, key: &str) -> String {
    String::from_utf8(bytes(entry, key)).unwrap()
}

/// The ordered, tagged view the capture script writes for Ruby values.
fn ordered(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => json!(b),
        Value::String(s) => json!(s),
        Value::Sequence(items) => Json::Array(items.iter().map(ordered).collect()),
        Value::Mapping(map) => {
            json!({"map": map.0.iter().map(|(k, v)| json!([ordered(k), ordered(v)])).collect::<Vec<_>>()})
        }
        other => panic!("unexpected YAML value {other:?}"),
    }
}

/// Scripted host: responses keyed by argv; a `seq` entry answers call by
/// call and repeats its last element.
struct ScriptedHost {
    responses: HashMap<Vec<String>, Json>,
    positions: HashMap<Vec<String>, usize>,
    calls: Vec<Vec<String>>,
}

impl ScriptedHost {
    fn new(responses: &Json) -> ScriptedHost {
        let responses = responses
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (argv(&r["argv"]), r.clone()))
            .collect();
        ScriptedHost {
            responses,
            positions: HashMap::new(),
            calls: Vec::new(),
        }
    }
}

fn argv(value: &Json) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect()
}

impl CommandHost for ScriptedHost {
    fn execute(
        &mut self,
        executable: &str,
        args: &[&str],
        timeout_secs: u64,
    ) -> Result<CommandOutput, HostError> {
        assert_eq!(timeout_secs, 30);
        let key: Vec<String> = std::iter::once(executable)
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect();
        self.calls.push(key.clone());
        let mut response = self
            .responses
            .get(&key)
            .unwrap_or_else(|| panic!("no response for {key:?}"))
            .clone();
        if let Some(seq) = response.get("seq").and_then(Json::as_array) {
            let position = self.positions.entry(key).or_insert(0);
            let entry = seq[(*position).min(seq.len() - 1)].clone();
            *position += 1;
            response = entry;
        }
        if response.get("enoent").is_some() {
            return Err(HostError::NotFound);
        }
        if let Some(raised) = response.get("raise") {
            return Err(HostError::Raised {
                class: raised["class"].as_str().unwrap().to_owned(),
                message: raised["message"].as_str().unwrap().to_owned(),
            });
        }
        Ok(CommandOutput {
            stdout: bytes(&response, "stdout"),
            stderr: bytes(&response, "stderr"),
            exit_status: response["exit_status"].as_i64().unwrap() as i32,
        })
    }
}

fn outcome<T>(result: Result<T, ExtractError>, ok: impl FnOnce(T) -> Json) -> Json {
    match result {
        Ok(value) => json!({"ok": ok(value)}),
        Err(error) => json!({"error": {"class": error.class_name(), "message": error.message()}}),
    }
}

/// The capture tags Ruby values once more when they sit inside a strategy
/// result hash (`ordered` applied to an already ordered view); mirror that.
fn retag(value: Json) -> Json {
    match value {
        Json::Object(object) => {
            let inner = object
                .into_iter()
                .map(|(k, v)| json!([k, retag(v)]))
                .collect::<Vec<_>>();
            json!({ "map": inner })
        }
        Json::Array(items) => Json::Array(items.into_iter().map(retag).collect()),
        other => other,
    }
}

/// The Psych text and our YAML must both read back as the captured data.
fn yaml_matches(id: &str, ours: &str, expected: &Json, tag: fn(Json) -> Json) {
    let data = &expected["data"];
    let reread: Value =
        yeptris::from_str(ours).unwrap_or_else(|e| panic!("{id}: our YAML does not parse: {e}"));
    assert_eq!(&tag(ordered(&reread)), data, "{id}: our YAML data");
    let psych: Value = yeptris::from_str(expected["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        &tag(ordered(&psych)),
        data,
        "{id}: Psych text read by yeptris"
    );
}

fn result_json(id: &str, result: ExtractionResult, expected: &Json) -> Json {
    // The source's hash key order differs between success and failure; the
    // port is a struct, so compare by key.
    let pairs = expected["map"].as_array().unwrap();
    let expected_yaml = pairs
        .iter()
        .find(|p| p[0] == "yaml")
        .map(|p| p[1].clone())
        .unwrap();
    // Inside the result hash the capture's {text, data} view is itself an
    // ordered map.
    let view: Option<Json> = expected_yaml.get("map").map(|pairs| {
        let field = |name: &str| {
            pairs
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p[0] == name)
                .map(|p| p[1].clone())
                .unwrap()
        };
        json!({"text": field("text"), "data": field("data")})
    });
    let yaml = match (&result.yaml, view) {
        (Some(ours), Some(view)) => {
            yaml_matches(id, ours, &view, retag);
            expected_yaml.clone()
        }
        (Some(ours), None) => json!(ours),
        (None, _) => Json::Null,
    };
    let method = result
        .method
        .map_or(Json::Null, |m| json!({"symbol": m.as_str()}));
    let mut ours: Vec<Json> = vec![
        json!(["success", result.success]),
        json!(["yaml", yaml]),
        json!(["method", method]),
        json!(["error", result.error]),
    ];
    let order: Vec<&Json> = pairs.iter().map(|p| &p[0]).collect();
    ours.sort_by_key(|p| order.iter().position(|k| **k == p[0]).unwrap());
    json!({"map": ours})
}

fn method(kind: &str) -> Option<ExtractionMethod> {
    match kind {
        "default" => None,
        "auto" => Some(ExtractionMethod::Auto),
        "native" => Some(ExtractionMethod::Native),
        "help" => Some(ExtractionMethod::Help),
        // The capture passed the String "help", not the Symbol :help.
        "str:help" => Some(ExtractionMethod::Other("help".to_owned())),
        other => Some(ExtractionMethod::Other(other.to_owned())),
    }
}

#[test]
fn recorded_help_texts_parse_like_the_source() {
    let oracle = oracle();
    let cases = oracle["parse"].as_array().unwrap();
    assert_eq!(cases.len(), 75);
    let mut with_options = 0;
    for case in cases {
        let options = help::parse_help_options(&text(case, "text"));
        with_options += usize::from(!options.is_empty());
        let actual = Json::Array(options.iter().map(|o| ordered(&o.to_value())).collect());
        assert_eq!(json!({"ok": actual}), case["expect"], "{}", case["id"]);
    }
    assert_eq!(with_options, 34);
}

#[test]
fn parameter_types_are_inferred_like_the_source() {
    let oracle = oracle();
    let cases = oracle["infer_type"].as_array().unwrap();
    assert_eq!(cases.len(), 27);
    for case in cases {
        let actual = help::infer_type(case["param"].as_str().unwrap()).as_str();
        assert_eq!(actual, case["type"], "{}", case["param"]);
    }
}

#[test]
fn command_not_found_heuristic_matches_the_source() {
    let oracle = oracle();
    let cases = oracle["command_not_found"].as_array().unwrap();
    assert_eq!(cases.len(), 91);
    let mut found = 0;
    for case in cases {
        let actual = help::is_command_not_found(&text(case, "stdout"), &text(case, "stderr"));
        found += usize::from(actual);
        assert_eq!(json!({"ok": actual}), case["expect"], "{}", case["id"]);
    }
    assert_eq!(found, 69);
}

#[test]
fn extraction_flows_match_the_source() {
    let oracle = oracle();
    let flows = oracle["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 247);
    let mut errors = 0;
    for flow in flows {
        let id = flow["id"].as_str().unwrap();
        let tool = flow["tool"].as_str().unwrap();
        let expected = &flow["expect"];
        let mut host =
            ScriptedHost::new(&oracle["response_sets"][flow["responses"].as_str().unwrap()]);
        let kind = flow["kind"].as_str().unwrap();
        let actual = match kind {
            "help_available" => outcome(HelpParser::new(tool, &mut host).available(), |v| json!(v)),
            "help_extract" => outcome(
                HelpParser::new(tool, &mut host).extract(),
                |yaml| match yaml {
                    Some(ours) => {
                        yaml_matches(id, &ours, &expected["ok"], |v| v);
                        expected["ok"].clone()
                    }
                    None => Json::Null,
                },
            ),
            "native_available" => outcome(NativeExtractor::new(tool, &mut host).available(), |v| {
                json!(v)
            }),
            "native_extract" => outcome(NativeExtractor::new(tool, &mut host).extract(), |v| {
                json!(v)
            }),
            _ => {
                let selector = kind.strip_prefix("extract:").unwrap();
                outcome(help::extract(tool, method(selector), &mut host), |r| {
                    result_json(id, r, &expected["ok"])
                })
            }
        };
        errors += usize::from(actual.get("error").is_some());
        assert_eq!(&actual, expected, "{id}");
        let calls: Vec<Vec<String>> = flow["calls"].as_array().unwrap().iter().map(argv).collect();
        assert_eq!(host.calls, calls, "{id}: command sequence");
    }
    assert_eq!(errors, 71);
}
