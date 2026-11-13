//! The response formats over the crate's own models.
//!
//! `SuccessResponse`/`ErrorResponse` implement `render::Renderable` and the
//! unified `types::Value` implements `render::InspectValue`
//! (`src/render/adapters.rs`). The expected bytes come from the pinned
//! Ruby oracle (ukiryu/ukiryu ab6df353, Ruby 4.0.1):
//!
//! - `render_oracle/golden.json` and `render_oracle/json_nonfinite.json`
//!   (the K22-RENDER captures): every oracle response is loaded into the
//!   native model and rendered in every captured format/colour/paint/file
//!   combination.
//! - `render_models_data/values.json` (`capture/values.rb` of
//!   F6-RENDER-ADAPTERS): `SuccessResponse.from_result` over request values
//!   of every class the unified value holds (Symbol, Time, Integers past
//!   i64/i128, Floats, ordered Hashes, nil, booleans), over YAML-tagged
//!   values loaded as the register loads YAML, and both dry-run variants.
//!
//! JSON, table, raw and dry-run text is compared byte for byte with Ruby.
//! YAML is the Yeptris document of the pinned ukiryu#27 settings: its
//! exact bytes are pinned (`render_oracle/yeptris.json` for the oracle
//! responses, `render_models_data/yeptris.json` here) and its data is
//! compared with Psych's load in the type-sensitive canonical encoding of
//! `render_oracle/support.rs`. No Psych byte equality is claimed for YAML.
#[path = "render_oracle/support.rs"]
mod support;

use num_bigint::BigInt;
use serde_json::{json, Map, Value as Json};
use support::{canon, decode, golden, paint, render_case_id, s};
use ukiryu::models::{ErrorResponse, ExecutionResultParts, Model, SuccessResponse};
use ukiryu::render::{
    output_response, render_dry_run, to_json_pretty, to_yaml, to_yaml_tree, DryRunStyle,
    OutputFormat, OutputWrite, PaintSupport, RenderErrorKind, RenderSettings, Renderable,
};
use ukiryu::types::{Time, Value};

const ORACLE_SHA: &str = "ab6df353c6dc2d7faf7817a86aafcafa82579896";

fn values() -> Json {
    serde_json::from_str(include_str!("render_models_data/values.json")).unwrap()
}

fn render_yeptris() -> Json {
    serde_json::from_str(include_str!("render_oracle/yeptris.json")).unwrap()
}

fn probes() -> Json {
    serde_json::from_str(include_str!("render_oracle/probes.json")).unwrap()
}

/// The capture's input encoding: `$int`, `$float` (bits), `$sym`, `$time`,
/// `$map` (ordered pairs), arrays and JSON scalars.
fn ruby(v: &Json) -> Value {
    match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::String(t) => Value::str(t.clone()),
        Json::Array(items) => Value::Array(items.iter().map(ruby).collect()),
        Json::Object(o) => {
            if let Some(i) = o.get("$int") {
                Value::Integer(s(i).parse::<BigInt>().unwrap())
            } else if let Some(f) = o.get("$float") {
                Value::Float(f64::from_bits(u64::from_str_radix(s(f), 16).unwrap()))
            } else if let Some(n) = o.get("$sym") {
                Value::sym(s(n))
            } else if let Some(t) = o.get("$time") {
                Value::Time(Time {
                    seconds: t["seconds"].as_i64().unwrap(),
                    nanos: t["nanos"].as_u64().unwrap() as u32,
                    utc_offset: t["utc_offset"].as_i64().unwrap() as i32,
                    utc: t["utc"].as_bool().unwrap(),
                })
            } else if let Some(pairs) = o.get("$map") {
                Value::Hash(
                    pairs
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| (ruby(&p[0]), ruby(&p[1])))
                        .collect(),
                )
            } else {
                panic!("unknown encoding {v}")
            }
        }
        Json::Number(n) => panic!("bare number {n}"),
    }
}

fn pairs(v: &Json) -> Vec<(String, Value)> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| (s(&p[0]).to_owned(), ruby(&p[1])))
        .collect()
}

fn load_canon(yaml: &str) -> Json {
    canon(&yeptris::from_str::<yeptris::Value>(yaml).expect("yeptris loads its own yaml"))
}

/// `(stdout, stderr, file)` of the writes `output_response` returns.
fn run<R: Renderable + ?Sized>(
    response: &R,
    format: OutputFormat,
    file: Option<&str>,
    settings: &RenderSettings,
) -> Json {
    let (mut out, mut err, mut contents) = (String::new(), String::new(), Json::Null);
    for w in output_response(response, format, file, settings).unwrap() {
        match w {
            OutputWrite::Stdout(t) => out.push_str(&t),
            OutputWrite::Stderr(t) => err.push_str(&t),
            OutputWrite::File { contents: c, .. } => contents = Json::String(c),
        }
    }
    json!({ "stdout": out, "stderr": err, "file": contents })
}

fn row_settings(row: &Json) -> RenderSettings {
    RenderSettings {
        use_color: row["use_color"].as_bool(),
        stdout_tty: row["tty"].as_bool().unwrap(),
        paint: paint(s(&row["paint"])),
    }
}

/// Ruby's plain settings of the values capture: `use_color: false`, no
/// TTY, the paint gem not loadable.
const PLAIN: RenderSettings = RenderSettings {
    use_color: Some(false),
    stdout_tty: false,
    paint: PaintSupport::UNAVAILABLE,
};

enum Native {
    Success(SuccessResponse),
    Error(ErrorResponse),
}

/// The native model of an oracle response. `error-nil` is
/// `ErrorResponse.new` (K22 capture.rb): every attribute keeps its
/// unserialized default, which a loaded document cannot express (loading
/// fills defaults as assigned values), so it is built the same way here.
fn native(id: &str, class: &str, yaml: &str) -> Native {
    match (class, id) {
        ("ErrorResponse", "error-nil") => Native::Error(ErrorResponse::default()),
        ("SuccessResponse", _) => Native::Success(SuccessResponse::from_yaml(yaml).unwrap()),
        ("ErrorResponse", _) => Native::Error(ErrorResponse::from_yaml(yaml).unwrap()),
        (other, _) => panic!("{other}"),
    }
}

impl Native {
    fn json(&self) -> String {
        match self {
            Native::Success(r) => to_json_pretty(r),
            Native::Error(r) => to_json_pretty(r),
        }
        .unwrap()
    }

    fn yaml(&self) -> String {
        match self {
            Native::Success(r) => to_yaml(r),
            Native::Error(r) => to_yaml(r),
        }
        .unwrap()
    }

    fn render(&self, format: OutputFormat, file: Option<&str>, settings: &RenderSettings) -> Json {
        match self {
            Native::Success(r) => run(r, format, file, settings),
            Native::Error(r) => run(r, format, file, settings),
        }
    }
}

#[test]
fn values_capture_is_the_pinned_oracle() {
    let v = values();
    assert_eq!(s(&v["provenance"]["oracle_sha"]), ORACLE_SHA);
    assert_eq!(s(&v["provenance"]["ruby"]), "4.0.1");
    assert_eq!(v["values"].as_array().unwrap().len(), 19);
    assert_eq!(v["tagged"].as_array().unwrap().len(), 8);
    assert_eq!(v["dry_runs"].as_array().unwrap().len(), 4);
}

/// Every oracle response, loaded into the native model, renders the
/// captured JSON/table/raw bytes and the pinned Yeptris YAML in every
/// captured combination.
#[test]
fn oracle_responses_render_through_the_native_models() {
    let g = golden();
    let pinned = render_yeptris();
    let mut rows = 0;
    for r in g["responses"].as_array().unwrap() {
        let id = s(&r["id"]);
        let model = native(id, s(&r["class"]), s(&r["yaml"]));
        assert_eq!(model.json(), s(&r["json"]), "{id}: json gem bytes");
        let yaml = model.yaml();
        assert_eq!(yaml, model.yaml(), "{id}: stable");
        assert_eq!(
            yaml,
            s(&pinned["responses"][id]),
            "{id}: pinned Yeptris bytes"
        );
        assert_eq!(
            load_canon(&yaml),
            load_canon(s(&r["yaml"])),
            "{id}: same data as the Psych document"
        );
        for row in g["renders"].as_array().unwrap() {
            if s(&row["response"]) != id {
                continue;
            }
            let format = OutputFormat::parse(s(&row["format"])).unwrap();
            let file = row["output_file"].as_bool().unwrap().then_some("${OUT}");
            let got = model.render(format, file, &row_settings(row));
            let case = render_case_id(row);
            if format == OutputFormat::Yaml {
                assert_eq!(
                    got, pinned["renders"][&case],
                    "{case}: pinned Yeptris render"
                );
            } else {
                assert_eq!(s(&got["stdout"]), s(&row["stdout"]), "{case}: stdout");
                assert_eq!(s(&got["stderr"]), s(&row["stderr"]), "{case}: stderr");
                assert_eq!(got["file"], row["file"], "{case}: file");
            }
            rows += 1;
        }
    }
    assert_eq!(rows, 429);
}

/// A NaN or infinite `duration_seconds` raises `JSON::GeneratorError`
/// through `SuccessResponse#to_json(pretty: true)`; YAML writes it.
#[test]
fn non_finite_duration_raises_in_json_and_is_written_in_yaml() {
    let data: Json =
        serde_json::from_str(include_str!("render_oracle/json_nonfinite.json")).unwrap();
    let mut current = Map::new();
    for (word, case) in data["cases"].as_object().unwrap() {
        let psych_yaml = s(&case["response_to_yaml"]);
        let model = SuccessResponse::from_yaml(psych_yaml).unwrap();
        let x = model.metadata.as_ref().unwrap().duration_seconds.unwrap();
        assert!(!x.is_finite(), "{word}");
        let err = to_json_pretty(&model).unwrap_err();
        assert_eq!(
            s(&case["response_to_json"]["class"]),
            "JSON::GeneratorError"
        );
        assert_eq!(err.kind(), RenderErrorKind::Generator, "{word}");
        assert_eq!(err.message(), s(&case["response_to_json"]["message"]));
        let yaml = to_yaml(&model).unwrap();
        assert_eq!(load_canon(&yaml), load_canon(psych_yaml), "{word}: data");
        current.insert(word.clone(), Json::String(yaml));
    }
    check_pinned("nonfinite", Json::Object(current));
}

fn parts(p: &Json) -> ExecutionResultParts {
    ExecutionResultParts {
        status: p["status"].as_i64().unwrap(),
        executable: s(&p["executable"]).to_owned(),
        full_command: s(&p["full_command"]).to_owned(),
        shell: s(&p["shell"]).to_owned(),
        stdout: s(&p["stdout"]).to_owned(),
        error_output: s(&p["error_output"]).to_owned(),
        started_at_iso8601: s(&p["started_at"]).to_owned(),
        finished_at_iso8601: s(&p["finished_at"]).to_owned(),
        duration: p["duration"].as_f64().unwrap(),
        formatted_duration: s(&p["formatted_duration"]).to_owned(),
    }
}

/// One captured `from_result` row: JSON, table and raw bytes equal Ruby's,
/// the YAML loads to Psych's data; returns the Yeptris YAML.
fn check_response_row(
    row: &Json,
    params: &[(String, Value)],
    result: &ExecutionResultParts,
) -> String {
    let id = s(&row["id"]);
    let model = SuccessResponse::from_result(result, params, None, None).unwrap();
    assert_eq!(
        to_json_pretty(&model).unwrap(),
        s(&row["json"]),
        "{id}: json"
    );
    let yaml = to_yaml(&model).unwrap();
    assert_eq!(yaml, to_yaml(&model).unwrap(), "{id}: stable");
    assert_eq!(load_canon(&yaml), row["psych_load"], "{id}: yaml data");
    for (name, format) in [("table", OutputFormat::Table), ("raw", OutputFormat::Raw)] {
        let got = run(&model, format, None, &PLAIN);
        let want = &row["renders"][name];
        assert_eq!(s(&got["stdout"]), s(&want["stdout"]), "{id} {name}: stdout");
        assert_eq!(s(&got["stderr"]), s(&want["stderr"]), "{id} {name}: stderr");
    }
    yaml
}

/// Symbol, Time, big Integer, Float, ordered Hash and nested values reach
/// the response through `Arguments.from_params`/`stringify_value`.
#[test]
fn request_values_of_every_class_render_like_ruby() {
    let v = values();
    let result = parts(&v["provenance"]["result"]);
    let mut current = Map::new();
    for row in v["values"].as_array().unwrap() {
        let yaml = check_response_row(row, &pairs(&row["params"]), &result);
        current.insert(s(&row["id"]).to_owned(), Json::String(yaml));
    }
    check_pinned("values", Json::Object(current));
}

/// YAML-tagged request values: the YAML is loaded through the crate's
/// pipeline (Yeptris tree -> `Value::from_yaml_value`, which applies each
/// tag with `types::value::apply_yaml_tag` -> `Utils.symbolize_keys`), as
/// the register loads it with Psych. The loaded value equals Psych's (a
/// tag is applied or dropped, never kept as a one-entry Hash) and the
/// response renders Ruby's bytes.
#[test]
fn tagged_request_values_render_like_ruby() {
    let v = values();
    let result = parts(&v["provenance"]["result"]);
    let mut current = Map::new();
    for row in v["tagged"].as_array().unwrap() {
        let id = s(&row["id"]);
        let tree: yeptris::Value = yeptris::from_str(s(&row["yaml_input"])).unwrap();
        let loaded = Value::from_yaml_value(&tree).unwrap();
        assert_eq!(loaded, ruby(&row["loaded"]), "{id}: loaded like Psych");
        let Value::Hash(entries) = loaded.symbolize_keys() else {
            panic!("{id}: a Hash document")
        };
        let params: Vec<(String, Value)> = entries
            .into_iter()
            .map(|(k, val)| (k.to_sym().unwrap(), val))
            .collect();
        assert_eq!(params, pairs(&row["params"]), "{id}: symbolized params");
        let yaml = check_response_row(row, &params, &result);
        current.insert(id.to_owned(), Json::String(yaml));
    }
    check_pinned("tagged", Json::Object(current));
}

/// Both `say_dry_run` variants print every unified value with Ruby
/// `#inspect` (Symbols, Times with sub-seconds and offsets, Integers past
/// i64/i128, Float spellings, Ruby 3.4 Hash style in insertion order).
#[test]
fn dry_run_inspects_the_unified_value_like_ruby() {
    let v = values();
    for row in v["dry_runs"].as_array().unwrap() {
        let style = match s(&row["style"]) {
            "run_command" => DryRunStyle::Exec,
            "response_formatter" => DryRunStyle::RunFile,
            other => panic!("{other}"),
        };
        let args = pairs(&row["arguments"]);
        let got = render_dry_run(style, "jq", "process", &args, row["use_color"].as_bool());
        assert_eq!(
            got,
            s(&row["stdout"]),
            "{style:?} color={}",
            row["use_color"]
        );
        assert_eq!(s(&row["stderr"]), "");
    }
}

fn probe_doc(value: &Json) -> yeptris::Value {
    yeptris::Value::Mapping(yeptris::Mapping(vec![(
        yeptris::Value::String("value".into()),
        decode(value),
    )]))
}

/// The two tagged render probes (`render_oracle/probes.json`) keep their
/// tags when the tree is emitted by `to_yaml_tree`: Yeptris reads back the
/// identical tagged tree. Every untagged probe is written with the same
/// bytes as `to_yaml`.
#[test]
fn tagged_trees_round_trip_through_to_yaml_tree() {
    let p = probes();
    let mut tagged = Map::new();
    for v in p["values"].as_array().unwrap() {
        let id = s(&v["id"]);
        let doc = probe_doc(&v["value"]);
        let tree = to_yaml_tree(&doc).map_err(|e| e.to_string());
        assert_eq!(
            tree,
            to_yaml_tree(&doc).map_err(|e| e.to_string()),
            "{id}: stable"
        );
        if v["value"].get("$tag").is_some() {
            let yaml = tree.unwrap();
            let back: yeptris::Value = yeptris::from_str(&yaml).unwrap();
            assert_eq!(back, doc, "{id}: tags survive\n{yaml}");
            tagged.insert(id.to_owned(), Json::String(yaml));
        } else {
            let serde = to_yaml(&doc).map_err(|e| e.to_string());
            assert_eq!(tree, serde, "{id}: same result as to_yaml");
        }
    }
    assert_eq!(tagged.len(), 2);
    check_pinned("tagged_trees", Json::Object(tagged));
}

/// Documented limit of the serde route (`to_yaml` of a `Serialize` value):
/// serde has no tag channel and Yeptris' `Serialize for Value` writes a
/// tagged node as a one-entry mapping (yeptris-rs src/ser.rs, serde_yaml's
/// convention). No native model reaches this: their values hold no tags.
/// The same tagged trees loaded into the unified value have their tags
/// applied as Psych applies them.
#[test]
fn serde_route_has_no_tag_channel_and_models_apply_tags() {
    let p = probes();
    let applied = [("tagged-local", "x"), ("tagged-str", "123")];
    for (id, psych) in applied {
        let v = p["values"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| s(&v["id"]) == id)
            .unwrap();
        let doc = probe_doc(&v["value"]);
        let tag = s(&v["value"]["$tag"]);
        let via_serde: yeptris::Value = yeptris::from_str(&to_yaml(&doc).unwrap()).unwrap();
        let one_entry = yeptris::Value::Mapping(yeptris::Mapping(vec![(
            yeptris::Value::String("value".into()),
            yeptris::Value::Mapping(yeptris::Mapping(vec![(
                yeptris::Value::String(tag.into()),
                decode(&v["value"]["value"]),
            )])),
        )]));
        assert_eq!(via_serde, one_entry, "{id}: serde route (Yeptris ser.rs)");
        let loaded = Value::from_yaml_value(&doc).unwrap();
        assert_eq!(
            loaded,
            Value::Hash(vec![(Value::str("value"), Value::str(psych))]),
            "{id}: tag applied"
        );
    }
}

/// `render_models_data/yeptris.json`: the exact Yeptris YAML of this
/// file's cases (ukiryu#27 settings). On a difference the current section
/// is printed with the `RMG|` prefix so the file can be regenerated from
/// the CI log.
fn check_pinned(section: &str, current: Json) {
    let want: Json = match std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/render_models_data/yeptris.json"),
    ) {
        Ok(t) => serde_json::from_str(&t).unwrap(),
        Err(_) => Json::Null,
    };
    if want[section] != current {
        let text = serde_json::to_string_pretty(&json!({ section: current })).unwrap();
        for line in text.lines() {
            println!("RMG|{line}");
        }
    }
    for (id, yaml) in current.as_object().unwrap() {
        assert!(!s(yaml).contains('\r'), "{section} {id}: no CR");
    }
    assert_eq!(want[section], current, "{section}: pinned Yeptris bytes");
}
