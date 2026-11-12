//! Ukiryu's shared core. Runtime adapters call this crate rather than
//! reimplementing configuration or maintaining another YAML stack.
pub mod config;
pub mod definition;
pub mod extractors;
pub mod routing;
pub mod shell;
pub mod types;
pub mod version_detection;
pub mod version_ranges;
pub mod wire;
pub use yeptris::{from_slice as from_yaml_slice, from_str as from_yaml, DeserError as YamlError};

#[cfg(feature = "c")]
pub mod ffi;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
