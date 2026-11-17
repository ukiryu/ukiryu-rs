//! Replays the pinned Ruby `Ukiryu::Tool.find_by` oracle through the port.
//!
//! `register_lookup_oracle/find_by.json` holds 88 rows captured from ukiryu
//! ab6df353 (Ruby 4.0.1, Psych 5.3.1, linux/bash). Each row ran in a fresh
//! Ruby process against a register built from `fixtures/register/spec`, with
//! `PATH` set to a directory of stub executables (no `git`) and `HOME` set to
//! an empty directory. This test rebuilds those inputs in a scratch directory,
//! checks them against the captured digests, and runs every row through the
//! same composition as the source: `Register.discover` (with its auto-clone),
//! the tool index, and `find_by`.
//!
//! Every row goes through `find_by_input`, which also takes the `nil` and
//! Integer identifiers and the platform and shell names outside the port's
//! enums that the oracle passes.
#![cfg(target_os = "linux")]

use serde_json::Value as Json;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use ukiryu::cache::ToolCache;
use ukiryu::environment::Environment;
use ukiryu::executable::{DiscoveryContext as PathContext, Platform};
use ukiryu::extractors::base::ProcessHost;
use ukiryu::register::{DiscoveryContext, Git, Register};
use ukiryu::register_lookup::{find_by_input, load_tool_index, Identifier, Target};
use ukiryu::runtime::ShellName;
use ukiryu::tool::{Tool, ToolOptions};
use ukiryu::tool_loader::RegisterSource;

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
    fs::set_permissions(profile(&d, "gzip_gnu"), fs::Permissions::from_mode(0o000)).unwrap();

    let d = add_register(root, "unreadable-index");
    fs::set_permissions(
        d.join("tools/bzip2_gnu/index.yaml"),
        fs::Permissions::from_mode(0o000),
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
fn describe(root: &Path, result: &Result<Option<Rc<Tool>>, (String, String)>) -> String {
    let inputs = root.display().to_string();
    match result {
        Ok(None) => "nil".to_owned(),
        Ok(Some(tool)) => format!(
            "tool {} available={} executable={}",
            tool.name().unwrap_or_default(),
            tool.is_available(),
            tool.executable()
                .map(|e| e.replace(&inputs, "<inputs>"))
                .unwrap_or_else(|| "nil".to_owned())
        ),
        Err((class, _)) => format!("raise {class}"),
    }
}

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

struct Row<'a> {
    root: &'a Path,
    row: &'a Json,
}

impl Row<'_> {
    fn id(&self) -> &str {
        self.row["id"].as_str().unwrap()
    }

    fn bin(&self) -> PathBuf {
        self.root
            .join("bin")
            .join(self.row["bin"].as_str().unwrap())
    }

    /// `Register.discover`, or `options[:register_path]` when given.
    fn register(&self) -> Result<Register, (String, String)> {
        if let Some(path) = self.row["options"]["register_path"].as_str() {
            let registers = self.root.join("registers").display().to_string();
            return Ok(Register::at(path.replace("<register>", &registers)));
        }
        let env_register = match self.row["register"].as_str() {
            None => None,
            Some("<abs-nonexistent>") => Some(self.root.join("absent-register")),
            Some(name) => Some(self.root.join("registers").join(name)),
        };
        let context = DiscoveryContext {
            env_register: env_register.map(Into::into),
            dev_path: None,
            home: Some(self.root.join("home-empty")),
        };
        let git = Git {
            program: self.bin().join("git"),
            ..Git::default()
        };
        context
            .discover(&git)
            .map_err(|e| (e.class_name().to_owned(), e.to_string()))
    }

    fn run(&self) -> (String, Option<bool>, String) {
        let options = &self.row["options"];
        let target = Target {
            platform: options["platform"].as_str(),
            shell: options["shell"].as_str(),
        };
        let bin = self.bin();
        let path = bin.display().to_string();
        let tool_options =
            ToolOptions::new(Platform::Linux, ShellName::Bash).discovery(PathContext::new(
                Platform::Linux,
                target.shell.unwrap_or("bash"),
                Some(path.as_str()),
                None,
            ));
        let runtime = ToolOptions::new(Platform::Linux, ShellName::Bash);
        let text = self.row["identifier"].as_str();
        let identifier = match self.row["identifier_type"].as_str().unwrap() {
            "nil" => Identifier::Nil,
            "integer" => Identifier::Integer(text.unwrap().parse().unwrap()),
            _ => Identifier::Str(text.unwrap()),
        };

        let register = match self.register() {
            Ok(register) => register,
            Err(e) => {
                let message = e.1.clone();
                return (describe(self.root, &Err(e)), None, message);
            }
        };
        let index = load_tool_index(&register);
        let source = RegisterSource::new(
            register.clone(),
            Platform::Linux,
            ProcessHost::new(Environment::new().set("PATH", &path), ShellName::Bash),
        );
        let cache = ToolCache::new();
        let call = || {
            find_by_input(
                &register,
                &index,
                &source,
                &cache,
                identifier,
                target,
                &tool_options,
                &runtime,
            )
            .map_err(|e| (e.class_name(), e.to_string()))
        };
        let first = call();
        let same = self.row["repeat"]
            .as_bool()
            .unwrap()
            .then(|| match (&first, &call()) {
                (Ok(Some(a)), Ok(Some(b))) => Rc::ptr_eq(a, b),
                (Ok(None), Ok(None)) => true,
                _ => false,
            });
        let message = first
            .as_ref()
            .err()
            .map(|e| e.1.clone())
            .unwrap_or_default();
        (describe(self.root, &first), same, message)
    }
}

#[test]
fn find_by_replays_the_pinned_ruby_oracle_row_by_row() {
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
        if got != want.as_str().unwrap() {
            failures += 1;
            writeln!(report, "INPUT {name}: built {got}, oracle {want}").unwrap();
        }
    }

    // Under root, mode 000 does not stop a read; those rows cannot reproduce.
    let unreadable = root.join("registers/unreadable-index/tools/bzip2_gnu/index.yaml");
    let root_user = fs::read(&unreadable).is_ok();

    let mut skipped = Vec::new();
    let mut message_differences = 0;
    let stub_git = regex::Regex::new(r"<inputs>/bin/[^/\s]+/git\b").unwrap();
    for row in rows {
        let r = Row { root, row };
        let id = r.id();
        let want = expected(row);
        if root_user
            && r.row["register"]
                .as_str()
                .is_some_and(|n| n.starts_with("unreadable"))
        {
            skipped.push(id.to_owned());
            writeln!(report, "SKIP\t{id}\t{want}\trunning as root").unwrap();
            continue;
        }
        let (got, same, message) = r.run();
        let want_same = row["expect"]["repeat_same_object"].as_bool();
        let mut verdict = if got == want && same == want_same {
            "ok"
        } else {
            failures += 1;
            "MISMATCH"
        };
        let want_message = row["expect"]["raised"]["message"].as_str();
        // Ruby starts git through PATH, so its Errno message names the bare
        // program; the port is handed the stub's absolute path instead.
        let got_message = stub_git
            .replace_all(
                &message.replace(&root.display().to_string(), "<inputs>"),
                regex::NoExpand("git"),
            )
            .into_owned();
        if let Some(m) = want_message {
            if got == want && m != got_message {
                message_differences += 1;
                failures += 1;
                verdict = "MESSAGE-MISMATCH";
            }
        }
        writeln!(
            report,
            "{verdict}\t{id}\twant: {want}{}\tgot: {got}{}{}",
            want_same.map(|s| format!(" same={s}")).unwrap_or_default(),
            same.map(|s| format!(" same={s}")).unwrap_or_default(),
            want_message
                .map(|m| format!("\n\twant message: {m}\n\tgot message:  {got_message}"))
                .unwrap_or_default(),
        )
        .unwrap();
    }

    println!("{report}");
    println!(
        "rows {} skipped {} failures {failures} message-differences {message_differences}",
        rows.len(),
        skipped.len()
    );
    assert_eq!(failures, 0, "find_by oracle replay:\n{report}");
}
