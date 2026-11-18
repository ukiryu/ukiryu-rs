//! Root ruling R2 comparison rules that apply to named rows only, beside
//! `migrated.json`: the Lutaml declared-field projection
//! (`projection.json`) and the emitter rows (`emitter.json`).
//!
//! Neither changes a corpus row. The projection removes, from the frozen
//! expectation, only ivars demonstrated to be Lutaml framework state, and
//! only on objects whose ivar set is exactly the declared fields plus those
//! ivars. The emitter rule compares emitted YAML by exact pinned Yeptris
//! bytes and by type-sensitive canonical equality with the frozen Psych text.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::harness::{Outcome, RowError};
use super::json::{canonical, clip, structural_eq, Oj};
use super::ops_yaml;
use super::sha256;

/// One model class: its declared attribute ivars and the framework-private
/// ivars the projection may drop.
#[derive(Clone, Debug)]
pub struct Projection {
    pub class: String,
    pub declared: Vec<String>,
    pub private: Vec<String>,
}

fn strings(v: Option<&Oj>) -> Vec<String> {
    v.and_then(Oj::arr)
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| x.str().map(str::to_string))
        .collect()
}

pub fn projections(spec: &Oj) -> Vec<Projection> {
    spec.get("classes")
        .and_then(Oj::arr)
        .unwrap_or(&[])
        .iter()
        .map(|c| Projection {
            class: c.field_str("class").unwrap_or("").to_string(),
            declared: strings(c.get("declared")),
            private: strings(c.get("framework_private")),
        })
        .collect()
}

pub fn projected_rows(spec: &Oj) -> BTreeSet<String> {
    strings(spec.get("rows")).into_iter().collect()
}

/// The frozen snapshot with framework-private ivars removed from every
/// object of a projected class. Fails unless each such object's ivar set is
/// exactly declared + private (the projection is exhaustive).
pub fn project(v: &Oj, projections: &[Projection]) -> Result<Oj, String> {
    match v {
        Oj::Arr(items) => Ok(Oj::Arr(
            items
                .iter()
                .map(|x| project(x, projections))
                .collect::<Result<_, _>>()?,
        )),
        Oj::Obj(pairs) => {
            let class = v.get("class").and_then(Oj::str);
            let spec = class.and_then(|c| projections.iter().find(|p| p.class == c));
            let mut out = Vec::new();
            for (k, x) in pairs {
                let x = project(x, projections)?;
                match (spec, k.as_str(), &x) {
                    (Some(p), "ivars", Oj::Obj(ivars)) => {
                        let have: BTreeSet<&str> = ivars.iter().map(|(n, _)| n.as_str()).collect();
                        let want: BTreeSet<&str> = p
                            .declared
                            .iter()
                            .chain(&p.private)
                            .map(String::as_str)
                            .collect();
                        if have != want {
                            return Err(format!(
                                "{} ivars {have:?} are not exactly declared + framework-private {want:?}",
                                p.class
                            ));
                        }
                        let kept = ivars
                            .iter()
                            .filter(|(n, _)| p.declared.contains(n))
                            .cloned()
                            .collect();
                        out.push((k.clone(), Oj::Obj(kept)));
                    }
                    _ => out.push((k.clone(), x)),
                }
            }
            Ok(Oj::Obj(out))
        }
        other => Ok(other.clone()),
    }
}

pub fn projection_problems(spec: &Oj, rows: &BTreeMap<&str, (&str, &Oj)>) -> Vec<String> {
    let mut problems = Vec::new();
    let ps = projections(spec);
    if ps.is_empty() {
        problems.push("projection.json: no classes".into());
    }
    for p in &ps {
        if p.class.is_empty() || p.declared.is_empty() || p.private.is_empty() {
            problems.push(format!(
                "projection {}: class, declared and framework_private are required",
                p.class
            ));
        }
        if p.private.iter().any(|x| p.declared.contains(x)) {
            problems.push(format!(
                "projection {}: a declared field is listed as private",
                p.class
            ));
        }
    }
    for key in ["evidence", "trace"] {
        if spec.get(key).and_then(Oj::str).is_none_or(str::is_empty) {
            problems.push(format!("projection.json: missing {key}"));
        }
    }
    for id in projected_rows(spec) {
        let Some((op, expect)) = rows.get(id.as_str()) else {
            problems.push(format!("projection {id}: no such corpus row"));
            continue;
        };
        if *op != "yaml.model_load" {
            problems.push(format!("projection {id}: op {op} is not yaml.model_load"));
        }
        match project(expect, &ps) {
            Err(e) => problems.push(format!("projection {id}: {e}")),
            Ok(p) if structural_eq(&p, expect) => {
                problems.push(format!("projection {id}: removes nothing"))
            }
            Ok(_) => {}
        }
    }
    problems
}

/// A pinned Yeptris emission for one `yaml.lutaml_roundtrip` row.
pub struct EmitterPin {
    pub golden: PathBuf,
}

pub fn emitter_pins(spec: &Oj, dir: &Path) -> BTreeMap<String, EmitterPin> {
    spec.get("rows")
        .and_then(Oj::arr)
        .unwrap_or(&[])
        .iter()
        .filter_map(|r| {
            Some((
                r.field_str("id").ok()?.to_string(),
                EmitterPin {
                    golden: dir.join(r.field_str("golden").ok()?),
                },
            ))
        })
        .collect()
}

pub fn emitter_problems(spec: &Oj, dir: &Path, rows: &BTreeMap<&str, (&str, &Oj)>) -> Vec<String> {
    let mut problems = Vec::new();
    for r in spec.get("rows").and_then(Oj::arr).unwrap_or(&[]) {
        let id = r.field_str("id").unwrap_or("<no id>");
        match rows.get(id) {
            Some((op, _)) if *op == "yaml.lutaml_roundtrip" => {}
            Some((op, _)) => problems.push(format!(
                "emitter {id}: op {op} is not yaml.lutaml_roundtrip"
            )),
            None => problems.push(format!("emitter {id}: no such corpus row")),
        }
        if !r.field_str("trace").unwrap_or("").contains('#') {
            problems.push(format!("emitter {id}: trace must cite the governing issue"));
        }
        let pinned = r.field_str("golden_sha256").unwrap_or("");
        let Ok(golden) = r.field_str("golden") else {
            problems.push(format!("emitter {id}: missing golden path"));
            continue;
        };
        match std::fs::read(dir.join(golden)) {
            Ok(bytes) => {
                let got = sha256::hex(&bytes);
                if got != pinned {
                    problems.push(format!(
                        "emitter {id}: golden sha256 {got} != pinned {pinned:?}"
                    ));
                }
            }
            Err(_) if !pinned.is_empty() => {
                problems.push(format!("emitter {id}: golden {golden} missing but pinned"))
            }
            Err(_) => {}
        }
    }
    problems
}

/// Type-sensitive canonical form of YAML text through the core reader.
fn canonical_yaml(text: &str) -> Result<Oj, RowError> {
    ops_yaml::load_text(text)
}

/// The emitter rule for one `yaml.lutaml_roundtrip` row: the outcome is
/// `{emitted, reparsed_equal}`; `reparsed_equal` equals the frozen value;
/// `emitted` loads to the same typed structure as the frozen Psych text and
/// equals the pinned Yeptris bytes exactly. `golden` is `None` when no
/// bytes are pinned yet; the row then fails and reports the bytes to pin.
pub fn emitter_mismatch(expect: &Oj, golden: Option<&[u8]>, outcome: &Outcome) -> Option<String> {
    let Some(frozen) = expect.get("ok") else {
        return Some("emitter rule needs an ok expectation".into());
    };
    let got = match outcome {
        Ok(v) => v,
        Err(e) => {
            return Some(format!(
                "expected ok, got error {}: {}",
                e.class,
                clip(&e.message, 300)
            ))
        }
    };
    let keys = |v: &Oj| -> BTreeSet<String> {
        match v {
            Oj::Obj(p) => p.iter().map(|(k, _)| k.clone()).collect(),
            _ => BTreeSet::new(),
        }
    };
    if keys(got) != keys(frozen) {
        return Some(format!(
            "emitter outcome keys differ: {}",
            clip(&canonical(got), 300)
        ));
    }
    if got.get("reparsed_equal") != frozen.get("reparsed_equal") {
        return Some("reparsed_equal differs from the frozen value".into());
    }
    let (Some(emitted), Some(psych)) = (
        got.get("emitted").and_then(Oj::str),
        frozen.get("emitted").and_then(Oj::str),
    ) else {
        return Some("emitted must be a string".into());
    };
    match (canonical_yaml(emitted), canonical_yaml(psych)) {
        (Ok(a), Ok(b)) if structural_eq(&a, &b) => {}
        (Ok(a), Ok(b)) => {
            return Some(format!(
                "emitted YAML is not canonically equal to the frozen Psych text: got {} want {}",
                clip(&canonical(&a), 300),
                clip(&canonical(&b), 300)
            ))
        }
        (a, b) => {
            return Some(format!(
                "canonical load failed: emitted {:?} frozen {:?}",
                a.err().map(|e| e.message),
                b.err().map(|e| e.message)
            ))
        }
    }
    match golden {
        None => Some(format!(
            "no pinned Yeptris golden; emitted bytes to pin (JSON string): {}",
            serde_json::to_string(emitted).unwrap_or_default()
        )),
        Some(g) if g == emitted.as_bytes() => None,
        Some(_) => Some(format!(
            "emitted bytes differ from the pinned Yeptris golden; got (JSON string): {}",
            serde_json::to_string(emitted).unwrap_or_default()
        )),
    }
}
