//! The parts of Addressable 2.9.2 (through `JSON::Util::URI`) that schema
//! validation observes: component splitting (for `$schema` lookup and
//! `$ref`) and whether `Addressable::URI.parse` raises (the `uri` format).

/// Components split by Addressable's RFC 3986 regex.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Parts<'a> {
    pub scheme: Option<&'a str>,
    pub authority: Option<&'a str>,
    pub path: &'a str,
    pub fragment: Option<&'a str>,
}

/// `URIREGEX` split. Callers reject strings containing a line break first:
/// Ruby's `^`/`$` are line anchors and that case is not reproduced.
pub(crate) fn split(s: &str) -> Parts<'_> {
    let mut rest = s;
    let run = rest.find([':', '/', '?', '#']).unwrap_or(rest.len());
    let scheme = if run > 0 && rest[run..].starts_with(':') {
        let sch = &rest[..run];
        rest = &rest[run + 1..];
        Some(sch)
    } else {
        None
    };
    let authority = if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        rest = &after[end..];
        Some(&after[..end])
    } else {
        None
    };
    let pend = rest.find(['?', '#']).unwrap_or(rest.len());
    let path = &rest[..pend];
    rest = &rest[pend..];
    if rest.starts_with('?') {
        let qend = rest.find('#').unwrap_or(rest.len());
        rest = &rest[qend..];
    }
    let fragment = rest.strip_prefix('#');
    Parts {
        scheme,
        authority,
        path,
        fragment,
    }
}

/// Host part of an authority: `sub(/^([^\[\]]*)@/, '')` then
/// `sub(/:([^:@\[\]]*?)$/, '')`.
pub(super) fn host_of(authority: &str) -> &str {
    let without_userinfo = userinfo_end(authority).map_or(authority, |i| &authority[i + 1..]);
    match port_start(without_userinfo) {
        Some(i) => &without_userinfo[..i],
        None => without_userinfo,
    }
}

/// Index of the `@` closing `^([^\[\]]*)@` (greedy: the last `@` before
/// any bracket).
fn userinfo_end(authority: &str) -> Option<usize> {
    let limit = authority.find(['[', ']']).unwrap_or(authority.len());
    authority[..limit].rfind('@')
}

/// Leftmost `:` from which `[^:@\[\]]*` reaches the end.
pub(super) fn port_start(s: &str) -> Option<usize> {
    let i = s.rfind(':')?;
    (!s[i + 1..].contains(['@', '[', ']'])).then_some(i)
}

pub(super) fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Percent-decoding as `Addressable::URI.unencode` (used for fragments).
pub(crate) fn unescape(s: &str) -> String {
    String::from_utf8_lossy(&percent_decode(s)).into_owned()
}

const IP_BASED: [&str; 13] = [
    "http", "https", "ftp", "tftp", "sftp", "ssh", "svn+ssh", "telnet", "nntp", "gopher", "wais",
    "ldap", "prospero",
];

/// Whether `Addressable::URI.parse(s)` succeeds (no line breaks; see
/// [`split`]).
pub(crate) fn addressable_parses(s: &str) -> bool {
    let p = split(s);
    if let Some(scheme) = p.scheme {
        let mut chars = scheme.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
        if !first_ok || !chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return false;
        }
    }
    let host = p.authority.map(host_of);
    if let Some(authority) = p.authority {
        if let Some(i) = port_start(authority) {
            let port = &authority[i + 1..];
            if !port.is_empty() {
                let decoded = percent_decode(port);
                match String::from_utf8(decoded) {
                    Ok(text) if !text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()) => {}
                    _ => return false,
                }
            }
        }
    }
    // path= prefixes "/" when a host is present.
    let mut path = p.path.to_owned();
    if !path.is_empty() && !path.starts_with('/') && host.is_some() {
        path.insert(0, '/');
    }
    if let Some(scheme) = p.scheme {
        if IP_BASED.contains(&scheme.to_ascii_lowercase().as_str())
            && host.is_none_or(str::is_empty)
            && path.is_empty()
        {
            return false;
        }
    }
    if host.is_none() && path.starts_with("//") {
        return false;
    }
    if let Some(h) = host {
        if h.chars()
            .any(|c| "<>{}/\\?#@\"".contains(c) || c.is_whitespace())
        {
            return false;
        }
        if h.len() >= 2 && h.starts_with('[') && h.ends_with(']') {
            let inner = &h[1..h.len() - 1];
            if !inner
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:".contains(c))
            {
                return false;
            }
        }
    }
    // to_s: "Cannot assemble URI string with ambiguous path"
    // (NORMPATH = /^(?!\/)[^\/:]*:.*$/ with no scheme).
    if p.scheme.is_none() && !path.is_empty() && !path.starts_with('/') {
        if let Some(colon) = path.find(':') {
            if !path[..colon].contains('/') {
                return false;
            }
        }
    }
    true
}

/// Resolve a schema URI reference. Relative schema identifiers stay relative,
/// as Addressable does when the parent is a generated UUID, not a file URI.
pub(super) fn resolve(base: &str, reference: &str) -> Result<String, super::SchemaError> {
    if !addressable_parses(reference) {
        let p = split(reference);
        let message = if let Some(scheme) = p.scheme.filter(|scheme| {
            let mut chars = scheme.chars();
            !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                || !chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        }) {
            format!("Invalid scheme format: '{scheme}'")
        } else if let Some(host) = p.authority.map(host_of).filter(|h| {
            h.chars()
                .any(|c| "<>{}/\\?#@\"".contains(c) || c.is_whitespace())
        }) {
            format!("Invalid character in host: '{host}'")
        } else if let Some(port) = p.authority.and_then(|a| port_start(a).map(|i| &a[i + 1..])) {
            format!("Invalid port number: {}", super::data::inspect_str(port))
        } else if p
            .scheme
            .is_some_and(|s| IP_BASED.contains(&s.to_ascii_lowercase().as_str()))
            && p.authority.is_none_or(str::is_empty)
            && p.path.is_empty()
        {
            format!("Absolute URI missing hierarchical segment: '{reference}'")
        } else if p.scheme.is_none() && !p.path.starts_with('/') && p.path.contains(':') {
            format!("Cannot assemble URI string with ambiguous path: '{reference}'")
        } else {
            return Err(super::SchemaError::unsupported(
                "schema URI",
                "unmeasured Addressable URI diagnostic",
            ));
        };
        return Err(super::SchemaError::Ruby {
            class: "JSON::Schema::UriError".into(),
            message,
        });
    }
    let r = split(reference);
    if r.scheme.is_some() {
        return Ok(reference.to_owned());
    }
    let b = split(base);
    if reference.starts_with('#') {
        return Ok(format!(
            "{}{reference}",
            base.split('#').next().unwrap_or(base)
        ));
    }
    let scheme = b.scheme.map(|s| format!("{s}:")).unwrap_or_default();
    if reference.starts_with("//") {
        return Ok(format!("{scheme}{reference}"));
    }
    let authority = b.authority.map(|a| format!("//{a}")).unwrap_or_default();
    let suffix = &reference[r.path.len()..];
    let path = if r.path.starts_with('/') {
        r.path.to_owned()
    } else if r.path.is_empty() {
        b.path.to_owned()
    } else {
        let parent = b
            .path
            .rsplit_once('/')
            .map(|(p, _)| format!("{p}/"))
            .unwrap_or_else(|| {
                if b.authority.is_some() {
                    "/".into()
                } else {
                    String::new()
                }
            });
        format!("{parent}{}", r.path)
    };
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|s| *s != "..") => {
                segments.pop();
            }
            ".." if !absolute => segments.push(segment),
            ".." => {}
            _ => segments.push(segment),
        }
    }
    let mut path = format!("{}{}", if absolute { "/" } else { "" }, segments.join("/"));
    if trailing && !path.ends_with('/') {
        path.push('/');
    }
    Ok(format!("{scheme}{authority}{path}{suffix}"))
}

/// json-schema discards the fragment's contents on schema IDs, retaining '#'.
pub(super) fn schema_id(base: &str, id: &str) -> Result<String, super::SchemaError> {
    if id.is_empty() {
        return Ok(base.into());
    }
    let resolved = resolve(base, id)?;
    Ok(match resolved.split_once('#') {
        Some((uri, _)) => format!("{uri}#"),
        None => resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_uriregex() {
        let p = split("http://u@h:80/p?q#f");
        assert_eq!(p.scheme, Some("http"));
        assert_eq!(p.authority, Some("u@h:80"));
        assert_eq!(p.path, "/p");
        assert_eq!(p.fragment, Some("f"));
        let p = split("#/definitions/x");
        assert_eq!((p.scheme, p.authority, p.path), (None, None, ""));
        assert_eq!(p.fragment, Some("/definitions/x"));
    }
}
