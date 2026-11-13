//! YAML and JSON text of a response.
//!
//! The source writes `response.to_yaml` (lutaml-model through Psych) and
//! `response.to_json(pretty: true)` (lutaml-model through the json gem's
//! pretty generator). The port serializes the same serde model through the
//! Yeptris emitter and `serde_json`.
use super::{RenderError, RenderErrorKind};
use serde::Serialize;
use serde_json::ser::{Formatter, PrettyFormatter};
use std::io;
use yeptris::{EmitOptions, LineWidth, Number, Value};

/// The pinned emitter settings for the `yaml` format (ukiryu#27):
/// fidelity (non-canonical) form, indent 2, the explicit `---` document
/// start Psych writes, and no folding or wrapping at any width, so a
/// scalar always stays on one line. Every knob is set explicitly so engine
/// default changes cannot move the output.
///
/// The document carries the same data as Psych's but not Psych's bytes:
/// Psych folds plain and single-quoted scalars at 80 columns and picks
/// scalar styles by YAML 1.1 rules (quoting `.a`, `/usr/bin/jq`,
/// timestamps; literal blocks for multi-line text), while the Yeptris
/// value emitter writes plain or double-quoted scalars under YAML 1.2
/// resolution and, with these settings, never folds. The oracle tests
/// record this as a gap and compare the YAML as data.
pub fn yaml_emit_options() -> EmitOptions {
    EmitOptions::new()
        .canonical(false)
        .indent(2)
        .explicit_document_start(true)
        .line_width(LineWidth::Unbounded)
        .block_fold(false)
}

/// The response YAML document, newline-terminated like Psych's.
pub fn to_yaml<T: Serialize + ?Sized>(value: &T) -> Result<String, RenderError> {
    let tree = yeptris::to_value(value).map_err(|e| RenderError::new(e.to_string()))?;
    let mut text = yeptris::to_string_with(&tree, &yaml_emit_options())
        .map_err(|e| RenderError::new(e.to_string()))?;
    text.push('\n');
    Ok(text)
}

/// The pretty JSON text (two-space indent, `"key": value`), with floats
/// laid out as the json gem's generator writes them. A NaN or infinite
/// float fails with [`RenderErrorKind::Generator`] and the json gem's
/// `GeneratorError` message (`NaN not allowed in JSON`), where
/// `serde_json` alone would write `null`.
pub fn to_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<String, RenderError> {
    let tree = yeptris::to_value(value).map_err(|e| RenderError::new(e.to_string()))?;
    if let Some(x) = first_non_finite(&tree) {
        let word = if x.is_nan() {
            "NaN"
        } else if x > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        };
        return Err(RenderError::with_kind(
            RenderErrorKind::Generator,
            format!("{word} not allowed in JSON"),
        ));
    }
    let mut out = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut out, RubyFloats(PrettyFormatter::new()));
    value
        .serialize(&mut ser)
        .map_err(|e| RenderError::new(e.to_string()))?;
    String::from_utf8(out).map_err(|e| RenderError::new(e.to_string()))
}

/// The first NaN or infinite float in document order.
fn first_non_finite(value: &Value) -> Option<f64> {
    match value {
        Value::Number(Number::F64(x)) if !x.is_finite() => Some(*x),
        Value::Sequence(items) => items.iter().find_map(first_non_finite),
        Value::Mapping(map) => map
            .0
            .iter()
            .find_map(|(k, v)| first_non_finite(k).or_else(|| first_non_finite(v))),
        Value::Tagged(t) => first_non_finite(&t.value),
        _ => None,
    }
}

/// The float text of the json gem 3.1.0 generator: `fpconv_dtoa` in
/// `ext/json/ext/vendor/fpconv.c`, whose `emit_digits` writes plain
/// decimals (`0.00002`, `123456789.0`) inside its exponent window and
/// `1e+16`/`1e-10` outside it. The digits are the shortest round-trip
/// digits; fpconv's Grisu2 can return a longer digit string for a few
/// values (the json gem writes `1.2345e21` as `1.2344999999999999e+21`).
/// Those values are gated by an explicit numeric-equality policy in the
/// oracle tests (same `f64` bits, different digits; ukiryu-rs#23), not by
/// adjusted expectations.
fn json_float_text(x: f64) -> String {
    let mut out = String::with_capacity(32);
    if x.is_sign_negative() {
        out.push('-');
    }
    if x == 0.0 {
        out.push_str("0.0");
        return out;
    }
    let sci = format!("{:e}", x.abs());
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let nd = digits.len() as i32;
    // value = digits * 10^k
    let k = exponent - (nd - 1);
    let exp = (k + nd - 1).abs();
    if k >= 0 && exp < 15 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', k as usize));
        out.push_str(".0");
    } else if k < 0 && (k > -7 || exp < 10) {
        let offset = nd + k;
        if offset <= 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-offset) as usize));
            out.push_str(&digits);
        } else {
            out.push_str(&digits[..offset as usize]);
            out.push('.');
            out.push_str(&digits[offset as usize..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if nd > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if k + nd - 1 < 0 { '-' } else { '+' });
        out.push_str(&exp.to_string());
    }
    out
}

struct RubyFloats<'a>(PrettyFormatter<'a>);

impl Formatter for RubyFloats<'_> {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(json_float_text(value).as_bytes())
    }

    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        self.write_f64(writer, f64::from(value))
    }

    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_array(w)
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array(w)
    }

    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_array_value(w, first)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array_value(w)
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object(w)
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object(w)
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_object_key(w, first)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object_value(w)
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object_value(w)
    }
}
