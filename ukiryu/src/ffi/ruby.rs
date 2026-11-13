//! Ruby binding (magnus), compiled with the `ruby` feature.
//!
//! All binding logic lives here, in the core; a Ruby extension crate is only
//! an entry point whose `#[magnus::init]` calls [`init`]. Every method below
//! hands Ruby data to an existing core API and converts the core result back;
//! no domain rule, parser or serializer is reimplemented:
//!
//! - models are native objects wrapping the core serde models, so `to_yaml`,
//!   `to_json`, `to_hash`, `from_yaml` and `from_hash` run through the core
//!   serde + Yeptris path ([`crate::models::Model`]);
//! - Ruby values cross the boundary as the core's single dynamic value type,
//!   [`crate::types::Value`], keeping Hash order, Symbols, Integers of any
//!   size, Time and Regexp; anything that type cannot hold exactly raises
//!   instead of being coerced ([`values`]);
//! - configuration uses [`crate::config`]; the operations shared with the C
//!   ABI dispatch the same [`crate::wire`] implementation in process;
//! - `Ukiryu::Errors::*` classes are defined from [`crate::errors::ErrorKind`].
//!
//! The Ruby namespace used here (`Ukiryu::Native`) is a development
//! placeholder: the shipped gem's module layout and public exports are
//! owner decisions (issue #24), not settled by this binding.
use magnus::{function, prelude::*, Error, RString, Ruby, Value as RValue};

mod config;
mod errors;
mod models;
mod values;
mod wire;

pub use values::{from_ruby, to_ruby};

// CRuby exports this so an extension can declare that the methods it defines
// afterwards may be called from non-main Ractors. rb-sys does not bind the
// helper, so it is declared here and resolves against libruby at load time.
extern "C" {
    fn rb_ext_ractor_safe(flag: bool);
}

/// Defines the binding's classes and methods. Called once from the
/// extension's `Init_` entry point.
///
/// The methods are Ractor-safe: they keep no Ruby objects in Rust state and
/// share no mutable data; each call converts its arguments, calls the core
/// and builds fresh Ruby objects in the calling Ractor.
pub fn init(ruby: &Ruby) -> Result<(), Error> {
    // SAFETY: plain C call made during extension initialisation, before any
    // method is defined; it only sets a flag read by later rb_define_* calls.
    unsafe { rb_ext_ractor_safe(true) };
    let ukiryu = ruby.define_module("Ukiryu")?;
    errors::define(ruby, ukiryu)?;
    let native = ukiryu.define_module("Native")?;
    errors::define_native(ruby, native)?;
    wire::define(ruby, native)?;
    config::define(ruby, native)?;
    models::define(ruby, native)?;
    native.define_module_function("core_value", function!(core_value, 1))?;
    native.define_module_function("register_hash", function!(register_hash, 1))?;
    Ok(())
}

/// `Ukiryu::Native.core_value(object)`: the object as the core holds it,
/// converted to [`crate::types::Value`] and back to fresh Ruby objects. It
/// raises where the conversion would not be exact (see [`values`]).
fn core_value(ruby: &Ruby, object: RValue) -> Result<RValue, Error> {
    to_ruby(ruby, &from_ruby(ruby, object)?)
}

/// `Ukiryu::Native.register_hash(yaml)`: the register's YAML load path
/// ([`crate::models::register_hash`]): Yeptris read, Psych tag application
/// and `Utils.symbolize_keys`; nil for an empty or null document.
fn register_hash(ruby: &Ruby, yaml: RString) -> Result<RValue, Error> {
    let yaml = values::text(ruby, yaml)?;
    match crate::models::register_hash(&yaml) {
        Ok(Some(value)) => to_ruby(ruby, &value),
        Ok(None) => Ok(ruby.qnil().as_value()),
        Err(error) => Err(errors::model_error(ruby, error)),
    }
}
