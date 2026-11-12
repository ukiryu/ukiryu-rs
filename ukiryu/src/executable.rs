//! Executable discovery.
//!
//! Port of the pinned Ruby implementation (`ukiryu` at `ab6df353`):
//! `lib/ukiryu/executable_locator.rb`, `lib/ukiryu/platform.rb`
//! (`executable_search_paths`), `lib/ukiryu/models/executable_info.rb`,
//! `lib/ukiryu/tool/executable_discovery.rb` and the alias-target helper of
//! `lib/ukiryu/tool_finder.rb`.
//!
//! Everything the Ruby code reads from process globals is explicit here:
//! the platform, the shell name, `PATH` and `PATHEXT` live in a
//! [`DiscoveryContext`], and the two strategies that need a running shell or
//! a child process (alias detection and `command -v`/`which`/`where`) are
//! supplied by the caller through [`AliasProbe`] and [`SystemCommandRunner`].
//! This module does not detect the shell and does not spawn processes.
//!
//! [`probe_search_paths`] covers profile `search_paths` (Windows Program Files
//! layouts, macOS app bundles). The pinned Ruby code never reads that profile
//! field, so it is a separate explicit call and is not part of
//! [`find_with_info`]'s strategy order.

mod locator;
mod path;
mod search_paths;

pub use locator::{
    alias_target_command, find, find_with_info, first_output_line, resolve_executable_name,
    system_command_invocations, AliasInfo, AliasProbe, Discovered, ExecutableInfo,
    ExecutableSource, NoAliases, NoSystemCommands, SystemCommandRunner,
    INTERNAL_COMMAND_TIMEOUT_SECS,
};
pub use path::{
    candidate_paths, find_in_path, is_executable, join_path, path_extensions, split_search_path,
    DiscoveryContext,
};
pub use search_paths::{probe_search_paths, search_path_candidates, SearchPathError};

/// Platform families known to the Ruby implementation (`:linux`, `:macos`,
/// `:windows`). Names match exactly; there is no fuzzy mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    /// The Ruby symbol name of the platform.
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Macos => "macos",
            Platform::Windows => "windows",
        }
    }

    /// Exact, case-sensitive lookup of a Ruby platform name.
    pub fn from_name(name: &str) -> Option<Platform> {
        match name {
            "linux" => Some(Platform::Linux),
            "macos" => Some(Platform::Macos),
            "windows" => Some(Platform::Windows),
            _ => None,
        }
    }

    /// Whether this is the Windows family.
    pub fn is_windows(self) -> bool {
        self == Platform::Windows
    }
}
