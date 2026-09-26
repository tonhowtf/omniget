//! Sites whose owners asked OmniGet not to support them.
//!
//! A domain listed here is refused at every download entry point, and no
//! connector that declares it is loaded. Owners ask for removal through the
//! process in PLATFORM-OWNERS.md; this list is what makes that promise hold.

/// Registrable domains whose owners asked for removal. Every subdomain of a
/// listed domain is covered too.
pub const OPTED_OUT_DOMAINS: &[&str] = &["kiwify.com.br", "kiwify.com", "kiwify.app"];

/// Error returned when a URL points at an opted-out site. The frontend maps
/// this exact string to `errors.platform_opted_out`.
pub const OPTED_OUT_ERROR: &str = "The owner of this site asked OmniGet not to support it.";

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .trim_start_matches("*.")
        .to_ascii_lowercase()
}

/// True when `host` is a listed domain or a subdomain of one.
pub fn is_opted_out_host(host: &str) -> bool {
    let host = normalize_host(host);
    if host.is_empty() {
        return false;
    }
    OPTED_OUT_DOMAINS.iter().any(|domain| {
        host == *domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// True when `input` is a URL, or a bare host, on an opted-out site.
pub fn is_opted_out(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Ok(parsed) = url::Url::parse(trimmed) {
        if let Some(host) = parsed.host_str() {
            return is_opted_out_host(host);
        }
    }
    if let Ok(parsed) = url::Url::parse(&format!("https://{trimmed}")) {
        if let Some(host) = parsed.host_str() {
            return is_opted_out_host(host);
        }
    }
    false
}

/// `Err(OPTED_OUT_ERROR)` when the URL points at an opted-out site.
pub fn ensure_allowed(url: &str) -> Result<(), String> {
    if is_opted_out(url) {
        Err(OPTED_OUT_ERROR.to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_domains_and_subdomains_are_refused() {
        for url in [
            "https://kiwify.com.br/",
            "https://dashboard.kiwify.com.br/course/abc",
            "https://members.kiwify.com.br/x?y=1",
            "http://KIWIFY.COM/",
            "https://pay.kiwify.com/checkout",
            "https://kiwify.app/",
            "https://a.b.kiwify.app/lesson",
            "kiwify.com.br",
            "dashboard.kiwify.com.br/path",
            "https://kiwify.com.br./",
        ] {
            assert!(is_opted_out(url), "{url} should be opted out");
            assert_eq!(ensure_allowed(url), Err(OPTED_OUT_ERROR.to_string()));
        }
    }

    #[test]
    fn other_sites_are_allowed() {
        for url in [
            "https://www.youtube.com/watch?v=abc",
            "https://notkiwify.com.br/",
            "https://kiwify.com.br.example.org/",
            "https://example.com/kiwify.com.br",
            "https://kiwify.org/",
            "",
            "not a url",
        ] {
            assert!(!is_opted_out(url), "{url} should be allowed");
            assert!(ensure_allowed(url).is_ok());
        }
    }

    #[test]
    fn hosts_are_matched_on_label_boundaries() {
        assert!(is_opted_out_host("kiwify.com"));
        assert!(is_opted_out_host("*.kiwify.com.br"));
        assert!(is_opted_out_host("cdn.kiwify.app"));
        assert!(!is_opted_out_host("xkiwify.com"));
        assert!(!is_opted_out_host("com.br"));
    }
}
