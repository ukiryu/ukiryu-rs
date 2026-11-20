use clap::{builder::OsStringValueParser, Arg, ArgAction, Command};

fn value(name: &'static str, short: Option<char>, help: &'static str) -> Arg {
    let arg = Arg::new(name)
        .long(name)
        .help(help)
        .value_parser(OsStringValueParser::new());
    match short {
        Some(short) => arg.short(short),
        None => arg,
    }
}

fn flag(name: &'static str, short: Option<char>, help: &'static str) -> Arg {
    let arg = Arg::new(name)
        .long(name)
        .help(help)
        .action(ArgAction::Set)
        .value_parser(clap::value_parser!(bool))
        .num_args(0..=1)
        .require_equals(true)
        .default_missing_value("true")
        .default_value("false");
    match short {
        Some(short) => arg.short(short),
        None => arg,
    }
}

fn positional(name: &'static str, required: bool) -> Arg {
    Arg::new(name)
        .required(required)
        .value_parser(OsStringValueParser::new())
}

fn register(command: Command) -> Command {
    command.arg(value("register", Some('r'), "Path to tool register"))
}

fn response(command: Command) -> Command {
    command
        // Do not insert defaults here: Configuration owns CLI > ENV > programmatic
        // > defaults. A parser default would incorrectly mask UKIRYU_FORMAT.
        .arg(value(
            "format",
            Some('f'),
            "Response format (yaml, json, table, raw)",
        ))
        .arg(value("output", Some('o'), "Output file (default: stdout)"))
}

/// Build the complete public command grammar without reading process state.
///
/// Unknown options are errors, as in FriendlyCLI. Literal arguments beginning
/// with `-` must follow `--`; those bytes are retained in exec's arguments.
pub fn command() -> Command {
    boolean_options(
        Command::new("ukiryu")
            .version(ukiryu::VERSION)
            .about("Platform-adaptive command execution from declarative tool profiles")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .disable_help_subcommand(false)
            .subcommand(
                response(register(Command::new("run-file")))
                    .about("Execute a structured execution request from YAML")
                    .arg(positional("request-file", true))
                    .arg(flag(
                        "dry-run",
                        Some('d'),
                        "Show the request without executing",
                    ))
                    .arg(value("shell", None, "Shell for command execution")),
            )
            .subcommand(
                response(register(Command::new("exec")))
                    .about("Execute TOOL [COMMAND] [KEY=VALUE ...]")
                    .arg(positional("tool", true))
                    // Keep command/default-command disambiguation in the handler:
                    // the first token may be KEY=VALUE, or the literal tool command.
                    .arg(positional("arguments", false).num_args(0..))
                    .arg(value("definition", Some('d'), "Tool definition file"))
                    .arg(flag(
                        "dry-run",
                        Some('D'),
                        "Show the request without executing",
                    ))
                    .arg(value("shell", Some('s'), "Shell for command execution"))
                    .arg(flag("stdin", None, "Read input from stdin"))
                    .arg(flag("raw", None, "Output raw stdout and stderr")),
            )
            .subcommand(register(Command::new("list").about("List available tools")))
            .subcommand(
                register(Command::new("info").about("Show tool information"))
                    .arg(positional("tool", false))
                    .arg(flag("all", None, "Show all implementations for interfaces")),
            )
            .subcommand(
                register(Command::new("commands").about("List commands for a tool"))
                    .arg(positional("tool", true)),
            )
            .subcommand(
                register(Command::new("opts").about("Show tool or command options"))
                    .arg(positional("tool", true))
                    .arg(positional("command", false)),
            )
            .subcommand(
                register(Command::new("describe").about("Describe a tool or command"))
                    .arg(positional("tool", true))
                    .arg(positional("command", false))
                    .arg(value(
                        "format",
                        Some('f'),
                        "Output format (text, yaml, json)",
                    )),
            )
            .subcommand(
                register(Command::new("system").about("Show system information"))
                    .arg(positional("subcommand", false)),
            )
            .subcommand(Command::new("version").about("Show Ukiryu version"))
            .subcommand(
                register(Command::new("which").about("Show selected tool implementation"))
                    .arg(positional("identifier", true))
                    .arg(value("platform", None, "Platform to check"))
                    .arg(value("shell", None, "Shell to check")),
            )
            .subcommand(
                register(Command::new("config").about("List, get, set or unset configuration"))
                    .arg(positional("action", false))
                    .arg(positional("key", false))
                    .arg(positional("value", false)),
            )
            .subcommand(validate())
            .subcommand(
                Command::new("extract")
                    .about("Extract a definition from an installed CLI tool")
                    .arg(positional("tool", true))
                    .arg(value("output", Some('o'), "Output file for the definition"))
                    .arg(value(
                        "method",
                        Some('m'),
                        "Extraction method (auto, native, help)",
                    ))
                    .arg(flag("verbose", Some('v'), "Enable verbose output")),
            )
            .subcommand(definitions())
            .subcommand(
                Command::new("cache")
                    .about("Manage definition cache")
                    .arg(flag("verbose", None, "Enable verbose output").global(true))
                    .arg(flag("dry-run", None, "Show changes without applying").global(true))
                    .subcommand(Command::new("info").about("Show cache information"))
                    .subcommand(Command::new("stats").about("Show cache statistics"))
                    .subcommand(Command::new("clear").about("Clear cached definitions")),
            )
            .subcommand(
                Command::new("resolve")
                    .about("Resolve the definition for a tool")
                    .arg(positional("tool", true))
                    .arg(positional("version-constraint", false)),
            )
            .subcommand(
                Command::new("register")
                    .about("Manage the tool register")
                    .arg(positional("subcommand", false))
                    .arg(flag("force", Some('f'), "Force re-clone the register"))
                    .arg(flag("verbose", Some('v'), "Show verbose output")),
            ),
    )
}

/// Preserve Thor's negated flags and last-value-wins behavior at each level.
fn boolean_options(mut command: Command) -> Command {
    command = command.args_override_self(true);
    for (positive, negative) in [
        ("dry-run", "no-dry-run"),
        ("stdin", "no-stdin"),
        ("raw", "no-raw"),
        ("all", "no-all"),
        ("verbose", "no-verbose"),
        ("force", "no-force"),
        ("strict", "no-strict"),
        ("executable", "no-executable"),
    ] {
        let global = command
            .get_arguments()
            .find(|argument| argument.get_id() == positive)
            .map(|argument| argument.is_global_set());
        if let Some(global) = global {
            command = command
                .mut_arg(positive, |argument| argument.overrides_with(negative))
                .arg(
                    Arg::new(negative)
                        .long(negative)
                        .action(ArgAction::SetTrue)
                        .global(global)
                        .overrides_with(positive)
                        .help("Disable the corresponding option"),
                );
        }
    }
    let children: Vec<_> = command
        .get_subcommands()
        .map(|child| child.get_name().to_owned())
        .collect();
    for name in children {
        command = command.mut_subcommand(name, boolean_options);
    }
    command
}

fn validate() -> Command {
    Command::new("validate")
        .about("Validate tool definitions")
        .arg(flag("verbose", None, "Enable verbose output").global(true))
        .arg(value("format", None, "Output format (text, json)").global(true))
        .arg(value("schema", None, "JSON Schema file").global(true))
        .arg(flag("strict", None, "Treat warnings as errors").global(true))
        .arg(flag("executable", None, "Test against the executable").global(true))
        .arg(flag("all", None, "Validate all definitions").global(true))
        .arg(value("register", None, "Tool register path").global(true))
        .subcommand(
            Command::new("file")
                .about("Validate a definition file")
                .arg(positional("path", true)),
        )
        .subcommand(Command::new("all").about("Validate all definitions in the register"))
        .subcommand(
            Command::new("string")
                .about("Validate a YAML string")
                .arg(positional("yaml", true)),
        )
}

fn definitions() -> Command {
    Command::new("definitions")
        .about("Manage tool definitions")
        .subcommand(
            Command::new("list")
                .about("List discovered definitions")
                .arg(flag("verbose", Some('v'), "Show detailed information")),
        )
        .subcommand(Command::new("path").about("Show definition search paths"))
        .subcommand(
            Command::new("add")
                .about("Add a definition to the user library")
                .arg(positional("source", true))
                .arg(value("name", Some('n'), "Tool name"))
                .arg(value("version", Some('V'), "Version")),
        )
        .subcommand(
            Command::new("remove")
                .about("Remove a definition from the user library")
                .arg(positional("tool", true))
                .arg(value("version", Some('v'), "Specific version"))
                .arg(flag("force", Some('f'), "Skip confirmation")),
        )
        .subcommand(
            Command::new("info")
                .about("Show definition details")
                .arg(positional("tool", true))
                .arg(value("version", Some('v'), "Specific version")),
        )
}
