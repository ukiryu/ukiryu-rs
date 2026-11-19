//! `WhichCommand`: exact name first, then compatible interface/alias
//! candidates in register order, preferring an available executable.
use super::*;
use ukiryu::runtime::ShellName;
use ukiryu::tool::DefinitionSource;

pub fn which(input: WhichInput, ctx: &HandlerContext) -> Result<CommandOutcome, InspectError> {
    let config = ctx.configuration()?;
    let mut out = Out::new(colors_disabled(&config));
    let register = setup_register(ctx, &config)
        .map_err(|error| error.into_error(Vec::new()))?
        .register;
    let runtime = tool_options(ctx, &config).map_err(|error| InspectError::Shell {
        partial: Vec::new(),
        error,
    })?;
    let platform = input
        .platform
        .as_deref()
        .unwrap_or(runtime.platform.as_str());
    let shell = input.shell.as_deref().unwrap_or(runtime.shell.as_str());
    let options = Platform::from_name(platform)
        .zip(ShellName::lookup(shell))
        .map(|(platform, shell)| {
            ToolOptions::new(platform, shell).discovery(DiscoveryContext::new(
                ctx.host.platform,
                shell.as_str(),
                ctx.env("PATH"),
                ctx.env("PATHEXT"),
            ))
        });
    let source = RegisterSource::new(
        register.clone(),
        ctx.host.platform,
        ProcessHost::new(Environment::new(), runtime.shell),
    );
    let cache = ToolCache::new();
    out.clear();
    out.say(
        &format!("Resolving: {}", input.identifier),
        Some(Color::Cyan),
    );
    out.say(&format!("  Platform: {platform}"), Some(Color::White));
    out.say(&format!("  Shell: {shell}"), Some(Color::White));
    out.clear();

    let get = |name: &str| -> Result<Option<Rc<Tool>>, InspectError> {
        let result = match options.as_ref() {
            Some(options) => tool::get(&source, &cache, name, options).map(Some),
            None => match source.load_definition_for(name, platform, shell) {
                Some(Ok(None)) => Ok(None),
                Some(Err(error)) => Err(error),
                // Do not invent a concrete platform/shell for an arbitrary
                // profile group. The core likewise refuses this target.
                _ => {
                    return Err(gated(
                        "which target",
                        "a concrete platform and shell",
                        Vec::new(),
                    ))
                }
            },
        };
        match result {
            Ok(tool) => Ok(tool),
            Err(ToolError::NotFound { .. }) => Ok(None),
            Err(error) => Err(tool_failure(&error, Vec::new())),
        }
    };
    let selected = match get(&input.identifier).map_err(|error| with_partial(error, &out))? {
        Some(tool) => {
            out.say("Match type: Exact name match", Some(Color::Green));
            Some(tool)
        }
        None => {
            let index = register_lookup::load_tool_index(&register);
            let mut candidates = Vec::new();
            for name in register.tool_names() {
                let Some(metadata) = register_lookup::load_tool_metadata(&register, &index, &name)
                    .map_err(|error| lookup_failure(error, out.writes.clone()))?
                else {
                    continue;
                };
                let interface_match =
                    metadata
                        .is_implements(&input.identifier, None)
                        .map_err(|error| InspectError::Raised {
                            class: error.ruby_class(),
                            message: error.message,
                            partial: out.writes.clone(),
                        })?;
                let alias_match = metadata
                    .aliases()
                    .iter()
                    .any(|alias| alias.as_str() == Some(input.identifier.as_str()));
                if !interface_match && !alias_match {
                    continue;
                }
                let metadata_name = txt(&metadata.name).unwrap_or_default();
                let Some(definition) =
                    register_lookup::load_tool_definition(&register, &metadata_name, None)
                        .map_err(|error| lookup_failure(error, out.writes.clone()))?
                else {
                    continue;
                };
                if definition
                    .compatible_profile(platform, shell)
                    .map_err(|error| InspectError::Raised {
                        class: error.ruby_class(),
                        message: error.message,
                        partial: out.writes.clone(),
                    })?
                    .is_some()
                {
                    candidates.push((
                        name,
                        if interface_match {
                            "interface"
                        } else {
                            "alias"
                        },
                    ));
                }
            }
            let mut selected = None;
            for candidate in &candidates {
                if get(&candidate.0)
                    .map_err(|error| with_partial(error, &out))?
                    .is_some_and(|tool| tool.is_available())
                {
                    selected = Some(candidate);
                    break;
                }
            }
            match selected.or_else(|| candidates.first()) {
                Some((name, kind)) => {
                    out.say(&format!("Match type: {kind} match"), Some(Color::Green));
                    get(name).map_err(|error| with_partial(error, &out))?
                }
                None => None,
            }
        }
    };
    let Some(tool) = selected else {
        let mut names = register.tool_names();
        names.sort();
        return Ok(out.error(&format!(
            "No tool found for: {}\nAvailable tools: {}",
            input.identifier,
            names.join(", ")
        )));
    };
    out.clear();
    out.say("Selected tool:", Some(Color::Yellow));
    let name = tool.name().unwrap_or_default();
    if name != input.identifier {
        out.say(
            &format!("  Query: {}", input.identifier),
            Some(Color::White),
        );
        out.say(&format!("  Resolved to: {name}"), Some(Color::White));
    } else {
        out.say(&format!("  Tool: {name}"), Some(Color::White));
    }
    if let Some(implements) = tool.definition().implements.as_ref() {
        let value = ModelValue::Array(implements.iter().map(Text::to_ruby_value).collect());
        out.say(
            &format!("  Implements: {}", ukiryu::types::ruby::inspect(&value)),
            Some(Color::White),
        );
    }
    let profile = tool.command_profile();
    out.say(
        &format!(
            "  Profile: {}",
            txt(&profile.name).unwrap_or_else(|| "default".into())
        ),
        Some(Color::White),
    );
    for (label, values) in [
        ("Platforms", &profile.platforms),
        ("Shells", &profile.shells),
    ] {
        let values = values
            .as_deref()
            .map(|values| join_texts(values, ", "))
            .unwrap_or_else(|| "all".into());
        out.say(&format!("    {label}: {values}"), Some(Color::Dim));
    }
    out.clear();
    if tool.is_available() {
        out.say("Status: AVAILABLE", Some(Color::Green));
        out.say(
            &format!("  Executable: {}", tool.executable().unwrap_or("")),
            Some(Color::White),
        );
        if let Some(options) = options.as_ref() {
            if let Some(version) = tool_version(&tool, options, &mut out.writes)? {
                out.say(&format!("  Version: {version}"), Some(Color::White));
            }
        }
    } else {
        out.say("Status: NOT AVAILABLE", Some(Color::Red));
        out.say("  Tool is not installed or not in PATH", Some(Color::Dim));
    }
    Ok(out.done())
}

fn with_partial(mut error: InspectError, out: &Out) -> InspectError {
    match &mut error {
        InspectError::Raised { partial, .. }
        | InspectError::Gated { partial, .. }
        | InspectError::Source { partial, .. } => *partial = out.writes.clone(),
        _ => {}
    }
    error
}
