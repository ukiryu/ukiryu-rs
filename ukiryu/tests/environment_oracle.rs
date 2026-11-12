//! `Ukiryu::Environment` against observations captured from the pinned Ruby
//! oracle (ukiryu ab6df353, `lib/ukiryu/environment.rb`). Every case of
//! `spec/ukiryu/environment_spec.rb` plus oracle-observed edge cases.
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use ukiryu::environment::Environment;
use ukiryu::shell::cmd::Cmd;

const ORACLE: &str = include_str!("../src/environment/testdata/oracle.json");

fn oracle() -> Value {
    serde_json::from_str(ORACLE).expect("oracle json")
}

fn pairs(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .expect("pairs")
        .iter()
        .map(|p| {
            (
                p[0].as_str().unwrap().to_string(),
                p[1].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn env_obs(env: &Environment) -> Value {
    json!({ "env": env.to_pairs() })
}

fn path_args(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().map(|s| s.as_str().unwrap().to_string()).collect(),
        other => panic!("path additions {other}"),
    }
}

fn hash_of(env: &Environment) -> u64 {
    let mut h = DefaultHasher::new();
    env.hash(&mut h);
    h.finish()
}

#[test]
fn environment_operation_cases_match_oracle() {
    let o = oracle();
    let cases = o["environment"].as_array().unwrap();
    assert_eq!(cases.len(), 59, "pinned environment case count");
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let init = pairs(&case["init"]);
        let original = Environment::from_pairs(init.clone());
        let mut env = original.clone();
        let mut observed = Vec::new();
        for op in case["ops"].as_array().unwrap() {
            let name = op[0].as_str().unwrap();
            let arg = |i: usize| op[i].as_str().unwrap().to_string();
            // Ruby symbol keys are converted with `to_s`; Rust keys are
            // already strings, so the *_sym ops are the string ops.
            let r = match name {
                "set" | "set_sym_key" => {
                    env = env.set(&arg(1), arg(2));
                    env_obs(&env)
                }
                "set_int" => {
                    env = env.set(&arg(1), arg(2).parse::<i64>().unwrap());
                    env_obs(&env)
                }
                "delete" => {
                    env = env.delete(&arg(1));
                    env_obs(&env)
                }
                "merge_env" => {
                    env = env.merge(&Environment::from_pairs(pairs(&op[1])));
                    env_obs(&env)
                }
                "merge_hash" => {
                    env = env.merge_pairs(pairs(&op[1]));
                    env_obs(&env)
                }
                "prepend_path" => {
                    env = env.prepend_path(&path_args(&op[1]));
                    env_obs(&env)
                }
                "append_path" => {
                    env = env.append_path(&path_args(&op[1]));
                    env_obs(&env)
                }
                "remove_path" => {
                    env = env.remove_path(&arg(1));
                    env_obs(&env)
                }
                "get" | "get_sym" => json!(env.get(&arg(1))),
                "key" | "key_sym" => json!(env.contains_key(&arg(1))),
                "keys" => json!(env.keys()),
                "to_h" => json!(env.to_pairs()),
                "path_contains" => json!(env.path_contains(&arg(1))),
                "path_array" => json!(env.path_array()),
                "inspect" => json!(env.inspect()),
                other => panic!("{id}: unknown op {other}"),
            };
            observed.push(r);
        }
        assert_eq!(
            Value::Array(observed),
            case["observations"],
            "{id}: observations"
        );
        assert_eq!(env.to_pairs(), pairs(&case["final"]), "{id}: final");
        // Immutability: the starting environment is untouched by every op.
        assert_eq!(
            original.to_pairs(),
            pairs(&case["original_after"]),
            "{id}: original"
        );
        assert_eq!(original.to_pairs(), init, "{id}: original equals init");
    }
}

#[test]
fn equality_and_hash_match_oracle() {
    let o = oracle();
    let cases = o["equality"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let a = Environment::from_pairs(pairs(&case["a"]));
        let b = Environment::from_pairs(pairs(&case["b"]));
        assert_eq!(a == b, case["eq"].as_bool().unwrap(), "{id}: ==");
        assert_eq!(
            hash_of(&a) == hash_of(&b),
            case["hash_eq"].as_bool().unwrap(),
            "{id}: hash"
        );
        // Language divergence, not parity: the source defines `==` and
        // `hash` but not `eql?`, so Ruby Hash lookup by an equal but
        // distinct Environment fails (frozen `eql_hash_key: false`). Rust's
        // `Hash`/`Eq` contract makes equal values interchangeable keys.
        assert_eq!(case["eql_hash_key"], json!(false), "{id}: frozen ruby");
        let map: HashMap<Environment, u8> = [(a.clone(), 1)].into_iter().collect();
        assert_eq!(map.contains_key(&b), a == b, "{id}: rust key lookup");
    }
    // `env == {...}` is false in Ruby; comparing with a non-Environment
    // does not type-check in Rust.
    assert_eq!(o["environment_vs_hash_eq"], json!(false));
}

#[test]
fn hash_key_with_same_value() {
    // spec: 'can be used as Hash key'
    let env = Environment::from_pairs([("A", "1")]);
    let map: HashMap<Environment, &str> = [(env.clone(), "value")].into_iter().collect();
    assert_eq!(map[&env], "value");
}

#[test]
fn from_env_snapshot_cases_match_oracle() {
    let o = oracle();
    let cases = o["from_env"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let env = Environment::from_snapshot(pairs(&case["snapshot"]));
        assert_eq!(env.to_pairs(), pairs(&case["result"]), "{id}");
    }
}

#[test]
fn from_process_reads_without_mutation() {
    let before: Vec<_> = std::env::vars_os().collect();
    let env = Environment::from_process().expect("CI environment is Unicode");
    let after: Vec<_> = std::env::vars_os().collect();
    assert_eq!(before, after, "capture must not mutate the process");
    // Native check of the source's PATH alias: whatever case the host uses
    // for the search path variable (Windows `Path`), `PATH` is present.
    let has_path_like = before
        .iter()
        .any(|(k, _)| k.to_str().is_some_and(|k| k.to_uppercase() == "PATH"));
    assert_eq!(env.contains_key("PATH"), has_path_like);
    if has_path_like {
        let first = before
            .iter()
            .find(|(k, _)| k.to_str() == Some("PATH"))
            .or_else(|| {
                before
                    .iter()
                    .find(|(k, _)| k.to_str().is_some_and(|k| k.to_uppercase() == "PATH"))
            })
            .unwrap();
        assert_eq!(env.get("PATH"), first.1.to_str());
    }
    // The returned copy is independent of the receiver (spec: from_env
    // 'returns a frozen copy').
    let mut copy = env.to_pairs();
    copy.push(("NEW_VAR".into(), "value".into()));
    assert!(!env.contains_key("NEW_VAR"));
}

#[test]
fn chroot_and_venv_scenarios_over_snapshot() {
    // spec 'chroot scenario', with an explicit snapshot instead of ENV.
    let base = Environment::from_snapshot([("PATH", "/usr/bin:/bin"), ("HOME", "/h")]);
    let env = base
        .prepend_path(&["/chroot/usr/bin"])
        .prepend_path(&["/chroot/bin"]);
    assert_eq!(env.path_array()[0], "/chroot/bin");
    assert_eq!(env.path_array()[1], "/chroot/usr/bin");
    let venv = base
        .set("VIRTUAL_ENV", "/home/user/.venv")
        .prepend_path(&["/home/user/.venv/bin"]);
    assert_eq!(venv.get("VIRTUAL_ENV"), Some("/home/user/.venv"));
    assert!(venv.path_contains("/home/user/.venv/bin"));
    assert_eq!(base.get("PATH"), Some("/usr/bin:/bin"));
}

#[test]
fn system_and_new_are_empty() {
    assert!(Environment::system().is_empty());
    assert!(Environment::new().to_pairs().is_empty());
    assert_eq!(
        format!("{:?}", Environment::system()),
        "#<Ukiryu::Environment keys=0>"
    );
}

#[test]
fn per_shell_variable_reference() {
    let o = oracle();
    let rows = o["env_var"].as_array().unwrap();
    assert_eq!(rows.len(), 8);
    for row in rows {
        // Every pinned shell inherits the identity `Base#format_environment`,
        // so an Environment is passed to the executor unchanged.
        assert_eq!(row["format_environment_identity"], json!(true));
    }
    // Shell formatters on main so far: cmd. The remaining rows are checked
    // when their ShellFormatting modules land (see the PR's open items).
    let cmd = rows.iter().find(|r| r["shell"] == "cmd").unwrap();
    assert_eq!(
        Environment::var_reference(&Cmd, "PATH"),
        cmd["env_var"].as_str().unwrap()
    );
}
