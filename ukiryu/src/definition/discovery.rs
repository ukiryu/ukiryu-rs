//! XDG and tool-bundled definition discovery.
//!
//! Source: `lib/ukiryu/definition/discovery.rb`
//! (`Ukiryu::Definition::Discovery`). The Ruby class reads the process
//! environment and globs `/opt/*/ukiryu`; here those inputs are captured in a
//! [`DiscoveryEnv`] so callers (and tests) choose them explicitly.
//!
//! Differences that cannot be mirrored: Ruby keys discovered definitions by
//! the YAML `name` value (Integer/Symbol names stay distinct from strings);
//! here non-string scalars are rendered with Ruby `to_s`. Directory entries
//! are visited in byte order where Ruby uses `readdir` order, which only
//! matters for metadata that compare equal (same priority, version and name).
use super::metadata::{DefinitionMetadata, SourceKind};
use super::ruby_split;
use super::sources::file::expand_path;
use std::path::Path;
use yeptris::{Number, Value};

/// Inputs Ruby reads from `ENV` and the filesystem root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryEnv {
    /// `HOME`, used to expand `~/.local/share`.
    pub home: Option<String>,
    /// `XDG_DATA_HOME`.
    pub xdg_data_home: Option<String>,
    /// `XDG_DATA_DIRS`.
    pub xdg_data_dirs: Option<String>,
    /// `PATH` (split on `:` as in Ruby).
    pub path: Option<String>,
    /// Directory whose `*/ukiryu` children are bundled locations (`/opt`).
    pub opt_root: String,
}

impl DiscoveryEnv {
    /// Values from the current process environment, with `opt_root = "/opt"`.
    pub fn from_process() -> Self {
        DiscoveryEnv {
            home: std::env::var("HOME").ok(),
            xdg_data_home: std::env::var("XDG_DATA_HOME").ok(),
            xdg_data_dirs: std::env::var("XDG_DATA_DIRS").ok(),
            path: std::env::var("PATH").ok(),
            opt_root: "/opt".to_string(),
        }
    }
}

/// `Ukiryu::Definition::Discovery` bound to one environment.
#[derive(Clone, Debug)]
pub struct Discovery {
    pub env: DiscoveryEnv,
}

/// Ruby `File.join` for string segments.
pub(crate) fn ruby_file_join(parts: &[&str]) -> String {
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            out.push_str(part);
            continue;
        }
        let has_trailing = out.ends_with('/');
        let has_leading = part.starts_with('/');
        match (has_trailing, has_leading) {
            (true, true) => out.push_str(&part[1..]),
            (false, false) => {
                out.push('/');
                out.push_str(part);
            }
            _ => out.push_str(part),
        }
    }
    out
}

/// Ruby `File.dirname`.
fn ruby_dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() {
            ".".into()
        } else {
            "/".into()
        };
    }
    match trimmed.rfind('/') {
        None => ".".into(),
        Some(i) => {
            let parent = trimmed[..i].trim_end_matches('/');
            if parent.is_empty() {
                "/".into()
            } else {
                parent.into()
            }
        }
    }
}

fn is_dir(p: &str) -> bool {
    !p.is_empty() && Path::new(p).is_dir()
}

/// Sorted names of entries in `dir` (`Dir.glob` sorts; `*` skips dotfiles).
fn sorted_entries(dir: &str) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names
}

/// `Dir.glob(File.join(dir, pattern))` for `*<suffix>` / `*-*<suffix>` patterns.
fn glob(dir: &str, need_dash: bool, suffix: &str) -> Vec<String> {
    sorted_entries(dir)
        .into_iter()
        .filter(|n| !n.starts_with('.') && n.ends_with(suffix))
        .filter(|n| !need_dash || n[..n.len() - suffix.len()].contains('-'))
        .map(|n| ruby_file_join(&[dir, &n]))
        .collect()
}

fn uniq(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

/// Ruby `Float#to_s` for finite values.
fn ruby_float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let abs = f.abs();
    if abs != 0.0 && !(1e-4..1e16).contains(&abs) {
        let s = format!("{f:e}");
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!(
            "{mantissa}e{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        );
    }
    let s = format!("{f}");
    if s.contains('.') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Ruby `to_s` of a YAML scalar used as name/version; `None` for nil/false
/// and for collections (not representable as names here).
fn scalar_to_s(v: &Value) -> Option<String> {
    match v {
        Value::Null | Value::Bool(false) => None,
        Value::Bool(true) => Some("true".into()),
        Value::String(s) => Some(s.clone()),
        Value::Number(Number::I64(i)) => Some(i.to_string()),
        Value::Number(Number::U64(u)) => Some(u.to_string()),
        Value::Number(Number::F64(f)) => Some(ruby_float_to_s(*f)),
        Value::Tagged(t) => scalar_to_s(&t.value),
        Value::Sequence(_) | Value::Mapping(_) => None,
    }
}

impl Discovery {
    pub fn new(env: DiscoveryEnv) -> Self {
        Discovery { env }
    }

    pub fn from_process() -> Self {
        Self::new(DiscoveryEnv::from_process())
    }

    /// `xdg_data_home`: `XDG_DATA_HOME` (even when empty) or `~/.local/share`.
    pub fn xdg_data_home(&self) -> String {
        match &self.env.xdg_data_home {
            Some(v) => v.clone(),
            None => format!("{}/.local/share", self.env.home.clone().unwrap_or_default()),
        }
    }

    /// `xdg_data_dirs`: `XDG_DATA_DIRS` split on `:`, stripped, empties
    /// dropped; `/usr/local/share`, `/usr/share` when unset.
    pub fn xdg_data_dirs(&self) -> Vec<String> {
        match &self.env.xdg_data_dirs {
            Some(v) => ruby_split(v, ':')
                .into_iter()
                .map(|s| super::ruby_strip(s).to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            None => vec!["/usr/local/share".into(), "/usr/share".into()],
        }
    }

    pub fn user_definitions_directory(&self) -> String {
        ruby_file_join(&[&self.xdg_data_home(), "ukiryu", "definitions"])
    }

    /// `search_paths`: user, tool-bundled, `XDG_DATA_DIRS` entries, then the
    /// user directory again (removed by `uniq`).
    pub fn search_paths(&self) -> Vec<String> {
        let mut paths = vec![self.user_definitions_directory()];
        paths.extend(self.tool_bundled_paths());
        for dir in self.xdg_data_dirs() {
            paths.push(ruby_file_join(&[&dir, "ukiryu", "definitions"]));
        }
        paths.push(ruby_file_join(&[
            &self.xdg_data_home(),
            "ukiryu",
            "definitions",
        ]));
        uniq(paths)
    }

    /// `tool_bundled_paths`: `<bin>/../ukiryu`, `<bin>/../share/ukiryu` for
    /// each existing `PATH` directory, then `<opt_root>/*/ukiryu`.
    pub fn tool_bundled_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for bin_dir in ruby_split(self.env.path.as_deref().unwrap_or(""), ':') {
            if !is_dir(bin_dir) {
                continue;
            }
            let parent = ruby_dirname(bin_dir);
            let ukiryu_dir = ruby_file_join(&[&parent, "ukiryu"]);
            if is_dir(&ukiryu_dir) {
                paths.push(ukiryu_dir);
            }
            let share_dir = ruby_file_join(&[&parent, "share", "ukiryu"]);
            if is_dir(&share_dir) {
                paths.push(share_dir);
            }
        }
        for name in sorted_entries(&self.env.opt_root) {
            if name.starts_with('.') {
                continue;
            }
            let candidate = ruby_file_join(&[&self.env.opt_root, &name, "ukiryu"]);
            if is_dir(&candidate) {
                paths.push(candidate);
            }
        }
        uniq(paths)
    }

    /// `discover`: tool name -> metadata sorted by priority, version
    /// (descending) and name. Keys keep first-discovery order.
    pub fn discover(&self) -> Vec<(String, Vec<DefinitionMetadata>)> {
        let mut found: Vec<(String, Vec<DefinitionMetadata>)> = Vec::new();
        for search_path in self.search_paths() {
            if !is_dir(&search_path) {
                continue;
            }
            for md in self.discover_in_path(&search_path) {
                match found.iter_mut().find(|(k, _)| *k == md.name) {
                    Some((_, list)) => list.push(md),
                    None => found.push((md.name.clone(), vec![md])),
                }
            }
        }
        for (_, list) in found.iter_mut() {
            list.sort_by(|a, b| a.compare(b));
        }
        found
    }

    /// `find(tool_name, version = nil)`: exact version match, or the
    /// highest-priority definition.
    pub fn find(&self, tool_name: &str, version: Option<&str>) -> Option<DefinitionMetadata> {
        let list = self.definitions_for(tool_name);
        match version {
            Some(v) => list.into_iter().find(|d| d.version == v),
            None => list.into_iter().next(),
        }
    }

    /// `available_tools`: sorted tool names.
    pub fn available_tools(&self) -> Vec<String> {
        let mut names: Vec<String> = self.discover().into_iter().map(|(k, _)| k).collect();
        names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        names
    }

    /// `definitions_for(tool_name)`.
    pub fn definitions_for(&self, tool_name: &str) -> Vec<DefinitionMetadata> {
        self.discover()
            .into_iter()
            .find(|(k, _)| k == tool_name)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }

    /// `discover_in_path(search_path)`: `{tool}/*.yaml` files in non-dot
    /// subdirectories, then flat `*-*.yaml` files.
    pub fn discover_in_path(&self, search_path: &str) -> Vec<DefinitionMetadata> {
        let source_type = self.determine_source_type(search_path);
        let mut out = Vec::new();
        if is_dir(search_path) {
            for tool_name in sorted_entries(search_path) {
                if tool_name.starts_with('.') {
                    continue;
                }
                let tool_dir = ruby_file_join(&[search_path, &tool_name]);
                if !is_dir(&tool_dir) {
                    continue;
                }
                for yaml_file in glob(&tool_dir, false, ".yaml") {
                    out.extend(Self::metadata_from_file(&yaml_file, source_type.clone()));
                }
            }
        }
        for yaml_file in glob(search_path, true, ".yaml") {
            out.extend(Self::metadata_from_file(&yaml_file, source_type.clone()));
        }
        out
    }

    /// `determine_source_type(path)`: substring tests on the expanded path.
    pub fn determine_source_type(&self, path: &str) -> SourceKind {
        let expanded = expand_path(Path::new(path));
        if expanded.contains(&self.xdg_data_home()) {
            SourceKind::User
        } else if expanded.contains("/opt/") {
            SourceKind::Bundled
        } else if expanded.contains("/usr/local/share") {
            SourceKind::LocalSystem
        } else if expanded.contains("/usr/share") {
            SourceKind::System
        } else {
            SourceKind::Bundled
        }
    }

    /// `metadata_from_file(yaml_file, source_type)`: `name` and `version`
    /// (default `"1.0"`) from the first YAML document; `None` when unreadable,
    /// unparsable, not a mapping or without a truthy `name`.
    pub fn metadata_from_file(
        yaml_file: &str,
        source_type: SourceKind,
    ) -> Option<DefinitionMetadata> {
        let bytes = std::fs::read(yaml_file).ok()?;
        let docs = yeptris::parse_stream(&bytes).ok()?;
        let Some(Value::Mapping(map)) = docs.into_iter().next() else {
            return None;
        };
        let name = scalar_to_s(map.get(&Value::String("name".into()))?)?;
        let version = map
            .get(&Value::String("version".into()))
            .and_then(scalar_to_s)
            .unwrap_or_else(|| "1.0".into());
        Some(DefinitionMetadata::new(
            name,
            version,
            yaml_file,
            source_type,
        ))
    }
}
