//! Model helper semantics against oracle captures
//! (producer: capture/helpers.rb at ukiryu ab6df353).
#[path = "models_data/support.rs"]
mod support;

use serde_json::{json, Value as Json};
use std::cmp::Ordering;
use support::*;
use ukiryu::models::ruby::{format_duration, Number};
use ukiryu::models::*;

fn s(v: &Json) -> &str {
    v.as_str().expect("string input")
}

fn opt_s(v: &Json) -> Option<&str> {
    v.as_str()
}

fn sym(v: Result<String, ModelError>) -> Json {
    json!({"$sym": v.expect("to_sym")})
}

fn ord(o: Option<Ordering>) -> Json {
    match o {
        None => Json::Null,
        Some(Ordering::Less) => json!(-1),
        Some(Ordering::Equal) => json!(0),
        Some(Ordering::Greater) => json!(1),
    }
}

fn texts(v: &[Text]) -> Json {
    Json::Array(v.iter().map(|t| enc(&t.to_ruby_value())).collect())
}

fn json_to_ruby(v: &Json) -> Value {
    match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::int(i),
            None => Value::Float(n.as_f64().unwrap()),
        },
        Json::String(s) => Value::str(s.clone()),
        Json::Array(items) => Value::Array(items.iter().map(json_to_ruby).collect()),
        Json::Object(m) => Value::Hash(
            m.iter()
                .map(|(k, v)| (Value::str(k.clone()), json_to_ruby(v)))
                .collect(),
        ),
    }
}

fn from<T: Model>(yaml: &Json) -> T {
    T::from_yaml(s(yaml)).expect("model yaml")
}

fn opt_model<T: Model>(m: Option<&T>) -> Json {
    m.map_or(Json::Null, enc_model)
}

fn semver_compare(a: &str, b: &str) -> Json {
    let x = SemanticVersion::new(Some(a));
    let y = SemanticVersion::new(Some(b));
    let yy = Some(VersionOperand::Version(&y));
    json!({
        "cmp": SemanticVersion::compare(Some(a), Some(b)),
        "spaceship": ord(x.cmp_with(yy)),
        "eq": x.eq_with(yy), "gt": x.gt(yy), "lt": x.lt(yy), "ge": x.ge(yy), "le": x.le(yy),
        "eql": x.eql(&y), "to_s": x.to_ruby_s(),
        "str_cmp": ord(x.cmp_with(Some(VersionOperand::Str(b)))),
    })
}

fn routing_group(group: &str) -> Json {
    let table = json_to_ruby(
        &json!({"convert": "magick", "identify": "magick-identify", "mogrify": "mogrify"}),
    );
    let path = |r: &Routing, p: &[&str]| Json::Array(r.resolve_path(p).iter().map(enc).collect());
    match group {
        "routing.basic" => {
            let r = Routing::new(&table);
            json!({
                "resolve_convert": opt_enc(r.resolve("convert")), "resolve_missing": opt_enc(r.resolve("nope")),
                "include": r.includes("identify"), "include_missing": r.includes("zz"), "keys": r.keys(),
                "empty": r.is_empty(), "size": r.size(), "to_h": enc(&r.to_h()), "to_s": r.to_ruby_s(),
                "circular": r.is_circular(), "path1": path(&r, &["convert"]),
                "path2": path(&r, &["convert", "identify"]), "path_missing": path(&r, &["zz"]),
                "path_empty": path(&r, &[]),
            })
        }
        "routing.empty" => {
            let r = Routing::new(&Value::Hash(vec![]));
            json!({"to_s": r.to_ruby_s(), "empty": r.is_empty(), "keys": r.keys(), "size": r.size()})
        }
        "routing.child" => {
            let r = Routing::new(&table);
            let c = r.child("convert");
            c.merge(&json_to_ruby(&json!({"sub": "x"})));
            json!({
                "child_resolve_sub": opt_enc(c.resolve("sub")), "child_resolve_parent": opt_enc(c.resolve("identify")),
                "child_include_parent": c.includes("mogrify"), "path": path(&r, &["convert", "sub"]),
                "same_child": r.child("convert").same_object(&c), "child_circular": c.is_circular(),
                "child_keys": c.keys(), "child_to_h": enc(&c.to_h()),
            })
        }
        "routing.merge" => {
            let r = Routing::new(&table);
            r.merge(&json_to_ruby(&json!({"convert": "other", "new": "n"})));
            json!({"to_h": enc(&r.to_h()), "to_s": r.to_ruby_s(), "keys": r.keys()})
        }
        _ => unreachable!(),
    }
}

fn opt_enc(v: Option<Value>) -> Json {
    v.as_ref().map_or(Json::Null, enc)
}

fn opt_ref(v: Option<&Value>) -> Json {
    v.map_or(Json::Null, enc)
}

fn component_ref(r: Option<ComponentRef<'_>>) -> Json {
    match r {
        None => Json::Null,
        Some(ComponentRef::Value(v)) => enc(v),
        Some(ComponentRef::ExitCodes(e)) => enc_model(e),
    }
}

fn parsed(p: ParsedPosition) -> Json {
    match p {
        ParsedPosition::Last => json!({"$sym": "last"}),
        ParsedPosition::First => json!({"$sym": "first"}),
        ParsedPosition::Index(i) => json!(i as i64),
    }
}

fn params(case: i64) -> Vec<(String, Value)> {
    let p = |k: &str, v: Value| (k.to_owned(), v);
    let int = |i: i128| Value::Integer(i.into());
    match case {
        0 => vec![
            p("verbose", Value::Bool(true)),
            p("quality", int(90)),
            p("input", Value::str("a.png")),
        ],
        1 => vec![
            p("verbose", Value::Bool(false)),
            p("quality", Value::Array(vec![int(1), int(2)])),
            p(
                "input",
                Value::Array(vec![Value::str("a"), Value::sym("b")]),
            ),
        ],
        2 => vec![
            p("verbose", Value::str("yes")),
            p("extra", Value::Bool(true)),
            p("other", Value::Bool(false)),
            p("list", Value::Array(vec![int(1), Value::Float(2.5)])),
            p("sym", Value::sym("s")),
            p("n", Value::Nil),
            p("f", Value::Float(1.5)),
            p("i", int(3)),
        ],
        3 => vec![
            p("verbose", Value::str("yes")),
            p("extra", Value::Bool(true)),
            p("list", Value::Array(vec![int(1), Value::Float(2.5)])),
            p("sym", Value::sym("s")),
            p("n", Value::Nil),
            p("h", Value::Hash(vec![(Value::str("a"), int(1))])),
        ],
        4 => vec![],
        _ => unreachable!(),
    }
}

fn snapshot<T: Model>(m: &T) -> Json {
    json!({"hash": enc(&m.to_hash().unwrap()), "yaml": m.to_yaml().unwrap(), "json": m.to_json().unwrap()})
}

/// Snapshot equality: hash and json exact, yaml structural.
fn snap_same(expected: &Json, actual: &Json) -> bool {
    let mut ok = true;
    for (k, v) in expected.as_object().unwrap() {
        let a = &actual[k.as_str()];
        ok &= if k == "yaml" {
            yaml_equiv(v.as_str().unwrap(), a.as_str().unwrap())
        } else {
            same(v, a)
        };
    }
    ok
}

fn evaluate(group: &str, i: &Json) -> Json {
    match group {
        "semver.parse" => json!(SemanticVersion::parse(opt_s(&i["v"]))
            .iter()
            .map(|x| *x as i64)
            .collect::<Vec<_>>()),
        "semver.compare" => semver_compare(s(&i["a"]), s(&i["b"])),
        "semver.int" => ord(
            SemanticVersion::new(Some("5.0")).cmp_with(Some(VersionOperand::Int(i128::from(
                i["i"].as_i64().unwrap(),
            )))),
        ),
        "semver.nil" => {
            let v = SemanticVersion::new(Some("1"));
            json!([ord(v.cmp_with(None)), v.eq_with(None)])
        }
        "compat.check" => {
            let c = VersionCompatibility::check(s(&i["installed"]), opt_s(&i["requirement"]));
            json!({"compatible": c.is_compatible(), "incompatible": c.is_incompatible(), "reason": c.reason,
                   "status_message": c.status_message(), "installed_version": c.installed_version,
                   "required_version": c.required_version})
        }
        "compat.new" => {
            json!(VersionCompatibility::new("1", Some("2"), false, None).status_message())
        }
        g if g.starts_with("routing.") => routing_group(g),
        "exit_codes" => {
            let e: ExitCodes = from(&i["yaml"]);
            let code = json_to_ruby(&i["code"]);
            let success = match e.is_success(&code) {
                Ok(b) => json!(b),
                Err(err) => json!(format!("raise {}", err.ruby_class())),
            };
            json!({"meaning": opt_ref(e.meaning(&code)), "defined": e.is_defined(&code), "success": success,
                   "all_codes": enc(&e.all_codes()), "standard_codes": enc(&e.standard_codes()),
                   "custom_codes": enc(&e.custom_codes())})
        }
        "exit_codes.new" => {
            let e = ExitCodes::default();
            let zero = Value::int(0);
            json!([
                opt_ref(e.meaning(&zero)),
                enc(&e.all_codes()),
                e.is_success(&zero).unwrap(),
                e.is_success(&Value::int(1)).unwrap()
            ])
        }
        "exit_codes.null" => {
            let e: ExitCodes = ExitCodes::from_yaml("standard: ~\ncustom: ~\n").unwrap();
            json!([
                opt_ref(e.meaning(&Value::int(0))),
                enc(&e.all_codes()),
                enc(&e.standard_codes())
            ])
        }
        "components" => {
            let c: Components = from(&i["yaml"]);
            let r = s(&i["ref"]);
            json!({"can_resolve": c.can_resolve(r), "resolve": component_ref(c.resolve(r))})
        }
        "components.lookup" => {
            let c: Components = from(&i["yaml"]);
            json!({"option": opt_ref(c.option("verbose")), "flag": opt_ref(c.flag("force")),
                   "argument": opt_ref(c.argument("input")), "missing": opt_ref(c.option("x"))})
        }
        "components.empty" => {
            let c: Components = from(&i["yaml"]);
            json!({"can": c.can_resolve("#/components/exit_codes/x"), "opt": opt_ref(c.option("a")),
                   "resolve": component_ref(c.resolve("#/components/exit_codes/x"))})
        }
        "command_definition" => {
            let c: CommandDefinition = from(&i["yaml"]);
            let positions: Vec<Json> = c
                .arguments
                .iter()
                .flatten()
                .map(|a| {
                    json!([
                        enc_text(a.name.as_ref()),
                        enc_text(a.position.get()),
                        a.is_last(),
                        parsed(a.parsed_position()),
                        a.numeric_position() as i64
                    ])
                })
                .collect();
            json!({
                "belongs_to_command": c.is_belongs_to_command(), "flag_action": c.is_flag_action(),
                "has_subcommand": c.has_subcommand(), "standalone_executable": c.is_standalone_executable(),
                "option": opt_model(c.option("resize")), "option_missing": opt_model(c.option("zz")),
                "flag": opt_model(c.flag("verbose")), "argument": opt_model(c.argument("first")),
                "last_argument": opt_model(c.last_argument()),
                "regular_arguments": c.regular_arguments().iter().map(|a| enc_text(a.name.as_ref())).collect::<Vec<_>>(),
                "arg_positions": positions,
            })
        }
        "command_definition.empty" => {
            let c: CommandDefinition = from(&i["yaml"]);
            json!({"belongs_to_command": c.is_belongs_to_command(), "flag_action": c.is_flag_action(),
                   "has_subcommand": c.has_subcommand(), "standalone_executable": c.is_standalone_executable(),
                   "option": opt_model(c.option("a")), "last_argument": opt_model(c.last_argument()),
                   "regular_arguments": c.regular_arguments().into_iter().map(enc_model).collect::<Vec<_>>(),
                   "argument": opt_model(c.argument("a"))})
        }
        "command_definition.new" => {
            let c = CommandDefinition::default();
            json!({"belongs_to_command": c.is_belongs_to_command(), "flag_action": c.is_flag_action(),
                   "standalone_executable": c.is_standalone_executable(),
                   "regular_arguments": c.regular_arguments().into_iter().map(enc_model).collect::<Vec<_>>()})
        }
        "argument_definition" => {
            let a: ArgumentDefinition = from(&i["yaml"]);
            json!({"index_name": opt_enc(a.index("name")), "index_type": opt_enc(a.index("type")),
                   "index_missing": opt_enc(a.index("nope")), "index_private": opt_enc(a.index("format_duration")),
                   "name_sym": sym(a.name_sym()), "type_sym": sym(a.type_sym()), "last": a.is_last(),
                   "parsed": parsed(a.parsed_position()), "numeric": a.numeric_position() as i64})
        }
        "option_definition" => {
            let o: OptionDefinition = from(&i["yaml"]);
            json!({"applies_linux": o.applies_to("linux").unwrap(), "applies_windows": o.applies_to("windows").unwrap(),
                   "boolean": o.is_boolean(), "delim_sym": sym(o.assignment_delimiter_sym()),
                   "index_cli": opt_enc(o.index("cli")), "index_missing": opt_enc(o.index("zz")),
                   "name_sym": sym(o.name_sym())})
        }
        "option_definition.default" => {
            let o: OptionDefinition = from(&i["yaml"]);
            json!({"applies_windows": o.applies_to("windows").unwrap(), "boolean": o.is_boolean(),
                   "delim_sym": sym(o.assignment_delimiter_sym())})
        }
        "flag_definition" => {
            let f: FlagDefinition = from(&i["yaml"]);
            json!({"default_value": f.default_value(), "applies_windows": f.applies_to("windows").unwrap(),
                   "applies_linux": f.applies_to("linux").unwrap(), "name_sym": sym(f.name_sym()),
                   "position_constraint_sym": f.position_constraint_sym().unwrap().map(|s| json!({"$sym": s}))})
        }
        "flag_definition.null_default" => {
            let f: FlagDefinition = from(&i["yaml"]);
            json!({"default_value": f.default_value(), "applies": f.applies_to("x").unwrap(),
                   "position_constraint_sym": f.position_constraint_sym().unwrap()})
        }
        "flag_definition.true" => json!(from::<FlagDefinition>(&i["yaml"]).default_value()),
        "env_var_definition" | "env_var_definition.null" => {
            let e: EnvVarDefinition = from(&i["yaml"]);
            json!(e
                .platforms_sym()
                .unwrap()
                .iter()
                .map(|s| json!({"$sym": s}))
                .collect::<Vec<_>>())
        }
        "invocation" | "invocation.empty" => {
            let v: Invocation = from(&i["yaml"]);
            json!([v.is_direct(), v.is_subcommand(), v.is_multi_call()])
        }
        "version_detection" => {
            let v: VersionDetection = from(&i["yaml"]);
            json!(["command", "pattern", "source", "nope", "modern_threshold"]
                .iter()
                .map(|k| opt_enc(v.index(k)))
                .collect::<Vec<_>>())
        }
        "platform_profile" => {
            let p: PlatformProfile = from(&i["yaml"]);
            let (pl, sh) = (s(&i["platform"]), s(&i["shell"]));
            json!({"supports_platform": p.supports_platform(pl).unwrap(), "supports_shell": p.supports_shell(sh).unwrap(),
                   "compatible": p.is_compatible(pl, sh).unwrap(), "universal": p.is_universal(),
                   "routing": p.has_routing(), "routing_h": enc(&p.routing().to_h()),
                   "command_names": p.command_names().iter().map(enc).collect::<Vec<_>>(),
                   "command": opt_model(p.command("convert")), "command_missing": opt_model(p.command("zz"))})
        }
        "platform_profile.empty" | "platform_profile.null" => {
            let p: PlatformProfile = from(&i["yaml"]);
            json!({"supports_platform": p.supports_platform("x").unwrap(), "universal": p.is_universal(),
                   "routing": p.has_routing(), "routing_h": enc(&p.routing().to_h()),
                   "command_names": p.command_names().iter().map(enc).collect::<Vec<_>>(),
                   "command": opt_model(p.command("a"))})
        }
        "tool_definition" => {
            let mut t: ToolDefinition = from(&i["yaml"]);
            let (pl, sh) = (s(&i["platform"]), s(&i["shell"]));
            let before = t
                .compatible_profile(pl, sh)
                .unwrap()
                .map(|p| enc_text(p.name.as_ref()));
            t.resolve_inheritance();
            let after = t.compatible_profile(pl, sh).unwrap().map(|p| {
                Json::Array(
                    p.commands
                        .iter()
                        .flatten()
                        .map(|c| {
                            json!([enc_text(c.name.as_ref()), enc_text(c.description.as_ref())])
                        })
                        .collect(),
                )
            });
            json!({
                "profile": before, "commands_after": after, "available_on": t.is_available_on(pl).unwrap(),
                "implements": t.is_implements("convert").unwrap(), "implements_str": t.is_implements("identify").unwrap(),
                "implements_missing": t.is_implements("zz").unwrap(),
                "implements_any": t.is_implements_any(&["zz", "identify"]).unwrap(),
                "implements_any_arr": t.is_implements_any(&["zz", "yy"]).unwrap(), "interfaces": texts(&t.interfaces()),
                "schema_version": enc_text(t.schema_version()), "self_uri": enc_text(t.self_uri.as_ref()),
                "schema_version_q": t.is_schema_version("1.0"), "schema_version_q2": t.is_schema_version("2.0"),
                "yaml_after": t.to_yaml().unwrap(),
            })
        }
        "tool_definition.empty" => {
            let mut t: ToolDefinition = from(&i["yaml"]);
            let available = match t.is_available_on("linux") {
                Ok(b) => json!(b),
                Err(e) => json!(format!("raise {}", e.ruby_class())),
            };
            let compatible = t.compatible_profile("linux", "bash").unwrap().is_some();
            json!({"implements": t.is_implements("x").unwrap(), "interfaces": texts(&t.interfaces()),
                   "available": available, "compatible": if compatible { json!("some") } else { Json::Null },
                   "resolve": t.resolve_inheritance().to_yaml().unwrap()})
        }
        "tool_definition.profiles_empty" => {
            let t: ToolDefinition = from(&i["yaml"]);
            json!({"available": t.is_available_on("linux").unwrap(),
                   "compatible": t.compatible_profile("linux", "bash").unwrap().map(|_| "some")})
        }
        "tool_metadata.from_hash" => {
            let m = ToolMetadata::from_hash(
                &json_to_ruby(&i["hash"]),
                s(&i["tool_name"]),
                opt_s(&i["register_path"]),
            )
            .unwrap();
            let symj = |v: Option<String>| v.map_or(Json::Null, |s| json!({"$sym": s}));
            json!({
                "implements_ref": m.implements_ref(), "implements_q": m.is_implements("zip", None).unwrap(),
                "implements_q_ver": m.is_implements("zip", Some("2.0")).unwrap(),
                "implements_q_badver": m.is_implements("zip", Some("9")).unwrap(),
                "implements_gzip": m.is_implements("gzip", None).unwrap(),
                "default_command": symj(m.default_command().unwrap()),
                "may_provide_list": m.may_provide_list().unwrap().iter().map(|s| json!({"$sym": s})).collect::<Vec<_>>(),
                "tool_name": enc_text(m.tool_name().as_ref()), "aliases": enc(&Value::Array(m.aliases())),
                "backing_tool": symj(m.backing_tool().unwrap()), "to_s": m.to_ruby_s(), "inspect": m.inspect(),
                "hash": enc(&m.to_hash().unwrap()), "yaml": m.to_yaml().unwrap(), "json": m.to_json().unwrap(),
                "register_path": enc_text(m.register_path.as_ref()), "implements": enc_text(m.implements.as_ref()),
                "implements_version": enc_text(m.implements_version.as_ref()),
            })
        }
        "tool_metadata.parse_implements" => {
            let v = json_to_ruby(&i["v"]);
            let (a, b) = ToolMetadata::parse_implements(Some(&v));
            json!([a, b])
        }
        "format_duration" => {
            let n = match i["seconds"].as_i64() {
                Some(x) if !i["seconds"].is_f64() => Number::Int(i128::from(x)),
                _ => Number::Float(i["seconds"].as_f64().unwrap()),
            };
            let f = format_duration(n);
            json!({"stage": f, "report": f})
        }
        "format_duration.nil" => json!(ukiryu::models::stage_metrics::format_duration(None)),
        "execution_report" | "execution_report.empty" => {
            let mut r: ExecutionReport = from(&i["yaml"]);
            let stages: Vec<Json> = r
                .all_stages()
                .iter()
                .map(|st| enc_text(st.name.get()))
                .collect();
            r.calculate_total();
            let total = r.total_duration.get().copied().unwrap();
            let total = if stages.is_empty() {
                json!(total as i64)
            } else {
                json!({"$float": ukiryu::models::ruby::float_to_s(total)})
            };
            json!({"stages": stages, "total": total, "formatted": enc_text(r.formatted_total_duration.get()),
                   "yaml": r.to_yaml().unwrap()})
        }
        "execution_report.new" => {
            let mut r = ExecutionReport::default();
            let before = r.to_yaml().unwrap();
            r.calculate_total();
            json!({"before": before, "after": r.to_yaml().unwrap()})
        }
        "error_response.from_message" => {
            let e = ErrorResponse::from_message(s(&i["message"]), i["exit_code"].as_i64().unwrap());
            snapshot(&e)
        }
        "error_response.from_message_default" => {
            json!(ErrorResponse::from_message("x", 1).to_json().unwrap())
        }
        "arguments.from_params" => {
            let cmd: CommandDefinition = from(&i["command_yaml"]);
            let with = i["with_command"].as_bool().unwrap();
            let a =
                Arguments::from_params(&params(i["case"].as_i64().unwrap()), with.then_some(&cmd))
                    .unwrap();
            snapshot(&a)
        }
        "stage_metrics.finish_without_start" => {
            let mut m = StageMetrics::named("x");
            m.finish(0.0, 0, false, Some("boom"));
            let h = m.to_hash().unwrap();
            let keys: Vec<String> = match &h {
                Value::Hash(p) => p.iter().map(|(k, _)| k.to_ruby_s()).collect(),
                _ => vec![],
            };
            json!({"keys": keys, "duration": attr_float(&m.duration), "formatted": attr_text(&m.formatted_duration),
                   "success": attr_bool(&m.success), "error": attr_text(&m.error),
                   "delta_is_after_minus_before": m.memory_delta.get() == Some(&(m.memory_after.get().unwrap() - m.memory_before.get().unwrap())),
                   "hash_formatted": opt_ref(h.get_str("formatted_duration"))})
        }
        "stage_metrics.finish_no_error" => {
            let mut m = StageMetrics::named("y");
            m.finish(0.0, 0, true, None);
            json!({"error": attr_text(&m.error),
                   "success": attr_bool(&m.success), "_hash": enc(&m.to_hash().unwrap())})
        }
        "stage_metrics.start_finish" => {
            let mut m = StageMetrics::named("z");
            let returns_self = std::ptr::eq(m.start(10.0, 100), &m as *const _);
            m.finish(10.25, 120, true, None);
            json!({"returns_self": returns_self,
                   "duration_positive": m.duration.get().copied().unwrap() > 0.0,
                   "formatted_suffix": attr_text(&m.formatted_duration).as_str().unwrap().trim_start_matches(|c: char| !c.is_ascii_alphabetic()),
                   "_hash": enc(&m.to_hash().unwrap())})
        }
        "success_response.from_result" => {
            let cmd: CommandDefinition = from(&i["command_yaml"]);
            let parts = ExecutionResultParts {
                status: 0,
                executable: "/usr/bin/magick".into(),
                full_command: "magick -quality 90 a.png".into(),
                shell: "bash".into(),
                stdout: "out\n".into(),
                error_output: String::new(),
                started_at_iso8601: "2024-01-02T03:04:05Z".into(),
                finished_at_iso8601: "2024-01-02T03:04:06Z".into(),
                duration: 1.25,
                formatted_duration: "1.25s".into(),
            };
            let r = SuccessResponse::from_result(&parts, &params(0), Some(&cmd), None).unwrap();
            let mut j = snapshot(&r);
            j["same_object"] = json!(
                r.request.as_deref() == r.command.as_ref().and_then(|c| c.arguments.as_deref())
            );
            j
        }
        "success_response.from_result_report" => {
            let report: ExecutionReport = from(&i["report_yaml"]);
            let parts = ExecutionResultParts {
                status: 2,
                executable: "x".into(),
                full_command: "x".into(),
                shell: "sh".into(),
                stdout: Vec::new(),
                error_output: "err\n".into(),
                started_at_iso8601: "2024-01-02T00:00:00Z".into(),
                finished_at_iso8601: "2024-01-02T00:00:00Z".into(),
                duration: 0.0,
                formatted_duration: "0ms".into(),
            };
            let r = SuccessResponse::from_result(&parts, &[], None, Some(report)).unwrap();
            json!({"hash": enc(&r.to_hash().unwrap()), "json": r.to_json().unwrap()})
        }
        "validation_result" => {
            let errors: Vec<String> = i["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| s(e).to_owned())
                .collect();
            let v = ValidationResult::new("t", i["valid"].as_bool().unwrap(), errors);
            json!({"status_message": v.status_message(), "valid": v.is_valid(), "invalid": v.is_invalid(),
                   "not_found": v.is_not_found()})
        }
        "validation_result.factories" => json!([
            ValidationResult::valid("a"),
            ValidationResult::invalid("b", vec!["x".into()]),
            ValidationResult::not_found("c")
        ]
        .iter()
        .map(|v| json!([
            v.tool_name,
            v.valid,
            v.errors,
            v.status_message(),
            v.is_not_found()
        ]))
        .collect::<Vec<_>>()),
        "version_info" => {
            let m = i["method"].as_str().map_or(Value::Nil, Value::sym);
            let avail = vec![Value::sym("command"), Value::sym("man_page")];
            let v = VersionInfo::new(Value::str("1.2"), m.clone(), avail.clone());
            json!({"to_s": enc(&v.to_ruby_s()), "to_h": enc(&v.to_h()), "from_command": v.is_from_command(),
                   "from_man_page": v.is_from_man_page(), "from_profile": v.is_from_profile(), "inspect": v.inspect(),
                   "eq_same": v == VersionInfo::new(Value::str("1.2"), m.clone(), avail),
                   "eq_diff": v == VersionInfo::new(Value::str("1.3"), m, vec![]), "eq_other": false})
        }
        "executable_info" => {
            let e = ExecutableInfo::new(
                json_to_ruby(&i["path"]),
                Value::sym(s(&i["source"])),
                json_to_ruby(&i["shell"]),
                json_to_ruby(&i["alias_definition"]),
            );
            json!({"description": e.description(), "alias": e.is_alias(), "path": e.is_path()})
        }
        "execution_profile" => {
            let e = ExecutionProfile::new(
                json_to_ruby(&i["name"]),
                &json_to_ruby(&i["platforms"]),
                &json_to_ruby(&i["shells"]),
                &json_to_ruby(&i["option_style"]),
                json_to_ruby(&i["executable_name"]),
                &json_to_ruby(&i["actions"]),
            )
            .unwrap();
            let syms = |v: &[String]| v.iter().map(|s| json!({"$sym": s})).collect::<Vec<_>>();
            json!({"platforms": syms(&e.platforms), "shells": syms(&e.shells), "option_style": {"$sym": e.option_style},
                   "compatible": e.is_compatible(&Value::sym("linux"), &Value::sym("bash")),
                   "compatible_str": e.is_compatible(&Value::str("linux"), &Value::str("bash")),
                   "compatible_no": e.is_compatible(&Value::sym("windows"), &Value::sym("bash")),
                   "command": opt_ref(e.command(&Value::sym("a"))), "command_str": opt_ref(e.command(&Value::str("a"))),
                   "commands": enc(e.commands()), "to_s": e.to_ruby_s(), "inspect": e.inspect(), "frozen": true})
        }
        other => panic!("unmapped helper group {other}"),
    }
}

/// Groups whose expected value is a snapshot (yaml compared structurally).
fn is_snapshot(group: &str) -> bool {
    matches!(
        group,
        "error_response.from_message" | "arguments.from_params" | "success_response.from_result"
    )
}

fn adjust(group: &str, expected: &Json, actual: &mut Json) {
    match group {
        // Key order of to_hash is checked through the captured key list.
        "stage_metrics.finish_no_error" | "stage_metrics.start_finish" => {
            let keys: Vec<String> = match actual["_hash"].as_object() {
                Some(o) => o.keys().cloned().collect(),
                None => vec![],
            };
            actual["keys"] = json!(keys);
            actual.as_object_mut().unwrap().remove("_hash");
        }
        "tool_definition" => {
            let e = expected["yaml_after"].as_str().unwrap();
            if yaml_equiv(e, actual["yaml_after"].as_str().unwrap()) {
                actual["yaml_after"] = json!(e);
            }
        }
        "tool_definition.empty" => {
            let e = expected["resolve"].as_str().unwrap();
            if yaml_equiv(e, actual["resolve"].as_str().unwrap()) {
                actual["resolve"] = json!(e);
            }
        }
        "execution_report" | "execution_report.empty" => {
            let e = expected["yaml"].as_str().unwrap();
            if yaml_equiv(e, actual["yaml"].as_str().unwrap()) {
                actual["yaml"] = json!(e);
            }
        }
        "execution_report.new" => {
            for k in ["before", "after"] {
                if yaml_equiv(expected[k].as_str().unwrap(), actual[k].as_str().unwrap()) {
                    actual[k] = expected[k].clone();
                }
            }
        }
        "tool_metadata.from_hash" => {
            let e = expected["yaml"].as_str().unwrap();
            if yaml_equiv(e, actual["yaml"].as_str().unwrap()) {
                actual["yaml"] = json!(e);
            }
        }
        // `frozen?` has no Rust counterpart; the object is immutable by construction.
        "execution_profile" => actual["frozen"] = expected["frozen"].clone(),
        // `v == '1.2'` (a String) is false by type; the Rust API only compares VersionInfo values.
        "version_info" => actual["eq_other"] = expected["eq_other"].clone(),
        _ => {}
    }
}

#[test]
fn model_helpers_match_the_oracle() {
    let golden = golden("helpers.json");
    let mut failures = Vec::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for row in golden["rows"].as_array().unwrap() {
        let group = row["group"].as_str().unwrap();
        let expected = &row["expect"]["ok"];
        assert!(
            !expected.is_null(),
            "{group}: oracle rows are successful calls"
        );
        let mut actual = evaluate(group, &row["input"]);
        adjust(group, expected, &mut actual);
        let ok = if is_snapshot(group) {
            snap_same(expected, &actual)
        } else {
            same(expected, &actual)
        };
        if !ok {
            failures.push(format!(
                "{group} {}\n  ruby: {expected}\n  rust: {actual}",
                row["input"]
            ));
        }
        *counts.entry(group.to_owned()).or_default() += 1;
    }
    println!(
        "rows: {}, groups: {}",
        counts.values().sum::<usize>(),
        counts.len()
    );
    for f in failures.iter().take(30) {
        println!("FAIL {f}");
    }
    report_integral_floats();
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}

#[test]
fn every_ruby_model_file_has_a_rust_module() {
    let golden = golden("helpers.json");
    let sources = golden["sources"].as_array().unwrap();
    assert_eq!(
        sources.len(),
        31,
        "lib/ukiryu/models/*.rb at the pinned oracle"
    );
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/models");
    for s in sources {
        let path = s["path"].as_str().unwrap();
        let stem = path.rsplit('/').next().unwrap().trim_end_matches(".rb");
        assert!(
            src.join(format!("{stem}.rs")).exists(),
            "no Rust module for {path}"
        );
    }
}
