use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProviderHealth {
    Ready { version: Option<String> },
    Unreachable { base_url: String },
    Unhealthy { detail: String },
    NotConfigured,
}

impl ProviderHealth {
    pub fn is_usable(&self) -> bool {
        matches!(self, ProviderHealth::Ready { .. })
    }
}

pub fn normalize_base_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let with_scheme = if t.starts_with("http://") || t.starts_with("https://") {
        t.to_string()
    } else {
        format!("http://{t}")
    };
    Some(with_scheme)
}

pub fn extractor_args(health: &ProviderHealth, base_url: &str) -> Vec<String> {
    if !health.is_usable() {
        return Vec::new();
    }
    let Some(url) = normalize_base_url(base_url) else {
        return Vec::new();
    };
    vec![
        "--extractor-args".to_string(),
        format!("youtubepot-bgutilhttp:base_url={url}"),
    ]
}

pub fn stderr_indicates_missing_token(stderr_lower: &str) -> bool {
    const SIGNS: &[&str] = &[
        "a po token is required",
        "po token is missing",
        "missing a po token",
        "content is not available on this app",
        "please sign in to confirm you're not a bot",
        "sign in to confirm you're not a bot",
    ];
    SIGNS.iter().any(|s| stderr_lower.contains(s))
}
