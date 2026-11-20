//! The two JSON operations the oracle performs: `JSON.dump` of the schema
//! (json 3.1.0; its bytes name the schema in every message) and
//! `JSON.parse` (strict RFC 8259 plus json 3.1.0's duplicate-key and
//! lone-surrogate rejection, probed in the K17-SCHEMA evidence).

use super::data::{json_float, Data};
use super::SchemaError;
use num_bigint::BigInt;

/// `JSON.dump(value)`.
pub(crate) fn dump(value: &Data) -> Result<String, SchemaError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(out: &mut String, value: &Data) -> Result<(), SchemaError> {
    match value {
        Data::Null => out.push_str("null"),
        Data::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Data::Integer(i) => out.push_str(&i.to_string()),
        Data::Float(f) => out.push_str(&json_float(*f)),
        Data::String(s) | Data::Symbol(s) => write_str(out, s),
        Data::AllocatedDate | Data::Date { .. } | Data::Time(_) => {
            write_str(out, &value.ruby_to_s()?)
        }
        Data::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Data::Hash(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let key = match k {
                    Data::String(s) | Data::Symbol(s) => s.clone(),
                    other => other.ruby_to_s()?,
                };
                write_str(out, &key);
                out.push(':');
                write_value(out, v)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.parse(text)`; `None` where Ruby raises `JSON::ParserError`.
pub(crate) fn parse(text: &str) -> Option<Data> {
    parse_schema(text).ok()
}

/// json-schema wraps the parser's exact diagnostic in JsonParseError.
pub(crate) fn parse_schema(text: &str) -> Result<Data, SchemaError> {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
        s: text,
        error: None,
    };
    p.ws();
    let value = p.value();
    if let Some(value) = value {
        p.ws();
        if p.i == p.b.len() {
            return Ok(value);
        }
        p.fail::<Data>(
            format!("unexpected token at end of stream '{}'", p.snippet(p.i)),
            p.i,
        );
    }
    Err(SchemaError::ruby(
        "JSON::Schema::JsonParseError",
        p.error.unwrap_or_else(|| "unexpected end of input".into()),
    ))
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    s: &'a str,
    error: Option<String>,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &str) -> bool {
        if self.b[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }
    fn snippet(&self, at: usize) -> String {
        self.s[at..].chars().take(32).collect()
    }
    fn fail<T>(&mut self, message: impl AsRef<str>, at: usize) -> Option<T> {
        if self.error.is_none() {
            let before = &self.b[..at];
            let line = before.iter().filter(|b| **b == b'\n').count() + 1;
            let line_start = before
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(0, |i| i + 1);
            let column = self.s[line_start..at].chars().count() + 1;
            self.error = Some(format!(
                "{} at line {line} column {column}",
                message.as_ref()
            ));
        }
        None
    }
    fn unexpected(&mut self) -> Option<Data> {
        match self.b.get(self.i) {
            None => self.fail("unexpected end of input", self.i),
            Some(b't' | b'f' | b'n' | b'N' | b'I') => self.fail(
                format!("unexpected token '{}'", self.snippet(self.i)),
                self.i,
            ),
            _ => self.fail(
                format!("unexpected character: '{}'", self.snippet(self.i)),
                self.i,
            ),
        }
    }
    fn value(&mut self) -> Option<Data> {
        match self.b.get(self.i).copied() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Data::String),
            Some(b't') if self.eat("true") => Some(Data::Bool(true)),
            Some(b'f') if self.eat("false") => Some(Data::Bool(false)),
            Some(b'n') if self.eat("null") => Some(Data::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.unexpected(),
        }
    }
    fn object(&mut self) -> Option<Data> {
        let start = self.i;
        self.i += 1;
        let mut pairs: Vec<(Data, Data)> = Vec::new();
        self.ws();
        if self.eat("}") {
            return Some(Data::Hash(pairs));
        }
        let mut after_comma = false;
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                let msg = if self.i == self.b.len() {
                    format!(
                        "expected object key, got{} EOF",
                        if after_comma { ":" } else { "" }
                    )
                } else {
                    format!(
                        "expected object key, got{} '{}'",
                        if after_comma { ":" } else { "" },
                        self.snippet(self.i)
                    )
                };
                return self.fail(msg, self.i);
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(":") {
                return self.fail("expected ':' after object key", self.i);
            }
            self.ws();
            let value = self.value()?;
            if pairs
                .iter()
                .any(|(k, _)| matches!(k, Data::String(s) if *s == key))
            {
                return self.fail(
                    format!("duplicate key {}", super::data::inspect_str(&key)),
                    start,
                );
            }
            pairs.push((Data::String(key), value));
            self.ws();
            if self.eat(",") {
                after_comma = true;
                continue;
            }
            if self.eat("}") {
                return Some(Data::Hash(pairs));
            }
            return self.fail(
                format!(
                    "expected ',' or '}}' after object value, got: {}",
                    if self.i == self.b.len() {
                        "EOF".into()
                    } else {
                        format!("'{}'", self.snippet(self.i))
                    }
                ),
                self.i,
            );
        }
    }
    fn array(&mut self) -> Option<Data> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat("]") {
            return Some(Data::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("]") {
                return Some(Data::Array(items));
            }
            return self.fail("expected ',' or ']' after array value", self.i);
        }
    }
    fn hex4(&mut self, escape: usize) -> Option<u32> {
        let value = self
            .s
            .get(self.i..self.i + 4)
            .and_then(|s| u32::from_str_radix(s, 16).ok());
        match value {
            Some(value) => {
                self.i += 4;
                Some(value)
            }
            None => self.fail(
                format!(
                    "incomplete unicode character escape sequence at '{}'",
                    self.snippet(escape)
                ),
                escape,
            ),
        }
    }
    fn string(&mut self) -> Option<String> {
        self.i += 1;
        // json 3.1 finds a closing quote before decoding escape sequences.
        // Thus an unterminated string wins over an invalid escape within it.
        let mut end = self.i;
        while let Some(c) = self.b.get(end) {
            match c {
                b'"' => break,
                b'\\' => end += 2,
                _ => end += 1,
            }
        }
        if end >= self.b.len() {
            return self.fail("unexpected end of input, expected closing \"", self.b.len());
        }
        let mut out = String::new();
        loop {
            let Some(c) = self.b.get(self.i).copied() else {
                return self.fail("unexpected end of input, expected closing \"", self.i);
            };
            match c {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                b'\\' => {
                    let escape = self.i;
                    self.i += 1;
                    let Some(e) = self.b.get(self.i).copied() else {
                        return self.fail(
                            format!(
                                "invalid escape character in string: '{}'",
                                self.snippet(escape)
                            ),
                            escape,
                        );
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4(escape)?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat("\\u") {
                                    return self.fail(
                                        format!(
                                            "incomplete surrogate pair at '{}'",
                                            self.snippet(escape)
                                        ),
                                        escape,
                                    );
                                }
                                let lo = self.hex4(escape)?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.fail(
                                        format!(
                                            "invalid surrogate pair at '{}'",
                                            self.snippet(escape)
                                        ),
                                        escape,
                                    );
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return self.fail(
                                    format!(
                                        "unpaired trailing surrogate at '{}'",
                                        self.snippet(escape)
                                    ),
                                    escape,
                                );
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp)?);
                        }
                        _ => {
                            return self.fail(
                                format!(
                                    "invalid escape character in string: '{}'",
                                    self.snippet(escape)
                                ),
                                escape,
                            )
                        }
                    }
                }
                0..=0x1f => {
                    return self.fail(
                        format!(
                            "invalid ASCII control character in string: {}",
                            self.snippet(self.i)
                        ),
                        self.i,
                    )
                }
                _ => {
                    let ch = self.s[self.i..].chars().next()?;
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }
    fn number(&mut self) -> Option<Data> {
        let start = self.i;
        if self.b[self.i] == b'-' {
            self.i += 1;
        }
        let mut valid = true;
        match self.b.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                if self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    valid = false;
                }
            }
            Some(b'1'..=b'9') => {
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            _ => valid = false,
        }
        let mut float = false;
        if self.b.get(self.i) == Some(&b'.') {
            float = true;
            self.i += 1;
            let d = self.i;
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
            valid &= self.i > d;
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            float = true;
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let d = self.i;
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
            valid &= self.i > d;
        }
        if !valid {
            return self.fail(format!("invalid number: '{}'", self.snippet(start)), start);
        }
        let text = &self.s[start..self.i];
        if float {
            text.parse::<f64>().ok().map(Data::Float)
        } else {
            text.parse::<BigInt>().ok().map(Data::Integer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_acceptance_matches_probe() {
        // Accept/reject pairs captured from json 3.1.0 (K17-SCHEMA evidence,
        // captures/json-parse-probe.txt).
        let reject = [
            "{\"a\":1 /* c */}",
            "[1,]",
            "\"a\tb\"",
            "01",
            "-",
            "NaN",
            "",
            "  ",
            "\"\\ud800\"",
            "{\"a\":1,\"a\":2,\"b\":3}",
            "1.",
            ".5",
            "[1 2]",
            "true false",
            "nul",
            "\"\\x\"",
            "{1:2}",
            "\"\u{1}\"",
            "123abc",
        ];
        for r in reject {
            assert_eq!(parse(r), None, "{r:?}");
        }
        assert_eq!(parse("1e400"), Some(Data::Float(f64::INFINITY)));
        assert_eq!(parse(" 1 "), Some(Data::Integer(1.into())));
        assert_eq!(parse("-0"), Some(Data::Integer(0.into())));
        assert_eq!(parse("1.5E+2"), Some(Data::Float(150.0)));
    }

    #[test]
    fn dump_escapes_like_json_gem() {
        let v = Data::Array(vec![Data::String("é\u{2028}</a>\u{7f}\u{0}\"\\/".into())]);
        assert_eq!(
            dump(&v).unwrap(),
            "[\"é\u{2028}</a>\u{7f}\\u0000\\\"\\\\/\"]"
        );
    }
}
