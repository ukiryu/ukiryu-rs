//! Source: lib/ukiryu/models/output_info.rb (`Ukiryu::Models::OutputInfo`).
use super::attr::{Attr, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

fn default_empty() -> Attr<Text> {
    Attr::Set(Text::new(""))
}

/// Captured process output of a success response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    #[serde(default = "default_empty", skip_serializing_if = "Attr::omit")]
    pub stdout: Attr<Text>,
    #[serde(default = "default_empty", skip_serializing_if = "Attr::omit")]
    pub stderr: Attr<Text>,
}

impl Default for OutputInfo {
    fn default() -> Self {
        OutputInfo {
            stdout: Attr::Default(Text::new("")),
            stderr: Attr::Default(Text::new("")),
        }
    }
}

impl Model for OutputInfo {}
