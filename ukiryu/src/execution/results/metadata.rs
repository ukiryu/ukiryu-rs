//! Source: lib/ukiryu/execution/metadata.rb (`Ukiryu::Execution::ExecutionMetadata`).
//!
//! The runtime timing record of one execution. This is not the serialisable
//! `Ukiryu::Models::ExecutionMetadata` response model (K17 models), which
//! carries ISO strings and is built from this value.
use serde::{Serialize, Serializer};

use super::ruby::{float_round, time_sub, to_ruby_json, JsonGeneratorError};
use crate::types::ruby::{float_to_s, time_to_s};
use crate::types::value::Time;

/// A Ruby numeric seconds value (`timeout:`), kept as Integer or Float because
/// both its JSON form and its interpolation differ (`30` vs `30.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seconds {
    Int(i64),
    Float(f64),
}

impl Seconds {
    /// `to_s` as string interpolation renders it.
    pub fn to_ruby_s(self) -> String {
        match self {
            Seconds::Int(i) => i.to_string(),
            Seconds::Float(f) => float_to_s(f),
        }
    }

    /// `duration > self` with Ruby's exact Float/Integer comparison.
    fn exceeded_by(self, duration: f64) -> bool {
        match self {
            Seconds::Float(f) => duration > f,
            Seconds::Int(i) => {
                if duration.is_nan() || duration <= -9.3e18 {
                    return false;
                }
                if duration >= 9.3e18 {
                    return true;
                }
                let floor = duration.floor();
                let whole = floor as i128;
                whole > i as i128 || (whole == i as i128 && duration > floor)
            }
        }
    }
}

impl Serialize for Seconds {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Seconds::Int(i) => s.serialize_i64(*i),
            Seconds::Float(f) => s.serialize_f64(*f),
        }
    }
}

/// `Ukiryu::Execution::ExecutionMetadata`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionMetadata {
    pub started_at: Option<Time>,
    pub finished_at: Option<Time>,
    pub timeout: Option<Seconds>,
    duration: Option<f64>,
}

/// `ExecutionMetadata#to_h`, in Ruby key order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionMetadataHash {
    /// `Time` values serialise through `Time#to_s`, as the json gem does.
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub duration_milliseconds: Option<f64>,
    pub timeout: Option<Seconds>,
    pub timed_out: bool,
}

impl ExecutionMetadataHash {
    pub(crate) fn floats(&self) -> Vec<f64> {
        let mut v: Vec<f64> = [
            self.duration,
            self.duration_seconds,
            self.duration_milliseconds,
        ]
        .into_iter()
        .flatten()
        .collect();
        if let Some(Seconds::Float(f)) = self.timeout {
            v.push(f);
        }
        v
    }
}

impl ExecutionMetadata {
    /// `ExecutionMetadata.new(started_at:, finished_at:, timeout:)`; the
    /// duration is computed once, as in the constructor.
    pub fn new(
        started_at: Option<Time>,
        finished_at: Option<Time>,
        timeout: Option<Seconds>,
    ) -> Self {
        let duration = match (&started_at, &finished_at) {
            (Some(s), Some(f)) => Some(time_sub(f, s)),
            _ => None,
        };
        ExecutionMetadata {
            started_at,
            finished_at,
            timeout,
            duration,
        }
    }

    /// `#duration` / `#duration_seconds`.
    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        self.duration
    }

    /// `#duration_milliseconds`: `duration * 1000`.
    pub fn duration_milliseconds(&self) -> Option<f64> {
        self.duration.map(|d| d * 1000.0)
    }

    /// `#timed_out?`: a set timeout (including `0`) that the duration exceeds.
    pub fn is_timed_out(&self) -> bool {
        match (self.timeout, self.duration) {
            (Some(t), Some(d)) => t.exceeded_by(d),
            _ => false,
        }
    }

    /// `#formatted_duration`.
    pub fn formatted_duration(&self) -> String {
        match self.duration {
            None => "N/A".to_owned(),
            Some(d) => format_duration(d),
        }
    }

    /// `#to_h`.
    pub fn to_h(&self) -> ExecutionMetadataHash {
        ExecutionMetadataHash {
            started_at: self.started_at.as_ref().map(time_to_s),
            finished_at: self.finished_at.as_ref().map(time_to_s),
            duration: self.duration,
            duration_seconds: self.duration,
            duration_milliseconds: self.duration_milliseconds(),
            timeout: self.timeout,
            timed_out: self.is_timed_out(),
        }
    }

    /// `to_h.to_json`.
    pub fn to_json(&self) -> Result<String, JsonGeneratorError> {
        let h = self.to_h();
        to_ruby_json(&h, &h.floats())
    }

    /// `#inspect`.
    pub fn inspect(&self) -> String {
        format!(
            "#<Ukiryu::Execution::ExecutionMetadata duration={}>",
            self.formatted_duration()
        )
    }
}

/// `#to_s`: `duration: <formatted>`.
impl std::fmt::Display for ExecutionMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "duration: {}", self.formatted_duration())
    }
}

/// The non-nil branch of `#formatted_duration` for a Float duration.
pub fn format_duration(d: f64) -> String {
    if d < 1.0 {
        format!("{}ms", float_to_s(float_round(d * 1000.0, 2)))
    } else if d < 60.0 {
        format!("{}s", float_to_s(float_round(d, 3)))
    } else {
        let minutes = d / 60.0;
        let mut seconds = d % 60.0;
        // Float#% (flodivmod): a result with the divisor's sign.
        if seconds < 0.0 {
            seconds += 60.0;
        }
        format!(
            "{}m{}s",
            minutes.trunc() as i64,
            float_to_s(float_round(seconds, 1))
        )
    }
}
