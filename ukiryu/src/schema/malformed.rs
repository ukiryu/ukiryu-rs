//! Psych diagnostics when File.read hands it invalid UTF-8 bytes.
use super::SchemaError;

fn invalid(message: &str) -> SchemaError {
    SchemaError::YamlSyntax {
        message: format!("(<unknown>): {message} at line 1 column 1"),
    }
}

pub(super) fn decode(bytes: Vec<u8>) -> Result<String, SchemaError> {
    // Ruby gives File.read's UTF-8 string to libyaml. Match its reader's
    // order: leading byte, available octets, trailing octets, shortest
    // encoding, Unicode scalar, YAML's printable-character set.
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let (width, mut cp, min) = match b {
            0x00..=0x7f => (1, u32::from(b), 0),
            0xc0..=0xdf => (2, u32::from(b & 0x1f), 0x80),
            0xe0..=0xef => (3, u32::from(b & 0x0f), 0x800),
            0xf0..=0xf7 => (4, u32::from(b & 0x07), 0x10000),
            _ => return Err(invalid("invalid leading UTF-8 octet")),
        };
        for j in 1..width {
            let trailing = bytes
                .get(i + j)
                .ok_or_else(|| invalid("incomplete UTF-8 octet sequence"))?;
            if trailing & 0xc0 != 0x80 {
                return Err(invalid("invalid trailing UTF-8 octet"));
            }
            cp = (cp << 6) | u32::from(trailing & 0x3f);
        }
        if cp < min {
            return Err(invalid("invalid length of a UTF-8 sequence"));
        }
        if (0xd800..=0xdfff).contains(&cp) || cp > 0x10ffff {
            return Err(invalid("invalid Unicode character"));
        }
        if !matches!(cp, 9 | 10 | 13 | 0x20..=0x7e | 0x85 | 0xa0..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
        {
            return Err(invalid("control characters are not allowed"));
        }
        i += width;
    }
    Ok(String::from_utf8(bytes).expect("validated UTF-8"))
}
