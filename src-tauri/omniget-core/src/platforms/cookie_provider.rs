use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub trait CookieProvider: Send + Sync {
    fn cookie_path_for(&self, domain: &str) -> Option<PathBuf>;

    fn cookie_path_for_account(&self, domain: &str, _slug: &str) -> Option<PathBuf> {
        self.cookie_path_for(domain)
    }

    fn manual_cookie_header(&self, _domain: &str) -> Option<String> {
        None
    }
}

static COOKIE_PROVIDER: OnceLock<Arc<dyn CookieProvider>> = OnceLock::new();

pub struct DefaultCookieProvider;

impl CookieProvider for DefaultCookieProvider {
    fn cookie_path_for(&self, _domain: &str) -> Option<PathBuf> {
        let path = crate::core::paths::app_data_dir()?
            .join("cookies")
            .join("cookies.txt");
        path.exists().then_some(path)
    }

    fn cookie_path_for_account(&self, domain: &str, slug: &str) -> Option<PathBuf> {
        Some(
            crate::core::paths::app_data_dir()?
                .join("cookies")
                .join(domain)
                .join(format!("{}.txt", slug)),
        )
    }
}

pub fn set_cookie_provider(provider: Arc<dyn CookieProvider>) {
    let _ = COOKIE_PROVIDER.set(provider);
}

pub fn cookie_path_for(domain: &str) -> Option<PathBuf> {
    COOKIE_PROVIDER
        .get()
        .and_then(|provider| provider.cookie_path_for(domain))
        .or_else(|| DefaultCookieProvider.cookie_path_for(domain))
}

pub fn cookie_path_for_account(domain: &str, slug: &str) -> Option<PathBuf> {
    COOKIE_PROVIDER
        .get()
        .and_then(|provider| provider.cookie_path_for_account(domain, slug))
        .or_else(|| DefaultCookieProvider.cookie_path_for_account(domain, slug))
}

pub fn manual_cookie_header(domain: &str) -> Option<String> {
    COOKIE_PROVIDER
        .get()
        .and_then(|provider| provider.manual_cookie_header(domain))
}

fn netscape_domain_matches(cookie_domain: &str, requested_domain: &str) -> bool {
    let cookie_domain = cookie_domain
        .trim()
        .trim_start_matches("#HttpOnly_")
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let requested_domain = requested_domain
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();

    !cookie_domain.is_empty()
        && !requested_domain.is_empty()
        && (cookie_domain == requested_domain
            || cookie_domain.ends_with(&format!(".{}", requested_domain))
            || requested_domain.ends_with(&format!(".{}", cookie_domain)))
}

pub fn cookie_header_from_netscape_for_domain(content: &str, domain: &str) -> Option<String> {
    let pairs: Vec<String> = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("# Netscape") {
                return None;
            }
            let normalized = trimmed.strip_prefix("#HttpOnly_").unwrap_or(trimmed);
            if normalized.starts_with('#') {
                return None;
            }
            let parts: Vec<&str> = normalized.split('\t').collect();
            if parts.len() < 7 || !netscape_domain_matches(parts[0], domain) {
                return None;
            }
            let name = parts[5].trim();
            let value = parts[6].trim();
            if name.is_empty() {
                return None;
            }
            Some(format!("{}={}", name, value))
        })
        .collect();

    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

pub fn cookie_header_from_netscape(content: &str) -> Option<String> {
    let pairs: Vec<String> = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("# Netscape") {
                return None;
            }
            let normalized = trimmed.strip_prefix("#HttpOnly_").unwrap_or(trimmed);
            if normalized.starts_with('#') {
                return None;
            }
            let parts: Vec<&str> = normalized.split('\t').collect();
            if parts.len() < 7 {
                return None;
            }
            let name = parts[5].trim();
            let value = parts[6].trim();
            if name.is_empty() {
                return None;
            }
            Some(format!("{}={}", name, value))
        })
        .collect();

    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

pub fn cookie_header_for_domains(domains: &[&str]) -> Option<String> {
    for domain in domains {
        if let Some(header) = manual_cookie_header(domain) {
            let trimmed = header.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        let Some(path) = cookie_path_for(domain) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(header) = cookie_header_from_netscape_for_domain(&content, domain) {
            return Some(header);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_netscape_cookies_by_domain() {
        let content = "# Netscape HTTP Cookie File\n.example.com\tTRUE\t/\tFALSE\t0\tsid\tbad\n.x.com\tTRUE\t/\tFALSE\t0\tauth_token\tok\ntwitter.com\tFALSE\t/\tFALSE\t0\tct0\tcsrf\n";

        let x_header = cookie_header_from_netscape_for_domain(content, "x.com").unwrap();
        assert_eq!(x_header, "auth_token=ok");

        let twitter_header =
            cookie_header_from_netscape_for_domain(content, "twitter.com").unwrap();
        assert_eq!(twitter_header, "ct0=csrf");
    }

    #[test]
    fn does_not_include_unrelated_domains() {
        let content = ".youtube.com\tTRUE\t/\tFALSE\t0\tSID\tsecret\n";
        assert!(cookie_header_from_netscape_for_domain(content, "x.com").is_none());
    }
}
