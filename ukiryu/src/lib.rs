//! Ukiryu's shared core. Runtime adapters call this crate rather than
//! reimplementing configuration or maintaining another YAML stack.
pub mod config;
pub mod executable;
pub mod shell;
pub mod wire;
pub use yeptris::{from_slice as from_yaml_slice, from_str as from_yaml, DeserError as YamlError};

#[cfg(feature = "c")]
pub mod ffi;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
