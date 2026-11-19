//! IPAddr observations captured from the pinned Ruby oracle, including
//! prefixes, masks, brackets, zones and exceptions escaping json-schema.
use super::{data::Data, SchemaError};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn prefix_error(message: impl Into<String>) -> SchemaError {
    SchemaError::IpAddress {
        prefix: true,
        message: message.into(),
    }
}

pub(super) fn valid(text: &str, ipv6: bool) -> Result<bool, SchemaError> {
    let bracketed = text.starts_with('[');
    let (address, prefix) = text
        .split_once('/')
        .map_or((text, None), |(a, p)| (a, Some(p)));
    let address = address
        .strip_prefix('[')
        .and_then(|a| a.strip_suffix(']'))
        .unwrap_or(address);
    let (address, zone) = address
        .split_once('%')
        .map_or((address, None), |(a, z)| (a, Some(z)));
    if zone.is_some_and(|z| z.is_empty() || z.contains('%') || z.chars().any(char::is_whitespace)) {
        return Ok(false);
    }
    let dotted = address.rsplit(':').next().unwrap_or(address);
    let parts: Vec<&str> = dotted.split('.').collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        for part in &parts {
            if part.parse::<u32>().map_or(true, |n| n > 255) {
                return Ok(false);
            }
            if part.len() > 1 && part.starts_with('0') {
                let spelling = if address.contains(':') {
                    Data::Array(parts.iter().map(|s| Data::String((*s).into())).collect())
                        .ruby_inspect()?
                } else {
                    address.to_owned()
                };
                return Err(SchemaError::IpAddress {
                    prefix: false,
                    message: format!("zero-filled number in IPv4 address is ambiguous: {spelling}"),
                });
            }
        }
    }
    let Ok(ip) = address.parse::<IpAddr>() else {
        return Ok(false);
    };
    if (zone.is_some() || bracketed) && ip.is_ipv4() {
        return Ok(false);
    }
    if let Some(prefix) = prefix {
        let bits = if ip.is_ipv6() { 128 } else { 32 };
        if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()) {
            if prefix.len() > 1 && prefix.starts_with('0') {
                return Err(prefix_error("leading zeros in prefix"));
            }
            if prefix.parse::<u32>().map_or(true, |n| n > bits) {
                return Err(prefix_error("invalid length"));
            }
        } else {
            if !prefix.contains(['.', ':']) {
                return Ok(false);
            }
            // IPAddr parses the netmask as another address, including its
            // escaping zero-filled-address exception.
            if !valid(prefix, ip.is_ipv6())? {
                if valid(prefix, !ip.is_ipv6())? {
                    return Err(prefix_error("address family is not same"));
                }
                return Ok(false);
            }
            let mask = if ip.is_ipv6() {
                prefix.parse::<Ipv6Addr>().ok().map(u128::from)
            } else {
                prefix
                    .parse::<Ipv4Addr>()
                    .ok()
                    .map(|a| u128::from(u32::from(a)))
            };
            let Some(mask) = mask else {
                return Ok(false);
            };
            let full = if bits == 128 {
                u128::MAX
            } else {
                u32::MAX as u128
            };
            let inverse = full ^ mask;
            if inverse & inverse.wrapping_add(1) != 0 {
                return Err(prefix_error(format!("invalid mask {prefix}")));
            }
        }
    }
    Ok(ip.is_ipv6() == ipv6)
}
