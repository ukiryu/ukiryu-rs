//! The `find_by` oracle rows (`register_lookup_oracle/find_by.json`, captured
//! from ukiryu ab6df353) dispatched through the wire's `find_by` operation.
//! The fixture registers, stub executables and digests are rebuilt as in
//! `register_lookup_oracle.rs`; each row becomes one JSON request and the
//! reply must equal the captured outcome exactly: tool name, availability and
//! executable, `nil`, or the raised class and message, plus whether a repeated
//! call returns the same object.
#![cfg(target_os = "linux")]

use serde_json::{json, Value as Json};
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const STUB: &str = "n=$(basename \"$0\")
case \"$n:$*\" in
  ping:*-V*) echo \"ping from iputils 2024.01.17\" ;;
  ping:*) echo \"PING localhost (127.0.0.1) 56(84) bytes of data.\" ;;
  *) echo \"$n 1.0 (stub)\" ;;
esac
";

const ALL: [&str; 16] = [
    "jq",
    "ping",
    "gzip",
    "bzip2",
    "unzip",
    "tar",
    "sort",
    "ffmpeg",
    "convert",
    "magick",
    "gs",
    "ghostscript",
    "pdf2ps",
    "exiftool",
    "inkscape",
    "yq",
];

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ukiryu-find-by-oracle-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir(&dir).expect("scratch directory must be new");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        restore_modes(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn restore_modes(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let mode = if meta.is_dir() { 0o755 } else { 0o644 };
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(mode));
        if meta.is_dir() {
            restore_modes(&path);
        }
    }
}

fn base_register() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("fixtures/register/spec")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn profile(register: &Path, tool: &str) -> PathBuf {
    register.join(format!("tools/{tool}/default/1.0.yaml"))
}

fn edit(path: &Path, f: impl FnOnce(&str) -> String) {
    let text = fs::read_to_string(path).unwrap();
    fs::write(path, f(&text)).unwrap();
}

/// Ruby `String#sub` with a regexp: replace the first match only.
fn sub(text: &str, pattern: &str, replacement: &str) -> String {
    let re = regex::Regex::new(pattern).unwrap();
    assert!(re.is_match(text), "pattern {pattern:?} must match");
    re.replacen(text, 1, regex::NoExpand(replacement))
        .into_owned()
}

/// Ruby `s.sub(/^aliases:.*?\n(?=\S)/m, replacement)`: from the first line
/// starting with `aliases:` through the first newline that is followed by a
/// non-space character.
fn sub_aliases_block(text: &str, replacement: &str) -> String {
    let start = if text.starts_with("aliases:") {
        0
    } else {
        text.find("\naliases:").expect("aliases line") + 1
    };
    let mut at = start;
    loop {
        let newline = at + text[at..].find('\n').expect("newline after aliases");
        let next = text[newline + 1..].chars().next();
        if next.is_some_and(|c| !c.is_whitespace()) {
            return format!("{}{}{}", &text[..start], replacement, &text[newline + 1..]);
        }
        at = newline + 1;
    }
}

fn add_register(root: &Path, name: &str) -> PathBuf {
    let dir = root.join("registers").join(name);
    copy_tree(&base_register(), &dir);
    dir
}

fn add_bin(root: &Path, name: &str, executables: &[&str]) {
    let dir = root.join("bin").join(name);
    fs::create_dir_all(&dir).unwrap();
    symlink("/bin/sh", dir.join("sh")).unwrap();
    symlink("/bin/bash", dir.join("bash")).unwrap();
    for exe in executables {
        let path = dir.join(exe);
        fs::write(&path, STUB).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// `build_inputs.rb` of the oracle, step for step.
fn build_inputs(root: &Path) {
    let without = |gone: &[&str]| -> Vec<&'static str> {
        ALL.iter().copied().filter(|e| !gone.contains(e)).collect()
    };
    add_bin(root, "full", &ALL);
    add_bin(root, "empty", &[]);
    add_bin(root, "no-bzip2", &without(&["bzip2"]));
    add_bin(root, "ping-only", &["ping"]);
    add_bin(root, "no-ping", &without(&["ping"]));
    add_bin(root, "bzip2-only", &["bzip2"]);

    add_register(root, "fixtures");

    let d = add_register(root, "shared-iface");
    for tool in ["bzip2", "bzip2_busybox", "bzip2_gnu"] {
        edit(&profile(&d, tool), |s| {
            sub(s, r"(?m)^implements: .*$", "implements: shared/1.0")
        });
    }

    let d = add_register(root, "shared-iface-array");
    edit(&profile(&d, "bzip2_gnu"), |s| {
        sub(
            s,
            r"(?m)^implements: .*$",
            "implements:\n- shared/1.0\n- other/1.0",
        )
    });
    edit(&profile(&d, "gzip_gnu"), |s| {
        sub(s, r"(?m)^implements: .*$", "implements: shared/1.0")
    });

    let d = add_register(root, "shared-alias");
    for tool in ["ping_bsd", "ping_gnu", "ping_windows"] {
        edit(&profile(&d, tool), |s| {
            sub_aliases_block(s, "aliases:\n- zz\n")
        });
    }
    edit(&profile(&d, "jq"), |s| {
        sub(s, r"(?m)^aliases: \[\]$", "aliases:\n- solo")
    });

    let d = add_register(root, "orphan");
    let jq = fs::read_to_string(profile(&d, "jq")).unwrap();
    let orphan = sub(&jq, r"(?m)^name: jq", "name: orphan");
    let orphan = sub(&orphan, r"(?m)^implements: .*$", "implements: orphan/1.0");
    let orphan = sub(&orphan, r"(?m)^aliases: \[\]$", "aliases:\n- orph");
    let win = sub(&jq, r"(?m)^name: jq", "name: orphan_win");
    let win = sub(&win, r"(?m)^implements: .*$", "implements: orphan_win/1.0");
    let win = sub(&win, r"(?m)^aliases: \[\]$", "aliases:\n- orphw");
    let win = sub(
        &win,
        r"platforms:\n(  - \w+\n)+",
        "platforms:\n  - windows\n",
    );
    for (tool, text) in [("orphan", orphan), ("orphan_win", win)] {
        let path = profile(&d, tool);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    let d = add_register(root, "missing-version-file");
    fs::remove_file(profile(&d, "bzip2_gnu")).unwrap();

    let d = add_register(root, "malformed-yaml");
    fs::write(
        profile(&d, "bzip2_gnu"),
        "implements: [unclosed\n  : : :\n\t- bad",
    )
    .unwrap();

    let d = add_register(root, "malformed-index");
    fs::write(
        d.join("tools/bzip2_gnu/index.yaml"),
        "implementations: [unclosed\n  : : :\n",
    )
    .unwrap();

    let d = add_register(root, "unreadable-profile");
    fs::set_permissions(profile(&d, "gzip_gnu"), fs::Permissions::from_mode(0)).unwrap();

    let d = add_register(root, "unreadable-index");
    fs::set_permissions(
        d.join("tools/bzip2_gnu/index.yaml"),
        fs::Permissions::from_mode(0),
    )
    .unwrap();

    let d = add_register(root, "empty-tools");
    fs::remove_dir_all(d.join("tools")).unwrap();
    fs::create_dir_all(d.join("tools")).unwrap();

    let d = add_register(root, "no-tools-dir");
    fs::remove_dir_all(d.join("tools")).unwrap();

    fs::write(root.join("registers/not-a-dir"), "x\n").unwrap();
    fs::create_dir_all(root.join("home-empty")).unwrap();
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn digest_lines(base: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::metadata(&path).unwrap();
        if meta.is_dir() {
            digest_lines(base, &path, out);
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        if meta.permissions().mode() & 0o777 == 0 {
            out.push((rel.clone(), format!("{rel}:UNREADABLE")));
        } else {
            let bytes = fs::read(&path).unwrap();
            out.push((
                rel.clone(),
                format!("{rel}:{}:{:016x}", bytes.len(), fnv1a(&bytes)),
            ));
        }
    }
}

/// The rule recorded in the fixture's `digest_rule`.
fn digest(dir: &Path) -> String {
    let mut entries = Vec::new();
    digest_lines(dir, dir, &mut entries);
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let lines: Vec<String> = entries.into_iter().map(|(_, line)| line).collect();
    format!(
        "{}:{:016x}",
        lines.len(),
        fnv1a(lines.join("\n").as_bytes())
    )
}

/// What one `find_by` call produced, in the oracle's terms.

fn expected(row: &Json) -> String {
    let e = &row["expect"];
    if e["nil"].as_bool() == Some(true) {
        return "nil".to_owned();
    }
    if let Some(t) = e.get("tool") {
        return format!(
            "tool {} available={} executable={}",
            t["name"].as_str().unwrap(),
            t["available"].as_bool().unwrap(),
            t["executable"].as_str().unwrap_or("nil")
        );
    }
    format!("raise {}", e["raised"]["class"].as_str().unwrap())
}

/// The row as a wire request.
fn request(root: &Path, row: &Json) -> Json {
    let options = &row["options"];
    let registers = root.join("registers");
    let register = if let Some(path) = options["register_path"].as_str() {
        json!({"kind": "path", "path": path.replace("<register>", &registers.display().to_string())})
    } else {
        let env_register = match row["register"].as_str() {
            None => Json::Null,
            Some("<abs-nonexistent>") => json!(root.join("absent-register")),
            Some(name) => json!(registers.join(name)),
        };
        json!({
            "kind": "discover",
            "env_register": env_register,
            "home": root.join("home-empty"),
            "git": root.join("bin").join(row["bin"].as_str().unwrap()).join("git"),
        })
    };
    let text = row["identifier"].as_str();
    let identifier = match row["identifier_type"].as_str().unwrap() {
        "nil" => Json::Null,
        "integer" => json!(text.unwrap().parse::<i64>().unwrap()),
        _ => json!(text.unwrap()),
    };
    json!({
        "op": "find_by",
        "register": register,
        "identifier": identifier,
        "platform": options["platform"],
        "shell": options["shell"],
        "path": root.join("bin").join(row["bin"].as_str().unwrap()),
        "repeat": row["repeat"],
    })
}

/// The reply in the oracle's `describe` form, plus the message and the
/// repeat verdict.
fn outcome(root: &Path, reply: &Json) -> (String, String, Option<bool>) {
    let inputs = root.display().to_string();
    if reply["ok"].as_bool() != Some(true) {
        let class = reply["error"]["source_class"]
            .as_str()
            .unwrap_or("<no class>");
        let message = reply["error"]["message"].as_str().unwrap_or_default();
        return (format!("raise {class}"), message.to_owned(), None);
    }
    let same = reply["same_object"].as_bool();
    let value = &reply["value"];
    if value.is_null() {
        return ("nil".to_owned(), String::new(), same);
    }
    let executable = value["executable"]
        .as_str()
        .map(|e| e.replace(&inputs, "<inputs>"))
        .unwrap_or_else(|| "nil".to_owned());
    (
        format!(
            "tool {} available={} executable={}",
            value["name"].as_str().unwrap_or_default(),
            value["available"].as_bool().unwrap(),
            executable
        ),
        String::new(),
        same,
    )
}

#[test]
fn find_by_rows_match_ruby_through_the_wire() {
    let doc: Json =
        serde_json::from_str(include_str!("register_lookup_oracle/find_by.json")).unwrap();
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 88);

    let scratch = Scratch::new();
    let root = scratch.0.as_path();
    build_inputs(root);

    let mut report = String::new();
    let mut failures = 0;
    for (name, want) in doc["input_digests"].as_object().unwrap() {
        let got = digest(&root.join("registers").join(name));
        if &got != want.as_str().unwrap() {
            failures += 1;
            writeln!(report, "INPUT {name}: built {got}, oracle {want}").unwrap();
        }
    }

    // Under root, mode 000 does not stop a read; those rows cannot reproduce.
    let unreadable = root.join("registers/unreadable-index/tools/bzip2_gnu/index.yaml");
    let root_user = fs::read(&unreadable).is_ok();

    let mut skipped = 0;
    for row in rows {
        let id = row["id"].as_str().unwrap();
        if root_user
            && row["register"]
                .as_str()
                .is_some_and(|n| n.starts_with("unreadable"))
        {
            skipped += 1;
            continue;
        }
        let input = request(root, row).to_string();
        let reply = ukiryu::wire::process(input.as_bytes());
        let (got, message, same) = outcome(root, &reply);
        let want = expected(row);
        let want_same = row["expect"]["repeat_same_object"].as_bool();
        let mut verdict = if got == want && same == want_same {
            "ok"
        } else {
            failures += 1;
            "MISMATCH"
        };
        // Ruby starts git through PATH, so its Errno message names the bare
        // program; the wire is handed the stub's absolute path instead.
        let got_message = regex::Regex::new(r"<inputs>/bin/[^/\s]+/git\b")
            .unwrap()
            .replace_all(
                &message.replace(&root.display().to_string(), "<inputs>"),
                regex::NoExpand("git"),
            )
            .into_owned();
        if let Some(m) = row["expect"]["raised"]["message"].as_str() {
            if got == want && m != got_message {
                failures += 1;
                verdict = "MESSAGE-MISMATCH";
            }
        }
        writeln!(
            report,
            "{verdict}\t{id}\twant: {want}\tgot: {got}\t{message}"
        )
        .unwrap();
    }
    println!("{report}");
    assert_eq!(
        failures, 0,
        "find_by wire replay ({skipped} skipped):\n{report}"
    );
}
