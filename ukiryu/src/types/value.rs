//! The Ruby value domain that `Ukiryu::Type` and `Ukiryu::Validation` operate on.
//!
//! Source: `lib/ukiryu/type.rb` and `lib/ukiryu/validation/*.rb` at
//! ukiryu/ukiryu ab6df353. Those modules receive arbitrary Ruby objects (option
//! values, Hash option bags, Hash command definitions) and return coerced Ruby
//! objects, so the port keeps the exact runtime type of every value: an
//! Integer is never a Float, a Symbol is never a String, and Hash keys keep
//! their type and insertion order.
//!
//! This is a value representation, not a model hierarchy: definition models
//! live in the models module and reach this code through [`OptionSource`].
use num_bigint::BigInt;

/// A Ruby `Time`: an instant plus the UTC offset it is displayed in.
///
/// `seconds` is floored Unix time, `nanos` the non-negative sub-second part.
/// `utc` distinguishes a UTC time (`2024-01-17 10:20:30 UTC`) from a local
/// time whose offset happens to be zero (`2024-01-17 10:20:30 +0000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Time {
    pub seconds: i64,
    pub nanos: u32,
    pub utc_offset: i32,
    pub utc: bool,
}

/// A Ruby `Regexp` literal: its source and option bits
/// (`1` IGNORECASE, `2` EXTENDED, `4` MULTILINE, as `Regexp#options`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regexp {
    pub source: String,
    pub options: u32,
}

impl Regexp {
    pub const IGNORECASE: u32 = 1;
    pub const EXTENDED: u32 = 2;
    pub const MULTILINE: u32 = 4;
}

/// A Ruby object as seen by type validation.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Integer(BigInt),
    Float(f64),
    String(String),
    Symbol(String),
    Time(Time),
    Regexp(Regexp),
    Array(Vec<Value>),
    /// Insertion-ordered pairs; keys are unique under Ruby `eql?`.
    Hash(Vec<(Value, Value)>),
}

impl Value {
    pub fn int(i: i64) -> Self {
        Value::Integer(BigInt::from(i))
    }

    pub fn str(s: impl Into<String>) -> Self {
        Value::String(s.into())
    }

    pub fn sym(s: impl Into<String>) -> Self {
        Value::Symbol(s.into())
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Ruby truthiness: everything except `nil` and `false`.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    /// `Object#class.name`.
    pub fn class_name(&self) -> &'static str {
        match self {
            Value::Nil => "NilClass",
            Value::Bool(true) => "TrueClass",
            Value::Bool(false) => "FalseClass",
            Value::Integer(_) => "Integer",
            Value::Float(_) => "Float",
            Value::String(_) => "String",
            Value::Symbol(_) => "Symbol",
            Value::Time(_) => "Time",
            Value::Regexp(_) => "Regexp",
            Value::Array(_) => "Array",
            Value::Hash(_) => "Hash",
        }
    }

    /// Ruby `Hash#[]` for a Hash value (`eql?` key lookup); `nil` otherwise absent.
    pub fn hash_get(&self, key: &Value) -> Option<&Value> {
        match self {
            Value::Hash(pairs) => pairs.iter().find(|(k, _)| eql(k, key)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `Hash#[]` with a Symbol key, `nil` when absent.
    pub fn sym_get(&self, key: &str) -> Value {
        self.hash_get(&Value::sym(key))
            .cloned()
            .unwrap_or(Value::Nil)
    }

    /// `Hash#key?` with a Symbol key.
    pub fn sym_key(&self, key: &str) -> bool {
        self.hash_get(&Value::sym(key)).is_some()
    }

    /// Whether Ruby stores this value as an immediate ("special constant"):
    /// nil, true, false, Symbols, Floats and Integers in the Fixnum range.
    /// Ruby's comparison error messages inspect these and name the class of
    /// anything else.
    pub(crate) fn is_special_const(&self) -> bool {
        match self {
            Value::Nil | Value::Bool(_) | Value::Symbol(_) | Value::Float(_) => true,
            Value::Integer(i) => {
                let min = -(BigInt::from(1) << 62u32);
                let max = (BigInt::from(1) << 62u32) - 1;
                *i >= min && *i <= max
            }
            _ => false,
        }
    }
}

/// Ruby `eql?` (also the identity Hash keys and `Array#-` use): no numeric
/// conversion, `1.eql?(1.0)` is false.
pub fn eql(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Integer(x), Value::Integer(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Symbol(x), Value::Symbol(y)) => x == y,
        (Value::Time(x), Value::Time(y)) => x.seconds == y.seconds && x.nanos == y.nanos,
        (Value::Regexp(x), Value::Regexp(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eql(p, q))
        }
        (Value::Hash(x), Value::Hash(y)) => hash_contents_equal(x, y, eql),
        _ => false,
    }
}

/// Ruby `==`: numeric values compare across Integer and Float.
pub fn ruby_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Integer(_) | Value::Float(_), Value::Integer(_) | Value::Float(_)) => {
            crate::types::ruby::numeric_cmp(a, b) == Some(std::cmp::Ordering::Equal)
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| ruby_eq(p, q))
        }
        (Value::Hash(x), Value::Hash(y)) => hash_contents_equal(x, y, ruby_eq),
        _ => eql(a, b),
    }
}

fn hash_contents_equal(
    x: &[(Value, Value)],
    y: &[(Value, Value)],
    value_eq: fn(&Value, &Value) -> bool,
) -> bool {
    x.len() == y.len()
        && x.iter().all(|(k, v)| {
            y.iter()
                .find(|(k2, _)| eql(k, k2))
                .is_some_and(|(_, v2)| value_eq(v, v2))
        })
}

/// Reads one option the way `Ukiryu::Type` does: `options[:key]`.
///
/// Ruby passes either a Hash (symbol keys) or a definition model whose `#[]`
/// returns the attribute named by the key and `nil` for anything else. Both
/// collapse "absent" and `nil`; `Type` only ever tests truthiness or reads the
/// value.
pub trait OptionSource {
    fn option(&self, key: &str) -> Value;
}

impl OptionSource for Value {
    fn option(&self, key: &str) -> Value {
        self.sym_get(key)
    }
}

/// No options at all (`{}`).
impl OptionSource for () {
    fn option(&self, _key: &str) -> Value {
        Value::Nil
    }
}
