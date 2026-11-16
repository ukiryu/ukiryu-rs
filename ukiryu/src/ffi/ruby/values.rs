//! Ruby objects <-> the core's dynamic value type, [`crate::types::Value`].
//!
//! The mapping is exact in both directions for the classes the core value
//! models: `nil`, `true`, `false`, `Integer` (any size), `Float` (including
//! NaN, infinities and -0.0), `String` (UTF-8 or US-ASCII text), `Symbol`,
//! `Time` (instant to the nanosecond, UTC offset and the UTC flag), `Regexp`
//! (source plus the IGNORECASE/EXTENDED/MULTILINE bits), `Array` and `Hash`
//! (insertion order kept, keys of any supported class).
//!
//! Anything the core value cannot hold exactly raises instead of being
//! coerced: other classes (including subclasses of the classes above, whose
//! class would be lost), binary or other non-UTF-8 strings, invalid UTF-8,
//! sub-nanosecond `Time` precision (in either direction), Regexp option bits other than the three
//! above, `compare_by_identity` hashes, recursive structures and nesting
//! deeper than [`MAX_DEPTH`].
//!
//! Known representation limits, kept visible here rather than hidden: a
//! US-ASCII string comes back as UTF-8 (same characters, `==` holds); a local
//! `Time` comes back as a fixed-offset `Time` with the same instant and UTC
//! offset (the core value has no zone name); a Hash's default value or proc
//! is not data and is not carried.
use crate::types::{Regexp, Time, Value};
use magnus::{
    error::IntoError,
    prelude::*,
    r_hash::ForEach,
    r_regexp::Opts,
    rb_sys::AsRawValue,
    time::{Offset, Timespec},
    Error, Float, Integer, IntoValue, RArray, RClass, RHash, RRegexp, RString, Ruby, Symbol,
    Value as RValue,
};
use num_bigint::BigInt;

/// Deepest Array/Hash nesting converted in either direction. Conversion is
/// recursive and runs on Ruby thread stacks, so depth is bounded explicitly.
pub const MAX_DEPTH: usize = 512;

const KEPT_REGEXP_BITS: i32 = 1 | 2 | 4;
const FIXEDENCODING: i32 = 16;

/// Converts a Ruby object to a core value.
pub fn from_ruby(ruby: &Ruby, value: RValue) -> Result<Value, Error> {
    let mut open = Vec::new();
    convert_in(ruby, value, &mut open)
}

/// Builds fresh Ruby objects for a core value.
pub fn to_ruby(ruby: &Ruby, value: &Value) -> Result<RValue, Error> {
    convert_out(ruby, value, 0)
}

fn is_class(value: RValue, class: RClass) -> bool {
    value.class().as_raw() == class.as_raw()
}

fn type_error(ruby: &Ruby, value: RValue) -> Error {
    Error::new(
        ruby.exception_type_error(),
        format!(
            "cannot convert an instance of {} to a core value",
            value.class().inspect()
        ),
    )
}

fn arg_error(ruby: &Ruby, message: impl Into<String>) -> Error {
    Error::new(ruby.exception_arg_error(), message.into())
}

fn convert_in(ruby: &Ruby, value: RValue, open: &mut Vec<RValue>) -> Result<Value, Error> {
    if value.is_nil() {
        return Ok(Value::Nil);
    }
    if is_class(value, ruby.class_true_class()) {
        return Ok(Value::Bool(true));
    }
    if is_class(value, ruby.class_false_class()) {
        return Ok(Value::Bool(false));
    }
    if is_class(value, ruby.class_integer()) {
        let integer = Integer::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        return Ok(Value::Integer(match integer.to_i64() {
            Ok(n) => BigInt::from(n),
            Err(_) => {
                let text: String = integer.funcall("to_s", ())?;
                text.parse::<BigInt>()
                    .map_err(|e| arg_error(ruby, format!("unreadable Integer {text}: {e}")))?
            }
        }));
    }
    if is_class(value, ruby.class_float()) {
        let float = Float::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        return Ok(Value::Float(float.to_f64()));
    }
    if is_class(value, ruby.class_symbol()) {
        let symbol = Symbol::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        return Ok(Value::Symbol(symbol.name()?.into_owned()));
    }
    if is_class(value, ruby.class_string()) {
        let string = RString::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        return Ok(Value::String(text(ruby, string)?));
    }
    if is_class(value, ruby.class_time()) {
        let time = magnus::Time::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        return time_in(ruby, time);
    }
    if is_class(value, ruby.class_regexp()) {
        let regexp = RRegexp::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        return regexp_in(ruby, regexp);
    }
    if is_class(value, ruby.class_array()) {
        let array = RArray::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        enter(ruby, value, open)?;
        let mut items = Vec::with_capacity(array.len());
        for index in 0..array.len() {
            let item: RValue = array.entry(index as isize)?;
            items.push(convert_in(ruby, item, open)?);
        }
        open.pop();
        return Ok(Value::Array(items));
    }
    if is_class(value, ruby.class_hash()) {
        let hash = RHash::from_value(value).ok_or_else(|| type_error(ruby, value))?;
        let by_identity: bool = hash.funcall("compare_by_identity?", ())?;
        if by_identity {
            return Err(arg_error(
                ruby,
                "a compare_by_identity Hash has no core value (keys are compared with eql?)",
            ));
        }
        enter(ruby, value, open)?;
        let mut pairs = Vec::with_capacity(hash.len());
        hash.foreach(|key: RValue, item: RValue| {
            pairs.push((convert_in(ruby, key, open)?, convert_in(ruby, item, open)?));
            Ok(ForEach::Continue)
        })?;
        open.pop();
        // Keys arrive unique under the Hash's own eql?, so the pairs are kept
        // verbatim (no re-insertion that could merge or reorder them).
        return Ok(Value::Hash(pairs));
    }
    Err(type_error(ruby, value))
}

/// Records an Array/Hash being converted; refuses cycles and deep nesting.
fn enter(ruby: &Ruby, value: RValue, open: &mut Vec<RValue>) -> Result<(), Error> {
    if open.iter().any(|seen| seen.as_raw() == value.as_raw()) {
        return Err(arg_error(
            ruby,
            "a recursive Array or Hash has no core value",
        ));
    }
    if open.len() >= MAX_DEPTH {
        return Err(arg_error(
            ruby,
            format!("nesting deeper than {MAX_DEPTH} levels is not converted"),
        ));
    }
    open.push(value);
    Ok(())
}

/// The text of a UTF-8 or US-ASCII string with valid bytes.
pub(super) fn text(ruby: &Ruby, string: RString) -> Result<String, Error> {
    if !string.is_utf8_compatible_encoding() {
        let encoding: RValue = string.funcall("encoding", ())?;
        let name: String = encoding.funcall("to_s", ())?;
        return Err(Error::new(
            ruby.exception_encoding_error(),
            format!("a {name} String has no core value (UTF-8 or US-ASCII text only)"),
        ));
    }
    // SAFETY: the bytes are copied before any further Ruby call can run.
    let bytes = unsafe { string.as_slice() }.to_vec();
    String::from_utf8(bytes).map_err(|_| {
        Error::new(
            ruby.exception_encoding_error(),
            "a String with invalid UTF-8 bytes has no core value",
        )
    })
}

fn time_in(ruby: &Ruby, time: magnus::Time) -> Result<Value, Error> {
    let subsec: RValue = time.funcall("subsec", ())?;
    let scaled: RValue = subsec.funcall("*", (1_000_000_000i64,))?;
    let denominator: i64 = scaled.funcall("denominator", ())?;
    if denominator != 1 {
        return Err(arg_error(
            ruby,
            "a Time with sub-nanosecond precision has no core value",
        ));
    }
    let spec = time.timespec()?;
    let utc: bool = time.funcall("utc?", ())?;
    let offset = i32::try_from(time.utc_offset())
        .map_err(|_| arg_error(ruby, "Time UTC offset out of range"))?;
    let nanos = u32::try_from(spec.tv_nsec)
        .map_err(|_| arg_error(ruby, "Time nanoseconds out of range"))?;
    Ok(Value::Time(Time::new(spec.tv_sec, nanos, offset, utc)))
}

fn regexp_in(ruby: &Ruby, regexp: RRegexp) -> Result<Value, Error> {
    let bits = i32::from(regexp.options());
    let source: RString = regexp.funcall("source", ())?;
    let source = text(ruby, source)?;
    // FIXEDENCODING is implied (and recreated) by non-ASCII source text; on an
    // ASCII source it records an explicit //u that the core value cannot hold.
    let dropped = bits & !KEPT_REGEXP_BITS;
    let implied_fixed_encoding = dropped == FIXEDENCODING && !source.is_ascii();
    if dropped != 0 && !implied_fixed_encoding {
        return Err(arg_error(
            ruby,
            format!("Regexp options {bits} have no core value (i, x and m only)"),
        ));
    }
    Ok(Value::Regexp(Regexp {
        source,
        options: (bits & KEPT_REGEXP_BITS) as u32,
    }))
}

fn convert_out(ruby: &Ruby, value: &Value, depth: usize) -> Result<RValue, Error> {
    if depth > MAX_DEPTH {
        return Err(arg_error(
            ruby,
            format!("nesting deeper than {MAX_DEPTH} levels is not converted"),
        ));
    }
    Ok(match value {
        Value::Nil => ruby.qnil().as_value(),
        Value::Bool(b) => b.into_value_with(ruby),
        Value::Integer(i) => match i64::try_from(i) {
            Ok(n) => ruby.integer_from_i64(n).as_value(),
            Err(_) => ruby.module_kernel().funcall("Integer", (i.to_string(),))?,
        },
        Value::Float(f) => ruby.float_from_f64(*f).as_value(),
        Value::String(s) => ruby.str_new(s).as_value(),
        Value::Symbol(s) => ruby.to_symbol(s).as_value(),
        Value::Time(t) => {
            if !t.is_integral_nanos() {
                return Err(arg_error(
                    ruby,
                    "a Time with sub-nanosecond precision is not converted",
                ));
            }
            let spec = Timespec {
                tv_sec: t.seconds,
                tv_nsec: i64::from(t.nanos),
            };
            let offset = if t.utc {
                Offset::utc()
            } else {
                Offset::from_secs(t.utc_offset).map_err(|e| e.into_error(ruby))?
            };
            ruby.time_timespec_new(spec, offset)?.as_value()
        }
        Value::Regexp(r) => {
            let mut opts = Opts::new();
            if r.options & Regexp::IGNORECASE != 0 {
                opts = opts.ignorecase();
            }
            if r.options & Regexp::EXTENDED != 0 {
                opts = opts.extend();
            }
            if r.options & Regexp::MULTILINE != 0 {
                opts = opts.multiline();
            }
            ruby.reg_new(&r.source, opts)?.as_value()
        }
        Value::Array(items) => {
            let array = ruby.ary_new_capa(items.len());
            for item in items {
                array.push(convert_out(ruby, item, depth + 1)?)?;
            }
            array.as_value()
        }
        Value::Hash(pairs) => {
            let hash = ruby.hash_new();
            for (key, item) in pairs {
                let key = convert_out(ruby, key, depth + 1)?;
                let item = convert_out(ruby, item, depth + 1)?;
                hash.aset(key, item)?;
            }
            hash.as_value()
        }
    })
}
