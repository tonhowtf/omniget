pub mod http;
pub mod store;

use serde::Serialize;
use serde_json::Value;

pub const ERR_INVALID_URL: &str =
    "OmniDisc: invalid instance URL. Use http:// or https:// without a username or password.";
pub const ERR_UNREACHABLE: &str =
    "OmniDisc: the server did not respond. Check the address or ask the owner for a new link.";

#[derive(Serialize)]
pub struct ConnectResult {
    pub url: String,
    pub recognized: bool,
    /// The session token and every message travel in the clear on this
    /// instance. Plain `http://` still works — a lot of self-hosting starts on
    /// a LAN — but the UI has to say so instead of showing the same padlock as
    /// everyone else.
    pub insecure: bool,
    pub instance: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
}

/// Loopback traffic never leaves the machine, so `http://localhost` is not the
/// thing this flag is warning about.
pub fn is_insecure_instance_url(base: &str) -> bool {
    let Ok(parsed) = url::Url::parse(base) else {
        return true;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    !matches!(
        parsed.host(),
        Some(url::Host::Domain("localhost"))
            | Some(url::Host::Ipv4(std::net::Ipv4Addr::LOCALHOST))
            | Some(url::Host::Ipv6(std::net::Ipv6Addr::LOCALHOST))
    )
}

pub fn normalize_instance_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ERR_INVALID_URL.to_string());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };
    let parsed = url::Url::parse(&with_scheme).map_err(|_| ERR_INVALID_URL.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ERR_INVALID_URL.to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ERR_INVALID_URL.to_string());
    }
    if parsed.host_str().map(str::is_empty).unwrap_or(true) {
        return Err(ERR_INVALID_URL.to_string());
    }
    let mut base = parsed.clone();
    base.set_query(None);
    base.set_fragment(None);
    let mut base = base.to_string();
    while base.ends_with('/') {
        base.pop();
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_host_and_adds_https() {
        assert_eq!(
            normalize_instance_url("chat.example.org").unwrap(),
            "https://chat.example.org"
        );
    }

    #[test]
    fn strips_trailing_slash_query_and_fragment() {
        assert_eq!(
            normalize_instance_url("https://chat.example.org/?x=1#frag").unwrap(),
            "https://chat.example.org"
        );
        assert_eq!(
            normalize_instance_url("http://localhost:8080/base/").unwrap(),
            "http://localhost:8080/base"
        );
    }

    #[test]
    fn plain_http_is_flagged_everywhere_but_loopback() {
        assert!(is_insecure_instance_url("http://chat.example.org"));
        assert!(is_insecure_instance_url("http://192.168.0.10:8080"));
        assert!(!is_insecure_instance_url("https://chat.example.org"));
        assert!(!is_insecure_instance_url("http://localhost:8080"));
        assert!(!is_insecure_instance_url("http://127.0.0.1:8080"));
        assert!(!is_insecure_instance_url("http://[::1]:8080"));
        assert!(is_insecure_instance_url("not a url"));
    }

    #[test]
    fn rejects_credentials_and_other_schemes() {
        assert!(normalize_instance_url("https://user:pw@chat.example.org").is_err());
        assert!(normalize_instance_url("ftp://chat.example.org").is_err());
        assert!(normalize_instance_url("").is_err());
        assert!(normalize_instance_url("   ").is_err());
    }
}
