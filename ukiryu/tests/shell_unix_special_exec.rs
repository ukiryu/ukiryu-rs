//! Real fish/tcsh execution of the Ruby-quoted command lines.
//!
//! GENERATED command strings; regenerate with the capture scripts.
//! Oracle: ukiryu/ukiryu@ab6df353c6dc2d7faf7817a86aafcafa82579896 (lib/ukiryu/shell/fish.rb, lib/ukiryu/shell/tcsh.rb), methods join, format_path, env_var; Ruby 4.0.1.
//! Capture sha256: 0f92241c0d2b34cbf3a0a31595fc5a5cfd020c36a934a59a05661f493fbc08f3
//!
//! Each case checks two things: the Rust command bytes equal the Ruby bytes, and
//! the real shell hands the original argument to the child unchanged. Cases marked
//! `defect` are source quoting defects: they must still NOT round-trip, and the
//! observed outcome is printed. Shells are found on PATH. With
//! UKIRYU_REQUIRE_SHELL_TESTS=1 a missing shell fails; otherwise it is reported as
//! UNVERIFIED (never as verified). Not applicable on Windows (no tests compiled).
#![cfg(unix)]

use std::process::Command;
use ukiryu::shell::unix_special::{Fish, Tcsh};
use ukiryu::shell::ShellFormatting;

const PRINTER: &str = "/usr/bin/printf";
const FORMAT: &str = "<%s>";
const SENTINEL: &str = "UKIRYU";
const ENV_NAME: &str = "UKIRYU_EXEC_VALUE";
const ENV_VALUE: &str = "value-\u{e9}_1";

struct ExecCase {
    input: &'static str,
    command: &'static str,
    defect: bool,
}

const FISH_ENV_COMMAND: &str = "'/usr/bin/printf' '<%s>' $UKIRYU_EXEC_VALUE";
const FISH_CASES: &[ExecCase] = &[
    ExecCase {
        input: "",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' ''",
        defect: false,
    },
    ExecCase {
        input: "simple",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'simple'",
        defect: false,
    },
    ExecCase {
        input: "with space",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'with space'",
        defect: false,
    },
    ExecCase {
        input: "  lead and trail  ",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '  lead and trail  '",
        defect: false,
    },
    ExecCase {
        input: "it's",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'it'\\''s'",
        defect: false,
    },
    ExecCase {
        input: "say \"hi\"",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'say \"hi\"'",
        defect: false,
    },
    ExecCase {
        input: "it's \"both\"",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'it'\\''s \"both\"'",
        defect: false,
    },
    ExecCase {
        input: "`cmd`",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '`cmd`'",
        defect: false,
    },
    ExecCase {
        input: "$HOME",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '$HOME'",
        defect: false,
    },
    ExecCase {
        input: "${VAR}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '${VAR}'",
        defect: false,
    },
    ExecCase {
        input: "$(cmd)",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '$(cmd)'",
        defect: false,
    },
    ExecCase {
        input: "a&b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a&b'",
        defect: false,
    },
    ExecCase {
        input: "a|b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a|b'",
        defect: false,
    },
    ExecCase {
        input: "a;b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a;b'",
        defect: false,
    },
    ExecCase {
        input: "a>b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a>b'",
        defect: false,
    },
    ExecCase {
        input: "a<b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a<b'",
        defect: false,
    },
    ExecCase {
        input: "*.txt",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '*.txt'",
        defect: false,
    },
    ExecCase {
        input: "what?",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'what?'",
        defect: false,
    },
    ExecCase {
        input: "[br]",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '[br]'",
        defect: false,
    },
    ExecCase {
        input: "{a,b}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '{a,b}'",
        defect: false,
    },
    ExecCase {
        input: "~user",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '~user'",
        defect: false,
    },
    ExecCase {
        input: "#hash",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '#hash'",
        defect: false,
    },
    ExecCase {
        input: "%PATH%",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '%PATH%'",
        defect: false,
    },
    ExecCase {
        input: "^caret",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '^caret'",
        defect: false,
    },
    ExecCase {
        input: "-flag",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '-flag'",
        defect: false,
    },
    ExecCase {
        input: "a=b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a=b'",
        defect: false,
    },
    ExecCase {
        input: "(paren)",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '(paren)'",
        defect: false,
    },
    ExecCase {
        input: "tab\u{9}here",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'tab\u{9}here'",
        defect: false,
    },
    ExecCase {
        input: "unicode \u{e9} \u{fc} \u{65e5}\u{672c}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'unicode \u{e9} \u{fc} \u{65e5}\u{672c}'",
        defect: false,
    },
    ExecCase {
        input: "\u{1f600}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\u{1f600}'",
        defect: false,
    },
    ExecCase {
        input: "/path/with spaces/file.txt",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '/path/with spaces/file.txt'",
        defect: false,
    },
    ExecCase {
        input: "relative/path",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'relative/path'",
        defect: false,
    },
    ExecCase {
        input: "C:\\Program Files\\x",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'C:\\Program Files\\x'",
        defect: false,
    },
    ExecCase {
        input: "back\\slash",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'back\\slash'",
        defect: false,
    },
    ExecCase {
        input: "end\\",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'end\\'",
        defect: true,
    },
    ExecCase {
        input: "\\\\",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\\\'",
        defect: true,
    },
    ExecCase {
        input: "\\'",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\'\\'''",
        defect: true,
    },
    ExecCase {
        input: "!bang",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '!bang'",
        defect: false,
    },
    ExecCase {
        input: "hello!",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'hello!'",
        defect: false,
    },
    ExecCase {
        input: "!!",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '!!'",
        defect: false,
    },
    ExecCase {
        input: "!$(rm -rf /)",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '!$(rm -rf /)'",
        defect: false,
    },
    ExecCase {
        input: "new\u{a}line",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'new\u{a}line'",
        defect: false,
    },
    ExecCase {
        input: "x\u{d}\u{a}y",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'x\u{d}\u{a}y'",
        defect: false,
    },
];

const TCSH_ENV_COMMAND: &str = "'/usr/bin/printf' '<%s>' $UKIRYU_EXEC_VALUE";
const TCSH_CASES: &[ExecCase] = &[
    ExecCase {
        input: "",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' ''",
        defect: false,
    },
    ExecCase {
        input: "simple",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'simple'",
        defect: false,
    },
    ExecCase {
        input: "with space",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'with space'",
        defect: false,
    },
    ExecCase {
        input: "  lead and trail  ",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '  lead and trail  '",
        defect: false,
    },
    ExecCase {
        input: "it's",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'it'\\''s'",
        defect: false,
    },
    ExecCase {
        input: "say \"hi\"",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'say \"hi\"'",
        defect: false,
    },
    ExecCase {
        input: "it's \"both\"",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'it'\\''s \"both\"'",
        defect: false,
    },
    ExecCase {
        input: "`cmd`",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '`cmd`'",
        defect: false,
    },
    ExecCase {
        input: "$HOME",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '$HOME'",
        defect: false,
    },
    ExecCase {
        input: "${VAR}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '${VAR}'",
        defect: false,
    },
    ExecCase {
        input: "$(cmd)",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '$(cmd)'",
        defect: false,
    },
    ExecCase {
        input: "a&b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a&b'",
        defect: false,
    },
    ExecCase {
        input: "a|b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a|b'",
        defect: false,
    },
    ExecCase {
        input: "a;b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a;b'",
        defect: false,
    },
    ExecCase {
        input: "a>b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a>b'",
        defect: false,
    },
    ExecCase {
        input: "a<b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a<b'",
        defect: false,
    },
    ExecCase {
        input: "*.txt",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '*.txt'",
        defect: false,
    },
    ExecCase {
        input: "what?",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'what?'",
        defect: false,
    },
    ExecCase {
        input: "[br]",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '[br]'",
        defect: false,
    },
    ExecCase {
        input: "{a,b}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '{a,b}'",
        defect: false,
    },
    ExecCase {
        input: "~user",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '~user'",
        defect: false,
    },
    ExecCase {
        input: "#hash",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '#hash'",
        defect: false,
    },
    ExecCase {
        input: "%PATH%",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '%PATH%'",
        defect: false,
    },
    ExecCase {
        input: "^caret",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '^caret'",
        defect: false,
    },
    ExecCase {
        input: "-flag",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '-flag'",
        defect: false,
    },
    ExecCase {
        input: "a=b",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'a=b'",
        defect: false,
    },
    ExecCase {
        input: "(paren)",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '(paren)'",
        defect: false,
    },
    ExecCase {
        input: "tab\u{9}here",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'tab\u{9}here'",
        defect: false,
    },
    ExecCase {
        input: "unicode \u{e9} \u{fc} \u{65e5}\u{672c}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'unicode \u{e9} \u{fc} \u{65e5}\u{672c}'",
        defect: false,
    },
    ExecCase {
        input: "\u{1f600}",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\u{1f600}'",
        defect: false,
    },
    ExecCase {
        input: "/path/with spaces/file.txt",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '/path/with spaces/file.txt'",
        defect: false,
    },
    ExecCase {
        input: "relative/path",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'relative/path'",
        defect: false,
    },
    ExecCase {
        input: "C:\\Program Files\\x",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'C:\\Program Files\\x'",
        defect: false,
    },
    ExecCase {
        input: "back\\slash",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'back\\slash'",
        defect: false,
    },
    ExecCase {
        input: "end\\",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'end\\'",
        defect: false,
    },
    ExecCase {
        input: "\\\\",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\\\'",
        defect: false,
    },
    ExecCase {
        input: "\\'",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\'\\'''",
        defect: false,
    },
    ExecCase {
        input: "!bang",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\!bang'",
        defect: false,
    },
    ExecCase {
        input: "hello!",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'hello\\!'",
        defect: false,
    },
    ExecCase {
        input: "!!",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\!\\!'",
        defect: false,
    },
    ExecCase {
        input: "!$(rm -rf /)",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' '\\!$(rm -rf /)'",
        defect: false,
    },
    ExecCase {
        input: "new\u{a}line",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'new\u{a}line'",
        defect: true,
    },
    ExecCase {
        input: "x\u{d}\u{a}y",
        command: "'/usr/bin/printf' '<%s>' 'UKIRYU' 'x\u{d}\u{a}y'",
        defect: true,
    },
];

fn locale() -> &'static str {
    if cfg!(target_os = "macos") {
        "en_US.UTF-8"
    } else {
        "C.UTF-8"
    }
}

fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn shell_or_skip(name: &str) -> Option<std::path::PathBuf> {
    let found = find_on_path(name);
    if found.is_none() {
        if std::env::var("UKIRYU_REQUIRE_SHELL_TESTS").as_deref() == Ok("1") {
            panic!("{name} is required (UKIRYU_REQUIRE_SHELL_TESTS=1) but not on PATH");
        }
        eprintln!("UNVERIFIED: {name} not on PATH; real {name} execution was not checked");
    }
    found
}

/// Runs `shell -c command` the way the Ruby UnixBase/Tcsh executors do.
fn run(shell: &std::path::Path, command: &str) -> (bool, Vec<u8>, Vec<u8>) {
    let output = Command::new(shell)
        .arg("-c")
        .arg(command)
        .env("LC_ALL", locale())
        .env("LANG", locale())
        .env(ENV_NAME, ENV_VALUE)
        .output()
        .expect("spawn shell");
    (output.status.success(), output.stdout, output.stderr)
}

fn check(shell: &impl ShellFormatting, cases: &[ExecCase], env_command: &str) {
    let name = shell.name();
    assert!(
        std::path::Path::new(PRINTER).is_file(),
        "{PRINTER} is missing"
    );
    // Formatting bytes must match the Ruby capture whether or not the shell exists.
    for case in cases {
        let args = vec![
            FORMAT.to_owned(),
            SENTINEL.to_owned(),
            shell.format_path(case.input),
        ];
        assert_eq!(
            shell.join(PRINTER, &args),
            case.command,
            "{name} join bytes for {:?}",
            case.input
        );
    }
    let env_join = format!(
        "{} {}",
        shell.join(PRINTER, &[FORMAT.to_owned()]),
        shell.env_var(ENV_NAME)
    );
    assert_eq!(env_join, env_command, "{name} env command bytes");

    let Some(path) = shell_or_skip(name) else {
        return;
    };
    let mut failures = Vec::new();
    for case in cases {
        let (ok, stdout, stderr) = run(&path, case.command);
        let expected = format!("<{SENTINEL}><{}>", case.input).into_bytes();
        let round_trips = ok && stdout == expected;
        let observed = format!(
            "{name} {:?}: success={ok} stdout={:?} stderr={:?}",
            case.input,
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
        if case.defect {
            eprintln!("SOURCE DEFECT (expected, not fixed): {observed}");
            if round_trips {
                failures.push(format!("declared defect now round-trips: {observed}"));
            }
        } else if !round_trips {
            failures.push(observed);
        }
    }
    let (ok, stdout, stderr) = run(&path, env_command);
    let expected = format!("<{ENV_VALUE}>").into_bytes();
    if !(ok && stdout == expected) {
        failures.push(format!(
            "{name} env_var: success={ok} stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        ));
    }
    assert!(
        failures.is_empty(),
        "{name}: {} real-shell mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn fish_executes_ruby_quoted_arguments() {
    check(&Fish, FISH_CASES, FISH_ENV_COMMAND);
}

#[test]
fn tcsh_executes_ruby_quoted_arguments() {
    check(&Tcsh, TCSH_CASES, TCSH_ENV_COMMAND);
}
