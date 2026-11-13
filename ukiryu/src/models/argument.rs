//! Source: lib/ukiryu/models/argument.rb (`Ukiryu::Models::Argument`).
use super::attr::{de_fns as d, omit_opt, Attr, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

fn default_type() -> Attr<Text> {
    Attr::Set(Text::new("argument"))
}

/// One structured argument of a response request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Argument {
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub value: Option<Text>,
    #[serde(default = "default_type", skip_serializing_if = "Attr::omit")]
    pub r#type: Attr<Text>,
}

impl Default for Argument {
    fn default() -> Self {
        Argument {
            name: None,
            value: None,
            r#type: Attr::Default(Text::new("argument")),
        }
    }
}

impl Argument {
    /// `Argument.new(name:, value:, type:)`.
    pub fn new(name: &str, value: &str, kind: &str) -> Self {
        Argument {
            name: Some(Text::new(name)),
            value: Some(Text::new(value)),
            r#type: Attr::Set(Text::new(kind)),
        }
    }
}

impl Model for Argument {}
