//! Register discovery, cloning and loading (port of `lib/ukiryu/register.rb`).
//!
//! A register is a directory of tool profiles:
//!
//! ```text
//! <register>/tools/<tool>/index.yaml
//! <register>/tools/<tool>/<implementation>/<file>.yaml
//! <register>/interfaces/<interface path>.yaml
//! ```
//!
//! Discovery never reads or mutates process-global state on its own: callers
//! pass a [`DiscoveryContext`] (`UKIRYU_REGISTER`, the development register
//! path and the home directory). [`DiscoveryContext::from_process`] is a
//! read-only convenience for adapters.
//!
//! Typed loads are generic over the caller's model type and go through the
//! shared YAML stack (`yeptris`), so this module defines no parallel models.
//! Deliberate divergences from the Ruby source are listed in
//! `fixtures/register/DIVERGENCES.md`.

use serde::de::DeserializeOwned;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use yeptris::{Number, Value as Yaml};

/// GitHub repository the user register is cloned from.
pub const REMOTE_URL: &str = "https://github.com/ukiryu/register";

/// Default user register location (relative to the home directory).
pub const DEFAULT_USER_PATH: &str = "~/.ukiryu/register";

/// Environment variable naming an explicit register directory.
pub const ENV_VAR: &str = "UKIRYU_REGISTER";

/// How a register was discovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Env,
    Dev,
    User,
    Manual,
    Unknown,
}

impl Source {
    /// The Ruby symbol name (`:env`, `:dev`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Env => "env",
            Source::Dev => "dev",
            Source::User => "user",
            Source::Manual => "manual",
            Source::Unknown => "unknown",
        }
    }
}

/// Register failures. `class_name` reports the Ruby exception class the
/// source raises for the same condition.
#[derive(Debug)]
pub enum Error {
    /// `Register::NotFoundError`.
    NotFound(String),
    /// `Register::CloneError`.
    Clone(String),
    /// `Register::Error` (update failures).
    Update(String),
    /// `ArgumentError` raised by the git layer when the register directory
    /// is not inside a git working tree; the source does not rescue it.
    NotGitWorkTree(String),
    /// Filesystem failure (`Errno::*` in the source).
    Io { path: PathBuf, error: io::Error },
    /// YAML or model conversion failure.
    Yaml { path: PathBuf, message: String },
    /// Index content the source cannot use (`TypeError`/`NoMethodError`).
    InvalidIndex { path: PathBuf, message: String },
}

impl Error {
    pub fn class_name(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "Ukiryu::Register::NotFoundError",
            Error::Clone(_) => "Ukiryu::Register::CloneError",
            Error::Update(_) => "Ukiryu::Register::Error",
            Error::NotGitWorkTree(_) => "ArgumentError",
            Error::Io { .. } => "SystemCallError",
            Error::Yaml { .. } => "Psych::Exception",
            Error::InvalidIndex { .. } => "TypeError",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(m) | Error::Clone(m) | Error::Update(m) | Error::NotGitWorkTree(m) => {
                f.write_str(m)
            }
            Error::Io { path, error } => write!(f, "{error} @ {}", path.display()),
            Error::Yaml { path, message } | Error::InvalidIndex { path, message } => {
                write!(f, "{message} ({})", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn io_error(path: &Path, error: io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        error,
    }
}

/// Explicit inputs for register discovery (no global state is consulted).
#[derive(Clone, Debug, Default)]
pub struct DiscoveryContext {
    /// Value of `UKIRYU_REGISTER`, if set.
    pub env_register: Option<OsString>,
    /// Development register path. The source derives it from its own file
    /// location (`<gem root>/register`); runtimes supply theirs.
    pub dev_path: Option<PathBuf>,
    /// Home directory used to expand [`DEFAULT_USER_PATH`].
    pub home: Option<PathBuf>,
}

impl DiscoveryContext {
    /// Read `UKIRYU_REGISTER` and the home directory from the process
    /// environment (read-only). `HOME` wins; on Windows `USERPROFILE` is the
    /// fallback, as in Ruby's `File.expand_path('~')`.
    pub fn from_process(dev_path: Option<PathBuf>) -> Self {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .or_else(|| {
                if cfg!(windows) {
                    std::env::var_os("USERPROFILE").filter(|h| !h.is_empty())
                } else {
                    None
                }
            })
            .map(PathBuf::from);
        DiscoveryContext {
            env_register: std::env::var_os(ENV_VAR),
            dev_path,
            home,
        }
    }

    /// `File.expand_path(DEFAULT_USER_PATH)`.
    pub fn user_path(&self) -> Option<PathBuf> {
        self.home
            .as_ref()
            .map(|home| expand(&home.join(".ukiryu").join("register")))
    }

    fn env_dir(&self) -> Option<PathBuf> {
        let env = self.env_register.as_ref()?;
        let path = PathBuf::from(env);
        path.is_dir().then_some(path)
    }

    fn dev_existing(&self) -> Option<PathBuf> {
        // Source: `dev_path&.exist?` — any filesystem entry, not validated.
        let dev = self.dev_path.as_ref()?;
        dev.exists().then(|| dev.clone())
    }

    /// Discover the register path without cloning: env, dev, valid user
    /// clone, then the user path as the auto-clone target.
    ///
    /// Returns `None` only when no home directory is known (the source raises
    /// from `File.expand_path` in that case).
    pub fn discover_path(&self) -> Option<(PathBuf, Source)> {
        if let Some(env) = self.env_dir() {
            return Some((env, Source::Env));
        }
        if let Some(dev) = self.dev_existing() {
            return Some((dev, Source::Dev));
        }
        // Steps 3 and 4 of the source both return the user path.
        self.user_path().map(|user| (user, Source::User))
    }

    /// Resolve an existing register without triggering a clone.
    pub fn resolve_path_without_clone(&self) -> Option<PathBuf> {
        if let Some(env) = self.env_dir() {
            return Some(env);
        }
        if let Some(dev) = self.dev_existing() {
            return Some(dev);
        }
        let user = self.user_path()?;
        valid_structure(&user).then_some(user)
    }

    /// `Register.exists?`: a register can be found without cloning and has
    /// valid structure. (The source returns `nil` rather than `false` when no
    /// path resolves.)
    pub fn exists(&self) -> bool {
        self.resolve_path_without_clone()
            .is_some_and(|p| p.is_dir() && valid_structure(&p))
    }

    /// `Register.discover`: resolve the path and ensure it exists, cloning
    /// the user register when needed.
    pub fn discover(&self, git: &Git) -> Result<Register> {
        let (path, source) = self
            .discover_path()
            .ok_or_else(|| Error::NotFound("No register found and auto-clone failed".into()))?;
        let register = Register::new(path, source);
        register.ensure_exists(git)?;
        Ok(register)
    }
}

/// Lexical `File.expand_path` without `~` handling: absolute and with `.`/`..`
/// folded.
fn expand(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Names matched by a single Ruby `*` glob segment: entries not starting
/// with a dot.
fn glob_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (!name.starts_with('.')).then(|| (name, e.path()))
        })
        .collect();
    out.sort();
    out
}

fn has_yaml_suffix(name: &str) -> bool {
    if cfg!(windows) {
        name.to_ascii_lowercase().ends_with(".yaml")
    } else {
        name.ends_with(".yaml")
    }
}

/// Valid register structure: a `tools` directory with at least one
/// `tools/*/*.yaml` entry.
pub fn valid_structure(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let tools = path.join("tools");
    if !tools.is_dir() {
        return false;
    }
    glob_entries(&tools).into_iter().any(|(_, tool)| {
        tool.is_dir()
            && glob_entries(&tool)
                .iter()
                .any(|(name, _)| has_yaml_suffix(name))
    })
}

/// The installed git executable used for clone/update.
#[derive(Clone, Debug)]
pub struct Git {
    pub program: PathBuf,
    pub remote_url: String,
}

impl Default for Git {
    fn default() -> Self {
        Git {
            program: PathBuf::from("git"),
            remote_url: REMOTE_URL.to_owned(),
        }
    }
}

/// Information about a register (`Register#info`).
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub path: PathBuf,
    pub source: Source,
    pub exists: bool,
    pub valid: bool,
    pub tools_count: usize,
    pub git_info: Option<GitInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GitInfo {
    pub branch: Option<String>,
    /// First 8 characters of the HEAD commit.
    pub commit: Option<String>,
    /// HEAD commit time (Unix seconds).
    pub last_update: Option<i64>,
}

impl Git {
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        // Matches the git gem: inherited repository overrides are cleared on
        // the child only.
        for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_SSH"] {
            command.env_remove(var);
        }
        command.stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&std::ffi::OsStr]) -> std::result::Result<String, String> {
        let output = self
            .command()
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!(
                "git {} failed with {}: {}",
                args.iter()
                    .map(|a| a.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    fn clone_into(&self, path: &Path) -> std::result::Result<(), String> {
        self.run(&[
            "clone".as_ref(),
            "--".as_ref(),
            self.remote_url.as_ref(),
            path.as_os_str(),
        ])
        .map(drop)
    }

    fn work_tree(&self, path: &Path) -> bool {
        self.run(&[
            "-C".as_ref(),
            path.as_os_str(),
            "rev-parse".as_ref(),
            "--show-toplevel".as_ref(),
        ])
        .is_ok()
    }

    fn pull(&self, path: &Path) -> std::result::Result<(), String> {
        self.run(&["-C".as_ref(), path.as_os_str(), "pull".as_ref()])
            .map(drop)
    }
}

/// One `list_versions` entry: the version file and its `equals` value with
/// its YAML type preserved.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionFile {
    pub path: PathBuf,
    pub equals: Yaml,
}

/// A register directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Register {
    path: PathBuf,
    source: Source,
}

impl Register {
    pub fn new(path: impl Into<PathBuf>, source: Source) -> Self {
        Register {
            path: path.into(),
            source,
        }
    }

    /// `Register.at(path)`: an expanded path with source `manual`.
    pub fn at(path: impl AsRef<Path>) -> Self {
        Register::new(expand(path.as_ref()), Source::Manual)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn source(&self) -> Source {
        self.source
    }

    /// The register directory exists.
    pub fn exists(&self) -> bool {
        self.path.is_dir()
    }

    /// The register has valid structure.
    pub fn valid(&self) -> bool {
        valid_structure(&self.path)
    }

    /// Clone unless the register already exists with valid structure.
    pub fn ensure_exists(&self, git: &Git) -> Result<()> {
        if self.exists() && self.valid() {
            return Ok(());
        }
        self.clone_register(git)
    }

    /// `Register#clone!`.
    ///
    /// Divergence (safety): the source removes the target path whenever git
    /// fails, including a pre-existing user directory. This port only removes
    /// a directory that did not exist before this call.
    pub fn clone_register(&self, git: &Git) -> Result<()> {
        if self.exists() && self.valid() {
            return Err(Error::Clone(format!(
                "Register already exists at {}",
                self.path.display()
            )));
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() && !parent.is_dir() {
                fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
            }
        }
        let existed_before = self.path.exists();
        if let Err(message) = git.clone_into(&self.path) {
            if !existed_before && self.path.is_dir() {
                let _ = fs::remove_dir_all(&self.path);
            }
            return Err(Error::Clone(self.clone_error_message(git, &message)));
        }
        if !self.valid() {
            return Err(Error::Clone("Register clone validation failed".into()));
        }
        Ok(())
    }

    /// `Register#update!`: clone when missing, otherwise `git pull`.
    pub fn update(&self, git: &Git) -> Result<()> {
        if !self.exists() {
            return self.clone_register(git);
        }
        if !git.work_tree(&self.path) {
            return Err(Error::NotGitWorkTree(format!(
                "'{}' is not in a git working tree",
                self.path.display()
            )));
        }
        git.pull(&self.path)
            .map_err(|m| Error::Update(format!("Failed to update register: {m}")))
    }

    /// Sorted names of `tools/*/index.yaml` directories.
    pub fn tool_names(&self) -> Vec<String> {
        let tools = self.path.join("tools");
        if !tools.is_dir() {
            return Vec::new();
        }
        let mut names: Vec<String> = glob_entries(&tools)
            .into_iter()
            .filter(|(_, dir)| dir.join("index.yaml").exists())
            .map(|(name, _)| name)
            .collect();
        names.sort();
        names
    }

    /// `tools/<name>/index.yaml` exists.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tool_dir(name).join("index.yaml").exists()
    }

    fn tool_dir(&self, name: &str) -> PathBuf {
        self.path.join("tools").join(name)
    }

    /// Read and parse a YAML file. `None` when the file is missing or the
    /// document is empty/`null`/`false` (the source's `return nil unless hash`).
    fn read_tree(&self, file: &Path) -> Result<Option<Yaml>> {
        if !file.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(file).map_err(|e| io_error(file, e))?;
        if content.trim().is_empty() {
            return Ok(None);
        }
        let tree: Yaml = crate::from_yaml(&content).map_err(|e| Error::Yaml {
            path: file.to_path_buf(),
            message: e.to_string(),
        })?;
        Ok(match tree {
            Yaml::Null | Yaml::Bool(false) => None,
            tree => Some(tree),
        })
    }

    fn load_typed<T: DeserializeOwned>(&self, file: &Path) -> Result<Option<T>> {
        let Some(tree) = self.read_tree(file)? else {
            return Ok(None);
        };
        yeptris::from_value(&tree)
            .map(Some)
            .map_err(|e| Error::Yaml {
                path: file.to_path_buf(),
                message: e.to_string(),
            })
    }

    /// Path of `tools/<tool>/index.yaml`.
    pub fn implementation_index_path(&self, tool: &str) -> PathBuf {
        self.tool_dir(tool).join("index.yaml")
    }

    /// Path of `tools/<tool>/<implementation>/<file>`.
    pub fn implementation_version_path(
        &self,
        tool: &str,
        implementation: &str,
        file: &str,
    ) -> PathBuf {
        self.tool_dir(tool).join(implementation).join(file)
    }

    /// Path of `interfaces/<interface_path>.yaml`.
    pub fn interface_path(&self, interface_path: &str) -> PathBuf {
        self.path
            .join("interfaces")
            .join(format!("{interface_path}.yaml"))
    }

    /// Load `tools/<tool>/index.yaml` into the caller's index model.
    pub fn load_implementation_index<T: DeserializeOwned>(&self, tool: &str) -> Result<Option<T>> {
        self.load_typed(&self.implementation_index_path(tool))
    }

    /// Load `tools/<tool>/<implementation>/<file>` into the caller's model.
    pub fn load_implementation_version<T: DeserializeOwned>(
        &self,
        tool: &str,
        implementation: &str,
        file: &str,
    ) -> Result<Option<T>> {
        self.load_typed(&self.implementation_version_path(tool, implementation, file))
    }

    /// Load `interfaces/<interface_path>.yaml` into the caller's model.
    pub fn load_interface<T: DeserializeOwned>(&self, interface_path: &str) -> Result<Option<T>> {
        self.load_typed(&self.interface_path(interface_path))
    }

    /// `Register#list_versions`: every `equals`+`file` version spec of every
    /// implementation, in index order. A repeated file keeps its first
    /// position and takes the last `equals` value (Ruby Hash semantics).
    pub fn list_versions(&self, tool: &str) -> Result<Vec<VersionFile>> {
        let index_path = self.implementation_index_path(tool);
        let Some(index) = self.read_tree(&index_path)? else {
            return Ok(Vec::new());
        };
        let invalid = |message: String| Error::InvalidIndex {
            path: index_path.clone(),
            message,
        };
        let Yaml::Mapping(index) = index else {
            return Err(invalid(format!(
                "undefined method '[]' for {}",
                ruby_class(&index)
            )));
        };
        let implementations = match get(&index, "implementations") {
            None | Some(Yaml::Null) | Some(Yaml::Bool(false)) => return Ok(Vec::new()),
            Some(Yaml::Sequence(items)) => items,
            Some(other) => {
                return Err(invalid(format!(
                    "undefined method 'map' for {}",
                    ruby_class(other)
                )))
            }
        };
        let mut versions: Vec<VersionFile> = Vec::new();
        for implementation in implementations {
            let Yaml::Mapping(implementation) = implementation else {
                return Err(invalid(format!(
                    "undefined method 'transform_keys' for {}",
                    ruby_class(implementation)
                )));
            };
            let name = get(implementation, "name").map_or_else(String::new, ruby_to_s);
            let specs = match get(implementation, "versions") {
                None | Some(Yaml::Null) | Some(Yaml::Bool(false)) => continue,
                Some(Yaml::Sequence(specs)) => specs,
                Some(other) => {
                    return Err(invalid(format!(
                        "undefined method 'each' for {}",
                        ruby_class(other)
                    )))
                }
            };
            for spec in specs {
                let Yaml::Mapping(spec) = spec else {
                    return Err(invalid(format!(
                        "undefined method '[]' for {}",
                        ruby_class(spec)
                    )));
                };
                let (Some(equals), Some(file)) = (
                    get(spec, "equals").filter(|v| truthy(v)),
                    get(spec, "file").filter(|v| truthy(v)),
                ) else {
                    continue;
                };
                let Yaml::String(file) = file else {
                    return Err(invalid(format!(
                        "no implicit conversion of {} into String",
                        ruby_class(file)
                    )));
                };
                let path = self.tool_dir(tool).join(&name).join(file);
                match versions.iter_mut().find(|v| v.path == path) {
                    Some(existing) => existing.equals = equals.clone(),
                    None => versions.push(VersionFile {
                        path,
                        equals: equals.clone(),
                    }),
                }
            }
        }
        Ok(versions)
    }

    /// `Register#load_tool_yaml`: raw YAML text of a tool version file.
    ///
    /// With a version, `tools/<name>/<version>.yaml` wins, then the first
    /// listed file whose basename (minus `.yaml`) equals the version. Without
    /// one, the last listed file.
    pub fn load_tool_yaml(&self, name: &str, version: Option<&str>) -> Result<Option<String>> {
        let read = |file: &Path| fs::read_to_string(file).map_err(|e| io_error(file, e));
        if let Some(version) = version {
            let file = self.tool_dir(name).join(format!("{version}.yaml"));
            if file.exists() {
                return read(&file).map(Some);
            }
        }
        let versions = self.list_versions(name)?;
        let Some(last) = versions.last() else {
            return Ok(None);
        };
        match version {
            Some(version) => versions
                .iter()
                .find(|v| basename_without_yaml(&v.path) == version)
                .map(|v| read(&v.path))
                .transpose(),
            None => read(&last.path).map(Some),
        }
    }

    /// `Register#info`.
    pub fn info(&self, git: &Git) -> Info {
        Info {
            path: self.path.clone(),
            source: self.source,
            exists: self.exists(),
            valid: self.valid(),
            tools_count: self.tool_names().len(),
            git_info: self.git_info(git),
        }
    }

    fn git_info(&self, git: &Git) -> Option<GitInfo> {
        if !self.path.join(".git").is_dir() {
            return None;
        }
        let dir = self.path.as_os_str();
        let branch = git
            .run(&[
                "-C".as_ref(),
                dir,
                "branch".as_ref(),
                "--show-current".as_ref(),
            ])
            .ok()?
            .trim()
            .to_owned();
        let log = git
            .run(&[
                "-C".as_ref(),
                dir,
                "log".as_ref(),
                "-1".as_ref(),
                "--format=%H %ct".as_ref(),
            ])
            .unwrap_or_default();
        let mut parts = log.split_whitespace();
        let commit = parts.next().map(|sha| sha.chars().take(8).collect());
        let last_update = parts.next().and_then(|t| t.parse().ok());
        Some(GitInfo {
            branch: (!branch.is_empty()).then_some(branch),
            commit,
            last_update,
        })
    }

    fn clone_error_message(&self, git: &Git, message: &str) -> String {
        if message.contains("cannot find")
            || message.contains("not found")
            || message.contains("path specified")
        {
            format!(
                "Failed to clone register: {message}\n\n\
                 This error usually means git is not in PATH or the target directory is not accessible.\n\n\
                 To fix this:\n  \
                 1. Verify git is installed and in PATH: git --version\n  \
                 2. On Windows, ensure Git for Windows is installed from https://git-scm.com\n  \
                 3. Or set UKIRYU_REGISTER to use a local register path\n\n\
                 Example (Windows):\n  \
                 set UKIRYU_REGISTER=C:\\path\\to\\register\n\n\
                 Example (Unix):\n  \
                 export UKIRYU_REGISTER=/path/to/register\n"
            )
        } else {
            format!(
                "Failed to clone register from {remote}: {message}\n\n\
                 To fix this:\n  \
                 1. Check your internet connection\n  \
                 2. Verify git is installed: git --version\n  \
                 3. Manually clone: git clone {remote} {path}\n  \
                 4. Or set UKIRYU_REGISTER to use a local register path\n\n\
                 Example:\n  \
                 export UKIRYU_REGISTER=/path/to/register\n",
                remote = git.remote_url,
                path = self.path.display()
            )
        }
    }
}

fn basename_without_yaml(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.strip_suffix(".yaml") {
        Some(stem) if !stem.is_empty() => stem.to_owned(),
        _ => name,
    }
}

/// Symbol-keyed lookup after `symbolize_keys`: any key whose `to_s` matches.
fn get<'a>(mapping: &'a yeptris::Mapping, key: &str) -> Option<&'a Yaml> {
    mapping
        .0
        .iter()
        .rev()
        .find(|(k, _)| ruby_to_s(k) == key)
        .map(|(_, v)| v)
}

fn truthy(value: &Yaml) -> bool {
    !matches!(value, Yaml::Null | Yaml::Bool(false))
}

fn ruby_class(value: &Yaml) -> &'static str {
    match value {
        Yaml::Null => "nil",
        Yaml::Bool(true) => "true",
        Yaml::Bool(false) => "false",
        Yaml::Number(Number::F64(_)) => "Float",
        Yaml::Number(_) => "Integer",
        Yaml::String(_) => "String",
        Yaml::Sequence(_) => "Array",
        Yaml::Mapping(_) => "Hash",
        Yaml::Tagged(_) => "Object",
    }
}

/// Ruby `to_s` for YAML scalars.
fn ruby_to_s(value: &Yaml) -> String {
    match value {
        Yaml::Null => String::new(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(Number::I64(n)) => n.to_string(),
        Yaml::Number(Number::U64(n)) => n.to_string(),
        Yaml::Number(Number::F64(n)) => ruby_float(*n),
        Yaml::String(s) => s.clone(),
        Yaml::Tagged(tagged) => ruby_to_s(&tagged.value),
        Yaml::Sequence(_) | Yaml::Mapping(_) => format!("{value:?}"),
    }
}

/// Ruby `Float#to_s` for ordinary values (`1.0`, `1.5`, `Infinity`, `NaN`).
pub fn ruby_float(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n.fract() == 0.0 && n.abs() < 1e16 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    }
}
