//! Source: `lib/ukiryu/definition/sources/file.rb`
//! (`Ukiryu::Definition::Sources::FileSource`).
use crate::definition::source::{sha256_hex, Source, SourceType};
use crate::definition::{DefinitionError, Result};
use std::cell::OnceCell;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A definition read from the filesystem, tracking its mtime.
#[derive(Clone, Debug)]
pub struct FileSource {
    path: String,
    mtime: Option<SystemTime>,
    content: Option<String>,
    real_path: OnceCell<String>,
}

impl FileSource {
    /// `FileSource.new(path)`: expands the path, then checks existence
    /// (`DefinitionNotFoundError`) and readability (`DefinitionLoadError`).
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = expand_path(path.as_ref());
        if !Path::new(&path).exists() {
            return Err(DefinitionError::not_found(format!(
                "Definition file not found: {path}"
            )));
        }
        if let Err(e) = std::fs::File::open(&path) {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                return Err(DefinitionError::load(format!(
                    "Definition file is not readable: {path}"
                )));
            }
        }
        Ok(FileSource {
            path,
            mtime: None,
            content: None,
            real_path: OnceCell::new(),
        })
    }

    /// Absolute (expanded, not symlink-resolved) path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The mtime recorded by the first `load`, if any.
    pub fn mtime(&self) -> Option<SystemTime> {
        self.mtime
    }

    /// `real_path`: symlinks resolved, or the path itself when that fails.
    /// Memoized on first call, as in Ruby.
    pub fn real_path(&self) -> &str {
        self.real_path
            .get_or_init(|| match std::fs::canonicalize(&self.path) {
                Ok(p) => p.to_string_lossy().into_owned(),
                Err(_) => self.path.clone(),
            })
    }

    fn current_mtime(&self) -> Result<SystemTime> {
        std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .map_err(|e| {
                DefinitionError::load(format!(
                    "Cannot access file metadata for {p}: {} @ rb_file_s_mtime - {p}",
                    strerror(&e),
                    p = self.path
                ))
            })
    }

    fn read_file(&self) -> Result<String> {
        let p = &self.path;
        let mut file = std::fs::File::open(p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                DefinitionError::load(format!(
                    "Permission denied reading file {p}: {} @ rb_sysopen - {p}",
                    strerror(&e)
                ))
            } else {
                DefinitionError::load(format!(
                    "Error reading file {p}: {} @ rb_sysopen - {p}",
                    strerror(&e)
                ))
            }
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| {
            DefinitionError::load(format!(
                "Error reading file {p}: {} @ io_fread - {p}",
                strerror(&e)
            ))
        })?;
        String::from_utf8(bytes).map_err(|_| {
            DefinitionError::load(format!("Error reading file {p}: invalid UTF-8 content"))
        })
    }
}

impl Source for FileSource {
    /// Fails when the file's mtime changed since the first load; otherwise
    /// returns the content read on the first load.
    fn load(&mut self) -> Result<String> {
        let current = self.current_mtime()?;
        if let Some(original) = self.mtime {
            if original != current {
                return Err(DefinitionError::load(format!(
                    "File {} has been modified since it was loaded. Original mtime: {}, Current mtime: {}",
                    self.path,
                    ruby_time_to_s(original),
                    ruby_time_to_s(current)
                )));
            }
        }
        self.mtime = Some(current);
        if self.content.is_none() {
            self.content = Some(self.read_file()?);
        }
        Ok(self.content.clone().unwrap_or_default())
    }

    /// `"file:#{sha256(path)}:#{mtime}"`, using the current mtime before the first load.
    fn cache_key(&self) -> Result<String> {
        let mtime = match self.mtime {
            Some(m) => m,
            None => self.current_mtime()?,
        };
        Ok(format!(
            "file:{}:{}",
            sha256_hex(&self.path),
            ruby_time_to_s(mtime)
        ))
    }

    fn source_type(&self) -> SourceType {
        SourceType::File
    }

    fn ruby_class_name(&self) -> &'static str {
        "Ukiryu::Definition::Sources::FileSource"
    }
}

/// OS error text without Rust's ` (os error N)` suffix, as Ruby's Errno message prefix.
pub(crate) fn strerror(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

/// Ruby `File.expand_path(path)`: `~` via `HOME`, relative paths against the
/// current directory, `.`/`..` removed lexically (symlinks are not resolved).
pub(crate) fn expand_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let path: PathBuf = if text == "~" || text.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(format!("{home}{}", &text[1..]))
    } else {
        path.to_path_buf()
    };
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// Ruby `Time#to_s` for a file time, rendered in UTC
/// (`"%Y-%m-%d %H:%M:%S +0000"`). Ruby uses the process local zone; the
/// reference captures run with `TZ=UTC`.
pub fn ruby_time_to_s(t: SystemTime) -> String {
    let secs: i64 = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => {
            let d = e.duration();
            -(d.as_secs() as i64) - i64::from(d.subsec_nanos() > 0)
        }
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
