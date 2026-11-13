//! `version`, `system` and `config get|set|unset` against Ruby CLI goldens.
//!
//! `inspect_goldens.json` holds stdout, stderr, exit status and the files
//! left under `$HOME` for each case, captured from the pinned Ruby CLI with a
//! controlled environment (see its `source.context`). Each case is replayed
//! here: the arguments go through the real parser, the handler runs over an
//! explicit context with the same variables, and the writes are applied.
//!
//! Recorded deviations:
//! - the version string is `ukiryu::VERSION` (the Ruby gem says 0.3.1);
//! - shell detection is strict (DECISIONS.md D2): rows with a `note` expect
//!   the shell error after the same partial output;
//! - the persistent config.yml is compared as YAML data, since Psych and the
//!   pinned emitter quote differently (`'10'`), and a YAML load failure is
//!   reported with the engine's message, so only its prefix is compared.

use serde_json::Value as Json;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use ukiryu::environment::Environment;
use ukiryu::executable::Platform;
use ukiryu::render::OutputWrite;
use ukiryu::runtime::{Host, ShellError};
use ukiryu_cli::commands::inspect::{
    self, ConfigAction, ConfigInput, HandlerContext, InspectError, SystemInput, VersionInput,
};
use ukiryu_cli::commands::CommandOutcome;

const RUBY_VERSION_LINE: &str = "Ukiryu version 0.3.1";

fn goldens() -> Json {
    let text = include_str!("inspect_goldens.json");
    serde_json::from_str(text).expect("inspect_goldens.json parses")
}

fn text(value: &Json) -> &str {
    value.as_str().expect("string")
}

struct Sandbox {
    root: PathBuf,
    home: String,
    bin: String,
    register: String,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let root =
            std::env::temp_dir().join(format!("ukiryu-cli-inspect-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = |leaf: &str| {
            let path = root.join(leaf);
            fs::create_dir_all(&path).unwrap();
            path.to_str().unwrap().to_owned()
        };
        Sandbox {
            home: dir("home"),
            bin: dir("bin"),
            register: dir("register"),
            root,
        }
    }

    fn substitute(&self, golden: &str) -> String {
        golden
            .replace("{HOME}", &self.home)
            .replace("{BIN}", &self.bin)
            .replace("{REGISTER}", &self.register)
            .replace(
                RUBY_VERSION_LINE,
                &format!("Ukiryu version {}", ukiryu::VERSION),
            )
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn install_executable(dir: &str, name: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = Path::new(dir).join(name);
    fs::write(&path, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn install_executable(_dir: &str, _name: &str) {
    unreachable!("PATH-dependent rows run on Unix hosts only")
}

fn arg(matches: &clap::ArgMatches, id: &str) -> Option<String> {
    matches
        .get_one::<OsString>(id)
        .map(|v| v.to_str().expect("UTF-8 argument").to_owned())
}

/// The parser-to-handler mapping documented for root's dispatcher.
fn dispatch(
    args: &[String],
    host: Host,
) -> Result<Result<CommandOutcome, InspectError>, clap::Error> {
    let matches = ukiryu_cli::arguments::command()
        .try_get_matches_from(std::iter::once("ukiryu".to_owned()).chain(args.iter().cloned()))?;
    let (verb, sub) = matches.subcommand().expect("subcommand");
    let register = if verb == "version" {
        None
    } else {
        arg(sub, "register")
    };
    let ctx = HandlerContext { host, register };
    Ok(match verb {
        "version" => inspect::version(VersionInput, &ctx),
        "system" => inspect::system(
            SystemInput {
                subcommand: arg(sub, "subcommand"),
            },
            &ctx,
        ),
        "config" => {
            let input = ConfigInput::from_arguments(
                arg(sub, "action").as_deref(),
                arg(sub, "key").as_deref(),
                arg(sub, "value").as_deref(),
            )
            .expect("config list has no handler yet");
            inspect::config(input, &ctx)
        }
        other => panic!("unexpected verb {other}"),
    })
}

struct Applied {
    stdout: String,
    stderr: String,
}

fn apply(writes: &[OutputWrite]) -> Applied {
    let mut applied = Applied {
        stdout: String::new(),
        stderr: String::new(),
    };
    for write in writes {
        match write {
            OutputWrite::Stdout(s) => applied.stdout.push_str(s),
            OutputWrite::Stderr(s) => applied.stderr.push_str(s),
            OutputWrite::File { path, contents } => fs::write(path, contents).unwrap(),
        }
    }
    applied
}

fn yaml_data(text: &str) -> Vec<yeptris::Value> {
    yeptris::parse_stream(text.as_bytes()).expect("YAML parses")
}

fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                found.push(rel.to_str().unwrap().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

/// Compare stdout, tolerating only the engine-specific YAML load message.
fn assert_stdout(name: &str, actual: &str, expected: &str) {
    const WARNING: &str = "\x1b[31mWarning: Failed to load config file: ";
    match expected.split_once('\n') {
        Some((first, rest)) if first.starts_with(WARNING) => {
            let (a_first, a_rest) = actual.split_once('\n').expect("warning line");
            assert!(
                a_first.starts_with(WARNING) && a_first.ends_with("\x1b[0m"),
                "{name}: {a_first:?}"
            );
            assert_eq!(a_rest, rest, "{name}");
        }
        _ => assert_eq!(actual, expected, "{name}"),
    }
}

#[test]
fn goldens_replay() {
    let goldens = goldens();
    let cases = goldens["cases"].as_array().unwrap();
    let mut replayed = 0;
    for case in cases {
        let name = text(&case["name"]);
        let platform = Platform::from_name(text(&case["platform"])).unwrap();
        let bins: Vec<&str> = case["bins"].as_array().unwrap().iter().map(text).collect();
        if !bins.is_empty() && !cfg!(unix) {
            // Executable lookup is host-native; these rows need a Unix host.
            continue;
        }
        let sandbox = Sandbox::new(name);
        for bin in &bins {
            install_executable(&sandbox.bin, bin);
        }
        for (rel, body) in case["home_before"].as_object().unwrap() {
            let path = Path::new(&sandbox.home).join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text(body)).unwrap();
        }
        let mut pairs = vec![
            ("HOME".to_owned(), sandbox.home.clone()),
            ("PATH".to_owned(), sandbox.bin.clone()),
            ("UKIRYU_REGISTER".to_owned(), sandbox.register.clone()),
        ];
        for (k, v) in case["env"].as_object().unwrap() {
            pairs.push((k.clone(), text(v).to_owned()));
        }
        let host = Host {
            platform,
            env: Environment::from_pairs(pairs),
        };
        let args: Vec<String> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| text(a).to_owned())
            .collect();
        let expected_stdout = sandbox.substitute(text(&case["stdout"]));
        let expected_stderr = sandbox.substitute(text(&case["stderr"]));
        let expected_exit = case["exit"].as_i64().unwrap() as i32;

        let result = match dispatch(&args, host) {
            Ok(result) => result,
            Err(error) => {
                // Arity errors belong to the parser (Thor's message differs).
                assert_eq!(
                    error.kind(),
                    clap::error::ErrorKind::UnknownArgument,
                    "{name}"
                );
                assert_eq!(expected_exit, 1, "{name}");
                replayed += 1;
                continue;
            }
        };
        match result {
            Ok(outcome) => {
                assert!(
                    case["note"].is_null(),
                    "{name}: expected the D2 shell error"
                );
                let applied = apply(&outcome.writes);
                assert_stdout(name, &applied.stdout, &expected_stdout);
                assert_eq!(applied.stderr, expected_stderr, "{name}");
                assert_eq!(outcome.exit_code, expected_exit, "{name}");
            }
            Err(InspectError::Shell { partial, error }) => {
                assert!(!case["note"].is_null(), "{name}: unexpected {error}");
                let applied = apply(&partial);
                let cut = expected_stdout
                    .rfind("\x1b[2mCurrent shell: ")
                    .expect("current shell line");
                assert_eq!(applied.stdout, &expected_stdout[..cut], "{name}");
                match name {
                    "system-linux-unknown-shell-var" => assert!(
                        matches!(error, ShellError::Unknown { ref value, .. } if value == "/opt/bin/xonsh"),
                        "{name}: {error:?}"
                    ),
                    _ => assert!(
                        matches!(error, ShellError::Missing { .. }),
                        "{name}: {error:?}"
                    ),
                }
            }
            Err(error) => {
                assert_eq!(expected_stdout, "", "{name}");
                assert_eq!(
                    format!("{}\n", error.friendly_message()),
                    expected_stderr,
                    "{name}"
                );
                assert_eq!(expected_exit, 1, "{name}");
            }
        }

        let expected_files = case["home_after"].as_object().unwrap();
        let home = Path::new(&sandbox.home);
        let actual_files = files_under(home);
        let wanted: Vec<String> = expected_files.keys().cloned().collect();
        assert_eq!(actual_files, wanted, "{name}: files under HOME");
        for (rel, body) in expected_files {
            let actual = fs::read_to_string(home.join(rel)).unwrap();
            assert_eq!(yaml_data(&actual), yaml_data(text(body)), "{name}: {rel}");
        }
        replayed += 1;
    }
    let expected = if cfg!(unix) {
        cases.len()
    } else {
        cases
            .iter()
            .filter(|c| c["bins"].as_array().unwrap().is_empty())
            .count()
    };
    assert_eq!(replayed, expected);
}

#[test]
fn config_list_has_no_handler_yet() {
    assert_eq!(ConfigInput::from_arguments(None, None, None), None);
    assert_eq!(ConfigInput::from_arguments(Some("list"), None, None), None);
    assert_eq!(
        ConfigInput::from_arguments(Some("get"), Some("format"), None),
        Some(ConfigInput {
            action: ConfigAction::Get {
                key: Some("format".to_owned())
            }
        })
    );
}

#[test]
fn config_set_keeps_existing_key_order() {
    let sandbox = Sandbox::new("order");
    let dir = Path::new(&sandbox.home).join(".ukiryu");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.yml"), "---\nz: 1\na: two\n").unwrap();
    let ctx = HandlerContext {
        host: Host {
            platform: Platform::Linux,
            env: Environment::from_pairs([("HOME", sandbox.home.as_str())]),
        },
        register: None,
    };
    let input = ConfigInput::from_arguments(Some("set"), Some("m"), Some("3")).unwrap();
    let outcome = inspect::config(input, &ctx).unwrap();
    apply(&outcome.writes);
    let written = fs::read_to_string(dir.join("config.yml")).unwrap();
    let data = yaml_data(&written);
    let yeptris::Value::Mapping(map) = &data[0] else {
        panic!("mapping: {written}")
    };
    let keys: Vec<&yeptris::Value> = map.0.iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        ["z", "a", "m"]
            .map(|k| yeptris::Value::String(k.to_owned()))
            .iter()
            .collect::<Vec<_>>()
    );
    // The integer the user wrote by hand stays an integer.
    assert_eq!(map.0[0].1, yeptris::Value::Number(yeptris::Number::I64(1)));
}

#[test]
fn config_needs_a_home_directory() {
    let ctx = HandlerContext {
        host: Host {
            platform: Platform::Linux,
            env: Environment::from_pairs(Vec::<(String, String)>::new()),
        },
        register: None,
    };
    let input = ConfigInput::from_arguments(Some("get"), Some("format"), None).unwrap();
    let error = inspect::config(input, &ctx).unwrap_err();
    assert_eq!(
        error.friendly_message(),
        "[ArgumentError] couldn't find HOME environment -- expanding '~'"
    );
}
