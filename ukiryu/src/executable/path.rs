//! PATH scanning (`ExecutableLocator::PathScanner`, `PathExtensions` and
//! `Platform.executable_search_paths`).

use super::Platform;

/// Snapshot of everything a discovery run reads from its environment.
///
/// Ruby memoises `PATH` for the process lifetime (`Platform.executable_search_paths`
/// until `reset_cache`) and reads `PATHEXT` on every scan. Here both are read
/// once, when the context is built; build a new context to pick up changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryContext {
    platform: Platform,
    shell: String,
    search_dirs: Vec<String>,
    extensions: Vec<String>,
}

impl DiscoveryContext {
    /// Build a context from explicit `PATH`/`PATHEXT` values (`None` means the
    /// variable is unset). `shell` is the caller's already-decided shell name;
    /// it is only recorded in discovery results.
    pub fn new(
        platform: Platform,
        shell: impl Into<String>,
        path: Option<&str>,
        pathext: Option<&str>,
    ) -> Self {
        DiscoveryContext {
            platform,
            shell: shell.into(),
            search_dirs: split_search_path(platform, path),
            extensions: path_extensions(platform, pathext),
        }
    }

    /// Build a context from this process's `PATH` and `PATHEXT`, read once now.
    /// Non-UTF-8 values are treated as unset.
    pub fn from_process_env(platform: Platform, shell: impl Into<String>) -> Self {
        let path = std::env::var("PATH").ok();
        let pathext = std::env::var("PATHEXT").ok();
        Self::new(platform, shell, path.as_deref(), pathext.as_deref())
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn shell(&self) -> &str {
        &self.shell
    }

    /// `PATH` directories in order, before additional paths and deduplication.
    pub fn search_dirs(&self) -> &[String] {
        &self.search_dirs
    }

    /// Extensions tried for each directory, in order.
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }
}

/// Ruby `String#split(sep)` with a literal separator: empty fields are kept
/// except trailing ones, and an empty string yields no fields.
fn ruby_split(value: &str, sep: char) -> Vec<String> {
    let mut fields: Vec<String> = value.split(sep).map(str::to_owned).collect();
    while fields.last().is_some_and(String::is_empty) {
        fields.pop();
    }
    fields
}

/// `Platform.executable_search_paths`: `PATH` split on `;` (Windows) or `:`.
pub fn split_search_path(platform: Platform, path: Option<&str>) -> Vec<String> {
    let sep = if platform.is_windows() { ';' } else { ':' };
    ruby_split(path.unwrap_or(""), sep)
}

/// `PathExtensions`: `PATHEXT` split on `;`, or `[""]` when unset. On Windows
/// the exact entries `.exe` and `.EXE` move to the front (in their original
/// order), the rest keep theirs. `PATHEXT` is honoured on every platform.
pub fn path_extensions(platform: Platform, pathext: Option<&str>) -> Vec<String> {
    let raw = match pathext {
        Some(value) => ruby_split(value, ';'),
        None => vec![String::new()],
    };
    if !platform.is_windows() {
        return raw;
    }
    let (preferred, others): (Vec<String>, Vec<String>) = raw
        .into_iter()
        .partition(|ext| ext == ".exe" || ext == ".EXE");
    preferred.into_iter().chain(others).collect()
}

fn is_separator(platform: Platform, c: char) -> bool {
    c == '/' || (platform.is_windows() && c == '\\')
}

/// Ruby `File.join(dir, name)` for two parts: when `name` starts with a
/// separator, trailing separators of `dir` are dropped; otherwise a `/` is
/// inserted unless `dir` already ends with one.
///
/// On Windows, CRuby (`file.c`, DOSISH `isdirsep`) also treats `\` as a
/// separator; that rule is source-derived and was not observed on a native
/// Windows Ruby.
pub fn join_path(platform: Platform, dir: &str, name: &str) -> String {
    if name.starts_with(|c| is_separator(platform, c)) {
        let base = dir.trim_end_matches(|c| is_separator(platform, c));
        format!("{base}{name}")
    } else if dir.ends_with(|c| is_separator(platform, c)) {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Every path `PathScanner.find` would test, in order: `PATH` directories then
/// `additional_paths`, deduplicated keeping the first occurrence, each joined
/// with `command + extension`. On Windows all `/` become `\`.
pub fn candidate_paths(
    context: &DiscoveryContext,
    command: &str,
    additional_paths: &[String],
) -> Vec<String> {
    let platform = context.platform;
    let mut dirs: Vec<&str> = Vec::new();
    for dir in context
        .search_dirs
        .iter()
        .chain(additional_paths.iter())
        .map(String::as_str)
    {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut candidates = Vec::with_capacity(dirs.len() * context.extensions.len());
    for dir in dirs {
        for ext in &context.extensions {
            let joined = join_path(platform, dir, &format!("{command}{ext}"));
            candidates.push(if platform.is_windows() {
                joined.replace('/', "\\")
            } else {
                joined
            });
        }
    }
    candidates
}

/// `PathScanner.find` / `ExecutableLocator.find_in_path`: the first candidate
/// that [`is_executable`] accepts.
pub fn find_in_path(
    context: &DiscoveryContext,
    command: &str,
    additional_paths: &[String],
) -> Option<String> {
    candidate_paths(context, command, additional_paths)
        .into_iter()
        .find(|candidate| is_executable(candidate))
}

/// `PathScanner.executable?`: Ruby `File.executable?(path) && !File.directory?(path)`
/// evaluated on this host. Symlinks are followed.
///
/// Unix: `access(path, X_OK)`, so only the bits that apply to the caller count
/// (an owner sees only the owner bits). Ruby uses the effective ids; the two
/// agree unless the process runs set-id.
///
/// Windows: CRuby marks a file executable when its extension is `.bat`, `.cmd`,
/// `.com` or `.exe` (any case) (`win32.c`); source-derived, not observed on a
/// native Windows Ruby.
///
/// A path containing a NUL byte is not executable (Ruby raises `ArgumentError`).
pub fn is_executable(path: &str) -> bool {
    if path.contains('\0') {
        return false;
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if metadata.is_dir() {
        return false;
    }
    host_executable(path)
}

#[cfg(unix)]
fn host_executable(path: &str) -> bool {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};

    extern "C" {
        fn access(path: *const c_char, mode: c_int) -> c_int;
    }
    const X_OK: c_int = 1;

    let Ok(c_path) = CString::new(path) else {
        return false;
    };
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call;
    // `access` only reads it.
    unsafe { access(c_path.as_ptr(), X_OK) == 0 }
}

#[cfg(windows)]
fn host_executable(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["bat", "cmd", "com", "exe"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

#[cfg(not(any(unix, windows)))]
fn host_executable(_path: &str) -> bool {
    false
}
