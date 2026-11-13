//! Bundled definition search: port of `Ukiryu::Tool.bundled_definition_search_paths`
//! and `Ukiryu::Tool.from_bundled` (`lib/ukiryu/tool.rb:705-757` at
//! ukiryu/ukiryu@ab6df353c6dc2d7faf7817a86aafcafa82579896).
//!
//! The pinned source looks for `<dir>/<tool>/*.yaml` in a fixed list of system
//! directories plus one user-local directory, and loads the first file that
//! loads. The directories are logical paths (`/usr/share/ukiryu`,
//! `C:/Program Files/Ukiryu`, ...); here they can be moved under a caller-owned
//! root so a test never needs a real `/usr/share` or `C:\`.
//!
//! This is only about where tool *definitions* are searched. Executable
//! discovery is unchanged and does not probe these directories.
//!
//! # Differences from the pinned source
//!
//! - The source loads a file with `Tool.load` and rescues
//!   `DefinitionLoadError` and `DefinitionNotFoundError`. Loading is
//!   supplied by the caller: `Ok(None)` is a rescued error (try the next
//!   file), `Err` is any other exception and ends the search.
//! - The platform and `HOME` are arguments. When there is no home directory
//!   the user-local entry is left out; the source raises `ArgumentError`.
//! - `File.expand_path('~/...')` also folds `.` and `..` segments; here only
//!   trailing separators of `home` are dropped.
//! - The tool name is used literally. The source passes it to `Dir.glob`, so a
//!   name holding glob metacharacters would be a pattern there.
//! - On a non-Windows host the source expands `C:/Program Files/Ukiryu` against
//!   the working directory (`File.expand_path` has no drive concept); the
//!   Windows entries here are the Windows result, `C:/Program Files/Ukiryu`.
use std::path::{Path, PathBuf};

use crate::executable::Platform;

const UNIX_DIRS: [&str; 3] = [
    "/usr/share/ukiryu",
    "/usr/local/share/ukiryu",
    "/opt/homebrew/share/ukiryu",
];

const WINDOWS_DIRS: [&str; 2] = ["C:/Program Files/Ukiryu", "C:/Program Files (x86)/Ukiryu"];

/// `Tool.bundled_definition_search_paths`: the system directories of the
/// platform family, then `<home>/.local/share/ukiryu`.
pub fn bundled_definition_search_paths(platform: Platform, home: Option<&str>) -> Vec<String> {
    let mut paths: Vec<String> = match platform {
        Platform::Linux | Platform::Macos => UNIX_DIRS.iter().map(|d| (*d).to_owned()).collect(),
        Platform::Windows => WINDOWS_DIRS.iter().map(|d| (*d).to_owned()).collect(),
    };
    if let Some(home) = home {
        paths.push(format!(
            "{}/.local/share/ukiryu",
            home.trim_end_matches(['/', '\\'])
        ));
    }
    paths
}

/// The home directory the source's `File.expand_path('~')` would use on this
/// host: `HOME`, else `USERPROFILE` on Windows. Non-UTF-8 and empty values are
/// treated as unset.
pub fn host_home_dir() -> Option<String> {
    let value = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    value("HOME").or_else(|| {
        if cfg!(windows) {
            value("USERPROFILE")
        } else {
            None
        }
    })
}

/// Move a logical search directory under `root`: a drive prefix `C:` becomes the
/// directory `C`, and leading separators are dropped, so
/// `C:/Program Files/Ukiryu` becomes `<root>/C/Program Files/Ukiryu` and
/// `/usr/share/ukiryu` becomes `<root>/usr/share/ukiryu`.
pub fn rooted_search_dir(root: &Path, logical: &str) -> PathBuf {
    let bytes = logical.as_bytes();
    let without_drive = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        format!("{}{}", &logical[..1], &logical[2..])
    } else {
        logical.to_owned()
    };
    let mut path = root.to_path_buf();
    for part in without_drive.split(['/', '\\']).filter(|p| !p.is_empty()) {
        path.push(part);
    }
    path
}

/// The files `Dir.glob(File.join(dir, tool, '*.yaml'))` returns: entries of
/// `<dir>/<tool>` whose name ends in `.yaml` (case-sensitive) and does not
/// start with `.`, in byte order of the name. A missing or unreadable
/// directory has no matches.
fn glob_yaml(dir: &Path, tool_name: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir.join(tool_name)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yaml") && !name.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| dir.join(tool_name).join(name))
        .collect()
}

/// `Tool.from_bundled`: walk [`bundled_definition_search_paths`] in order, and
/// within each directory the `<tool_name>/*.yaml` files in name order, and
/// return the first file `load` accepts.
///
/// `root` moves every search directory under it (see [`rooted_search_dir`]);
/// `None` searches the real locations. `load` returns `Ok(None)` for a file
/// the source's loader would reject with `DefinitionLoadError` or
/// `DefinitionNotFoundError` (skipped, the search goes on), and `Err` for
/// anything else, which ends the search.
pub fn from_bundled<T, E>(
    platform: Platform,
    home: Option<&str>,
    root: Option<&Path>,
    tool_name: &str,
    mut load: impl FnMut(&Path) -> Result<Option<T>, E>,
) -> Result<Option<T>, E> {
    for logical in bundled_definition_search_paths(platform, home) {
        let dir = match root {
            Some(root) => rooted_search_dir(root, &logical),
            None => PathBuf::from(&logical),
        };
        for file in glob_yaml(&dir, tool_name) {
            if let Some(loaded) = load(&file)? {
                return Ok(Some(loaded));
            }
        }
    }
    Ok(None)
}
