//! `YAML.safe_load` / `Psych.safe_load_file` as each source call site
//! configures it (Psych 5.3.1, the pinned oracle's version).
//!
//! The document is parsed by Yeptris; this module walks the Yeptris DOM so
//! that a YAML tag sees its scalar's source text, the way Psych's
//! `Visitors::ToRuby` does. Untagged scalars keep the Yeptris resolution the
//! rest of the crate reads (the framework reader is Yeptris, not runtime
//! Psych); tagged nodes follow `ToRuby#deserialize`,
//! `#visit_Psych_Nodes_Sequence` and `#visit_Psych_Nodes_Mapping`:
//!
//! - `!!binary` decodes the text with `String#unpack('m')`.
//! - `!!str`, `!str`, `!ruby/string` keep the text.
//! - `!!float` is `Float(tokenize(text))`.
//! - `!ruby/sym...` is `text.to_sym` (needs `Symbol` permitted).
//! - Every other scalar tag (`!!int`, `!!bool`, `!!null`, `!!timestamp`,
//!   local and foreign tags) is `ScalarScanner#tokenize(text)`, so `!!int
//!   "010"` is 8 and `!!timestamp ...` raises `Psych::DisallowedClass`
//!   unless `Time`/`Date` is permitted.
//! - `!!omap` on a sequence builds an ordered Hash (no class check);
//!   `!!set` and `!!omap` on a mapping load the `Psych::Set`/`Psych::Omap`
//!   classes and raise `DisallowedClass`.
//! - `!ruby/...` object tags resolve their class through the call's
//!   `permitted_classes`.
//! - Aliases raise `Psych::AliasesNotEnabled` unless the call passes
//!   `aliases: true`; `<<` merge keys follow `ToRuby#revive_hash`.
//!
//! A tag is applied or dropped, never kept as a one-entry Hash.
//!
//! Two Ruby facts have no separate representation in [`Value`] and are
//! lowered to the equal value: a `!!binary` result is an ASCII-8BIT String
//! (held as `Value::String` when its bytes are ASCII, where Ruby's `==`,
//! `eql?`, `hash`, `inspect` and `to_s` cannot tell the encodings apart;
//! non-ASCII bytes are `Unsupported`), and a sequence `!!omap` is a
//! `Psych::Omap` Hash subclass (held as a `Value::Hash` with the same
//! pairs, which Ruby's `Hash#==` treats as equal). Recursive aliases,
//! revived Ruby objects, and Date/Time scalars are `Unsupported`.
use super::error::{ModelError, ModelErrorKind};
use super::value::Value;
use crate::schema::{tokenize, Data, Permitted};
use yeptris::{Document, Node, NodeKind};

/// The `permitted_classes:` / `aliases:` pair of one `safe_load` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadPolicy {
    /// Class names the call permits (`permitted_classes:`).
    pub permitted_classes: &'static [&'static str],
    /// `aliases: true`.
    pub aliases: bool,
}

impl LoadPolicy {
    /// `YAML.safe_load(text, permitted_classes: [Symbol], aliases: true)`:
    /// `Ukiryu::Register` (register.rb:146/175/381/398/440), `ToolIndex`
    /// (tool_index.rb:295/340/404), `Definition::Discovery` (discovery.rb:217).
    pub const REGISTER: LoadPolicy = LoadPolicy {
        permitted_classes: &["Symbol"],
        aliases: true,
    };

    /// `Psych.safe_load_file(path, permitted_classes: [Symbol, String,
    /// Integer, Array, Hash, TrueClass, FalseClass])`: `Interface.from_yaml`,
    /// `ImplementationIndex.from_yaml`, `ImplementationVersion.from_yaml`.
    pub const MODEL_FILE: LoadPolicy = LoadPolicy {
        permitted_classes: &[
            "Symbol",
            "String",
            "Integer",
            "Array",
            "Hash",
            "TrueClass",
            "FalseClass",
        ],
        aliases: false,
    };

    /// `YAML.safe_load(text, permitted_classes: [Symbol])` and `YAML.load`'s
    /// defaults: `RunFileCommand` (run_file_command.rb:61), `RunCommand`
    /// (run_command.rb:150), `ConfigCommand` (config_command.rb:267).
    pub const SYMBOL: LoadPolicy = LoadPolicy {
        permitted_classes: &["Symbol"],
        aliases: false,
    };

    fn permits(&self, class: &str) -> bool {
        self.permitted_classes.contains(&class)
    }

    fn scanner(&self) -> Permitted {
        Permitted {
            symbol: self.permits("Symbol"),
            date: self.permits("Date"),
            time: self.permits("Time"),
        }
    }
}

/// `YAML.safe_load(text, **policy)`: the first document's Ruby value
/// (`nil` for an empty stream).
pub fn safe_load(text: &str, policy: LoadPolicy) -> Result<Value, ModelError> {
    if text
        .lines()
        .all(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
    {
        return Ok(Value::Nil);
    }
    let doc = Document::parse(text.as_bytes()).map_err(|e| {
        ModelError::new(
            ModelErrorKind::Syntax,
            e.message()
                .map(str::to_owned)
                .unwrap_or_else(|| e.to_string()),
        )
    })?;
    let Some(root) = doc.first_root() else {
        return Ok(Value::Nil);
    };
    Loader {
        policy,
        open: Vec::new(),
    }
    .node(root)
}

fn unsupported(message: impl Into<String>) -> ModelError {
    ModelError::new(ModelErrorKind::Unsupported, message)
}

fn disallowed(class: &str) -> ModelError {
    ModelError::new(
        ModelErrorKind::DisallowedClass,
        format!("Tried to load unspecified class: {class}"),
    )
}

fn no_method_first_for_nil() -> ModelError {
    ModelError::new(ModelErrorKind::NoMethod, "undefined method 'first' for nil")
}

struct Loader {
    policy: LoadPolicy,
    /// Collection nodes being converted (an alias to one of them is a
    /// recursive structure).
    open: Vec<u32>,
}

impl Loader {
    fn node(&mut self, node: Node<'_>) -> Result<Value, ModelError> {
        match node.kind() {
            NodeKind::Alias => {
                if !self.policy.aliases {
                    return Err(ModelError::new(
                        ModelErrorKind::AliasesNotEnabled,
                        "Alias parsing was not enabled. To enable it, pass `aliases: true` to `Psych::load` or `Psych::safe_load`.",
                    ));
                }
                let target = node
                    .alias_target()
                    .ok_or_else(|| ModelError::new(ModelErrorKind::Syntax, "unknown alias"))?;
                if self.open.contains(&target.id()) {
                    return Err(unsupported(
                        "a recursive YAML alias builds a self-referencing Ruby object; not reproduced",
                    ));
                }
                self.node(target)
            }
            NodeKind::Scalar => self.scalar(node),
            NodeKind::Sequence => {
                self.open.push(node.id());
                let out = self.sequence(node);
                self.open.pop();
                out
            }
            NodeKind::Mapping => {
                self.open.push(node.id());
                let out = self.mapping(node);
                self.open.pop();
                out
            }
        }
    }

    fn scalar(&mut self, node: Node<'_>) -> Result<Value, ModelError> {
        let Some(tag) = tag_of(node)? else {
            let resolved = yeptris::Value::from_root(node)
                .map_err(|e| ModelError::new(ModelErrorKind::Syntax, e.to_string()))?;
            return Value::from_yaml_value(&resolved);
        };
        let text = node
            .str()
            .ok_or_else(|| unsupported("a YAML scalar that is not UTF-8 text"))?;
        self.tagged_scalar(tag, text)
    }

    /// `ToRuby#deserialize` for a scalar with an explicit tag.
    fn tagged_scalar(&self, tag: &str, text: &str) -> Result<Value, ModelError> {
        if tag == "!binary" || tag == "tag:yaml.org,2002:binary" {
            return binary(text);
        }
        if let Some(class) = string_tag_class(tag) {
            // `klass = resolve_class($1)`; no class keeps the text.
            return match class {
                None | Some("") => Ok(Value::String(text.to_owned())),
                Some(name) if !self.policy.permits(name) => Err(disallowed(name)),
                Some("String") => Ok(Value::String(text.to_owned())),
                Some(name) => Err(unsupported(format!(
                    "`{name}.allocate.replace` for the tag {tag}; not reproduced"
                ))),
            };
        }
        let class_object = match tag {
            "!ruby/object:BigDecimal" => Some("BigDecimal"),
            "!ruby/object:DateTime" => Some("DateTime"),
            "!ruby/object:Complex" => Some("Complex"),
            "!ruby/object:Rational" => Some("Rational"),
            "!ruby/regexp" => Some("Regexp"),
            "!ruby/range" => Some("Range"),
            _ => None,
        };
        if let Some(class) = class_object {
            return Err(self.object_class(class, tag));
        }
        if tag == "!ruby/encoding" {
            return Err(unsupported("an Encoding object (`!ruby/encoding`)"));
        }
        if tag == "!ruby/class" || tag == "!ruby/module" {
            if text.is_empty() {
                return Ok(Value::Nil);
            }
            return Err(self.object_class(text, tag));
        }
        if tag == "tag:yaml.org,2002:float" || tag == "!float" {
            return ruby_float(self.tokenize(text)?);
        }
        if tag.starts_with("!ruby/sym") {
            if !self.policy.permits("Symbol") {
                return Err(disallowed("Symbol"));
            }
            return Ok(Value::Symbol(text.to_owned()));
        }
        self.tokenize(text)
    }

    /// A class the tag loads: `DisallowedClass` unless permitted, and a
    /// permitted class builds an object outside the value domain.
    fn object_class(&self, class: &str, tag: &str) -> ModelError {
        if self.policy.permits(class) {
            unsupported(format!(
                "a {class} object revived from the tag {tag}; not reproduced"
            ))
        } else {
            disallowed(class)
        }
    }

    /// `ScalarScanner#tokenize` with the call's permitted classes.
    fn tokenize(&self, text: &str) -> Result<Value, ModelError> {
        let data = tokenize(text, self.policy.scanner()).map_err(|e| match e.ruby_class() {
            Some(class) if class == "Psych::DisallowedClass" => {
                ModelError::new(ModelErrorKind::DisallowedClass, e.message())
            }
            _ => unsupported(e.message()),
        })?;
        Ok(match data {
            Data::Null => Value::Nil,
            Data::Bool(b) => Value::Bool(b),
            Data::Integer(i) => Value::Integer(i),
            Data::Float(f) => Value::Float(f),
            Data::String(s) => Value::String(s),
            Data::Symbol(s) => Value::Symbol(s),
            Data::Date { .. } | Data::Time(_) => {
                return Err(unsupported(
                    "a Date or Time scalar; the value domain has no YAML Date/Time object",
                ))
            }
            Data::Array(_) | Data::Hash(_) => {
                return Err(unsupported("tokenize returned a collection"))
            }
        })
    }

    fn sequence(&mut self, node: Node<'_>) -> Result<Value, ModelError> {
        let tag = tag_of(node)?;
        match tag {
            Some("!omap" | "tag:yaml.org,2002:omap") => return self.omap(node),
            Some(t) => {
                if let Some(name) = t
                    .strip_prefix("!seq:")
                    .or_else(|| t.strip_prefix("!ruby/array:"))
                {
                    if name.is_empty() {
                        return Err(unsupported(format!(
                            "`nil.allocate` for the tag {t}; not reproduced"
                        )));
                    }
                    if !self.policy.permits(name) {
                        return Err(disallowed(name));
                    }
                    if name != "Array" {
                        return Err(unsupported(format!(
                            "a {name} object revived from the tag {t}; not reproduced"
                        )));
                    }
                }
            }
            None => {}
        }
        // `register_empty`: a plain Array whatever the other tag is.
        let mut items = Vec::with_capacity(node.seq_count());
        for item in node.items() {
            items.push(self.node(item)?);
        }
        Ok(Value::Array(items))
    }

    /// `!!omap` on a sequence: `map[accept(a.children.first)] =
    /// accept(a.children.last)` for every item `a`.
    fn omap(&mut self, node: Node<'_>) -> Result<Value, ModelError> {
        let mut out = Value::Hash(Vec::new());
        for item in node.items() {
            let (first, last) = match item.kind() {
                NodeKind::Mapping => {
                    let n = item.map_count();
                    match (
                        item.map_at(0),
                        n.checked_sub(1).and_then(|i| item.map_at(i)),
                    ) {
                        (Some((k, _)), Some((_, v))) => (k, v),
                        _ => return Err(empty_omap_item()),
                    }
                }
                NodeKind::Sequence => {
                    let n = item.seq_count();
                    match (
                        item.seq_at(0),
                        n.checked_sub(1).and_then(|i| item.seq_at(i)),
                    ) {
                        (Some(k), Some(v)) => (k, v),
                        _ => return Err(empty_omap_item()),
                    }
                }
                NodeKind::Scalar | NodeKind::Alias => return Err(no_method_first_for_nil()),
            };
            let key = self.node(first)?;
            let value = self.node(last)?;
            out.insert(key, value);
        }
        Ok(out)
    }

    fn mapping(&mut self, node: Node<'_>) -> Result<Value, ModelError> {
        let Some(tag) = tag_of(node)? else {
            return self.revive_hash(node);
        };
        if tag == "!set" || tag == "tag:yaml.org,2002:set" {
            return Err(self.object_class("Psych::Set", tag));
        }
        if tag == "!omap" || tag == "tag:yaml.org,2002:omap" {
            return Err(self.object_class("Psych::Omap", tag));
        }
        if let Some(rest) = tag.strip_prefix("!ruby/object") {
            let name = rest.strip_prefix(':').unwrap_or(rest);
            return match name {
                "Hash" => self.revive_hash(node),
                "" => Err(self.object_class("Object", tag)),
                name => Err(self.object_class(name, tag)),
            };
        }
        if let Some(rest) = tag.strip_prefix("!ruby/struct") {
            let name = rest.strip_prefix(':').unwrap_or(rest);
            let class = if name.is_empty() { "Struct" } else { name };
            return Err(self.object_class(class, tag));
        }
        if let Some(rest) = tag.strip_prefix("!ruby/exception") {
            // The members are built before the class is resolved.
            self.revive_hash(node)?;
            let name = rest.strip_prefix(':').unwrap_or(rest);
            let class = if name.is_empty() { "Exception" } else { name };
            return Err(self.object_class(class, tag));
        }
        if let Some(name) = tag
            .strip_prefix("!map:")
            .or_else(|| tag.strip_prefix("!ruby/hash:"))
        {
            if name.is_empty() {
                return Err(unsupported(format!(
                    "`nil.allocate` for the tag {tag}; not reproduced"
                )));
            }
            if !self.policy.permits(name) {
                return Err(disallowed(name));
            }
            if name != "Hash" {
                return Err(unsupported(format!(
                    "a {name} object revived from the tag {tag}; not reproduced"
                )));
            }
            return self.revive_hash(node);
        }
        if let Some(rest) = tag.strip_prefix("!ruby/marshalable:") {
            return Err(self.object_class(rest, tag));
        }
        if tag == "!ruby/range" {
            return Err(self.object_class("Range", tag));
        }
        // `/^!ruby\/data(-with-ivars)?(?::(.*))?$/`
        if let Some(rest) = tag.strip_prefix("!ruby/data") {
            let rest = rest.strip_prefix("-with-ivars").unwrap_or(rest);
            if rest.is_empty() {
                return Err(self.object_class("Data", tag));
            }
            if let Some(name) = rest.strip_prefix(':') {
                return Err(self.object_class(if name.is_empty() { "Data" } else { name }, tag));
            }
        }
        let revived_elsewhere = tag.starts_with("!ruby/array:")
            || tag.starts_with("!ruby/hash-with-ivars")
            || string_tag_class(tag).is_some();
        if revived_elsewhere {
            return Err(unsupported(format!(
                "Psych's object revival for the mapping tag {tag}; not reproduced"
            )));
        }
        // Core `!!map`, local, foreign and other tags no `when` matches
        // (`!ruby/symbol {..}` included): a plain Hash.
        self.revive_hash(node)
    }

    /// `ToRuby#revive_hash`, including `<<` merge keys.
    fn revive_hash(&mut self, node: Node<'_>) -> Result<Value, ModelError> {
        let mut hash = Value::Hash(Vec::new());
        for (k, v) in node.entries() {
            let key = self.node(k)?;
            let val = self.node(v)?;
            let merge_key = matches!(&key, Value::String(s) if s == "<<")
                && tag_of(k)? != Some("tag:yaml.org,2002:str");
            if !merge_key {
                hash.insert(key, val);
                continue;
            }
            let merged = match (v.kind(), &val) {
                (NodeKind::Alias | NodeKind::Mapping, Value::Hash(pairs)) => Some(pairs.clone()),
                // `val.reverse_each { |h| merged.merge!(h) }`
                (NodeKind::Sequence, Value::Array(items))
                    if items.iter().all(|i| matches!(i, Value::Hash(_))) =>
                {
                    let mut merged = Value::Hash(Vec::new());
                    for item in items.iter().rev() {
                        if let Value::Hash(pairs) = item {
                            for (mk, mv) in pairs {
                                merged.insert(mk.clone(), mv.clone());
                            }
                        }
                    }
                    match merged {
                        Value::Hash(pairs) => Some(pairs),
                        _ => None,
                    }
                }
                // `merge!` raises TypeError for a non-Hash, which
                // revive_hash rescues by storing the pair as is.
                _ => None,
            };
            match merged {
                Some(pairs) => {
                    for (mk, mv) in pairs {
                        hash.insert(mk, mv);
                    }
                }
                None => hash.insert(key, val),
            }
        }
        Ok(hash)
    }
}

fn empty_omap_item() -> ModelError {
    unsupported(
        "an empty `!!omap` item (Psych raises NoMethodError visit_NilClass); not reproduced",
    )
}

/// The node's tag as text.
fn tag_of<'a>(node: Node<'a>) -> Result<Option<&'a str>, ModelError> {
    node.tag()
        .map(|t| std::str::from_utf8(t).map_err(|_| unsupported("a non-UTF-8 YAML tag")))
        .transpose()
}

/// `/^!(?:str|ruby\/string)(?::(.*))?$/` or the core `str` tag: `Some($1)`.
fn string_tag_class(tag: &str) -> Option<Option<&str>> {
    if tag == "tag:yaml.org,2002:str" {
        return Some(None);
    }
    let rest = tag
        .strip_prefix("!str")
        .or_else(|| tag.strip_prefix("!ruby/string"))?;
    if rest.is_empty() {
        return Some(None);
    }
    rest.strip_prefix(':').map(Some)
}

/// `String#unpack('m')` (lenient Base64: characters outside the alphabet
/// are skipped, `=` ends the data). The result is an ASCII-8BIT String.
fn binary(text: &str) -> Result<Value, ModelError> {
    let bytes = unpack_base64(text.as_bytes());
    if !bytes.is_ascii() {
        return Err(unsupported(
            "a `!!binary` String with non-ASCII bytes; Value text is UTF-8 and cannot carry ASCII-8BIT bytes",
        ));
    }
    Ok(Value::String(
        String::from_utf8(bytes).expect("ASCII bytes are UTF-8"),
    ))
}

fn b64(c: u8) -> i32 {
    match c {
        b'A'..=b'Z' => i32::from(c - b'A'),
        b'a'..=b'z' => i32::from(c - b'a') + 26,
        b'0'..=b'9' => i32::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        _ => -1,
    }
}

/// pack.c `case 'm'` (non-strict branch), reading past the end as the NUL
/// terminator Ruby strings carry.
pub(crate) fn unpack_base64(s: &[u8]) -> Vec<u8> {
    let n = s.len();
    let at = |i: usize| if i < n { s[i] } else { 0 };
    // `while ((x = table[*s]) == -1 && s < send) { [if (*s == '=') break;] s++; }`
    let next = |i: &mut usize, stop_at_pad: bool| -> i32 {
        loop {
            let v = b64(at(*i));
            if v != -1 || *i >= n || (stop_at_pad && at(*i) == b'=') {
                return v;
            }
            *i += 1;
        }
    };
    let mut out = Vec::with_capacity(n * 3 / 4 + 3);
    let mut tail = None;
    let mut i = 0;
    loop {
        let a = next(&mut i, false);
        if i >= n {
            break;
        }
        i += 1;
        let b = next(&mut i, false);
        if i >= n {
            break;
        }
        i += 1;
        let c = next(&mut i, true);
        if at(i) == b'=' || i >= n {
            tail = Some((a, b, c));
            break;
        }
        i += 1;
        let d = next(&mut i, true);
        if at(i) == b'=' || i >= n {
            tail = Some((a, b, c));
            break;
        }
        i += 1;
        out.push(((a << 2) | (b >> 4)) as u8);
        out.push(((b << 4) | (c >> 2)) as u8);
        out.push(((c << 6) | d) as u8);
    }
    if let Some((a, b, c)) = tail {
        out.push(((a << 2) | (b >> 4)) as u8);
        if c != -1 {
            out.push(((b << 4) | (c >> 2)) as u8);
        }
    }
    out
}

/// `Kernel#Float` applied to a tokenized scalar.
fn ruby_float(v: Value) -> Result<Value, ModelError> {
    let type_error = |what: &str| {
        ModelError::new(
            ModelErrorKind::Type,
            format!("can't convert {what} into Float"),
        )
    };
    match v {
        Value::Float(f) => Ok(Value::Float(f)),
        // Integer#to_f: round to nearest, ties to even (the decimal text
        // parses with the same rounding).
        Value::Integer(i) => i
            .to_string()
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| unsupported("Integer#to_f overflow")),
        Value::Nil => Err(type_error("nil")),
        Value::Bool(true) => Err(type_error("true")),
        Value::Bool(false) => Err(type_error("false")),
        Value::Symbol(_) => Err(type_error("Symbol")),
        Value::String(s) => float_from_string(&s),
        other => Err(unsupported(format!(
            "Float() of a {}; not reproduced",
            other.class_name()
        ))),
    }
}

/// `Float(string)` for the decimal forms Ruby accepts (surrounding
/// whitespace, `_` between digits, optional fraction and exponent). Text
/// with no digit at all raises ArgumentError; other shapes are not guessed.
fn float_from_string(s: &str) -> Result<Value, ModelError> {
    let invalid = || {
        ModelError::new(
            ModelErrorKind::Argument,
            format!(
                "invalid value for Float(): {}",
                Value::String(s.to_owned()).inspect()
            ),
        )
    };
    let body = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if let Some(clean) = decimal_float(body) {
        return clean
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| invalid());
    }
    if !body.bytes().any(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    if decimal_float_prefix_with_bad_exponent(body) {
        return Err(invalid());
    }
    Err(unsupported(format!(
        "Float({s:?}) for this spelling is not reproduced"
    )))
}

/// `[+-]?D(\.D)?([eE][+-]?D)?` with `D = digit(_?digit)*`, returned
/// without underscores; `None` when `s` has another shape.
fn decimal_float(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = String::with_capacity(s.len());
    if matches!(b.first(), Some(b'+' | b'-')) {
        out.push(b[0] as char);
        i = 1;
    }
    let digits = |i: &mut usize, out: &mut String| -> bool {
        let start = *i;
        while *i < b.len() {
            if b[*i].is_ascii_digit() {
                out.push(b[*i] as char);
                *i += 1;
            } else if b[*i] == b'_'
                && *i > start
                && b[*i - 1].is_ascii_digit()
                && b.get(*i + 1).is_some_and(u8::is_ascii_digit)
            {
                *i += 1;
            } else {
                break;
            }
        }
        *i > start
    };
    if !digits(&mut i, &mut out) {
        return None;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        out.push('.');
        if !digits(&mut i, &mut out) {
            return None;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        out.push('e');
        if matches!(b.get(i), Some(b'+' | b'-')) {
            out.push(b[i] as char);
            i += 1;
        }
        if !digits(&mut i, &mut out) {
            return None;
        }
    }
    (i == b.len()).then_some(out)
}

/// `1e`, `1e+`: a decimal mantissa followed by an exponent marker with no
/// digits, which Ruby rejects.
fn decimal_float_prefix_with_bad_exponent(s: &str) -> bool {
    let Some(pos) = s.find(['e', 'E']) else {
        return false;
    };
    let exp = s[pos + 1..].trim_start_matches(['+', '-']);
    exp.is_empty() && decimal_float(&s[..pos]).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_m_matches_pack_c() {
        assert_eq!(unpack_base64(b"aGk="), b"hi");
        assert_eq!(unpack_base64(b"aGk"), b"hi");
        assert_eq!(unpack_base64(b"a"), b"");
        assert_eq!(unpack_base64(b"a*G=k="), b"h");
        assert_eq!(unpack_base64(b"aGVsbG8g\nd29ybGQ=\n"), b"hello world");
        assert_eq!(unpack_base64(b"/w=="), vec![0xffu8]);
    }

    #[test]
    fn float_strings() {
        assert_eq!(decimal_float("1e5").as_deref(), Some("1e5"));
        assert_eq!(decimal_float("1_0e1_0").as_deref(), Some("10e10"));
        assert_eq!(decimal_float("1."), None);
        assert_eq!(decimal_float("1__0"), None);
        assert_eq!(decimal_float("1e"), None);
        assert!(decimal_float_prefix_with_bad_exponent("1e"));
    }
}
