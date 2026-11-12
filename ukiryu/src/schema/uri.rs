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
fn host_of(authority: &str) -> &str {
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
fn port_start(s: &str) -> Option<usize> {
    let i = s.rfind(':')?;
    (!s[i + 1..].contains(['@', '[', ']'])).then_some(i)
}

fn percent_decode(s: &str) -> Vec<u8> {
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
