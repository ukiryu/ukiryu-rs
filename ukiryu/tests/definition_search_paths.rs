//! Definition `search_paths` probing (ukiryu/ukiryu#18): macOS app bundles
//! and Windows Program Files trees declared by a tool definition. Not in the
//! pinned Ruby source; it runs only after the Ruby chain finds nothing and
//! only when the discovery context allows it. Every tree is built in a temp
//! directory, so the same cases run on Linux, macOS and Windows hosts.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ukiryu::executable::{
    expand_search_path, find_in_search_paths, natural_cmp, segment_matches, DiscoveryContext,
    ExecutableSource, Platform,
};
use ukiryu::models::{Model, ToolDefinition};
use ukiryu::runtime::{native_platform, ShellName};
use ukiryu::tool::{Tool, ToolOptions};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ukiryu-search-paths-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempRoot(dir)
    }

    /// Create an executable file at `rel` (forward slashes) and return its
    /// path as the host writes it.
    fn exe(&self, rel: &str) -> String {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        host_path(&path)
    }

    fn entry(&self, rel: &str) -> String {
        format!("{}/{rel}", self.0.to_str().unwrap().replace('\\', "/"))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn host() -> Platform {
    native_platform().expect("supported host")
}

fn host_path(path: &Path) -> String {
    let s = path.to_str().unwrap().to_owned();
    if host().is_windows() {
        s.replace('/', "\\")
    } else {
        s
    }
}

#[test]
fn wildcard_segments() {
    assert!(segment_matches("gs*", "gs10.05.0", false));
    assert!(segment_matches(
        "ImageMagick*",
        "ImageMagick-7.1.1-Q16-HDRI",
        false
    ));
    assert!(segment_matches("gs?.*", "gs9.56.1", false));
    assert!(!segment_matches("gs?.*", "gs10.05.0", false));
    assert!(segment_matches("*", "", false));
    assert!(!segment_matches("gs*", "GS10", false));
    assert!(segment_matches("gs*", "GS10", true));
    assert!(segment_matches("a*b*c", "aXbYbZc", false));
    assert!(!segment_matches("a*b*c", "aXbYbZ", false));
}

#[test]
fn newest_version_sorts_first() {
    use std::cmp::Ordering::*;
    assert_eq!(natural_cmp("gs10.05.0", "gs9.56.1"), Greater);
    assert_eq!(natural_cmp("gs9.56.1", "gs9.56.1"), Equal);
    assert_eq!(natural_cmp("gs9.5", "gs9.56"), Less);
    assert_eq!(natural_cmp("gs09", "gs9"), Equal);
    assert_eq!(natural_cmp("a", "b"), Less);
}

#[test]
fn program_files_layout_prefers_newest_match() {
    let root = TempRoot::new("pf");
    root.exe("Program Files/gs/gs9.56.1/bin/gswin64c.exe");
    let newest = root.exe("Program Files/gs/gs10.05.0/bin/gswin64c.exe");
    std::fs::create_dir_all(root.0.join("Program Files/gs/gs10.06.0/bin")).unwrap();
    let entry = root.entry("Program Files/gs/gs*/bin/gswin64c.exe");
    let expanded = expand_search_path(host(), &entry);
    assert_eq!(expanded.len(), 3, "{expanded:?}");
    assert_eq!(
        find_in_search_paths(host(), &[entry]),
        Some(newest),
        "gs10.06.0 has no binary; gs10.05.0 beats gs9.56.1"
    );
}

#[test]
fn app_bundle_layout_and_entry_order() {
    let root = TempRoot::new("app");
    let bundle = root.exe("Applications/Inkscape.app/Contents/MacOS/inkscape.exe");
    let other = root.exe("opt/bin/inkscape.exe");
    let entries = vec![
        root.entry("missing/inkscape.exe"),
        root.entry("Applications/Inkscape.app/Contents/MacOS/inkscape.exe"),
        root.entry("opt/bin/inkscape.exe"),
    ];
    assert_eq!(find_in_search_paths(host(), &entries), Some(bundle));
    assert_eq!(find_in_search_paths(host(), &entries[2..]), Some(other));
    assert_eq!(find_in_search_paths(host(), &entries[..1]), None);
    // A directory is not an executable.
    assert_eq!(
        find_in_search_paths(host(), &[root.entry("Applications/Inkscape.app")]),
        None
    );
}

fn definition(platform: Platform, entries: &[String]) -> ToolDefinition {
    let list: String = entries.iter().map(|e| format!("  - '{e}'\n")).collect();
    let yaml = format!(
        "---\nname: fake\nversion: '1.0'\nsearch_paths:\n  {}:\n{list}profiles:\n- name: default\n  platforms: [linux, macos, windows]\n  shells: [bash, zsh, sh, powershell, cmd]\n  commands:\n  - name: run\n",
        platform.as_str()
    );
    ToolDefinition::from_yaml(&yaml).expect("definition")
}

fn tool(definition: ToolDefinition, path: &str, probe: bool) -> Tool {
    let pathext = if cfg!(windows) { Some(".exe") } else { None };
    let context = DiscoveryContext::new(host(), "bash", Some(path), pathext)
        .with_definition_search_paths(probe);
    let options = ToolOptions::new(host(), ShellName::Bash).discovery(context);
    Tool::new(definition, &options).expect("tool")
}

#[test]
fn tool_falls_back_to_definition_search_paths() {
    let root = TempRoot::new("tool");
    let found = root.exe("Applications/Fake.app/Contents/MacOS/fake.exe");
    let empty = root.0.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let entries = vec![root.entry("Applications/Fake*.app/Contents/MacOS/fake.exe")];

    let t = tool(definition(host(), &entries), &host_path(&empty), true);
    assert_eq!(t.executable(), Some(found.as_str()));
    let info = t.executable_info().unwrap();
    assert_eq!(info.source, ExecutableSource::SearchPath);
    assert_eq!(info.source.as_str(), "search_path");

    // Off (the default for explicit contexts): Ruby behaviour, not found.
    let t = tool(definition(host(), &entries), &host_path(&empty), false);
    assert_eq!(t.executable(), None);

    // Entries for another platform are ignored.
    let other = [Platform::Linux, Platform::Macos, Platform::Windows]
        .into_iter()
        .find(|p| *p != host())
        .unwrap();
    let t = tool(definition(other, &entries), &host_path(&empty), true);
    assert_eq!(t.executable(), None);
}

#[test]
fn path_wins_over_search_paths() {
    let root = TempRoot::new("path");
    root.exe("Applications/Fake.app/Contents/MacOS/fake.exe");
    let on_path = root.exe(if cfg!(windows) {
        "bin/fake.exe"
    } else {
        "bin/fake"
    });
    let entries = vec![root.entry("Applications/Fake.app/Contents/MacOS/fake.exe")];
    let t = tool(
        definition(host(), &entries),
        &host_path(&root.0.join("bin")),
        true,
    );
    assert_eq!(t.executable(), Some(on_path.as_str()));
    assert_eq!(t.executable_info().unwrap().source, ExecutableSource::Path);
}

#[test]
fn search_paths_are_read_but_not_written() {
    let def = definition(
        Platform::Windows,
        &["C:/Program Files/gs/gs*/bin/gswin64c.exe".into()],
    );
    assert_eq!(
        def.search_paths_for("windows"),
        vec!["C:/Program Files/gs/gs*/bin/gswin64c.exe".to_owned()]
    );
    assert!(def.search_paths_for("linux").is_empty());
    let written = serde_json::to_value(&def).unwrap();
    assert!(written.get("search_paths").is_none(), "{written}");
    // Malformed values load as Ruby loads them (the key is ignored).
    for bad in [
        "search_paths: 3",
        "search_paths: [a, b]",
        "search_paths: {windows: x}",
    ] {
        let def = ToolDefinition::from_yaml(&format!("---\nname: fake\n{bad}\n")).expect(bad);
        assert!(def.search_paths_for("windows").is_empty(), "{bad}");
    }
}
