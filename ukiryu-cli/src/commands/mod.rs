//! CLI command handlers. Each handler is a plain function over typed inputs that returns a
//! [`CommandOutcome`]; root wires parsing and dispatch in `main.rs` / `arguments.rs`.

/// What a handler asks the binary to do: ordered output writes (from `ukiryu::render`) and an exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub writes: Vec<ukiryu::render::OutputWrite>,
    pub exit_code: i32,
}

pub mod inspect;
