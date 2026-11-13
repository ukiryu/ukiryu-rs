//! Response rendering against the pinned Ruby oracle.
//!
//! `render_oracle/golden.json` is captured by running the original
//! `ResponseFormatter#output_response`, `ResponseFormatter#say_dry_run`,
//! `RunCommand#say_dry_run` and `BaseCommand#say` of ukiryu/ukiryu at
//! `ab6df353c6dc2d7faf7817a86aafcafa82579896` (Ruby 4.0.1, Psych 5.3.1,
//! libyaml 0.2.2, json 3.1.0, lutaml-model 0.8.175, paint 2.3.0); its
//! `provenance` block names the capture script. Each render ran in a forked
//! process so the Paint load state is per case.
//!
//! The responses are the oracle's own `to_yaml` documents read back through
//! Yeptris, so every format renders the same data the Ruby formatter saw.
//! JSON, table, raw and dry-run output are compared byte for byte. YAML is
//! emitted by Yeptris with the pinned ukiryu#27 settings and its bytes are
//! pinned separately (`render_yeptris.rs`); these Psych observations stay
//! frozen, the YAML document is compared with them as data, and the
//! colouring/`say` layer is compared byte for byte over Psych's own text.
use serde::{Serialize, Serializer};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::path::PathBuf;
use ukiryu::render::{
    emit_output, format_yaml_text, output_response, render_dry_run, to_json_pretty, to_yaml,
    yaml_emit_options, DryRunStyle, InspectValue, OutputFormat, OutputWrite, PaintSupport,
    RenderErrorKind, RenderSettings, Renderable, RequestSummary, ResponseView, SuccessView,
    OUTPUT_FORMATS,
};
use yeptris::{LineWidth, Value};

const ORACLE_SHA: &str = "ab6df353c6dc2d7faf7817a86aafcafa82579896";

fn golden() -> Json {
    serde_json::from_str(include_str!("render_oracle/golden.json")).expect("golden json")
}

fn s(v: &Json) -> &str {
    v.as_str().expect("string")
}

/// An oracle response: the Ruby `to_yaml` document as a Yeptris tree.
struct Oracle {
    error: bool,
    tree: Value,
}

impl Serialize for Oracle {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.tree.serialize(ser)
    }
}

fn get<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for key in path {
        let Value::Mapping(m) = cur else { return None };
        cur =
            m.0.iter()
                .find(|(k, _)| matches!(k, Value::String(s) if s == key))
                .map(|(_, v)| v)?;
    }
    Some(cur)
}

fn text(v: &Value, path: &[&str]) -> String {
    match get(v, path) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => panic!("non-string at {path:?}: {other:?}"),
    }
}

fn list<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    match get(v, path) {
        Some(Value::Sequence(items)) => items,
        _ => &[],
    }
}

impl Renderable for Oracle {
    fn view(&self) -> ResponseView {
        let t = &self.tree;
        if self.error {
            return ResponseView::Error {
                error: text(t, &["error"]),
            };
        }
        let exit_code = match get(t, &["exit_code"]) {
            Some(Value::Number(yeptris::Number::I64(i))) => Some(*i),
            None => None,
            other => panic!("exit_code {other:?}"),
        };
        let pair = |a: &Value| (text(a, &["name"]), text(a, &["value"]));
        ResponseView::Success(SuccessView {
            exit_code,
            formatted_duration: text(t, &["metadata", "formatted_duration"]),
            request: RequestSummary {
                options: list(t, &["request", "options"]).iter().map(pair).collect(),
                flags: list(t, &["request", "flags"])
                    .iter()
                    .map(|f| match f {
                        Value::String(s) => s.clone(),
                        other => panic!("flag {other:?}"),
                    })
                    .collect(),
                positional: list(t, &["request", "positional"])
                    .iter()
                    .map(|a| text(a, &["value"]))
                    .collect(),
            },
            full_command: text(t, &["command", "full_command"]),
            stdout: text(t, &["output", "stdout"]),
            stderr: text(t, &["output", "stderr"]),
        })
    }
}

fn responses(g: &Json) -> Vec<(String, Oracle, String, String)> {
    g["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let yaml = s(&r["yaml"]).to_owned();
            let tree: Value = yeptris::from_str(&yaml).expect("oracle yaml loads");
            let oracle = Oracle {
                error: s(&r["class"]) == "ErrorResponse",
                tree,
            };
            (
                s(&r["id"]).to_owned(),
                oracle,
                yaml,
                s(&r["json"]).to_owned(),
            )
        })
        .collect()
}

#[test]
fn golden_provenance_is_the_pinned_oracle() {
    let g = golden();
    assert_eq!(s(&g["provenance"]["oracle_sha"]), ORACLE_SHA);
    assert_eq!(g["responses"].as_array().unwrap().len(), 11);
    assert_eq!(g["renders"].as_array().unwrap().len(), 429);
    assert_eq!(g["dry_runs"].as_array().unwrap().len(), 22);
}

#[test]
fn output_formats_match_the_source_constant() {
    let g = golden();
    let captured: Vec<&str> = g["output_formats"]
        .as_array()
        .unwrap()
        .iter()
        .map(s)
        .collect();
    assert_eq!(OUTPUT_FORMATS.to_vec(), captured);
    for name in OUTPUT_FORMATS {
        assert_eq!(OutputFormat::parse(name).unwrap().as_str(), name);
    }
    // `error! "Invalid format: #{format}. Must be one of: #{OUTPUT_FORMATS.join(', ')}"`
    let err = OutputFormat::parse("human").unwrap_err();
    assert_eq!(
        err.message(),
        format!(
            "Invalid format: human. Must be one of: {}",
            captured.join(", ")
        )
    );
}

#[test]
fn yaml_emitter_settings_are_pinned() {
    let o = yaml_emit_options();
    assert!(!o.is_canonical());
    assert_eq!(o.get_line_width(), LineWidth::Unbounded);
    assert!(!o.is_block_fold());
    assert!(o.is_explicit_document_start());
    assert_eq!(o.get_indent(), 2);
}

#[test]
fn yaml_is_the_pinned_yeptris_document_of_the_same_data() {
    let g = golden();
    let mut byte_equal = Vec::new();
    for (id, oracle, ruby_yaml, _) in responses(&g) {
        let rust = to_yaml(&oracle).unwrap();
        // Byte-stable across repeated renders and equal to an independent
        // emit with the settings spelled out here.
        assert_eq!(rust, to_yaml(&oracle).unwrap(), "{id}: repeat render");
        let pinned = yeptris::EmitOptions::new()
            .canonical(false)
            .indent(2)
            .explicit_document_start(true)
            .line_width(LineWidth::Unbounded)
            .block_fold(false);
        let tree = yeptris::to_value(&oracle).unwrap();
        assert_eq!(
            rust,
            format!("{}\n", yeptris::to_string_with(&tree, &pinned).unwrap()),
            "{id}: pinned settings"
        );
        assert!(rust.starts_with("---"), "{id}: explicit document start");
        // The same data as Psych's document.
        let a: Value = yeptris::from_str(&ruby_yaml).unwrap();
        let b: Value = yeptris::from_str(&rust).unwrap();
        assert_eq!(
            a, b,
            "{id}: same data\n--- ruby\n{ruby_yaml}--- rust\n{rust}"
        );
        if rust == ruby_yaml {
            byte_equal.push(id);
        }
    }
    eprintln!("byte-identical to Psych: {byte_equal:?}");
}

/// Psych folds the 30-word stdout of `success-long-line` over three lines
/// at 80 columns; the pinned settings keep every scalar on one line.
#[test]
fn yaml_never_folds_long_scalars() {
    let g = golden();
    let (_, oracle, ruby_yaml, _) = responses(&g)
        .into_iter()
        .find(|r| r.0 == "success-long-line")
        .unwrap();
    assert!(ruby_yaml.contains("word12\n    word13"), "psych folded");
    let rust = to_yaml(&oracle).unwrap();
    let line = rust
        .lines()
        .find(|l| l.starts_with("  stdout: "))
        .expect("stdout line");
    assert!(line.contains("word1 ") && line.contains("word30"), "{line}");
}

/// Numeric-equality policy (ukiryu-rs#23 render contract): the json gem's
/// fpconv uses Grisu2, which returns a non-shortest digit string for a few
/// doubles; the port writes the shortest round-trip digits. Such a row
/// passes only when both texts parse to the identical `f64` bits. The
/// expected bytes stay the captured ones; this list names every gated
/// value by its bits.
const GRISU_DIGIT_POLICY: &[&str] = &["4450bb084255acee"];

#[test]
fn json_floats_match_the_json_gem_layout() {
    let g = golden();
    let mut gated = Vec::new();
    for row in g["json_floats"].as_array().unwrap() {
        let hex = s(&row["bits"]);
        let x = f64::from_bits(u64::from_str_radix(hex, 16).unwrap());
        let got = to_json_pretty(&[x]).unwrap();
        let want = s(&row["json"]);
        if got == want {
            continue;
        }
        let num = |t: &str| -> f64 { t.trim_matches(|c| "[]\n ".contains(c)).parse().unwrap() };
        assert!(
            GRISU_DIGIT_POLICY.contains(&hex),
            "{hex}: {got:?} vs captured {want:?}"
        );
        assert_eq!(num(&got).to_bits(), x.to_bits(), "{hex}: port digits");
        assert_eq!(num(want).to_bits(), x.to_bits(), "{hex}: json gem digits");
        gated.push(hex);
    }
    assert_eq!(gated, GRISU_DIGIT_POLICY);
}

/// NaN and infinities raise `JSON::GeneratorError` in the json gem, both
/// for a bare array and through `SuccessResponse#to_json(pretty: true)`.
#[test]
fn json_non_finite_floats_raise_like_the_json_gem() {
    let data: Json =
        serde_json::from_str(include_str!("render_oracle/json_nonfinite.json")).unwrap();
    let cases = data["cases"].as_object().unwrap();
    assert_eq!(cases.len(), 3);
    for (word, case) in cases {
        let x = match word.as_str() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            other => panic!("{other}"),
        };
        for call in ["pretty_generate", "response_to_json"] {
            assert_eq!(s(&case[call]["class"]), "JSON::GeneratorError");
        }
        let want = s(&case["response_to_json"]["message"]);
        let err = to_json_pretty(&[x]).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::Generator);
        assert_eq!(err.message(), s(&case["pretty_generate"]["message"]));
        // Inside a response document, as the formatter hits it.
        let doc: Value = yeptris::from_str(&format!(
            "status: success\nmetadata:\n  duration_seconds: {}\n",
            match word.as_str() {
                "NaN" => ".nan",
                "Infinity" => ".inf",
                _ => "-.inf",
            }
        ))
        .unwrap();
        let err = to_json_pretty(&doc).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::Generator);
        assert_eq!(err.message(), want);
    }
}

#[test]
fn json_matches_the_json_gem() {
    let g = golden();
    for (id, oracle, _, ruby_json) in responses(&g) {
        assert_eq!(to_json_pretty(&oracle).unwrap(), ruby_json, "{id}");
    }
}

fn paint(name: &str) -> PaintSupport {
    match name {
        "unavailable" => PaintSupport::UNAVAILABLE,
        "available" => PaintSupport::AVAILABLE,
        "loaded" => PaintSupport::LOADED,
        "loaded-mode0" => PaintSupport {
            enabled: false,
            ..PaintSupport::LOADED
        },
        other => panic!("paint state {other}"),
    }
}

#[test]
fn output_response_matches_every_captured_render() {
    let g = golden();
    let all = responses(&g);
    let by_id: HashMap<&str, &(String, Oracle, String, String)> =
        all.iter().map(|r| (r.0.as_str(), r)).collect();
    let mut count = 0;
    for row in g["renders"].as_array().unwrap() {
        let (id, oracle, ruby_yaml, _) = by_id[s(&row["response"])];
        let format = OutputFormat::parse(s(&row["format"])).unwrap();
        let settings = RenderSettings {
            use_color: row["use_color"].as_bool(),
            stdout_tty: row["tty"].as_bool().unwrap(),
            paint: paint(s(&row["paint"])),
        };
        let to_file = row["output_file"].as_bool().unwrap();
        let writes =
            output_response(oracle, format, to_file.then_some("${OUT}"), &settings).unwrap();
        let (mut out, mut err, mut file) = (String::new(), String::new(), None::<String>);
        for w in writes {
            match w {
                OutputWrite::Stdout(t) => out.push_str(&t),
                OutputWrite::Stderr(t) => err.push_str(&t),
                OutputWrite::File { path, contents } => {
                    assert_eq!(path, "${OUT}");
                    file = Some(contents);
                }
            }
        }
        let want_out = s(&row["stdout"]).to_owned();
        let want_file = row["file"].as_str().map(str::to_owned);
        if format == OutputFormat::Yaml {
            // The colouring and `say` layer, byte for byte over Psych's text.
            let file = to_file.then_some("${OUT}");
            let mut o = String::new();
            let mut f = None;
            for w in emit_output(format_yaml_text(ruby_yaml, &settings), file, &settings) {
                match w {
                    OutputWrite::Stdout(t) => o.push_str(&t),
                    OutputWrite::File { contents, .. } => f = Some(contents),
                    OutputWrite::Stderr(_) => panic!("yaml writes no stderr"),
                }
            }
            assert_eq!(o, want_out, "yaml stdout {id} {row}");
            assert_eq!(f, want_file, "yaml file {id} {row}");
            // The real path: the same layer over the pinned Yeptris text.
            let rust_yaml = to_yaml(oracle).unwrap();
            let expect = emit_output(format_yaml_text(&rust_yaml, &settings), file, &settings);
            assert_eq!(
                output_response(oracle, format, file, &settings).unwrap(),
                expect
            );
            assert!(err.is_empty());
            count += 1;
            continue;
        }
        let ctx = format!("{id} {row}");
        assert_eq!(out, want_out, "stdout {ctx}");
        assert_eq!(err, s(&row["stderr"]), "stderr {ctx}");
        assert_eq!(file, want_file, "file {ctx}");
        count += 1;
    }
    assert_eq!(count, 429);
}

/// A dry-run argument with its Ruby `inspect` taken from the oracle.
struct Captured<'a> {
    inspect: String,
    string: Option<String>,
    strings: &'a HashMap<String, String>,
}

impl InspectValue for Captured<'_> {
    fn inspect(&self) -> String {
        self.inspect.clone()
    }
    fn as_ruby_string(&self) -> Option<&str> {
        self.string.as_deref()
    }
    fn inspect_string(&self, text: &str) -> String {
        self.strings
            .get(text)
            .unwrap_or_else(|| panic!("no captured inspect for {text:?}"))
            .clone()
    }
}

fn args<'a>(rows: &Json, strings: &'a HashMap<String, String>) -> Vec<(String, Captured<'a>)> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                s(&a["key"]).to_owned(),
                Captured {
                    inspect: s(&a["inspect"]).to_owned(),
                    string: a["string"].as_str().map(str::to_owned),
                    strings,
                },
            )
        })
        .collect()
}

fn string_inspects(g: &Json) -> HashMap<String, String> {
    g["string_inspects"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), s(v).to_owned()))
        .collect()
}

#[test]
fn dry_run_matches_both_source_variants() {
    let g = golden();
    let strings = string_inspects(&g);
    for row in g["dry_runs"].as_array().unwrap() {
        let style = match s(&row["style"]) {
            "run_command" => DryRunStyle::Exec,
            "response_formatter" => DryRunStyle::RunFile,
            other => panic!("{other}"),
        };
        let got = render_dry_run(
            style,
            s(&row["tool"]),
            s(&row["command"]),
            &args(&row["arguments"], &strings),
            row["use_color"].as_bool(),
        );
        assert_eq!(got, s(&row["stdout"]), "{} {}", row["style"], row["case"]);
        assert_eq!(s(&row["stderr"]), "");
    }
}

fn corpus_dir() -> PathBuf {
    std::env::var_os("UKIRYU_CONFORMANCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../corpus/conformance"))
}

/// The corpus `cli-output` dry-run rows (captured from the Ruby CLI on
/// Linux), rendered from the argument values the original
/// `RunCommand#parse_inline_params` produced for the same argv.
#[test]
fn corpus_dry_run_rows() {
    let g = golden();
    let strings = string_inspects(&g);
    let cases = std::fs::read_to_string(corpus_dir().join("cases/cli-output.jsonl")).unwrap();
    let params = g["corpus_dry_run_params"].as_object().unwrap();
    let mut seen = 0;
    for line in cases.lines() {
        let row: Json = serde_json::from_str(line).unwrap();
        let id = s(&row["id"]);
        if !id.starts_with("cli-output.exec-dry-run-") {
            continue;
        }
        let argv: Vec<&str> = row["input"]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(s)
            .collect();
        assert_eq!(argv[0], "exec");
        let got = render_dry_run(
            DryRunStyle::Exec,
            argv[1],
            argv[2],
            &args(&params[id], &strings),
            None,
        );
        let want = &row["expect"]["ok"];
        assert_eq!(got, s(&want["stdout"]), "{id}");
        assert_eq!(s(&want["stderr"]), "");
        assert_eq!(want["exit"].as_i64(), Some(0));
        seen += 1;
    }
    assert_eq!(seen, 3, "pinned corpus dry-run row count");
}
