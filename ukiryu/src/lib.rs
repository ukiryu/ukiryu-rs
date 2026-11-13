//! Ukiryu's shared core. Runtime adapters call this crate rather than
//! reimplementing configuration or maintaining another YAML stack.
pub mod cache;
pub mod command_builder;
pub mod config;
pub mod definition;
pub mod environment;
pub mod errors;
pub mod executable;
pub mod execution;
pub mod execution_context;
pub mod executor;
pub mod extractors;
pub mod model_loading;
pub mod models;
pub mod options;
pub mod register;
pub mod register_lookup;
pub mod render;
pub mod request;
pub mod routing;
pub mod runtime;
pub mod schema;
pub mod shell;
pub mod tool;
pub mod tool_loader;
pub mod types;
pub mod version_detection;
pub mod version_ranges;
pub mod wire;

#[cfg(feature = "wasm")]
pub mod wasm;
pub use yeptris::{from_slice as from_yaml_slice, from_str as from_yaml, DeserError as YamlError};

#[cfg(any(feature = "c", feature = "ruby"))]
pub mod ffi;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
