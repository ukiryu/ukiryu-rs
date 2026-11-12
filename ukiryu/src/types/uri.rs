//! `URI.parse(value).to_s` as `Ukiryu::Type` uses it for `:uri` values.
//!
//! Source: Ruby's `uri` gem 1.1.2 (the version the pinned oracle loads):
//! `URI::RFC3986_Parser#split` (`RFC3986_URI` then `RFC3986_relative_ref`),
//! `URI.for`, `URI::Generic#initialize`/`#to_s` and the scheme classes HTTP,
//! HTTPS, WS, WSS, FTP, File, MailTo, LDAP and LDAPS. The regular expressions
//! are reproduced as explicit matchers; their possessive quantifiers make
//! every component boundary deterministic except inside IP literals, where
//! [`ip_literal_ok`] explores all alternatives like the backtracking engine.
use super::error::{Error, ErrorClass};
use super::ruby::{dump, string_inspect};

/// How `URI.parse` failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// `URI::InvalidURIError` with this message (rescued by `Type`).
    Invalid(String),
    /// `URI::InvalidComponentError` (not rescued by `Type`).
    InvalidComponent(String),
}

impl From<UriError> for Error {
    fn from(e: UriError) -> Self {
        match e {
            UriError::Invalid(m) => Error::new(ErrorClass::Validation, m),
            UriError::InvalidComponent(m) => Error::new(ErrorClass::UriInvalidComponent, m),
        }
    }
}

#[derive(Debug, Default)]
struct Split {
    scheme: Option<String>,
    userinfo: Option<String>,
    host: Option<String>,
    port: Option<String>,
    path: Option<String>,
    opaque: Option<String>,
    query: Option<String>,
    fragment: Option<String>,
}

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// `[!$&-.0-9;=A-Z_a-z~]`
fn unreserved_sub(b: u8) -> bool {
    matches!(b, b'!' | b'$' | b'&'..=b'.' | b'0'..=b'9' | b';' | b'=' | b'A'..=b'Z' | b'_' | b'a'..=b'z' | b'~')
}

fn userinfo_char(b: u8) -> bool {
    unreserved_sub(b) || b == b':'
}

/// SEG: `[!$&-.0-9:;=@A-Z_a-z~/]`
fn seg_char(b: u8) -> bool {
    unreserved_sub(b) || matches!(b, b':' | b'@' | b'/')
}

/// SEG_NC: `[!$&-.0-9;=@A-Z_a-z~]`
fn seg_nc_char(b: u8) -> bool {
    unreserved_sub(b) || b == b'@'
}

fn fragment_char(b: u8) -> bool {
    seg_char(b) || b == b'?'
}

/// Possessive `(?:%\h\h|[set])*+` from `pos`; returns the end.
fn run(s: &[u8], mut pos: usize, set: fn(u8) -> bool) -> usize {
    while pos < s.len() {
        let b = s[pos];
        if b == b'%' {
            if pos + 2 < s.len() && is_hex(s[pos + 1]) && is_hex(s[pos + 2]) {
                pos += 3;
                continue;
            }
            break;
        }
        if !set(b) {
            break;
        }
        pos += 1;
    }
    pos
}

// ----------------------------------------------------------------- IP literal

#[derive(Clone)]
enum P {
    Lit(&'static str),
    H16,
    Ipv4,
    Seq(Vec<P>),
    Rep(Box<P>, usize, usize),
    Opt(Box<P>),
    Alt(Vec<P>),
}

fn hc() -> P {
    P::Seq(vec![P::H16, P::Lit(":")])
}

fn ls32() -> P {
    P::Alt(vec![P::Seq(vec![P::H16, P::Lit(":"), P::H16]), P::Ipv4])
}

fn ends(p: &P, s: &[u8], pos: usize) -> Vec<usize> {
    let mut out = match p {
        P::Lit(l) => {
            if s[pos..].starts_with(l.as_bytes()) {
                vec![pos + l.len()]
            } else {
                vec![]
            }
        }
        P::H16 => (1..=4)
            .take_while(|k| pos + k <= s.len() && s[pos..pos + k].iter().all(|b| is_hex(*b)))
            .map(|k| pos + k)
            .collect(),
        P::Ipv4 => {
            let mut cur = dec_octet(s, pos);
            for _ in 0..3 {
                let mut next = vec![];
                for e in cur {
                    if s.get(e) == Some(&b'.') {
                        next.extend(dec_octet(s, e + 1));
                    }
                }
                cur = next;
            }
            cur
        }
        P::Seq(items) => {
            let mut cur = vec![pos];
            for item in items {
                let mut next = vec![];
                for e in cur {
                    next.extend(ends(item, s, e));
                }
                next.sort_unstable();
                next.dedup();
                cur = next;
            }
            cur
        }
        P::Rep(inner, min, max) => {
            let mut all = vec![];
            let mut cur = vec![pos];
            for i in 0..=*max {
                if i >= *min {
                    all.extend(cur.iter().copied());
                }
                if i == *max {
                    break;
                }
                let mut next = vec![];
                for e in &cur {
                    next.extend(ends(inner, s, *e));
                }
                next.sort_unstable();
                next.dedup();
                if next.is_empty() {
                    break;
                }
                cur = next;
            }
            all
        }
        P::Opt(inner) => {
            let mut v = ends(inner, s, pos);
            v.push(pos);
            v
        }
        P::Alt(items) => items.iter().flat_map(|i| ends(i, s, pos)).collect(),
    };
    out.sort_unstable();
    out.dedup();
    out
}

/// `(?<dec-octet>[1-9]\d|1\d{2}|2[0-4]\d|25[0-5]|\d)`, all match ends.
fn dec_octet(s: &[u8], pos: usize) -> Vec<usize> {
    let d = |i: usize| s.get(pos + i).copied().filter(u8::is_ascii_digit);
    let mut out = vec![];
    if let (Some(a), Some(_)) = (d(0), d(1)) {
        if a != b'0' {
            out.push(pos + 2);
        }
    }
    if let (Some(b'1'), Some(_), Some(_)) = (d(0), d(1), d(2)) {
        out.push(pos + 3);
    }
    if let (Some(b'2'), Some(b), Some(_)) = (d(0), d(1), d(2)) {
        if b <= b'4' {
            out.push(pos + 3);
        }
    }
    if let (Some(b'2'), Some(b'5'), Some(c)) = (d(0), d(1), d(2)) {
        if c <= b'5' {
            out.push(pos + 3);
        }
    }
    if d(0).is_some() {
        out.push(pos + 1);
    }
    out
}

fn ipv6_pattern() -> P {
    let pre = |n: usize| P::Opt(Box::new(P::Seq(vec![P::Rep(Box::new(hc()), 0, n), P::H16])));
    P::Alt(vec![
        P::Seq(vec![P::Rep(Box::new(hc()), 6, 6), ls32()]),
        P::Seq(vec![P::Lit("::"), P::Rep(Box::new(hc()), 5, 5), ls32()]),
        // `\h{1,4}?::` is a lazy (not optional) quantifier in Onigmo.
        P::Seq(vec![
            P::H16,
            P::Lit("::"),
            P::Rep(Box::new(hc()), 4, 4),
            ls32(),
        ]),
        P::Seq(vec![
            pre(1),
            P::Lit("::"),
            P::Rep(Box::new(hc()), 3, 3),
            ls32(),
        ]),
        P::Seq(vec![
            pre(2),
            P::Lit("::"),
            P::Rep(Box::new(hc()), 2, 2),
            ls32(),
        ]),
        P::Seq(vec![pre(3), P::Lit("::"), hc(), ls32()]),
        P::Seq(vec![pre(4), P::Lit("::"), ls32()]),
        P::Seq(vec![pre(5), P::Lit("::"), P::H16]),
        P::Seq(vec![pre(6), P::Lit("::")]),
    ])
}

/// The content between `[` and `]`: `IPv6address | IPvFuture`.
fn ip_literal_ok(content: &[u8]) -> bool {
    if ends(&ipv6_pattern(), content, 0).contains(&content.len()) {
        return true;
    }
    // IPvFuture: v\h++\.[!$&-.0-9:;=A-Z_a-z~]++
    let Some(rest) = content.strip_prefix(b"v") else {
        return false;
    };
    let hex = rest.iter().take_while(|b| is_hex(**b)).count();
    if hex == 0 || rest.get(hex) != Some(&b'.') {
        return false;
    }
    let tail = &rest[hex + 1..];
    !tail.is_empty() && tail.iter().all(|b| userinfo_char(*b))
}

/// HOST at `pos`; returns its end. An IP literal must be well formed,
/// otherwise the reg-name alternative matches the empty string.
fn host_end(s: &[u8], pos: usize) -> usize {
    if s.get(pos) == Some(&b'[') {
        if let Some(close) = s[pos + 1..].iter().position(|b| *b == b']') {
            if ip_literal_ok(&s[pos + 1..pos + 1 + close]) {
                return pos + close + 2;
            }
        }
        return pos;
    }
    run(s, pos, unreserved_sub)
}

// ----------------------------------------------------------------- split

fn text(s: &[u8], a: usize, b: usize) -> String {
    String::from_utf8(s[a..b].to_vec()).expect("ASCII input")
}

/// `(?:\?(?<query>[^#]*+))?(?:\#(?<fragment>FRAGMENT))?\z`
fn tail(s: &[u8], mut pos: usize, split: &mut Split) -> bool {
    if s.get(pos) == Some(&b'?') {
        let end = s[pos + 1..]
            .iter()
            .position(|b| *b == b'#')
            .map_or(s.len(), |i| pos + 1 + i);
        split.query = Some(text(s, pos + 1, end));
        pos = end;
    }
    if s.get(pos) == Some(&b'#') {
        let end = run(s, pos + 1, fragment_char);
        split.fragment = Some(text(s, pos + 1, end));
        pos = end;
    }
    pos == s.len()
}

/// `//authority path-abempty`; `relative` applies `(?<host>…(?<!/))?`.
fn authority(s: &[u8], start: usize, relative: bool) -> Option<Split> {
    let mut sp = Split::default();
    let mut pos = start;
    let ui_end = run(s, pos, userinfo_char);
    if s.get(ui_end) == Some(&b'@') {
        sp.userinfo = Some(text(s, pos, ui_end));
        pos = ui_end + 1;
    }
    let h_end = host_end(s, pos);
    if !(relative && h_end == pos && s[pos - 1] == b'/') {
        sp.host = Some(text(s, pos, h_end));
        pos = h_end;
    }
    if s.get(pos) == Some(&b':') {
        let d_end = pos
            + 1
            + s[pos + 1..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
        sp.port = Some(text(s, pos + 1, d_end));
        pos = d_end;
    }
    let p_end = if s.get(pos) == Some(&b'/') {
        run(s, pos + 1, seg_char)
    } else {
        pos
    };
    sp.path = Some(text(s, pos, p_end));
    if tail(s, p_end, &mut sp) {
        Some(sp)
    } else {
        None
    }
}

fn with_path(s: &[u8], start: usize, end: usize) -> Option<Split> {
    let mut sp = Split {
        path: Some(text(s, start, end)),
        ..Split::default()
    };
    if tail(s, end, &mut sp) {
        Some(sp)
    } else {
        None
    }
}

fn split_absolute(s: &[u8]) -> Option<Split> {
    if !s.first()?.is_ascii_alphabetic() {
        return None;
    }
    let scheme_end = 1 + s[1..]
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        .count();
    if s.get(scheme_end) != Some(&b':') {
        return None;
    }
    let scheme = text(s, 0, scheme_end);
    let p = scheme_end + 1;
    let mut sp = None;
    if s[p..].starts_with(b"//") {
        sp = authority(s, p + 2, false);
    }
    if sp.is_none() && s.get(p) == Some(&b'/') {
        let end = if s.get(p + 1).is_some_and(|b| *b != b'/') {
            run(s, p + 1, seg_char)
        } else {
            p + 1
        };
        sp = with_path(s, p, end);
    }
    if sp.is_none() && s.get(p).is_some_and(|b| *b != b'/') {
        let end = run(s, p, seg_char);
        if end > p {
            if let Some(mut rootless) = with_path(s, p, end) {
                let mut opaque = rootless.path.take().expect("path set");
                if let Some(q) = rootless.query.take() {
                    opaque.push('?');
                    opaque.push_str(&q);
                }
                rootless.opaque = Some(opaque);
                sp = Some(rootless);
            }
        }
    }
    if sp.is_none() {
        sp = with_path(s, p, p);
    }
    let mut sp = sp?;
    sp.scheme = Some(scheme);
    Some(sp)
}

fn split_relative(s: &[u8]) -> Option<Split> {
    if s.starts_with(b"//") {
        if let Some(sp) = authority(s, 2, true) {
            return Some(sp);
        }
    }
    if s.first() == Some(&b'/') {
        if let Some(sp) = with_path(s, 0, run(s, 1, seg_char)) {
            return Some(sp);
        }
    }
    let nc = run(s, 0, seg_nc_char);
    if nc > 0 {
        let end = if s.get(nc) == Some(&b'/') {
            run(s, nc + 1, seg_char)
        } else {
            nc
        };
        if let Some(sp) = with_path(s, 0, end) {
            return Some(sp);
        }
    }
    with_path(s, 0, 0)
}

// ----------------------------------------------------------------- components

#[derive(Clone, Copy, PartialEq)]
enum Scheme {
    Generic,
    Http,
    Https,
    Ws,
    Wss,
    Ftp,
    File,
    MailTo,
    Ldap,
    Ldaps,
}

impl Scheme {
    fn from_name(name: Option<&str>) -> Self {
        match name.map(str::to_ascii_uppercase).as_deref() {
            Some("HTTP") => Scheme::Http,
            Some("HTTPS") => Scheme::Https,
            Some("WS") => Scheme::Ws,
            Some("WSS") => Scheme::Wss,
            Some("FTP") => Scheme::Ftp,
            Some("FILE") => Scheme::File,
            Some("MAILTO") => Scheme::MailTo,
            Some("LDAP") => Scheme::Ldap,
            Some("LDAPS") => Scheme::Ldaps,
            _ => Scheme::Generic,
        }
    }

    fn default_port(self) -> Option<&'static str> {
        match self {
            Scheme::Http | Scheme::Ws => Some("80"),
            Scheme::Https | Scheme::Wss => Some("443"),
            Scheme::Ftp => Some("21"),
            Scheme::Ldap => Some("389"),
            Scheme::Ldaps => Some("636"),
            _ => None,
        }
    }
}

/// `URI::Generic#query=`: drop TAB/CR/LF, reject `%\H\H`, escape the rest.
fn set_query(v: &str) -> Result<String, UriError> {
    let b: Vec<u8> = v
        .bytes()
        .filter(|c| !matches!(c, b'\t' | b'\r' | b'\n'))
        .collect();
    for i in 0..b.len() {
        if b[i] == b'%' && i + 2 < b.len() && !is_hex(b[i + 1]) && !is_hex(b[i + 2]) {
            return Err(UriError::Invalid(format!(
                "invalid percent escape: {}",
                text(&b, i, i + 3)
            )));
        }
    }
    let keep =
        |c: u8| matches!(c, b'!' | b'$'..=b'&' | b'('..=b';' | b'=' | b'?'..=b'_' | b'a'..=b'~');
    Ok(escape_bytes(&b, keep))
}

fn escape_bytes(b: &[u8], keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(b.len());
    for &c in b {
        if keep(c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// `URI::Generic#fragment=`.
fn set_fragment(v: &str) -> String {
    let b: Vec<u8> = v
        .bytes()
        .filter(|c| !matches!(c, b'\t' | b'\r' | b'\n'))
        .collect();
    escape_bytes(&b, |c| (b'!'..=b'~').contains(&c))
}

/// `[^@,;]+@[^@,;]+(?:\z|[,;])` repeated, anchored (`URI::MailTo`).
fn mailto_to_ok(to: &[u8]) -> bool {
    let plain = |b: &u8| !matches!(b, b'@' | b',' | b';');
    let mut i = 0;
    while i < to.len() {
        let a = to[i..].iter().take_while(|b| plain(b)).count();
        if a == 0 || to.get(i + a) != Some(&b'@') {
            return false;
        }
        i += a + 1;
        let d = to[i..].iter().take_while(|b| plain(b)).count();
        if d == 0 {
            return false;
        }
        i += d;
        if i < to.len() {
            if matches!(to[i], b',' | b';') {
                i += 1;
            } else {
                return false;
            }
        }
    }
    true
}

/// `URI.parse(value).to_s`.
pub fn parse_to_s(value: &str) -> Result<String, UriError> {
    if !value.is_ascii() {
        return Err(UriError::Invalid(format!(
            "URI must be ascii only {}",
            dump(value)
        )));
    }
    let s = value.as_bytes();
    let sp = split_absolute(s)
        .or_else(|| split_relative(s))
        .ok_or_else(|| {
            UriError::Invalid(format!("bad URI (is not URI?): {}", string_inspect(value)))
        })?;
    let kind = Scheme::from_name(sp.scheme.as_deref());

    // URI::FTP#initialize runs before Generic#initialize.
    let mut path = sp.path.clone();
    if kind == Scheme::Ftp {
        let Some(p) = path.as_deref() else {
            return Err(UriError::Invalid("URI::InvalidURIError".into()));
        };
        let p = p.strip_prefix('/').unwrap_or(p);
        let p = match p.strip_prefix("%2F") {
            Some(rest) => format!("/{rest}"),
            None => p.to_string(),
        };
        // URI::FTP#set_path
        path = Some(match p.strip_prefix('/') {
            Some(rest) => format!("/%2F{rest}"),
            None => format!("/{p}"),
        });
    }

    // URI::Generic#initialize (arg_check = false)
    let scheme = sp.scheme.as_deref().map(str::to_ascii_lowercase);
    let host = match kind {
        Scheme::File => Some(match sp.host.as_deref() {
            None | Some("localhost") => String::new(),
            Some(h) => h.to_string(),
        }),
        _ => sp.host.clone(),
    };
    let mut port = match kind {
        Scheme::File => None,
        _ => sp.port.as_deref().filter(|p| !p.is_empty()).map(|p| {
            let t = p.trim_start_matches('0');
            if t.is_empty() { "0" } else { t }.to_string()
        }),
    };
    let userinfo = match kind {
        Scheme::File => None,
        _ => sp.userinfo.clone().filter(|u| !u.is_empty()),
    };
    let query = sp.query.as_deref().map(set_query).transpose()?;
    let mut opaque = sp.opaque.clone();
    let fragment = sp.fragment.as_deref().map(set_fragment);
    if path.is_none() && opaque.is_none() {
        path = Some(String::new());
    }
    if port.is_none() {
        port = kind.default_port().map(str::to_string);
    }

    match kind {
        Scheme::MailTo => {
            if let (Some(q), None) = (&query, &opaque) {
                opaque = Some(format!("?{q}"));
            }
            let Some(op) = opaque else {
                return Err(UriError::InvalidComponent(
                    "missing opaque part for mailto URL".into(),
                ));
            };
            let (to, header) = match op.split_once('?') {
                Some((t, h)) => (t.to_string(), Some(h.to_string())),
                None => (op.clone(), None),
            };
            if !mailto_to_ok(to.as_bytes()) {
                return Err(UriError::InvalidComponent(format!(
                    "unrecognised opaque part for mailtoURL: {op}"
                )));
            }
            let mut out = format!("{}:{to}", scheme.expect("mailto has a scheme"));
            if let Some(h) = header {
                let mut fields: Vec<&str> = h.split('&').collect();
                while fields.last() == Some(&"") {
                    fields.pop();
                }
                if !fields.is_empty() {
                    out.push('?');
                    out.push_str(&fields.join("&"));
                }
            }
            if let Some(f) = fragment {
                out.push('#');
                out.push_str(&f);
            }
            return Ok(out);
        }
        Scheme::Ldap | Scheme::Ldaps if fragment.is_some() || path.is_none() => {
            return Err(UriError::Invalid("bad LDAP URL".into()));
        }
        _ => {}
    }

    // URI::Generic#to_s
    let mut out = String::new();
    if let Some(sc) = &scheme {
        out.push_str(sc);
        out.push(':');
    }
    if let Some(op) = &opaque {
        out.push_str(op);
    } else {
        let path = path.expect("path or opaque is set");
        if host.is_some() || matches!(scheme.as_deref(), Some("file" | "postgres")) {
            out.push_str("//");
        }
        if let Some(u) = &userinfo {
            out.push_str(u);
            out.push('@');
        }
        if let Some(h) = &host {
            out.push_str(h);
        }
        if let Some(p) = &port {
            if Some(p.as_str()) != kind.default_port() {
                out.push(':');
                out.push_str(p);
            }
        }
        if (host.is_some() || port.is_some()) && !path.is_empty() && !path.starts_with('/') {
            out.push('/');
        }
        out.push_str(&path);
        if let Some(q) = &query {
            out.push('?');
            out.push_str(q);
        }
    }
    if let Some(f) = &fragment {
        out.push('#');
        out.push_str(f);
    }
    Ok(out)
}
