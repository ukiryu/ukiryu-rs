//! `Ukiryu::Validation::Validator`.
//!
//! Source: `lib/ukiryu/validation/validator.rb` (ukiryu/ukiryu ab6df353).
//! The Ruby validator reads a command definition through `Hash#[]` with
//! symbol keys and reads option values with `instance_variable_get`. Both are
//! kept: the definition is a Hash [`Value`] and the options object is an
//! [`OptionsObject`]. A `Models::CommandDefinition` does not respond to `#[]`
//! in the pinned source, so the validator does not accept models.
//!
//! Note: the pinned gem never loads `validation/constraints.rb` or
//! `validation/validator.rb` (`lib/ukiryu/validation.rb` is an empty module),
//! so `Options::Base#validate!` raises NameError there. This port reproduces
//! the classes themselves, as captured with those files loaded explicitly.
use std::collections::BTreeMap;

use super::constraints::Constraint;
use super::error::{Error, ErrorClass, Result};
use super::ruby::{no_method, to_s};
use super::validate::{Host, StdHost};
use super::value::Value;

/// The options object whose instance variables hold the option values.
pub trait OptionsObject {
    /// The value of `@name` (`nil` when unset). `name` is a valid identifier.
    fn instance_variable(&self, name: &str) -> Value;
}

impl OptionsObject for BTreeMap<String, Value> {
    fn instance_variable(&self, name: &str) -> Value {
        self.get(name).cloned().unwrap_or(Value::Nil)
    }
}

impl OptionsObject for Vec<(String, Value)> {
    fn instance_variable(&self, name: &str) -> Value {
        self.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Nil)
    }
}

/// `instance_variable_get("@#{name}")`, including its NameError.
fn ivar(options: &dyn OptionsObject, name: &Value) -> Result<Value> {
    let name = to_s(name);
    let mut chars = name.chars();
    let valid = match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() || !c.is_ascii() => {
            chars.all(|c| c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii())
        }
        _ => false,
    };
    if !valid {
        return Err(Error::new(
            ErrorClass::Name,
            format!("'@{name}' is not allowed as an instance variable name"),
        ));
    }
    Ok(options.instance_variable(&name))
}

/// `defs.each` for a definition list (`nil`/`false` means none).
fn definitions(v: &Value) -> Result<&[Value]> {
    match v {
        Value::Nil | Value::Bool(false) => Ok(&[]),
        Value::Array(items) => Ok(items),
        Value::Hash(_) => Err(Error::unsupported(
            "a Hash where the validator expects a definition list is not ported",
        )),
        other => Err(no_method("each", other)),
    }
}

fn hash_def(v: &Value) -> Result<&Value> {
    match v {
        Value::Hash(_) => Ok(v),
        Value::Nil => Err(no_method("[]", v)),
        _ => Err(Error::unsupported(
            "a non-Hash definition entry is not ported",
        )),
    }
}

fn push(pairs: &mut Vec<(Value, Value)>, key: &str, value: Value) {
    pairs.push((Value::sym(key), value));
}

/// `Validator#build_type_constraint`.
fn type_constraint(attribute: Value, defn: &Value) -> Constraint {
    let mut opts = Vec::new();
    if defn.sym_key("must_exist") {
        push(&mut opts, "require_existing", defn.sym_get("must_exist"));
    }
    if defn.sym_key("allow_empty") {
        push(&mut opts, "allow_empty", defn.sym_get("allow_empty"));
    }
    for key in ["pattern", "range", "min", "max", "values"] {
        let v = defn.sym_get(key);
        if v.truthy() {
            push(&mut opts, key, v);
        }
    }
    let type_spec = defn.sym_get("type");
    if let Value::Hash(_) = type_spec {
        let name = type_spec.sym_get("name");
        let of = type_spec.sym_get("of");
        if name == Value::sym("array") && of.truthy() {
            push(&mut opts, "of", of);
        }
        let keys = type_spec.sym_get("keys");
        if name == Value::sym("hash") && keys.truthy() {
            push(&mut opts, "keys", keys);
        }
    }
    Constraint::Type {
        attribute,
        type_spec,
        options: Value::Hash(opts),
    }
}

/// `min, max = range`.
fn bounds(range: &Value) -> (Value, Value) {
    match range {
        Value::Array(items) => (
            items.first().cloned().unwrap_or(Value::Nil),
            items.get(1).cloned().unwrap_or(Value::Nil),
        ),
        other => (other.clone(), Value::Nil),
    }
}

/// `Validator#build_constraints`.
pub fn build_constraints(command_def: &Value) -> Result<Vec<Constraint>> {
    if !matches!(command_def, Value::Hash(_)) {
        return Err(Error::unsupported(
            "a non-Hash command definition is not ported",
        ));
    }
    let mut out = Vec::new();
    for arg in definitions(&command_def.sym_get("arguments"))? {
        let arg = hash_def(arg)?;
        let name = arg.sym_get("name");
        let min = match arg.sym_get("min") {
            m if m.truthy() => m,
            _ => Value::int(1),
        };
        out.push(Constraint::Required {
            attribute: name.clone(),
            min: min.clone(),
        });
        if arg.sym_get("type").truthy() {
            out.push(type_constraint(name.clone(), arg));
        }
        let max = arg.sym_get("max");
        if arg.sym_get("variadic").truthy() && max.truthy() {
            out.push(Constraint::Cardinality {
                attribute: name,
                min,
                max,
            });
        }
    }
    for key in ["options", "post_options"] {
        for opt in definitions(&command_def.sym_get(key))? {
            let opt = hash_def(opt)?;
            let name = opt.sym_get("name");
            if opt.sym_get("type").truthy() {
                out.push(type_constraint(name.clone(), opt));
            }
            let range = opt.sym_get("range");
            if range.truthy() {
                let (min, max) = bounds(&range);
                out.push(Constraint::Range {
                    attribute: name.clone(),
                    min,
                    max,
                });
            }
            let values = opt.sym_get("values");
            if values.truthy() {
                out.push(Constraint::Enum {
                    attribute: name,
                    allowed_values: values,
                });
            }
        }
    }
    for flag in definitions(&command_def.sym_get("flags"))? {
        out.push(Constraint::BooleanFlag {
            attribute: hash_def(flag)?.sym_get("name"),
        });
    }
    for dep in definitions(&command_def.sym_get("dependencies"))? {
        let dep = hash_def(dep)?;
        out.push(Constraint::Dependency {
            option: dep.sym_get("option"),
            requires: dep.sym_get("requires"),
            conflicts: dep.sym_get("conflicts"),
            implies: dep.sym_get("implies"),
        });
    }
    Ok(out)
}

/// `Ukiryu::Validation::Validator.new(options, command_def)`.
pub struct Validator<'a> {
    options: &'a dyn OptionsObject,
    constraints: Vec<Constraint>,
    host: &'a dyn Host,
}

impl<'a> Validator<'a> {
    /// Builds the constraints with the process host ([`StdHost`]).
    pub fn new(options: &'a dyn OptionsObject, command_def: &Value) -> Result<Self> {
        Self::with_host(options, command_def, &StdHost)
    }

    pub fn with_host(
        options: &'a dyn OptionsObject,
        command_def: &Value,
        host: &'a dyn Host,
    ) -> Result<Self> {
        Ok(Validator {
            options,
            constraints: build_constraints(command_def)?,
            host,
        })
    }

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    fn check(&self, constraint: &Constraint) -> Result<()> {
        let value = ivar(self.options, constraint.attribute())?;
        let accessor = |name: &Value| ivar(self.options, name);
        constraint.validate(&value, Some(&accessor), self.host)
    }

    /// `validate!`: `Ok(true)`, or the first issue as
    /// `Ukiryu::Errors::ValidationError`; other errors propagate.
    pub fn validate(&self) -> Result<bool> {
        for c in &self.constraints {
            match self.check(c) {
                Ok(()) => {}
                Err(e) if e.class == ErrorClass::ValidationIssue => {
                    return Err(Error::validation(e.message))
                }
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// `valid?`.
    pub fn is_valid(&self) -> Result<bool> {
        match self.validate() {
            Ok(_) => Ok(true),
            Err(e) if e.class == ErrorClass::Validation => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `errors`: every issue message, in constraint order.
    pub fn errors(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for c in &self.constraints {
            match self.check(c) {
                Ok(()) => {}
                Err(e) if e.class == ErrorClass::ValidationIssue => out.push(e.message),
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
}
