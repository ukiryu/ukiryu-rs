//! YAML and JSON text of a response.
//!
//! The source writes `response.to_yaml` (lutaml-model through Psych) and
//! `response.to_json(pretty: true)` (lutaml-model through the json gem's
//! pretty generator). The port serializes the same serde model through the
//! Yeptris emitter and `serde_json`.
use super::RenderError;
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
/// Psych folds plain and single-quoted scalars at 80 columns; with these
/// settings Yeptris keeps them on one line, a deliberate difference
/// recorded by the oracle tests.
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

/// The pretty JSON text (two-space indent, `"key": value`), with floats in
/// Ruby's `Float#to_s` shape (`2.0e-05`, `1.0e+16`) as the json gem writes
/// them.
pub fn to_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<String, RenderError> {
    let mut out = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut out, RubyFloats(PrettyFormatter::new()));
    value
        .serialize(&mut ser)
        .map_err(|e| RenderError::new(e.to_string()))?;
    String::from_utf8(out).map_err(|e| RenderError::new(e.to_string()))
}

/// `Float#to_s` text through the Yeptris engine's float printer (the same
/// shape). `serde_json` writes non-finite floats as `null` before reaching
/// the formatter, where the json gem raises `GeneratorError`; responses
/// carry finite durations only.
fn float_text(x: f64) -> io::Result<String> {
    let text = yeptris::to_string(&Value::Number(Number::F64(x)))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    Ok(text.lines().next().unwrap_or_default().to_owned())
}

struct RubyFloats<'a>(PrettyFormatter<'a>);

impl Formatter for RubyFloats<'_> {
    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(float_text(value)?.as_bytes())
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
