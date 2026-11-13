//! Argument grammar shared by the binary and command handlers.
//!
//! The source contract is ukiryu's pinned cli.rb and its validate, definitions,
//! and cache Thor subcommands. Help wording follows clap; option names, aliases,
//! defaults, argument order, and the explicit `--` boundary remain observable.
pub mod arguments;
pub mod commands;
pub mod stdin;
