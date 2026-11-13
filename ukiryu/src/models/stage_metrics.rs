//! Source: lib/ukiryu/models/stage_metrics.rb (`Ukiryu::Models::StageMetrics`).
//!
//! `start!`/`finish!` sample wall time and the process RSS through `ps`;
//! process sampling is executor semantics, so the port takes the measured
//! values as arguments and keeps the Ruby state transitions.
use super::attr::{Attr, Model};
use super::ruby::{format_duration as fmt_duration, Number};
use super::value::Text;
use serde::{Deserialize, Serialize};

fn s() -> Attr<Text> {
    Attr::Set(Text::new(""))
}
fn d0() -> Attr<f64> {
    Attr::Set(0.0)
}
fn z() -> Attr<i64> {
    Attr::Set(0)
}
fn t() -> Attr<bool> {
    Attr::Set(true)
}

/// Timing and memory of one execution stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageMetrics {
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub name: Attr<Text>,
    #[serde(default = "d0", skip_serializing_if = "Attr::omit")]
    pub duration: Attr<f64>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub formatted_duration: Attr<Text>,
    #[serde(default = "z", skip_serializing_if = "Attr::omit")]
    pub memory_before: Attr<i64>,
    #[serde(default = "z", skip_serializing_if = "Attr::omit")]
    pub memory_after: Attr<i64>,
    #[serde(default = "z", skip_serializing_if = "Attr::omit")]
    pub memory_delta: Attr<i64>,
    #[serde(default = "t", skip_serializing_if = "Attr::omit")]
    pub success: Attr<bool>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub error: Attr<Text>,
    /// `@start_time` (seconds on the caller's clock); not serialized.
    #[serde(skip)]
    pub start_time: Option<f64>,
}

impl Default for StageMetrics {
    fn default() -> Self {
        StageMetrics {
            name: Attr::Default(Text::new("")),
            duration: Attr::Default(0.0),
            formatted_duration: Attr::Default(Text::new("")),
            memory_before: Attr::Default(0),
            memory_after: Attr::Default(0),
            memory_delta: Attr::Default(0),
            success: Attr::Default(true),
            error: Attr::Default(Text::new("")),
            start_time: None,
        }
    }
}

/// The private `format_duration(seconds)` (`nil` or zero → `"0ms"`).
pub fn format_duration(seconds: Option<f64>) -> String {
    match seconds {
        None => "0ms".into(),
        Some(s) => fmt_duration(Number::Float(s)),
    }
}

impl StageMetrics {
    /// `StageMetrics.new(name:)`.
    pub fn named(name: &str) -> Self {
        StageMetrics {
            name: Attr::Set(Text::new(name)),
            ..StageMetrics::default()
        }
    }

    /// `#start!` with the sampled clock and RSS (KiB).
    pub fn start(&mut self, now: f64, memory_before: i64) -> &mut Self {
        self.start_time = Some(now);
        self.memory_before.assign_ivar(memory_before);
        self
    }

    /// `#finish!(success:, error:)` with the sampled clock and RSS. Like the
    /// Ruby ivar writes, implicit defaults stay unserialized.
    pub fn finish(&mut self, now: f64, memory_after: i64, success: bool, error: Option<&str>) {
        if let Some(start) = self.start_time {
            self.duration.assign_ivar(now - start);
        }
        self.formatted_duration
            .assign_ivar(Text::new(format_duration(self.duration.get().copied())));
        self.success.assign_ivar(success);
        if let Some(e) = error {
            self.error.assign_ivar(Text::new(e));
        }
        self.memory_after.assign_ivar(memory_after);
        let before = self.memory_before.get().copied().unwrap_or(0);
        self.memory_delta.assign_ivar(memory_after - before);
    }
}

impl Model for StageMetrics {}
