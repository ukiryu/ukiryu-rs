//! Port of `Ukiryu::Extractors::NativeExtractor`: asks a tool for its own
//! definition through `--ukiryu-definition` when its `--help` mentions it.

use super::host::{execute_command, joined, CommandHost, ExtractError};
use super::text;

/// The flag a tool answers with its own definition.
pub const NATIVE_FLAG: &str = "--ukiryu-definition";

/// Source `NativeExtractor`.
pub struct NativeExtractor<'h, H: CommandHost + ?Sized> {
    tool_name: String,
    host: &'h mut H,
}

impl<'h, H: CommandHost + ?Sized> NativeExtractor<'h, H> {
    pub fn new(tool_name: impl Into<String>, host: &'h mut H) -> Self {
        NativeExtractor {
            tool_name: tool_name.into(),
            host,
        }
    }

    /// Source `available?`: `which <tool>` exits 0, `--help` exits 0, and
    /// the help output mentions `ukiryu` (case-insensitively) or the flag.
    pub fn available(&mut self) -> Result<bool, ExtractError> {
        let tool = self.tool_name.clone();
        let which = execute_command(&mut *self.host, "which", &[&tool])?;
        if which.exit_status != 0 {
            return Ok(false);
        }
        let help = execute_command(&mut *self.host, &tool, &["--help"])?;
        if help.exit_status != 0 {
            return Ok(false);
        }
        let output = joined(&help);
        let lower = text::downcase_bytes(&output)?;
        Ok(lower.contains("ukiryu")
            || output
                .windows(NATIVE_FLAG.len())
                .any(|w| w == NATIVE_FLAG.as_bytes()))
    }

    /// Source `extract`: the tool's `--ukiryu-definition` stdout, unchanged,
    /// when it exits 0 with non-blank stdout.
    pub fn extract(&mut self) -> Result<Option<String>, ExtractError> {
        if !self.available()? {
            return Ok(None);
        }
        let tool = self.tool_name.clone();
        let result = execute_command(&mut *self.host, &tool, &[NATIVE_FLAG])?;
        if result.exit_status != 0 {
            return Ok(None);
        }
        if text::strip_bytes(&result.stdout)?.is_empty() {
            return Ok(None);
        }
        // strip_bytes succeeded, so stdout is valid UTF-8.
        Ok(Some(text::regex_input(&result.stdout)?.to_owned()))
    }
}
