//! Identifies where a saved API key may be sent: the provider plus the normalized
//! server URL it was saved with. A key is only used when the current settings
//! produce the same binding, so it can never leak to a different server.
//! Port of `key-binding.js` (WHATWG URL normalization, for the URL shapes used here).

pub fn key_binding(provider: &str, base_url: &str) -> String {
    let url = base_url.trim().trim_end_matches('/');
    let normalized = normalize_url(url).unwrap_or_else(|| url.to_lowercase());
    format!("{provider}|{normalized}")
}

/// `${protocol}//${host}${pathname without trailing slashes}`, or `None` where
/// `new URL()` would throw.
fn normalize_url(url: &str) -> Option<String> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let rest = &url[colon + 1..];
    let special = matches!(scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp");

    let (host, path) = if special || rest.starts_with("//") {
        let after = rest.trim_start_matches(|c| c == '/' || (special && c == '\\'));
        let end = after.find(|c| c == '/' || c == '?' || c == '#' || (special && c == '\\')).unwrap_or(after.len());
        let authority = &after[..end];
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        if special && host_port.is_empty() {
            return None;
        }
        let host_port = host_port.to_lowercase();
        let default_port = match scheme.as_str() {
            "http" | "ws" => Some(":80"),
            "https" | "wss" => Some(":443"),
            "ftp" => Some(":21"),
            _ => None,
        };
        let host = match default_port {
            Some(p) if host_port.ends_with(p) => host_port[..host_port.len() - p.len()].to_string(),
            _ => host_port,
        };
        (host, &after[end..])
    } else {
        (String::new(), rest)
    };
    let path_end = path.find(|c| c == '?' || c == '#').unwrap_or(path.len());
    let path = path[..path_end].replace('\\', "/");
    Some(format!("{scheme}://{host}{}", path.trim_end_matches('/')))
}
