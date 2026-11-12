//! Source: lib/ukiryu/models/run_environment.rb (`Ukiryu::Models::RunEnvironment`).
//!
//! `RunEnvironment.collect` probes the host (hostname, shell detection,
//! `ps`/`sysctl`) and belongs to the excluded detection/executor semantics;
//! callers build the model from measured values instead.
use super::attr::{Attr, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

fn s() -> Attr<Text> {
    Attr::Set(Text::new(""))
}
fn f() -> Attr<bool> {
    Attr::Set(false)
}
fn z() -> Attr<i64> {
    Attr::Set(0)
}

/// The host a command ran on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunEnvironment {
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub hostname: Attr<Text>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub platform: Attr<Text>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub os_version: Attr<Text>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub shell: Attr<Text>,
    #[serde(default = "f", skip_serializing_if = "Attr::omit")]
    pub shell_override: Attr<bool>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub shell_version: Attr<Text>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub ruby_version: Attr<Text>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub ukiryu_version: Attr<Text>,
    #[serde(default = "z", skip_serializing_if = "Attr::omit")]
    pub cpu_count: Attr<i64>,
    #[serde(default = "z", skip_serializing_if = "Attr::omit")]
    pub total_memory: Attr<i64>,
    #[serde(default = "s", skip_serializing_if = "Attr::omit")]
    pub working_directory: Attr<Text>,
}

impl Default for RunEnvironment {
    fn default() -> Self {
        let s = || Attr::Default(Text::new(""));
        RunEnvironment {
            hostname: s(),
            platform: s(),
            os_version: s(),
            shell: s(),
            shell_override: Attr::Default(false),
            shell_version: s(),
            ruby_version: s(),
            ukiryu_version: s(),
            cpu_count: Attr::Default(0),
            total_memory: Attr::Default(0),
            working_directory: s(),
        }
    }
}

impl Model for RunEnvironment {}
