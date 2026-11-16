//! `Ukiryu::Native::Tool` and `Ukiryu::Native::ExecutionResult`: the core's
//! `Tool.get`, `Tool#execute` and execution result, handed through as they
//! are.
//!
//! `Tool.get` builds a [`RegisterSource`] over the register the caller names
//! (`register_path:`, else the register discovery that needs no clone) and
//! calls [`tool::get`]; `Tool#execute` calls [`Tool::execute`]. The result is
//! the core's [`ExecutionResult`], read through its own accessors. Failures
//! are raised by [`tool_error`]. Nothing about tools, commands or results is
//! decided here.
//!
//! Differences from the Ruby source, each visible to a caller:
//!
//! - the tool cache is per call: `Tool.get(name)` twice returns two objects,
//!   where the source returns the cached one;
//! - the register is never cloned: without `register_path:` and without an
//!   existing register, `Tool.get` raises `RuntimeError`;
//! - a failed run does not keep its result on the exception (the core's
//!   [`ExecuteError::Failed`] holds it; the source's `ExecutionError` has no
//!   `#result`), and `ExecutionResult#to_h` is not bound yet.
use super::errors::{shell_error, tool_error};
use super::values::{from_ruby, to_ruby};
use crate::cache::ToolCache;
use crate::environment::Environment;
use crate::execution::{ExecutionResult, InvalidByteSequence, Seconds};
use crate::extractors::base::ProcessHost;
use crate::register::{DiscoveryContext, Register};
use crate::runtime::{Clock, Host, Platform, Runtime, ShellName, ShellSource};
use crate::tool::{self, ExecuteSettings, Tool, ToolOptions};
use crate::tool_loader::RegisterSource;
use crate::types::Value;
use magnus::{
    function, method, prelude::*, scan_args::scan_args, typed_data::Obj, Error, RHash, RModule,
    RString, Ruby, Value as RValue,
};
use std::rc::Rc;

/// The Ruby object: one core tool, freed with its Ruby object.
#[magnus::wrap(class = "Ukiryu::Native::Tool", free_immediately, size)]
struct NativeTool(Tool);

// SAFETY: magnus requires `Send` for wrapped data. A `Tool` holds a
// `ToolDefinition`, whose profile caches its routing tree in `Rc<RefCell<_>>`,
// so the derived bound fails. `get` builds the tool in a cache it drops before
// returning, so the Rc graph inside this object is not shared with any other
// object or Rust state; the object cannot be copied or moved to another
// Ractor (no allocator, not shareable), and Ruby threads of one Ractor run
// only while holding its lock, so the counts are never touched concurrently.
unsafe impl Send for NativeTool {}

/// The Ruby object for one finished run.
#[magnus::wrap(class = "Ukiryu::Native::ExecutionResult", free_immediately, size)]
struct NativeResult(ExecutionResult);

pub(super) fn define(ruby: &Ruby, native: RModule) -> Result<(), Error> {
    let tool = native.define_class("Tool", ruby.class_object())?;
    tool.undef_default_alloc_func();
    tool.define_singleton_method("get", function!(get, -1))?;
    tool.define_method("name", method!(NativeTool::name, 0))?;
    tool.define_method("version", method!(NativeTool::version, 0))?;
    tool.define_method("available?", method!(NativeTool::is_available, 0))?;
    tool.define_method("execute", method!(NativeTool::execute, -1))?;

    let result = native.define_class("ExecutionResult", ruby.class_object())?;
    result.undef_default_alloc_func();
    result.define_method("command", method!(NativeResult::command, 0))?;
    result.define_method("executable", method!(NativeResult::executable, 0))?;
    result.define_method("executable_name", method!(NativeResult::executable_name, 0))?;
    result.define_method("stdout", method!(NativeResult::stdout, 0))?;
    result.define_method("stderr", method!(NativeResult::stderr, 0))?;
    for name in ["status", "exit_code", "exit_status"] {
        result.define_method(name, method!(NativeResult::status, 0))?;
    }
    result.define_method("started_at", method!(NativeResult::started_at, 0))?;
    result.define_method("finished_at", method!(NativeResult::finished_at, 0))?;
    result.define_method("duration", method!(NativeResult::duration, 0))?;
    result.define_method("execution_time", method!(NativeResult::execution_time, 0))?;
    result.define_method("success?", method!(NativeResult::is_success, 0))?;
    result.define_method("failure?", method!(NativeResult::is_failure, 0))?;
    result.define_method("output", method!(NativeResult::output, 0))?;
    result.define_method("error_output", method!(NativeResult::error_output, 0))?;
    result.define_method("to_json", method!(NativeResult::to_json, -1))?;
    Ok(())
}

/// A String or Symbol argument as text.
fn name_text(ruby: &Ruby, value: RValue, what: &str) -> Result<String, Error> {
    match from_ruby(ruby, value)? {
        Value::String(s) | Value::Symbol(s) => Ok(s),
        _ => Err(Error::new(
            ruby.exception_type_error(),
            format!("{what} must be a String or Symbol"),
        )),
    }
}

/// `options[key]` as text; absent and nil are the same.
fn option_text(ruby: &Ruby, options: &Value, key: &str) -> Result<Option<String>, Error> {
    match options.get_sym(key) {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::String(s) | Value::Symbol(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Error::new(
            ruby.exception_type_error(),
            format!("option {key} must be a String or Symbol"),
        )),
    }
}

/// `Ukiryu::Native::Tool.get(name, options = {})` with the options
/// `register_path`, `platform`, `shell`, `version` and `executable`. The
/// platform and shell default to the host's, the shell strictly
/// ([`Runtime::shell`]).
fn get(ruby: &Ruby, args: &[RValue]) -> Result<Obj<NativeTool>, Error> {
    let args = scan_args::<(RValue,), (Option<RHash>,), (), (), (), ()>(args)?;
    let (name,) = args.required;
    let (options,) = args.optional;
    let name = name_text(ruby, name, "tool name")?;
    let options = match options {
        Some(hash) => from_ruby(ruby, hash.as_value())?,
        None => Value::Hash(Vec::new()),
    };

    let host =
        Host::native().map_err(|e| Error::new(ruby.exception_runtime_error(), e.to_string()))?;
    let current = host.platform;
    let runtime = Runtime::new(host, None, Clock::default());
    let platform = match option_text(ruby, &options, "platform")? {
        Some(name) => Platform::from_name(&name).ok_or_else(|| {
            Error::new(
                ruby.exception_arg_error(),
                format!("unknown platform: {name}"),
            )
        })?,
        None => runtime.platform(),
    };
    let shell = match option_text(ruby, &options, "shell")? {
        Some(name) => ShellName::parse(&name, ShellSource::Explicit),
        None => runtime.shell(),
    }
    .map_err(|e| shell_error(ruby, e))?;

    let register = match option_text(ruby, &options, "register_path")? {
        Some(path) => Register::at(path),
        None => match DiscoveryContext::from_process(None).resolve_path_without_clone() {
            Some(path) => Register::at(path),
            None => {
                return Err(Error::new(
                    ruby.exception_runtime_error(),
                    "no register found; pass register_path:",
                ))
            }
        },
    };
    let mut source = RegisterSource::new(
        register,
        current,
        ProcessHost::new(Environment::new(), shell),
    );
    if let Some(executable) = option_text(ruby, &options, "executable")? {
        source = source.executable(executable);
    }
    let mut tool_options = ToolOptions::new(platform, shell);
    if let Some(version) = option_text(ruby, &options, "version")? {
        tool_options = tool_options.version(version);
    }

    let cache = ToolCache::new();
    let found =
        tool::get(&source, &cache, &name, &tool_options).map_err(|e| tool_error(ruby, e))?;
    drop(cache);
    let tool = Rc::try_unwrap(found).unwrap_or_else(|shared| (*shared).clone());
    Ok(ruby.obj_wrap(NativeTool(tool)))
}

impl NativeTool {
    fn name(rb_self: &Self) -> Option<String> {
        rb_self.0.name()
    }

    fn version(rb_self: &Self) -> Option<String> {
        rb_self.0.version()
    }

    fn is_available(rb_self: &Self) -> bool {
        rb_self.0.is_available()
    }

    /// `#execute(root_path, execution_timeout:, **params)`. The timeout is an
    /// Integer or Float; the rest of the keywords are the command's
    /// parameters, passed to the core as it receives them.
    fn execute(ruby: &Ruby, rb_self: &Self, args: &[RValue]) -> Result<Obj<NativeResult>, Error> {
        let args = scan_args::<(RValue,), (), (), (), RHash, ()>(args)?;
        let (root_path,) = args.required;
        let root_path = name_text(ruby, root_path, "root path")?;
        let mut params = from_ruby(ruby, args.keywords.as_value())?;
        let timeout = take_timeout(ruby, &mut params)?;
        let settings = ExecuteSettings::new(timeout);
        let execution = rb_self
            .0
            .execute(&root_path, &params, &settings)
            .map_err(|e| tool_error(ruby, e))?;
        Ok(ruby.obj_wrap(NativeResult(execution.result)))
    }
}

/// Removes `execution_timeout:` from the keyword parameters and returns it.
fn take_timeout(ruby: &Ruby, params: &mut Value) -> Result<Seconds, Error> {
    let Value::Hash(pairs) = params else {
        return Err(Error::new(ruby.exception_arg_error(), "expected keywords"));
    };
    let position = pairs
        .iter()
        .position(|(key, _)| matches!(key, Value::Symbol(s) if s == "execution_timeout"))
        .ok_or_else(|| {
            Error::new(
                ruby.exception_arg_error(),
                "missing keyword: :execution_timeout",
            )
        })?;
    match pairs.remove(position).1 {
        Value::Integer(seconds) => i64::try_from(seconds).map(Seconds::Int).map_err(|_| {
            Error::new(
                ruby.exception_arg_error(),
                "execution_timeout is out of range",
            )
        }),
        Value::Float(seconds) => Ok(Seconds::Float(seconds)),
        _ => Err(Error::new(
            ruby.exception_arg_error(),
            "execution_timeout must be an Integer or a Float",
        )),
    }
}

/// Bytes as the UTF-8 tagged String the source reads from a pipe; invalid
/// sequences stay as they are.
fn utf8(ruby: &Ruby, bytes: &[u8]) -> Result<RString, Error> {
    let string = ruby.str_from_slice(bytes);
    string.funcall::<_, _, RValue>("force_encoding", ("UTF-8",))?;
    Ok(string)
}

/// The Ruby class the core records for an invalid byte sequence.
fn invalid_bytes(ruby: &Ruby, error: InvalidByteSequence) -> Error {
    match super::errors::error_class(ruby, error.ruby_class()) {
        Ok(class) => Error::new(class, error.message()),
        Err(error) => error,
    }
}

fn time_value(ruby: &Ruby, time: Option<&crate::types::Time>) -> Result<RValue, Error> {
    match time {
        Some(time) => to_ruby(ruby, &Value::Time(time.clone())),
        None => Ok(ruby.qnil().as_value()),
    }
}

impl NativeResult {
    fn command(rb_self: &Self) -> String {
        rb_self.0.command().to_owned()
    }

    fn executable(rb_self: &Self) -> String {
        rb_self.0.executable().to_owned()
    }

    fn executable_name(rb_self: &Self) -> String {
        rb_self.0.executable_name()
    }

    fn stdout(ruby: &Ruby, rb_self: &Self) -> Result<RString, Error> {
        utf8(ruby, rb_self.0.stdout())
    }

    fn stderr(ruby: &Ruby, rb_self: &Self) -> Result<RString, Error> {
        utf8(ruby, rb_self.0.stderr())
    }

    fn status(rb_self: &Self) -> i64 {
        rb_self.0.status()
    }

    fn started_at(ruby: &Ruby, rb_self: &Self) -> Result<RValue, Error> {
        time_value(ruby, rb_self.0.started_at())
    }

    fn finished_at(ruby: &Ruby, rb_self: &Self) -> Result<RValue, Error> {
        time_value(ruby, rb_self.0.finished_at())
    }

    fn duration(rb_self: &Self) -> Option<f64> {
        rb_self.0.duration()
    }

    fn execution_time(rb_self: &Self) -> Option<f64> {
        rb_self.0.execution_time()
    }

    fn is_success(rb_self: &Self) -> bool {
        rb_self.0.is_success()
    }

    fn is_failure(rb_self: &Self) -> bool {
        rb_self.0.is_failure()
    }

    fn output(ruby: &Ruby, rb_self: &Self) -> Result<RString, Error> {
        match rb_self.0.output() {
            Ok(text) => Ok(ruby.str_new(text)),
            Err(error) => Err(invalid_bytes(ruby, error)),
        }
    }

    fn error_output(ruby: &Ruby, rb_self: &Self) -> Result<RString, Error> {
        match rb_self.0.error_output() {
            Ok(text) => Ok(ruby.str_new(text)),
            Err(error) => Err(invalid_bytes(ruby, error)),
        }
    }

    /// `#to_json(*)`: the generator state argument the json gem passes is
    /// ignored; the core writes the JSON.
    fn to_json(ruby: &Ruby, rb_self: &Self, _args: &[RValue]) -> Result<RString, Error> {
        match rb_self.0.to_json() {
            Ok(json) => Ok(ruby.str_new(&json)),
            Err(error) => match super::errors::error_class(ruby, error.ruby_class()) {
                Ok(class) => Err(Error::new(class, error.message().to_owned())),
                Err(error) => Err(error),
            },
        }
    }
}
