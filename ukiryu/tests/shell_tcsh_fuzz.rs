//! Deterministic adversarial fuzz for tcsh formatting (ukiryu/ukiryu#19, UK19-A3).
//!
//! Exact bytes are pinned by the Ruby captures in `shell_unix_special_golden.rs`
//! and the real-shell cases in `shell_unix_special_exec.rs`. This file checks
//! structural properties of the same algorithms over many more generated
//! inputs, and replays a generated subset through a real tcsh:
//!
//! - `quote` output decodes back to the input under tcsh single-quote rules
//!   (`\!` is a history escape, `'\''` is a literal quote);
//! - `escape` only ever prefixes `! $ ` " \` with one backslash;
//! - `join` keeps the executable and each argument separate;
//! - a real `tcsh -c` hands every generated argument to the child unchanged.
//!
//! CR and LF are left out of the alphabet: the Ruby source does not quote them
//! safely (declared source defects in `shell_unix_special_exec.rs`). NUL cannot
//! appear in an argument. With UKIRYU_REQUIRE_SHELL_TESTS=1 a missing tcsh fails;
//! otherwise the real-shell test reports UNVERIFIED.
#![cfg(unix)]

use std::process::Command;
use ukiryu::shell::unix_special::Tcsh;
use ukiryu::shell::ShellFormatting;

const ALPHABET: &str = " '\"`$-\\/;&|(){}@#%!~*?<>[],=:^+\taZ09\u{e9}\u{20ac}\u{1f600}";

/// Small deterministic generator (SplitMix64) so failures reproduce exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn string(&mut self, max_len: usize) -> String {
        let alphabet: Vec<char> = ALPHABET.chars().collect();
        let len = self.below(max_len + 1);
        (0..len)
            .map(|_| alphabet[self.below(alphabet.len())])
            .collect()
    }
}

/// Splits a joined command line on spaces outside single quotes and decodes
/// each word under tcsh rules: inside quotes `\!` is `!`; outside, `\'` is `'`.
fn decode_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quote {
            match c {
                '\'' => in_quote = false,
                '\\' if chars.peek() == Some(&'!') => {
                    chars.next();
                    cur.push('!');
                }
                other => cur.push(other),
            }
        } else {
            match c {
                '\'' => in_quote = true,
                '\\' => {
                    assert_eq!(chars.next(), Some('\''), "bare backslash outside quotes");
                    cur.push('\'');
                }
                ' ' => words.push(std::mem::take(&mut cur)),
                other => panic!("unquoted {other:?} in {line:?}"),
            }
        }
    }
    assert!(!in_quote, "unterminated quote in {line:?}");
    words.push(cur);
    words
}

#[test]
fn quote_decodes_back_to_the_input() {
    let sh = Tcsh;
    let mut rng = Rng(23);
    for _ in 0..30_000 {
        let value = rng.string(16);
        let quoted = sh.quote(&value);
        assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
        assert_eq!(decode_words(&quoted), [value.clone()], "value={value:?}");
        assert_eq!(sh.format_path(&value), value);
    }
}

#[test]
fn escape_prefixes_only_the_special_characters() {
    let sh = Tcsh;
    let mut rng = Rng(2323);
    for _ in 0..30_000 {
        let value = rng.string(16);
        let escaped = sh.escape(&value);
        let mut restored = String::new();
        let mut chars = escaped.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                let next = chars.next().expect("backslash escapes a character");
                assert!(
                    matches!(next, '!' | '$' | '`' | '"' | '\\'),
                    "unexpected escape \\{next} in {escaped:?}"
                );
                restored.push(next);
            } else {
                assert!(
                    !matches!(c, '!' | '$' | '`' | '"'),
                    "unescaped {c:?} in {escaped:?}"
                );
                restored.push(c);
            }
        }
        assert_eq!(restored, value);
    }
}

#[test]
fn join_keeps_executable_and_arguments_separate() {
    let sh = Tcsh;
    let mut rng = Rng(232_323);
    for _ in 0..10_000 {
        let exe = rng.string(10);
        let args: Vec<String> = (0..rng.below(6)).map(|_| rng.string(10)).collect();
        let mut want = vec![exe.clone()];
        want.extend(args.iter().cloned());
        let joined = sh.join(&exe, &args);
        assert_eq!(decode_words(&joined), want, "exe={exe:?} args={args:?}");
    }
}

const PRINTER: &str = "/usr/bin/printf";
const FORMAT: &str = "<%s>";
const SENTINEL: &str = "UKIRYU";

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

#[test]
fn real_tcsh_receives_generated_arguments_unchanged() {
    let sh = Tcsh;
    assert!(std::path::Path::new(PRINTER).is_file(), "{PRINTER} missing");
    let Some(tcsh) = find_on_path("tcsh") else {
        if std::env::var("UKIRYU_REQUIRE_SHELL_TESTS").as_deref() == Ok("1") {
            panic!("tcsh is required (UKIRYU_REQUIRE_SHELL_TESTS=1) but not on PATH");
        }
        eprintln!("UNVERIFIED: tcsh not on PATH; real tcsh execution was not checked");
        return;
    };
    let mut rng = Rng(42_23);
    let mut failures = Vec::new();
    for _ in 0..400 {
        let value = rng.string(12);
        let args = vec![FORMAT.to_owned(), SENTINEL.to_owned(), value.clone()];
        let command = sh.join(PRINTER, &args);
        let output = Command::new(&tcsh)
            .arg("-c")
            .arg(&command)
            .env("LC_ALL", locale())
            .env("LANG", locale())
            .output()
            .expect("spawn tcsh");
        let expected = format!("<{SENTINEL}><{value}>").into_bytes();
        if !(output.status.success() && output.stdout == expected) {
            failures.push(format!(
                "{value:?}: command={command:?} success={} stdout={:?} stderr={:?}",
                output.status.success(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} real-tcsh mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
