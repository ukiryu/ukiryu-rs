//! Exception classes: the `Ukiryu::Errors` taxonomy from
//! [`crate::errors::ErrorKind`], and the binding's own failure classes.
use crate::errors::ErrorKind;
use crate::models::{ModelError, ModelErrorKind};
use magnus::{
    method, prelude::*, Error, ExceptionClass, RArray, RClass, RModule, RString, Ruby,
    Value as RValue,
};

/// Defines `Ukiryu::Errors::*` with the core's names and parents, plus the
/// `#suggestions` the Ruby classes define. An already defined class with the
/// same superclass is reused.
pub(super) fn define(ruby: &Ruby, ukiryu: RModule) -> Result<(), Error> {
    let module = ukiryu.define_module("Errors")?;
    for kind in ErrorKind::ALL {
        let superclass = match kind.parent() {
            Some(parent) => error_class(ruby, parent.ruby_class())?,
            None => ruby.exception_standard_error(),
        };
        module.define_error(short_name(kind.ruby_class()), superclass)?;
    }
    let base = error_class(ruby, ErrorKind::Error.ruby_class())?;
    base.define_method("suggestions", method!(suggestions, 0))?;
    Ok(())
}

/// `Ukiryu::Native::ModelError` (with `#kind` and `#source_class`) for core
/// model failures, and `Ukiryu::Native::WireError` (with `#code`) for wire
/// replies without a Ruby class.
pub(super) fn define_native(ruby: &Ruby, native: RModule) -> Result<(), Error> {
    let model_error = native.define_error("ModelError", ruby.exception_standard_error())?;
    model_error.funcall::<_, _, RValue>(
        "attr_reader",
        (ruby.to_symbol("kind"), ruby.to_symbol("source_class")),
    )?;
    let wire_error = native.define_error("WireError", ruby.exception_standard_error())?;
    wire_error.funcall::<_, _, RValue>("attr_reader", (ruby.to_symbol("code"),))?;
    Ok(())
}

fn short_name(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

/// Looks a class up by its full constant path (classes are shareable
/// constants, so this is valid in any Ractor).
pub(super) fn error_class(ruby: &Ruby, path: &str) -> Result<ExceptionClass, Error> {
    ruby.class_object().funcall("const_get", (path,))
}

/// `Ukiryu::Errors::Error#suggestions`: the nearest ancestor class the core
/// knows decides the list, as Ruby method inheritance would.
fn suggestions(ruby: &Ruby, rb_self: RValue) -> Result<RArray, Error> {
    let platform: RString = ruby.class_object().const_get("RUBY_PLATFORM")?;
    let platform = super::values::text(ruby, platform)?;
    let ancestors: RArray = rb_self.class().funcall("ancestors", ())?;
    for index in 0..ancestors.len() {
        let ancestor: RValue = ancestors.entry(index as isize)?;
        let Some(class) = RClass::from_value(ancestor) else {
            continue;
        };
        let name: Option<String> = class.funcall("name", ())?;
        if let Some(kind) = name.as_deref().and_then(ErrorKind::from_ruby_class) {
            let lines = kind.suggestions(&platform);
            let out = ruby.ary_new_capa(lines.len());
            for line in lines {
                out.push(ruby.str_new(&line))?;
            }
            return Ok(out);
        }
    }
    Ok(ruby.ary_new())
}

fn model_kind_name(kind: ModelErrorKind) -> &'static str {
    match kind {
        ModelErrorKind::InvalidFormat => "invalid_format",
        ModelErrorKind::IncorrectModel => "incorrect_model",
        ModelErrorKind::NoMethod => "no_method",
        ModelErrorKind::Syntax => "syntax",
        ModelErrorKind::Io => "io",
        ModelErrorKind::Unsupported => "unsupported",
        ModelErrorKind::DisallowedClass => "disallowed_class",
        ModelErrorKind::AliasesNotEnabled => "aliases_not_enabled",
        ModelErrorKind::Argument => "argument",
        ModelErrorKind::Type => "type",
    }
}

/// Raises a core [`ModelError`] as `Ukiryu::Native::ModelError`; `#kind` is
/// the core kind and `#source_class` the class the Ruby source raised.
pub(super) fn model_error(ruby: &Ruby, error: ModelError) -> Error {
    build(
        ruby,
        "Ukiryu::Native::ModelError",
        &error.message,
        &[
            (
                "@kind",
                ruby.to_symbol(model_kind_name(error.kind)).as_value(),
            ),
            ("@source_class", ruby.str_new(error.ruby_class()).as_value()),
        ],
    )
}

/// Raises a wire failure reply as the Ruby class it names when that is
/// `ArgumentError` or a `Ukiryu::Errors` class, else as
/// `Ukiryu::Native::WireError`.
pub(super) fn wire_error(ruby: &Ruby, error: &serde_json::Value) -> Error {
    let code = error["code"].as_str().unwrap_or("INTERNAL_ERROR");
    let message = error["message"].as_str().unwrap_or("");
    match error["source_class"].as_str() {
        Some("ArgumentError") => Error::new(ruby.exception_arg_error(), message.to_owned()),
        Some(class) if ErrorKind::from_ruby_class(class).is_some() => {
            match error_class(ruby, class) {
                Ok(class) => Error::new(class, message.to_owned()),
                Err(error) => error,
            }
        }
        _ => build(
            ruby,
            "Ukiryu::Native::WireError",
            message,
            &[("@code", ruby.str_new(code).as_value())],
        ),
    }
}

fn build(ruby: &Ruby, class: &str, message: &str, ivars: &[(&str, RValue)]) -> Error {
    let made = (|| -> Result<Error, Error> {
        let class = error_class(ruby, class)?;
        let exception: RValue = class.funcall("new", (message,))?;
        for (name, value) in ivars {
            exception.funcall::<_, _, RValue>("instance_variable_set", (*name, *value))?;
        }
        Ok(Error::from(
            magnus::Exception::from_value(exception)
                .ok_or_else(|| Error::new(ruby.exception_runtime_error(), "not an exception"))?,
        ))
    })();
    made.unwrap_or_else(|error| error)
}
