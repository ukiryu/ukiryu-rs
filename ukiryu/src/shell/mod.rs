//! Pure shell formatting. Detection and process execution have separate contracts.
//! Implementations must reproduce the pinned Ruby shell byte behavior.
pub mod base;
pub mod posix;

pub trait ShellFormatting {
    fn name(&self) -> &'static str;
    fn escape(&self, value: &str) -> String;
    fn quote(&self, value: &str) -> String;
    fn needs_quoting(&self, value: &str) -> bool;
    fn format_path(&self, path: &str) -> String;
    fn env_var(&self, name: &str) -> String;
    fn join(&self, executable: &str, arguments: &[String]) -> String;
}
