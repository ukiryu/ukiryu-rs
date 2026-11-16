//! Thor's help text against captures of the Ruby CLI.
//!
//! `thor_help_goldens.json` holds `ukiryu help VERB` for every plain command
//! and `ukiryu help` at several `THOR_COLUMNS` widths, captured from the
//! pinned source (see its `source`). Each must match byte for byte.
use serde_json::Value;
use ukiryu_cli::thor_help;

fn goldens() -> Value {
    serde_json::from_str(include_str!("thor_help_goldens.json")).expect("goldens parse")
}

#[test]
fn command_help_matches_thor_for_every_plain_command() {
    let goldens = goldens();
    let captured = goldens["command_help"].as_object().unwrap();
    let mut rendered: Vec<&str> = thor_help::command_help_verbs().collect();
    rendered.sort_unstable();
    let mut expected: Vec<&str> = captured.keys().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(rendered, expected, "the commands with Thor help");
    for (verb, text) in captured {
        assert_eq!(
            thor_help::command_help(verb).as_deref(),
            text.as_str(),
            "help {verb}"
        );
    }
}

#[test]
fn root_help_matches_thor_at_each_width() {
    let goldens = goldens();
    for (width, text) in goldens["root_help"].as_object().unwrap() {
        let width: usize = width.parse().unwrap();
        assert_eq!(
            thor_help::root_help(width),
            text.as_str().unwrap(),
            "help at width {width}"
        );
    }
}

#[test]
fn groups_and_builtins_have_no_command_help_here() {
    for name in [
        "validate",
        "definitions",
        "cache",
        "lint",
        "help",
        "tree",
        "bogus",
    ] {
        assert_eq!(thor_help::command_help(name), None, "{name}");
    }
}
