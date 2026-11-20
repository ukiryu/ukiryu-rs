//! Shared CLI parser, handlers and read-only snapshot renderer.
pub mod arguments;
pub mod commands;
pub mod dispatch;
pub mod snapshot;
pub mod stdin;
pub mod thor;
mod thor_help;
pub mod capture {
    /// Captured UTF-8 output and process status.
    #[derive(Debug, Default, serde::Serialize)]
    pub struct CapturedOutput {
        pub stdout: String,
        pub stderr: String,
        pub exit: i32,
    }
}
