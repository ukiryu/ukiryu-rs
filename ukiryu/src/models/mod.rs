//! Typed domain models ported from `lib/ukiryu/models/**` of the Ruby gem.
//!
//! Every lutaml-serializable class is a serde struct whose fields follow the
//! lutaml mapping order and keys; YAML goes through Yeptris and JSON through
//! serde_json. The attribute semantics observed at the pinned oracle
//! (defaults, explicit nulls, omission of empty values, scalar casting,
//! single-value wrapping for collections) live in [`attr`]. Plain Ruby
//! classes that hold raw YAML Hashes keep them as [`RubyValue`].
//!
//! Where lutaml keeps an uncastable raw object (for example a mapping in a
//! `:string` attribute or `"off"` in a `:boolean` attribute) the port fails
//! with [`ModelErrorKind::Unsupported`] instead of guessing a type.
//!
//! Out of scope here (host probing, detection and execution): the defaults
//! of `ToolDefinition#compatible_profile`, `RunEnvironment.collect` and the
//! process sampling of `StageMetrics#start!`/`#finish!`, whose measured
//! values are passed in by the caller.
pub mod attr;
pub mod error;
pub mod ruby;
pub mod value;

pub mod argument;
pub mod argument_definition;
pub mod arguments;
pub mod command_definition;
pub mod command_info;
pub mod components;
pub mod env_var_definition;
pub mod error_response;
pub mod executable_info;
pub mod execution_metadata;
pub mod execution_report;
pub mod exit_codes;
pub mod flag_definition;
pub mod implementation_index;
pub mod implementation_version;
pub mod interface;
pub mod invocation;
pub mod option_definition;
pub mod output_info;
pub mod platform_profile;
pub mod routing;
pub mod run_environment;
pub mod semantic_version;
pub mod stage_metrics;
pub mod success_response;
pub mod tool_definition;
pub mod tool_metadata;
pub mod validation_result;
pub mod version_compatibility;
pub mod version_detection;
pub mod version_info;

pub use argument::Argument;
pub use argument_definition::{ArgumentDefinition, ParsedPosition};
pub use arguments::{ArgumentKind, Arguments};
pub use attr::{Attr, Model};
pub use command_definition::CommandDefinition;
pub use command_info::CommandInfo;
pub use components::{ComponentRef, Components};
pub use env_var_definition::EnvVarDefinition;
pub use error::{ModelError, ModelErrorKind};
pub use error_response::ErrorResponse;
pub use executable_info::ExecutableInfo;
pub use execution_metadata::ExecutionMetadata;
pub use execution_report::ExecutionReport;
pub use exit_codes::ExitCodes;
pub use flag_definition::FlagDefinition;
pub use implementation_index::ImplementationIndex;
pub use implementation_version::{ExecutionProfile, ImplementationVersion};
pub use interface::{register_hash, Interface};
pub use invocation::Invocation;
pub use option_definition::OptionDefinition;
pub use output_info::OutputInfo;
pub use platform_profile::PlatformProfile;
pub use routing::Routing;
pub use run_environment::RunEnvironment;
pub use semantic_version::{SemanticVersion, VersionOperand};
pub use stage_metrics::StageMetrics;
pub use success_response::{ExecutionResultParts, SuccessResponse};
pub use tool_definition::ToolDefinition;
pub use tool_metadata::ToolMetadata;
pub use validation_result::ValidationResult;
pub use value::{RubyValue, Text};
pub use version_compatibility::{Constraint, RequirementParser, VersionCompatibility};
pub use version_detection::{VersionDetection, VersionDetectionMethod};
pub use version_info::VersionInfo;
