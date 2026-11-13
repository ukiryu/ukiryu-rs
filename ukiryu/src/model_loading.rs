//! Typed model construction for the register loader.
//!
//! Source: lib/ukiryu/tool/loader.rb (`Ukiryu::Tool::Loader`) at ukiryu
//! ab6df353, which builds the legacy tool model with lutaml keyword
//! constructors:
//!
//! * `Models::CommandDefinition.new(name: ..., options: ..., ...)` in
//!   `convert_hash_to_command_definition`;
//! * `Models::PlatformProfile.new(**profile_data, commands: ...)` in
//!   `convert_profile_to_platform_profile`;
//! * `Models::ToolDefinition.new(name:, version:, display_name:, implements:,
//!   profiles:, version_detection:, aliases:)` in `convert_to_tool_definition`.
//!
//! The register documents themselves load through the model entry points
//! (`register_hash`, `Interface`/`ImplementationIndex`/`ImplementationVersion`
//! `::from_register_yaml`/`from_hash`); this module does not repeat them.
//!
//! lutaml `Model.new(**kwargs)` differs from `Model.from_hash` in two ways that
//! show up in the loader's output, both measured against the oracle:
//!
//! * a key passed as `nil` stores `nil` and does not fall back to the
//!   declared default (`PlatformProfile.new(option_style: nil).option_style`
//!   is `nil`, not `"single_dash_space"`);
//! * a declared default for a key that was not passed stays implicit, so
//!   `to_hash`/`to_yaml` omit it. Nested Hashes (command options, flags,
//!   arguments, ...) are cast with the same constructor semantics;
//! * keys are attribute names, not YAML keys: `routing_data:` and
//!   `self_uri:` are taken, while `routing:` and `$self` are ignored.
//!
//! The loader's Hash helpers (`deep_merge_hashes`, `convert_actions_to_array`,
//! `flatten_signature`, `extract_profile_data`) and the orchestration stay with
//! the register/loader owner; these functions only turn their Hashes into
//! typed models.
use crate::models::{
    ArgumentDefinition, Attr, CommandDefinition, EnvVarDefinition, ExitCodes, FlagDefinition,
    Model, ModelError, OptionDefinition, PlatformProfile, ToolDefinition, Value, VersionDetection,
};

/// `Hash#key?` for a Symbol or String key (a key passed as `nil` counts).
fn passed(kwargs: &Value, key: &str) -> bool {
    match kwargs {
        Value::Hash(pairs) => pairs.iter().any(|(k, _)| match k {
            Value::Symbol(s) | Value::String(s) => s == key,
            _ => false,
        }),
        _ => false,
    }
}

/// The value passed for `key` (Symbol key first).
fn arg<'a>(kwargs: &'a Value, key: &str) -> Option<&'a Value> {
    match kwargs {
        Value::Hash(pairs) => pairs
            .iter()
            .find(|(k, _)| matches!(k, Value::Symbol(s) if s == key))
            .or_else(|| {
                pairs
                    .iter()
                    .find(|(k, _)| matches!(k, Value::String(s) if s == key))
            })
            .map(|(_, v)| v),
        _ => None,
    }
}

/// A declared default for a key that was not passed stays implicit.
fn implicit<T>(attr: &mut Attr<T>, kwargs: &Value, key: &str) {
    if !passed(kwargs, key) && matches!(attr, Attr::Set(_)) {
        if let Attr::Set(v) = std::mem::replace(attr, Attr::Null) {
            *attr = Attr::Default(v);
        }
    }
}

/// Applies `fix` to each built item with the Hash it was cast from.
fn each_item<T>(items: &mut Option<Vec<T>>, source: Option<&Value>, fix: impl Fn(&mut T, &Value)) {
    let (Some(items), Some(source)) = (items.as_mut(), source) else {
        return;
    };
    match source {
        Value::Array(sources) => {
            for (item, src) in items.iter_mut().zip(sources) {
                fix(item, src);
            }
        }
        // A single Hash given for a collection wraps into one item.
        Value::Hash(_) => {
            if let Some(item) = items.first_mut() {
                fix(item, source);
            }
        }
        _ => {}
    }
}

fn option_new(o: &mut OptionDefinition, kw: &Value) {
    implicit(&mut o.r#type, kw, "type");
    implicit(&mut o.assignment_delimiter, kw, "assignment_delimiter");
}

fn flag_new(f: &mut FlagDefinition, kw: &Value) {
    implicit(&mut f.default, kw, "default");
}

fn argument_new(a: &mut ArgumentDefinition, kw: &Value) {
    implicit(&mut a.r#type, kw, "type");
    implicit(&mut a.required, kw, "required");
    implicit(&mut a.position, kw, "position");
    implicit(&mut a.variadic, kw, "variadic");
    implicit(&mut a.separator, kw, "separator");
}

fn env_var_new(_e: &mut EnvVarDefinition, _kw: &Value) {}

fn exit_codes_new(e: &mut ExitCodes, kw: &Value) {
    implicit(&mut e.standard, kw, "standard");
    implicit(&mut e.custom, kw, "custom");
}

fn key_is(k: &Value, names: &[&str]) -> bool {
    matches!(k, Value::Symbol(s) | Value::String(s) if names.contains(&s.as_str()))
}

/// Translates constructor keywords into the YAML mapping `from_ruby` reads:
/// drops `ignored` keys (typed arguments and YAML-only keys `new` does not
/// know) and renames attributes whose YAML key differs.
fn mapping(kwargs: &Value, ignored: &[&str], renames: &[(&str, &str)]) -> Value {
    let Value::Hash(pairs) = kwargs else {
        return kwargs.clone();
    };
    let mut out = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        if key_is(k, ignored) {
            continue;
        }
        let k = match renames.iter().find(|(attr, _)| key_is(k, &[*attr])) {
            Some((_, key)) => Value::sym(*key),
            None => k.clone(),
        };
        out.retain(|(o, _): &(Value, Value)| o != &k);
        out.push((k, v.clone()));
    }
    Value::Hash(out)
}

fn build<T: Model>(kwargs: &Value) -> Result<T, ModelError> {
    T::from_ruby(kwargs)
}

/// `Models::CommandDefinition.new(**kwargs)` (the loader's
/// `convert_hash_to_command_definition` call), with nested option, flag,
/// argument, env var and exit code Hashes cast to their models.
pub fn command_definition_new(kwargs: &Value) -> Result<CommandDefinition, ModelError> {
    let mut cmd: CommandDefinition = build(kwargs)?;
    implicit(
        &mut cmd.standalone_executable,
        kwargs,
        "standalone_executable",
    );
    each_item(&mut cmd.options, arg(kwargs, "options"), option_new);
    each_item(
        &mut cmd.post_options,
        arg(kwargs, "post_options"),
        option_new,
    );
    each_item(&mut cmd.flags, arg(kwargs, "flags"), flag_new);
    each_item(&mut cmd.arguments, arg(kwargs, "arguments"), argument_new);
    each_item(&mut cmd.env_vars, arg(kwargs, "env_vars"), env_var_new);
    if let Some(codes) = cmd.exit_codes.as_mut() {
        if let Some(kw) = arg(kwargs, "exit_codes") {
            exit_codes_new(codes, kw);
        }
    }
    Ok(cmd)
}

/// `Models::PlatformProfile.new(**kwargs, commands: commands)` (the loader's
/// `convert_profile_to_platform_profile` call). A `commands` entry in
/// `kwargs` is ignored; the typed `commands` win, as in the source call.
/// Keys are attribute names (`routing_data`, not the YAML key `routing`).
pub fn platform_profile_new(
    kwargs: &Value,
    commands: Vec<CommandDefinition>,
) -> Result<PlatformProfile, ModelError> {
    let fields = mapping(
        kwargs,
        &["commands", "routing"],
        &[("routing_data", "routing")],
    );
    let mut profile: PlatformProfile = build(&fields)?;
    implicit(&mut profile.option_style, &fields, "option_style");
    implicit(&mut profile.routing_data, &fields, "routing");
    implicit(&mut profile.env_var_sets, &fields, "env_var_sets");
    if let Some(codes) = profile.exit_codes.as_mut() {
        if let Some(kw) = arg(&fields, "exit_codes") {
            exit_codes_new(codes, kw);
        }
    }
    profile.commands = Some(commands);
    Ok(profile)
}

/// `Models::ToolDefinition.new(**kwargs, profiles: profiles,
/// version_detection: version_detection)` (the loader's
/// `convert_to_tool_definition` call, before `resolve_inheritance!`).
/// `profiles`/`version_detection` entries in `kwargs` are ignored; keys are
/// attribute names (`self_uri`, not the YAML key `$self`).
pub fn tool_definition_new(
    kwargs: &Value,
    profiles: Vec<PlatformProfile>,
    version_detection: Option<VersionDetection>,
) -> Result<ToolDefinition, ModelError> {
    let fields = mapping(
        kwargs,
        &["profiles", "version_detection", "$self"],
        &[("self_uri", "$self")],
    );
    let mut tool: ToolDefinition = build(&fields)?;
    tool.profiles = Some(profiles);
    tool.version_detection = version_detection.map(Box::new);
    Ok(tool)
}
