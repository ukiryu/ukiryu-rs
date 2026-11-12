//! `Ukiryu::Validation` constraint classes.
//!
//! Source: `lib/ukiryu/validation/constraints.rb` (ukiryu/ukiryu ab6df353):
//! RequiredConstraint, RangeConstraint, EnumConstraint, TypeConstraint,
//! CardinalityConstraint, BooleanFlagConstraint and DependencyConstraint.
//! Failures are `ValidationIssue` errors carrying the attribute name and the
//! validation type symbol; other Ruby errors (failed comparisons, missing
//! methods) propagate unchanged, as in the source.
use super::error::{Error, ErrorClass, Result};
use super::ruby::{gt, join, lt, no_method, to_s};
use super::validate::{validate, Host};
use super::value::{eql, ruby_eq, Value};

/// `options_accessor` from the validation context: the current value of
/// another option by name.
pub type Accessor<'a> = &'a dyn Fn(&Value) -> Result<Value>;

/// One constraint, with the constructor arguments the Ruby class keeps.
#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    Required {
        attribute: Value,
        min: Value,
    },
    Range {
        attribute: Value,
        min: Value,
        max: Value,
    },
    Enum {
        attribute: Value,
        allowed_values: Value,
    },
    Type {
        attribute: Value,
        type_spec: Value,
        /// The `validation_options` Hash passed to `Type.validate`.
        options: Value,
    },
    Cardinality {
        attribute: Value,
        min: Value,
        max: Value,
    },
    BooleanFlag {
        attribute: Value,
    },
    Dependency {
        option: Value,
        requires: Value,
        conflicts: Value,
        implies: Value,
    },
}

fn is_empty_collection(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.is_empty())
}

/// Ruby `each` over a constructor argument that should be an Array.
fn each_item(v: &Value) -> Result<&[Value]> {
    match v {
        Value::Array(items) => Ok(items),
        Value::Hash(_) => Err(Error::unsupported("iterating a Hash here is not ported")),
        other => Err(no_method("each", other)),
    }
}

impl Constraint {
    /// The Ruby class name without the `Ukiryu::Validation::` prefix.
    pub fn class_name(&self) -> &'static str {
        match self {
            Constraint::Required { .. } => "RequiredConstraint",
            Constraint::Range { .. } => "RangeConstraint",
            Constraint::Enum { .. } => "EnumConstraint",
            Constraint::Type { .. } => "TypeConstraint",
            Constraint::Cardinality { .. } => "CardinalityConstraint",
            Constraint::BooleanFlag { .. } => "BooleanFlagConstraint",
            Constraint::Dependency { .. } => "DependencyConstraint",
        }
    }

    /// `attribute_name`, or `option_name` for a dependency.
    pub fn attribute(&self) -> &Value {
        match self {
            Constraint::Required { attribute, .. }
            | Constraint::Range { attribute, .. }
            | Constraint::Enum { attribute, .. }
            | Constraint::Type { attribute, .. }
            | Constraint::Cardinality { attribute, .. }
            | Constraint::BooleanFlag { attribute } => attribute,
            Constraint::Dependency { option, .. } => option,
        }
    }

    /// `Constraint#validate(value, context)`. `accessor` is
    /// `context[:options_accessor]`.
    pub fn validate(&self, value: &Value, accessor: Option<Accessor<'_>>, host: &dyn Host) -> Result<()> {
        match self {
            Constraint::Required { attribute, min } => {
                let zero = match min {
                    Value::Integer(i) => *i == num_bigint::BigInt::from(0),
                    Value::Float(f) => *f == 0.0,
                    other => return Err(no_method("zero?", other)),
                };
                if zero {
                    return Ok(());
                }
                let empty = match value {
                    Value::Nil => true,
                    Value::String(s) | Value::Symbol(s) => s.is_empty(),
                    Value::Array(a) => a.is_empty(),
                    Value::Hash(h) => h.is_empty(),
                    _ => false,
                };
                if empty {
                    return Err(Error::issue(
                        attribute.clone(),
                        "required",
                        format!("Attribute {} is required but missing", to_s(attribute)),
                    ));
                }
                Ok(())
            }
            Constraint::Range { attribute, min, max } => {
                if value.is_nil() {
                    return Ok(());
                }
                let check = match value {
                    Value::Array(a) => Value::int(a.len() as i64),
                    other => other.clone(),
                };
                if lt(&check, min)? || gt(&check, max)? {
                    return Err(Error::issue(
                        attribute.clone(),
                        "range",
                        format!(
                            "{} must be between {} and {}, got {}",
                            to_s(attribute),
                            to_s(min),
                            to_s(max),
                            to_s(&check)
                        ),
                    ));
                }
                Ok(())
            }
            Constraint::Enum {
                attribute,
                allowed_values,
            } => {
                if value.is_nil() {
                    return Ok(());
                }
                let Value::Array(allowed) = allowed_values else {
                    return Err(Error::unsupported(
                        "EnumConstraint with non-Array allowed_values is not ported",
                    ));
                };
                if let Value::Array(items) = value {
                    let invalid: Vec<Value> = items
                        .iter()
                        .filter(|v| !allowed.iter().any(|a| eql(a, v)))
                        .cloned()
                        .collect();
                    if !invalid.is_empty() {
                        return Err(Error::issue(
                            attribute.clone(),
                            "enum",
                            format!(
                                "{} contains invalid values: {}. Valid values: {}",
                                to_s(attribute),
                                join(&invalid, ", "),
                                join(allowed, ", ")
                            ),
                        ));
                    }
                } else if !allowed.iter().any(|a| ruby_eq(a, value)) {
                    return Err(Error::issue(
                        attribute.clone(),
                        "enum",
                        format!(
                            "{} must be one of {}, got {}",
                            to_s(attribute),
                            join(allowed, ", "),
                            to_s(value)
                        ),
                    ));
                }
                Ok(())
            }
            Constraint::Type {
                attribute,
                type_spec,
                options,
            } => {
                if value.is_nil() {
                    return Ok(());
                }
                match validate(value, type_spec, options, host) {
                    Ok(_) => Ok(()),
                    Err(e) if e.class == ErrorClass::Validation => Err(Error::issue(
                        attribute.clone(),
                        "type",
                        format!("{}: {}", to_s(attribute), e.message),
                    )),
                    Err(e) => Err(e),
                }
            }
            Constraint::Cardinality { attribute, max, .. } => {
                let Value::Array(items) = value else {
                    return Ok(());
                };
                let unbounded = matches!(max, Value::Float(f) if *f == f64::INFINITY);
                let size = Value::int(items.len() as i64);
                if !unbounded && gt(&size, max)? {
                    return Err(Error::issue(
                        attribute.clone(),
                        "cardinality",
                        format!(
                            "Too many values for {}: got {}, max {}",
                            to_s(attribute),
                            items.len(),
                            to_s(max)
                        ),
                    ));
                }
                Ok(())
            }
            Constraint::BooleanFlag { attribute } => match value {
                Value::Nil | Value::Bool(_) => Ok(()),
                other => Err(Error::issue(
                    attribute.clone(),
                    "boolean",
                    format!(
                        "Flag {} must be boolean, got {}: {}",
                        to_s(attribute),
                        other.class_name(),
                        to_s(other)
                    ),
                )),
            },
            Constraint::Dependency {
                option,
                requires,
                conflicts,
                implies,
            } => {
                let Some(accessor) = accessor else {
                    return Err(Error::new(
                        ErrorClass::Argument,
                        "Dependency validation requires :options_accessor",
                    ));
                };
                let issue = |message: String| Error::issue(option.clone(), "dependency", message);
                let name = to_s(option);
                if requires.truthy() {
                    for required in each_item(requires)? {
                        let v = accessor(required)?;
                        if v.is_nil() || is_empty_collection(&v) {
                            return Err(issue(format!(
                                "Option {name} requires {} to be set",
                                to_s(required)
                            )));
                        }
                    }
                }
                if conflicts.truthy() {
                    for conflict in each_item(conflicts)? {
                        let v = accessor(conflict)?;
                        if v.truthy() && !is_empty_collection(&v) {
                            return Err(issue(format!(
                                "Option {name} conflicts with {}",
                                to_s(conflict)
                            )));
                        }
                    }
                }
                if implies.truthy() {
                    let Value::Hash(pairs) = implies else {
                        return Err(Error::unsupported("implies given as a non-Hash is not ported"));
                    };
                    for (implied, definition) in pairs {
                        if !matches!(definition, Value::Hash(_)) {
                            return Err(Error::unsupported(
                                "implies definition given as a non-Hash is not ported",
                            ));
                        }
                        let current = accessor(implied)?;
                        let expected = definition.sym_get("value");
                        let present = definition.sym_get("present");
                        if present.truthy() && (current.is_nil() || is_empty_collection(&current)) {
                            return Err(issue(format!(
                                "Option {name} implies {} should be set",
                                to_s(implied)
                            )));
                        } else if !present.truthy() && !current.is_nil() {
                            return Err(issue(format!(
                                "Option {name} implies {} should not be set",
                                to_s(implied)
                            )));
                        } else if expected.truthy() && !ruby_eq(&current, &expected) {
                            return Err(issue(format!(
                                "Option {name} implies {} should be {}, got {}",
                                to_s(implied),
                                to_s(&expected),
                                to_s(&current)
                            )));
                        }
                    }
                }
                Ok(())
            }
        }
    }
}
