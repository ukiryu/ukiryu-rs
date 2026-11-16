//! Thor's help text for the source CLI.
//!
//! The source is `Ukiryu::Cli` (ukiryu/ukiryu ab6df35, cli.rb) on Thor 1.5.0.
//! Its help output is the contract, so this module reproduces Thor's own
//! rendering from the commands' `desc` and `method_option` declarations:
//!
//! - [`root_help`]: `Thor.help`, the sorted command table under
//!   `ukiryu commands:`, truncated to the terminal width;
//! - [`command_help`]: `Thor.command_help`, the usage line, the option table
//!   and the description of one command.
//!
//! Parsing stays with clap ([`crate::arguments`]); only the text is Thor's.
//! The subcommand groups (`validate`, `definitions`, `cache`) and the target
//! addition `lint` keep clap's help.

/// The version the source gem reports (`Ukiryu::VERSION` at ab6df35). The
/// `version` verb prints it, as the oracle does; the crate's own package
/// version is separate.
pub const COMPATIBILITY_VERSION: &str = "0.3.1";

/// The program name in usage lines (`basename` of the source executable).
const PROGRAM: &str = "ukiryu";

/// `Thor::Shell::Terminal::DEFAULT_TERMINAL_WIDTH`.
const DEFAULT_WIDTH: usize = 80;

/// The kind of a Thor option, which decides its usage sample and default.
enum Kind {
    /// `type: :string`: `--name=BANNER`, with the default shown when set.
    Value {
        banner: &'static str,
        default: Option<&'static str>,
    },
    /// `type: :boolean`: `--name`, its `--no-` and `--skip-` forms, and the
    /// default always shown.
    Flag { default: bool },
}

/// One `method_option`, in declaration order.
struct Opt {
    alias: Option<char>,
    /// The dashed switch name, without the leading `--`.
    name: &'static str,
    kind: Kind,
    description: &'static str,
}

/// One command of the source CLI.
struct Verb {
    name: &'static str,
    /// The `desc` usage, without the program name.
    usage: &'static str,
    description: &'static str,
    options: &'static [Opt],
    /// Whether `help VERB` renders [`command_help`] here (a plain command
    /// that this binary also parses); groups and Thor built-ins are listed
    /// in the root table only.
    command_help: bool,
}

const REGISTER: Opt = Opt {
    alias: Some('r'),
    name: "register",
    kind: Kind::Value {
        banner: "REGISTER",
        default: None,
    },
    description: "Path to tool register",
};

const RESPONSE_FORMAT: Opt = Opt {
    alias: Some('f'),
    name: "format",
    kind: Kind::Value {
        banner: "FORMAT",
        default: Some("yaml"),
    },
    description: "Response format (yaml, json, table, raw)",
};

const RESPONSE_OUTPUT: Opt = Opt {
    alias: Some('o'),
    name: "output",
    kind: Kind::Value {
        banner: "OUTPUT",
        default: None,
    },
    description: "Output file for response (default: stdout)",
};

const EXECUTION_SHELL: &str =
    "Shell to use for command execution (bash, zsh, fish, sh, powershell, cmd)";

const DRY_RUN: &str = "Show execution request without executing";

/// Every command of `Ukiryu::Cli`, sorted as `Thor.help` sorts its table.
const VERBS: &[Verb] = &[
    Verb {
        name: "cache",
        usage: "cache [ACTION]",
        description: "Manage definition cache",
        options: &[],
        command_help: false,
    },
    Verb {
        name: "commands",
        usage: "commands TOOL",
        description: "List all commands available for a tool",
        options: &[REGISTER],
        command_help: true,
    },
    Verb {
        name: "config",
        usage: "config [ACTION] [KEY] [VALUE]",
        description: "Manage configuration (list, get, set, unset)",
        options: &[REGISTER],
        command_help: true,
    },
    Verb {
        name: "definitions",
        usage: "definitions [COMMAND]",
        description: "Manage tool definitions",
        options: &[],
        command_help: false,
    },
    Verb {
        name: "describe",
        usage: "describe TOOL [COMMAND]",
        description: "Show comprehensive documentation for a tool or specific command",
        options: &[
            REGISTER,
            Opt {
                alias: Some('f'),
                name: "format",
                kind: Kind::Value {
                    banner: "FORMAT",
                    default: Some("text"),
                },
                description: "Output format (text, yaml, json)",
            },
        ],
        command_help: true,
    },
    Verb {
        name: "exec",
        usage: "exec TOOL [COMMAND] [KEY=VALUE ...]",
        description: "Execute a tool command inline",
        options: &[
            REGISTER,
            Opt {
                alias: Some('d'),
                name: "definition",
                kind: Kind::Value {
                    banner: "DEFINITION",
                    default: None,
                },
                description: "Path to tool definition file",
            },
            RESPONSE_FORMAT,
            RESPONSE_OUTPUT,
            Opt {
                alias: Some('D'),
                name: "dry-run",
                kind: Kind::Flag { default: false },
                description: DRY_RUN,
            },
            Opt {
                alias: Some('s'),
                name: "shell",
                kind: Kind::Value {
                    banner: "SHELL",
                    default: None,
                },
                description: EXECUTION_SHELL,
            },
            Opt {
                alias: None,
                name: "stdin",
                kind: Kind::Flag { default: false },
                description: "Read input from stdin (pass to command)",
            },
            Opt {
                alias: None,
                name: "raw",
                kind: Kind::Flag { default: false },
                description: "Output raw stdout/stderr (for pipe composition)",
            },
        ],
        command_help: true,
    },
    Verb {
        name: "extract",
        usage: "extract TOOL",
        description: "Extract tool definition from an installed CLI tool",
        options: &[
            Opt {
                alias: Some('o'),
                name: "output",
                kind: Kind::Value {
                    banner: "OUTPUT",
                    default: None,
                },
                description: "Output file for extracted definition",
            },
            Opt {
                alias: Some('m'),
                name: "method",
                kind: Kind::Value {
                    banner: "METHOD",
                    default: Some("auto"),
                },
                description: "Extraction method (auto, native, help)",
            },
            Opt {
                alias: Some('v'),
                name: "verbose",
                kind: Kind::Flag { default: false },
                description: "Enable verbose output",
            },
        ],
        command_help: true,
    },
    Verb {
        name: "help",
        usage: "help [COMMAND]",
        description: "Describe available commands or one specific command",
        options: &[],
        command_help: false,
    },
    Verb {
        name: "info",
        usage: "info [TOOL]",
        description: "Show detailed information about a tool",
        options: &[
            REGISTER,
            Opt {
                alias: None,
                name: "all",
                kind: Kind::Flag { default: false },
                description: "Show all implementations for interfaces",
            },
        ],
        command_help: true,
    },
    Verb {
        name: "list",
        usage: "list",
        description: "List all available tools in the register",
        options: &[REGISTER],
        command_help: true,
    },
    Verb {
        name: "opts",
        usage: "opts TOOL [COMMAND]",
        description: "Show options for a tool or specific command",
        options: &[REGISTER],
        command_help: true,
    },
    Verb {
        name: "register",
        usage: "register [SUBCOMMAND]",
        description: "Manage the tool register",
        options: &[
            Opt {
                alias: Some('f'),
                name: "force",
                kind: Kind::Flag { default: false },
                description: "Force re-clone the register",
            },
            Opt {
                alias: Some('v'),
                name: "verbose",
                kind: Kind::Flag { default: false },
                description: "Show verbose output",
            },
        ],
        command_help: true,
    },
    Verb {
        name: "resolve",
        usage: "resolve TOOL [VERSION]",
        description: "Resolve which definition would be used for a tool",
        options: &[],
        command_help: true,
    },
    Verb {
        name: "run-file",
        usage: "run-file REQUEST_FILE",
        description: "Execute a Ukiryu Structured Execution Request from a YAML file",
        options: &[
            REGISTER,
            RESPONSE_OUTPUT,
            RESPONSE_FORMAT,
            Opt {
                alias: Some('d'),
                name: "dry-run",
                kind: Kind::Flag { default: false },
                description: DRY_RUN,
            },
            Opt {
                alias: None,
                name: "shell",
                kind: Kind::Value {
                    banner: "SHELL",
                    default: None,
                },
                description: EXECUTION_SHELL,
            },
        ],
        command_help: true,
    },
    Verb {
        name: "system",
        usage: "system [SUBCOMMAND]",
        description: "Show system information (shells, etc.)",
        options: &[REGISTER],
        command_help: true,
    },
    Verb {
        name: "tree",
        usage: "tree",
        description: "Print a tree of all available commands",
        options: &[],
        command_help: false,
    },
    Verb {
        name: "validate",
        usage: "validate [COMMAND]",
        description: "Validate tool definitions",
        options: &[],
        command_help: false,
    },
    Verb {
        name: "version",
        usage: "version",
        description: "Show Ukiryu version",
        options: &[],
        command_help: true,
    },
    Verb {
        name: "which",
        usage: "which IDENTIFIER",
        description: "Show which tool implementation would be selected",
        options: &[
            REGISTER,
            Opt {
                alias: None,
                name: "platform",
                kind: Kind::Value {
                    banner: "PLATFORM",
                    default: None,
                },
                description: "Platform to check (macos, linux, windows)",
            },
            Opt {
                alias: None,
                name: "shell",
                kind: Kind::Value {
                    banner: "SHELL",
                    default: None,
                },
                description: "Shell to check (bash, zsh, fish, sh, powershell, cmd)",
            },
        ],
        command_help: true,
    },
];

/// The commands whose help is [`command_help`] (the plain commands).
pub fn command_help_verbs() -> impl Iterator<Item = &'static str> {
    VERBS.iter().filter(|v| v.command_help).map(|v| v.name)
}

/// `Thor::Shell::Terminal.terminal_width`: `THOR_COLUMNS` read as Ruby's
/// `to_i`, otherwise the terminal's width (taken from `COLUMNS`, which is what
/// `tput cols` reports for a terminal it cannot query), otherwise 80; a width
/// under 10 is 80.
pub fn terminal_width() -> usize {
    let width = match std::env::var("THOR_COLUMNS") {
        Ok(value) => ruby_to_i(&value),
        Err(_) => std::env::var("COLUMNS")
            .ok()
            .map_or(0, |value| ruby_to_i(&value)),
    };
    usize::try_from(width)
        .ok()
        .filter(|w| *w >= 10)
        .unwrap_or(DEFAULT_WIDTH)
}

/// Ruby's `String#to_i`: optional leading whitespace and sign, then the
/// leading digits (underscores between digits allowed); 0 when none.
fn ruby_to_i(text: &str) -> i64 {
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut value: i64 = 0;
    let mut previous_digit = false;
    for c in digits.chars() {
        match c {
            '0'..='9' => {
                value = value
                    .saturating_mul(10)
                    .saturating_add(i64::from(c as u8 - b'0'));
                previous_digit = true;
            }
            '_' if previous_digit => previous_digit = false,
            _ => break,
        }
    }
    if negative {
        -value
    } else {
        value
    }
}

/// `Thor.help`: the command table, sorted, truncated to `width`, then a blank
/// line (`Ukiryu::Cli` declares no class options).
pub fn root_help(width: usize) -> String {
    let rows: Vec<[String; 2]> = VERBS
        .iter()
        .map(|verb| {
            [
                format!("{PROGRAM} {}", verb.usage),
                format!("# {}", verb.description),
            ]
        })
        .collect();
    let mut text = format!("{PROGRAM} commands:\n");
    text.push_str(&table(&rows, 2, Some(width)));
    text.push('\n');
    text
}

/// `Thor.command_help` for one plain command, or `None` for a name that is
/// not one.
pub fn command_help(name: &str) -> Option<String> {
    let verb = VERBS.iter().find(|v| v.name == name && v.command_help)?;
    let mut text = format!("Usage:\n  {PROGRAM} {}\n\n", verb.usage);
    text.push_str(&options_help(verb.options));
    text.push_str(verb.description);
    text.push('\n');
    Some(text)
}

/// `Thor::Base.print_options`: `Options:`, the table, a blank line; nothing
/// when there are no options.
fn options_help(options: &[Opt]) -> String {
    if options.is_empty() {
        return String::new();
    }
    let padding = options
        .iter()
        .map(|o| aliases_for_usage(o).chars().count())
        .max()
        .unwrap_or(0);
    let mut rows: Vec<[String; 2]> = Vec::new();
    for option in options {
        rows.push([usage(option, padding), format!("# {}", option.description)]);
        let default = match option.kind {
            Kind::Flag { default } => Some(if default { "true" } else { "false" }),
            Kind::Value { default, .. } => default,
        };
        if let Some(default) = default {
            rows.push([String::new(), format!("# Default: {default}")]);
        }
    }
    format!("Options:\n{}\n", table(&rows, 2, None))
}

/// `Thor::Option#aliases_for_usage`.
fn aliases_for_usage(option: &Opt) -> String {
    option.alias.map_or_else(String::new, |a| format!("-{a}, "))
}

/// `Thor::Option#usage(padding)`: the aliases left-justified to `padding`,
/// then the optional switch sample; a boolean (other than `force`) adds its
/// `--no-` and `--skip-` forms.
fn usage(option: &Opt, padding: usize) -> String {
    let aliases = aliases_for_usage(option);
    let mut sample = match option.kind {
        Kind::Value { banner, .. } => format!("[--{}={banner}]", option.name),
        Kind::Flag { .. } => format!("[--{}]", option.name),
    };
    if matches!(option.kind, Kind::Flag { .. }) && option.name != "force" {
        sample.push_str(&format!(", [--no-{0}], [--skip-{0}]", option.name));
    }
    format!("{aliases:<padding$}{sample}")
}

/// `Thor::Shell::TablePrinter#print` for two-column rows: the first column
/// left-justified to its widest cell plus two spaces, the last column as is,
/// each line indented. With `truncate`, a line whose text (without the
/// indent) is longer than `truncate` characters keeps its first
/// `truncate - 3 - indent` characters and gains `...`.
fn table(rows: &[[String; 2]], indent: usize, truncate: Option<usize>) -> String {
    let width = rows
        .iter()
        .map(|row| row[0].chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let mut text = String::new();
    for row in rows {
        let mut sentence = format!("{:<width$}{}", row[0], row[1]);
        if let Some(limit) = truncate {
            if sentence.chars().count() > limit {
                let keep = limit.saturating_sub(3 + indent);
                sentence = sentence.chars().take(keep).collect::<String>() + "...";
            }
        }
        text.push_str(&" ".repeat(indent));
        text.push_str(&sentence);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruby_to_i_reads_the_leading_integer() {
        assert_eq!(ruby_to_i("100"), 100);
        assert_eq!(ruby_to_i("  42abc"), 42);
        assert_eq!(ruby_to_i("1_000"), 1000);
        assert_eq!(ruby_to_i("-5"), -5);
        assert_eq!(ruby_to_i("abc"), 0);
        assert_eq!(ruby_to_i(""), 0);
    }

    #[test]
    fn options_without_aliases_are_padded_to_the_widest_alias() {
        let help = command_help("which").unwrap();
        assert!(help.contains("\n      [--platform=PLATFORM]  # Platform"));
    }
}
