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
        Data::Date { .. } | Data::Time(_) => {
            return Err(SchemaError::unsupported(
                "JSON.dump of Date/Time",
                "a schema holding a Date or Time value is not reproduced",
            ))
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
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
        s: text,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    s: &'a str,
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

    fn value(&mut self) -> Option<Data> {
        match *self.b.get(self.i)? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Data::String),
            b't' => self.eat("true").then_some(Data::Bool(true)),
            b'f' => self.eat("false").then_some(Data::Bool(false)),
            b'n' => self.eat("null").then_some(Data::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn object(&mut self) -> Option<Data> {
        self.i += 1;
        let mut pairs: Vec<(Data, Data)> = Vec::new();
        self.ws();
        if self.eat("}") {
            return Some(Data::Hash(pairs));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return None;
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(":") {
                return None;
            }
            self.ws();
            let value = self.value()?;
            if pairs
                .iter()
                .any(|(k, _)| matches!(k, Data::String(s) if *s == key))
            {
                return None;
            }
            pairs.push((Data::String(key), value));
            self.ws();
            if self.eat(",") {
                continue;
            }
            return self.eat("}").then_some(Data::Hash(pairs));
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
            return self.eat("]").then_some(Data::Array(items));
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = self.s.get(self.i..self.i + 4)?;
        let v = u32::from_str_radix(h, 16).ok()?;
        self.i += 4;
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = *self.b.get(self.i)?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.i += 1;
                    let e = *self.b.get(self.i)?;
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
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat("\\u") {
                                    return None;
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return None;
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return None;
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp)?);
                        }
                        _ => return None,
                    }
                }
                0..=0x1f => return None,
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
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            _ => return None,
        }
        let mut float = false;
        if self.b.get(self.i) == Some(&b'.') {
            float = true;
            self.i += 1;
            let d = self.i;
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
            if self.i == d {
                return None;
            }
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
            if self.i == d {
                return None;
            }
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
