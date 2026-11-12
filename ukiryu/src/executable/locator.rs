//! Strategy order of `ExecutableLocator.find_with_info`, the
//! `ExecutableInfo` model and small helpers.

use super::path::{find_in_path, DiscoveryContext};
use super::Platform;

/// Timeout, in seconds, that Ruby gives each internal `command -v`/`which`/
/// `where` call (`ExecutableLocator::INTERNAL_COMMAND_TIMEOUT`). Runners should
/// apply it.
pub const INTERNAL_COMMAND_TIMEOUT_SECS: u64 = 5;

/// How an executable was discovered (`ExecutableInfo#source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutableSource {
    Path,
    Alias,
}

impl ExecutableSource {
    /// The Ruby symbol name (`path` or `alias`).
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutableSource::Path => "path",
            ExecutableSource::Alias => "alias",
        }
    }
}

/// `Models::ExecutableInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableInfo {
    pub path: String,
    pub source: ExecutableSource,
    pub shell: String,
    pub alias_definition: Option<String>,
}

impl ExecutableInfo {
    /// `ExecutableInfo#description`.
    pub fn description(&self) -> String {
        match self.source {
            ExecutableSource::Path => format!("Found in PATH at {}", self.path),
            ExecutableSource::Alias => format!(
                "Shell alias in {}: {}",
                self.shell,
                self.alias_definition.as_deref().unwrap_or("")
            ),
        }
    }

    pub fn is_alias(&self) -> bool {
        self.source == ExecutableSource::Alias
    }

    pub fn is_path(&self) -> bool {
        self.source == ExecutableSource::Path
    }
}

/// Result of `find_with_info`: `{path:, info:}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub path: String,
    pub info: ExecutableInfo,
}

/// What a shell reports for an alias (`detect_alias` → `{target:, definition:}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasInfo {
    pub target: String,
    pub definition: Option<String>,
}

/// Alias detection for the context's shell (Ruby: `Shell.class_for(shell).detect_alias`).
pub trait AliasProbe {
    fn detect_alias(&mut self, command: &str) -> Option<AliasInfo>;
}

/// Runs one internal lookup command (see [`system_command_invocations`]) and
/// returns its stdout when it exits successfully, `None` otherwise (failure,
/// timeout, spawn error). Ruby passes the detected shell to its executor; that
/// choice stays with the runner.
pub trait SystemCommandRunner {
    fn run(&mut self, executable: &str, args: &[String]) -> Option<String>;
}

/// An [`AliasProbe`] that never reports an alias.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAliases;

impl AliasProbe for NoAliases {
    fn detect_alias(&mut self, _command: &str) -> Option<AliasInfo> {
        None
    }
}

/// A [`SystemCommandRunner`] whose every command fails.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSystemCommands;

impl SystemCommandRunner for NoSystemCommands {
    fn run(&mut self, _executable: &str, _args: &[String]) -> Option<String> {
        None
    }
}

/// The commands `SystemCommandExecutor.find` tries, in order. Windows:
/// `where <command>.exe`; elsewhere `sh -c "command -v '<command>' 2>/dev/null"`
/// then `which <command>`. The command name is embedded verbatim, as in Ruby.
pub fn system_command_invocations(platform: Platform, command: &str) -> Vec<(String, Vec<String>)> {
    if platform.is_windows() {
        vec![("where".to_owned(), vec![format!("{command}.exe")])]
    } else {
        vec![
            (
                "sh".to_owned(),
                vec![
                    "-c".to_owned(),
                    format!("command -v '{command}' 2>/dev/null"),
                ],
            ),
            ("which".to_owned(), vec![command.to_owned()]),
        ]
    }
}

/// Characters removed by Ruby `String#strip`.
fn is_ruby_strip(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ')
}

/// Characters matched by Ruby regexp `\s`.
fn is_ruby_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ')
}

/// `SystemCommandExecutor#extract_first_line`: the text before the first
/// newline, stripped; `None` when that is empty.
pub fn first_output_line(stdout: &str) -> Option<String> {
    let first = stdout.split('\n').next().unwrap_or("");
    let line = first.trim_matches(is_ruby_strip);
    (!line.is_empty()).then(|| line.to_owned())
}

/// `ToolFinder.extract_alias_target`: `target.split(/\s+/).first`. Leading
/// whitespace yields `Some("")`; an empty or all-whitespace target yields `None`.
pub fn alias_target_command(target: &str) -> Option<String> {
    if target.chars().all(is_ruby_space) {
        return None;
    }
    let end = target.find(is_ruby_space).unwrap_or(target.len());
    Some(target[..end].to_owned())
}

/// `Tool::ExecutableDiscovery#find_executable` name choice: the selected
/// command profile's `executable_name`, else the profile name. An empty
/// `executable_name` is kept (Ruby `||` only skips `nil`).
pub fn resolve_executable_name<'a>(
    command_profile_executable_name: Option<&'a str>,
    profile_name: &'a str,
) -> &'a str {
    command_profile_executable_name.unwrap_or(profile_name)
}

fn build(
    context: &DiscoveryContext,
    path: String,
    source: ExecutableSource,
    alias_definition: Option<String>,
) -> Discovered {
    Discovered {
        info: ExecutableInfo {
            path: path.clone(),
            source,
            shell: context.shell().to_owned(),
            alias_definition,
        },
        path,
    }
}

fn discover(
    context: &DiscoveryContext,
    command: &str,
    aliases: &mut dyn AliasProbe,
    system: &mut dyn SystemCommandRunner,
) -> Option<Discovered> {
    // AliasDiscovery: the alias name itself first, then its raw target, both
    // through the PATH scan (an absolute target is joined to each directory,
    // as in Ruby, and so is not found directly).
    if let Some(alias) = aliases.detect_alias(command) {
        let path = find_in_path(context, command, &[])
            .or_else(|| find_in_path(context, &alias.target, &[]));
        if let Some(path) = path {
            return Some(build(
                context,
                path,
                ExecutableSource::Alias,
                alias.definition,
            ));
        }
    }

    // SystemCommandDiscovery.
    for (executable, args) in system_command_invocations(context.platform(), command) {
        if let Some(path) = system
            .run(&executable, &args)
            .and_then(|stdout| first_output_line(&stdout))
        {
            return Some(build(context, path, ExecutableSource::Path, None));
        }
    }

    // PathDiscovery.
    find_in_path(context, command, &[])
        .map(|path| build(context, path, ExecutableSource::Path, None))
}

/// `ExecutableLocator.find_with_info`: for `tool_name` then each alias in
/// order, try alias discovery, the system lookup commands, then the PATH scan;
/// the first hit wins. `None` when nothing is found.
pub fn find_with_info(
    context: &DiscoveryContext,
    tool_name: &str,
    aliases: &[String],
    alias_probe: &mut dyn AliasProbe,
    system: &mut dyn SystemCommandRunner,
) -> Option<Discovered> {
    std::iter::once(tool_name)
        .chain(aliases.iter().map(String::as_str))
        .find_map(|name| discover(context, name, alias_probe, system))
}

/// `ExecutableLocator.find`: the path of [`find_with_info`].
pub fn find(
    context: &DiscoveryContext,
    tool_name: &str,
    aliases: &[String],
    alias_probe: &mut dyn AliasProbe,
    system: &mut dyn SystemCommandRunner,
) -> Option<String> {
    find_with_info(context, tool_name, aliases, alias_probe, system).map(|found| found.path)
}
