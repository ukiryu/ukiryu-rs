//! `Models::ImplementationVersion#compatible_profile`
//! (lib/ukiryu/models/implementation_version.rb).

/// The part of an execution profile that profile selection reads.
///
/// The loader builds it from a register-loaded (key-symbolized) profile hash:
/// `platforms` is `Array(prof[:platforms] || prof[:platform]).map(&:to_s)`,
/// `shells` likewise from `:shells`/`:shell`, `name` and `inherits` are the
/// values' `to_s`, and `has_commands` is false when `commands` is missing or
/// empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileView {
    pub name: Option<String>,
    pub platforms: Vec<String>,
    pub shells: Vec<String>,
    pub inherits: Option<String>,
    pub has_commands: bool,
}

/// The selected profile and where its commands come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileMatch {
    /// Index of the first profile listing both the platform and the shell.
    pub profile: usize,
    /// Index of the profile whose `commands` the result carries: the parent
    /// named by `inherits` when that parent exists and the selected profile has
    /// no commands of its own, otherwise `profile`.
    pub commands_from: usize,
}

/// Exact membership: a profile matches only if its platform list contains
/// `platform` and its shell list contains `shell`. Unlike the `ToolIndex`
/// alias check, an empty list matches nothing.
pub fn compatible_profile(
    profiles: &[ProfileView],
    platform: &str,
    shell: &str,
) -> Option<ProfileMatch> {
    let profile = profiles.iter().position(|p| {
        p.platforms.iter().any(|x| x == platform) && p.shells.iter().any(|x| x == shell)
    })?;
    let selected = &profiles[profile];
    let mut commands_from = profile;
    if let Some(inherits) = &selected.inherits {
        // `prof_name.to_s == inherits.to_s`: a nameless profile is "".
        let parent = profiles
            .iter()
            .position(|p| p.name.as_deref().unwrap_or("") == inherits);
        if let Some(parent) = parent {
            if !selected.has_commands {
                commands_from = parent;
            }
        }
    }
    Some(ProfileMatch {
        profile,
        commands_from,
    })
}
