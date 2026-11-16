//! Argument grammar shared by the binary and command handlers.
//!
//! The source contract is ukiryu's pinned cli.rb and its validate, definitions,
//! and cache Thor subcommands. clap parses; the root command table and each
//! plain command's help are Thor's text ([`thor_help`]). Option names, aliases,
//! defaults, argument order, and the explicit `--` boundary remain observable.
pub mod arguments;
pub mod commands;
pub mod dispatch;
pub mod stdin;
pub mod thor_help;
