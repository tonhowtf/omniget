pub mod cache;
pub mod fetch;
pub mod project;
pub mod runner;

use serde::Serialize;

pub const FETCH_TIMEOUT_SECS: u64 = 15;
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
pub const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BuiltinRegistry {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    pub index_url: &'static str,
    pub item_url: &'static str,
    pub docs_url: Option<&'static str>,
}

pub const BUILTIN_REGISTRIES: &[BuiltinRegistry] = &[
    BuiltinRegistry {
        id: "shadcn",
        name: "shadcn/ui",
        homepage: "https://ui.shadcn.com",
        index_url: "https://ui.shadcn.com/r/styles/new-york-v4/registry.json",
        item_url: "https://ui.shadcn.com/r/styles/new-york-v4/{name}.json",
        docs_url: Some("https://ui.shadcn.com/docs/components/{name}"),
    },
    BuiltinRegistry {
        id: "magicui",
        name: "Magic UI",
        homepage: "https://magicui.design",
        index_url: "https://magicui.design/r/registry.json",
        item_url: "https://magicui.design/r/{name}.json",
        docs_url: Some("https://magicui.design/docs/components/{name}"),
    },
    BuiltinRegistry {
        id: "react-bits",
        name: "React Bits",
        homepage: "https://reactbits.dev",
        index_url: "https://reactbits.dev/r/registry.json",
        item_url: "https://reactbits.dev/r/{name}.json",
        docs_url: None,
    },
    BuiltinRegistry {
        id: "cult-ui",
        name: "Cult UI",
        homepage: "https://www.cult-ui.com",
        index_url: "https://cult-ui.com/r/registry.json",
        item_url: "https://cult-ui.com/r/{name}.json",
        docs_url: Some("https://www.cult-ui.com/docs/components/{name}"),
    },
];

pub fn validate_url(raw: &str) -> Result<url::Url, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("url_empty");
    }
    if trimmed.len() > 2048 {
        return Err("url_too_long");
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| "url_invalid")?;
    if parsed.scheme() != "https" {
        return Err("url_not_https");
    }
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() {
        return Err("url_invalid");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("url_credentials");
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err("url_private");
    }
    let private = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => is_private_ip(ip.into()),
        Some(url::Host::Ipv6(ip)) => is_private_ip(ip.into()),
        _ => false,
    };
    if private {
        return Err("url_private");
    }
    Ok(parsed)
}

pub fn is_private_ip(ip: std::net::IpAddr) -> bool {
    let v4 = match ip {
        std::net::IpAddr::V4(v4) => v4,
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4,
            None => {
                return v6.is_loopback()
                    || v6.is_unspecified()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    || (v6.segments()[0] & 0xffc0) == 0xfe80;
            }
        },
    };
    let [a, b, ..] = v4.octets();
    v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || a == 0
        || (a == 100 && (b & 0xc0) == 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_public_https() {
        assert!(validate_url("https://ui.shadcn.com/r/index.json").is_ok());
        assert!(validate_url("  https://magicui.design/r/registry.json ").is_ok());
    }

    #[test]
    fn rejects_other_schemes_and_local_hosts() {
        assert!(validate_url("http://ui.shadcn.com/r/index.json").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("https://localhost/r.json").is_err());
        assert!(validate_url("https://127.0.0.1/r.json").is_err());
        assert!(validate_url("https://192.168.1.4/r.json").is_err());
        assert!(validate_url("https://10.0.0.1/r.json").is_err());
        assert!(validate_url("https://[::1]/r.json").is_err());
        assert!(validate_url("https://[::ffff:127.0.0.1]/r.json").is_err());
        assert!(validate_url("https://[::ffff:192.168.0.1]/r.json").is_err());
        assert!(validate_url("https://100.64.0.1/r.json").is_err());
        assert!(validate_url("https://100.127.255.254/r.json").is_err());
        assert!(validate_url("https://0.1.2.3/r.json").is_err());
        assert!(validate_url("https://100.128.0.1/r.json").is_ok());
        assert!(validate_url("https://[::ffff:8.8.8.8]/r.json").is_ok());
        assert!(is_private_ip("fd00::1".parse().unwrap()));
        assert!(!is_private_ip("2606:4700::1".parse().unwrap()));
        assert!(validate_url("https://user:pw@example.com/r.json").is_err());
        assert!(validate_url("").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn builtins_are_valid() {
        for r in BUILTIN_REGISTRIES {
            assert!(validate_url(r.index_url).is_ok(), "{}", r.id);
            assert!(r.item_url.contains("{name}"), "{}", r.id);
            assert!(validate_url(&r.item_url.replace("{name}", "x")).is_ok());
        }
    }
}
