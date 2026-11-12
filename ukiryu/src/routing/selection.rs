//! Implementation and version-file selection of `Ukiryu::Tool::Loader`
//! (lib/ukiryu/tool/loader.rb).

use super::profile::{compatible_profile, ProfileView};
use super::{RoutingError, SourceFault};

/// One version spec of an implementation, as far as selection reads it.
/// The range keys (`equals`, `before`, `after`, `between`) stay with the
/// version-range matcher behind [`SelectionEnv::match_version`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionEntry {
    /// `spec[:file]`.
    pub file: Option<String>,
    /// Ruby truthiness of `spec[:default]`.
    pub default: bool,
}

/// One entry of an implementation index's `implementations:` list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Implementation {
    /// `impl[:name]`.
    pub name: Option<String>,
    /// `impl[:default]`, the default version file.
    pub default: Option<String>,
    /// `impl[:versions]`; `None` when the key is missing.
    pub versions: Option<Vec<VersionEntry>>,
}

/// What detection reported for one implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// No output, or the presence pattern did not match: try the next one.
    NotDetected,
    /// Present; `version` is the extracted version, `None` when the pattern has
    /// no capture (or there is no pattern) and the default spec applies.
    Detected { version: Option<String> },
}

/// A version file loaded for the platform-override path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadedVersion {
    /// `impl_version.version.to_s` (`None` when the file has no version).
    pub version: Option<String>,
    pub profiles: Vec<ProfileView>,
}

/// The collaborators selection calls, in the order the source calls them.
/// Implementations are addressed by their position in the index.
pub trait SelectionEnv {
    type Error;

    /// `run_detection_command` plus the extraction and presence check of
    /// `try_implementation` (the version-detection owner's
    /// `probe_index_detection`).
    fn probe(&mut self, implementation: usize) -> Result<Probe, Self::Error>;

    /// `VersionSchemeResolver.resolve(impl[:version_scheme])`. Called before
    /// the version is looked at, so a bad scheme fails even when detection
    /// found no version.
    fn resolve_scheme(&mut self, implementation: usize) -> Result<(), Self::Error>;

    /// `find_matching_version_spec(impl[:versions], version, scheme)`: the
    /// position of the first matching version spec.
    fn match_version(
        &mut self,
        implementation: usize,
        version: &str,
    ) -> Result<Option<usize>, Self::Error>;

    /// `Register.load_implementation_version(tool, impl[:name], file)`;
    /// `None` when the file does not exist or is empty.
    fn load_version(
        &mut self,
        implementation: usize,
        file: &str,
    ) -> Result<Option<LoadedVersion>, Self::Error>;
}

/// Platform and shell for one selection run. Both are explicit: the caller
/// detects the current platform and decides the shell (D2 is owner-pending).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRequest<'a> {
    /// `Platform.detect`.
    pub current_platform: &'a str,
    /// `options[:platform]`.
    pub target_platform: Option<&'a str>,
    /// `options[:shell]`, already defaulted by the caller.
    pub shell: &'a str,
}

/// The loader's `{implementation_name:, version:, file:}` hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplementationSpec {
    pub implementation_name: Option<String>,
    /// The detected version, the loaded file's version on the platform path,
    /// or `None` for a default spec.
    pub version: Option<String>,
    pub file: Option<String>,
}

fn no_method(method: &str) -> SourceFault {
    SourceFault::new(
        "NoMethodError",
        format!("undefined method '{method}' for nil"),
    )
}

/// `detect_implementation_and_version`.
///
/// With a target platform different from the current one, detection is
/// skipped and [`find_implementation_for_platform`] decides. Otherwise each
/// implementation is probed in declaration order; the first one detected whose
/// version resolves to a spec wins. If none does, the first implementation's
/// default spec is used (`None` when there are no implementations).
pub fn detect_implementation_and_version<E: SelectionEnv>(
    implementations: &[Implementation],
    request: &SelectionRequest<'_>,
    env: &mut E,
) -> Result<Option<ImplementationSpec>, RoutingError<E::Error>> {
    if let Some(target) = request.target_platform {
        if target != request.current_platform {
            return find_implementation_for_platform(implementations, target, request.shell, env);
        }
    }
    for (i, implementation) in implementations.iter().enumerate() {
        let Probe::Detected { version } = env.probe(i).map_err(RoutingError::Env)? else {
            continue;
        };
        env.resolve_scheme(i).map_err(RoutingError::Env)?;
        let spec = match version {
            None => Some(build_default_spec(implementation)?),
            Some(version) => env
                .match_version(i, &version)
                .map_err(RoutingError::Env)?
                .map(|spec| ImplementationSpec {
                    implementation_name: implementation.name.clone(),
                    version: Some(version),
                    file: implementation
                        .versions
                        .as_ref()
                        .and_then(|v| v.get(spec))
                        .and_then(|v| v.file.clone()),
                }),
        };
        if spec.is_some() {
            return Ok(spec);
        }
    }
    match implementations.first() {
        None => Ok(None),
        Some(first) => Ok(Some(build_default_spec(first)?)),
    }
}

/// `find_implementation_for_platform`: the first implementation, in
/// declaration order, with a version file (in spec order) that loads and has a
/// [`compatible_profile`] for `platform` and `shell`. Specs without a file and
/// files that do not load are skipped.
pub fn find_implementation_for_platform<E: SelectionEnv>(
    implementations: &[Implementation],
    platform: &str,
    shell: &str,
    env: &mut E,
) -> Result<Option<ImplementationSpec>, RoutingError<E::Error>> {
    for (i, implementation) in implementations.iter().enumerate() {
        let versions = implementation
            .versions
            .as_ref()
            .ok_or_else(|| no_method("each"))?;
        for entry in versions {
            let Some(file) = &entry.file else { continue };
            let Some(loaded) = env.load_version(i, file).map_err(RoutingError::Env)? else {
                continue;
            };
            if compatible_profile(&loaded.profiles, platform, shell).is_some() {
                return Ok(Some(ImplementationSpec {
                    implementation_name: implementation.name.clone(),
                    version: loaded.version,
                    file: Some(file.clone()),
                }));
            }
        }
    }
    Ok(None)
}

/// `build_default_spec`: the spec whose file is `impl[:default]`, else the
/// spec flagged `default:` when there is no `impl[:default]`, else the last
/// spec. The file falls back to `impl[:default]` when the spec has none.
pub fn build_default_spec(
    implementation: &Implementation,
) -> Result<ImplementationSpec, SourceFault> {
    let versions = implementation
        .versions
        .as_ref()
        .ok_or_else(|| no_method("find"))?;
    let chosen = match &implementation.default {
        Some(default) => versions
            .iter()
            .find(|v| v.file.as_deref() == Some(default.as_str())),
        None => versions.iter().find(|v| v.default),
    }
    .or(versions.last())
    .ok_or_else(|| no_method("[]"))?;
    Ok(ImplementationSpec {
        implementation_name: implementation.name.clone(),
        version: None,
        file: chosen
            .file
            .clone()
            .or_else(|| implementation.default.clone()),
    })
}

/// `build_tool_name`: `tool_impl` for any implementation other than
/// `default` (exact, case-sensitive), the plain tool name otherwise.
pub fn build_tool_name(tool: &str, implementation: Option<&str>) -> String {
    match implementation {
        Some(name) if name != "default" => format!("{tool}_{name}"),
        _ => tool.to_string(),
    }
}
