pub const OPTED_OUT_DOMAINS: &[&str] = &["kiwify.com.br", "kiwify.com", "kiwify.app"];

pub const OPTED_OUT_ERROR: &str = "The owner of this site asked OmniGet not to support it.";

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .trim_start_matches("*.")
        .to_ascii_lowercase()
}

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

pub fn ensure_allowed(url: &str) -> Result<(), String> {
    if is_opted_out(url) {
        Err(OPTED_OUT_ERROR.to_string())
    } else {
        Ok(())
    }
}
