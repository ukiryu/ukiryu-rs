//! Parameter type coercion and validation constraints.
//!
//! Port of `Ukiryu::Type` (`lib/ukiryu/type.rb`) and `Ukiryu::Validation`
//! (`lib/ukiryu/validation/{constraints,validator}.rb`) from ukiryu/ukiryu at
//! ab6df353c6dc2d7faf7817a86aafcafa82579896. Values keep their exact Ruby
//! types ([`Value`]); errors carry the Ruby exception class and message
//! ([`Error`]). Filesystem, regular-expression and local-time-zone access go
//! through [`Host`], so the pure data behaviour is separate from the
//! environment. Inputs that need Ruby behaviour this port does not reproduce
//! fail with [`ErrorClass::Unsupported`] instead of a guessed answer.
pub mod constraints;
pub mod datetime;
pub mod error;
mod nonprintable;
pub mod number;
pub mod ruby;
pub mod uri;
pub mod validate;
pub mod validator;
pub mod value;

pub use constraints::Constraint;
pub use error::{Error, ErrorClass, Result};
pub use validate::{valid_type, validate, Host, StdHost, VALID_TYPES};
pub use validator::{build_constraints, OptionsObject, Validator};
pub use value::{OptionSource, Regexp, Time, Value};
