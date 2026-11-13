//! Platform-family discovery layouts and the bundled definition search,
//! checked against data captured from the pinned Ruby oracle
//! (`discovery_layouts_data/layouts.json`, produced by
//! `discovery_layouts_data/capture/capture_layouts.rb`).
//!
//! Every tree is built in a temp directory; nothing is installed and no test
//! needs a real `C:\` or `/usr/share`. The discovery cases use only names and
//! modes that give the same answer on Linux, macOS and Windows hosts (see the
//! capture script header), so one golden file is checked natively on all
//! three. Cases the golden marks `unix_only` (a bare-name executable) are
//! skipped on Windows.
//!
//! The pinned Ruby has no `.app/Contents/MacOS` probing and no Program Files
//! probing: the cases show that those directories are found only when they are
//! on `PATH` or in `additional_paths`. Whether that should change is an owner
//! decision and is not decided here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use ukiryu::executable::{find_in_path, DiscoveryContext, Platform};
use ukiryu::models::{Model, ToolDefinition};
use ukiryu::runtime::{native_platform, ShellName};
use ukiryu::tool::{
    bundled_definition_search_paths, from_bundled, rooted_search_dir, Tool, ToolOptions,
};

const GOLDEN: &str = include_str!("discovery_layouts_data/layouts.json");

fn golden() -> Value {
    serde_json::from_str(GOLDEN).unwrap()
}

fn rows<'a>(golden: &'a Value, key: &str) -> &'a Vec<Value> {
    golden[key].as_array().unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

/// A fresh empty directory under the system temp dir, removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ukiryu-layouts-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempRoot(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn text(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build the golden's layout rows: `file` (default content, mode ignored on
/// Windows) and `dir`.
fn build_layout(root: &Path, layout: &[Value]) {
    for entry in layout {
        let path = root.join(entry["path"].as_str().unwrap());
        match entry["kind"].as_str().unwrap_or("file") {
            "file" => {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let content = entry["content"].as_str().unwrap_or("#!/bin/sh\nexit 0\n");
                std::fs::write(&path, content).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = entry["mode"].as_u64().unwrap_or(0o644) as u32;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
                }
            }
            "dir" => std::fs::create_dir_all(&path).unwrap(),
            other => panic!("unknown layout kind {other}"),
        }
    }
}

fn host_platform() -> Platform {
    native_platform().expect("supported host")
}

fn host_path_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

/// A discovery result with the temp root written as `<ROOT>` and `\` as `/`,
/// the form the golden uses.
fn normalise(root: &str, path: &str) -> String {
    path.replace(root, "<ROOT>").replace('\\', "/")
}

#[test]
fn platform_layouts_match_ruby() {
    let golden = golden();
    let root = TempRoot::new("discovery");
    build_layout(root.path(), rows(&golden, "discovery_layout"));
    let root_text = root.text().to_owned();
    let cases = rows(&golden, "discovery");
    assert_eq!(cases.len(), 24);
    let mut checked = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if cfg!(windows) && case["unix_only"].as_bool().unwrap() {
            continue;
        }
        let dirs: Vec<String> = strings(&case["path"])
            .iter()
            .map(|dir| dir.replace("<ROOT>", &root_text))
            .collect();
        let path = dirs.join(host_path_separator());
        let additional: Vec<String> = strings(&case["additional"])
            .iter()
            .map(|dir| dir.replace("<ROOT>", &root_text))
            .collect();
        let context = DiscoveryContext::new(
            host_platform(),
            "bash",
            Some(path.as_str()),
            case["pathext"].as_str(),
        );
        let found = find_in_path(&context, case["command"].as_str().unwrap(), &additional);
        assert_eq!(
            found.map(|p| normalise(&root_text, &p)),
            case["found"].as_str().map(str::to_owned),
            "{name}"
        );
        checked += 1;
    }
    assert_eq!(checked, if cfg!(windows) { 22 } else { 24 });
}

#[test]
fn bundled_search_paths_match_ruby() {
    let golden = golden();
    let cases = rows(&golden, "search_paths");
    assert_eq!(cases.len(), 3);
    for case in cases {
        let platform = Platform::from_name(case["platform"].as_str().unwrap()).unwrap();
        let prefix = case["host_expand_prefix"].as_str().unwrap();
        // On a non-Windows host Ruby expands the drive paths against the
        // working directory; the Windows host result has no such prefix.
        let expected: Vec<String> = strings(&case["paths"])
            .into_iter()
            .map(|p| p.strip_prefix(prefix).unwrap_or(&p).to_owned())
            .collect();
        let actual = bundled_definition_search_paths(platform, case["home"].as_str());
        assert_eq!(actual, expected, "{case}");
    }
}

#[test]
fn bundled_search_paths_home_handling() {
    // Not oracle-captured: Ruby raises ArgumentError without a home
    // directory, and File.expand_path drops a trailing separator.
    assert_eq!(
        bundled_definition_search_paths(Platform::Linux, None),
        [
            "/usr/share/ukiryu",
            "/usr/local/share/ukiryu",
            "/opt/homebrew/share/ukiryu"
        ]
    );
    assert_eq!(
        bundled_definition_search_paths(Platform::Macos, Some("/Users/x/"))
            .last()
            .unwrap(),
        "/Users/x/.local/share/ukiryu"
    );
}

fn version_of(text: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix("version: '"))
        .and_then(|rest| rest.strip_suffix('\''))
        .unwrap()
        .to_owned()
}

#[test]
fn from_bundled_matches_ruby() {
    let golden = golden();
    let cases = rows(&golden, "bundled");
    assert_eq!(cases.len(), 16);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let root = TempRoot::new("bundled");
        build_layout(root.path(), case["files"].as_array().unwrap());
        let mut tried: Vec<String> = Vec::new();
        let found = from_bundled(
            Platform::Linux,
            Some("/home/tester"),
            Some(root.path()),
            case["tool"].as_str().unwrap(),
            |file: &Path| -> Result<Option<String>, std::io::Error> {
                let text = std::fs::read_to_string(file)?;
                // The stand-in for Tool.load used by the capture script.
                if text.starts_with("invalid") {
                    return Ok(None);
                }
                let relative = file.strip_prefix(root.path()).unwrap();
                tried.push(relative.to_string_lossy().replace('\\', "/"));
                Ok(Some(version_of(&text)))
            },
        )
        .unwrap();
        assert_eq!(found, case["version"].as_str().map(str::to_owned), "{name}");
        assert_eq!(tried, strings(&case["tried"]), "{name}");
    }
}

#[test]
fn from_bundled_stops_at_a_load_error() {
    // Not oracle-captured: the source rescues only the two definition
    // errors, so anything else escapes `from_bundled`.
    let root = TempRoot::new("bundled-error");
    build_layout(
        root.path(),
        &[
            serde_json::json!({"path": "usr/share/ukiryu/foo/a.yaml", "content": "x"}),
            serde_json::json!({"path": "usr/share/ukiryu/foo/b.yaml", "content": "x"}),
        ],
    );
    let mut calls = 0;
    let result = from_bundled(
        Platform::Linux,
        None,
        Some(root.path()),
        "foo",
        |_: &Path| -> Result<Option<()>, &'static str> {
            calls += 1;
            Err("boom")
        },
    );
    assert_eq!(result, Err("boom"));
    assert_eq!(calls, 1);
}

#[test]
fn from_bundled_windows_directories_move_under_the_root() {
    // Not oracle-captured: Ruby expands the drive paths against the working
    // directory on a non-Windows host, so there is nothing to compare. This
    // checks the injectable root and the order of the two Program Files
    // directories.
    assert_eq!(
        rooted_search_dir(Path::new("R"), "C:/Program Files (x86)/Ukiryu"),
        Path::new("R")
            .join("C")
            .join("Program Files (x86)")
            .join("Ukiryu")
    );
    let root = TempRoot::new("bundled-windows");
    build_layout(
        root.path(),
        &[
            serde_json::json!({"path": "C/Program Files (x86)/Ukiryu/foo/1.yaml", "content": "x86"}),
            serde_json::json!({"path": "C/Program Files/Ukiryu/foo/1.yaml", "content": "native"}),
            serde_json::json!({"path": "usr/share/ukiryu/foo/1.yaml", "content": "unix"}),
        ],
    );
    let found = from_bundled(
        Platform::Windows,
        None,
        Some(root.path()),
        "foo",
        |file: &Path| -> Result<Option<String>, std::io::Error> {
            Ok(Some(std::fs::read_to_string(file)?))
        },
    )
    .unwrap();
    assert_eq!(found.as_deref(), Some("native"));
}

const SEARCH_PATHS_TOOL: &str = "
name: layoutprobe
version: '1.0'
search_paths:
  linux: ['<DIR>']
  macos: ['<DIR>']
  windows: ['<DIR>']
profiles:
- name: default
  platforms: [macos, linux, windows]
  shells: [bash, zsh, sh, fish, powershell, cmd]
  executable_name: layoutprobe
  search_paths:
    linux: ['<DIR>']
    macos: ['<DIR>']
    windows: ['<DIR>']
  commands:
  - name: version
    subcommand: version
";

fn probe_tool(dir: &str, path: &str) -> Tool {
    let shell = if cfg!(windows) {
        ShellName::Cmd
    } else {
        ShellName::Bash
    };
    let platform = host_platform();
    let context = DiscoveryContext::new(platform, shell.as_str(), Some(path), Some(".exe"));
    Tool::new(
        ToolDefinition::from_yaml(&SEARCH_PATHS_TOOL.replace("<DIR>", dir)).expect("definition"),
        &ToolOptions::new(platform, shell).discovery(context),
    )
    .expect("tool")
}

/// Ruby `Tool#find_executable` hands `ExecutableLocator.find_with_info` only
/// the executable name, the aliases and the platform; the profile
/// `search_paths` (schema-only in the pinned source) never reach discovery.
/// This is source-derived, not oracle-captured: the oracle's model stack is
/// not available to the capture script.
#[test]
fn profile_search_paths_are_not_consulted() {
    let root = TempRoot::new("search-paths");
    let probe_dir = root.path().join("Program Files").join("Probe").join("bin");
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&probe_dir).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    build_layout(
        root.path(),
        &[serde_json::json!({
            "path": "Program Files/Probe/bin/layoutprobe.exe",
            "mode": 0o755
        })],
    );
    let probe_text = probe_dir.to_str().unwrap();

    // The directory is named in the definition but not on PATH: not found.
    let missing = probe_tool(probe_text, elsewhere.to_str().unwrap());
    assert!(!missing.is_available(), "{:?}", missing.executable());

    // Control: the same definition with the directory on PATH finds it, so
    // the executable and the rest of the setup are real.
    let found = probe_tool(probe_text, probe_text);
    assert!(found.is_available());
    assert!(found
        .executable()
        .unwrap()
        .replace('\\', "/")
        .ends_with("Program Files/Probe/bin/layoutprobe.exe"));
}
