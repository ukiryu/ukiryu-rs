//! A loaded request feeds the command builder the way `Tool#execute` does:
//! `stringify_keys(request['arguments'])` becomes the `params` Hash that
//! `CommandBuilder#build_args` reads. Expectations come from
//! `fixtures/request/capture-request-builder.rb` (pinned Ruby source
//! ab6df353, Ruby 4.0.1, corpus oracle Gemfile.lock fc3b12f2, bash, linux).
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
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let request = load_execution_request(format!("{}/cases/{id}.yaml", root())).unwrap();
        let params = request.arguments().stringify_keys().unwrap();
        let got = match build_args(&command, &params, &ctx) {
            Ok(args) => json!({ "ok": args }),
            Err(e) => json!({ "error": { "class": e.class.ruby_name(), "message": e.message } }),
        };
        if got != case["build_args"] {
            failures.push(format!("{id}: got {got}, expected {}", case["build_args"]));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
