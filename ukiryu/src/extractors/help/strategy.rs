//! Port of `Ukiryu::Extractors::Extractor` (extractor.rb): try the native
//! flag, then help parsing, and report the outcome with the source's
//! messages.

use super::host::{CommandHost, ExtractError};
use super::native::NativeExtractor;
use super::HelpParser;

/// The `method:` option. The source compares against the symbols `:auto`,
/// `:native` and `:help`; anything else, including the string `"help"`, is
/// unknown, so callers pass such values as [`ExtractionMethod::Other`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractionMethod {
    Auto,
    Native,
    Help,
    /// Any other value, as its `to_s`.
    Other(String),
}

/// The `:method` value of a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultMethod {
    Native,
    Help,
}

impl ResultMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            ResultMethod::Native => "native",
            ResultMethod::Help => "help",
        }
    }
}

/// The source's `{success:, yaml:, method:, error:}` result hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractionResult {
    pub success: bool,
    pub yaml: Option<String>,
    pub method: Option<ResultMethod>,
    pub error: Option<String>,
}

impl ExtractionResult {
    fn ok(yaml: String, method: ResultMethod) -> ExtractionResult {
        ExtractionResult {
            success: true,
            yaml: Some(yaml),
            method: Some(method),
            error: None,
        }
    }

    fn failed(error: String, method: Option<ResultMethod>) -> ExtractionResult {
        ExtractionResult {
            success: false,
            yaml: None,
            method,
            error: Some(error),
        }
    }
}

/// Source `Extractor.extract(tool_name, method: ...)`; `None` means the
/// option was not given (`:auto`). Exceptions raised by the strategies
/// propagate, as in the source.
pub fn extract<H: CommandHost + ?Sized>(
    tool_name: &str,
    method: Option<ExtractionMethod>,
    host: &mut H,
) -> Result<ExtractionResult, ExtractError> {
    match method.unwrap_or(ExtractionMethod::Auto) {
        ExtractionMethod::Auto => {
            let native = extract_with_native(tool_name, host)?;
            if native.success {
                return Ok(native);
            }
            extract_with_help(tool_name, host)
        }
        ExtractionMethod::Native => extract_with_native(tool_name, host),
        ExtractionMethod::Help => extract_with_help(tool_name, host),
        ExtractionMethod::Other(name) => Ok(ExtractionResult::failed(
            format!("Unknown extraction method: {name}"),
            None,
        )),
    }
}

fn extract_with_native<H: CommandHost + ?Sized>(
    tool_name: &str,
    host: &mut H,
) -> Result<ExtractionResult, ExtractError> {
    let mut extractor = NativeExtractor::new(tool_name, host);
    if !extractor.available()? {
        return Ok(ExtractionResult::failed(
            format!("Tool '{tool_name}' does not support native definition extraction"),
            Some(ResultMethod::Native),
        ));
    }
    Ok(match extractor.extract()? {
        Some(yaml) => ExtractionResult::ok(yaml, ResultMethod::Native),
        None => ExtractionResult::failed(
            "Native extraction failed".to_owned(),
            Some(ResultMethod::Native),
        ),
    })
}

fn extract_with_help<H: CommandHost + ?Sized>(
    tool_name: &str,
    host: &mut H,
) -> Result<ExtractionResult, ExtractError> {
    let mut parser = HelpParser::new(tool_name, host);
    if !parser.available()? {
        return Ok(ExtractionResult::failed(
            format!("Tool '{tool_name}' does not have help output"),
            Some(ResultMethod::Help),
        ));
    }
    Ok(match parser.extract()? {
        Some(yaml) => ExtractionResult::ok(yaml, ResultMethod::Help),
        None => {
            ExtractionResult::failed("Help parsing failed".to_owned(), Some(ResultMethod::Help))
        }
    })
}
