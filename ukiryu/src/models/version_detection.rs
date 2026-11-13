//! Source: lib/ukiryu/models/version_detection.rb
//! (`Ukiryu::Models::VersionDetectionMethod`, `Ukiryu::Models::VersionDetection`).
use super::argument_definition::{opt_value, text_list_value};
use super::attr::{de_fns as d, omit_list, omit_opt, ser_model_list, Attr, Model};
use super::value::{RubyValue, Text};
use serde::{Deserialize, Serialize};

/// One detection method (`command` or `man_page`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VersionDetectionMethod {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub r#type: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub command: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub pattern: Option<Text>,
    /// Platform-specific man page paths (`:hash`, no default).
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub paths: Option<RubyValue>,
}

impl Model for VersionDetectionMethod {}

fn default_source() -> Attr<Text> {
    Attr::Set(Text::new("command"))
}

/// How a tool's version is detected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VersionDetection {
    #[serde(
        default = "d::empty",
        deserialize_with = "d::text_list",
        skip_serializing_if = "omit_list"
    )]
    pub command: Option<Vec<Text>>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub pattern: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub modern_threshold: Option<Text>,
    #[serde(default = "default_source", skip_serializing_if = "Attr::omit")]
    pub source: Attr<Text>,
    #[serde(
        default = "d::empty",
        deserialize_with = "d::model_list",
        serialize_with = "ser_model_list",
        skip_serializing_if = "omit_list"
    )]
    pub detection_methods: Option<Vec<VersionDetectionMethod>>,
}

impl Default for VersionDetection {
    fn default() -> Self {
        VersionDetection {
            command: Some(Vec::new()),
            pattern: None,
            modern_threshold: None,
            source: Attr::Default(Text::new("command")),
            detection_methods: Some(Vec::new()),
        }
    }
}

impl VersionDetection {
    /// `#[](key)`: attribute readers by name.
    pub fn index(&self, key: &str) -> Option<RubyValue> {
        Some(match key {
            "command" => text_list_value(&self.command),
            "pattern" => opt_value(self.pattern.as_ref()),
            "modern_threshold" => opt_value(self.modern_threshold.as_ref()),
            "source" => opt_value(self.source.get()),
            _ => return None,
        })
    }
}

impl Model for VersionDetection {}
