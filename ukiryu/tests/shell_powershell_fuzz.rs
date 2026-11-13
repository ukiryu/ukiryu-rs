//! Deterministic adversarial fuzz for PowerShell formatting. Exact bytes are
//! pinned by `shell_powershell_oracle.rs`; this file checks structural
//! properties of the same algorithms over many more generated inputs:
//! quoting is reversible, and executable/argument separation survives
//! `join` and the `-Command` invocation script.

use ukiryu::shell::powershell::{InvocationHost, PowerShell};
use ukiryu::shell::ShellFormatting;

const ALPHABET: &[char] = &[
    ' ', '\'', '"', '`', '$', '-', '\\', '/', ';', '&', '|', '(', ')', '{', '}', '@', '#', '%',
    '!', '~', '*', '?', '<', '>', '[', ']', ',', '=', ':', '^', '+', '\t', '\n', '\r', '\u{0b}',
    '\u{0c}', '\0', 'a', 'b', 'Z', '0', '9', 'é', '€', '😀', '\u{a0}', '\u{3000}', '\u{85}',
    '\u{2028}',
];

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

    fn string(&mut self, alphabet: &[char], max_len: usize) -> String {
        let len = self.below(max_len + 1);
        let mut out: String = (0..len)
            .map(|_| alphabet[self.below(alphabet.len())])
            .collect();
        if self.below(5) == 0 {
            out.insert(0, '-');
        }
        out
    }
}

/// Undo `escape_for_double_quotes`: drop each backtick escape.
fn unescape_double(inner: &str) -> String {
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '`' {
            let next = chars.next().expect("backtick escapes a character");
            assert!(matches!(next, '`' | '"' | '$'), "unexpected escape `{next}");
            out.push(next);
        } else {
            assert!(!matches!(c, '"' | '$'), "unescaped {c:?} in {inner:?}");
            out.push(c);
        }
    }
    out
}

/// Split a command script on spaces, decoding PowerShell single-quoted
/// literals (`''` is one quote). Only valid for tokens without bare spaces.
fn split_single_quoted(script: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next().expect("closing quote") {
                    '\'' if chars.peek() == Some(&'\'') => {
                        chars.next();
                        cur.push('\'');
                    }
                    '\'' => break,
                    other => cur.push(other),
                }
            },
            ' ' => tokens.push(std::mem::take(&mut cur)),
            other => cur.push(other),
        }
    }
    tokens.push(cur);
    tokens
}

#[test]
fn quote_and_escape_are_reversible() {
    let sh = PowerShell;
    let mut rng = Rng(19);
    for _ in 0..20_000 {
        let value = rng.string(ALPHABET, 16);
        let quoted = sh.quote(&value);
        assert!(quoted.len() >= 2 && quoted.starts_with('"') && quoted.ends_with('"'));
        assert_eq!(unescape_double(&quoted[1..quoted.len() - 1]), value);
        assert_eq!(sh.escape(&value).replace("''", "'"), value);
        assert_eq!(sh.format_path(&value), value);
    }
}

#[test]
fn join_keeps_executable_and_arguments_separate() {
    let sh = PowerShell;
    let mut rng = Rng(1919);
    for _ in 0..5_000 {
        let exe = rng.string(ALPHABET, 10);
        let args: Vec<String> = (0..rng.below(6))
            .map(|_| rng.string(ALPHABET, 10))
            .collect();
        let joined = sh.join(&exe, &args);
        let mut expected = vec![if sh.needs_quoting(&exe) {
            sh.quote(&exe)
        } else {
            exe.clone()
        }];
        let mut skip = false;
        for a in &args {
            expected.push(if skip || !sh.needs_quoting(a) {
                a.clone()
            } else {
                sh.quote(a)
            });
            skip = !skip && (a == "-Command" || a == "-File");
        }
        assert_eq!(joined, expected.join(" "), "exe={exe:?} args={args:?}");
        for (a, part) in args.iter().zip(&expected[1..]) {
            if sh.needs_quoting(a) && part.starts_with('"') {
                assert_eq!(unescape_double(&part[1..part.len() - 1]), *a);
            }
        }
    }
}

#[test]
fn invocation_script_round_trips_arguments() {
    let sh = PowerShell;
    // Unquoted arguments are emitted raw, so keep bare whitespace other than
    // the plain space (which forces quoting) out of this property.
    let alphabet: Vec<char> = ALPHABET
        .iter()
        .copied()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{0b}' | '\u{0c}'))
        .collect();
    let mut rng = Rng(42);
    for _ in 0..5_000 {
        let exe = rng.string(&alphabet, 10);
        let args: Vec<String> = (0..rng.below(6))
            .map(|_| rng.string(&alphabet, 10))
            .filter(|a| !a.is_empty())
            .collect();
        for host in [InvocationHost::Unix, InvocationHost::Windows] {
            let argv = sh.command_argv(&exe, &args, host);
            assert_eq!(argv.len(), 4);
            assert_eq!(argv[0], sh.powershell_command(host));
            assert_eq!(argv[1..3], ["-NoLogo", "-Command"]);
            let body = argv[3]
                .strip_prefix("& ")
                .and_then(|b| b.strip_suffix("; exit $LASTEXITCODE"))
                .expect("call operator script shape");
            let want_exe = match host {
                InvocationHost::Windows => exe.replace('/', "\\"),
                InvocationHost::Unix => exe.clone(),
            };
            let mut want = vec![want_exe];
            want.extend(args.iter().cloned());
            // Unquoted arguments may contain a raw quote only on hosts that
            // pass them through; decode just the fully literal cases.
            if args.iter().all(|a| !a.contains('\'') || a.starts_with('-')) {
                assert_eq!(split_single_quoted(body), want, "{host:?} {exe:?} {args:?}");
            }
        }
    }
}
