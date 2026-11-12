//! Implementation routing: which tool, implementation and version file a
//! request resolves to.
//!
//! Port of the pure selection logic in the pinned Ruby source
//! (`ukiryu/ukiryu` at `ab6df353`):
//!
//! - [`ToolIndex`], the port of `Ukiryu::ToolIndex` (lib/ukiryu/tool_index.rb):
//!   interface and alias index over a register's `tools/<tool>/<impl>/*.yaml`
//!   files, its "primary file" choice (`load_yaml_for_tool` and
//!   `select_latest_version_by_content`) and its alias platform/shell
//!   compatibility check.
//! - [`interface_lookup_keys`], the versioned-interface fallback keys of
//!   `Register.load_tool_metadata` (lib/ukiryu/register.rb).
//! - [`detect_implementation_and_version`] and
//!   [`find_implementation_for_platform`], the implementation and version-file
//!   selection of `Ukiryu::Tool::Loader` (lib/ukiryu/tool/loader.rb), with
//!   `build_implementation_spec`, `build_default_spec` and
//!   `fallback_to_default`.
//! - [`compatible_profile`], the port of `Models::ImplementationVersion#compatible_profile`
//!   (exact platform and shell membership plus hash-level `inherits`).
//! - [`build_tool_name`], the implementation-suffixed tool name.
//!
//! Nothing here touches the filesystem, starts a process or detects the
//! platform or shell. The register loader supplies the scanned files and
//! parsed documents, and the collaborators that run detection commands,
//! resolve version schemes, match version ranges and load version files are
//! reached through [`SelectionEnv`]. The shell is always an explicit input:
//! defaulting it (`Shell.detect`) is the owner-pending D2 decision and is not
//! made here.
//!
//! Source faults stay faults. A Ruby exception the selection code itself
//! raises (for example `NoMethodError` on an implementation without
//! `versions`) is returned as [`SourceFault`] carrying the Ruby class name; an
//! error raised by a collaborator (for example Versionian's calver ordering
//! error) is returned unchanged as [`RoutingError::Env`].
mod profile;
mod selection;
mod tool_index;
mod version_order;

pub use profile::{compatible_profile, ProfileMatch, ProfileView};
pub use selection::{
    build_default_spec, build_tool_name, detect_implementation_and_version,
    find_implementation_for_platform, Implementation, ImplementationSpec, LoadedVersion, Probe,
    SelectionEnv, SelectionRequest, VersionEntry,
};
pub use tool_index::{
    interface_lookup_keys, FileRef, IndexWarning, RegisterFile, RegisterImplementation,
    RegisterTool, ToolIndex,
};

use std::fmt;

/// A Ruby exception raised by the ported selection code itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFault {
    /// The Ruby exception class the pinned source raises, e.g. `NoMethodError`.
    pub class: &'static str,
    pub message: String,
}

impl SourceFault {
    pub(crate) fn new(class: &'static str, message: impl Into<String>) -> Self {
        SourceFault {
            class,
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}

impl std::error::Error for SourceFault {}

/// Failure of a selection run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutingError<E> {
    /// The selection code raised (see [`SourceFault`]).
    Source(SourceFault),
    /// A collaborator reached through [`SelectionEnv`] failed; propagated as is.
    Env(E),
}

impl<E> From<SourceFault> for RoutingError<E> {
    fn from(fault: SourceFault) -> Self {
        RoutingError::Source(fault)
    }
}

impl<E: fmt::Display> fmt::Display for RoutingError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoutingError::Source(fault) => fault.fmt(f),
            RoutingError::Env(error) => error.fmt(f),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for RoutingError<E> {}
