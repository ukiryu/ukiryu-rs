//! Profile `search_paths` probing (Windows Program Files layouts, macOS app
//! bundles, fixed Unix locations).
//!
//! The schema (`tool.schema.yaml`, `search_paths`) maps a platform name to a
//! list of executable paths. The pinned Ruby code validates the field but never
//! reads it during discovery, so there is no oracle for this behavior and it is
//! not part of `find_with_info`. Rules kept deliberately narrow:
//!
//! - only the entry whose key equals the platform's Ruby name is used;
//! - entries are tested in listed order, literally, with `is_executable`;
//! - no glob expansion: an entry containing `*`, `?` or `[` is an error, not a
//!   guess (the schema example uses `C:/Program Files/ImageMagick*/magick.exe`);
//! - no `.app` directory probing beyond the literal path given.

use std::collections::BTreeMap;
use std::fmt;

use super::path::is_executable;
use super::Platform;

/// A `search_paths` entry that cannot be probed literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchPathError {
    GlobPattern { platform: Platform, entry: String },
}

impl fmt::Display for SearchPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchPathError::GlobPattern { platform, entry } => write!(
                f,
                "search_paths entry for {} is a glob pattern, which is not supported: {entry}",
                platform.as_str()
            ),
        }
    }
}

impl std::error::Error for SearchPathError {}

/// The entries listed for exactly this platform, validated as literal paths.
pub fn search_path_candidates(
    platform: Platform,
    search_paths: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, SearchPathError> {
    let Some(entries) = search_paths.get(platform.as_str()) else {
        return Ok(Vec::new());
    };
    if let Some(entry) = entries.iter().find(|entry| entry.contains(['*', '?', '['])) {
        return Err(SearchPathError::GlobPattern {
            platform,
            entry: entry.clone(),
        });
    }
    Ok(entries.clone())
}

/// The first listed path for this platform that is an executable file here.
pub fn probe_search_paths(
    platform: Platform,
    search_paths: &BTreeMap<String, Vec<String>>,
) -> Result<Option<String>, SearchPathError> {
    Ok(search_path_candidates(platform, search_paths)?
        .into_iter()
        .find(|path| is_executable(path)))
}
