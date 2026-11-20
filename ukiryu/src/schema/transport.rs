//! Native open-uri transport shared by schema references and the definition
//! fallback. WASM must use a host-supplied schema; it never imports a client.
use super::{uri, SchemaError};

pub(super) fn is_http(reference: &str) -> bool {
    uri::split(reference)
        .scheme
        .is_some_and(|s| s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read(reference: &str) -> Result<String, SchemaError> {
    Err(SchemaError::unsupported(
        "HTTP schema on WASM",
        format!("host must supply schema resources for {reference:?}"),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read(reference: &str) -> Result<String, SchemaError> {
    use std::error::Error;
    if reference.contains(['\r', '\n']) {
        return Err(SchemaError::Ruby {
            class: "URI::InvalidURIError".into(),
            message: format!(
                "bad URI (is not URI?): {}",
                super::data::inspect_str(reference)
            ),
        });
    }
    let location = reference.split('#').next().unwrap_or(reference);
    // Ruby follows HTTP redirects, but preserves the requested schema URI as
    // its resolution base. ureq's response URL is deliberately not used.
    let result = ureq::AgentBuilder::new().build().get(location).call();
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(_, _)) => {
            return Err(SchemaError::ReadFailed {
                message: format!("Read of URI at {reference} failed"),
            });
        }
        Err(ureq::Error::Transport(error)) => {
            let mut source = error.source();
            while let Some(e) = source {
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused)
                {
                    let parts = uri::split(location);
                    let authority = parts.authority.unwrap_or("");
                    let (host, port) = authority.rsplit_once(':').unwrap_or((
                        authority,
                        if location.starts_with("https:") {
                            "443"
                        } else {
                            "80"
                        },
                    ));
                    return Err(SchemaError::Io {
                        errno: "ECONNREFUSED".into(),
                        message: format!("Failed to open TCP connection to {host}:{port} (Connection refused - connect(2) for \"{host}\" port {port})"),
                    });
                }
                source = e.source();
            }
            if error.kind() == ureq::ErrorKind::InvalidUrl {
                return Err(SchemaError::Ruby {
                    class: "URI::InvalidURIError".into(),
                    message: format!(
                        "bad URI (is not URI?): {}",
                        super::data::inspect_str(reference)
                    ),
                });
            }
            if error.kind() == ureq::ErrorKind::Dns {
                return Err(SchemaError::ReadFailed {
                    message: format!("Read of URI at {reference} failed"),
                });
            }
            return Err(SchemaError::unsupported(
                "HTTP transport failure",
                error.to_string(),
            ));
        }
    };
    response
        .into_string()
        .map_err(|error| SchemaError::unsupported("HTTP response encoding/read", error.to_string()))
}
