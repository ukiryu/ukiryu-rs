//! Source: lib/ukiryu/models/execution_report.rb (`Ukiryu::Models::ExecutionReport`).
use super::attr::{de_fns as d, omit_model, Attr, Model};
use super::ruby::{float_sum, format_duration, Number};
use super::run_environment::RunEnvironment;
use super::stage_metrics::StageMetrics;
use super::value::Text;
use serde::{Deserialize, Serialize};

fn d0() -> Attr<f64> {
    Attr::Set(0.0)
}
fn s() -> Attr<Text> {
    Attr::Set(Text::new(""))
}

/// Per-stage metrics of one execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionReport {
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub tool_resolution: Option<Box<StageMetrics>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub command_building: Option<Box<StageMetrics>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub execution: Option<Box<StageMetrics>>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub response_building: Option<Box<StageMetrics>>,
    #[serde(default = "d0", skip_serializing_if = "Attr::omit")]
    pub total_duration: Attr<f64>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub formatted_total_duration: Attr<Text>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub run_environment: Option<Box<RunEnvironment>>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub timestamp: Attr<Text>,
}

impl Default for ExecutionReport {
    fn default() -> Self {
        ExecutionReport {
            tool_resolution: None,
            command_building: None,
            execution: None,
            response_building: None,
            total_duration: Attr::Default(0.0),
            formatted_total_duration: Attr::Default(Text::new("")),
            run_environment: None,
            timestamp: Attr::Default(Text::new("")),
        }
    }
}

impl ExecutionReport {
    /// `#all_stages` (present stages in pipeline order).
    pub fn all_stages(&self) -> Vec<&StageMetrics> {
        [
            &self.tool_resolution,
            &self.command_building,
            &self.execution,
            &self.response_building,
        ]
        .into_iter()
        .filter_map(|s| s.as_deref())
        .collect()
    }

    /// `#calculate_total`: `Array#sum` of the stage durations.
    pub fn calculate_total(&mut self) {
        let durations: Vec<f64> = self
            .all_stages()
            .iter()
            .map(|s| s.duration.get().copied().unwrap_or(0.0))
            .collect();
        let total = if durations.is_empty() {
            Number::Int(0)
        } else {
            Number::Float(float_sum(&durations))
        };
        let as_float = match total {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        };
        self.total_duration.assign_ivar(as_float);
        self.formatted_total_duration
            .assign_ivar(Text::new(format_duration(total)));
    }
}

impl Model for ExecutionReport {}
