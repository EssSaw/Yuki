use url::Url;

/// Validates that a candidate endpoint string is a valid loopback HTTP URL conforming to safety policy:
/// - Only unencrypted `http` scheme is permitted for local loopback MCP.
/// - Host MUST be strictly loopback: `localhost`, `127.0.0.1`, or `::1`.
/// - Misleading localhost suffixes (e.g. `localhost.attacker.com`, `evil-localhost`) are rejected.
/// - Non-loopback IPs and DNS aliases are rejected.
/// - Credentials (username/password) are forbidden.
/// - Query parameters are forbidden.
/// - Fragment identifiers are forbidden.
/// - Returns canonicalized endpoint string `http://<host>:<port><path>`.
pub fn validate_and_canonicalize_loopback_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Endpoint URL cannot be empty".to_string());
    }

    let parsed = Url::parse(trimmed).map_err(|e| format!("Invalid URL format: {e}"))?;

    // 1. Only unencrypted HTTP is permitted for loopback MCP
    if parsed.scheme() != "http" {
        return Err(format!(
            "Invalid scheme '{}': only unencrypted HTTP loopback is supported for local MCP",
            parsed.scheme()
        ));
    }

    // 2. Reject credentials
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL credentials (username/password) are forbidden".to_string());
    }

    // 3. Reject query parameters
    if parsed.query().is_some() {
        return Err("URL query parameters are forbidden".to_string());
    }

    // 4. Reject fragment
    if parsed.fragment().is_some() {
        return Err("URL fragment identifiers are forbidden".to_string());
    }

    // 5. Host validation: strictly loopback (localhost, 127.0.0.1, ::1)
    let host_str = match parsed.host() {
        Some(url::Host::Domain(d)) => {
            let lower = d.to_ascii_lowercase();
            if lower == "localhost" {
                "localhost".to_string()
            } else {
                return Err(format!(
                    "Non-loopback host '{}' is forbidden. Only 'localhost', '127.0.0.1', or '::1' are permitted.",
                    d
                ));
            }
        }
        Some(url::Host::Ipv4(ip)) => {
            if ip == std::net::Ipv4Addr::new(127, 0, 0, 1) {
                "127.0.0.1".to_string()
            } else {
                return Err(format!(
                    "IPv4 address '{}' is forbidden. Only '127.0.0.1' is permitted for loopback MCP.",
                    ip
                ));
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            if ip == std::net::Ipv6Addr::LOCALHOST {
                "[::1]".to_string()
            } else {
                return Err(format!(
                    "IPv6 address '{}' is forbidden. Only '::1' is permitted for loopback MCP.",
                    ip
                ));
            }
        }
        None => return Err("URL host is missing".to_string()),
    };

    // 6. Port validation
    let port = parsed.port().unwrap_or(80);
    if port == 0 {
        return Err("Invalid port 0".to_string());
    }

    // 7. Path canonicalization: normalize empty or root to "/", trim trailing slash for multi-segment paths
    let path = parsed.path();
    let canonical_path = if path.is_empty() || path == "/" {
        "/".to_string()
    } else {
        let trimmed_path = path.trim_end_matches('/');
        if trimmed_path.is_empty() {
            "/".to_string()
        } else {
            trimmed_path.to_string()
        }
    };

    Ok(format!("http://{}:{}{}", host_str, port, canonical_path))
}
