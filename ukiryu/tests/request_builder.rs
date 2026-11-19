//! A loaded request feeds the command builder the way `Tool#execute` does:
//! `stringify_keys(request['arguments'])` becomes the `params` Hash that
//! `CommandBuilder#build_args` reads. Expectations come from
//! `fixtures/request/capture-request-builder.rb` (pinned Ruby source
//! ab6df353, Ruby 4.0.1, corpus oracle Gemfile.lock fc3b12f2, bash, linux).
//! Every request fixture in `fixtures/request/cases` (the run-file corpus) is
//! listed: 38 are rejected by the loader and only their error class is kept
//! here. The other 32 reach the builder in Ruby and must give the Ruby argv or
//! the Ruby error, except the 2 that `pending.json` declares (the port does
//! not load them as Ruby does yet).
//! This covers argv assembly only; it does not resolve tools or run them.
use serde_json::{json, Value as Json};
use ukiryu::command_builder::{build_args, BuildContext, CommandSpec};
use ukiryu::request::load_execution_request;
use ukiryu::shell::posix::Bash;
use ukiryu::types::StdHost;

fn root() -> String {
    format!("{}/../fixtures/request", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn request_arguments_build_the_ruby_argv() {
    let path = format!("{}/builder.json", root());
    let reference: Json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        reference["provenance"]["revision"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    assert_eq!(reference["provenance"]["shell"], "bash");
    let command: CommandSpec = serde_json::from_value(reference["command"].clone()).unwrap();
    let ctx = BuildContext {
        shell: &Bash,
        platform: reference["provenance"]["platform"].as_str().unwrap(),
        host: &StdHost,
    };
    let cases = reference["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    // Cases request_reference.rs declares `pending` (the port does not load
    // them as Ruby does yet) are skipped here: 3 cases, 2 of them among the 32;
    // that test fails once they match.
    let pending: Json =
        serde_json::from_str(&std::fs::read_to_string(format!("{}/pending.json", root())).unwrap())
            .unwrap();
    let is_pending = |id: &str| pending["cases"][id]["status"] == "pending";
    let (mut built, mut skipped) = (0, 0);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        if is_pending(id) {
            skipped += 1;
            continue;
        }
        let loaded = load_execution_request(format!("{}/cases/{id}.yaml", root()));
        if let Some(load) = case.get("load") {
            // Rejected at load time in Ruby: it never reaches the builder.
            // The message is checked by request_reference.rs.
            match loaded {
                Ok(_) => failures.push(format!("{id}: loaded, expected {load}")),
                Err(e) => {
                    if let Some(class) = e.source_class() {
                        if class != load["error"]["class"] {
                            failures.push(format!("{id}: load class {class}, expected {load}"));
                        }
                    }
                }
            }
            continue;
        }
        let request = loaded.unwrap_or_else(|e| panic!("{id}: {e}"));
        let got = match request.arguments().stringify_keys() {
            Err(e) => json!({ "error": { "class": e.ruby_class(), "message": e.message } }),
            Ok(params) => match build_args(&command, &params, &ctx) {
                Ok(args) => json!({ "ok": args }),
                Err(e) => {
                    json!({ "error": { "class": e.class.ruby_name(), "message": e.message } })
                }
            },
        };
        built += 1;
        if got != case["build_args"] {
            failures.push(format!("{id}: got {got}, expected {}", case["build_args"]));
        }
    }
    // Every request fixture is listed; 32 of the 70 reach the builder in Ruby.
    let fixtures = std::fs::read_dir(format!("{}/cases", root()))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "yaml")
        })
        .count();
    assert_eq!(cases.len(), fixtures);
    assert_eq!((built, skipped), (30, 3));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
