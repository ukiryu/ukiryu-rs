//! The source corpus traverses the JSON boundary, never a second formatter.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn cases() -> Vec<(String, Value, Value)> {
    let manifest: Value =
        serde_json::from_str(include_str!("../../corpus/conformance/manifest.json")).unwrap();
    let mut cases = Vec::new();
    let mut shells = BTreeMap::<String, usize>::new();
    let mut methods = BTreeSet::new();
    for line in include_str!("../../corpus/conformance/cases/escape.jsonl")
        .split('\n')
        .filter(|s| !s.is_empty())
    {
        let row: Value = serde_json::from_str(line).unwrap();
        assert_eq!(row["kind"], "escape");
        assert_eq!(row["applicability"]["requires"], json!([]));
        assert_eq!(
            row["applicability"]["platforms"],
            json!(["linux", "macos", "windows"])
        );
        let method = row["op"].as_str().unwrap().strip_prefix("shell.").unwrap();
        assert!([
            "escape",
            "quote",
            "needs_quoting",
            "format_path",
            "env_var",
            "join"
        ]
        .contains(&method));
        methods.insert(method.to_owned());
        let mut operation = row["input"].as_object().unwrap().clone();
        let shell = operation.remove("shell").unwrap();
        *shells
            .entry(shell.as_str().unwrap().to_owned())
            .or_default() += 1;
        operation.insert("method".into(), json!(method));
        assert!(
            row["expect"].get("ok").is_some(),
            "unexpected source error: {}",
            row["id"]
        );
        cases.push((
            row["id"].as_str().unwrap().to_owned(),
            json!({"op":"escape","shell":shell,"operation":operation}),
            row["expect"]["ok"].clone(),
        ));
    }
    assert_eq!(
        cases.len(),
        manifest["cases"]["by_kind"]["escape"].as_u64().unwrap() as usize
    );
    assert_eq!(shells.len(), 8);
    assert_eq!(methods.len(), 6);
    assert!(shells.values().all(|count| *count == 1577));
    let provenance: Value =
        serde_json::from_str(include_str!("../../tests/escape-wire/provenance.json")).unwrap();
    let shared = cases.len();
    for line in include_str!("../../tests/escape-wire/supplemental.jsonl")
        .split('\n')
        .filter(|s| !s.is_empty())
    {
        let row: Value = serde_json::from_str(line).unwrap();
        cases.push((
            row["id"].as_str().unwrap().to_owned(),
            row["request"].clone(),
            row["value"].clone(),
        ));
    }
    assert_eq!(
        cases.len() - shared,
        provenance["rows"].as_u64().unwrap() as usize
    );
    assert_eq!(
        cases
            .iter()
            .map(|(id, _, _)| id)
            .collect::<BTreeSet<_>>()
            .len(),
        cases.len()
    );
    cases
}

fn verify(transport: impl Fn(&[u8]) -> Value) {
    let mut failures = Vec::new();
    let cases = cases();
    for (id, request, expected) in &cases {
        let actual = transport(&serde_json::to_vec(request).unwrap());
        if actual != json!({"ok":true,"value":expected}) {
            failures.push(format!("{id}: expected {expected}, actual {actual}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed; first 12: {:#?}",
        failures.len(),
        cases.len(),
        &failures[..failures.len().min(12)]
    );
    println!("{} source-derived escape requests passed", cases.len());
}

#[test]
fn source_corpus_through_wire() {
    verify(ukiryu::wire::process);
}

#[cfg(feature = "c")]
#[test]
fn source_corpus_through_owned_c_abi() {
    verify(|bytes| {
        // SAFETY: the byte span lives until the call completes. The reply is
        // copied before being freed exactly once by its originating allocator.
        unsafe {
            let pointer = ukiryu::ffi::ukiryu_json(bytes.as_ptr(), bytes.len());
            assert!(!pointer.is_null());
            let reply = std::ffi::CStr::from_ptr(pointer).to_bytes().to_vec();
            ukiryu::ffi::ukiryu_free(pointer);
            serde_json::from_slice(&reply).unwrap()
        }
    });
}

#[test]
fn strict_names_types_and_operations() {
    for name in [
        "bash",
        "zsh",
        "fish",
        "sh",
        "dash",
        "tcsh",
        "powershell",
        "cmd",
    ] {
        let plain =
            json!({"op":"escape","shell":name,"operation":{"method":"quote","string":"Ω\0$\n'\""}});
        let mut path = plain.clone();
        path["shell"] = json!(format!("/tools/{name}"));
        assert_eq!(
            ukiryu::wire::process(&serde_json::to_vec(&plain).unwrap()),
            ukiryu::wire::process(&serde_json::to_vec(&path).unwrap())
        );
    }
    for name in [
        "",
        "unix",
        "windows",
        "pwsh",
        "mybash",
        "/bin/mybash",
        "BASH",
        "bash.exe",
    ] {
        let reply = ukiryu::wire::process(
            &serde_json::to_vec(
                &json!({"op":"escape","shell":name,"operation":{"method":"escape","string":"x"}}),
            )
            .unwrap(),
        );
        assert_eq!(reply["error"]["code"], "UNKNOWN_SHELL");
        assert_eq!(
            reply["error"]["source_class"],
            "Ukiryu::Errors::UnknownShellError"
        );
    }
    for operation in [
        json!({"method":"escape","string":false}),
        json!({"method":"join","executable":"x","args":[1]}),
        json!({"method":"quote","string":"x","extra":1}),
        json!({"method":"execute","string":"x"}),
        json!({"method":"escape"}),
    ] {
        let reply = ukiryu::wire::process(
            &serde_json::to_vec(&json!({"op":"escape","shell":"bash","operation":operation}))
                .unwrap(),
        );
        assert_eq!(reply["error"]["code"], "INVALID_REQUEST");
    }
    for operation in [
        json!({"method":"quote","string":"x","for_exe":true}),
        json!({"method":"escape_for_double_quotes","string":"x"}),
    ] {
        let reply = ukiryu::wire::process(
            &serde_json::to_vec(&json!({"op":"escape","shell":"bash","operation":operation}))
                .unwrap(),
        );
        assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT");
    }
}
