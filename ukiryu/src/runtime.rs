//! Port of `Ukiryu::Runtime` (`lib/ukiryu/runtime.rb`), the platform and
//! shell detection that `Runtime.instance` reaches (`Ukiryu::Platform.detect`,
//! `Ukiryu::Shell.detect`), and an injected clock.
//!
//! The source keeps one process-wide singleton. The port makes the runtime
//! an ordinary value built from an explicit [`Host`] snapshot, so reading it
//! never touches global state and tests can supply any host.
//!
//! Shell resolution follows the decided strict contract (D2, issues #19,
//! #20 and #29): a shell must be one of the registered concrete shell names,
//! matched exactly (or by exact basename of `SHELL`). Unknown names, platform
//! groups (`unix`, `windows`) and a missing shell raise; there is no suffix
//! matching, `/etc/shells` scan, PATH probe or default shell. The pinned Ruby
//! observations that differ stay recorded in `environment/testdata`.

use crate::environment::Environment;
use crate::extractors::man::Today;
use std::cell::OnceCell;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// `Ukiryu::Platform.detect` values (the executable-discovery platform type).
pub use crate::executable::Platform;

/// `Ukiryu::Errors::UnsupportedPlatformError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedPlatform {
    pub host_os: String,
}

impl fmt::Display for UnsupportedPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unable to detect platform. Host OS: {}", self.host_os)
    }
}

impl std::error::Error for UnsupportedPlatform {}

/// `Platform.detect` from an `RbConfig::CONFIG['host_os']` string with
/// `Gem.win_platform?` false: case-insensitive substring tests for
/// `mswin|mingw|windows`, then `darwin|mac os`, then `linux`.
pub fn platform_from_host_os(host_os: &str) -> Result<Platform, UnsupportedPlatform> {
    let h = host_os.to_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| h.contains(n));
    if any(&["mswin", "mingw", "windows"]) {
        Ok(Platform::Windows)
    } else if any(&["darwin", "mac os"]) {
        Ok(Platform::Macos)
    } else if h.contains("linux") {
        Ok(Platform::Linux)
    } else {
        Err(UnsupportedPlatform {
            host_os: host_os.to_string(),
        })
    }
}

/// The platform this binary was compiled for.
pub fn native_platform() -> Result<Platform, UnsupportedPlatform> {
    if cfg!(windows) {
        Ok(Platform::Windows)
    } else if cfg!(target_os = "macos") {
        Ok(Platform::Macos)
    } else if cfg!(target_os = "linux") {
        Ok(Platform::Linux)
    } else {
        Err(UnsupportedPlatform {
            host_os: std::env::consts::OS.to_string(),
        })
    }
}

/// The registered concrete shells (`Shell::INDIVIDUAL_SHELLS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellName {
    Bash,
    Zsh,
    Fish,
    Sh,
    Dash,
    Tcsh,
    PowerShell,
    Cmd,
}

/// `Shell::PLATFORM_GROUPS` names that are not themselves concrete shells.
/// `powershell` is both a group and a concrete shell and parses as the shell.
pub const PLATFORM_GROUP_ONLY: [&str; 2] = ["unix", "windows"];

impl ShellName {
    pub const ALL: [ShellName; 8] = [
        ShellName::Bash,
        ShellName::Zsh,
        ShellName::Fish,
        ShellName::Sh,
        ShellName::Dash,
        ShellName::Tcsh,
        ShellName::PowerShell,
        ShellName::Cmd,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ShellName::Bash => "bash",
            ShellName::Zsh => "zsh",
            ShellName::Fish => "fish",
            ShellName::Sh => "sh",
            ShellName::Dash => "dash",
            ShellName::Tcsh => "tcsh",
            ShellName::PowerShell => "powershell",
            ShellName::Cmd => "cmd",
        }
    }

    /// Exact registered name lookup; anything else (including `pwsh` and
    /// the `unix`/`windows` groups) is `None`.
    pub fn lookup(name: &str) -> Option<ShellName> {
        ShellName::ALL.into_iter().find(|s| s.as_str() == name)
    }

    /// Strict parse of a configured shell name, recording where it came from.
    pub fn parse(name: &str, source: ShellSource) -> Result<ShellName, ShellError> {
        if let Some(shell) = ShellName::lookup(name) {
            return Ok(shell);
        }
        if PLATFORM_GROUP_ONLY.contains(&name) {
            return Err(ShellError::PlatformGroup {
                source,
                value: name.to_string(),
            });
        }
        Err(ShellError::Unknown {
            source,
            value: name.to_string(),
        })
    }

    /// `unix_shell?`: the source lists only bash, zsh, fish and sh, so dash
    /// and tcsh answer `false`.
    pub fn is_unix_shell(self) -> bool {
        matches!(
            self,
            ShellName::Bash | ShellName::Zsh | ShellName::Fish | ShellName::Sh
        )
    }

    /// `windows_shell?`: powershell and cmd.
    pub fn is_windows_shell(self) -> bool {
        matches!(self, ShellName::PowerShell | ShellName::Cmd)
    }
}

impl fmt::Display for ShellName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a shell name came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellSource {
    /// `Runtime#shell=` or an explicit context value.
    Explicit,
    /// `Config.shell` (`--shell`, `UKIRYU_SHELL` or programmatic config).
    Configured,
    /// The `UKIRYU_TEST_SHELL` variable.
    TestOverride,
    /// The `SHELL` variable.
    ShellVariable,
}

/// `Ukiryu::Errors::UnknownShellError` under the strict D2 contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellError {
    /// Not a registered concrete shell name.
    Unknown { source: ShellSource, value: String },
    /// A platform group used where a concrete shell is required.
    PlatformGroup { source: ShellSource, value: String },
    /// Nothing configured and nothing in the snapshot names a shell.
    Missing { platform: Platform },
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellError::Unknown {
                source: ShellSource::ShellVariable,
                value,
            } => write!(f, "Unknown shell in SHELL: {value}"),
            ShellError::Unknown { value, .. } => write!(f, "Unknown shell: {value}"),
            ShellError::PlatformGroup { value, .. } => {
                write!(f, "Shell platform group is not a concrete shell: {value}")
            }
            ShellError::Missing {
                platform: Platform::Windows,
            } => f.write_str(
                "Unable to detect shell: none of PSModulePath, MSYSTEM, MINGW_PREFIX or WSL_DISTRO is set",
            ),
            ShellError::Missing { .. } => f.write_str("Unable to detect shell: SHELL not set"),
        }
    }
}

impl std::error::Error for ShellError {}

/// `Shell.detect` over an explicit snapshot, under the strict contract.
///
/// - `UKIRYU_TEST_SHELL`, when present, must be a concrete shell name.
/// - Windows (names matched ASCII case-insensitively, as Ruby's Windows
///   `ENV` does): `PSModulePath` gives powershell; `MSYSTEM`, `MINGW_PREFIX` or
///   `WSL_DISTRO` gives bash; otherwise the shell is missing (the source
///   defaults to cmd).
/// - Elsewhere: the exact basename of `SHELL`; a missing `SHELL` raises
///   (the source scans `/etc/shells` and PATH).
pub fn detect_shell(platform: Platform, env: &Environment) -> Result<ShellName, ShellError> {
    if let Some(name) = env_lookup(platform, env, "UKIRYU_TEST_SHELL") {
        return ShellName::parse(name, ShellSource::TestOverride);
    }
    if platform == Platform::Windows {
        let set = |name: &str| env_lookup(platform, env, name).is_some();
        if set("PSModulePath") {
            return Ok(ShellName::PowerShell);
        }
        if set("MSYSTEM") || set("MINGW_PREFIX") || set("WSL_DISTRO") {
            return Ok(ShellName::Bash);
        }
        return Err(ShellError::Missing { platform });
    }
    match env.get("SHELL") {
        Some(path) => ShellName::lookup(basename(path)).ok_or_else(|| ShellError::Unknown {
            source: ShellSource::ShellVariable,
            value: path.to_string(),
        }),
        None => Err(ShellError::Missing { platform }),
    }
}

/// Ruby `ENV[name]`: exact on Unix, ASCII case-insensitive on Windows
/// (first matching key in snapshot order).
fn env_lookup<'a>(platform: Platform, env: &'a Environment, name: &str) -> Option<&'a str> {
    if platform == Platform::Windows {
        env.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    } else {
        env.get(name)
    }
}

/// `File.basename` for a Unix path: trailing slashes are ignored.
fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// The source of "today" for date-dependent parsing (`Date.today`).
/// Native callers use the system clock; tests inject a fixed day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// The system clock, shifted by an explicit UTC offset in seconds. The
    /// port does not read the host time zone; the caller supplies it.
    System { utc_offset_secs: i32 },
    /// A fixed day.
    Fixed(Today),
}

impl Default for Clock {
    fn default() -> Clock {
        Clock::System { utc_offset_secs: 0 }
    }
}

impl Clock {
    pub fn today(&self) -> Today {
        match *self {
            Clock::Fixed(today) => today,
            Clock::System { utc_offset_secs } => {
                let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(d) => d.as_secs() as i64,
                    Err(e) => -(e.duration().as_secs() as i64),
                };
                civil_from_unix_secs(secs + i64::from(utc_offset_secs))
            }
        }
    }
}

/// The proleptic Gregorian day containing a Unix timestamp.
pub fn civil_from_unix_secs(secs: i64) -> Today {
    let days = secs.div_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let mday = doy - (153 * mp + 2) / 5 + 1;
    let mon = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(mon <= 2);
    Today {
        year: year as i32,
        mon: mon as i32,
        mday: mday as i32,
    }
}

/// The host facts a runtime detects from.
#[derive(Clone, Debug, PartialEq)]
pub struct Host {
    pub platform: Platform,
    pub env: Environment,
}

impl Host {
    /// The compiled-for platform and a read-only snapshot of the process
    /// environment.
    pub fn native() -> Result<Host, HostError> {
        Ok(Host {
            platform: native_platform().map_err(HostError::Platform)?,
            env: Environment::from_process().map_err(HostError::Environment)?,
        })
    }
}

/// Why [`Host::native`] could not capture the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    Platform(UnsupportedPlatform),
    Environment(crate::environment::NonUnicodeVariable),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Platform(e) => e.fmt(f),
            HostError::Environment(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for HostError {}

/// `RuntimeError: Runtime is locked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeLocked;

impl fmt::Display for RuntimeLocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Runtime is locked")
    }
}

impl std::error::Error for RuntimeLocked {}

/// `Ukiryu::Runtime`, as a value. Platform and shell are resolved lazily
/// and cached, as in the source; a failed shell resolution is not cached.
#[derive(Clone, Debug)]
pub struct Runtime {
    host: Host,
    configured_shell: Option<String>,
    clock: Clock,
    platform: Option<Platform>,
    shell: OnceCell<ShellName>,
    locked: bool,
}

impl Runtime {
    /// A runtime over `host`. `configured_shell` is `Config.shell`.
    pub fn new(host: Host, configured_shell: Option<String>, clock: Clock) -> Runtime {
        Runtime {
            host,
            configured_shell,
            clock,
            platform: None,
            shell: OnceCell::new(),
            locked: false,
        }
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// `Date.today` through the injected clock.
    pub fn today(&self) -> Today {
        self.clock.today()
    }

    /// `runtime.platform`: an explicit value, else the host platform.
    pub fn platform(&self) -> Platform {
        self.platform.unwrap_or(self.host.platform)
    }

    /// `runtime.shell`: explicit value, else `Config.shell`, else detection.
    pub fn shell(&self) -> Result<ShellName, ShellError> {
        if let Some(shell) = self.shell.get() {
            return Ok(*shell);
        }
        let shell = match &self.configured_shell {
            Some(name) => ShellName::parse(name, ShellSource::Configured)?,
            None => detect_shell(self.platform(), &self.host.env)?,
        };
        Ok(*self.shell.get_or_init(|| shell))
    }

    /// `runtime.platform = value`.
    pub fn set_platform(&mut self, platform: Platform) -> Result<(), RuntimeLocked> {
        if self.locked {
            return Err(RuntimeLocked);
        }
        self.platform = Some(platform);
        Ok(())
    }

    /// `runtime.shell = value`.
    pub fn set_shell(&mut self, shell: ShellName) -> Result<(), RuntimeLocked> {
        if self.locked {
            return Err(RuntimeLocked);
        }
        self.shell = OnceCell::from(shell);
        Ok(())
    }

    /// `runtime.lock!`.
    pub fn lock(&mut self) {
        self.locked = true;
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// `runtime.reset!`: drops explicit values and caches and unlocks.
    pub fn reset(&mut self) {
        self.platform = None;
        self.shell = OnceCell::new();
        self.locked = false;
    }

    pub fn on_platform(&self, platform: Platform) -> bool {
        self.platform() == platform
    }

    pub fn using_shell(&self, shell: ShellName) -> Result<bool, ShellError> {
        Ok(self.shell()? == shell)
    }

    pub fn is_windows(&self) -> bool {
        self.on_platform(Platform::Windows)
    }

    pub fn is_macos(&self) -> bool {
        self.on_platform(Platform::Macos)
    }

    pub fn is_linux(&self) -> bool {
        self.on_platform(Platform::Linux)
    }

    pub fn unix_shell(&self) -> Result<bool, ShellError> {
        Ok(self.shell()?.is_unix_shell())
    }

    pub fn windows_shell(&self) -> Result<bool, ShellError> {
        Ok(self.shell()?.is_windows_shell())
    }
}
