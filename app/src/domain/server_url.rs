// The address of a Qamille server, normalized (spec A1, A6). No dependencies and no `crate::`
// paths: `app/build.rs` includes this file to validate `QAMILLE_PRESET_SERVER` at build time.

/// Why an address is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerUrlError {
    /// Not `scheme://host[:port]`: a path, a query, credentials or another scheme.
    Invalid,
    /// Plain `http` to a host other than this computer.
    HttpsRequired,
}

impl ServerUrlError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "invalid_url",
            Self::HttpsRequired => "https_required",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Invalid => "Write the server address as https://name.example, without a path",
            Self::HttpsRequired => "The server address must start with https://",
        }
    }
}

/// `scheme://host[:port]` in lower case, without a trailing slash. `https` is required except for
/// `127.0.0.1`, `localhost` and `[::1]`, which may use `http` for tests on this computer.
pub fn normalize_server_url(input: &str) -> Result<String, ServerUrlError> {
    let trimmed = input.trim();
    let (scheme, rest) = trimmed.split_once("://").ok_or(ServerUrlError::Invalid)?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(ServerUrlError::Invalid);
    }
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty()
        || rest.chars().any(|c| matches!(c, '/' | '?' | '#' | '@' | '\\') || c.is_whitespace() || c.is_control())
    {
        return Err(ServerUrlError::Invalid);
    }
    let authority = rest.to_ascii_lowercase();
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let end = inner.find(']').ok_or(ServerUrlError::Invalid)?;
        let address = &inner[..end];
        if address.is_empty() || !address.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return Err(ServerUrlError::Invalid);
        }
        let after = &inner[end + 1..];
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or(ServerUrlError::Invalid)?),
        };
        (&authority[..end + 2], port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority.as_str(), None),
        }
    };
    if let Some(port) = port {
        let valid = !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit())
            && port.parse::<u32>().is_ok_and(|n| (1..=65_535).contains(&n));
        if !valid {
            return Err(ServerUrlError::Invalid);
        }
    }
    if !host.starts_with('[') {
        let labels_ok = !host.is_empty()
            && host.split('.').all(|label| {
                !label.is_empty() && !label.starts_with('-') && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
        if !labels_ok {
            return Err(ServerUrlError::Invalid);
        }
    }
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
    if scheme == "http" && !loopback {
        return Err(ServerUrlError::HttpsRequired);
    }
    Ok(format!("{scheme}://{authority}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_are_normalized() {
        assert_eq!(normalize_server_url(" HTTPS://Calcolo.Ufficio.it/ ").unwrap(), "https://calcolo.ufficio.it");
        assert_eq!(normalize_server_url("https://calcolo.ufficio.it:8443").unwrap(), "https://calcolo.ufficio.it:8443");
        assert_eq!(normalize_server_url("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        assert_eq!(normalize_server_url("http://LOCALHOST:8787/").unwrap(), "http://localhost:8787");
        assert_eq!(normalize_server_url("http://[::1]:8787").unwrap(), "http://[::1]:8787");
    }
    #[test]
    fn remote_http_paths_credentials_and_other_schemes_are_refused() {
        assert_eq!(normalize_server_url("http://calcolo.ufficio.it"), Err(ServerUrlError::HttpsRequired));
        assert_eq!(normalize_server_url("http://192.168.1.4:8787"), Err(ServerUrlError::HttpsRequired));
        for bad in [
            "https://calcolo.ufficio.it/api", "https://user:pw@calcolo.ufficio.it", "ftp://calcolo.ufficio.it",
            "calcolo.ufficio.it", "https://", "https://calcolo ufficio.it", "https://calcolo.ufficio.it?x=1",
            "https://calcolo.ufficio.it:0", "https://calcolo.ufficio.it:99999", "https://-bad.example",
            "https://calcolo..it", "https://[zz]:1",
        ] {
            assert_eq!(normalize_server_url(bad), Err(ServerUrlError::Invalid), "{bad}");
        }
    }
}
