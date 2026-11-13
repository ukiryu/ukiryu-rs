//! The `stdin` step of `ukiryu exec`.
//!
//! Source: `RunCommand#run` and `RunCommand#parse_inline_params` in
//! `lib/ukiryu/cli_commands/run_command.rb` at ukiryu/ukiryu ab6df353:
//!
//! ```ruby
//! skip_yaml_parse = key == 'stdin' && (value == '-' || value.start_with?('@'))
//! ...
//! if options[:stdin] || arguments[:stdin] == '-'
//!   arguments[:stdin] = $stdin.read
//! elsif arguments[:stdin]
//!   if arguments[:stdin].is_a?(String) && arguments[:stdin].start_with?('@')
//!     file_path = arguments[:stdin][1..]
//!     begin
//!       arguments[:stdin] = File.read(file_path)
//!     rescue Errno::ENOENT
//!       error! "File not found: #{file_path}"
//!     end
//!   end
//! end
//! ```
//!
//! Observed with Ruby 4.0.1 through the real CLI (`tests/stdin_data/cases.json`):
//!
//! - `--stdin` and `stdin=-` read the whole reader. EOF straight away gives
//!   an empty String that is still stored. `--stdin` wins over every
//!   `stdin=` value, including `stdin=@FILE`, a missing file and `false`.
//!   `--no-stdin` does not stop `stdin=-` from reading.
//! - `stdin=@FILE` reads the file as given: relative to the working
//!   directory, no `~` expansion. `stdin=@` reads the path `""`.
//! - Bytes are kept exactly as read (no newline conversion, BOM kept, invalid
//!   UTF-8 kept). The data is a UTF-8-tagged Ruby String that may be invalid;
//!   this module returns the bytes and leaves their representation to the
//!   caller, as `ukiryu::execution` does for process output. Nothing here
//!   replaces invalid bytes.
//! - A missing file (`ENOENT`, including a dangling symlink and the empty
//!   path) is the Thor error `File not found: <path>`. Any other `Errno`
//!   escapes to the CLI's top-level handler, which prints
//!   `[Errno::EISDIR] Is a directory @ io_fread - <path>` and exits 1. A read
//!   error on the process stdin names the path `<STDIN>`.
//! - The `@` test runs on the parsed String, so `stdin="@x"` (YAML-quoted)
//!   also reads the file `x`.
//! - Every other value is left alone: `nil` (`stdin=`, `stdin=null`), `false`,
//!   numbers, Hashes, and Strings that are neither `-` nor `@...`
//!   (`stdin=-literal`, `stdin=plain`).
//!
//! The source stores the result with `arguments[:stdin] = data`: an existing
//! `stdin` key keeps its position, otherwise the key is appended. [`store`]
//! does that for an ordered pair list.
//!
//! Not reproduced (fails as [`StdinError::Unmapped`] rather than guessing):
//! `Errno` classes other than EPERM, EACCES, ENOTDIR, EISDIR and ELOOP, and
//! every non-`ENOENT` error on platforms other than Linux and macOS. Ruby on
//! Windows reads in text mode; that has not been measured, so no newline
//! conversion is applied anywhere.
use std::fmt;
use std::fs::File;
use std::io::{self, Read};

/// The argument name the source treats specially.
pub const KEY: &str = "stdin";

/// The path Ruby's `$stdin` reports in `Errno` messages.
pub const STDIN_PATH: &str = "<STDIN>";

/// `parse_inline_params`' YAML bypass: `stdin=-` and `stdin=@...` keep the
/// raw text as a String instead of going through `YAML.safe_load`.
pub fn keeps_literal(key: &str, raw: &str) -> bool {
    key == KEY && (raw == "-" || raw.starts_with('@'))
}

/// The parsed `stdin` argument, as far as the stdin step looks at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param<'a> {
    /// No `stdin` key, or a Ruby-falsy value (`nil`, `false`).
    Falsy,
    /// A Ruby String.
    Text(&'a str),
    /// Any other truthy value (Integer, Float, Hash, Array, `true`, Symbol...).
    Other,
}

/// Where the stdin data comes from, decided before anything is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source<'a> {
    /// Read the whole reader (`--stdin` or `stdin=-`).
    Reader,
    /// Read this file (`stdin=@path`; the path is the text after `@`).
    File(&'a str),
    /// Leave the argument as it is.
    Keep,
}

/// The branch `RunCommand#run` takes for `options[:stdin]` (`--stdin`) and
/// the parsed `stdin` argument.
pub fn source(flag: bool, param: Param<'_>) -> Source<'_> {
    match param {
        _ if flag => Source::Reader,
        Param::Text("-") => Source::Reader,
        Param::Text(text) => match text.strip_prefix('@') {
            Some(path) => Source::File(path),
            None => Source::Keep,
        },
        Param::Falsy | Param::Other => Source::Keep,
    }
}

/// Why the stdin step stopped.
#[derive(Debug)]
pub enum StdinError {
    /// `error! "File not found: #{file_path}"` (a `Thor::Error`).
    FileNotFound { path: String },
    /// An `Errno` exception that escapes `run`.
    Raised {
        /// The Ruby class, e.g. `Errno::EISDIR`.
        class: &'static str,
        /// The exception message, e.g. `Is a directory @ io_fread - dir`.
        message: String,
    },
    /// An I/O error whose Ruby exception has not been measured.
    Unmapped { path: String, error: io::Error },
}

impl StdinError {
    /// The text the CLI prints: the Thor message as is, an `Errno` as
    /// `[Class] message` (`Ukiryu::ThorExt#format_error_message`). Both exit 1.
    pub fn cli_message(&self) -> String {
        match self {
            StdinError::FileNotFound { path } => format!("File not found: {path}"),
            StdinError::Raised { class, message } => format!("[{class}] {message}"),
            StdinError::Unmapped { path, error } => {
                format!("reading {path}: {error} (Ruby exception not reproduced)")
            }
        }
    }

    /// Whether this is the source's own `error!` (a `Thor::Error`).
    pub fn is_thor_error(&self) -> bool {
        matches!(self, StdinError::FileNotFound { .. })
    }
}

impl fmt::Display for StdinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.cli_message())
    }
}

impl std::error::Error for StdinError {}

/// The `rb_sys_fail_path` call site the message names.
#[derive(Clone, Copy)]
enum Stage {
    Open,
    Read,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Open => "rb_sysopen",
            Stage::Read => "io_fread",
        }
    }
}

/// `Errno` class and `strerror` text for the measured error numbers.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn errno(code: i32) -> Option<(&'static str, &'static str)> {
    #[cfg(target_os = "linux")]
    const ELOOP: i32 = 40;
    #[cfg(target_os = "macos")]
    const ELOOP: i32 = 62;
    Some(match code {
        1 => ("Errno::EPERM", "Operation not permitted"),
        13 => ("Errno::EACCES", "Permission denied"),
        20 => ("Errno::ENOTDIR", "Not a directory"),
        21 => ("Errno::EISDIR", "Is a directory"),
        ELOOP => ("Errno::ELOOP", "Too many levels of symbolic links"),
        _ => return None,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn errno(_code: i32) -> Option<(&'static str, &'static str)> {
    None
}

fn raised(path: &str, stage: Stage, error: io::Error) -> StdinError {
    match error.raw_os_error().and_then(errno) {
        Some((class, text)) => StdinError::Raised {
            class,
            message: format!("{text} @ {} - {path}", stage.name()),
        },
        None => StdinError::Unmapped {
            path: path.to_owned(),
            error,
        },
    }
}

/// `$stdin.read`: every byte up to EOF.
pub fn read_reader(reader: &mut dyn Read) -> Result<Vec<u8>, StdinError> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| raised(STDIN_PATH, Stage::Read, e))?;
    Ok(bytes)
}

/// `File.read(path)` with the source's `Errno::ENOENT` rescue.
pub fn read_file(path: &str) -> Result<Vec<u8>, StdinError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(StdinError::FileNotFound {
                path: path.to_owned(),
            })
        }
        Err(e) => return Err(raised(path, Stage::Open, e)),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| raised(path, Stage::Read, e))?;
    Ok(bytes)
}

/// The whole stdin step. `Ok(Some(bytes))` replaces the `stdin` argument
/// (see [`store`]); `Ok(None)` leaves it unchanged. `reader` is only read
/// for [`Source::Reader`].
pub fn resolve(
    flag: bool,
    param: Param<'_>,
    reader: &mut dyn Read,
) -> Result<Option<Vec<u8>>, StdinError> {
    match source(flag, param) {
        Source::Reader => read_reader(reader).map(Some),
        Source::File(path) => read_file(path).map(Some),
        Source::Keep => Ok(None),
    }
}

/// `arguments[:stdin] = value` on Hash-ordered pairs: replace in place,
/// append when absent.
pub fn store<V>(arguments: &mut Vec<(String, V)>, value: V) {
    match arguments.iter_mut().find(|(k, _)| k == KEY) {
        Some((_, slot)) => *slot = value,
        None => arguments.push((KEY.to_owned(), value)),
    }
}
