//! Real `cmd.exe` execution checks for `ukiryu::shell::cmd::Cmd`.
//!
//! Every command line is first compared with the bytes the pinned Ruby oracle
//! produces for the same input (ukiryu/ukiryu @ ab6df353c6dc2d7faf7817a86aafcafa82579896, lib/ukiryu/shell/cmd.rb, Ruby 4.0.1), then executed by the real
//! `cmd.exe` found on PATH, and the output of its `echo`/`dir` builtins is
//! compared with the outcome the formatting is meant to produce.
//!
//! Execution only exists on Windows. Elsewhere the execution test is reported
//! as ignored (not applicable), never as passed. With
//! `UKIRYU_REQUIRE_SHELL_TESTS=1`, a missing `cmd.exe` fails instead of being
//! reported as unverified.
//!
//! The case table is generated from the oracle capture; do not edit it by hand.

// The outcome model is only exercised by the Windows-only execution tests.
#![cfg_attr(not(windows), allow(dead_code))]

use ukiryu::shell::cmd::Cmd;
use ukiryu::shell::ShellFormatting;

const PROBE_NAME: &str = "UKIRYU_CMD_PROBE";
const PROBE_VALUE: &str = "expanded-ok";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Each word comes back unchanged, or wrapped in the double quotes that
    /// `quote` added when the word holds a space or tab.
    Words,
    /// Source defect kept as is: `quote` does not neutralize `%VAR%` inside
    /// double quotes, so `cmd.exe` expands it.
    PercentInQuotes,
    /// `echo` prints the formatted text after `echo ` verbatim.
    Literal,
    /// `join` keeps the arguments after a leading `/c` as one caret-escaped
    /// command string for `cmd.exe` itself.
    SlashC,
}

struct ExecCase {
    kind: Kind,
    executable: &'static str,
    args: &'static [&'static str],
    ruby_join: &'static str,
}

const ENV_VAR_PROBE: &str = "%UKIRYU_CMD_PROBE%";

const CASES: &[ExecCase] = &[
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["hello"],
        ruby_join: "echo hello",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["100%"],
        ruby_join: "echo 100^%",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["%UKIRYU_CMD_PROBE%"],
        ruby_join: "echo ^%UKIRYU_CMD_PROBE^%",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["%%"],
        ruby_join: "echo ^%^%",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a^b"],
        ruby_join: "echo a^^b",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a^^b"],
        ruby_join: "echo a^^^^b",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a&b"],
        ruby_join: "echo a^&b",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a|b"],
        ruby_join: "echo a^|b",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a<b"],
        ruby_join: "echo a^<b",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a>b"],
        ruby_join: "echo a^>b",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["2>&1"],
        ruby_join: "echo 2^>^&1",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a|b&c%d"],
        ruby_join: "echo a^|b^&c^%d",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["&&calc"],
        ruby_join: "echo ^&^&calc",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["!UKIRYU_CMD_PROBE!"],
        ruby_join: "echo !UKIRYU_CMD_PROBE!",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["(x)"],
        ruby_join: "echo (x)",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["*.txt"],
        ruby_join: "echo *.txt",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["$HOME"],
        ruby_join: "echo $HOME",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["it's"],
        ruby_join: "echo it's",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["C:\\path\\x.exe"],
        ruby_join: "echo C:\\path\\x.exe",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["\\\\server\\share\\f"],
        ruby_join: "echo \\\\server\\share\\f",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["caf\u{E9}"],
        ruby_join: "echo caf\u{E9}",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["\u{65E5}\u{672C}\u{8A9E}"],
        ruby_join: "echo \u{65E5}\u{672C}\u{8A9E}",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["\u{DC}n\u{EF}c\u{F6}d\u{E9}^&|"],
        ruby_join: "echo \u{DC}n\u{EF}c\u{F6}d\u{E9}^^^&^|",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["-o", "out.txt", "100%", "a&b", "x|y", "%UKIRYU_CMD_PROBE%"],
        ruby_join: "echo -o out.txt 100^% a^&b x^|y ^%UKIRYU_CMD_PROBE^%",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a b"],
        ruby_join: "echo \"a b\"",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["hello & world"],
        ruby_join: "echo \"hello & world\"",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["x | y > z"],
        ruby_join: "echo \"x | y > z\"",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a ^ b"],
        ruby_join: "echo \"a ^ b\"",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["\u{65E5}\u{672C} \u{8A9E}"],
        ruby_join: "echo \"\u{65E5}\u{672C} \u{8A9E}\"",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["C:\\Program Files\\x.exe"],
        ruby_join: "echo \"C:\\Program Files\\x.exe\"",
    },
    ExecCase {
        kind: Kind::Words,
        executable: "echo",
        args: &["a b", "c&d", "e f"],
        ruby_join: "echo \"a b\" c^&d \"e f\"",
    },
    ExecCase {
        kind: Kind::PercentInQuotes,
        executable: "echo",
        args: &["a b %UKIRYU_CMD_PROBE%"],
        ruby_join: "echo \"a b %UKIRYU_CMD_PROBE%\"",
    },
    ExecCase {
        kind: Kind::Literal,
        executable: "echo",
        args: &["c\"d"],
        ruby_join: "echo c\"d",
    },
    ExecCase {
        kind: Kind::Literal,
        executable: "echo",
        args: &["a\"b c"],
        ruby_join: "echo \"a\"b c\"",
    },
    ExecCase {
        kind: Kind::Literal,
        executable: "echo",
        args: &["", "x"],
        ruby_join: "echo  x",
    },
    ExecCase {
        kind: Kind::SlashC,
        executable: "cmd",
        args: &["/c", "echo", "a&b", "100%", "x y", "%UKIRYU_CMD_PROBE%"],
        ruby_join: "cmd /c echo a^&b 100^% x y ^%UKIRYU_CMD_PROBE^%",
    },
];

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn expected_words(args: &[&str]) -> String {
    args.iter()
        .map(|w| {
            if w.contains([' ', '\t']) {
                format!("\"{w}\"")
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn exec_case_command_lines_match_oracle() {
    for case in CASES {
        assert_eq!(
            Cmd.join(case.executable, &owned(case.args)),
            case.ruby_join,
            "join({:?}, {:?})",
            case.executable,
            case.args
        );
    }
    assert_eq!(Cmd.env_var(PROBE_NAME), ENV_VAR_PROBE);
}

#[cfg(not(windows))]
#[test]
#[ignore = "cmd.exe execution runs only on Windows; not applicable on this platform (unverified here)"]
fn cmd_exe_execution_not_applicable() {}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    fn find_cmd() -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join("cmd.exe"))
            .find(|candidate| candidate.is_file())
    }

    fn cmd_or_unverified(test: &str) -> Option<PathBuf> {
        if let Some(cmd) = find_cmd() {
            return Some(cmd);
        }
        if std::env::var("UKIRYU_REQUIRE_SHELL_TESTS").as_deref() == Ok("1") {
            panic!("{test}: cmd.exe not found on PATH and UKIRYU_REQUIRE_SHELL_TESTS=1");
        }
        eprintln!("UNVERIFIED {test}: cmd.exe not found on PATH; execution skipped");
        None
    }

    /// Runs `cmd.exe` with a raw (unquoted by Rust) command line. `/u` makes
    /// builtin output UTF-16LE so Unicode survives independent of code page.
    fn run(cmd: &Path, raw: &str) -> String {
        let output = Command::new(cmd)
            .raw_arg(raw)
            .env(PROBE_NAME, PROBE_VALUE)
            .stdin(Stdio::null())
            .output()
            .expect("spawn cmd.exe");
        let units: Vec<u16> = output
            .stdout
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        eprintln!(
            "cmd.exe {raw:?} -> status {:?}, stdout {text:?}",
            output.status.code()
        );
        text.trim_end_matches(['\r', '\n']).to_string()
    }

    #[test]
    fn echo_outcomes_match_formatting_contract() {
        let Some(cmd) = cmd_or_unverified("echo_outcomes_match_formatting_contract") else {
            return;
        };
        let mut failures = Vec::new();
        for case in CASES.iter().filter(|c| c.kind != Kind::SlashC) {
            let line = Cmd.join(case.executable, &owned(case.args));
            let expected = match case.kind {
                Kind::Words => expected_words(case.args),
                Kind::PercentInQuotes => {
                    expected_words(case.args).replace(&format!("%{PROBE_NAME}%"), PROBE_VALUE)
                }
                Kind::Literal => line["echo ".len()..].to_string(),
                Kind::SlashC => unreachable!(),
            };
            let got = run(&cmd, &format!("/d /u /s /c \"{line}\""));
            if got != expected {
                failures.push(format!("{line:?}: got {got:?}, expected {expected:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn slash_c_join_runs_as_one_command_string() {
        let Some(cmd) = cmd_or_unverified("slash_c_join_runs_as_one_command_string") else {
            return;
        };
        for case in CASES.iter().filter(|c| c.kind == Kind::SlashC) {
            let line = Cmd.join(case.executable, &owned(case.args));
            let rest = line.strip_prefix("cmd ").expect("executable is cmd");
            let got = run(&cmd, &format!("/d /u {rest}"));
            assert_eq!(got, case.args[2..].join(" "), "{line:?}");
        }
    }

    #[test]
    fn env_var_reference_expands() {
        let Some(cmd) = cmd_or_unverified("env_var_reference_expands") else {
            return;
        };
        let line = format!("echo {}", Cmd.env_var(PROBE_NAME));
        assert_eq!(run(&cmd, &format!("/d /u /s /c \"{line}\"")), PROBE_VALUE);
    }

    #[test]
    fn quoted_path_with_metacharacters_reaches_dir() {
        let Some(cmd) = cmd_or_unverified("quoted_path_with_metacharacters_reaches_dir") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("ukiryu cmd & ^ test {}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create probe dir");
        std::fs::write(dir.join("a b&c.txt"), b"x").expect("create probe file");
        let native = dir.to_str().expect("utf-8 temp path").to_string();
        let formatted = Cmd.format_path(&native.replace('\\', "/"));
        assert_eq!(formatted, native);
        let line = Cmd.join("dir", &["/b".to_string(), formatted]);
        let got = run(&cmd, &format!("/d /u /s /c \"{line}\""));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(got, "a b&c.txt", "{line:?}");
    }
}
