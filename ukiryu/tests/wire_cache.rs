use serde_json::{json, Value};

fn step(state: &mut Value, now: f64, action: Value) -> Value {
    let reply = ukiryu::wire::process(
        &serde_json::to_vec(&json!({
            "op":"cache", "max_size":2, "ttl":10.0, "now":now,
            "state":state, "action":action
        }))
        .unwrap(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    *state = reply["value"]["state"].clone();
    reply["value"]["result"].clone()
}

#[test]
fn snapshots_preserve_lru_and_created_time_expiry() {
    let mut state = json!([]);
    step(&mut state, 0.0, json!({"method":"set","key":1}));
    step(&mut state, 1.0, json!({"method":"set","key":2}));
    assert_eq!(step(&mut state, 2.0, json!({"method":"get","key":1})), 1);
    step(&mut state, 3.0, json!({"method":"set","key":3}));
    assert_eq!(
        step(&mut state, 3.0, json!({"method":"keys"})),
        json!([1, 3])
    );
    // A read moves LRU recency but cannot extend the creation-based TTL.
    assert_eq!(step(&mut state, 10.0, json!({"method":"get","key":1})), 1);
    assert!(step(&mut state, 10.1, json!({"method":"get","key":1})).is_null());
    assert_eq!(step(&mut state, 10.1, json!({"method":"keys"})), json!([3]));
}

#[test]
fn full_cache_overwrite_and_key_queries_match_ruby() {
    let mut state = json!([]);
    step(&mut state, 0.0, json!({"method":"set","key":1}));
    step(&mut state, 1.0, json!({"method":"set","key":2}));
    // key? must not touch access time, unlike get.
    assert_eq!(
        step(&mut state, 2.0, json!({"method":"contains_key","key":1})),
        true
    );
    // Ruby evicts before overwriting, even when the new key already exists.
    step(&mut state, 3.0, json!({"method":"set","key":2}));
    assert_eq!(step(&mut state, 3.0, json!({"method":"keys"})), json!([2]));
    assert_eq!(step(&mut state, 3.0, json!({"method":"delete","key":2})), 2);
    assert_eq!(
        step(&mut state, 3.0, json!({"method":"stats"})),
        json!({
            "size":0,"max_size":2,"ttl":10.0,"utilization":0.0
        })
    );
    step(&mut state, 4.0, json!({"method":"set","key":3}));
    step(&mut state, 4.0, json!({"method":"clear"}));
    assert_eq!(state, json!([]));
}

#[test]
fn tool_key_uses_the_core_contract() {
    let mut state = json!([]);
    assert_eq!(
        step(
            &mut state,
            0.0,
            json!({"method":"tool_key","name":"jq",
        "platform":"linux","shell":"bash","version":null})
        ),
        "jq-linux-bash-latest"
    );
}

#[test]
fn factory_preserves_aliases_and_rejects_string_names() {
    for (name, class) in [
        ("unix", "Bash"),
        ("bash", "Bash"),
        ("windows", "Cmd"),
        ("pwsh", "PowerShell"),
        ("powershell", "PowerShell"),
        ("fish", "Fish"),
    ] {
        let reply = ukiryu::wire::process(
            &serde_json::to_vec(&json!({
                "op":"resolve","method":"shell_class","name":{"$sym":name}
            }))
            .unwrap(),
        );
        assert_eq!(reply["value"], format!("Ukiryu::Shell::{class}"), "{reply}");
    }
    let reply = ukiryu::wire::process(br#"{"op":"resolve","method":"shell_class","name":"bash"}"#);
    assert_eq!(
        reply["error"]["source_class"],
        "Ukiryu::Errors::UnknownShellError"
    );
    assert_eq!(reply["error"]["message"], "Unknown shell: bash");
}
