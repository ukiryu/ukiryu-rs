//! Entry point only, the same shape as a shipped extension: all binding
//! logic is in the core's `ffi::ruby` module.
use magnus::{Error, Ruby};

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    ukiryu::ffi::ruby::init(ruby)
}
