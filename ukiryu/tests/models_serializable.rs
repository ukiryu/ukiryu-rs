//! Every lutaml-serializable model against oracle captures of `from_yaml`,
//! `to_yaml`, `to_json`, attribute readers and `Model.new`
//! (producer: capture/serializable.rb at ukiryu ab6df353).
#[path = "models_data/support.rs"]
mod support;

use serde_json::{Map, Value as Json};
use support::*;
use ukiryu::models::*;

macro_rules! g {
    ($m:expr; $($name:ident => $f:ident),* $(,)?) => {{
        let mut o = Map::new();
        $( o.insert(stringify!($name).trim_start_matches("r#").to_string(), $f(&$m.$name)); )*
        Json::Object(o)
    }};
}

fn argument(m: &Argument) -> Json {
    g!(m; name => opt_text, value => opt_text, r#type => attr_text)
}
fn arguments(m: &Arguments) -> Json {
    g!(m; options => model_list, flags => text_list, positional => model_list)
}
fn command_info(m: &CommandInfo) -> Json {
    g!(m; executable => opt_text, executable_name => opt_text, tool_name => opt_text,
        arguments => model, full_command => opt_text, shell => opt_text)
}
fn components(m: &Components) -> Json {
    g!(m; options => attr_hash, flags => attr_hash, arguments => attr_hash, exit_codes => model)
}
fn command_definition(m: &CommandDefinition) -> Json {
    g!(m; name => opt_text, description => opt_text, usage => opt_text, subcommand => opt_text,
        belongs_to => opt_text, cli_flag => opt_text, standalone_executable => attr_bool,
        aliases => text_list, use_env_vars => text_list, implements => text_list,
        options => model_list, flags => model_list, arguments => model_list,
        post_options => model_list, env_vars => model_list, exit_codes => model)
}
fn argument_definition(m: &ArgumentDefinition) -> Json {
    g!(m; name => opt_text, r#type => attr_text, required => attr_bool, position => attr_text,
        variadic => attr_bool, min => opt_int, max => opt_int, size => int_list, of => opt_text,
        range => int_list, values => text_list, separator => attr_text, format => opt_text,
        description => opt_text)
}
fn option_definition(m: &OptionDefinition) -> Json {
    g!(m; name => opt_text, cli => opt_text, r#type => attr_text, assignment_delimiter => attr_text,
        separator => opt_text, default => opt_text, range => int_list, values => text_list,
        of => opt_text, description => opt_text, platforms => text_list)
}
fn flag_definition(m: &FlagDefinition) -> Json {
    g!(m; name => opt_text, cli => opt_text, cli_short => opt_text, default => attr_bool,
        description => opt_text, platforms => text_list, position_constraint => opt_text,
        position_after => opt_text, conflicts_with => text_list)
}
fn env_var_definition(m: &EnvVarDefinition) -> Json {
    g!(m; name => opt_text, value => opt_text, from => opt_text, platforms => text_list,
        description => opt_text)
}
fn exit_codes(m: &ExitCodes) -> Json {
    g!(m; standard => attr_hash, custom => attr_hash)
}
fn run_environment(m: &RunEnvironment) -> Json {
    g!(m; hostname => attr_text, platform => attr_text, os_version => attr_text, shell => attr_text,
        shell_override => attr_bool, shell_version => attr_text, ruby_version => attr_text,
        ukiryu_version => attr_text, cpu_count => attr_int, total_memory => attr_int,
        working_directory => attr_text)
}
fn stage_metrics(m: &StageMetrics) -> Json {
    g!(m; name => attr_text, duration => attr_float, formatted_duration => attr_text,
        memory_before => attr_int, memory_after => attr_int, memory_delta => attr_int,
        success => attr_bool, error => attr_text)
}
fn execution_report(m: &ExecutionReport) -> Json {
    g!(m; tool_resolution => model, command_building => model, execution => model,
        response_building => model, total_duration => attr_float,
        formatted_total_duration => attr_text, run_environment => model, timestamp => attr_text)
}
fn execution_metadata(m: &ExecutionMetadata) -> Json {
    g!(m; started_at => opt_text, finished_at => opt_text, duration_seconds => opt_float,
        formatted_duration => opt_text)
}
fn output_info(m: &OutputInfo) -> Json {
    g!(m; stdout => attr_text, stderr => attr_text)
}
fn error_response(m: &ErrorResponse) -> Json {
    g!(m; status => attr_text, exit_code => attr_int, error => opt_text)
}
fn success_response(m: &SuccessResponse) -> Json {
    g!(m; status => attr_text, exit_code => opt_int, request => model, command => model,
        output => model, metadata => model, execution_report => model)
}
fn invocation(m: &Invocation) -> Json {
    g!(m; r#type => opt_text, multi_call => attr_bool, symlink_detection => attr_bool)
}
fn version_detection_method(m: &VersionDetectionMethod) -> Json {
    g!(m; r#type => opt_text, command => opt_text, pattern => opt_text, paths => opt_hash)
}
fn version_detection(m: &VersionDetection) -> Json {
    g!(m; command => text_list, pattern => opt_text, modern_threshold => opt_text,
        source => attr_text, detection_methods => model_list)
}
fn platform_profile(m: &PlatformProfile) -> Json {
    g!(m; name => opt_text, display_name => opt_text, platforms => text_list, shells => text_list,
        option_style => attr_text, executable_name => opt_text, commands => model_list,
        inherits => opt_text, routing_data => attr_hash, version_requirement => opt_text,
        exit_codes => model, env_var_sets => attr_hash)
}
fn tool_definition(m: &ToolDefinition) -> Json {
    g!(m; ukiryu_schema => opt_text, self_uri => opt_text, name => opt_text,
        display_name => opt_text, homepage => opt_text, version => opt_text,
        implements => text_list, aliases => text_list, invocation => model, profiles => model_list,
        version_detection => model, components => model)
}
fn tool_metadata(m: &ToolMetadata) -> Json {
    let mut o = match g!(m; name => opt_text, version => opt_text, display_name => opt_text,
        implements => opt_text, implements_version => opt_text, homepage => opt_text,
        description => opt_text, register_path => opt_text, backed_by => opt_text)
    {
        Json::Object(o) => o,
        _ => unreachable!(),
    };
    o.insert("aliases".into(), enc(&Value::Array(m.aliases())));
    o.insert("tool_name".into(), enc_text(m.tool_name().as_ref()));
    let dc = m.default_command().expect("default_command");
    o.insert(
        "default_command".into(),
        enc(&dc.map_or(Value::Nil, Value::Symbol)),
    );
    o.insert("may_provide".into(), attr_text(&m.may_provide));
    Json::Object(o)
}

struct Snap {
    yaml: String,
    json: String,
    getters: Json,
}

fn snap<T: Model>(m: &T, getters: fn(&T) -> Json) -> Result<Snap, ModelError> {
    Ok(Snap {
        yaml: m.to_yaml()?,
        json: m.to_json()?,
        getters: getters(m),
    })
}

fn parse<T: Model + Default>(
    yaml: Option<&str>,
    getters: fn(&T) -> Json,
) -> Result<Snap, ModelError> {
    match yaml {
        Some(y) => snap(&T::from_yaml(y)?, getters),
        None => snap(&T::default(), getters),
    }
}

/// `yaml = None` builds the model with `Model.new`.
fn run(class: &str, yaml: Option<&str>) -> Result<Snap, ModelError> {
    match class {
        "Argument" => parse(yaml, argument),
        "Arguments" => parse(yaml, arguments),
        "CommandInfo" => parse(yaml, command_info),
        "Components" => parse(yaml, components),
        "CommandDefinition" => parse(yaml, command_definition),
        "ArgumentDefinition" => parse(yaml, argument_definition),
        "OptionDefinition" => parse(yaml, option_definition),
        "FlagDefinition" => parse(yaml, flag_definition),
        "EnvVarDefinition" => parse(yaml, env_var_definition),
        "ExitCodes" => parse(yaml, exit_codes),
        "RunEnvironment" => parse(yaml, run_environment),
        "StageMetrics" => parse(yaml, stage_metrics),
        "ExecutionReport" => parse(yaml, execution_report),
        "ExecutionMetadata" => parse(yaml, execution_metadata),
        "OutputInfo" => parse(yaml, output_info),
        "ErrorResponse" => parse(yaml, error_response),
        "SuccessResponse" => parse(yaml, success_response),
        "Invocation" => parse(yaml, invocation),
        "VersionDetectionMethod" => parse(yaml, version_detection_method),
        "VersionDetection" => parse(yaml, version_detection),
        "ToolMetadata" => parse(yaml, tool_metadata),
        "PlatformProfile" => parse(yaml, platform_profile),
        "ToolDefinition" => parse(yaml, tool_definition),
        other => panic!("unmapped model {other}"),
    }
}

fn compare(label: &str, expected: &Json, actual: &Snap, failures: &mut Vec<String>) {
    let ruby_yaml = expected["yaml"].as_str().expect("yaml");
    if !yaml_equiv(ruby_yaml, &actual.yaml) {
        failures.push(format!(
            "{label}: yaml\n  ruby: {ruby_yaml:?}\n  rust: {:?}",
            actual.yaml
        ));
    }
    if expected["json"].as_str() != Some(actual.json.as_str()) {
        failures.push(format!(
            "{label}: json\n  ruby: {}\n  rust: {}",
            expected["json"], actual.json
        ));
    }
    if !same(&expected["getters"], &actual.getters) {
        failures.push(format!(
            "{label}: getters\n  ruby: {}\n  rust: {}",
            expected["getters"], actual.getters
        ));
    }
}

#[test]
fn every_serializable_model_matches_the_oracle() {
    let golden = golden("serializable.json");
    let mut failures = Vec::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut yaml_bytes_equal = 0usize;
    let mut compared = 0usize;

    let models = golden["models"].as_array().expect("models");
    assert_eq!(models.len(), 23, "declared Serializable models");
    for m in models {
        let class = m["class"].as_str().unwrap();
        match run(class, None) {
            Ok(snap) => compare(&format!("{class}.new"), &m["new"], &snap, &mut failures),
            Err(e) => failures.push(format!("{class}.new: rust error {e}")),
        }
        *counts.entry("new".into()).or_default() += 1;
    }

    for case in golden["cases"].as_array().expect("cases") {
        let id = case["id"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let class = case["class"].as_str().unwrap();
        let yaml = case["input_yaml"].as_str().unwrap();
        let expect = &case["expect"];
        let actual = run(class, Some(yaml));
        let outcome = match (expect.get("ok"), actual) {
            (Some(ok), Ok(snap)) => {
                compared += 1;
                if ok["yaml"]
                    .as_str()
                    .map(|y| y.trim_start_matches("---\n").trim_end())
                    == Some(snap.yaml.as_str())
                {
                    yaml_bytes_equal += 1;
                }
                compare(id, ok, &snap, &mut failures);
                "ok"
            }
            (Some(_), Err(e)) if kind == "divergence" && e.kind == ModelErrorKind::Unsupported => {
                "unsupported"
            }
            (Some(_), Err(e)) => {
                failures.push(format!("{id} ({kind}): ruby ok, rust error {e}"));
                "fail"
            }
            (None, Err(e)) if kind == "divergence" && e.kind == ModelErrorKind::Unsupported => {
                "unsupported"
            }
            (None, Err(e)) => {
                let class = expect["error"]["class"].as_str().unwrap();
                if e.ruby_class() != class {
                    failures.push(format!("{id}: error class ruby {class}, rust {e}"));
                }
                "error"
            }
            (None, Ok(snap)) => {
                failures.push(format!(
                    "{id}: ruby raised {}, rust ok {}",
                    expect["error"], snap.json
                ));
                "fail"
            }
        };
        *counts.entry(format!("{kind}/{outcome}")).or_default() += 1;
    }
    println!("counts: {counts:?}");
    println!("yaml byte-identical (ignoring document marker): {yaml_bytes_equal}/{compared}");
    for f in failures.iter().take(40) {
        println!("FAIL {f}");
    }
    report_integral_floats();
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}
