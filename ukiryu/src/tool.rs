//! The typed tool runtime: port of `Ukiryu::Tool` (`lib/ukiryu/tool.rb`) and
//! its instance modules (`lib/ukiryu/tool/command_resolution.rb`,
//! `executable_discovery.rb`, `version_detection.rb`) from ukiryu/ukiryu at
//! ab6df353c6dc2d7faf7817a86aafcafa82579896.
//!
//! A [`Tool`] is built from an already-loaded [`ToolDefinition`] and explicit
//! [`ToolOptions`] (platform, shell, optional version and discovery context).
//! Loading register files and choosing an implementation belong to the
//! register loader; it plugs in through [`DefinitionSource`], which
//! [`get`] and [`find`] use together with a caller-owned [`ToolCache`].
//!
//! Everything else composes existing modules:
//!
//! * profile choice follows `Tool#find_command_profile`;
//! * the executable comes from [`crate::executable::find_with_info`];
//! * arguments and environment come from [`crate::command_builder`];
//! * processes run through [`crate::executor::execute`];
//! * exit-code meanings use [`crate::execution::exit_code_meaning`];
//! * version detection uses [`crate::version_detection::detect_profile_version`];
//! * [`Tool::check_version_compatibility`] answers with the core
//!   [`crate::models::VersionCompatibility`], and [`Tool::probe_flag`] runs
//!   through a [`FlagProbe`] ([`ExecutorProbe`] is the executor).
//!
//! # Intentional differences from the pinned source
//!
//! Each is checked against the oracle capture in
//! `tests/tool_runtime_data/oracle.json`:
//!
//! - With several profiles and none matching, the source raises `NameError`
//!   (`ProfileNotFoundError` is not resolvable from `Tool::CommandResolution`).
//!   Here it is [`ToolError::ProfileNotFound`] with the message the source
//!   meant to build, naming the tool.
//! - A command that needs the executable when none was found fails with
//!   [`ToolError::ExecutableNotFound`]. The source either passes `nil` to the
//!   executor or raises `TypeError` from `File.dirname(nil)`.
//! - `stdin` must be a String parameter; for an IO, build the request with
//!   [`Tool::request`] and attach a reader.
//! - The environment passed to the executor is the caller's base
//!   environment with the command's variables applied over it. The executor
//!   requires an explicit environment, shell and timeout; with an empty base
//!   (the default) this is what the source passes.
//! - [`Tool::exit_code_meaning`] reads the tool's selected profile; the
//!   source's response object looks the tool up again by name first.
mod bundled;
mod command;
mod resolution;
mod version;

pub use bundled::{
    bundled_definition_search_paths, from_bundled, host_home_dir, rooted_search_dir,
};
pub use command::{BuiltCommand, ExecuteSettings};
pub use resolution::{profile_is_modern, ActionResolution};
pub use version::{
    version_info_to_s, CompatibilityMode, ExecutorProbe, FlagProbe, FlagProbeRequest,
    PROBE_FLAG_TIMEOUT_SECS,
};

use std::fmt;
use std::rc::Rc;

use crate::cache::{cache_key_for, RuntimeDefaults, ToolCache, ToolKeyOptions};
use crate::errors::ErrorKind;
use crate::executable::{
    find_with_info, resolve_executable_name, AliasProbe, Discovered, DiscoveryContext,
    ExecutableInfo, NoAliases, NoSystemCommands, Platform, SystemCommandRunner,
};
use crate::executor::ExecuteError;
use crate::models::{CommandDefinition, PlatformProfile, Text, ToolDefinition};
use crate::runtime::{native_platform, Runtime, ShellError, ShellName};
use crate::types;

/// `Tool.new(profile, options)` options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOptions {
    /// `options[:platform]`, used for profile choice and env var platforms.
    pub platform: Platform,
    /// `options[:shell]`, used for profile choice, formatting and execution.
    pub shell: ShellName,
    /// `options[:version]`.
    pub version: Option<String>,
    /// Where executables are searched. `None` reads this process's `PATH`
    /// and `PATHEXT` for the host platform, as the source's `PathScanner`
    /// does whatever platform the tool targets.
    pub discovery: Option<DiscoveryContext>,
}

impl ToolOptions {
    pub fn new(platform: Platform, shell: ShellName) -> ToolOptions {
        ToolOptions {
            platform,
            shell,
            version: None,
            discovery: None,
        }
    }

    /// The runtime's platform and shell (`Runtime.instance` defaults).
    pub fn from_runtime(runtime: &Runtime) -> Result<ToolOptions, ShellError> {
        Ok(ToolOptions::new(runtime.platform(), runtime.shell()?))
    }

    pub fn version(mut self, version: impl Into<String>) -> ToolOptions {
        self.version = Some(version.into());
        self
    }

    pub fn discovery(mut self, context: DiscoveryContext) -> ToolOptions {
        self.discovery = Some(context);
        self
    }

    fn discovery_context(&self) -> DiscoveryContext {
        self.discovery.clone().unwrap_or_else(|| {
            let host = native_platform().unwrap_or(self.platform);
            DiscoveryContext::from_process_env(host, self.shell.as_str())
        })
    }
}

/// Why a tool could not be loaded, built or run.
#[derive(Debug)]
pub enum ToolError {
    /// `ToolNotFoundError`: the definition source has no such tool.
    NotFound { name: String },
    /// `ProfileNotFoundError`.
    ProfileNotFound { message: String },
    /// `ArgumentError`: unknown command or unresolvable action path.
    Argument { message: String },
    /// The command needs the executable and none was found.
    ExecutableNotFound { tool: String },
    /// Parameter validation or argument building failed (`build_args`).
    Build(Box<types::Error>),
    /// The definition holds something the source fails on at run time,
    /// named by the Ruby exception class it raises.
    Ruby {
        class: &'static str,
        message: String,
    },
    /// The definition source failed.
    Source { message: String },
    /// The executor failed (including a non-zero exit status).
    Execute(Box<ExecuteError>),
}

impl ToolError {
    /// The `Ukiryu::Errors` class this error corresponds to, when it has one.
    pub fn kind(&self) -> Option<ErrorKind> {
        match self {
            ToolError::NotFound { .. } => Some(ErrorKind::ToolNotFound),
            ToolError::ProfileNotFound { .. } => Some(ErrorKind::ProfileNotFound),
            ToolError::ExecutableNotFound { .. } => Some(ErrorKind::ExecutableNotFound),
            ToolError::Build(e) if e.class == types::ErrorClass::Validation => {
                Some(ErrorKind::Validation)
            }
            ToolError::Execute(e) => e.kind(),
            _ => None,
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::NotFound { name } => write!(f, "Tool not found: {name}"),
            ToolError::ProfileNotFound { message }
            | ToolError::Argument { message }
            | ToolError::Ruby { message, .. }
            | ToolError::Source { message } => f.write_str(message),
            ToolError::ExecutableNotFound { tool } => {
                write!(f, "Executable for tool '{tool}' not found")
            }
            ToolError::Build(e) => f.write_str(&e.message),
            ToolError::Execute(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ToolError::Execute(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<ExecuteError> for ToolError {
    fn from(e: ExecuteError) -> Self {
        ToolError::Execute(Box::new(e))
    }
}

/// A tool bound to one platform, shell and profile.
#[derive(Debug, Clone)]
pub struct Tool {
    definition: ToolDefinition,
    profile: usize,
    platform: Platform,
    shell: ShellName,
    version: Option<String>,
    executable: Option<Discovered>,
}

/// `Text` as the String a Ruby reader returns (`nil` is `None`).
pub(crate) fn text(t: &Text) -> Option<String> {
    match t {
        Text::Nil => None,
        other => Some(other.to_ruby_s()),
    }
}

impl Tool {
    /// `Tool.new(profile, options)`: choose the command profile, then find
    /// the executable (shell alias and system lookups disabled; see
    /// [`Tool::with_discovery`]).
    pub fn new(definition: ToolDefinition, options: &ToolOptions) -> Result<Tool, ToolError> {
        Tool::with_discovery(definition, options, &mut NoAliases, &mut NoSystemCommands)
    }

    /// `Tool.new` with explicit alias and system-command discovery.
    pub fn with_discovery(
        definition: ToolDefinition,
        options: &ToolOptions,
        aliases: &mut dyn AliasProbe,
        system: &mut dyn SystemCommandRunner,
    ) -> Result<Tool, ToolError> {
        let profile =
            resolution::find_command_profile(&definition, options.platform, options.shell)?;
        let mut tool = Tool {
            definition,
            profile,
            platform: options.platform,
            shell: options.shell,
            version: options.version.clone(),
            executable: None,
        };
        tool.executable = tool.find_executable(&options.discovery_context(), aliases, system);
        Ok(tool)
    }

    /// `ExecutableDiscovery#find_executable`.
    fn find_executable(
        &self,
        context: &DiscoveryContext,
        aliases: &mut dyn AliasProbe,
        system: &mut dyn SystemCommandRunner,
    ) -> Option<Discovered> {
        let exe_name = self
            .command_profile()
            .executable_name
            .as_ref()
            .and_then(text);
        let name = self.name().unwrap_or_default();
        let tool_name = resolve_executable_name(exe_name.as_deref(), &name);
        let alias_names: Vec<String> = self
            .definition
            .aliases
            .iter()
            .flatten()
            .filter_map(text)
            .collect();
        find_with_info(context, tool_name, &alias_names, aliases, system)
    }

    /// `#profile`: the definition the tool was built from.
    pub fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    /// The selected command profile (`@command_profile`).
    pub fn command_profile(&self) -> &PlatformProfile {
        &self
            .definition
            .profiles
            .as_ref()
            .expect("profile chosen at construction")[self.profile]
    }

    /// Index of the selected profile in the definition's `profiles`.
    pub fn profile_index(&self) -> usize {
        self.profile
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn shell(&self) -> ShellName {
        self.shell
    }

    /// `#name`.
    pub fn name(&self) -> Option<String> {
        self.definition.name.as_ref().and_then(text)
    }

    /// `#version` without detection: the `version` option, else the
    /// definition's version. The source falls back to
    /// [`Tool::detect_version`] when both are missing.
    pub fn version(&self) -> Option<String> {
        self.version
            .clone()
            .or_else(|| self.definition.version.as_ref().and_then(text))
    }

    /// `#executable`.
    pub fn executable(&self) -> Option<&str> {
        self.executable.as_ref().map(|d| d.path.as_str())
    }

    /// `#executable_info`.
    pub fn executable_info(&self) -> Option<&ExecutableInfo> {
        self.executable.as_ref().map(|d| &d.info)
    }

    /// `#available?`.
    pub fn is_available(&self) -> bool {
        self.executable.is_some()
    }

    /// `#unavailability_reason`.
    pub fn unavailability_reason(&self) -> Option<String> {
        if self.is_available() {
            return None;
        }
        Some(format!(
            "Tool '{}' not found in PATH. Please install the tool and ensure it's in your PATH.",
            self.name().unwrap_or_default()
        ))
    }

    /// `#alias?`.
    pub fn is_alias(&self) -> bool {
        self.executable_info().is_some_and(ExecutableInfo::is_alias)
    }

    /// `#path_found?`.
    pub fn is_path_found(&self) -> bool {
        self.executable_info().is_some_and(ExecutableInfo::is_path)
    }

    /// `#discovery_description`.
    pub fn discovery_description(&self) -> Option<String> {
        self.executable_info().map(ExecutableInfo::description)
    }

    /// `#commands`.
    pub fn commands(&self) -> &[CommandDefinition] {
        self.command_profile()
            .commands
            .as_deref()
            .unwrap_or_default()
    }

    /// `#command_definition(name)`.
    pub fn command_definition(&self, name: &str) -> Option<&CommandDefinition> {
        self.command_profile().command(name)
    }

    /// `#command?(name)`.
    pub fn has_command(&self, name: &str) -> bool {
        self.command_definition(name).is_some()
    }

    /// `#exit_code_meaning` for a finished command: the command's own exit
    /// code table, else the profile's.
    pub fn exit_code_meaning(&self, command_name: &str, status: i64) -> Option<String> {
        let profile = self.command_profile();
        let command = profile
            .command(command_name)
            .and_then(|c| c.exit_codes.as_deref());
        crate::execution::exit_code_meaning(status, command, profile.exit_codes.as_deref())
    }
}

/// Where [`get`] takes tool definitions from: the register loader's
/// `load_with_implementation_index` up to (not including) `Tool.new`.
/// `Ok(None)` means the tool does not exist (`ToolNotFoundError`).
pub trait DefinitionSource {
    fn load_definition(
        &self,
        name: &str,
        options: &ToolOptions,
    ) -> Result<Option<ToolDefinition>, ToolError>;
}

impl<F> DefinitionSource for F
where
    F: Fn(&str, &ToolOptions) -> Result<Option<ToolDefinition>, ToolError>,
{
    fn load_definition(
        &self,
        name: &str,
        options: &ToolOptions,
    ) -> Result<Option<ToolDefinition>, ToolError> {
        self(name, options)
    }
}

/// `ToolCache.cache_key_for(name, options)` for these options.
pub fn cache_key(name: &str, options: &ToolOptions) -> String {
    let key = ToolKeyOptions {
        platform: Some(options.platform.as_str()),
        shell: Some(options.shell.as_str()),
        version: options.version.as_deref(),
    };
    let runtime = RuntimeDefaults {
        platform: options.platform.as_str(),
        shell: options.shell.as_str(),
    };
    cache_key_for(name, &key, &runtime)
}

/// `Tool.get(name, options)`: the cached tool, or a new one built from the
/// source's definition and cached.
pub fn get(
    source: &dyn DefinitionSource,
    cache: &ToolCache<Rc<Tool>>,
    name: &str,
    options: &ToolOptions,
) -> Result<Rc<Tool>, ToolError> {
    let key = cache_key(name, options);
    if let Some(tool) = cache.get(&key) {
        return Ok(tool);
    }
    let definition = source
        .load_definition(name, options)?
        .ok_or_else(|| ToolError::NotFound {
            name: name.to_owned(),
        })?;
    let tool = Rc::new(Tool::new(definition, options)?);
    cache.set(key, Rc::clone(&tool));
    Ok(tool)
}

/// `Tool.find(name, options)`: [`get`], with a missing tool as `None`.
pub fn find(
    source: &dyn DefinitionSource,
    cache: &ToolCache<Rc<Tool>>,
    name: &str,
    options: &ToolOptions,
) -> Result<Option<Rc<Tool>>, ToolError> {
    match get(source, cache, name, options) {
        Ok(tool) => Ok(Some(tool)),
        Err(ToolError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}
