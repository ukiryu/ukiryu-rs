//! `Ukiryu::OptionsBuilder::Validator`: the `validate!` method of an options
//! class.
//!
//! Source: `lib/ukiryu/options_builder/validator.rb` (ukiryu/ukiryu ab6df353).
//! Every argument is required unless its `min` is zero or negative; `required`
//! is not consulted. Only `nil` and an empty array count as missing.

use super::{ArgumentDef, CommandDef, OptionValue, OptionsError};

/// `validate!` over the argument slots read through `slot(name)`.
pub fn validate<C: CommandDef + ?Sized, V: OptionValue>(
    def: &C,
    slot: impl Fn(&str) -> V,
) -> Result<(), OptionsError> {
    let mut errors = Vec::new();
    for a in def.arguments() {
        let name = a.name().unwrap_or_default();
        let min = a.min().unwrap_or(1);
        let value = slot(name);
        let missing = value.is_nil() || value.as_array().is_some_and(<[V]>::is_empty);
        if min > 0 && missing {
            errors.push(format!("Missing required argument: {name}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(OptionsError::validation(errors.join(", ")))
    }
}

/// The message of the (unused by the pinned code) nested
/// `Validator::ValidationError`.
pub fn validation_failed_message(messages: &[&str]) -> String {
    format!("Validation failed: {}", messages.join(", "))
}
