//! Source: lib/ukiryu/models/execution_metadata.rb (`Ukiryu::Models::ExecutionMetadata`).
use super::attr::{de_fns as d, omit_opt, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

/// Timing metadata of a success response.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionMetadata {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub started_at: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub finished_at: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub duration_seconds: Option<f64>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub formatted_duration: Option<Text>,
}

impl Model for ExecutionMetadata {}
