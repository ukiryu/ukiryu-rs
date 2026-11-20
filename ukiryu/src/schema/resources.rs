//! Resolve local and native HTTP schema resources before preparing data.
use super::{data::Data, json, transport, uri, SchemaError};
use std::collections::HashSet;
use std::path::PathBuf;

fn get<'a>(pairs: &'a [(Data, Data)], key: &str) -> Option<&'a Data> {
    pairs
        .iter()
        .find(|(k, _)| matches!(k,Data::String(s) if s == key))
        .map(|(_, v)| v)
}

fn scan(
    node: &Data,
    base: &str,
    ids: &mut HashSet<String>,
    refs: &mut Vec<(String, bool)>,
) -> Result<(), SchemaError> {
    let Data::Hash(pairs) = node else {
        return Ok(());
    };
    let base = match get(pairs, "id") {
        Some(Data::String(id)) => uri::schema_id(base, id)?,
        _ => base.into(),
    };
    ids.insert(base.split('#').next().unwrap_or(&base).into());
    if let Some(Data::String(r)) = get(pairs, "$ref") {
        // Fragment-only references never load a resource.
        if !r.starts_with('#') && !r.is_empty() {
            refs.push((uri::resolve(&base, r)?, false));
        }
    }
    if let Some(Data::String(reference)) = get(pairs, "extends") {
        refs.push((uri::resolve(&base, reference)?, true));
    }
    for key in ["definitions", "properties", "patternProperties"] {
        if let Some(Data::Hash(pairs)) = get(pairs, key) {
            for (_, child) in pairs {
                scan(child, &base, ids, refs)?;
            }
        }
    }
    for key in [
        "type",
        "disallow",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "extends",
        "items",
        "additionalProperties",
        "additionalItems",
        "dependencies",
    ] {
        match get(pairs, key) {
            Some(Data::Array(items)) => {
                for child in items {
                    scan(child, &base, ids, refs)?;
                }
            }
            Some(child @ Data::Hash(_)) => scan(child, &base, ids, refs)?,
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn local_path(reference: &str) -> Result<PathBuf, SchemaError> {
    // Native Windows drive paths are paths, rather than URI schemes.
    if cfg!(windows) && PathBuf::from(reference).is_absolute() && !reference.starts_with("file:") {
        return Ok(PathBuf::from(reference));
    }
    let parts = uri::split(reference);
    if parts
        .scheme
        .is_some_and(|s| !s.eq_ignore_ascii_case("file"))
        || parts
            .authority
            .is_some_and(|s| !s.is_empty() && s != "localhost")
    {
        return Err(SchemaError::unsupported(
            "schema transport",
            format!("non-file schema path {reference:?} is not supported"),
        ));
    }
    let decoded = uri::unescape(parts.path);
    let decoded =
        if cfg!(windows) && decoded.starts_with('/') && decoded.as_bytes().get(2) == Some(&b':') {
            &decoded[1..]
        } else {
            &decoded
        };
    let path = PathBuf::from(decoded);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| SchemaError::unsupported("schema working directory", e.to_string()))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

pub(super) fn read(reference: &str) -> Result<Data, SchemaError> {
    if transport::is_http(reference) {
        let text = transport::read(reference)?;
        return json::parse(&text).ok_or_else(|| {
            SchemaError::unsupported("schema JSON parse", "invalid JSON schema response")
        });
    }
    let path = local_path(reference)?;
    let text = std::fs::read_to_string(&path).map_err(|_| SchemaError::ReadFailed {
        message: format!("Read of file at {} failed", path_text(&path)),
    })?;
    json::parse_schema(&text)
}

// A String `extends` is eagerly opened even though its validation attribute
// subsequently reports it as an invalid schema. Non-file URI schemes reach
// Ruby's ordinary file opener rather than JSON::Schema::Reader's file wrapper.
fn read_extension(reference: &str) -> Result<Data, SchemaError> {
    let scheme = uri::split(reference).scheme;
    if scheme.is_some_and(|s| !["file", "http", "https"].contains(&s))
        && !(cfg!(windows) && PathBuf::from(reference).is_absolute())
    {
        let text = super::read_text(std::path::Path::new(reference))?;
        return json::parse_schema(&text);
    }
    read(reference)
}

pub(super) fn load(root: &Data, base: &str) -> Result<Vec<(String, Data)>, SchemaError> {
    load_with(root, base, |name, extension| {
        if extension {
            read_extension(name)
        } else {
            read(name)
        }
    })
}

pub(super) fn load_with(
    root: &Data,
    base: &str,
    read: impl Fn(&str, bool) -> Result<Data, SchemaError>,
) -> Result<Vec<(String, Data)>, SchemaError> {
    let mut documents: Vec<(String, Data)> = Vec::new();
    loop {
        let mut ids = HashSet::new();
        let mut refs = Vec::new();
        scan(root, base, &mut ids, &mut refs)?;
        for (name, node) in &documents {
            ids.insert(name.clone());
            scan(node, name, &mut ids, &mut refs)?;
        }
        let missing = refs
            .iter()
            .map(|(r, extension)| (r.split('#').next().unwrap_or(r), *extension))
            .find(|(r, _)| !ids.contains(*r));
        let Some((name, extension)) = missing else {
            return Ok(documents);
        };
        if documents.len() >= 128 {
            return Err(SchemaError::unsupported(
                "schema resources",
                "more than 128 external schema documents",
            ));
        }
        documents.push((name.into(), read(name, extension)?));
    }
}

/// Native paths and file URIs have distinct spellings on Windows.
pub(super) fn path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn file_uri(path: &std::path::Path) -> String {
    let text = path_text(path);
    format!(
        "file://{}{}",
        if text.starts_with('/') { "" } else { "/" },
        text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_extends_uses_the_supplied_resource_loader() {
        let root = json::parse_schema(r#"{"id":"urn:root","extends":"urn:external"}"#).unwrap();
        let error = load_with(&root, "urn:root", |name, extension| {
            assert_eq!(name, "urn:external");
            assert!(extension);
            Err(SchemaError::unsupported(
                "snapshot schema resource",
                "denied",
            ))
        })
        .unwrap_err();
        assert!(matches!(error, SchemaError::Unsupported { feature, .. }
            if feature == "snapshot schema resource"));
    }
}
