//! Oracle parity for `ukiryu::execution` (`Ukiryu::Execution::*` and the
//! profile exit-code mapping of `Ukiryu::Response::Base`).
//!
//! Every expectation under `tests/execution_result_golden/` was captured by
//! running the original Ruby implementation; see that directory's README.
//! Row counts are pinned so a dropped or added row fails loudly.
use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use serde_json::Value;
use ukiryu::execution::results::ruby::{json_float, time_iso8601};
use ukiryu::execution::{
    exit_code_meaning, format_duration, format_meaning, CommandInfo, ExecutionMetadata,
    ExecutionResult, ExitCodeTable, Output, PathStyle, Seconds,
};
use ukiryu::models::success_response::ExecutionResultParts;
use ukiryu::models::value::RubyValue;
use ukiryu::models::{Attr, ExitCodes, ModelErrorKind};
use ukiryu::types::ruby::float_to_s;
use ukiryu::types::value::Time;

fn golden(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/execution_result_golden")
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(|l| serde_json::from_str(l).expect("golden row is JSON"))
        .collect()
}

fn text(v: &Value) -> &str {
    v.as_str()
        .unwrap_or_else(|| panic!("expected a string, got {v}"))
}

fn opt_text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn time(v: &Value) -> Option<Time> {
    if v.is_null() {
        return None;
    }
    Some(Time::new(
        v["seconds"].as_i64().unwrap(),
        v["nanos"].as_u64().unwrap() as u32,
        v["utc_offset"].as_i64().unwrap() as i32,
        v["utc"].as_bool().unwrap(),
    ))
}

fn seconds(v: &Value) -> Option<Seconds> {
    if v.is_null() {
        return None;
    }
    if let Some(i) = v.get("int") {
        return Some(Seconds::Int(i.as_i64().unwrap()));
    }
    Some(Seconds::Float(text(&v["float"]).parse().unwrap()))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| text(s).to_owned())
        .collect()
}

fn float_s(v: Option<f64>) -> Value {
    v.map_or(Value::Null, |f| Value::String(float_to_s(f)))
}

fn output_from(input: &Value) -> Output {
    Output::new(
        text(&input["stdout"]),
        text(&input["stderr"]),
        input["exit_status"].as_i64().unwrap(),
    )
}

fn command_info_from(input: &Value) -> CommandInfo {
    CommandInfo {
        executable: text(&input["executable"]).to_owned(),
        arguments: strings(&input["arguments"]),
        full_command: text(&input["full_command"]).to_owned(),
        shell: opt_text(&input["shell"]),
        tool_name: opt_text(&input["tool_name"]),
        command_name: opt_text(&input["command_name"]),
    }
}

fn metadata_from(input: &Value) -> ExecutionMetadata {
    ExecutionMetadata::new(
        time(&input["started_at"]),
        time(&input["finished_at"]),
        seconds(&input["timeout"]),
    )
}

#[test]
fn output_matches_oracle() {
    let rows = golden("output");
    assert_eq!(rows.len(), 19);
    let patterns = strings(&rows[0]["patterns"]);
    let regexes: Vec<Regex> = rows[0]["regexes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| Regex::new(text(&pair[1])).unwrap())
        .collect();
    for row in &rows[1..] {
        let o = output_from(&row["input"]);
        let e = &row["expect"];
        let ctx = &row["input"];
        assert_eq!(o.stdout(), text(&e["stdout"]), "{ctx}");
        assert_eq!(o.stderr(), text(&e["stderr"]), "{ctx}");
        assert_eq!(o.stdout_lines(), strings(&e["stdout_lines"]), "{ctx}");
        assert_eq!(o.stderr_lines(), strings(&e["stderr_lines"]), "{ctx}");
        assert_eq!(o.is_stdout_empty(), e["stdout_empty"] == true, "{ctx}");
        assert_eq!(o.is_stderr_empty(), e["stderr_empty"] == true, "{ctx}");
        assert_eq!(
            o.stdout_length() as u64,
            e["stdout_length"].as_u64().unwrap()
        );
        assert_eq!(
            o.stderr_length() as u64,
            e["stderr_length"].as_u64().unwrap()
        );
        assert_eq!(o.is_success(), e["success"] == true, "{ctx}");
        assert_eq!(o.is_failure(), e["failure"] == true, "{ctx}");
        assert_eq!(
            serde_json::to_string(&o.to_h()).unwrap(),
            text(&e["to_h_json"]),
            "{ctx}"
        );
        assert_eq!(o.to_string(), text(&e["to_s"]), "{ctx}");
        assert_eq!(o.inspect(), text(&e["inspect"]), "{ctx}");
        for (i, p) in patterns.iter().enumerate() {
            assert_eq!(
                o.stdout_contains(p.as_str()),
                e["stdout_contains"][i] == true,
                "{ctx} {p:?}"
            );
            assert_eq!(
                o.stderr_contains(p.as_str()),
                e["stderr_contains"][i] == true,
                "{ctx} {p:?}"
            );
        }
        for (i, r) in regexes.iter().enumerate() {
            assert_eq!(
                o.stdout_contains(r),
                e["stdout_matches"][i] == true,
                "{ctx} {r}"
            );
            assert_eq!(
                o.stderr_contains(r),
                e["stderr_matches"][i] == true,
                "{ctx} {r}"
            );
        }
    }
}

#[test]
fn metadata_matches_oracle() {
    let rows = golden("metadata");
    assert_eq!(rows.len(), 139);
    for row in &rows {
        let m = metadata_from(&row["input"]);
        let e = &row["expect"];
        let ctx = &row["input"];
        assert_eq!(float_s(m.duration()), e["duration"], "{ctx}");
        assert_eq!(
            float_s(m.duration_seconds()),
            e["duration_seconds"],
            "{ctx}"
        );
        assert_eq!(
            float_s(m.duration_milliseconds()),
            e["duration_milliseconds"],
            "{ctx}"
        );
        assert_eq!(m.is_timed_out(), e["timed_out"] == true, "{ctx}");
        assert_eq!(
            m.formatted_duration(),
            text(&e["formatted_duration"]),
            "{ctx}"
        );
        assert_eq!(m.to_string(), text(&e["to_s"]), "{ctx}");
        assert_eq!(m.inspect(), text(&e["inspect"]), "{ctx}");
        assert_eq!(m.to_json().unwrap(), text(&e["to_h_json"]), "{ctx}");
        let iso = |t: Option<&Time>| t.map_or(Value::Null, |t| Value::String(time_iso8601(t)));
        assert_eq!(iso(m.started_at.as_ref()), e["started_iso8601"], "{ctx}");
        assert_eq!(iso(m.finished_at.as_ref()), e["finished_iso8601"], "{ctx}");
    }
}

#[test]
fn duration_format_and_json_floats_match_oracle() {
    let rows = golden("duration_format");
    assert_eq!(rows.len(), 1534);
    let mut mismatches = Vec::new();
    for row in &rows {
        let bits = u64::from_str_radix(text(&row["bits"]), 16).unwrap();
        let d = f64::from_bits(bits);
        let got = [
            format_duration(d),
            float_to_s(d),
            json_float(d),
            json_float(d * 1000.0),
        ];
        let want = [
            text(&row["formatted"]),
            text(&row["to_s"]),
            text(&row["json"]),
            text(&row["ms_json"]),
        ];
        if got.iter().zip(want).any(|(g, w)| g != w) {
            mismatches.push(format!("{d:e}: got {got:?}, want {want:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn command_info_matches_oracle() {
    let rows = golden("command_info");
    assert_eq!(rows.len(), 10);
    for row in &rows {
        let c = command_info_from(&row["input"]);
        let e = &row["expect"];
        let ctx = &row["input"];
        // Captured on Linux: Posix separator rules.
        let style = PathStyle::Posix;
        assert_eq!(
            c.executable_name_with(style),
            text(&e["executable_name"]),
            "{ctx}"
        );
        assert_eq!(
            c.argument_count() as u64,
            e["argument_count"].as_u64().unwrap()
        );
        assert_eq!(c.to_string(), text(&e["to_s"]), "{ctx}");
        assert_eq!(c.inspect_with(style), text(&e["inspect"]), "{ctx}");
        assert_eq!(
            serde_json::to_string(&c.to_h_with(style)).unwrap(),
            text(&e["to_h_json"]),
            "{ctx}"
        );
    }
}

#[test]
fn result_matches_oracle() {
    let rows = golden("result");
    assert_eq!(rows.len(), 12);
    for row in &rows {
        let input = &row["input"];
        let r = ExecutionResult::new(
            command_info_from(&input["command_info"]),
            output_from(&input["output"]),
            metadata_from(&input["metadata"]),
        );
        let e = &row["expect"];
        assert_eq!(r.command(), text(&e["command"]), "{input}");
        assert_eq!(r.executable(), text(&e["executable"]));
        assert_eq!(
            r.command_info.executable_name_with(PathStyle::Posix),
            text(&e["executable_name"])
        );
        assert_eq!(r.stdout(), text(&e["stdout"]));
        assert_eq!(r.stderr(), text(&e["stderr"]));
        assert_eq!(r.status(), e["status"].as_i64().unwrap());
        assert_eq!(r.exit_code(), e["exit_code"].as_i64().unwrap());
        assert_eq!(r.exit_status(), e["exit_status"].as_i64().unwrap());
        assert_eq!(float_s(r.duration()), e["duration"]);
        assert_eq!(float_s(r.execution_time()), e["execution_time"]);
        assert_eq!(r.is_success(), e["success"] == true);
        assert_eq!(r.is_failure(), e["failure"] == true);
        assert_eq!(r.output(), text(&e["output"]));
        assert_eq!(r.error_output(), text(&e["error_output"]));
        assert_eq!(r.stdout_lines(), strings(&e["stdout_lines"]));
        assert_eq!(r.stderr_lines(), strings(&e["stderr_lines"]));
        assert_eq!(
            r.to_json_with(PathStyle::Posix).unwrap(),
            text(&e["to_json"]),
            "{input}"
        );
        assert_eq!(r.to_string(), text(&e["to_s"]));
        assert_eq!(r.inspect(), text(&e["inspect"]));
        // The projection `SuccessResponse.from_result` reads.
        let parts = ExecutionResultParts::try_from(&r).unwrap();
        assert_eq!(parts.status, e["status"].as_i64().unwrap());
        assert_eq!(parts.executable, text(&e["executable"]));
        assert_eq!(parts.full_command, text(&e["command"]));
        assert_eq!(
            parts.shell,
            input["command_info"]["shell"].as_str().unwrap_or("")
        );
        assert_eq!(parts.stdout, text(&e["stdout"]));
        assert_eq!(parts.error_output, text(&e["error_output"]));
        assert_eq!(float_s(Some(parts.duration)), e["duration"]);
        assert_eq!(parts.formatted_duration, r.metadata.formatted_duration());
    }
}

#[test]
fn result_without_times_cannot_project_to_success_parts() {
    let r = ExecutionResult::new(
        CommandInfo {
            executable: "x".into(),
            arguments: vec![],
            full_command: "x".into(),
            shell: None,
            tool_name: None,
            command_name: None,
        },
        Output::new("", "", 0),
        ExecutionMetadata::new(None, None, None),
    );
    let err = ExecutionResultParts::try_from(&r).unwrap_err();
    assert_eq!(err.kind, ModelErrorKind::NoMethod);
}

#[test]
fn non_finite_timeout_fails_json_like_the_gem() {
    let rows = golden("json_errors");
    assert_eq!(rows.len(), 3);
    for row in &rows {
        let timeout: f64 = text(&row["timeout"]).parse().unwrap();
        let utc = |seconds| Time::new(seconds, 0, 0, true);
        let r = ExecutionResult::new(
            CommandInfo {
                executable: "x".into(),
                arguments: vec![],
                full_command: "x".into(),
                shell: None,
                tool_name: None,
                command_name: None,
            },
            Output::new("", "", 0),
            ExecutionMetadata::new(Some(utc(0)), Some(utc(2)), Some(Seconds::Float(timeout))),
        );
        assert_eq!(r.metadata.is_timed_out(), row["timed_out"] == true);
        let err = r
            .to_json()
            .expect_err("non-finite timeout must not serialise");
        assert_eq!(err.ruby_class(), text(&row["expect"]["class"]));
        assert_eq!(err.message, text(&row["expect"]["message"]));
    }
}

/// A stand-in for `Ukiryu::Models::ExitCodes#meaning` (custom, then
/// standard, keyed by `code.to_s`) so the mapping is exercised without the
/// models module.
struct Table {
    standard: HashMap<String, String>,
    custom: HashMap<String, String>,
}

impl ExitCodeTable for Table {
    fn meaning(&self, code: i64) -> Option<String> {
        let key = code.to_string();
        self.custom
            .get(&key)
            .or_else(|| self.standard.get(&key))
            .cloned()
    }
}

fn table(v: &Value) -> Option<Table> {
    if v.is_null() {
        return None;
    }
    let map = |m: &Value| {
        m.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), text(v).to_owned()))
            .collect()
    };
    Some(Table {
        standard: map(&v["standard"]),
        custom: map(&v["custom"]),
    })
}

fn model_table(v: &Value) -> Option<ExitCodes> {
    if v.is_null() {
        return None;
    }
    let hash = |m: &Value| {
        RubyValue::Hash(
            m.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (RubyValue::str(k.clone()), RubyValue::str(text(v))))
                .collect(),
        )
    };
    Some(ExitCodes {
        standard: Attr::Set(hash(&v["standard"])),
        custom: Attr::Set(hash(&v["custom"])),
    })
}

#[test]
fn exit_code_mapping_matches_oracle() {
    check_exit_code_mapping(table);
}

#[test]
fn exit_code_mapping_with_models_exit_codes_matches_oracle() {
    check_exit_code_mapping(model_table);
}

fn check_exit_code_mapping<T: ExitCodeTable>(build: fn(&Value) -> Option<T>) {
    let rows = golden("exit_code_mapping");
    assert_eq!(rows.len(), 405);
    let tables: HashMap<String, Option<T>> = rows[0]["tables"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), build(v)))
        .collect();
    let mut mapped = 0;
    let mut lookup_short_circuits = 0;
    for row in &rows[1..] {
        let input = &row["input"];
        if input.get("lookup").is_some() {
            // Tool/command lookup returned nil before any table was read:
            // register integration, not this mapping.
            assert!(row["expect"]["meaning"].is_null());
            lookup_short_circuits += 1;
            continue;
        }
        if input.is_null() {
            continue;
        }
        let status = input["status"].as_i64().unwrap();
        let command = if input["command_present"] == true {
            tables[text(&input["command_table"])].as_ref()
        } else {
            None
        };
        let profile = tables[text(&input["profile_table"])].as_ref();
        let meaning = exit_code_meaning(status, command, profile);
        let e = &row["expect"];
        assert_eq!(meaning.as_deref(), e["meaning"].as_str(), "{input}");
        assert_eq!(
            format_meaning(meaning.as_deref()),
            text(&e["format_meaning"]),
            "{input}"
        );
        // `success` stays `status.zero?` whatever the table says.
        assert_eq!(
            Output::new("", "", status).is_success(),
            e["success"] == true,
            "{input}"
        );
        mapped += 1;
    }
    assert_eq!(mapped, 400);
    assert_eq!(lookup_short_circuits, 3);
    // Source defect kept as evidence: Response::Base#timed_out? raises.
    let last = rows.last().unwrap();
    assert_eq!(last["timed_out"]["class"], "NoMethodError");
}

#[test]
fn executor_real_runs_match_message_builders() {
    let rows = golden("executor_real");
    assert_eq!(rows.len(), 4);
    let mut timeouts = 0;
    for row in &rows {
        let e = &row["expect"];
        match text(&row["case"]) {
            "timeout" => {
                let t = seconds(&row["input"]["timeout"]).unwrap();
                let exe = text(&row["input"]["executable"]);
                assert_eq!(e["class"], "Ukiryu::Errors::TimeoutError");
                assert_eq!(
                    ukiryu::errors::command_timed_out_message(t, exe),
                    text(&e["message"])
                );
                // The executor raises without `timeout:`; the field is nil.
                assert!(e["timeout_field"].is_null());
                timeouts += 1;
            }
            "nonzero" => {
                assert_eq!(e["class"], "Ukiryu::Errors::ExecutionError");
                assert_eq!(e["has_result"], false);
                // Inputs: what `sh -c 'echo out; echo err >&2; exit 42'` writes,
                // and the bash join of its argv.
                assert_eq!(
                    ukiryu::errors::execution_failed_message(
                        "sh",
                        "'sh' '-c' 'echo out; echo err >&2; exit 42'",
                        Some("out\n"),
                        Some("err\n"),
                        42
                    ),
                    text(&e["message"])
                );
            }
            "allow_failure" => {
                let o = Output::new(text(&e["stdout"]), "", e["status"].as_i64().unwrap());
                assert_eq!(o.is_success(), e["success"] == true);
                assert_eq!(o.stdout_lines(), vec!["a", "b"]);
                assert_eq!(e["duration_class"], "Float");
            }
            other => panic!("unknown case {other}"),
        }
    }
    assert_eq!(timeouts, 2);
}
