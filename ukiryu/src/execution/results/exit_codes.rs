//! Profile exit-code mapping. Source: lib/ukiryu/response/base.rb
//! (`Response::Base#exit_code_meaning`, `#format_meaning`).
//!
//! Only the pure selection is here. The Ruby method first looks the tool up
//! (`Tool.find_by(tool_name)`, `profile.compatible_profile`,
//! `command_profile.command(command_name)`) and answers `nil` when
//! `tool_name`/`command_name` is missing or any lookup is nil; that lookup is
//! register/tool integration and stays with its owner. The tables themselves
//! are `Ukiryu::Models::ExitCodes` ([`crate::models::ExitCodes`]), which
//! implements [`ExitCodeTable`].
use crate::models::value::Value;
use crate::models::ExitCodes;

/// A table that maps an exit status to its meaning
/// (`Ukiryu::Models::ExitCodes#meaning`).
pub trait ExitCodeTable {
    fn meaning(&self, code: i64) -> Option<String>;
}

/// `#exit_code_meaning` once the command and profile are resolved: the
/// command's own table when it has one, otherwise the profile's. A command
/// table that lacks the code answers `None`; it does not fall back to the
/// profile table. Unknown codes are `None`, never success.
pub fn exit_code_meaning<T: ExitCodeTable + ?Sized>(
    status: i64,
    command_codes: Option<&T>,
    profile_codes: Option<&T>,
) -> Option<String> {
    command_codes.or(profile_codes)?.meaning(status)
}

/// `#format_meaning`: `": <meaning>"`, or empty without one.
pub fn format_meaning(meaning: Option<&str>) -> String {
    meaning.map_or_else(String::new, |m| format!(": {m}"))
}

/// `ExitCodes#meaning(code)` for an Integer status. A nil or false meaning
/// is `None` (`format_meaning` treats both as absent); any other value reads
/// as its `to_s`.
impl ExitCodeTable for ExitCodes {
    fn meaning(&self, code: i64) -> Option<String> {
        ExitCodes::meaning(self, &Value::int(code))
            .filter(|v| v.truthy())
            .map(Value::to_ruby_s)
    }
}
