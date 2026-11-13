//! Source: lib/ukiryu/models/success_response.rb (`Ukiryu::Models::SuccessResponse`).
use super::arguments::Arguments;
use super::attr::{de_fns as d, omit_model, omit_opt, Attr, Model};
use super::command_definition::CommandDefinition;
use super::command_info::CommandInfo;
use super::error::ModelError;
use super::execution_metadata::ExecutionMetadata;
use super::execution_report::ExecutionReport;
use super::output_info::OutputInfo;
use super::value::{RubyValue, Text};
use serde::{Deserialize, Serialize};

fn default_status() -> Attr<Text> {
    Attr::Set(Text::new("success"))
}

/// The structured success response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuccessResponse {
    #[serde(default = "default_status", skip_serializing_if = "Attr::omit")]
    pub status: Attr<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub exit_code: Option<i64>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub request: Option<Box<Arguments>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub command: Option<Box<CommandInfo>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub output: Option<Box<OutputInfo>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub metadata: Option<Box<ExecutionMetadata>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub execution_report: Option<Box<ExecutionReport>>,
}

impl Default for SuccessResponse {
    fn default() -> Self {
        SuccessResponse {
            status: Attr::Default(Text::new("success")),
            exit_code: None,
            request: None,
            command: None,
            output: None,
            metadata: None,
            execution_report: None,
        }
    }
}

/// The parts of an executor result that `SuccessResponse.from_result` reads
/// (`result.status`, `result.executable`, `result.command_info.full_command`,
/// `result.command_info.shell.to_s`, `result.stdout`, `result.error_output`,
/// `started_at.iso8601`, `finished_at.iso8601`, `result.duration` and
/// `result.metadata.formatted_duration`).
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionResultParts {
    pub status: i64,
    pub executable: String,
    pub full_command: String,
    pub shell: String,
    pub stdout: String,
    pub error_output: String,
    pub started_at_iso8601: String,
    pub finished_at_iso8601: String,
    pub duration: f64,
    pub formatted_duration: String,
}

impl SuccessResponse {
    /// `SuccessResponse.from_result(result, params, command_definition, execution_report:)`.
    pub fn from_result(
        result: &ExecutionResultParts,
        params: &[(String, RubyValue)],
        command_definition: Option<&CommandDefinition>,
        execution_report: Option<ExecutionReport>,
    ) -> Result<SuccessResponse, ModelError> {
        let request = Arguments::from_params(params, command_definition)?;
        Ok(SuccessResponse {
            status: Attr::Set(Text::new("success")),
            exit_code: Some(result.status),
            command: Some(Box::new(CommandInfo {
                executable: Some(Text::new(result.executable.clone())),
                arguments: Some(Box::new(request.clone())),
                full_command: Some(Text::new(result.full_command.clone())),
                shell: Some(Text::new(result.shell.clone())),
                ..CommandInfo::default()
            })),
            request: Some(Box::new(request)),
            output: Some(Box::new(OutputInfo {
                stdout: Attr::Set(Text::new(result.stdout.clone())),
                stderr: Attr::Set(Text::new(result.error_output.clone())),
            })),
            metadata: Some(Box::new(ExecutionMetadata {
                started_at: Some(Text::new(result.started_at_iso8601.clone())),
                finished_at: Some(Text::new(result.finished_at_iso8601.clone())),
                duration_seconds: Some(result.duration),
                formatted_duration: Some(Text::new(result.formatted_duration.clone())),
            })),
            execution_report: execution_report.map(Box::new),
        })
    }
}

impl Model for SuccessResponse {}
