//! `Ukiryu::Execution`: the result of running one command and its views.
//!
//! Source: lib/ukiryu/execution.rb and lib/ukiryu/execution/*.rb at
//! ukiryu/ukiryu ab6df353. These are the runtime values the executor returns.
//! The serialisable response models (`Ukiryu::Models::CommandInfo`,
//! `OutputInfo`, `ExecutionMetadata`, `SuccessResponse`, `ErrorResponse`,
//! `ExecutionReport`, `ExitCodes`) are separate Ruby classes and live in the
//! models module. The process executor itself is not part of this module.
pub mod result;
pub mod results;

pub use result::{ExecutionResult, ExecutionResultHash};
pub use results::command_info::{CommandInfo, CommandInfoHash};
pub use results::exit_codes::{exit_code_meaning, format_meaning, ExitCodeTable};
pub use results::metadata::{format_duration, ExecutionMetadata, ExecutionMetadataHash, Seconds};
pub use results::output::{Output, OutputHash, Pattern};
pub use results::ruby::{JsonGeneratorError, PathStyle};
