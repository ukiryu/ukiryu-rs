//! `ukiryu run-file REQUEST_FILE` (`RunFileCommand#run`).
//!
//! The request is loaded with [`ukiryu::request::load_execution_request`]
//! and the tool resolved through the register ([`Registry`]).
//!
//! Frozen source observation: once the tool and its options object are
//! built, `execute_request` calls `tool.execute(command, options, timeout:
//! timeout)` (a positional options object and `timeout:` instead of
//! `execution_timeout:`), which raises `ArgumentError` for every request.
//! The source therefore never runs a process from `run-file`; the handler
//! answers with that `ErrorResponse` (see the `runfile_execute*` captures).
use super::{
    colors_disabled, config_format, config_string, configuration, options_check, reject_debug,
    reject_metrics, render, render_use_color, tool_options, truthy, ExecuteContext, ExecuteError,
    Out, Registry, Response,
};
use crate::commands::CommandOutcome;
use ukiryu::models::ErrorResponse;
use ukiryu::render::{render_dry_run, DryRunStyle, OutputWrite};
use ukiryu::request::{load_execution_request, RequestError};

/// `ukiryu run-file REQUEST_FILE` options, as given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunFileInput {
    /// `REQUEST_FILE`.
    pub request_file: String,
    /// `--register` / `-r`.
    pub register: Option<String>,
    /// `--output` / `-o`.
    pub output: Option<String>,
    /// `--format` / `-f` (`None` when not given).
    pub format: Option<String>,
    /// `--dry-run` / `-d` (Thor default false).
    pub dry_run: bool,
    /// `--shell`.
    pub shell: Option<String>,
}

/// `RunFileCommand#run(request_file)`.
pub fn run_file(input: RunFileInput, ctx: &ExecuteContext) -> Result<CommandOutcome, ExecuteError> {
    let config = configuration(
        &ctx.host,
        input.format.as_deref(),
        input.output.as_deref(),
        input.register.as_deref(),
        input.shell.as_deref(),
        Some(input.dry_run),
    )?;
    let out = Out::new(colors_disabled(&config));
    reject_debug(&config)?;

    let format = match config_format(&config)? {
        Ok(f) => f,
        Err(message) => return Ok(out.error(&message)),
    };

    let request = match load_execution_request(&input.request_file) {
        Ok(r) => r,
        Err(RequestError::Invalid(v)) => {
            // Raised in the `else` branch of the loading `begin`, which its
            // `rescue` clauses do not cover.
            return Err(ExecuteError::raised("RuntimeError", v.to_string()));
        }
        Err(RequestError::Unsupported { feature, detail }) => {
            return Err(ExecuteError::unsupported(format!(
                "request file YAML ({feature}): {detail}"
            )))
        }
        Err(e) => return Ok(out.error(&e.to_string())),
    };

    if truthy(config.resolve("dry_run")) {
        let arguments: Vec<_> = request
            .arguments()
            .entries()
            .iter()
            .map(|(k, v)| (k.to_ruby_s(), v.clone()))
            .collect();
        let text = render_dry_run(
            DryRunStyle::RunFile,
            &request.tool().to_ruby_s(),
            &request.command().to_ruby_s(),
            &arguments,
            render_use_color(&config),
        );
        let mut out = out;
        out.push(OutputWrite::Stdout(text));
        return Ok(out.done());
    }

    reject_metrics(&config)?;
    // `stringify_keys(request['arguments'])` runs before the `begin`.
    request
        .arguments()
        .stringify_keys()
        .map_err(|e| ExecuteError::raised(e.ruby_class(), e.message.clone()))?;
    // `tool_name.to_sym` is the first statement inside the `begin`.
    if let Err(e) = request.tool().to_sym() {
        let response = Response::Error(ErrorResponse::from_message(
            &format!("Unexpected error: {}: {}", e.ruby_class(), e.message),
            1,
        ));
        let settings = ctx.render_settings(&config);
        let output = config_string(&config, "output");
        return render(&response, format, output.as_deref(), &settings);
    }
    let tool_name = request.tool().to_ruby_s();
    let command = request.command().to_ruby_s();
    let options = tool_options(&ctx.host, config_string(&config, "shell"))?;
    let registry = Registry::new(ctx, &config, options.shell);
    let response = match registry.get(&tool_name, &options, &out)? {
        Err(response) => response,
        Ok(tool) => {
            let keys: Vec<String> = request
                .arguments()
                .entries()
                .iter()
                .map(|(k, _)| k.to_ruby_s())
                .collect();
            let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
            match options_check(
                &tool, &tool_name, &command, &keys, &registry, &options, &out,
            )? {
                Some(response) => response,
                // `tool.execute(command_name.to_sym, options, timeout: timeout)`
                // does not match `execute(root_path, execution_timeout:, **params)`.
                None => ErrorResponse::from_message(EXECUTE_SIGNATURE_ERROR, 1),
            }
        }
    };
    let settings = ctx.render_settings(&config);
    let output = config_string(&config, "output");
    render(
        &Response::Error(response),
        format,
        output.as_deref(),
        &settings,
    )
}

/// The `ArgumentError` message of the source's `tool.execute` call, rescued
/// as `Invalid arguments: ...` (captured from the pinned oracle).
const EXECUTE_SIGNATURE_ERROR: &str =
    "Invalid arguments: wrong number of arguments (given 2, expected 1; required keyword: execution_timeout)";
