//! The `--dry-run` request summary.
//!
//! Two variants exist at the pinned source: `exec` uses the
//! `RunCommand#say_dry_run` override (label `Ukiryu::Tool:`, `:stdin`
//! preview), `run-file` uses `ResponseFormatter#say_dry_run` (label
//! `Tool:`, every value inspected as is).
use super::{say_with, SayColor};

/// Which `say_dry_run` the command uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DryRunStyle {
    /// `RunCommand#say_dry_run` (`exec`).
    Exec,
    /// `ResponseFormatter#say_dry_run` (`run-file`).
    RunFile,
}

/// A request argument value as the dry run prints it: Ruby `#inspect` of
/// the value the CLI parsed, implemented by the caller's value model.
pub trait InspectValue {
    /// Ruby `#inspect` of this value.
    fn inspect(&self) -> String;
    /// The text when the value is a Ruby String.
    fn as_ruby_string(&self) -> Option<&str>;
    /// Ruby `String#inspect` of `text`.
    fn inspect_string(&self, text: &str) -> String;
}

/// `say_dry_run(request)` stdout. `arguments` are the request's argument
/// pairs in Hash order; for [`DryRunStyle::Exec`] the keys are the Symbol
/// names and the `stdin` key gets the source's preview (first 101
/// characters plus `...` when longer than 100; `[IO Stream]` for a
/// non-String). `use_color` is `Config#use_color`.
pub fn render_dry_run<V: InspectValue>(
    style: DryRunStyle,
    tool: &str,
    command: &str,
    arguments: &[(String, V)],
    use_color: Option<bool>,
) -> String {
    let off = use_color == Some(false);
    let tool_label = match style {
        DryRunStyle::Exec => "Ukiryu::Tool",
        DryRunStyle::RunFile => "Tool",
    };
    let mut out = String::new();
    out.push_str(&say_with(
        "DRY RUN - Ukiryu Structured Execution Request:",
        Some(SayColor::Yellow),
        off,
    ));
    out.push_str(&say_with("", Some(SayColor::Other), off));
    out.push_str(&say_with(
        &format!("{tool_label}: {tool}"),
        Some(SayColor::Cyan),
        off,
    ));
    out.push_str(&say_with(
        &format!("Command: {command}"),
        Some(SayColor::Cyan),
        off,
    ));
    out.push_str(&say_with("Arguments:", Some(SayColor::Cyan), off));
    for (key, value) in arguments {
        let shown = if style == DryRunStyle::Exec && key == "stdin" {
            match value.as_ruby_string() {
                Some(s) => {
                    let mut preview: String = s.chars().take(101).collect();
                    if s.chars().count() > 100 {
                        preview.push_str("...");
                    }
                    value.inspect_string(&preview)
                }
                None => value.inspect_string("[IO Stream]"),
            }
        } else {
            value.inspect()
        };
        out.push_str(&say_with(
            &format!("  {key}: {shown}"),
            Some(SayColor::White),
            off,
        ));
    }
    out
}
