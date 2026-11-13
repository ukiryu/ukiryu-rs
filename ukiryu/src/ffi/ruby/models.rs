//! Native-backed models: `Ukiryu::Native::Models::<Name>` for every core
//! serde model ([`crate::models::Model`]).
//!
//! Each Ruby object owns one core model value. `from_yaml`/`to_yaml` go
//! through the core serde + Yeptris path, `to_json` through serde_json and
//! `from_hash`/`to_hash` through the core value type; the binding adds no
//! mapping of its own. All classes share the `Ukiryu::Native::Model` base,
//! whose singleton methods resolve the concrete model from the receiver.
use super::errors::model_error;
use super::values::{from_ruby, text, to_ruby};
use crate::models::{self, Model, ModelError};
use magnus::{
    method, prelude::*, typed_data::Obj, Error, RClass, RModule, RString, Ruby, Value as RValue,
};

macro_rules! native_models {
    ($($name:ident),+ $(,)?) => {
        /// One core model value of any bound model class.
        #[derive(Clone, Debug, PartialEq)]
        enum AnyModel {
            $($name(Box<models::$name>)),+
        }

        const NAMES: &[&str] = &[$(stringify!($name)),+];

        impl AnyModel {
            fn new(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($name) => Some(AnyModel::$name(Box::default())),)+
                    _ => None,
                }
            }

            fn from_yaml(name: &str, yaml: &str) -> Option<Result<Self, ModelError>> {
                match name {
                    $(stringify!($name) => Some(
                        <models::$name as Model>::from_yaml(yaml).map(|m| AnyModel::$name(Box::new(m))),
                    ),)+
                    _ => None,
                }
            }

            fn from_value(
                name: &str,
                value: &crate::types::Value,
            ) -> Option<Result<Self, ModelError>> {
                match name {
                    $(stringify!($name) => Some(
                        <models::$name as Model>::from_ruby(value).map(|m| AnyModel::$name(Box::new(m))),
                    ),)+
                    _ => None,
                }
            }

            fn to_yaml(&self) -> Result<String, ModelError> {
                match self {
                    $(AnyModel::$name(m) => m.to_yaml(),)+
                }
            }

            fn to_json(&self) -> Result<String, ModelError> {
                match self {
                    $(AnyModel::$name(m) => m.to_json(),)+
                }
            }

            fn to_hash(&self) -> Result<crate::types::Value, ModelError> {
                match self {
                    $(AnyModel::$name(m) => m.to_hash(),)+
                }
            }
        }
    };
}

native_models!(
    Argument,
    Arguments,
    CommandInfo,
    Components,
    CommandDefinition,
    ArgumentDefinition,
    OptionDefinition,
    FlagDefinition,
    EnvVarDefinition,
    ExitCodes,
    RunEnvironment,
    StageMetrics,
    ExecutionReport,
    ExecutionMetadata,
    OutputInfo,
    ErrorResponse,
    SuccessResponse,
    Invocation,
    VersionDetectionMethod,
    VersionDetection,
    ToolMetadata,
    PlatformProfile,
    ToolDefinition,
);

const MODELS_PREFIX: &str = "Ukiryu::Native::Models::";

/// The Ruby object: a core model value, freed with its Ruby object.
#[magnus::wrap(class = "Ukiryu::Native::Model", free_immediately, size)]
struct NativeModel(AnyModel);

// SAFETY: magnus requires `Send` for wrapped data. `PlatformProfile` (and the
// models containing it) caches its routing tree in `Rc<RefCell<_>>`
// (`models::routing::Routing`), so the derived bound fails. The Rc graph is
// owned by this one object: no method hands a clone of the model, or of any
// Rc inside it, to another Ruby object or to Rust state. A T_DATA object
// without an allocator or shareable flag cannot be copied, moved or shared
// to another Ractor, and Ruby threads of one Ractor run only while holding
// its lock, so the reference counts are never touched concurrently; the GC
// frees the object while holding that lock too.
unsafe impl Send for NativeModel {}

pub(super) fn define(ruby: &Ruby, native: RModule) -> Result<(), Error> {
    let base = native.define_class("Model", ruby.class_object())?;
    base.undef_default_alloc_func();
    base.define_singleton_method("new", method!(new, 0))?;
    base.define_singleton_method("from_yaml", method!(from_yaml, 1))?;
    base.define_singleton_method("from_hash", method!(from_hash, 1))?;
    base.define_method("to_yaml", method!(NativeModel::to_yaml, 0))?;
    base.define_method("to_json", method!(NativeModel::to_json, 0))?;
    base.define_method("to_hash", method!(NativeModel::to_hash, 0))?;
    base.define_method("==", method!(NativeModel::equal, 1))?;
    let module = native.define_module("Models")?;
    for name in NAMES {
        let class = module.define_class(*name, base)?;
        class.undef_default_alloc_func();
    }
    Ok(())
}

/// The bound model a receiver class stands for: itself or its nearest
/// `Ukiryu::Native::Models::*` superclass.
fn model_name(ruby: &Ruby, class: RClass) -> Result<String, Error> {
    let mut current = Some(class);
    while let Some(class) = current {
        let name: Option<String> = class.funcall("name", ())?;
        if let Some(short) = name.as_deref().and_then(|n| n.strip_prefix(MODELS_PREFIX)) {
            if NAMES.contains(&short) {
                return Ok(short.to_owned());
            }
        }
        // `Class#superclass` is nil above BasicObject.
        current = class.funcall("superclass", ())?;
    }
    Err(Error::new(
        ruby.exception_not_imp_error(),
        "call this on a Ukiryu::Native::Models class",
    ))
}

fn wrap(ruby: &Ruby, class: RClass, model: AnyModel) -> Obj<NativeModel> {
    ruby.obj_wrap_as(NativeModel(model), class)
}

/// `Model.new`: the model with its attribute defaults.
fn new(ruby: &Ruby, rb_self: RClass) -> Result<Obj<NativeModel>, Error> {
    let name = model_name(ruby, rb_self)?;
    let model = AnyModel::new(&name).expect("bound model name");
    Ok(wrap(ruby, rb_self, model))
}

/// `Model.from_yaml(yaml)` through the core serde + Yeptris reader.
fn from_yaml(ruby: &Ruby, rb_self: RClass, yaml: RString) -> Result<Obj<NativeModel>, Error> {
    let name = model_name(ruby, rb_self)?;
    let yaml = text(ruby, yaml)?;
    let model = AnyModel::from_yaml(&name, &yaml)
        .expect("bound model name")
        .map_err(|e| model_error(ruby, e))?;
    Ok(wrap(ruby, rb_self, model))
}

/// `Model.from_hash(hash)` (String and Symbol keys alike).
fn from_hash(ruby: &Ruby, rb_self: RClass, hash: RValue) -> Result<Obj<NativeModel>, Error> {
    let name = model_name(ruby, rb_self)?;
    let value = from_ruby(ruby, hash)?;
    let model = AnyModel::from_value(&name, &value)
        .expect("bound model name")
        .map_err(|e| model_error(ruby, e))?;
    Ok(wrap(ruby, rb_self, model))
}

impl NativeModel {
    fn to_yaml(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        rb_self.0.to_yaml().map_err(|e| model_error(ruby, e))
    }

    fn to_json(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        rb_self.0.to_json().map_err(|e| model_error(ruby, e))
    }

    fn to_hash(ruby: &Ruby, rb_self: &Self) -> Result<RValue, Error> {
        let value = rb_self.0.to_hash().map_err(|e| model_error(ruby, e))?;
        to_ruby(ruby, &value)
    }

    /// Same model class and equal attribute values.
    fn equal(rb_self: &Self, other: RValue) -> bool {
        match <&NativeModel>::try_convert(other) {
            Ok(other) => rb_self.0 == other.0,
            Err(_) => false,
        }
    }
}
