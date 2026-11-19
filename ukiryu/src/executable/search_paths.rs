//! Definition `search_paths`: the per-platform executable locations a tool
//! definition declares (`search_paths: {macos: [...], windows: [...]}` in
//! `tool.schema.yaml`), such as `/Applications/Inkscape.app/Contents/MacOS/inkscape`
//! or `C:/Program Files/gs/gs*/bin/gswin64c.exe`.
//!
//! This is new behaviour, asked for by ukiryu/ukiryu#18 ("macOS app-bundle
//! probing, Windows Program Files probing"). The pinned Ruby gem documents
//! the key in its schema but never reads it. It is only used after the Ruby
//! discovery chain (alias, system command, `PATH`) has found nothing, so
//! every executable Ruby finds is still found the same way. It also needs
//! [`super::DiscoveryContext::definition_search_paths`]: on for the context
//! read from the process environment, off for explicit contexts, whose
//! results must not depend on what is installed on the host.
//!
//! Entries are tried in the order the definition lists them. An entry is an
//! absolute path; any segment may hold `*` (any run of characters) and `?`
//! (one character). A wildcard segment is matched against the entries of its
//! parent directory (case-insensitively on Windows), and the matches are
//! tried newest first: names are compared with runs of digits taken as
//! numbers, highest first, so `gs10.05.0` comes before `gs9.56.1`. The first
//! candidate that [`is_executable`] accepts wins. On Windows the result uses
//! `\` separators, as `PATH` results do.

use super::path::is_executable;
use super::Platform;
use std::cmp::Ordering;

/// Whether a path segment holds a wildcard.
fn is_pattern(segment: &str) -> bool {
    segment.contains(['*', '?'])
}

/// Wildcard match of one path segment (`*` and `?` only).
pub fn segment_matches(pattern: &str, name: &str, ignore_case: bool) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if ignore_case {
            s.chars().flat_map(char::to_lowercase).collect()
        } else {
            s.chars().collect()
        }
    };
    let (p, n) = (fold(pattern), fold(name));
    // Iterative glob match with single-star backtracking.
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || (p[pi] != '*' && p[pi] == n[ni])) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Compare two names with digit runs taken as numbers.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Every concrete path an entry names on this host, in the order they are
/// tried. An entry without wildcards yields itself (whether or not it
/// exists); a wildcard segment yields only existing directory entries.
pub fn expand_search_path(platform: Platform, entry: &str) -> Vec<String> {
    let windows = platform.is_windows();
    let normalized = if windows {
        entry.replace('\\', "/")
    } else {
        entry.to_owned()
    };
    let mut segments = normalized.split('/');
    // The first segment is "" for `/abs/path` and `C:` for a drive path.
    let mut prefixes = vec![segments.next().unwrap_or("").to_owned()];
    for segment in segments {
        if segment.is_empty() {
            continue;
        }
        let mut next = Vec::new();
        for prefix in &prefixes {
            if !is_pattern(segment) {
                next.push(format!("{prefix}/{segment}"));
                continue;
            }
            let dir = if prefix.is_empty() || (windows && prefix.ends_with(':')) {
                format!("{prefix}/")
            } else {
                prefix.clone()
            };
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|name| segment_matches(segment, name, windows))
                .collect();
            names.sort_by(|a, b| natural_cmp(b, a));
            next.extend(names.into_iter().map(|name| format!("{prefix}/{name}")));
        }
        prefixes = next;
    }
    if windows {
        prefixes.iter().map(|p| p.replace('/', "\\")).collect()
    } else {
        prefixes
    }
}

/// The first executable named by `entries` (in order), see the module docs.
pub fn find_in_search_paths(platform: Platform, entries: &[String]) -> Option<String> {
    entries
        .iter()
        .flat_map(|entry| expand_search_path(platform, entry))
        .find(|candidate| is_executable(candidate))
}
