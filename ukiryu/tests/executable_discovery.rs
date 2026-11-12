//! Executable discovery checked against data captured from the pinned Ruby
//! oracle (see `executable_oracle.rs` for provenance). The Windows-only test
//! checks the host predicate, which is source-derived (CRuby win32.c) and not
//! oracle-captured.

#[path = "executable_oracle.rs"]
mod oracle;

#[cfg(unix)]
use std::cell::RefCell;
#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use ukiryu::executable::{
    alias_target_command, candidate_paths, find, find_in_path, first_output_line, join_path,
    path_extensions, resolve_executable_name, split_search_path, DiscoveryContext, ExecutableInfo,
    ExecutableSource, NoAliases, NoSystemCommands, Platform,
};
#[cfg(unix)]
use ukiryu::executable::{
    find_with_info, is_executable, AliasInfo, AliasProbe, SystemCommandRunner,
};

fn golden() -> Value {
    serde_json::from_str(oracle::ORACLE_JSON).unwrap()
}

fn rows<'a>(golden: &'a Value, key: &str) -> &'a Vec<Value> {
    golden[key].as_array().unwrap()
}

fn opt_str(value: &Value) -> Option<&str> {
    value.as_str()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn platform_of(windows: bool) -> Platform {
    if windows {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

/// A fresh empty directory under the system temp dir, removed on drop.
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ukiryu-executable-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempRoot(dir)
    }

    #[cfg(unix)]
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }

    #[cfg(unix)]
    fn sub(&self, value: &str) -> String {
        value.replace("<ROOT>", self.path())
    }

    #[cfg(unix)]
    fn rel(&self, value: &str) -> String {
        value.replace(self.path(), "<ROOT>")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn file_join_matches_ruby_on_unix_host() {
    let golden = golden();
    let cases = rows(&golden, "file_join");
    assert_eq!(cases.len(), 17);
    for case in cases {
        let joined = join_path(
            Platform::Linux,
            case["dir"].as_str().unwrap(),
            case["name"].as_str().unwrap(),
        );
        assert_eq!(joined, case["joined"].as_str().unwrap(), "{case}");
    }
}

#[test]
fn path_splitting_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "path_split");
    assert_eq!(cases.len(), 26);
    for case in cases {
        let dirs = split_search_path(
            platform_of(case["windows"].as_bool().unwrap()),
            opt_str(&case["path"]),
        );
        assert_eq!(dirs, strings(&case["dirs"]), "{case}");
    }
}

#[test]
fn pathext_order_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "path_extensions");
    assert_eq!(cases.len(), 18);
    for case in cases {
        let exts = path_extensions(
            platform_of(case["windows"].as_bool().unwrap()),
            opt_str(&case["pathext"]),
        );
        assert_eq!(exts, strings(&case["extensions"]), "{case}");
    }
}

#[test]
fn candidate_order_matches_ruby_including_simulated_windows() {
    let golden = golden();
    let cases = rows(&golden, "candidates");
    assert_eq!(cases.len(), 13);
    for case in cases {
        let context = DiscoveryContext::new(
            platform_of(case["windows"].as_bool().unwrap()),
            "bash",
            opt_str(&case["path"]),
            opt_str(&case["pathext"]),
        );
        let actual = candidate_paths(
            &context,
            case["command"].as_str().unwrap(),
            &strings(&case["additional"]),
        );
        assert_eq!(actual, strings(&case["candidates"]), "{case}");
    }
}

#[cfg(unix)]
fn build_layout(root: &TempRoot, layout: &[Value]) {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for entry in layout {
        let path = root.0.join(entry["path"].as_str().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        match entry["kind"].as_str().unwrap() {
            "file" => {
                std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
                let mode = entry["mode"].as_u64().unwrap() as u32;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            "dir" => {
                std::fs::create_dir_all(&path).unwrap();
                let mode = entry["mode"].as_u64().unwrap() as u32;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            }
            "symlink" => symlink(entry["target"].as_str().unwrap(), &path).unwrap(),
            other => panic!("unknown layout kind {other}"),
        }
    }
}

/// Root ignores permission bits for `access`, so owner/group/other cases
/// observed as a normal user do not apply.
#[cfg(unix)]
fn running_as_root(root: &TempRoot) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let probe = root.0.join("root-probe");
    std::fs::write(&probe, "").unwrap();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o010)).unwrap();
    let as_root = is_executable(probe.to_str().unwrap());
    std::fs::remove_file(&probe).unwrap();
    as_root
}

#[cfg(unix)]
#[test]
fn unix_filesystem_lookup_matches_ruby() {
    let golden = golden();
    let root = TempRoot::new("fs");
    build_layout(&root, rows(&golden, "fs_layout"));
    let as_root = running_as_root(&root);
    let cases = rows(&golden, "fs_find");
    assert_eq!(cases.len(), 20);
    let mut checked = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if as_root && name == "group-other-bits-only-not-owner" {
            continue;
        }
        let path = case["path"].as_str().map(|p| root.sub(p));
        let context = DiscoveryContext::new(
            Platform::Linux,
            "bash",
            path.as_deref(),
            opt_str(&case["pathext"]),
        );
        let additional: Vec<String> = strings(&case["additional"])
            .iter()
            .map(|p| root.sub(p))
            .collect();
        let found = find_in_path(&context, case["command"].as_str().unwrap(), &additional);
        assert_eq!(
            found.as_deref().map(|p| root.rel(p)),
            case["found"].as_str().map(str::to_owned),
            "{name}"
        );
        checked += 1;
    }
    assert!(checked >= 19);
}

#[cfg(unix)]
type Events = Rc<RefCell<Vec<Vec<String>>>>;

#[cfg(unix)]
struct TableAliases {
    table: BTreeMap<String, AliasInfo>,
    events: Events,
}

#[cfg(unix)]
impl AliasProbe for TableAliases {
    fn detect_alias(&mut self, command: &str) -> Option<AliasInfo> {
        self.events
            .borrow_mut()
            .push(vec!["alias".to_owned(), command.to_owned()]);
        self.table.get(command).cloned()
    }
}

#[cfg(unix)]
struct TableRunner {
    responses: Vec<(String, Vec<String>, bool, String)>,
    events: Events,
}

#[cfg(unix)]
impl SystemCommandRunner for TableRunner {
    fn run(&mut self, executable: &str, args: &[String]) -> Option<String> {
        let mut event = vec!["system".to_owned(), executable.to_owned()];
        event.extend(args.iter().cloned());
        self.events.borrow_mut().push(event);
        self.responses
            .iter()
            .find(|(exe, a, _, _)| exe == executable && a == args)
            .and_then(|(_, _, ok, stdout)| ok.then(|| stdout.clone()))
    }
}

#[cfg(unix)]
#[test]
fn strategy_order_matches_ruby() {
    let golden = golden();
    let root = TempRoot::new("strategy");
    build_layout(&root, rows(&golden, "fs_layout"));
    let cases = rows(&golden, "strategy");
    assert_eq!(cases.len(), 14);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let platform = Platform::from_name(case["platform"].as_str().unwrap()).unwrap();
        let events: Events = Rc::default();
        let mut aliases = TableAliases {
            table: case["alias_table"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        AliasInfo {
                            target: root.sub(v["target"].as_str().unwrap()),
                            definition: v["definition"].as_str().map(|d| root.sub(d)),
                        },
                    )
                })
                .collect(),
            events: events.clone(),
        };
        let mut runner = TableRunner {
            responses: case["responses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    (
                        r[0][0].as_str().unwrap().to_owned(),
                        strings(&r[0][1]),
                        r[1][0].as_bool().unwrap(),
                        r[1][1].as_str().unwrap().to_owned(),
                    )
                })
                .collect(),
            events: events.clone(),
        };
        let path = root.sub(case["path"].as_str().unwrap());
        let context =
            DiscoveryContext::new(platform, case["shell"].as_str().unwrap(), Some(&path), None);
        let found = find_with_info(
            &context,
            case["tool"].as_str().unwrap(),
            &strings(&case["aliases"]),
            &mut aliases,
            &mut runner,
        );

        let expected_events: Vec<Vec<String>> = case["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] != "path_scan")
            .map(strings)
            .collect();
        let actual_events: Vec<Vec<String>> = events
            .borrow()
            .iter()
            .map(|e| e.iter().map(|s| root.rel(s)).collect())
            .collect();
        assert_eq!(actual_events, expected_events, "{name}: probe order");

        let expected = &case["result"];
        match found {
            None => assert!(expected.is_null(), "{name}: expected {expected}"),
            Some(found) => {
                assert_eq!(
                    root.rel(&found.path),
                    expected["path"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(found.info.path, found.path, "{name}");
                assert_eq!(found.info.source.as_str(), expected["source"], "{name}");
                assert_eq!(
                    found.info.shell,
                    expected["shell"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    found.info.alias_definition.as_deref().map(|d| root.rel(d)),
                    expected["alias_definition"].as_str().map(str::to_owned),
                    "{name}"
                );
                assert_eq!(
                    root.rel(&found.info.description()),
                    expected["description"],
                    "{name}"
                );
                assert_eq!(found.info.is_alias(), expected["alias?"], "{name}");
                assert_eq!(found.info.is_path(), expected["path?"], "{name}");
            }
        }
    }
}

#[test]
fn first_output_line_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "first_line");
    assert_eq!(cases.len(), 13);
    for case in cases {
        assert_eq!(
            first_output_line(case["stdout"].as_str().unwrap()).as_deref(),
            case["line"].as_str(),
            "{case}"
        );
    }
}

#[test]
fn alias_target_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "alias_target");
    assert_eq!(cases.len(), 9);
    for case in cases {
        assert_eq!(
            alias_target_command(case["target"].as_str().unwrap()).as_deref(),
            case["command"].as_str(),
            "{case}"
        );
    }
}

#[test]
fn executable_info_description_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "description");
    assert_eq!(cases.len(), 4);
    for case in cases {
        let info = ExecutableInfo {
            path: case["path"].as_str().unwrap().to_owned(),
            source: match case["source"].as_str().unwrap() {
                "path" => ExecutableSource::Path,
                "alias" => ExecutableSource::Alias,
                other => panic!("{other}"),
            },
            shell: case["shell"].as_str().unwrap().to_owned(),
            alias_definition: case["alias_definition"].as_str().map(str::to_owned),
        };
        assert_eq!(info.description(), case["description"].as_str().unwrap());
        assert_eq!(info.is_alias(), case["alias?"]);
        assert_eq!(info.is_path(), case["path?"]);
    }
}

#[test]
fn executable_name_resolution_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "executable_name");
    assert_eq!(cases.len(), 3);
    for case in cases {
        let resolved = resolve_executable_name(
            case["executable_name"].as_str(),
            case["profile_name"].as_str().unwrap(),
        );
        let call = &case["locator_call"];
        assert_eq!(resolved, call["tool_name"].as_str().unwrap(), "{case}");
        // `@profile.aliases || []`
        let aliases = case["profile_aliases"]
            .as_array()
            .map(|_| strings(&case["profile_aliases"]))
            .unwrap_or_default();
        assert_eq!(aliases, strings(&call["aliases"]), "{case}");
    }
}

#[test]
fn platform_names_are_exact() {
    for (name, platform) in [
        ("linux", Platform::Linux),
        ("macos", Platform::Macos),
        ("windows", Platform::Windows),
    ] {
        assert_eq!(Platform::from_name(name), Some(platform));
        assert_eq!(platform.as_str(), name);
    }
    for fuzzy in ["Linux", "darwin", "mac", "osx", "win32", "unix", ""] {
        assert_eq!(Platform::from_name(fuzzy), None, "{fuzzy}");
    }
}

#[test]
fn explicit_no_op_strategies_leave_only_the_path_scan() {
    let context = DiscoveryContext::new(Platform::Linux, "bash", Some(""), None);
    assert_eq!(
        find(
            &context,
            "definitely-not-here",
            &[],
            &mut NoAliases,
            &mut NoSystemCommands
        ),
        None
    );
}

/// Windows host predicate (source-derived from CRuby win32.c, not observed on
/// a native Windows Ruby): files with .bat/.cmd/.com/.exe in any case are
/// executable, other files and directories are not.
#[cfg(windows)]
#[test]
fn windows_path_scan_uses_extension_rule() {
    let root = TempRoot::new("winpath");
    let a = root.0.join("a");
    let b = root.0.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(b.join("gs.EXE")).unwrap();
    std::fs::write(a.join("gs.ps1"), "").unwrap();
    std::fs::write(a.join("gs.Cmd"), "").unwrap();
    let path = format!("{};{}", b.to_str().unwrap(), a.to_str().unwrap());
    let context = DiscoveryContext::new(
        Platform::Windows,
        "cmd",
        Some(&path),
        Some(".PS1;.EXE;.CMD"),
    );
    let found = find_in_path(&context, "gs", &[]).unwrap();
    assert!(found.ends_with("\\a\\gs.CMD"), "{found}");
}
