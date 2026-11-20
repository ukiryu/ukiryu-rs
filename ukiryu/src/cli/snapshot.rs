//! Explicit, read-only inputs for portable CLI rendering. No host filesystem,
//! environment, executable search, bootstrap, network or process adapter is used.
use super::commands::{author, inspect, CommandOutcome};
use crate::definition::DiscoveryEnv;
use crate::register::Register;
use crate::runtime::{Clock, Host, Runtime};
use crate::tool::{Tool, ToolOptions};
use crate::tool_loader::{load_definition_from_text, LoadError, LoadOptions};
use crate::version_detection::{CommandOutput, CommandRequest, CommandRunner, RunnerError};
use clap::ArgMatches;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Paths are caller-defined virtual names, not paths to open on the host.
/// An omitted file is absent. Empty discovery/probes explicitly mean none.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub files: BTreeMap<String, String>,
    pub executables: BTreeMap<String, Executable>,
    pub probes: Vec<Probe>,
    pub definitions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Executable {
    pub path: String,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub executable: String,
    pub args: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
}

struct Runner<'a>(&'a [Probe]);
impl CommandRunner for Runner<'_> {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, RunnerError> {
        self.0
            .iter()
            .find(|p| p.executable == request.executable && p.args == request.args)
            .map(|p| CommandOutput {
                stdout: p.stdout.as_bytes().to_vec(),
                stderr: p.stderr.as_bytes().to_vec(),
                exit_status: p.exit,
            })
            .ok_or_else(|| RunnerError::new("no probe result in the read-only snapshot"))
    }
}

fn text(sub: &ArgMatches, name: &str) -> Option<String> {
    sub.try_get_one::<std::ffi::OsString>(name)
        .ok()
        .flatten()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
}
fn flag(sub: &ArgMatches, name: &str) -> bool {
    sub.try_get_one::<bool>(name)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

impl Snapshot {
    fn tool(&self, register: &str, name: &str, host: &Host) -> Result<Option<Tool>, String> {
        let runtime = Runtime::new(
            host.clone(),
            host.env.get("UKIRYU_SHELL").map(str::to_owned),
            Clock::default(),
        );
        let shell = runtime.shell().map_err(|e| e.to_string())?;
        let options = LoadOptions {
            current_platform: host.platform.as_str(),
            platform: None,
            shell: shell.as_str(),
            executable: None,
        };
        let read = |path: &std::path::Path| -> Result<Option<String>, LoadError> {
            Ok(self
                .files
                .get(&path.to_string_lossy().replace('\\', "/"))
                .cloned())
        };
        let definition = load_definition_from_text(
            &Register::new(
                std::path::PathBuf::from(register),
                crate::register::Source::Manual,
            ),
            name,
            &options,
            &mut Runner(&self.probes),
            &read,
        )
        .map_err(|e| e.to_string())?;
        definition
            .map(|d| {
                let executable =
                    self.executables
                        .get(name)
                        .map(|e| crate::executable::Discovered {
                            path: e.path.clone(),
                            info: crate::executable::ExecutableInfo {
                                path: e.path.clone(),
                                source: crate::executable::ExecutableSource::Path,
                                shell: shell.as_str().to_owned(),
                                alias_definition: None,
                            },
                        });
                Tool::from_snapshot(d, &ToolOptions::new(host.platform, shell), executable)
                    .map_err(|e| e.to_string())
            })
            .transpose()
    }

    fn names(&self, register: &str) -> Vec<String> {
        let prefix = format!("{register}/tools/");
        self.files
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix)?.strip_suffix("/index.yaml"))
            .filter(|p| !p.contains('/'))
            .map(str::to_owned)
            .collect()
    }

    pub(super) fn render(
        &self,
        verb: &str,
        sub: &ArgMatches,
        host: &Host,
    ) -> Result<CommandOutcome, String> {
        let colors_off = inspect::snapshot_colors(host).map_err(|e| e.to_string())?;
        let ctx = author::AuthorContext {
            thor_colors: false,
            discovery: DiscoveryEnv {
                home: None,
                xdg_data_home: None,
                xdg_data_dirs: None,
                path: None,
                opt_root: String::new(),
            },
            schema_path_env: None,
            remote_schema: crate::definition::definition_validator::RemoteSchemaSource::Unavailable,
            default_register: None,
            config_colors_disabled: colors_off,
        };
        if verb == "validate" {
            let Some(("file", leaf)) = sub.subcommand() else {
                return Err("snapshot rendering supports validate file only".into());
            };
            if flag(leaf, "executable") || flag(leaf, "all") {
                return Err("executable validation is forbidden in a read-only snapshot".into());
            }
            let path = text(leaf, "path").unwrap_or_default();
            let yaml = self
                .files
                .get(&path)
                .ok_or_else(|| format!("input is absent from snapshot: {path}"))?;
            let schema_path = text(leaf, "schema");
            let schema = schema_path
                .as_ref()
                .map(|p| {
                    let format = if p.ends_with(".json") {
                        crate::schema::DefinitionSchemaFormat::Json
                    } else {
                        crate::schema::DefinitionSchemaFormat::Yaml
                    };
                    self.files
                        .get(p)
                        .map(|s| (s.as_str(), format))
                        .ok_or_else(|| format!("schema is absent from snapshot: {p}"))
                })
                .transpose()?;
            return author::validate_snapshot(
                &path,
                yaml,
                schema,
                text(leaf, "format").as_deref().unwrap_or("text"),
                flag(leaf, "strict"),
                &ctx,
            )
            .map_err(|e| e.to_string());
        }
        if !matches!(verb, "commands" | "info" | "opts" | "describe" | "resolve") {
            return Err(format!(
                "'{verb}' is unavailable in read-only snapshot rendering"
            ));
        }
        if flag(sub, "all")
            || host
                .env
                .get("UKIRYU_DEBUG")
                .is_some_and(|v| v != "false" && !v.is_empty())
        {
            return Err("snapshot rendering does not support --all/debug inspection".into());
        }
        let name = text(sub, "tool").unwrap_or_default();
        if verb == "resolve" {
            if !self.definitions.is_empty() {
                return Err(
                    "nonempty definition discovery is not yet supported by snapshot rendering"
                        .into(),
                );
            }
            return inspect::from_snapshot(verb, name, None, None, &[], colors_off, None)
                .map_err(|e| e.to_string());
        }
        let register = text(sub, "register")
            .filter(|p| !p.is_empty())
            .or_else(|| host.env.get("UKIRYU_REGISTER").map(str::to_owned))
            .ok_or("snapshot rendering requires an explicit register")?;
        let names = self.names(&register);
        let tool = self.tool(&register, &name, host)?;
        let version = self.executables.get(&name).and_then(|e| e.version.clone());
        if verb == "info"
            && tool.as_ref().is_some_and(|t| {
                t.definition().implements.is_some() && t.name().as_deref() != Some(name.as_str())
            })
        {
            return Err("snapshot rendering does not yet support interface inspection".into());
        }
        if verb == "describe" {
            let Some(tool) = tool else {
                return Err(format!("tool absent from snapshot: {name}"));
            };
            return author::describe_snapshot(
                author::DescribeInput {
                    tool: name,
                    command: text(sub, "command"),
                    register: Some(register),
                },
                &ctx,
                &tool,
                version,
            )
            .map_err(|e| e.to_string());
        }
        inspect::from_snapshot(
            verb,
            name,
            text(sub, "command"),
            tool.as_ref(),
            &names,
            colors_off,
            version,
        )
        .map_err(|e| e.to_string())
    }
}

/// Wire request: the environment is complete, never an overlay on host ENV.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub argv: Vec<String>,
    pub platform: String,
    pub env: BTreeMap<String, String>,
    pub snapshot: Snapshot,
}

pub fn process(request: Request) -> serde_json::Value {
    let platform = match request.platform.as_str() {
        "macos" => Ok(crate::runtime::Platform::Macos),
        other => crate::runtime::platform_from_host_os(other),
    };
    let platform = match platform {
        Ok(p) => p,
        Err(e) => return crate::wire::failure("INVALID_REQUEST", e),
    };
    let host = Host {
        platform,
        env: crate::environment::Environment::from_snapshot(request.env),
    };
    let args = std::iter::once("ukiryu".to_owned())
        .chain(request.argv)
        .collect();
    match super::dispatch::capture_snapshot(args, host, request.snapshot) {
        Ok(output) => serde_json::json!({"ok":true,"value":output}),
        Err(error) => crate::wire::failure("UNSUPPORTED_CAPABILITY", error),
    }
}
