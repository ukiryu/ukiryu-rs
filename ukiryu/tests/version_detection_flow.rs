//! Detection strategy order, fallbacks and exact runner requests against
//! cases captured from the pinned Ruby (ukiryu ab6df353) with its process,
//! shell, platform and man-page boundaries replaced by scripted stubs.
use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};
use ukiryu::version_detection::{
    detect_info, detect_profile_version, detect_with_methods, probe_index_detection, CommandOutput, CommandRequest,
    CommandRunner, DetectedVersion, DetectionMethodView, IndexDetection, IndexProbe, InfoRequest, ManPageDates,
    MethodSpec, MethodsRequest, PatternError, PatternSpec, RunnerError, ToolContext, VersionDetectionView,
};

fn golden() -> Value {
    serde_json::from_str(include_str!("version_detection_golden/flow.json")).unwrap()
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

struct Scripted {
    responses: VecDeque<Value>,
    calls: Vec<CommandRequest>,
}

impl CommandRunner for Scripted {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, RunnerError> {
        self.calls.push(request.clone());
        let response = self.responses.pop_front().expect("runner called more often than Ruby executed");
        if let Some(message) = response["raise"].as_str() {
            return Err(RunnerError::new(message));
        }
        Ok(CommandOutput {
            stdout: unhex(response["stdout_hex"].as_str().unwrap()),
            stderr: unhex(response["stderr_hex"].as_str().unwrap()),
            exit_status: response["status"].as_i64().unwrap() as i32,
        })
    }
}

struct Dates {
    dates: BTreeMap<String, String>,
    calls: Vec<String>,
}

impl ManPageDates for Dates {
    fn parse_date(&mut self, path: &str) -> Option<String> {
        self.calls.push(path.to_string());
        self.dates.get(path).cloned()
    }
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// A Ruby `command` value: a string becomes a one-element list.
fn command_list(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(items) => Some(items.iter().map(|i| i.as_str().unwrap().to_string()).collect()),
        other => panic!("unexpected command {other}"),
    }
}

fn pattern_spec(v: &Value) -> Option<PatternSpec> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(PatternSpec::Source(s.clone())),
        Value::Object(o) if o.get("regexp_literal").is_some() => Some(PatternSpec::DefaultLiteral),
        other => panic!("unexpected pattern {other}"),
    }
}

fn paths(v: &Value) -> Option<Vec<(String, String)>> {
    v.as_object().map(|o| {
        o.iter()
            .map(|(k, p)| (k.clone(), p.as_str().unwrap().to_string()))
            .collect()
    })
}

fn version_json(found: Option<DetectedVersion>) -> Value {
    match found {
        None => Value::Null,
        Some(v) => json!({
            "value": v.value,
            "method_used": v.method_used.name(),
            "available_methods": v.available_methods.iter().map(|m| m.name()).collect::<Vec<_>>(),
        }),
    }
}

/// What Ruby passed as `shell:` maps to the request's shell; the stub's
/// `Shell.detect` answer means the caller passed none.
fn expected_calls(case: &Value) -> Vec<CommandRequest> {
    case["expected"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| CommandRequest {
            executable: c["executable"].as_str().unwrap().to_string(),
            args: command_list(&c["args"]).unwrap(),
            shell: c["shell"].as_str().filter(|s| *s != "detected_shell").map(str::to_string),
            timeout_secs: c["timeout"].as_u64().unwrap(),
            allow_failure: c["allow_failure"].as_bool().unwrap(),
        })
        .collect()
}

enum Actual {
    Version(Result<Option<DetectedVersion>, PatternError>),
    Index(Result<IndexProbe, PatternError>),
}

fn run_case(case: &Value, runner: &mut Scripted, dates: &mut Dates) -> Actual {
    let input = &case["input"];
    let platform = case["platform"].as_str().unwrap().to_string();
    match case["api"].as_str().unwrap() {
        "detect_info" => Actual::Version(detect_info(
            &InfoRequest {
                executable: opt_str(&input["executable"]),
                command: command_list(&input["command"]).unwrap(),
                pattern: pattern_spec(&input["pattern"]).unwrap(),
                shell: opt_str(&input["shell"]),
                source: opt_str(&input["source"]),
                timeout_secs: input["timeout"].as_u64().unwrap(),
            },
            runner,
        )),
        "detect_with_methods" => {
            let methods = input["methods"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| match m["type"].as_str().unwrap() {
                    "command" => MethodSpec::Command {
                        command: command_list(&m["command"]),
                        pattern: pattern_spec(&m["pattern"]),
                    },
                    "man_page" => MethodSpec::ManPage { paths: paths(&m["paths"]) },
                    other => MethodSpec::Other(other.to_string()),
                })
                .collect();
            Actual::Version(detect_with_methods(
                &MethodsRequest {
                    executable: opt_str(&input["executable"]),
                    methods,
                    shell: opt_str(&input["shell"]),
                    timeout_secs: input["timeout"].as_u64().unwrap(),
                    platform,
                },
                runner,
                dates,
            ))
        }
        "tool_detect_version" => {
            let vd = &input["version_detection"];
            let view = (!vd.is_null()).then(|| VersionDetectionView {
                command: command_list(&vd["command"]),
                pattern: opt_str(&vd["pattern"]),
                source: opt_str(&vd["source"]),
                detection_methods: vd["detection_methods"].as_array().map(|methods| {
                    methods
                        .iter()
                        .map(|m| DetectionMethodView {
                            method_type: opt_str(&m["type"]),
                            command: command_list(&m["command"]),
                            pattern: opt_str(&m["pattern"]),
                            paths: paths(&m["paths"]),
                        })
                        .collect()
                }),
            });
            let tool = ToolContext {
                profile_version: opt_str(&input["profile_version"]),
                executable: opt_str(&input["executable"]),
                shell: opt_str(&input["shell"]),
                platform,
            };
            Actual::Version(detect_profile_version(view.as_ref(), &tool, runner, dates))
        }
        "index_probe" => {
            let d = &input["detection"];
            let detection = IndexDetection {
                command: command_list(&d["command"]),
                executables: command_list(&d["executables"]),
                executable: opt_str(&d["executable"]),
                pattern: opt_str(&d["pattern"]),
            };
            Actual::Index(probe_index_detection(
                &detection,
                input["tool_name"].as_str().unwrap(),
                input["options"]["executable"].as_str(),
                input["options"]["shell"].as_str(),
                runner,
            ))
        }
        other => panic!("unknown api {other}"),
    }
}

#[test]
fn captured_detection_cases_match_ruby() {
    let golden = golden();
    let cases = golden["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 252, "case count changed: regenerate deliberately");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let expected = &case["expected"];
        let mut runner = Scripted {
            responses: case["responses"].as_array().unwrap().iter().cloned().collect(),
            calls: Vec::new(),
        };
        let mut dates = Dates {
            dates: case["man_dates"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect(),
            calls: Vec::new(),
        };
        match run_case(case, &mut runner, &mut dates) {
            Actual::Version(result) => {
                assert!(expected["raised"].is_null(), "{id}: Ruby raised");
                let found = result.unwrap_or_else(|e| panic!("{id}: {e}"));
                let mut want = expected["result"].clone();
                if let Some(o) = want.as_object_mut() {
                    o.remove("to_s");
                }
                assert_eq!(version_json(found), want, "{id}: result");
            }
            Actual::Index(result) => match (&expected["raised"], result) {
                (Value::Null, Ok(probe)) => {
                    let want = &expected["result"];
                    let got = match probe {
                        IndexProbe::NotDetected => json!("not_detected"),
                        IndexProbe::Detected { output, version } => {
                            assert_eq!(want["output"], json!(output), "{id}: output");
                            assert_eq!(want["version"], json!(version), "{id}: version");
                            json!("detected")
                        }
                    };
                    assert_eq!(got, want["outcome"], "{id}: outcome");
                }
                (raised, Err(PatternError::RubyInvalid { .. })) if raised["class"] == "RegexpError" => {}
                (raised, other) => panic!("{id}: Ruby raised {raised}, port gave {other:?}"),
            },
        }
        assert_eq!(runner.calls, expected_calls(case), "{id}: runner requests");
        assert_eq!(
            runner.responses.len() as u64,
            expected["unused_responses"].as_u64().unwrap(),
            "{id}: executions"
        );
        let man_calls: Vec<String> = expected["man_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        assert_eq!(dates.calls, man_calls, "{id}: man page lookups");
        *counts.entry(case["api"].as_str().unwrap().to_string()).or_default() += 1;
    }
    eprintln!("flow cases: {counts:?}");
}

#[test]
fn unsupported_pattern_is_an_error_not_a_miss() {
    let mut runner = Scripted {
        responses: VecDeque::new(),
        calls: Vec::new(),
    };
    let mut request = InfoRequest::new("/usr/bin/tool");
    request.pattern = PatternSpec::Source(r"(?<=v)(\d+)".to_string());
    let result = detect_info(&request, &mut runner);
    assert!(matches!(result, Err(PatternError::Unsupported { .. })));
    assert!(runner.calls.is_empty(), "nothing runs for a pattern the port cannot reproduce");
}
