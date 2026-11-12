//! Byte parity of `ukiryu::shell::powershell` against bytes captured from the
//! pinned Ruby oracle (ukiryu/ukiryu ab6df353, lib/ukiryu/shell/powershell.rb).
//! The fixture's `provenance` field names the capture script and methods.

use serde_json::Value;
use ukiryu::shell::powershell::{InvocationHost, PowerShell};
use ukiryu::shell::ShellFormatting;

const ORACLE_SHA: &str = "ab6df353c6dc2d7faf7817a86aafcafa82579896";

fn golden() -> Value {
    serde_json::from_str(include_str!("../src/shell/powershell/oracle_golden.json"))
        .expect("fixture is valid JSON")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string field")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array field")
        .iter()
        .map(|e| s(e).to_owned())
        .collect()
}

fn records<'a>(g: &'a Value, key: &str) -> &'a Vec<Value> {
    g[key].as_array().expect("record array")
}

#[test]
fn fixture_is_pinned_to_the_oracle() {
    let g = golden();
    assert_eq!(s(&g["provenance"]["oracle_sha"]), ORACLE_SHA);
    assert!(records(&g, "strings").len() >= 1000);
    assert!(records(&g, "joins").len() >= 300);
    assert!(records(&g, "invocations").len() >= 300);
}

#[test]
fn name_and_executable_match_oracle() {
    let g = golden();
    let sh = PowerShell;
    assert_eq!(sh.name(), s(&g["name"]));
    assert_eq!(PowerShell::EXECUTABLE, s(&g["powershell_command"]["unix"]));
    assert_eq!(
        sh.powershell_command(InvocationHost::Unix),
        s(&g["powershell_command"]["unix"])
    );
    assert_eq!(
        sh.powershell_command(InvocationHost::Windows),
        s(&g["powershell_command"]["windows"])
    );
}

#[test]
fn string_operations_match_oracle_bytes() {
    let g = golden();
    let sh = PowerShell;
    let mut failures = Vec::new();
    for r in records(&g, "strings") {
        let input = s(&r["input"]);
        let checks: [(&str, String, &str); 6] = [
            ("escape", sh.escape(input), s(&r["escape"])),
            (
                "escape_for_double_quotes",
                sh.escape_for_double_quotes(input),
                s(&r["escape_for_double_quotes"]),
            ),
            ("quote", sh.quote(input), s(&r["quote"])),
            // Ruby `quote(s, for_exe: true)` ignores the keyword.
            ("quote_for_exe", sh.quote(input), s(&r["quote_for_exe"])),
            ("format_path", sh.format_path(input), s(&r["format_path"])),
            ("env_var", sh.env_var(input), s(&r["env_var"])),
        ];
        for (op, got, want) in checks {
            if got != want {
                failures.push(format!("{op}({input:?}): got {got:?}, want {want:?}"));
            }
        }
        let want = r["needs_quoting"].as_bool().expect("bool");
        if sh.needs_quoting(input) != want {
            failures.push(format!("needs_quoting({input:?}): want {want}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn join_matches_oracle_bytes() {
    let g = golden();
    let sh = PowerShell;
    let mut failures = Vec::new();
    for r in records(&g, "joins") {
        let exe = s(&r["executable"]);
        let args = strings(&r["args"]);
        let got = sh.join(exe, &args);
        if got != s(&r["out"]) {
            failures.push(format!(
                "join({exe:?}, {args:?}): got {got:?}, want {:?}",
                s(&r["out"])
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn invocation_argv_matches_oracle_bytes() {
    let g = golden();
    let sh = PowerShell;
    let mut failures = Vec::new();
    let mut seen = [0usize; 4];
    for r in records(&g, "invocations") {
        let windows = r["windows"].as_bool().expect("bool");
        let host = if windows {
            InvocationHost::Windows
        } else {
            InvocationHost::Unix
        };
        let exe = s(&r["executable"]);
        let args = strings(&r["args"]);
        let got = match r["stdin_path"].as_str() {
            Some(path) => sh.stdin_command_argv(exe, &args, path, host),
            None => sh.command_argv(exe, &args, host),
        };
        seen[usize::from(windows) * 2 + usize::from(r["stdin_path"].is_string())] += 1;
        let want = strings(&r["argv"]);
        if got != want {
            failures.push(format!("{r}: got {got:?}"));
        }
    }
    assert!(
        seen.iter().all(|&n| n > 0),
        "every host/stdin variant covered: {seen:?}"
    );
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
