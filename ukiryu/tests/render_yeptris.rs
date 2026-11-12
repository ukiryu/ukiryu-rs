//! Exact Yeptris bytes of the `yaml` format and their cross-engine meaning.
//!
//! The `yaml` format is emitted by Yeptris with the settings ukiryu#27
//! pins (`render::yaml_emit_options`: unbounded width, no block folding,
//! explicit `---`). Its bytes are not Psych's. This file pins them:
//!
//! - `render_oracle/yeptris.json`: the exact output of every captured
//!   render case (by case ID), every oracle response document and every
//!   authored probe value, plus the Yeptris load of each probe and
//!   snippet. Each entry traces to ukiryu#27. The same file is checked on
//!   every CI platform, so a pass on all of them is the cross-platform
//!   byte-stability check; every render is also repeated in-process.
//! - `render_oracle/psych_cross.json`: what Psych (the pinned oracle
//!   runtime) makes of those exact Yeptris bytes, compared in the
//!   type-sensitive canonical encoding of `render_oracle/support.rs`,
//!   with the style difference table (Psych vs Yeptris) per case.
//!
//! The Psych observations in `render_oracle/golden.json` stay frozen and
//! are never rewritten from Yeptris output.
//!
//! When `yeptris.json` is absent or differs, the test prints the current
//! values as `YG|` lines so the file can be regenerated from the CI log.
#[path = "render_oracle/support.rs"]
mod support;

use serde_json::{json, Map, Value as Json};
use std::collections::BTreeSet;
use std::path::PathBuf;
use support::{canon, decode, golden, paint, render_case_id, responses, s};
use ukiryu::render::{output_response, to_yaml, OutputFormat, OutputWrite, RenderSettings};
use yeptris::{Mapping, Value};

const TRACE: &str = "ukiryu/ukiryu#27: yaml format emitted by Yeptris with pinned EmitOptions \
(canonical false, indent 2, explicit document start, LineWidth::Unbounded, block_fold false)";

fn data_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/render_oracle")
        .join(name)
}

fn probes() -> Json {
    serde_json::from_str(include_str!("render_oracle/probes.json")).unwrap()
}

fn load_canon(yaml: &str) -> Json {
    match yeptris::from_str::<Value>(yaml) {
        Ok(v) => canon(&v),
        Err(e) => json!({ "$error": e.to_string() }),
    }
}

fn probe_doc(value: &Json) -> Value {
    Value::Mapping(Mapping(vec![(
        Value::String("value".into()),
        decode(value),
    )]))
}

fn render_row(row: &Json, oracle: &support::Oracle) -> Json {
    let settings = RenderSettings {
        use_color: row["use_color"].as_bool(),
        stdout_tty: row["tty"].as_bool().unwrap(),
        paint: paint(s(&row["paint"])),
    };
    let format = OutputFormat::parse(s(&row["format"])).unwrap();
    let file = row["output_file"].as_bool().unwrap().then_some("${OUT}");
    let (mut out, mut err, mut contents) = (String::new(), String::new(), Json::Null);
    for w in output_response(oracle, format, file, &settings).unwrap() {
        match w {
            OutputWrite::Stdout(t) => out.push_str(&t),
            OutputWrite::Stderr(t) => err.push_str(&t),
            OutputWrite::File { contents: c, .. } => contents = Json::String(c),
        }
    }
    json!({ "stdout": out, "stderr": err, "file": contents })
}

/// Every value the golden file pins, computed now.
fn compute() -> Json {
    let g = golden();
    let all = responses(&g);
    let mut docs = Map::new();
    for (id, oracle, _, _) in &all {
        docs.insert(id.clone(), Json::String(to_yaml(oracle).unwrap()));
    }
    let mut renders = Map::new();
    for row in g["renders"].as_array().unwrap() {
        let id = s(&row["response"]);
        let oracle = &all.iter().find(|r| r.0 == id).unwrap().1;
        let case = render_case_id(row);
        let value = render_row(row, oracle);
        assert!(
            renders.insert(case.clone(), value).is_none(),
            "duplicate {case}"
        );
    }
    let p = probes();
    let mut probe_yaml = Map::new();
    let mut probe_loads = Map::new();
    for v in p["values"].as_array().unwrap() {
        let id = s(&v["id"]).to_owned();
        match to_yaml(&probe_doc(&v["value"])) {
            Ok(yaml) => {
                probe_loads.insert(id.clone(), load_canon(&yaml));
                probe_yaml.insert(id, Json::String(yaml));
            }
            Err(e) => {
                probe_yaml.insert(id, json!({ "$error": e.to_string() }));
            }
        }
    }
    let mut snippets = Map::new();
    for sn in p["snippets"].as_array().unwrap() {
        snippets.insert(s(&sn["id"]).to_owned(), load_canon(s(&sn["yaml"])));
    }
    json!({
        "trace": TRACE,
        "settings": {
            "canonical": false, "indent": 2, "explicit_document_start": true,
            "line_width": "unbounded", "block_fold": false
        },
        "responses": docs,
        "renders": renders,
        "probe_yaml": probe_yaml,
        "probe_yeptris_loads": probe_loads,
        "snippet_yeptris_loads": snippets,
    })
}

fn dump(current: &Json) {
    for line in serde_json::to_string_pretty(current).unwrap().lines() {
        println!("YG|{line}");
    }
}

#[test]
fn yeptris_bytes_match_the_pinned_goldens() {
    let current = compute();
    assert_eq!(current, compute(), "repeat run gives the same bytes");
    for section in ["responses", "renders", "probe_yaml"] {
        for (id, v) in current[section].as_object().unwrap() {
            if let Some(t) = v.as_str() {
                assert!(!t.contains('\r'), "{section} {id}: no CR on any platform");
            }
        }
    }
    let text = match std::fs::read_to_string(data_path("yeptris.json")) {
        Ok(t) => t,
        Err(_) => {
            dump(&current);
            panic!("render_oracle/yeptris.json is missing; current values printed as YG| lines");
        }
    };
    let want: Json = serde_json::from_str(&text).unwrap();
    if want != current {
        dump(&current);
    }
    assert_eq!(want["trace"], current["trace"]);
    assert_eq!(want["settings"], current["settings"]);
    for section in [
        "responses",
        "renders",
        "probe_yaml",
        "probe_yeptris_loads",
        "snippet_yeptris_loads",
    ] {
        let (w, c) = (
            want[section].as_object().unwrap(),
            current[section].as_object().unwrap(),
        );
        let wk: BTreeSet<&String> = w.keys().collect();
        let ck: BTreeSet<&String> = c.keys().collect();
        assert_eq!(wk, ck, "{section}: case IDs");
        for (id, value) in c {
            assert_eq!(&w[id], value, "{section} {id}");
        }
    }
    assert_eq!(want["renders"].as_object().unwrap().len(), 429);
}

/// The non-yaml formats keep the captured oracle bytes: the Yeptris
/// goldens of those rows equal `golden.json` exactly.
#[test]
fn non_yaml_render_goldens_equal_the_oracle_capture() {
    let g = golden();
    let current = compute();
    let renders = current["renders"].as_object().unwrap();
    let mut n = 0;
    for row in g["renders"].as_array().unwrap() {
        if s(&row["format"]) == "yaml" {
            continue;
        }
        let got = &renders[&render_case_id(row)];
        assert_eq!(s(&got["stdout"]), s(&row["stdout"]));
        assert_eq!(s(&got["stderr"]), s(&row["stderr"]));
        assert_eq!(got["file"], row["file"]);
        n += 1;
    }
    assert_eq!(n, 429 - 264 - 22);
}

/// The comparator is type-sensitive: the values YAML 1.1 and 1.2 disagree
/// on never compare equal.
#[test]
fn canonical_encoding_is_type_sensitive() {
    let pairs = [
        (Value::String("no".into()), Value::Bool(false)),
        (Value::String("yes".into()), Value::Bool(true)),
        (
            Value::String("123".into()),
            Value::Number(yeptris::Number::I64(123)),
        ),
        (
            Value::String("1.25".into()),
            Value::Number(yeptris::Number::F64(1.25)),
        ),
        (
            Value::Number(yeptris::Number::F64(0.0)),
            Value::Number(yeptris::Number::F64(-0.0)),
        ),
        (Value::String("~".into()), Value::Null),
        (Value::String("line\n".into()), Value::String("line".into())),
    ];
    for (a, b) in pairs {
        assert_ne!(canon(&a), canon(&b), "{a:?} vs {b:?}");
    }
    let ts = Value::String("2024-01-17T10:20:30Z".into());
    assert_ne!(
        canon(&ts),
        json!({ "$time": "2024-01-17T10:20:30.000000000Z" })
    );
}

/// Yeptris reads back every value it emits as the same canonical value,
/// except the recorded cases in `psych_cross.json`.
#[test]
fn psych_cross_engine_evidence() {
    let path = data_path("psych_cross.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        panic!("render_oracle/psych_cross.json is missing");
    };
    let cross: Json = serde_json::from_str(&text).unwrap();
    let current = compute();
    let p = probes();

    // The evidence was taken over exactly these Yeptris bytes.
    for (id, yaml) in cross["probe_yaml"].as_object().unwrap() {
        assert_eq!(&current["probe_yaml"][id], yaml, "probe {id} bytes");
    }
    for (id, yaml) in cross["response_yaml"].as_object().unwrap() {
        assert_eq!(&current["responses"][id], yaml, "response {id} bytes");
    }

    // Yeptris round trip of its own output.
    let mut yeptris_rt = Vec::new();
    for v in p["values"].as_array().unwrap() {
        let id = s(&v["id"]);
        let want = json!({ "$map": [["value", v["value"].clone()]] });
        if current["probe_yeptris_loads"][id] != want {
            yeptris_rt.push(id.to_owned());
        }
    }
    assert_eq!(
        Json::from(yeptris_rt),
        cross["verdicts"]["yeptris_roundtrip_differs"],
        "probe ids whose Yeptris round trip changes the value"
    );

    // Psych reading the Yeptris bytes of each probe.
    let mut psych_differs = Vec::new();
    for v in p["values"].as_array().unwrap() {
        let id = s(&v["id"]);
        let want = json!({ "$map": [["value", v["value"].clone()]] });
        if cross["probe_psych_loads"][id] != want {
            psych_differs.push(id.to_owned());
        }
    }
    assert_eq!(
        Json::from(psych_differs),
        cross["verdicts"]["psych_load_of_yeptris_differs"],
        "probe ids Psych reads as a different value"
    );

    // Psych reading the Yeptris response documents vs Psych's own.
    let mut response_differs = Vec::new();
    for (id, psych) in cross["response_psych_loads"].as_object().unwrap() {
        if psych["of_yeptris"] != psych["of_psych"] {
            response_differs.push(id.clone());
        }
    }
    assert_eq!(
        Json::from(response_differs),
        cross["verdicts"]["response_psych_load_differs"]
    );

    // Snippets: Yeptris (YAML 1.2) vs Psych (YAML 1.1) reading the same text.
    let mut snippet_differs = Vec::new();
    for sn in p["snippets"].as_array().unwrap() {
        let id = s(&sn["id"]);
        if current["snippet_yeptris_loads"][id] != cross["snippet_psych_loads"][id] {
            snippet_differs.push(id.to_owned());
        }
    }
    assert_eq!(
        Json::from(snippet_differs),
        cross["verdicts"]["snippet_loads_differ"]
    );

    // Every style row names a pinned case.
    for row in cross["style_differences"].as_array().unwrap() {
        let case = s(&row["case"]);
        assert!(
            current["responses"].get(case).is_some() || current["probe_yaml"].get(case).is_some(),
            "{case}"
        );
        assert_ne!(row["psych_style"], row["yeptris_style"]);
    }
}
