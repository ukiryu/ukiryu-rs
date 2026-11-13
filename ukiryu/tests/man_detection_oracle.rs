//! `version_detection::detect_with_methods` with the real man-page parser
//! (`extractors::man::ManPageParser`) as its `ManPageDates` boundary, against
//! cases captured from the pinned Ruby (ukiryu ab6df353) running the real
//! `ManPageParser.parse_date` inside `VersionDetector.detect_with_methods`.
//! Man-page errors (`Date::Error`, `RangeError`, `ArgumentError`,
//! `Errno::EISDIR`) must escape with the source's class and message.
//! "Today" comes from an injected `runtime::Clock`, never the host clock.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;

use serde_json::{json, Value};
use ukiryu::execution_context::ExecutionContext;
use ukiryu::extractors::man::{ManPageParser, ManPageSource, Today};
use ukiryu::runtime::Clock;
use ukiryu::version_detection::{
    detect_with_methods, CommandOutput, CommandRequest, CommandRunner, DetectedVersion,
    DetectionError, ManPageDates, MethodSpec, MethodsRequest, PatternSpec, RunnerError,
};

fn golden() -> Value {
    serde_json::from_str(include_str!(
        "man_detection_golden/detect_with_methods.json"
    ))
    .unwrap()
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn captured_today(g: &Value) -> Today {
    let t = &g["today"];
    Today {
        year: t["year"].as_i64().unwrap() as i32,
        mon: t["mon"].as_i64().unwrap() as i32,
        mday: t["mday"].as_i64().unwrap() as i32,
    }
}

/// The capture's temp directory, as an in-memory file system.
#[derive(Clone, Default)]
struct Pages {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

impl Pages {
    fn from_golden(g: &Value) -> Self {
        Pages {
            files: g["pages"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(path, hex)| (path.clone(), unhex(hex.as_str().unwrap())))
                .collect(),
            dirs: g["directories"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_string())
                .collect(),
        }
    }
}

impl ManPageSource for Pages {
    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path)
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        if self.dirs.contains(path) {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "Is a directory",
            ));
        }
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

struct Scripted {
    responses: VecDeque<Value>,
    calls: Vec<CommandRequest>,
}

impl CommandRunner for Scripted {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, RunnerError> {
        self.calls.push(request.clone());
        let response = self
            .responses
            .pop_front()
            .expect("runner called more often than Ruby executed");
        Ok(CommandOutput {
            stdout: unhex(response["stdout_hex"].as_str().unwrap()),
            stderr: Vec::new(),
            exit_status: response["status"].as_i64().unwrap() as i32,
        })
    }
}

/// Records which pages detection asked for, then delegates to the real parser.
struct Recording<'a, D: ManPageDates> {
    inner: &'a mut D,
    calls: Vec<String>,
}

impl<D: ManPageDates> ManPageDates for Recording<'_, D> {
    fn parse_date(
        &mut self,
        path: &str,
    ) -> Result<Option<String>, ukiryu::extractors::man::ManPageError> {
        self.calls.push(path.to_string());
        self.inner.parse_date(path)
    }
}

fn method_spec(m: &Value) -> MethodSpec {
    match m["type"].as_str().unwrap() {
        "man_page" => MethodSpec::ManPage {
            paths: Some(
                m["paths"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                    .collect(),
            ),
        },
        "command" => MethodSpec::Command {
            command: Some(
                m["command"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap().to_string())
                    .collect(),
            ),
            pattern: Some(match m["pattern"].as_str() {
                Some(p) => PatternSpec::Source(p.to_string()),
                None => PatternSpec::DefaultLiteral,
            }),
        },
        other => panic!("unexpected method type {other}"),
    }
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

fn expected_calls(case: &Value) -> Vec<CommandRequest> {
    case["expected"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| CommandRequest {
            executable: c["executable"].as_str().unwrap().to_string(),
            args: c["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_string())
                .collect(),
            shell: c["shell"].as_str().map(str::to_string),
            timeout_secs: c["timeout"].as_u64().unwrap(),
            allow_failure: c["allow_failure"].as_bool().unwrap(),
        })
        .collect()
}

fn run(
    case: &Value,
    parser: &mut ManPageParser<Pages>,
) -> (
    Result<Option<DetectedVersion>, DetectionError>,
    Scripted,
    Vec<String>,
) {
    let mut runner = Scripted {
        responses: case["responses"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .collect(),
        calls: Vec::new(),
    };
    let mut dates = Recording {
        inner: parser,
        calls: Vec::new(),
    };
    let request = MethodsRequest {
        executable: Some("/usr/bin/tool".to_string()),
        methods: case["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(method_spec)
            .collect(),
        shell: Some("bash".to_string()),
        timeout_secs: 30,
        platform: "linux".to_string(),
    };
    let result = detect_with_methods(&request, &mut runner, &mut dates);
    let calls = dates.calls;
    (result, runner, calls)
}

#[test]
fn man_page_errors_propagate_like_ruby() {
    let g = golden();
    let cases = g["cases"].as_array().unwrap();
    assert_eq!(
        cases.len(),
        15,
        "case count changed: regenerate deliberately"
    );
    // The execution context's injected clock supplies "today".
    let clock = ExecutionContext::new()
        .with_clock(Clock::Fixed(captured_today(&g)))
        .clock();
    let mut parser = ManPageParser::with_clock(Pages::from_golden(&g), clock);

    let (mut ok, mut raised) = (0, 0);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let expected = &case["expected"];
        let (result, runner, man_calls) = run(case, &mut parser);

        match (&expected["raised"], result) {
            (Value::Null, Ok(found)) => {
                assert_eq!(version_json(found), expected["result"], "{id}: result");
                ok += 1;
            }
            (want, Err(DetectionError::ManPage(e))) if !want.is_null() => {
                let class = want["class"].as_str().unwrap();
                assert_eq!(e.ruby_class(), class, "{id}: class");
                if class.starts_with("Errno::") {
                    // Ruby's message carries the capture path; the port has
                    // the OS error text.
                    assert!(
                        want["message"]
                            .as_str()
                            .unwrap()
                            .starts_with("Is a directory"),
                        "{id}"
                    );
                } else {
                    assert_eq!(e.to_string(), want["message"].as_str().unwrap(), "{id}");
                }
                raised += 1;
            }
            (want, other) => panic!("{id}: Ruby raised {want}, port gave {other:?}"),
        }

        assert_eq!(runner.calls, expected_calls(case), "{id}: runner requests");
        assert_eq!(
            runner.responses.len() as u64,
            expected["unused_responses"].as_u64().unwrap(),
            "{id}: executions"
        );
        // Every man_page method up to the result or error reads its page;
        // nothing after an error runs.
        let linux_paths: Vec<String> = case["methods"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["paths"]["linux"].as_str().map(str::to_string))
            .collect();
        assert!(
            linux_paths.starts_with(&man_calls),
            "{id}: man page lookups {man_calls:?} not a prefix of {linux_paths:?}"
        );
    }
    assert_eq!((ok, raised), (8, 7), "ok/raised split changed");
}

#[test]
fn partial_date_follows_the_injected_clock_not_the_host() {
    let g = golden();
    let pages = Pages::from_golden(&g);
    let day = |year| Today {
        year,
        mon: 1,
        mday: 1,
    };
    for year in [1999, 2031] {
        let mut parser = ManPageParser::with_clock(pages.clone(), Clock::Fixed(day(year)));
        assert_eq!(
            ManPageDates::parse_date(&mut parser, "<man>/partial_date").unwrap(),
            Some(format!("{year}-09-21"))
        );
    }
}

#[test]
fn errors_keep_their_ruby_class_through_detection_error() {
    let g = golden();
    let mut parser = ManPageParser::new(Pages::from_golden(&g), captured_today(&g));
    let err = ManPageDates::parse_date(&mut parser, "<man>/invalid_date").unwrap_err();
    let err = DetectionError::from(err);
    assert_eq!(err.ruby_class(), Some("Date::Error"));
    assert_eq!(err.to_string(), "invalid date");
    assert!(std::error::Error::source(&err).is_some());
}
