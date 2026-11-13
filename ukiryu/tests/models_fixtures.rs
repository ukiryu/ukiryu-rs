//! The four model families over the oracle's own YAML fixtures (byte copies
//! in `models_data/fixtures`, sha256 pinned): ToolDefinition profiles,
//! ImplementationVersion, ImplementationIndex and Interface, through both
//! the `from_yaml(path)` entry points and the register's
//! `from_hash(symbolize_keys(YAML.safe_load(...)))` path
//! (producer: capture/fixtures.rb at ukiryu ab6df353).
#[path = "models_data/support.rs"]
mod support;

use serde_json::{json, Map, Value as Json};
use std::path::PathBuf;
use support::*;
use ukiryu::models::*;

fn fixture(rel: &str) -> PathBuf {
    data_dir().join("fixtures").join(rel)
}

fn text(rel: &str) -> String {
    std::fs::read_to_string(fixture(rel)).unwrap()
}

fn opt_enc(v: Option<&Value>) -> Json {
    v.map_or(Json::Null, enc)
}

fn vd_snap(v: &Option<VersionDetection>) -> Json {
    match v {
        None => Json::Null,
        Some(vd) => json!({
            "hash": enc(&vd.to_hash().unwrap()),
            "yaml": vd.to_yaml().unwrap(),
            "json": vd.to_json().unwrap(),
        }),
    }
}

fn vd_same(expected: &Json, actual: &Json) -> bool {
    match (expected, actual) {
        (Json::Null, Json::Null) => true,
        (Json::Object(e), Json::Object(a)) => {
            e["hash"] == a["hash"]
                && e["json"] == a["json"]
                && yaml_equiv(e["yaml"].as_str().unwrap(), a["yaml"].as_str().unwrap())
        }
        _ => false,
    }
}

fn vd_ivar(v: &Option<VersionDetection>) -> Json {
    v.as_ref().map_or(Json::Null, enc_model)
}

fn version_ivars(v: &ImplementationVersion) -> Json {
    json!({
        "@aliases": v.aliases,
        "@display_name": enc(&v.display_name),
        "@execution_profiles": enc(&v.execution_profiles),
        "@implements": enc(&v.implements),
        "@version": enc(&v.version),
        "@version_detection": vd_ivar(&v.version_detection),
    })
}

fn index_snapshot(i: &ImplementationIndex) -> Json {
    json!({
        "ivars": {
            "@implementations": i.implementations.iter().map(enc).collect::<Vec<_>>(),
            "@interface": enc(&i.interface),
            "@interface_version": enc(&i.interface_version),
            "@name": enc(&i.name),
        },
        "names": i.implementation_names().iter().map(enc).collect::<Vec<_>>(),
        "to_s": i.to_ruby_s(),
        "inspect": i.inspect(),
        "default": opt_enc(i.implementation(&Value::str("default"))),
        "missing": opt_enc(i.implementation(&Value::str("zz"))),
    })
}

fn interface_snapshot(i: &Interface) -> Json {
    let first = match &i.actions {
        Value::Array(items) => match items.first() {
            Some(h @ Value::Hash(_)) => h.get_sym("name").cloned(),
            _ => None,
        },
        _ => None,
    };
    let action_first = match &first {
        Some(name) if name.truthy() => opt_enc(i.action(name).unwrap()),
        _ => Json::Null,
    };
    let raise = |e: ModelError| format!("raise {}", e.ruby_class());
    json!({
        "ivars": {
            "@actions": enc(&i.actions),
            "@aliases": i.aliases.iter().map(|a| json!({"$sym": a})).collect::<Vec<_>>(),
            "@display_name": enc(&i.display_name),
            "@name": enc(&i.name),
            "@version": enc(&i.version),
        },
        "to_s": i.to_ruby_s(),
        "inspect": i.inspect().unwrap_or_else(raise),
        "first_action": first.as_ref().map_or(Json::Null, enc),
        "action_first": action_first,
        "action_q": i.has_action(&Value::str("zz")).map_or_else(|e| json!(raise(e)), |b| json!(b)),
    })
}

/// `frozen?` has no Rust counterpart (values are immutable unless `mut`).
fn check(id: &str, expected: &Json, actual: &Json, failures: &mut Vec<String>) {
    let mut expected = expected.clone();
    if let Some(o) = expected.as_object_mut() {
        o.remove("frozen");
    }
    let expected = &expected;
    if !same(expected, actual) {
        failures.push(format!("{id}\n  ruby: {expected}\n  rust: {actual}"));
    }
}

#[test]
fn fixture_copies_match_their_pinned_digests() {
    let golden = golden("fixtures.json");
    let fixtures = golden["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 64);
    for f in fixtures {
        let bytes = std::fs::read(fixture(f["file"].as_str().unwrap())).unwrap();
        let digest = sha256_hex(&bytes);
        assert_eq!(digest, f["sha256"].as_str().unwrap(), "{}", f["file"]);
    }
}

#[test]
fn four_model_families_match_the_oracle() {
    let golden = golden("fixtures.json");
    let mut failures = Vec::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for row in golden["rows"].as_array().unwrap() {
        let file = row["file"].as_str().unwrap();
        let api = row["api"].as_str().unwrap();
        let id = format!("{api} {file}");
        let expected = &row["expect"]["ok"];
        assert!(
            !expected.is_null(),
            "{id}: oracle rows are all successful loads"
        );
        *counts.entry(api.to_owned()).or_default() += 1;
        match api {
            "ToolDefinition.from_yaml" => {
                let text = std::fs::read_to_string(fixture(file)).unwrap();
                match ToolDefinition::from_yaml(&text) {
                    Ok(t) => {
                        check(
                            &id,
                            &expected["hash"],
                            &enc(&t.to_hash().unwrap()),
                            &mut failures,
                        );
                        if expected["json"].as_str() != Some(t.to_json().unwrap().as_str()) {
                            failures.push(format!("{id}: json differs"));
                        }
                        if !yaml_equiv(expected["yaml"].as_str().unwrap(), &t.to_yaml().unwrap()) {
                            failures.push(format!("{id}: yaml differs"));
                        }
                    }
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            "ImplementationVersion.from_yaml" => {
                match ImplementationVersion::from_yaml(&fixture(file)) {
                    Ok(v) => check(&id, expected, &version_ivars(&v), &mut failures),
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            "ImplementationVersion.from_yaml.compatible" => {
                match ImplementationVersion::from_yaml(&fixture(file)) {
                    Ok(v) => {
                        let actual = json!({
                            "linux/bash": opt_enc(v.compatible_profile("linux", "bash").unwrap().as_ref()),
                            "to_s": v.to_ruby_s(),
                        });
                        let exp =
                            json!({"linux/bash": expected["linux/bash"], "to_s": expected["to_s"]});
                        check(&id, &exp, &actual, &mut failures);
                        if !vd_same(
                            &expected["version_detection"],
                            &vd_snap(&v.version_detection),
                        ) {
                            failures.push(format!("{id}: version_detection"));
                        }
                    }
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            "ImplementationVersion.from_hash" => {
                match ImplementationVersion::from_register_yaml(&text(file)).map(Option::unwrap) {
                    Ok(v) => {
                        let mut compatible = Map::new();
                        for p in ["linux", "macos", "windows"] {
                            for s in ["bash", "zsh", "sh", "fish", "powershell", "cmd"] {
                                let prof = v.compatible_profile(p, s).unwrap();
                                compatible.insert(format!("{p}/{s}"), opt_enc(prof.as_ref()));
                            }
                        }
                        let mut profile = Map::new();
                        for p in v.execution_profiles.as_array().unwrap_or(&[]) {
                            let name = p.get_sym("name").cloned().unwrap_or(Value::Nil);
                            profile.insert(name.to_ruby_s(), opt_enc(v.profile(&name).unwrap()));
                        }
                        let actual = json!({
                            "ivars": version_ivars(&v),
                            "to_s": v.to_ruby_s(),
                            "inspect": v.inspect().unwrap(),
                            "compatible": compatible,
                            "profile": profile,
                            "profile_missing": opt_enc(v.profile(&Value::str("zz")).unwrap()),
                        });
                        let mut exp = expected.clone();
                        let vd = exp
                            .as_object_mut()
                            .unwrap()
                            .remove("version_detection")
                            .unwrap();
                        check(&id, &exp, &actual, &mut failures);
                        if !vd_same(&vd, &vd_snap(&v.version_detection)) {
                            failures.push(format!("{id}: version_detection"));
                        }
                    }
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            "ImplementationIndex.from_yaml" => match ImplementationIndex::from_yaml(&fixture(file))
            {
                Ok(i) => check(&id, expected, &index_snapshot(&i), &mut failures),
                Err(e) => failures.push(format!("{id}: {e}")),
            },
            "ImplementationIndex.from_hash" => {
                match ImplementationIndex::from_register_yaml(&text(file)).map(Option::unwrap) {
                    Ok(i) => check(&id, expected, &index_snapshot(&i), &mut failures),
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            "Interface.from_yaml" => match Interface::from_yaml(&fixture(file)) {
                Ok(i) => check(&id, expected, &interface_snapshot(&i), &mut failures),
                Err(e) => failures.push(format!("{id}: {e}")),
            },
            "Interface.from_hash" => {
                match Interface::from_register_yaml(&text(file)).map(Option::unwrap) {
                    Ok(i) => check(&id, expected, &interface_snapshot(&i), &mut failures),
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            other => panic!("unmapped api {other}"),
        }
    }
    println!("counts: {counts:?}");
    for f in failures.iter().take(30) {
        println!("FAIL {f}");
    }
    report_integral_floats();
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}

#[test]
fn missing_files_and_non_mapping_roots_fail_like_ruby() {
    let missing = data_dir().join("fixtures/does-not-exist.yaml");
    assert_eq!(
        Interface::from_yaml(&missing).unwrap_err().kind,
        ModelErrorKind::Io
    );
    assert_eq!(
        ImplementationIndex::from_hash(&Value::Nil)
            .unwrap_err()
            .ruby_class(),
        "NoMethodError"
    );
    assert_eq!(
        Interface::from_hash(&Value::Nil).unwrap_err().ruby_class(),
        "NoMethodError"
    );
    // The register returns nil for an empty or null document.
    assert_eq!(Interface::from_register_yaml("").unwrap(), None);
    assert_eq!(
        ImplementationIndex::from_register_yaml("~\n").unwrap(),
        None
    );
    assert_eq!(ImplementationVersion::from_register_yaml("").unwrap(), None);
}

/// Minimal SHA-256 so the digest check needs no extra dev-dependency.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
