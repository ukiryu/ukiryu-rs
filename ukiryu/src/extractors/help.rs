//! Help-output extraction: port of `Ukiryu::Extractors::HelpParser`,
//! `NativeExtractor`, `BaseExtractor#execute_command` and the strategy
//! dispatch in `Extractor` (lib/ukiryu/extractors/*.rb at ab6df353).
//!
//! Parsing works on supplied text ([`parse_help_options`],
//! [`HelpDefinition::from_help_text`]). The flows that run `--help`,
//! `--version`, `which` and `--ukiryu-definition` take an injected
//! [`CommandHost`], so no process is started by this module.

mod case_table;
mod host;
mod native;
mod parser;
mod strategy;
mod text;

pub use host::{CommandHost, CommandOutput, ExtractError, HostError, EXTRACTION_TIMEOUT_SECS};
pub use native::{NativeExtractor, NATIVE_FLAG};
pub use parser::{
    extract_version_number, infer_type, is_command_not_found, parse_help_line, parse_help_options,
    HelpDefinition, HelpOption, OptionForm, OptionType,
};
pub use strategy::{extract, ExtractionMethod, ExtractionResult, ResultMethod};

use host::{execute_command, joined};

/// Source `HelpParser`: reverse-engineers a definition from `--help`.
pub struct HelpParser<'h, H: CommandHost + ?Sized> {
    tool_name: String,
    host: &'h mut H,
}

impl<'h, H: CommandHost + ?Sized> HelpParser<'h, H> {
    pub fn new(tool_name: impl Into<String>, host: &'h mut H) -> Self {
        HelpParser {
            tool_name: tool_name.into(),
            host,
        }
    }

    fn run(&mut self, arg: &str) -> Result<CommandOutput, ExtractError> {
        let tool = self.tool_name.clone();
        execute_command(&mut *self.host, &tool, &[arg])
    }

    /// Source `available?`: the tool is not reported missing and `--help`
    /// exits 0 with non-blank output. Runs `--help` twice, as the source does.
    pub fn available(&mut self) -> Result<bool, ExtractError> {
        if self.command_not_found()? {
            return Ok(false);
        }
        let help = self.run("--help")?;
        if help.exit_status != 0 {
            return Ok(false);
        }
        Ok(!text::strip_bytes(&joined(&help))?.is_empty())
    }

    /// Source `extract` up to the hash: `None` when unavailable or when the
    /// third `--help` run fails or is blank.
    pub fn extract_definition(&mut self) -> Result<Option<HelpDefinition>, ExtractError> {
        if !self.available()? {
            return Ok(None);
        }
        let help = self.run("--help")?;
        if help.exit_status != 0 {
            return Ok(None);
        }
        let help_bytes = joined(&help);
        if text::strip_bytes(&help_bytes)?.is_empty() {
            return Ok(None);
        }
        // strip_bytes succeeded, so the text is valid UTF-8.
        let help_text = text::regex_input(&help_bytes)?;
        let version = self.extract_version()?;
        Ok(Some(HelpDefinition::from_help_text(
            &self.tool_name,
            version.as_deref(),
            help_text,
        )))
    }

    /// Source `extract`: the YAML definition, or `None`. The YAML is emitted
    /// by yeptris, so it carries the source's data but not Psych's layout.
    pub fn extract(&mut self) -> Result<Option<String>, ExtractError> {
        self.extract_definition()?
            .map(|definition| definition.to_yaml().map_err(yaml_error))
            .transpose()
    }

    /// Source `extract_version`: the first version number in `--version`
    /// stdout when it exits 0.
    fn extract_version(&mut self) -> Result<Option<String>, ExtractError> {
        let result = self.run("--version")?;
        if result.exit_status != 0 {
            return Ok(None);
        }
        let stdout = text::regex_input(&result.stdout)?;
        Ok(extract_version_number(stdout).map(str::to_owned))
    }

    /// Source `command_not_found?` (the `UKIRYU_DEBUG_HELP_PARSER` stderr
    /// logging is not ported).
    fn command_not_found(&mut self) -> Result<bool, ExtractError> {
        let help = self.run("--help")?;
        let combined = [help.stdout.as_slice(), b" ", help.stderr.as_slice()].concat();
        text::regex_input(&combined)?;
        // Both halves are valid when the joined text is.
        let stdout = String::from_utf8_lossy(&help.stdout);
        let stderr = String::from_utf8_lossy(&help.stderr);
        Ok(is_command_not_found(&stdout, &stderr))
    }
}

/// The yeptris emitter failing on a plain string tree is not a source
/// behaviour; surface it as a Ruby-shaped error rather than panicking.
fn yaml_error(error: yeptris::Error) -> ExtractError {
    ExtractError::yaml_emit(&error.to_string())
}
