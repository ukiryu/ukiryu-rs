//! Source: lib/ukiryu/models/error_response.rb (`Ukiryu::Models::ErrorResponse`).
use super::attr::{de_fns as d, omit_opt, Attr, Model};
use super::value::Text;
use serde::{Deserialize, Serialize};

fn default_status() -> Attr<Text> {
    Attr::Set(Text::new("error"))
}

fn default_exit_code() -> Attr<i64> {
    Attr::Set(1)
}

/// The structured error response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    #[serde(default = "default_status", skip_serializing_if = "Attr::omit")]
    pub status: Attr<Text>,
    #[serde(default = "default_exit_code", skip_serializing_if = "Attr::omit")]
    pub exit_code: Attr<i64>,
    #[serde(default, deserialize_with = "d::opt", skip_serializing_if = "omit_opt")]
    pub error: Option<Text>,
}

impl Default for ErrorResponse {
    fn default() -> Self {
        ErrorResponse {
            status: Attr::Default(Text::new("error")),
            exit_code: Attr::Default(1),
            error: None,
        }
    }
}

impl ErrorResponse {
    /// `ErrorResponse.from_message(message, exit_code: 1)`.
    pub fn from_message(message: &str, exit_code: i64) -> Self {
        ErrorResponse {
            status: Attr::Set(Text::new("error")),
            exit_code: Attr::Set(exit_code),
            error: Some(Text::new(message)),
        }
    }
}

impl Model for ErrorResponse {}
