//! Additional keyword behavior captured from json-schema 6.2.1, including
//! Ruby exceptions for malformed keyword arguments. The fixture is the contract.
use super::*;

pub(super) fn ruby_error(class: &str, message: impl Into<String>) -> SchemaError {
    SchemaError::Ruby {
        class: class.into(),
        message: message.into(),
    }
}

pub(super) fn class_name(value: &Data) -> &'static str {
    match value {
        Data::Null => "NilClass",
        Data::Bool(false) => "FalseClass",
        Data::Bool(true) => "TrueClass",
        Data::Integer(_) => "Integer",
        Data::Float(_) => "Float",
        Data::String(_) => "String",
        Data::Array(_) => "Array",
        Data::Hash(_) => "Hash",
        Data::Symbol(_) => "Symbol",
        Data::Date { .. } => "Date",
        Data::Time(_) => "Time",
    }
}

pub(super) fn comparison_name(value: &Data) -> &'static str {
    match value {
        Data::Null => "nil",
        Data::Bool(false) => "false",
        Data::Bool(true) => "true",
        _ => class_name(value),
    }
}

pub(super) fn no_method(value: &Data, method: &str) -> SchemaError {
    let receiver = match value {
        Data::Null => "nil".into(),
        Data::Bool(b) => b.to_string(),
        _ => format!("an instance of {}", class_name(value)),
    };
    ruby_error(
        "NoMethodError",
        format!("undefined method '{method}' for {receiver}"),
    )
}

fn string_conversion(value: &Data) -> SchemaError {
    ruby_error(
        "TypeError",
        format!(
            "no implicit conversion of {} into String",
            comparison_name(value)
        ),
    )
}

// Ruby's two-argument each blocks destructure array entries and yield Hash keys.
static NIL: Data = Data::Null;
pub(super) fn each_pairs(value: &Data) -> Result<Vec<(&Data, &Data)>, SchemaError> {
    match value {
        Data::Hash(pairs) => Ok(pairs.iter().map(|(k, v)| (k, v)).collect()),
        Data::Array(items) => Ok(items
            .iter()
            .map(|item| match item {
                Data::Array(parts) => (parts.first().unwrap_or(&NIL), parts.get(1).unwrap_or(&NIL)),
                other => (other, &NIL),
            })
            .collect()),
        other => Err(no_method(other, "each")),
    }
}

pub(super) fn ruby_index<'a>(value: &'a Data, key: &str) -> Result<Option<&'a Data>, SchemaError> {
    match value {
        Data::Hash(pairs) => Ok(get(pairs, key)),
        Data::Integer(_) | Data::Array(_) => Err(ruby_error(
            "TypeError",
            "no implicit conversion of String into Integer",
        )),
        Data::String(_) => Ok(None),
        other => Err(no_method(other, "[]")),
    }
}

pub(super) fn check_schema_uri(s: &str) -> Result<(), SchemaError> {
    if uri_format_valid(s)? {
        return Ok(());
    }
    let parts = uri::split(s);
    if let Some(scheme) = parts.scheme {
        let mut chars = scheme.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            return Err(ruby_error(
                "JSON::Schema::UriError",
                format!("Invalid scheme format: '{scheme}'"),
            ));
        }
    }
    if let Some(authority) = parts.authority {
        if let Some(i) = uri::port_start(authority) {
            let port = String::from_utf8(uri::percent_decode(&authority[i + 1..]))
                .map_err(|_| ruby_error("JSON::Schema::UriError", "Invalid encoding in port"))?;
            if !port.is_empty() && !port.bytes().all(|c| c.is_ascii_digit()) {
                return Err(ruby_error(
                    "JSON::Schema::UriError",
                    format!(
                        "Invalid port number: {}",
                        super::super::data::inspect_str(&port)
                    ),
                ));
            }
        }
        let host = uri::host_of(authority);
        if host
            .chars()
            .any(|c| "<>{}/\\?#@\"".contains(c) || c.is_whitespace())
            || host.starts_with('[')
                && host.ends_with(']')
                && !host[1..host.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:".contains(c))
        {
            return Err(ruby_error(
                "JSON::Schema::UriError",
                format!("Invalid character in host: '{host}'"),
            ));
        }
    }
    if parts.path.is_empty() && parts.authority.is_none_or(str::is_empty) {
        return Err(ruby_error(
            "JSON::Schema::UriError",
            format!("Absolute URI missing hierarchical segment: '{s}'"),
        ));
    }
    Err(ruby_error(
        "JSON::Schema::UriError",
        format!(
            "Cannot assemble URI string with ambiguous path: '{}'",
            parts.path
        ),
    ))
}

pub(super) fn compile_pattern(value: &Data) -> Result<(OnigRegex, &str), SchemaError> {
    let Data::String(source) = value else {
        return Err(string_conversion(value));
    };
    let regex = OnigRegex::compile(source, 0).map_err(|message| {
        ruby_error(
            "RegexpError",
            format!("{message}: /{}/", source.replace('/', "\\/")),
        )
    })?;
    Ok((regex, source))
}

fn pattern_captures(
    regex: &OnigRegex,
    text: &str,
) -> Result<Option<crate::types::onig::Captures>, SchemaError> {
    regex
        .captures(text)
        .ok_or_else(|| unsupported("pattern", "Onigmo match error is not reproduced"))
}

pub(super) fn pattern_match(regex: &OnigRegex, text: &str) -> Result<bool, SchemaError> {
    Ok(pattern_captures(regex, text)?.is_some())
}

pub(super) fn uri_format_valid(text: &str) -> Result<bool, SchemaError> {
    if !text.contains('\n') {
        return Ok(uri::addressable_parses(text));
    }
    let regex = OnigRegex::compile(
        r"^(([^:/?#]+):)?(//([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?$",
        0,
    )
    .expect("fixed Addressable expression");
    let captures = pattern_captures(&regex, text)?;
    let Some(captures) = captures else {
        return Ok(false);
    };
    Ok(uri::addressable_parses(
        captures.text(0, text).unwrap_or(""),
    ))
}

impl<'s> Validation<'s> {
    pub(super) fn const_attr(
        &mut self,
        ctx: &Ctx<'s>,
        value: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        if !value.ruby_eq(data) {
            self.push(
                ctx,
                format!(
                    "The property '{}' value {} did not match constant '{}'",
                    build_fragment(frags),
                    data.ruby_inspect()?,
                    value.ruby_to_s()?
                ),
                frags,
                Attr::Keyword,
                Vec::new(),
            );
        }
        Ok(())
    }

    pub(super) fn unique_items(
        &mut self,
        ctx: &Ctx<'s>,
        _value: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Array(items) = data else {
            return Ok(());
        };
        // Presence selects the attribute even for false/null, like the gem.
        if items
            .iter()
            .enumerate()
            .any(|(i, a)| items[..i].iter().any(|b| a.eql(b)))
        {
            self.push(
                ctx,
                format!(
                    "The property '{}' contained duplicated array values",
                    build_fragment(frags)
                ),
                frags,
                Attr::Keyword,
                Vec::new(),
            );
        }
        Ok(())
    }

    pub(super) fn multiple_of(
        &mut self,
        ctx: &Ctx<'s>,
        value: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        if !matches!(data, Data::Integer(_) | Data::Float(_)) {
            return Ok(());
        }
        let spelling = value.ruby_to_s()?;
        let divisor = match value {
            Data::Integer(_) | Data::Float(_) | Data::String(_) => decimal(&spelling)?,
            _ => None,
        }
        .ok_or_else(|| {
            ruby_error(
                "ArgumentError",
                format!("invalid value for BigDecimal(): \"{spelling}\""),
            )
        })?;
        let number = decimal(&data.ruby_to_s()?)?.expect("numeric Ruby spelling");
        if !number.is_multiple_of(&divisor) {
            self.push(
                ctx,
                format!(
                    "The property '{}' was not divisible by {spelling}",
                    build_fragment(frags)
                ),
                frags,
                Attr::Keyword,
                Vec::new(),
            );
        }
        Ok(())
    }

    pub(super) fn additional_items(
        &mut self,
        ctx: &Ctx<'s>,
        value: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let (Data::Array(items), Some(Data::Array(schemas))) = (data, ctx.get("items")) else {
            return Ok(());
        };
        if items.len() <= schemas.len() {
            return Ok(());
        }
        if matches!(value, Data::Bool(false)) {
            self.push(ctx,format!("The property '{}' contains additional array elements outside of the schema when none are allowed",build_fragment(frags)),frags,Attr::Keyword,Vec::new());
        } else if matches!(value, Data::Hash(_)) {
            let child = self.new_schema(value, ctx.uri.clone(), ctx.dialect)?;
            for (i, item) in items.iter().enumerate().skip(schemas.len()) {
                let mut f = frags.to_vec();
                f.push(i.to_string());
                self.validate(&child, item, &f)?;
            }
        }
        Ok(())
    }

    pub(super) fn pattern_properties(
        &mut self,
        ctx: &Ctx<'s>,
        value: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Hash(pairs) = data else {
            return Ok(());
        };
        for (pattern, sub) in each_pairs(value)? {
            let (regex, _) = compile_pattern(pattern)?;
            for (key, datum) in pairs {
                let text = key.ruby_to_s()?;
                if pattern_match(&regex, &text)? {
                    let child = self.new_schema(sub, ctx.uri.clone(), ctx.dialect)?;
                    let mut f = frags.to_vec();
                    f.push(text);
                    self.validate(&child, datum, &f)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn extra_properties(
        &self,
        ctx: &Ctx<'s>,
        pairs: &[(Data, Data)],
    ) -> Result<Vec<String>, SchemaError> {
        let mut extra: Vec<String> = pairs
            .iter()
            .filter_map(|(k, _)| {
                if let Data::String(s) = k {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
        if let Some(props) = ctx.get("properties").filter(|v| v.truthy()) {
            let defined = each_pairs(props)?;
            extra.retain(|key| {
                !defined
                    .iter()
                    .any(|(k, _)| matches!(k,Data::String(s) if s==key))
            });
        }
        if let Some(patterns) = ctx.get("patternProperties").filter(|v| v.truthy()) {
            for (pattern, _) in each_pairs(patterns)? {
                let (regex, _) = compile_pattern(pattern)?;
                let mut kept = Vec::new();
                for key in extra {
                    if !pattern_match(&regex, &key)? {
                        kept.push(key);
                    }
                }
                extra = kept;
            }
        }
        if let Some(extends) = ctx.get("extends").filter(|v| v.truthy()) {
            let schemas = match extends {
                Data::Array(a) => a.iter().collect(),
                other => vec![other],
            };
            for sub in schemas {
                if let Data::Hash(pairs) = sub {
                    if let Some(Data::Hash(props)) = get(pairs, "properties") {
                        extra.retain(|key| !has_key(props, key));
                    }
                }
            }
        }
        Ok(extra)
    }

    pub(super) fn dependencies(
        &mut self,
        ctx: &Ctx<'s>,
        value: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Hash(pairs) = data else {
            return Ok(());
        };
        for (name, dependency) in each_pairs(value)? {
            let name = name.ruby_to_s()?;
            if !has_key(pairs, &name) {
                continue;
            }
            match dependency {
                Data::Array(names) => {
                    for required in names {
                        let required = required.ruby_to_s()?;
                        if !has_key(pairs, &required) {
                            self.push(ctx,format!("The property '{}' has a property '{name}' that depends on a missing property '{required}'",build_fragment(frags)),frags,Attr::Keyword,Vec::new());
                        }
                    }
                }
                Data::String(required)
                    if ctx.dialect == Dialect::Draft3 && !has_key(pairs, required) =>
                {
                    self.push(ctx,format!("The property '{}' has a property '{name}' that depends on a missing property '{required}'",build_fragment(frags)),frags,Attr::Keyword,Vec::new());
                }
                Data::Hash(_) => {
                    let child = self.new_schema(dependency, ctx.uri.clone(), ctx.dialect)?;
                    self.validate(&child, data, frags)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn property_names(
        &mut self,
        ctx: &Ctx<'s>,
        value: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Hash(pairs) = data else {
            return Ok(());
        };
        if matches!(value, Data::Bool(false)) {
            return self.additional_properties(ctx, value, data, frags);
        }
        if !matches!(value, Data::Hash(_)) {
            return Ok(());
        }
        for (key, _) in pairs {
            let mut f = frags.to_vec();
            f.push(key.ruby_to_s()?);
            let child = self.new_schema(value, ctx.uri.clone(), ctx.dialect)?;
            self.validate(&child, key, &f)?;
        }
        Ok(())
    }

    pub(super) fn non_array_enum(
        &mut self,
        ctx: &Ctx<'s>,
        value: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        match value {
            Data::Hash(pairs) => {
                if pairs.iter().any(|(k, _)| k.eql(data)) {
                    return Ok(());
                }
                let mut names = Vec::new();
                for _ in pairs {
                    names.push("array".to_owned());
                }
                self.push(
                    ctx,
                    format!(
                        "The property '{}' value {} did not match one of the following values: {}",
                        build_fragment(frags),
                        data.ruby_inspect()?,
                        names.join(", ")
                    ),
                    frags,
                    Attr::Enum,
                    Vec::new(),
                );
                Ok(())
            }
            Data::String(text) => {
                let Data::String(needle) = data else {
                    return Err(string_conversion(data));
                };
                if text.contains(needle) {
                    Ok(())
                } else {
                    Err(no_method(value, "map"))
                }
            }
            other => Err(no_method(other, "include?")),
        }
    }
}

// BigDecimal observes decimal Float spelling. Scientific exponents stay
// symbolic: modular exponentiation avoids allocating a giant power of ten.
enum Decimal {
    Finite {
        coefficient: num_bigint::BigInt,
        exponent: i64,
    },
    Infinity,
    Nan,
}

impl Decimal {
    fn is_multiple_of(&self, divisor: &Self) -> bool {
        use num_bigint::BigInt;
        let Self::Finite {
            coefficient: value,
            exponent: value_power,
        } = self
        else {
            return false;
        };
        let Self::Finite {
            coefficient: factor,
            exponent: factor_power,
        } = divisor
        else {
            return matches!(divisor, Self::Infinity) && *value == BigInt::from(0);
        };
        if *factor == BigInt::from(0) {
            return false;
        }
        if *value == BigInt::from(0) {
            return true;
        }
        let modulus = if *factor < BigInt::from(0) {
            -factor
        } else {
            factor.clone()
        };
        let shift = i128::from(*value_power) - i128::from(*factor_power);
        if shift >= 0 {
            (value * BigInt::from(10).modpow(&BigInt::from(shift), &modulus)) % modulus
                == BigInt::from(0)
        } else {
            let digits = value.to_string().len();
            if -shift > digits as i128 {
                return false;
            }
            let power = u32::try_from(-shift).expect("bounded by input digits");
            value % (modulus * BigInt::from(10).pow(power)) == BigInt::from(0)
        }
    }
}

fn decimal(text: &str) -> Result<Option<Decimal>, SchemaError> {
    let text = text.trim_matches(|c: char| c.is_ascii_whitespace());
    if text == "NaN" {
        return Ok(Some(Decimal::Nan));
    }
    if matches!(text, "Infinity" | "+Infinity" | "-Infinity") {
        return Ok(Some(Decimal::Infinity));
    }
    let mut clean = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        if c == '_' {
            if i == 0 || !text.as_bytes()[i - 1].is_ascii_digit() {
                return Ok(None);
            }
        } else {
            clean.push(c);
        }
    }
    static SYNTAX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let syntax = SYNTAX.get_or_init(|| {
        regex::Regex::new(r"\A[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eEdD][+-]?[0-9]+)?\z")
            .expect("fixed decimal syntax")
    });
    if !syntax.is_match(&clean) {
        return Ok(None);
    }
    let (mantissa, exponent) = match clean.find(['e', 'E', 'd', 'D']) {
        Some(i) => (&clean[..i], clean[i + 1..].parse::<i64>().ok()),
        None => (clean.as_str(), Some(0)),
    };
    let (digits, scale) = match mantissa.split_once('.') {
        Some((a, b)) => (
            format!("{a}{b}"),
            i64::try_from(b.len()).expect("input length fits address space"),
        ),
        None => (mantissa.to_owned(), 0),
    };
    let exponent = exponent.and_then(|e| e.checked_sub(scale)).ok_or_else(|| {
        unsupported(
            "decimal exponent",
            "BigDecimal exponent outside signed 64-bit needs an oracle probe",
        )
    })?;
    Ok(Some(Decimal::Finite {
        coefficient: digits.parse().expect("validated decimal digits"),
        exponent,
    }))
}
