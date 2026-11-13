//! [`Renderable`] and [`InspectValue`] for the crate's own models.
//!
//! The formatter reads `SuccessResponse`/`ErrorResponse` exactly as
//! `lib/ukiryu/cli_commands/response_formatter.rb` does: YAML and JSON
//! serialize the whole model (its serde mapping is the lutaml mapping),
//! table and raw read the attributes below through Ruby `#to_s`. The dry
//! run prints request values with Ruby `#inspect` of the unified
//! [`crate::types::Value`].
//!
//! The source dereferences `response.metadata`, `response.command`,
//! `response.request` and `response.output` without a nil check, so a
//! response missing one of them raises `NoMethodError` in Ruby.
//! `SuccessResponse.from_result` always sets all four; a partial model
//! (only reachable by loading a hand-written document) renders the missing
//! parts as empty text here instead of failing.
use super::{InspectValue, Renderable, RequestSummary, ResponseView, SuccessView};
use crate::models::{Argument, Arguments, ErrorResponse, SuccessResponse, Text};
use crate::types::Value;

fn text(t: Option<&Text>) -> String {
    t.map(Text::to_ruby_s).unwrap_or_default()
}

fn argument_value(a: &Argument) -> String {
    text(a.value.as_ref())
}

/// `request.options`/`request.flags`/`request.positional` as
/// `format_request_summary` reads them.
fn request_summary(request: Option<&Arguments>) -> RequestSummary {
    let Some(r) = request else {
        return RequestSummary::default();
    };
    RequestSummary {
        options: r
            .options
            .iter()
            .flatten()
            .map(|a| (text(a.name.as_ref()), argument_value(a)))
            .collect(),
        flags: r.flags.iter().flatten().map(Text::to_ruby_s).collect(),
        positional: r.positional.iter().flatten().map(argument_value).collect(),
    }
}

impl Renderable for SuccessResponse {
    fn view(&self) -> ResponseView {
        let output = self.output.as_deref();
        ResponseView::Success(SuccessView {
            exit_code: self.exit_code,
            formatted_duration: text(
                self.metadata
                    .as_deref()
                    .and_then(|m| m.formatted_duration.as_ref()),
            ),
            request: request_summary(self.request.as_deref()),
            full_command: text(
                self.command
                    .as_deref()
                    .and_then(|c| c.full_command.as_ref()),
            ),
            stdout: text(output.and_then(|o| o.stdout.get())),
            stderr: text(output.and_then(|o| o.stderr.get())),
        })
    }
}

impl Renderable for ErrorResponse {
    fn view(&self) -> ResponseView {
        ResponseView::Error {
            error: text(self.error.as_ref()),
        }
    }
}

/// A request argument value as the CLI holds it (a Symbol, Time, Integer
/// of any size, Float, ordered Hash, ...): the dry run prints its Ruby
/// `#inspect`.
impl InspectValue for Value {
    fn inspect(&self) -> String {
        Value::inspect(self)
    }

    fn as_ruby_string(&self) -> Option<&str> {
        self.as_str()
    }

    fn inspect_string(&self, text: &str) -> String {
        Value::str(text).inspect()
    }
}
