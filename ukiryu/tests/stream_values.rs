//! Process output values at every stage the pinned Ruby implementation shows
//! them (ukiryu/ukiryu@ab6df353): the executor's result streams, the
//! `Execution::Output`/`Result` readers, `to_h`/`to_json`,
//! `SuccessResponse.from_result` (the `OutputInfo` cast), and the CLI
//! response formats with and without an output file.
//!
//! `stream_values_data/observations.json` was captured by running
//! `capture_stream_values.rb` against the locked oracle bundle;
//! `stream_values_data/cli.json` by running the real `ukiryu exec` CLI
//! through `cli/capture_cli.py`. Each payload is written by a real bash
//! child, once on stdout and once on stderr. No expectation is hand-written.
use std::path::Path;

use regex::Regex;
use serde_json::Value as Json;
use ukiryu::execution::results::bytes;
use ukiryu::execution::{
    CommandInfo, ExecutionMetadata, ExecutionResult, InvalidByteSequence, Output, PathStyle,
    Seconds,
};
use ukiryu::models::{ErrorResponse, ExecutionResultParts, SuccessResponse};
use ukiryu::render::{
    output_response, to_json_pretty, OutputFormat, OutputWrite, PaintSupport, RenderSettings,
    Renderable,
};
use ukiryu::types::value::Time;

/// The settings of the capture: `say` colours (the CLI config does not
/// disable them) while the YAML is not coloured (no TTY).
const SETTINGS: RenderSettings = RenderSettings {
    use_color: None,
    stdout_tty: false,
    paint: PaintSupport::UNAVAILABLE,
};

fn data(name: &str) -> Json {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stream_values_data")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("JSON")
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn s(v: &Json) -> &str {
    v.as_str()
        .unwrap_or_else(|| panic!("expected a string, got {v}"))
}

fn strings(v: &Json) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| s(x).to_owned())
        .collect()
}

/// The Ruby error recorded for a stage, if it raised.
fn ruby_error(v: &Json) -> Option<(String, String)> {
    let e = v.get("error")?;
    Some((
        s(&e["class"]).to_owned(),
        String::from_utf8(unhex(s(&e["message_hex"]))).expect("message"),
    ))
}

fn assert_raised(id: &str, stage: &str, got: &InvalidByteSequence, want: &Json) {
    let (class, message) =
        ruby_error(want).unwrap_or_else(|| panic!("{id} {stage}: Ruby did not raise, got {got:?}"));
    assert_eq!(got.ruby_class(), class, "{id} {stage}: class");
    assert_eq!(got.message(), message, "{id} {stage}: message");
}

/// A stage that returns a value in Ruby or raises.
fn check<T: std::fmt::Debug>(
    id: &str,
    stage: &str,
    got: Result<T, InvalidByteSequence>,
    want: &Json,
    eq: impl Fn(&T, &Json) -> bool,
) {
    match got {
        Ok(value) => {
            assert!(
                ruby_error(want).is_none(),
                "{id} {stage}: Ruby raised {want}, got {value:?}"
            );
            assert!(eq(&value, want), "{id} {stage}: got {value:?}, want {want}");
        }
        Err(e) => assert_raised(id, stage, &e, want),
    }
}

fn text_eq(got: &&str, want: &Json) -> bool {
    hex(got.as_bytes()) == s(&want["hex"])
}

#[allow(clippy::ptr_arg)] // `check` passes `&T` for `T = Vec<String>`.
fn lines_eq(got: &Vec<String>, want: &Json) -> bool {
    let want: Vec<String> = strings(want);
    got.iter().map(|l| hex(l.as_bytes())).collect::<Vec<_>>() == want
}

fn bool_eq(got: &bool, want: &Json) -> bool {
    want.as_bool() == Some(*got)
}

fn time(v: &Json) -> Time {
    Time {
        seconds: v["seconds"].as_i64().unwrap(),
        nanos: v["nanos"].as_u64().unwrap() as u32,
        utc_offset: v["utc_offset"].as_i64().unwrap() as i32,
        utc: false,
    }
}

fn fixed_metadata(doc: &Json) -> ExecutionMetadata {
    let m = &doc["fixed_metadata"];
    ExecutionMetadata::new(
        Some(time(&m["started_at"])),
        Some(time(&m["finished_at"])),
        Some(Seconds::Int(m["timeout"].as_i64().unwrap())),
    )
}

fn command_info(row: &Json) -> CommandInfo {
    CommandInfo {
        executable: "bash".to_owned(),
        arguments: strings(&row["arguments"]),
        full_command: s(&row["full_command"]).to_owned(),
        shell: Some("bash".to_owned()),
        tool_name: None,
        command_name: None,
    }
}

/// The ordered writes of one `output_response` call, as the capture
/// records them (stdout, stderr, file bytes; the file path in stdout is
/// `<FILE>`).
fn writes<R: Renderable + ?Sized>(response: &R, format: OutputFormat, file: bool) -> Json {
    let path = "/tmp/stream-values-out";
    let writes = output_response(response, format, file.then_some(path), &SETTINGS)
        .unwrap_or_else(|e| panic!("render: {e:?}"));
    let (mut out, mut err, mut contents) = (String::new(), String::new(), None);
    for w in writes {
        match w {
            OutputWrite::Stdout(t) => out.push_str(&t),
            OutputWrite::Stderr(t) => err.push_str(&t),
            OutputWrite::File { contents: c, .. } => contents = Some(c),
        }
    }
    let mut row = serde_json::json!({
        "stdout_hex": hex(out.replace(path, "<FILE>").as_bytes()),
        "stderr_hex": hex(err.as_bytes()),
    });
    if file {
        row["file_hex"] = contents.map_or(Json::Null, |c| Json::String(hex(c.as_bytes())));
    }
    row
}

fn model_text(model: &SuccessResponse, stream: &str) -> String {
    let output = model.output.as_deref().expect("output");
    let attr = if stream == "stdout" {
        &output.stdout
    } else {
        &output.stderr
    };
    attr.get()
        .and_then(|t| t.as_str())
        .expect("String attribute")
        .to_owned()
}

#[test]
fn every_stage_matches_the_ruby_capture() {
    let doc = data("observations.json");
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 38, "pinned row count");
    let needles: Vec<String> = strings(&doc["needles_hex"])
        .iter()
        .map(|h| String::from_utf8(unhex(h)).unwrap())
        .collect();
    let regexes: Vec<Regex> = strings(&doc["regexes"])
        .iter()
        .map(|r| Regex::new(r).unwrap())
        .collect();
    let mut checked = 0;
    for row in rows {
        let id = s(&row["id"]);
        if row["outcome"] != "result" {
            continue;
        }
        checked += 1;
        let out = unhex(s(&row["stdout_payload_hex"]));
        let err = unhex(s(&row["stderr_payload_hex"]));
        let status = row["status"].as_i64().unwrap();
        let output = Output::new(out.clone(), err.clone(), status);
        let result = ExecutionResult::new(command_info(row), output.clone(), fixed_metadata(&doc));

        // Raw streams keep every byte.
        assert_eq!(hex(result.stdout()), s(&row["raw_stdout"]["hex"]), "{id}");
        assert_eq!(hex(result.stderr()), s(&row["raw_stderr"]["hex"]), "{id}");

        check(id, "output", result.output(), &row["output"], text_eq);
        check(
            id,
            "error_output",
            result.error_output(),
            &row["error_output"],
            text_eq,
        );
        check(
            id,
            "stdout_lines",
            result.stdout_lines(),
            &row["stdout_lines"],
            lines_eq,
        );
        check(
            id,
            "stderr_lines",
            result.stderr_lines(),
            &row["stderr_lines"],
            lines_eq,
        );
        for (i, needle) in needles.iter().enumerate() {
            let n = needle.as_str();
            check(
                id,
                "stdout_contains",
                result.stdout_contains(n),
                &row["stdout_contains"][i],
                bool_eq,
            );
            check(
                id,
                "stderr_contains",
                result.stderr_contains(n),
                &row["stderr_contains"][i],
                bool_eq,
            );
        }
        for (i, re) in regexes.iter().enumerate() {
            check(
                id,
                "stdout_matches",
                result.stdout_contains(re),
                &row["stdout_matches"][i],
                bool_eq,
            );
            check(
                id,
                "stderr_matches",
                result.stderr_contains(re),
                &row["stderr_matches"][i],
                bool_eq,
            );
        }
        check(
            id,
            "stdout_empty",
            output.is_stdout_empty(),
            &row["stdout_empty"],
            bool_eq,
        );
        check(
            id,
            "stderr_empty",
            output.is_stderr_empty(),
            &row["stderr_empty"],
            bool_eq,
        );
        assert_eq!(
            output.stdout_length() as u64,
            row["stdout_length"].as_u64().unwrap(),
            "{id}"
        );
        assert_eq!(
            output.stderr_length() as u64,
            row["stderr_length"].as_u64().unwrap(),
            "{id}"
        );
        assert_eq!(output.to_string(), s(&row["output_to_s"]), "{id}");
        check(
            id,
            "output_to_h_json",
            output.to_h().map(|h| serde_json::to_string(&h).unwrap()),
            &row["output_to_h_json"],
            |got, want| hex(got.as_bytes()) == s(want),
        );
        match result.to_json_with(PathStyle::Posix) {
            Ok(json) => assert_eq!(hex(json.as_bytes()), s(&row["result_to_json"]), "{id}"),
            Err(e) => {
                let (class, message) = ruby_error(&row["result_to_json"])
                    .unwrap_or_else(|| panic!("{id}: to_json raised {e:?}"));
                assert_eq!(e.ruby_class(), class, "{id} to_json");
                assert_eq!(e.message(), message, "{id} to_json");
            }
        }

        // SuccessResponse.from_result: OutputInfo casts the streams.
        let want = &row["from_result"];
        let model = match ExecutionResultParts::try_from(&result) {
            Err(e) => {
                let (class, message) =
                    ruby_error(want).unwrap_or_else(|| panic!("{id}: from_result raised {e:?}"));
                assert_eq!(e.ruby_class(), class, "{id} from_result");
                assert_eq!(e.message(), message, "{id} from_result");
                continue;
            }
            Ok(parts) => {
                assert_eq!(parts.stdout, out, "{id}: parts keep raw stdout");
                SuccessResponse::from_result(&parts, &[], None, None).unwrap()
            }
        };
        assert!(ruby_error(want).is_none(), "{id}: Ruby raised {want}");
        assert_eq!(
            hex(model_text(&model, "stdout").as_bytes()),
            s(&want["stdout"]["hex"]),
            "{id}"
        );
        assert_eq!(
            hex(model_text(&model, "stderr").as_bytes()),
            s(&want["stderr"]["hex"]),
            "{id}"
        );

        // CLI formats, to stdout and to an output file.
        for (name, format) in [
            ("raw", OutputFormat::Raw),
            ("yaml", OutputFormat::Yaml),
            ("json", OutputFormat::Json),
        ] {
            for file in [false, true] {
                let key = if file {
                    format!("{name}_file")
                } else {
                    name.to_owned()
                };
                assert_eq!(
                    writes(&model, format, file),
                    row["render"][&key],
                    "{id} {key}"
                );
            }
        }
    }
    assert_eq!(checked, 34, "rows that return a result");
}

/// The executor run itself: the bytes the child wrote reach the result
/// unchanged, and a failed run raises what the source raises.
#[cfg(unix)]
#[test]
fn real_bash_children_match_the_ruby_capture() {
    use ukiryu::environment::Environment;
    use ukiryu::executor::{execute, ExecuteError, ExecuteRequest};
    use ukiryu::runtime::ShellName;

    let doc = data("observations.json");
    for row in doc["rows"].as_array().unwrap() {
        let id = s(&row["id"]);
        let request = ExecuteRequest::new(
            "bash",
            strings(&row["arguments"]),
            Environment::new(),
            ShellName::Bash,
            Seconds::Int(30),
        );
        let out = unhex(s(&row["stdout_payload_hex"]));
        let err = unhex(s(&row["stderr_payload_hex"]));
        match execute(request) {
            Ok(execution) => {
                assert_eq!(row["outcome"], "result", "{id}");
                assert_eq!(execution.stdout, out, "{id}: bytes written");
                assert_eq!(execution.stderr, err, "{id}: bytes written");
                assert_eq!(
                    hex(execution.result.stdout()),
                    s(&row["raw_stdout"]["hex"]),
                    "{id}"
                );
                assert_eq!(
                    hex(execution.result.stderr()),
                    s(&row["raw_stderr"]["hex"]),
                    "{id}"
                );
                assert_eq!(execution.result.command(), s(&row["full_command"]), "{id}");
            }
            Err(ExecuteError::Failed { error, execution }) => {
                assert_eq!(row["outcome"], "raise", "{id}");
                assert_eq!(error.ruby_class(), s(&row["error"]["class"]), "{id}");
                assert_eq!(
                    hex(error.message().as_bytes()),
                    s(&row["error"]["message_hex"]),
                    "{id}"
                );
                assert_eq!(execution.stdout, out, "{id}");
            }
            Err(ExecuteError::InvalidByteSequence { error, execution }) => {
                assert_eq!(row["outcome"], "raise", "{id}");
                assert_raised(id, "execute", &error, row);
                assert_eq!(execution.stdout, out, "{id}: bytes kept");
                assert_eq!(execution.stderr, err, "{id}: bytes kept");
            }
            Err(other) => panic!("{id}: {other:?}"),
        }
    }
}

/// The real `ukiryu exec --raw` and `--format json` runs: the library
/// stages above, with the run command's `rescue` mapping of the error
/// classes, give the same bytes.
#[test]
fn cli_runs_match_the_library_stages() {
    let doc = data("cli.json");
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 36, "pinned row count");
    let observations = data("observations.json");
    let mut checked = 0;
    for row in rows {
        let id = s(&row["id"]);
        let out = unhex(s(&row["stdout_payload_hex"]));
        let err = unhex(s(&row["stderr_payload_hex"]));
        let status = row["status"].as_i64().unwrap();
        let cli_error = row["json"]["response"]["error"].as_str();
        if cli_error.is_some_and(|e| e.starts_with("Command failed: ")) {
            // The ExecutionError message names the host path of the tool;
            // its bytes are compared by the executor rows above.
            continue;
        }
        checked += 1;
        let result = ExecutionResult::new(
            CommandInfo {
                executable: "emit".to_owned(),
                arguments: vec![],
                full_command: "emit".to_owned(),
                shell: Some("bash".to_owned()),
                tool_name: None,
                command_name: None,
            },
            Output::new(out.clone(), err.clone(), status),
            fixed_metadata(&observations),
        );
        // Executor.format_error strips stdout, then stderr, for a failed run.
        let execute = if status != 0 {
            bytes::strip(&out).and(bytes::strip(&err)).map(|_| ())
        } else {
            Ok(())
        };
        let response = execute.and_then(|()| {
            ExecutionResultParts::try_from(&result).map_err(|e| match e {
                ukiryu::execution::ResultPartsError::InvalidByteSequence(e) => e,
                other => panic!("{id}: {other:?}"),
            })
        });
        match response {
            Ok(parts) => {
                let model = SuccessResponse::from_result(&parts, &[], None, None).unwrap();
                let raw = writes(&model, OutputFormat::Raw, false);
                assert_eq!(
                    raw["stdout_hex"], row["raw"]["stdout_hex"],
                    "{id} raw stdout"
                );
                assert_eq!(
                    raw["stderr_hex"], row["raw"]["stderr_hex"],
                    "{id} raw stderr"
                );
                let raw_file = writes(&model, OutputFormat::Raw, true);
                assert_eq!(
                    raw_file["file_hex"], row["raw_file"]["file_hex"],
                    "{id} raw file"
                );
                let json: Json = serde_json::from_str(&to_json_pretty(&model).unwrap()).unwrap();
                // Both streams empty: the model omits `output` (null here).
                let output = json["output"].as_object().map(|o| {
                    o.iter()
                        .map(|(k, v)| (k.clone(), Json::String(hex(s(v).as_bytes()))))
                        .collect::<serde_json::Map<_, _>>()
                });
                assert_eq!(
                    output.map_or(Json::Null, Json::Object),
                    row["json"]["response"]["output"],
                    "{id} json output"
                );
            }
            Err(e) => {
                // RunCommand#execute_request: `rescue ArgumentError` and
                // `rescue StandardError`.
                let message = if e.ruby_class() == "ArgumentError" {
                    format!("Invalid arguments: {}", e.message())
                } else {
                    format!("Unexpected error: {}: {}", e.ruby_class(), e.message())
                };
                assert_eq!(cli_error, Some(message.as_str()), "{id} json error");
                let response = ErrorResponse::from_message(&message, 1);
                let raw = writes(&response, OutputFormat::Raw, false);
                assert_eq!(
                    raw["stdout_hex"], row["raw"]["stdout_hex"],
                    "{id} raw stdout"
                );
                assert_eq!(
                    raw["stderr_hex"], row["raw"]["stderr_hex"],
                    "{id} raw stderr"
                );
                assert_eq!(row["raw_file"]["file_hex"], Json::Null, "{id}: no file");
            }
        }
    }
    assert_eq!(checked, 35, "CLI rows checked");
}
