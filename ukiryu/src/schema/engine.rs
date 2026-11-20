//! `JSON::Validator.fully_validate` of json-schema 6.2.1, for draft-03,
//! draft-04 and draft-06 keyword subsets, evaluated in schema key order
//! with the gem's exact messages.
//!
//! Every input the port does not reproduce is an explicit
//! [`SchemaError::Unsupported`]; nothing is silently skipped.

use super::data::{bigint_to_f64, num_cmp, Data};
use super::uri;
use super::SchemaError;
use crate::types::onig::OnigRegex;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const INDENT: &str = "    ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    Draft3,
    Draft4,
    Draft6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attr {
    Type,
    Required,
    Properties,
    AdditionalProperties,
    OneOf,
    AnyOf,
    AllOf,
    Not,
    Enum,
    Format,
    Pattern,
    Limit,
}

/// A recorded `JSON::Schema::ValidationError`.
#[derive(Clone, Debug)]
pub(crate) struct VErr {
    message: String,
    uri: Rc<str>,
    fragments: Vec<String>,
    attr: Attr,
    properties: Vec<String>,
    sub_errors: Vec<(String, Vec<VErr>)>,
}

impl VErr {
    fn message_with_schema(&self) -> String {
        format!("{} in schema {}", self.message, self.uri)
    }

    /// `ValidationError#to_string` (what `fully_validate` returns).
    pub(crate) fn to_string_at(&self, level: usize) -> String {
        if self.sub_errors.is_empty() {
            return if level == 0 {
                self.message_with_schema()
            } else {
                self.message.clone()
            };
        }
        let mut lines = vec![format!(
            "{}. The schema specific errors were:\n",
            self.message
        )];
        for (name, errors) in &self.sub_errors {
            lines.push(format!("- {name}:"));
            for e in errors {
                lines.push(format!("{INDENT}- {}", e.to_string_at(level + 1)));
            }
        }
        let pad = INDENT.repeat(level);
        lines
            .iter()
            .map(|l| format!("{pad}{l}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `to_hash[:message]` and `to_hash[:fragment]` as DefinitionValidator
    /// formats them: `"#{message} (at #{fragment})"`.
    pub(crate) fn definition_line(&self) -> String {
        format!(
            "{} (at {})",
            self.message_with_schema(),
            build_fragment(&self.fragments)
        )
    }
}

fn build_fragment(fragments: &[String]) -> String {
    format!("#/{}", fragments.join("/"))
}

#[derive(Clone)]
struct Ctx<'s> {
    pairs: &'s [(Data, Data)],
    node: &'s Data,
    uri: Rc<str>,
    dialect: Dialect,
}

impl<'s> Ctx<'s> {
    fn get(&self, key: &str) -> Option<&'s Data> {
        get(self.pairs, key)
    }
}

fn get<'a>(pairs: &'a [(Data, Data)], key: &str) -> Option<&'a Data> {
    pairs
        .iter()
        .find(|(k, _)| matches!(k, Data::String(s) if s == key))
        .map(|(_, v)| v)
}

fn has_key(pairs: &[(Data, Data)], key: &str) -> bool {
    get(pairs, key).is_some()
}

fn unsupported(feature: &str, detail: impl Into<String>) -> SchemaError {
    SchemaError::unsupported(feature, detail)
}

pub(crate) struct Validation<'s> {
    root_uri: Rc<str>,
    strict: bool,
    active: HashSet<(*const Data, *const Data)>,
    locations: RefCell<HashMap<*const Data, Rc<str>>>,
    schemas: RefCell<HashMap<String, (&'s Data, Dialect)>>,
    walked: HashSet<*const Data>,
    errors: Vec<VErr>,
}

/// Run `fully_validate(schema, data, strict:)`. `schema` is already
/// stringified, `data` yields the prepared document, `root_uri` is the
/// schema's UUID.
pub(crate) fn fully_validate(
    schema: &Data,
    root_uri: &str,
    data: impl FnOnce() -> Result<Data, SchemaError>,
    strict: bool,
) -> Result<Vec<VErr>, SchemaError> {
    let resources = super::resources::load(schema, root_uri)?;
    let mut v = Validation {
        root_uri: Rc::from(root_uri),
        strict,
        active: HashSet::new(),
        locations: RefCell::new(HashMap::new()),
        schemas: RefCell::new(HashMap::new()),
        walked: HashSet::new(),
        errors: Vec::new(),
    };
    let root = v.new_schema(schema, v.root_uri.clone(), Dialect::Draft6)?;
    v.walked.insert(schema as *const Data);
    v.build(&root)?;
    for (name, node) in &resources {
        let resource_uri = if super::transport::is_http(name) {
            format!("{name}#")
        } else {
            name.clone()
        };
        let resource = v.new_schema(node, Rc::from(resource_uri.as_str()), Dialect::Draft6)?;
        v.schemas.borrow_mut().insert(
            name.trim_end_matches('#').to_owned(),
            (node, resource.dialect),
        );
        v.walked.insert(node as *const Data);
        v.build(&resource)?;
    }
    // The gem prepares the data only after the schema is built.
    let data = data()?;
    v.validate(&root, &data, &[])?;
    Ok(v.errors)
}

/// The dialect `$schema` selects: `validators["#{scheme}://#{host}#{path}"]`.
fn resolve_dialect(value: &Data) -> Result<Dialect, SchemaError> {
    let Data::String(s) = value else {
        return Err(unsupported(
            "$schema",
            "a non-string $schema raises TypeError in Addressable; not reproduced",
        ));
    };
    if s.contains(['\n', '\r']) || !uri::addressable_parses(s) {
        return Err(unsupported(
            "$schema",
            format!("$schema {s:?} raises JSON::Schema::UriError; not reproduced"),
        ));
    }
    let p = uri::split(s);
    let host = p.authority.map(|a| {
        let a = a.rsplit_once('@').map_or(a, |(_, h)| h);
        match a.rfind(':') {
            Some(i) if !a[i + 1..].contains([']']) => &a[..i],
            _ => a,
        }
    });
    let key = format!(
        "{}://{}{}",
        p.scheme.unwrap_or(""),
        host.unwrap_or(""),
        p.path
    );
    match key.as_str() {
        "http://json-schema.org/draft-03/schema" => Ok(Dialect::Draft3),
        "http://json-schema.org/draft-04/schema" => Ok(Dialect::Draft4),
        "http://json-schema.org/draft-06/schema" => Ok(Dialect::Draft6),
        "http://json-schema.org/draft-01/schema"
        | "http://json-schema.org/draft-02/schema"
        | "http://json-schema.org/draft-01/hyper-schema"
        | "http://json-schema.org/draft-02/hyper-schema"
        | "http://json-schema.org/draft-03/hyper-schema"
        | "http://json-schema.org/draft-04/hyper-schema"
        | "http://json-schema.org/draft-06/hyper-schema" => Err(unsupported(
            "$schema dialect",
            format!("json-schema 6.2.1 validates {s} with a dialect this port does not implement"),
        )),
        _ => Err(SchemaError::Schema {
            message: format!("Schema not found: {s}"),
        }),
    }
}

/// Ruby `String#to_i` for array fragment indexes.
fn to_i(s: &str) -> i64 {
    let t = s.trim_start();
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits: String = body
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .filter(|c| *c != '_')
        .collect();
    let v: i64 = digits.parse().unwrap_or(0);
    if neg {
        -v
    } else {
        v
    }
}

fn type_of_data(data: &Data) -> &'static str {
    match data {
        Data::Integer(_) => "integer",
        Data::Float(_) => "number",
        Data::String(_) => "string",
        Data::Bool(_) => "boolean",
        Data::Hash(_) => "object",
        Data::Array(_) => "array",
        Data::Null => "null",
        Data::Symbol(_) | Data::Date { .. } | Data::Time(_) => "any",
    }
}

/// `data_valid_for_type?(data, type)`: unknown names (and non-strings) are
/// treated as valid by the gem.
fn valid_for_type(data: &Data, ty: Option<&Data>) -> bool {
    let Some(Data::String(name)) = ty else {
        return true;
    };
    match name.as_str() {
        "string" => matches!(data, Data::String(_)),
        "number" => matches!(data, Data::Integer(_) | Data::Float(_)),
        "integer" => matches!(data, Data::Integer(_)),
        "boolean" => matches!(data, Data::Bool(_)),
        "object" => matches!(data, Data::Hash(_)),
        "array" => matches!(data, Data::Array(_)),
        "null" => matches!(data, Data::Null),
        _ => true,
    }
}

/// `ArraySet#include?` element normalisation: top-level numerics as Float.
fn array_set_key(v: &Data) -> Data {
    match v {
        Data::Integer(i) => Data::Float(bigint_to_f64(i)),
        other => other.clone(),
    }
}

impl<'s> Validation<'s> {
    /// `JSON::Schema.new(node, uri, parent_validator)`.
    fn new_schema(
        &self,
        node: &'s Data,
        uri: Rc<str>,
        parent: Dialect,
    ) -> Result<Ctx<'s>, SchemaError> {
        let Data::Hash(pairs) = node else {
            return Err(unsupported(
                "subschema",
                format!(
                    "a {} where a schema object is expected is not reproduced",
                    type_of_data(node)
                ),
            ));
        };
        let known = self.locations.borrow().get(&(node as *const Data)).cloned();
        let uri = if let Some(known) =
            known.filter(|_| matches!(get(pairs, "id"), Some(Data::String(id)) if !id.is_empty()))
        {
            known
        } else {
            let resolved = match get(pairs, "id") {
                Some(Data::String(id)) => Rc::from(uri::schema_id(&uri, id)?),
                _ => uri,
            };
            self.locations
                .borrow_mut()
                .entry(node as *const Data)
                .or_insert_with(|| resolved.clone());
            resolved
        };
        let dialect = match get(pairs, "$schema") {
            Some(v) if v.truthy() => resolve_dialect(v)?,
            _ => parent,
        };
        self.schemas
            .borrow_mut()
            .entry(uri.trim_end_matches('#').to_owned())
            .or_insert((node, dialect));
        Ok(Ctx {
            pairs,
            node,
            uri,
            dialect,
        })
    }

    /// `handle_schema`: only Hash members are schemas.
    fn handle(&mut self, parent: &Ctx<'s>, obj: &'s Data) -> Result<(), SchemaError> {
        if matches!(obj, Data::Hash(_)) {
            let child = self.new_schema(obj, parent.uri.clone(), parent.dialect)?;
            self.walked.insert(obj as *const Data);
            self.build(&child)?;
        }
        Ok(())
    }

    /// `build_schemas`.
    fn build(&mut self, ctx: &Ctx<'s>) -> Result<(), SchemaError> {
        if let Some(r) = ctx.get("$ref") {
            if r.truthy() {
                self.check_local_ref(r)?;
            }
        }
        if let Some(e) = ctx.get("extends") {
            if matches!(e, Data::String(_)) {
                return Err(unsupported("extends", "extends is not reproduced"));
            }
        }
        for key in ["type", "disallow"] {
            if let Some(Data::Array(items)) = ctx.get(key) {
                for item in items {
                    self.handle(ctx, item)?;
                }
            }
        }
        for key in ["definitions", "properties", "patternProperties"] {
            match ctx.get(key) {
                Some(Data::Hash(pairs)) => {
                    for (_, inner) in pairs {
                        self.handle(ctx, inner)?;
                    }
                }
                Some(Data::Array(_)) | None | Some(Data::Null | Data::Bool(false)) => {}
                Some(_) => {
                    return Err(unsupported(
                        key,
                        format!("a non-object {key} raises NoMethodError; not reproduced"),
                    ))
                }
            }
        }
        for key in [
            "additionalProperties",
            "additionalItems",
            "dependencies",
            "extends",
        ] {
            if let Some(v @ Data::Hash(_)) = ctx.get(key) {
                self.handle(ctx, v)?;
            }
        }
        for key in ["allOf", "anyOf", "oneOf", "not", "extends"] {
            if let Some(Data::Array(items)) = ctx.get(key) {
                for item in items {
                    self.handle(ctx, item)?;
                }
            }
        }
        match ctx.get("items") {
            Some(Data::Array(items)) => {
                for item in items {
                    self.handle(ctx, item)?;
                }
            }
            Some(v) if v.truthy() => self.handle(ctx, v)?,
            _ => {}
        }
        Ok(())
    }

    fn check_local_ref(&self, r: &Data) -> Result<(), SchemaError> {
        let Data::String(_) = r else {
            let (class, message) = match r {
                Data::Null | Data::Bool(false) => (
                    "NoMethodError",
                    "undefined method 'normalize_ref' for nil".into(),
                ),
                other => (
                    "TypeError",
                    format!(
                        "Can't convert {} into String.",
                        match other {
                            Data::Integer(_) => "Integer",
                            Data::Float(_) => "Float",
                            Data::Bool(true) => "TrueClass",
                            Data::Array(_) => "Array",
                            Data::Hash(_) => "Hash",
                            Data::Symbol(_) => "Symbol",
                            Data::Date { .. } => "Date",
                            Data::Time(_) => "Time",
                            _ => unreachable!(),
                        }
                    ),
                ),
            };
            return Err(SchemaError::Ruby {
                class: class.into(),
                message,
            });
        };
        Ok(())
    }

    fn push(
        &mut self,
        ctx: &Ctx<'s>,
        message: String,
        frags: &[String],
        attr: Attr,
        properties: Vec<String>,
    ) {
        self.errors.push(VErr {
            message,
            uri: ctx.uri.clone(),
            fragments: frags.to_vec(),
            attr,
            properties,
            sub_errors: Vec::new(),
        });
    }

    /// `Validator#validate`: every implemented attribute, in schema order.
    fn validate(
        &mut self,
        ctx: &Ctx<'s>,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let key = (ctx.node as *const Data, data as *const Data);
        if self.active.len() >= 256 || !self.active.insert(key) {
            return Err(unsupported(
                "schema recursion",
                "non-progressing reference cycle or schema nesting deeper than 256",
            ));
        }
        let result = self.validate_inner(ctx, data, frags);
        self.active.remove(&key);
        result
    }

    fn validate_inner(
        &mut self,
        ctx: &Ctx<'s>,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        for (k, v) in ctx.pairs {
            let Data::String(key) = k else { continue };
            match key.as_str() {
                "allOf" | "anyOf" | "oneOf" | "not" | "required" | "minProperties"
                | "maxProperties"
                    if ctx.dialect == Dialect::Draft3 => {}
                "type" => self.type_attr(ctx, v, data, frags)?,
                "allOf" => self.all_of(ctx, v, data, frags)?,
                "anyOf" => self.any_of(ctx, v, data, frags)?,
                "oneOf" => self.one_of(ctx, v, data, frags)?,
                "not" => self.not_attr(ctx, v, data, frags)?,
                "format" => self.format_attr(ctx, v, data, frags)?,
                "maximum" | "minimum" | "minItems" | "maxItems" | "minProperties"
                | "maxProperties" | "minLength" | "maxLength" => {
                    self.limit(ctx, key, v, data, frags)?
                }
                "enum" => self.enum_attr(ctx, v, data, frags)?,
                "properties" => self.properties(ctx, v, data, frags)?,
                "required" => self.required(ctx, v, data, frags)?,
                "additionalProperties" => self.additional_properties(ctx, v, data, frags)?,
                "items" => self.items(ctx, v, data, frags)?,
                "$ref" => self.ref_attr(ctx, v, data, frags)?,
                "extends" if ctx.dialect == Dialect::Draft3 => self.extends(ctx, v, data, frags)?,
                "disallow" if ctx.dialect == Dialect::Draft3 => {
                    self.disallow(ctx, v, data, frags)?
                }
                "disallow" | "extends" => {
                    return Err(unsupported(key, format!("{key} is not reproduced")))
                }
                "const" if ctx.dialect == Dialect::Draft6 => {
                    return Err(unsupported(key, "const is not reproduced"))
                }
                "uniqueItems" if matches!(data, Data::Array(_)) => {
                    return Err(unsupported(key, "uniqueItems is not reproduced"))
                }
                "additionalItems"
                    if matches!(data, Data::Array(_))
                        && matches!(ctx.get("items"), Some(Data::Array(_))) =>
                {
                    return Err(unsupported(key, "additionalItems is not reproduced"))
                }
                "multipleOf" if matches!(data, Data::Integer(_) | Data::Float(_)) => {
                    return Err(unsupported(key, "multipleOf is not reproduced"))
                }
                "pattern" => self.pattern_attr(ctx, v, data, frags)?,
                "patternProperties" | "dependencies" if matches!(data, Data::Hash(_)) => {
                    return Err(unsupported(key, format!("{key} is not reproduced")))
                }
                "propertyNames"
                    if ctx.dialect == Dialect::Draft6 && matches!(data, Data::Hash(_)) =>
                {
                    return Err(unsupported(key, "propertyNames is not reproduced"))
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn type_attr(
        &mut self,
        ctx: &Ctx<'s>,
        types: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let (list, union): (Vec<&Data>, bool) = match types {
            Data::Array(items) => (items.iter().collect(), true),
            other => (vec![other], false),
        };
        let mut sub_errors = Vec::new();
        for (i, ty) in list.iter().enumerate() {
            if matches!(ty, Data::Hash(_)) && ctx.dialect == Dialect::Draft3 {
                if !union {
                    continue;
                }
                let child = self.new_schema(ty, ctx.uri.clone(), ctx.dialect)?;
                let pre = self.errors.len();
                self.validate(&child, data, frags)?;
                let errors = self.pop_since(pre);
                if errors.is_empty() {
                    return Ok(());
                }
                sub_errors.push((format!("type #{i}"), errors));
            } else if valid_for_type(data, Some(ty)) {
                return Ok(());
            }
        }
        let names: Vec<String> = list
            .iter()
            .map(|t| match t {
                Data::String(s) => s.clone(),
                _ => "(schema)".to_owned(),
            })
            .collect();
        let message = format!(
            "The property '{}' of type {} did not match {}: {}",
            build_fragment(frags),
            type_of_data(data),
            if union {
                "one or more of the following types"
            } else {
                "the following type"
            },
            names.join(", ")
        );
        self.push(ctx, message, frags, Attr::Type, Vec::new());
        self.errors.last_mut().expect("just pushed").sub_errors = sub_errors;
        Ok(())
    }

    fn extends(
        &mut self,
        ctx: &Ctx<'s>,
        list: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        match list {
            Data::Hash(_) => {
                let child = self.new_schema(list, ctx.uri.clone(), ctx.dialect)?;
                self.validate(&child, data, frags)
            }
            Data::Array(items) => {
                for item in items {
                    let child = self.new_schema(item, ctx.uri.clone(), ctx.dialect)?;
                    self.validate(&child, data, frags)?;
                }
                Ok(())
            }
            _ => Err(unsupported(
                "extends",
                "non-object/non-array extends is not reproduced",
            )),
        }
    }

    fn disallow(
        &mut self,
        ctx: &Ctx<'s>,
        types: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let list: Vec<&Data> = match types {
            Data::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        let mut matched = false;
        for ty in &list {
            if matches!(ty, Data::Hash(_)) {
                let child = self.new_schema(ty, ctx.uri.clone(), ctx.dialect)?;
                let pre = self.errors.len();
                self.validate(&child, data, frags)?;
                matched |= self.pop_since(pre).is_empty();
            } else {
                matched |= valid_for_type(data, Some(ty));
            }
        }
        if matched {
            let names = list
                .iter()
                .map(|t| match t {
                    Data::String(s) => s.as_str(),
                    _ => "(schema)",
                })
                .collect::<Vec<_>>()
                .join(", ");
            self.push(
                ctx,
                format!(
                    "The property '{}' matched one or more of the following types: {names}",
                    build_fragment(frags)
                ),
                frags,
                Attr::Type,
                Vec::new(),
            );
        }
        Ok(())
    }

    fn required(
        &mut self,
        ctx: &Ctx<'s>,
        req: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Hash(pairs) = data else {
            return Ok(());
        };
        let Data::Array(names) = req else {
            return Err(unsupported(
                "required",
                "a non-array required is not reproduced",
            ));
        };
        for name in names {
            let name = name.ruby_to_s()?;
            if has_key(pairs, &name) {
                continue;
            }
            let message = format!(
                "The property '{}' did not contain a required property of '{name}'",
                build_fragment(frags)
            );
            self.push(ctx, message, frags, Attr::Required, Vec::new());
        }
        Ok(())
    }

    fn properties(
        &mut self,
        ctx: &Ctx<'s>,
        props: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Hash(pairs) = data else {
            return Ok(());
        };
        let Data::Hash(schema_props) = props else {
            return Err(unsupported(
                "properties",
                "a non-object properties is not reproduced",
            ));
        };
        for (name, sub) in schema_props {
            let name = name.ruby_to_s()?;
            let present = get(pairs, &name);
            let required = if ctx.dialect == Dialect::Draft3 {
                let Data::Hash(sub) = sub else {
                    return Err(unsupported("properties", "a non-object property schema"));
                };
                get(sub, "required").map_or(self.strict, Data::truthy)
            } else {
                self.strict
            };
            if required && present.is_none() {
                let message = format!(
                    "The property '{}' did not contain a required property of '{name}'",
                    build_fragment(frags)
                );
                self.push(ctx, message, frags, Attr::Properties, Vec::new());
            }
            if let Some(value) = present {
                let child = self.new_schema(sub, ctx.uri.clone(), ctx.dialect)?;
                let mut f = frags.to_vec();
                f.push(name);
                self.validate(&child, value, &f)?;
            }
        }
        if !self.strict || has_key(ctx.pairs, "additionalProperties") {
            return Ok(());
        }
        if ctx.get("patternProperties").is_some() {
            return Err(unsupported(
                "patternProperties",
                "strict mode with patternProperties needs Ruby regular expressions; not reproduced",
            ));
        }
        let diff: Vec<String> = pairs
            .iter()
            .filter_map(|(k, _)| match k {
                Data::String(s) if !has_key(schema_props, s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        if !diff.is_empty() {
            let message = format!(
                "The property '{}' contained undefined properties: '{}'",
                build_fragment(frags),
                diff.join(", ")
            );
            self.push(ctx, message, frags, Attr::Properties, diff);
        }
        Ok(())
    }

    fn additional_properties(
        &mut self,
        ctx: &Ctx<'s>,
        addprop: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Hash(pairs) = data else {
            return Ok(());
        };
        match ctx.get("type") {
            None | Some(Data::Null) => {}
            Some(Data::String(t)) if t == "object" => {}
            Some(_) => return Ok(()),
        }
        if ctx
            .get("patternProperties")
            .is_some_and(|v| !matches!(v, Data::Null))
        {
            return Err(unsupported(
                "patternProperties",
                "additionalProperties with patternProperties needs Ruby regular expressions; not reproduced",
            ));
        }
        if ctx.get("extends").is_some_and(Data::truthy) {
            return Err(unsupported("extends", "extends is not reproduced"));
        }
        let mut extra: Vec<String> = pairs
            .iter()
            .filter_map(|(k, _)| match k {
                Data::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        match ctx.get("properties") {
            Some(Data::Hash(props)) => extra.retain(|k| !has_key(props, k)),
            Some(Data::Null | Data::Bool(false)) | None => {}
            Some(_) => {
                return Err(unsupported(
                    "properties",
                    "a non-object properties is not reproduced",
                ))
            }
        }
        if let Data::Hash(_) = addprop {
            for key in &extra {
                let child = self.new_schema(addprop, ctx.uri.clone(), ctx.dialect)?;
                let value = get(pairs, key).expect("extra key present");
                let mut f = frags.to_vec();
                f.push(key.clone());
                self.validate(&child, value, &f)?;
            }
            extra.clear();
        }
        if !extra.is_empty() && matches!(addprop, Data::Bool(false)) {
            let listed: Vec<String> = extra.iter().map(|k| super::data::inspect_str(k)).collect();
            let message = format!(
                "The property '{}' contains additional properties [{}] outside of the schema when none are allowed",
                build_fragment(frags),
                listed.join(", ")
            );
            self.push(ctx, message, frags, Attr::AdditionalProperties, Vec::new());
        }
        Ok(())
    }

    fn items(
        &mut self,
        ctx: &Ctx<'s>,
        items: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Array(values) = data else {
            return Ok(());
        };
        match items {
            Data::Hash(_) => {
                let child = self.new_schema(items, ctx.uri.clone(), ctx.dialect)?;
                for (i, value) in values.iter().enumerate() {
                    let mut f = frags.to_vec();
                    f.push(i.to_string());
                    self.validate(&child, value, &f)?;
                }
            }
            Data::Array(schemas) => {
                for (i, sub) in schemas.iter().enumerate() {
                    if i >= values.len() {
                        break;
                    }
                    let child = self.new_schema(sub, ctx.uri.clone(), ctx.dialect)?;
                    let mut f = frags.to_vec();
                    f.push(i.to_string());
                    self.validate(&child, &values[i], &f)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn ref_attr(
        &mut self,
        ctx: &Ctx<'s>,
        r: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        self.check_local_ref(r)?;
        let Data::String(s) = r else { unreachable!() };
        let resolved = uri::resolve(&ctx.uri, s)?;
        let (name, fragment) = resolved.split_once('#').unwrap_or((&resolved, ""));
        let fragment = uri::unescape(fragment);
        let (mut target, dialect) = self.schemas.borrow().get(name).copied().ok_or_else(|| {
            unsupported(
                "schema resource",
                format!("unregistered schema resource {name:?}"),
            )
        })?;
        let target_uri = self
            .locations
            .borrow()
            .get(&(target as *const Data))
            .cloned()
            .unwrap_or_else(|| self.root_uri.clone());
        let diagnostic_uri = if target_uri.as_ref() == name
            && !name.contains("://")
            && super::resources::local_path(name).is_ok_and(|p| p.is_file())
        {
            super::resources::file_uri(&super::resources::local_path(name)?)
        } else {
            target_uri.to_string()
        };
        let mut path = String::new();
        let mut string_target: Option<String> = None;
        for seg in fragment.split('/') {
            if seg.is_empty() {
                continue;
            }
            let seg = seg.replace("~0", "~").replace("~1", "/");
            let next = if let Some(text) = &string_target {
                if text.contains(&seg) {
                    string_target = Some(seg.clone());
                    Some(target)
                } else {
                    None
                }
            } else {
                match target {
                    Data::Array(items) => {
                        let i = to_i(&seg);
                        let idx = if i < 0 { items.len() as i64 + i } else { i };
                        usize::try_from(idx).ok().and_then(|i| items.get(i))
                    }
                    Data::Hash(pairs) => get(pairs, &seg),
                    Data::Integer(_) => {
                        return Err(SchemaError::Ruby {
                            class: "TypeError".into(),
                            message: "no implicit conversion of String into Integer".into(),
                        });
                    }
                    Data::String(text) => {
                        // Ruby String#[] with a string argument is a substring lookup.
                        if text.contains(&seg) {
                            string_target = Some(seg.clone());
                            Some(target)
                        } else {
                            None
                        }
                    }
                    other => {
                        let receiver = match other {
                            Data::Float(_) => "an instance of Float",
                            Data::Bool(true) => "true",
                            Data::Bool(false) => "false",
                            Data::Null => "nil",
                            Data::Symbol(_) => "an instance of Symbol",
                            Data::Date { .. } => "an instance of Date",
                            Data::Time(_) => "an instance of Time",
                            _ => unreachable!(),
                        };
                        return Err(SchemaError::Ruby {
                            class: "NoMethodError".into(),
                            message: format!("undefined method '[]' for {receiver}"),
                        });
                    }
                }
            };
            path.push('/');
            path.push_str(&seg);
            match next {
                Some(n) if !matches!(n, Data::Null) => target = n,
                _ => {
                    return Err(SchemaError::Schema {
                        message: format!(
                            "The fragment '{path}' does not exist on schema {}",
                            diagnostic_uri
                        ),
                    })
                }
            }
        }
        if string_target.is_some() {
            return Err(SchemaError::Ruby {
                class: "NoMethodError".into(),
                message: "undefined method 'each' for an instance of String".into(),
            });
        }
        let uri = if resolved.contains('#') {
            format!("{}#", target_uri.trim_end_matches('#'))
        } else {
            target_uri.to_string()
        };
        let mut child = self.new_schema(target, Rc::from(uri.as_str()), dialect)?;
        // The referenced fragment keeps the resource URI, unless it declares an ID.
        if !matches!(child.get("id"), Some(Data::String(id)) if !id.is_empty()) {
            child.uri = Rc::from(uri);
        }
        self.validate(&child, data, frags)
    }

    fn sub_schemas(
        &self,
        ctx: &Ctx<'s>,
        list: &'s Data,
        key: &str,
    ) -> Result<Vec<Ctx<'s>>, SchemaError> {
        let Data::Array(items) = list else {
            return Err(unsupported(
                key,
                format!("a non-array {key} is not reproduced"),
            ));
        };
        items
            .iter()
            .map(|item| self.new_schema(item, ctx.uri.clone(), ctx.dialect))
            .collect()
    }

    /// Pop the errors recorded since `pre` (last first, as the gem does).
    fn pop_since(&mut self, pre: usize) -> Vec<VErr> {
        let mut popped = Vec::new();
        while self.errors.len() > pre {
            popped.push(self.errors.pop().expect("len checked"));
        }
        popped
    }

    fn one_of(
        &mut self,
        ctx: &Ctx<'s>,
        list: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let schemas = self.sub_schemas(ctx, list, "oneOf")?;
        let mut sub: Vec<(String, Vec<VErr>)> = Vec::new();
        let mut failures = 0usize;
        for (i, child) in schemas.iter().enumerate() {
            let pre = self.errors.len();
            self.validate(child, data, frags)?;
            let popped = self.pop_since(pre);
            if !popped.is_empty() {
                failures += 1;
                sub.push((format!("oneOf #{i}"), popped));
            }
        }
        if schemas.len().checked_sub(1) == Some(failures) {
            return Ok(());
        }
        let message = if failures == schemas.len() {
            format!(
                "The property '{}' of type {} did not match any of the required schemas",
                build_fragment(frags),
                type_of_data(data)
            )
        } else {
            format!(
                "The property '{}' of type {} matched more than one of the required schemas",
                build_fragment(frags),
                type_of_data(data)
            )
        };
        self.push(ctx, message, frags, Attr::OneOf, Vec::new());
        self.errors.last_mut().expect("pushed").sub_errors = sub;
        Ok(())
    }

    fn any_of(
        &mut self,
        ctx: &Ctx<'s>,
        list: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let schemas = self.sub_schemas(ctx, list, "anyOf")?;
        let mut sub: Vec<(String, Vec<VErr>)> = Vec::new();
        let mut valid = false;
        for (i, child) in schemas.iter().enumerate() {
            let pre = self.errors.len();
            self.validate(child, data, frags)?;
            let popped = self.pop_since(pre);
            valid = popped.is_empty();
            if !popped.is_empty() {
                sub.push((format!("anyOf #{i}"), popped));
            }
            if valid {
                break;
            }
        }
        if !valid {
            let message = format!(
                "The property '{}' of type {} did not match one or more of the required schemas",
                build_fragment(frags),
                type_of_data(data)
            );
            self.push(ctx, message, frags, Attr::AnyOf, Vec::new());
            self.errors.last_mut().expect("pushed").sub_errors = sub;
        }
        Ok(())
    }

    fn all_of(
        &mut self,
        ctx: &Ctx<'s>,
        list: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let schemas = self.sub_schemas(ctx, list, "allOf")?;
        let mut sub: Vec<(String, Vec<VErr>)> = Vec::new();
        for (i, child) in schemas.iter().enumerate() {
            let pre = self.errors.len();
            self.validate(child, data, frags)?;
            let popped = self.pop_since(pre);
            if !popped.is_empty() {
                sub.push((format!("allOf #{i}"), popped));
            }
        }
        let all: Vec<&VErr> = sub.iter().flat_map(|(_, e)| e.iter()).collect();
        let mut common: Vec<String> = Vec::new();
        if self.strict && !sub.is_empty() {
            let mut sets = all.iter().map(|e| &e.properties);
            if let Some(first) = sets.next() {
                common = first.clone();
                common.dedup();
                for s in sets {
                    common.retain(|p| s.contains(p));
                }
            }
        }
        let non_missing = all.iter().any(|e| e.attr != Attr::Properties);
        if non_missing || !common.is_empty() {
            let message = format!(
                "The property '{}' of type {} did not match all of the required schemas",
                build_fragment(frags),
                type_of_data(data)
            );
            self.push(ctx, message, frags, Attr::AllOf, Vec::new());
            self.errors.last_mut().expect("pushed").sub_errors = sub;
        }
        Ok(())
    }

    fn not_attr(
        &mut self,
        ctx: &Ctx<'s>,
        schema: &'s Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let child = self.new_schema(schema, ctx.uri.clone(), ctx.dialect)?;
        let pre = self.errors.len();
        self.validate(&child, data, frags)?;
        if self.errors.len() != pre {
            self.errors.truncate(pre);
        } else {
            let message = format!(
                "The property '{}' of type {} matched the disallowed schema",
                build_fragment(frags),
                type_of_data(data)
            );
            self.push(ctx, message, frags, Attr::Not, Vec::new());
        }
        Ok(())
    }

    fn enum_attr(
        &mut self,
        ctx: &Ctx<'s>,
        list: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::Array(values) = list else {
            return Err(unsupported("enum", "a non-array enum is not reproduced"));
        };
        // build_schemas turned enums of walked schemas into an ArraySet;
        // the others keep Array#include? (==).
        let found = if self.walked.contains(&(ctx.node as *const Data)) {
            let probe = array_set_key(data);
            values.iter().any(|v| array_set_key(v).eql(&probe))
        } else {
            values.iter().any(|v| v.ruby_eq(data))
        };
        if found {
            return Ok(());
        }
        let mut names = Vec::with_capacity(values.len());
        for v in values {
            names.push(match v {
                Data::Null => "null".to_owned(),
                Data::Array(_) => "array".to_owned(),
                Data::Hash(_) => "object".to_owned(),
                other => other.ruby_to_s()?,
            });
        }
        let message = format!(
            "The property '{}' value {} did not match one of the following values: {}",
            build_fragment(frags),
            data.ruby_inspect()?,
            names.join(", ")
        );
        self.push(ctx, message, frags, Attr::Enum, Vec::new());
        Ok(())
    }

    fn format_attr(
        &mut self,
        ctx: &Ctx<'s>,
        format: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        if !valid_for_type(data, ctx.get("type")) {
            return Ok(());
        }
        let Data::String(text) = data else {
            return Ok(());
        };
        match format.ruby_to_s()?.as_str() {
            "uri" => {
                if text.contains(['\n', '\r']) {
                    return Err(unsupported(
                        "format: uri",
                        "Addressable line-anchor behaviour for multi-line strings is not reproduced",
                    ));
                }
                if !uri::addressable_parses(text) {
                    let message = format!(
                        "The property '{}' must be a valid URI",
                        build_fragment(frags)
                    );
                    self.push(ctx, message, frags, Attr::Format, Vec::new());
                }
            }
            "date-time" if !super::date_time::valid(text) => {
                let message = format!(
                    "The property '{}' must be a valid RFC3339 date/time string",
                    build_fragment(frags)
                );
                self.push(ctx, message, frags, Attr::Format, Vec::new());
            }
            f @ ("ipv4" | "ipv6") if !super::ip::valid(text, f == "ipv6")? => {
                let version = if f == "ipv6" { 6 } else { 4 };
                let message = format!(
                    "The property '{}' must be a valid IPv{version} address",
                    build_fragment(frags)
                );
                self.push(ctx, message, frags, Attr::Format, Vec::new());
            }
            _ => {}
        }
        Ok(())
    }

    /// `PatternAttribute`: `Regexp.new(pattern).match(data)` for a String,
    /// with Ruby's Onigmo. A pattern Ruby would not compile is refused.
    fn pattern_attr(
        &mut self,
        ctx: &Ctx<'s>,
        pattern: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let Data::String(text) = data else {
            return Ok(());
        };
        let Data::String(source) = pattern else {
            return Err(unsupported(
                "pattern",
                "Regexp.new with a non-String pattern is not reproduced",
            ));
        };
        let regex = OnigRegex::compile(source, 0).map_err(|e| {
            unsupported(
                "pattern",
                format!("Regexp.new raises for {source:?} ({e}); not reproduced"),
            )
        })?;
        let Some(matched) = regex.is_match(text) else {
            return Err(unsupported(
                "pattern",
                "Onigmo reported a match error; not reproduced",
            ));
        };
        if !matched {
            let message = format!(
                "The property '{}' value {} did not match the regex '{}'",
                build_fragment(frags),
                data.ruby_inspect()?,
                source
            );
            self.push(ctx, message, frags, Attr::Pattern, Vec::new());
        }
        Ok(())
    }

    fn limit(
        &mut self,
        ctx: &Ctx<'s>,
        name: &str,
        limit: &Data,
        data: &Data,
        frags: &[String],
    ) -> Result<(), SchemaError> {
        let value = match (name, data) {
            ("maximum" | "minimum", Data::Integer(_) | Data::Float(_)) => data.clone(),
            ("minItems" | "maxItems", Data::Array(a)) => Data::Integer(a.len().into()),
            ("minProperties" | "maxProperties", Data::Hash(h)) => Data::Integer(h.len().into()),
            ("minLength" | "maxLength", Data::String(s)) => Data::Integer(s.chars().count().into()),
            _ => return Ok(()),
        };
        let exclusive = match name {
            "minimum" => ctx.get("exclusiveMinimum").is_some_and(Data::truthy),
            "maximum" => ctx.get("exclusiveMaximum").is_some_and(Data::truthy),
            _ => false,
        };
        let Some(ord) = num_cmp(&value, limit) else {
            return Err(unsupported(
                name,
                "a non-numeric or NaN limit is not reproduced",
            ));
        };
        use std::cmp::Ordering::*;
        let invalid = if name.starts_with("max") {
            if exclusive {
                ord != Less
            } else {
                ord == Greater
            }
        } else if exclusive {
            ord != Greater
        } else {
            ord == Less
        };
        if !invalid {
            return Ok(());
        }
        let shown = limit.ruby_to_s()?;
        let description = match name {
            "minimum" | "maximum" => format!(
                "did not have a {name} value of {shown}, {}",
                if exclusive {
                    "exclusively"
                } else {
                    "inclusively"
                }
            ),
            "minItems" => format!("did not contain a minimum number of items {shown}"),
            "maxItems" => format!("had more items than the allowed {shown}"),
            "minProperties" => format!("did not contain a minimum number of properties {shown}"),
            "maxProperties" => format!("had more properties than the allowed {shown}"),
            "minLength" => format!("was not of a minimum string length of {shown}"),
            _ => format!("was not of a maximum string length of {shown}"),
        };
        let message = format!("The property '{}' {description}", build_fragment(frags));
        self.push(ctx, message, frags, Attr::Limit, Vec::new());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_error_rendering_matches_gem() {
        let leaf = VErr {
            message: "The property '#/c' of type array did not match the following type: string"
                .into(),
            uri: Rc::from("u"),
            fragments: vec!["c".into()],
            attr: Attr::Type,
            properties: vec![],
            sub_errors: vec![],
        };
        let top = VErr {
            message: "The property '#/c' of type array did not match any of the required schemas"
                .into(),
            uri: Rc::from("u"),
            fragments: vec!["c".into()],
            attr: Attr::OneOf,
            properties: vec![],
            sub_errors: vec![("oneOf #0".into(), vec![leaf])],
        };
        assert_eq!(
            top.to_string_at(0),
            "The property '#/c' of type array did not match any of the required schemas. The schema specific errors were:\n\n- oneOf #0:\n    - The property '#/c' of type array did not match the following type: string"
        );
    }
}
