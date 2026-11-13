//! `ukiryu_cli::stdin` against the pinned Ruby CLI.
//!
//! `stdin_data/cases.json` was captured from ukiryu/ukiryu ab6df353 with Ruby
//! 4.0.1 on Linux: each case ran `ukiryu exec jq process <params> -D filter=.`
//! with real files, pipes and directories, and an observer recorded the
//! `stdin` argument after `parse_inline_params` (`parsed`) and the request
//! arguments `say_dry_run` received (`resolved`). Paths inside the case's
//! temporary directory are written as `{dir}`. The cases are rebuilt here
//! with real files; each runs the stdin step and compares bytes, key order and
//! the error text the CLI printed.
use serde_json::Value as Json;
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use ukiryu_cli::stdin::{self, Param, Source, StdinError};

fn cases() -> Json {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/stdin_data/cases.json"),
    )
    .expect("cases.json");
    serde_json::from_str(&text).expect("cases.json parses")
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

fn scratch(id: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "ukiryu-stdin-{}-{}-{}",
        std::process::id(),
        nanos,
        id
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Built {
    dir: PathBuf,
    /// Why the case cannot be rebuilt on this host.
    skip: Option<&'static str>,
}

fn build(case: &Json, id: &str) -> Built {
    let dir = scratch(id);
    let setup = &case["setup"];
    for (name, hex) in setup["files"].as_object().expect("files") {
        std::fs::write(dir.join(name), unhex(hex.as_str().expect("hex"))).expect("write file");
    }
    for name in setup["dirs"].as_array().expect("dirs") {
        std::fs::create_dir(dir.join(name.as_str().expect("dir"))).expect("mkdir");
    }
    let symlinks = setup["symlinks"].as_object().expect("symlinks");
    let chmod0 = setup["chmod0"].as_array().expect("chmod0");
    let mut skip = None;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for (name, target) in symlinks {
            std::os::unix::fs::symlink(dir.join(target.as_str().expect("target")), dir.join(name))
                .expect("symlink");
        }
        for name in chmod0 {
            let path = dir.join(name.as_str().expect("name"));
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
            if std::fs::read(&path).is_ok() {
                skip = Some("a mode-000 file is readable (running as root)");
            }
        }
    }
    #[cfg(not(unix))]
    {
        if !symlinks.is_empty() {
            skip = Some("symlink fixtures need Unix");
        }
        if !chmod0.is_empty() {
            skip = Some("mode-000 fixtures need Unix");
        }
    }
    if case["pipe"]["mode"] == "directory" && !cfg!(unix) {
        skip = Some("opening a directory as stdin needs Unix");
    }
    Built { dir, skip }
}

fn cleanup(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let _ =
                    std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

fn reader(case: &Json, dir: &Path) -> Box<dyn Read> {
    let pipe = &case["pipe"];
    match pipe["mode"].as_str().expect("mode") {
        "bytes" => Box::new(Cursor::new(unhex(pipe["hex"].as_str().expect("hex")))),
        "devnull" => Box::new(File::open(null_device()).expect("null device")),
        // A closed fd 0: Ruby's `$stdin.read` returned "", and Rust's
        // `std::io::stdin()` reads EOF for a closed descriptor.
        "closed" => Box::new(io::empty()),
        "directory" => Box::new(
            File::open(dir.join(pipe["name"].as_str().expect("name"))).expect("open directory"),
        ),
        other => panic!("unknown pipe mode {other}"),
    }
}

/// The `stdin` argument as `parse_inline_params` left it.
fn param_of(parsed: &Json) -> (Param<'static>, Option<String>) {
    let stdin = &parsed["stdin"];
    if stdin.is_null() {
        return (Param::Falsy, None);
    }
    match stdin["class"].as_str().expect("class") {
        "NilClass" | "FalseClass" => (Param::Falsy, None),
        "String" => {
            let text = String::from_utf8(unhex(stdin["bytes_hex"].as_str().expect("hex")))
                .expect("parsed stdin text is UTF-8");
            (Param::Other, Some(text))
        }
        _ => (Param::Other, None),
    }
}

fn substitute(text: &str, dir: &Path) -> String {
    text.replace("{dir}", &dir.to_string_lossy())
}

fn flag_of(case: &Json) -> bool {
    case["params"]
        .as_array()
        .expect("params")
        .iter()
        .any(|p| p == "--stdin")
}

#[test]
fn stdin_step_matches_the_pinned_cli() {
    let data = cases();
    assert_eq!(
        data["provenance"]["source_revision"],
        "ab6df353c6dc2d7faf7817a86aafcafa82579896"
    );
    let home = std::env::current_dir().expect("cwd");
    let mut ran = 0;
    let mut skipped = Vec::new();
    for case in data["cases"].as_array().expect("cases") {
        let id = case["id"].as_str().expect("id");
        let built = build(case, id);
        if let Some(why) = built.skip {
            skipped.push(format!("{id}: {why}"));
            cleanup(&built.dir);
            continue;
        }
        let dir = built.dir.clone();
        if case["setup"]["chdir"] == true {
            std::env::set_current_dir(&dir).expect("chdir");
        }

        let parsed = &case["parsed"];
        let (kind, text) = param_of(parsed);
        let text = text.map(|t| substitute(&t, &dir));
        let param = match (&text, kind) {
            (Some(t), _) => Param::Text(t.as_str()),
            (None, kind) => kind,
        };
        let flag = flag_of(case);
        let mut input = reader(case, &dir);
        let outcome = stdin::resolve(flag, param, &mut *input);
        std::env::set_current_dir(&home).expect("restore cwd");

        let exit = case["exit_status"].as_i64().expect("exit");
        if exit == 0 {
            let resolved = &case["resolved"];
            let got = outcome.unwrap_or_else(|e| panic!("{id}: unexpected error {e}"));
            let mut keys: Vec<(String, ())> = parsed["keys"]
                .as_array()
                .expect("keys")
                .iter()
                .map(|k| (k.as_str().expect("key").to_owned(), ()))
                .collect();
            match &got {
                Some(bytes) => {
                    assert_eq!(resolved["stdin"]["class"], "String", "{id}");
                    assert_eq!(resolved["stdin"]["encoding"], "UTF-8", "{id}");
                    let want = unhex(resolved["stdin"]["bytes_hex"].as_str().expect("hex"));
                    assert_eq!(bytes, &want, "{id}: bytes");
                    assert_eq!(
                        std::str::from_utf8(bytes).is_ok(),
                        resolved["stdin"]["valid_encoding"] == true,
                        "{id}: validity"
                    );
                    stdin::store(&mut keys, ());
                }
                None => assert_eq!(resolved["stdin"], parsed["stdin"], "{id}: kept"),
            }
            let order: Vec<&str> = keys.iter().map(|(k, _)| k.as_str()).collect();
            let want: Vec<&str> = resolved["keys"]
                .as_array()
                .expect("keys")
                .iter()
                .map(|k| k.as_str().expect("key"))
                .collect();
            assert_eq!(order, want, "{id}: key order");
        } else {
            assert_eq!(exit, 1, "{id}");
            assert!(case["resolved"].is_null(), "{id}: no dry run");
            let stderr = String::from_utf8(unhex(case["stderr_hex"].as_str().expect("hex")))
                .expect("stderr text");
            let error = outcome
                .err()
                .unwrap_or_else(|| panic!("{id}: expected an error"));
            // Thor's `say` adds no newline after a message ending in a space.
            let want = substitute(stderr.strip_suffix('\n').unwrap_or(&stderr), &dir);
            // The Errno cases were measured on Linux only; elsewhere the
            // step must still stop, but its Ruby text is not known.
            let unmeasured = want.starts_with("[Errno::")
                && !cfg!(any(target_os = "linux", target_os = "macos"));
            if !unmeasured {
                assert_eq!(error.cli_message(), want, "{id}");
                assert_eq!(error.is_thor_error(), !want.starts_with('['), "{id}");
            }
        }
        cleanup(&built.dir);
        ran += 1;
    }
    assert!(ran >= 30, "ran {ran}; skipped {skipped:?}");
    eprintln!("ran {ran} cases; skipped {skipped:?}");
}

#[test]
fn literal_bypass_matches_parsed_strings() {
    // Every captured `stdin=-` / `stdin=@...` reached the stdin step as the
    // raw String, and the YAML-parsed ones did not.
    for case in cases()["cases"].as_array().expect("cases") {
        let params: Vec<&str> = case["params"]
            .as_array()
            .expect("params")
            .iter()
            .filter_map(|p| p.as_str()?.strip_prefix("stdin="))
            .collect();
        let [raw] = params.as_slice() else { continue };
        let parsed = &case["parsed"]["stdin"];
        let kept = parsed["class"] == "String"
            && unhex(parsed["bytes_hex"].as_str().unwrap_or("")) == raw.as_bytes();
        if stdin::keeps_literal("stdin", raw) {
            assert!(kept, "{}: {raw}", case["id"]);
        }
    }
    assert!(stdin::keeps_literal("stdin", "-"));
    assert!(stdin::keeps_literal("stdin", "@"));
    assert!(!stdin::keeps_literal("stdin", "-literal"));
    assert!(!stdin::keeps_literal("input", "-"));
}

#[test]
fn source_branches() {
    assert_eq!(stdin::source(true, Param::Falsy), Source::Reader);
    assert_eq!(stdin::source(true, Param::Text("@f")), Source::Reader);
    assert_eq!(stdin::source(true, Param::Other), Source::Reader);
    assert_eq!(stdin::source(false, Param::Text("-")), Source::Reader);
    assert_eq!(stdin::source(false, Param::Text("@f")), Source::File("f"));
    assert_eq!(stdin::source(false, Param::Text("@")), Source::File(""));
    assert_eq!(stdin::source(false, Param::Text("x")), Source::Keep);
    assert_eq!(stdin::source(false, Param::Other), Source::Keep);
    assert_eq!(stdin::source(false, Param::Falsy), Source::Keep);
}

#[test]
fn keep_does_not_touch_the_reader() {
    struct Fails;
    impl Read for Fails {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read"))
        }
    }
    assert!(stdin::resolve(false, Param::Text("plain"), &mut Fails)
        .expect("kept")
        .is_none());
    let error = stdin::resolve(true, Param::Falsy, &mut Fails).expect_err("reader error");
    assert!(matches!(error, StdinError::Unmapped { ref path, .. } if path == stdin::STDIN_PATH));
}

#[test]
fn store_replaces_in_place_or_appends() {
    let mut args = vec![("stdin".to_owned(), 1), ("filter".to_owned(), 2)];
    stdin::store(&mut args, 3);
    assert_eq!(
        args,
        vec![("stdin".to_owned(), 3), ("filter".to_owned(), 2)]
    );
    let mut args = vec![("filter".to_owned(), 2)];
    stdin::store(&mut args, 3);
    assert_eq!(
        args,
        vec![("filter".to_owned(), 2), ("stdin".to_owned(), 3)]
    );
}
