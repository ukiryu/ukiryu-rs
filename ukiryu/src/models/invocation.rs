//! Source: lib/ukiryu/models/invocation.rb (`Ukiryu::Models::Invocation`).
use super::attr::{de_fns as d, omit_opt, Attr, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

fn default_false() -> Attr<bool> {
    Attr::Set(false)
}

/// How a tool is invoked (schema v2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invocation {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub r#type: Option<Text>,
    #[serde(default = "default_false", skip_serializing_if = "Attr::omit")]
    pub multi_call: Attr<bool>,
    #[serde(default = "default_false", skip_serializing_if = "Attr::omit")]
    pub symlink_detection: Attr<bool>,
}

impl Default for Invocation {
    fn default() -> Self {
        Invocation {
            r#type: None,
            multi_call: Attr::Default(false),
            symlink_detection: Attr::Default(false),
        }
    }
}

impl Invocation {
    /// `#direct?`
    pub fn is_direct(&self) -> bool {
        self.r#type.as_ref().is_some_and(|t| t.eq_str("direct"))
    }

    /// `#subcommand?`
    pub fn is_subcommand(&self) -> bool {
        self.r#type.as_ref().is_some_and(|t| t.eq_str("subcommand"))
    }

    /// `#multi_call?`
    pub fn is_multi_call(&self) -> bool {
        self.multi_call.get() == Some(&true)
    }
}

impl Model for Invocation {}
