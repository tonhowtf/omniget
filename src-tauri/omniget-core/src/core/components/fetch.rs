use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::cache::{self, CacheEntry};
use super::{validate_url, CACHE_TTL_SECS, FETCH_TIMEOUT_SECS, MAX_BODY_BYTES};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FetchError {
    pub code: String,
    pub detail: Option<String>,
}

impl FetchError {
    fn new(code: &str, detail: Option<String>) -> Self {
        Self {
            code: code.into(),
            detail,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchResult {
    pub url: String,
    pub body: serde_json::Value,
    pub fetched_at: u64,
    pub from_cache: bool,
    pub stale: bool,
    pub offline_error: Option<FetchError>,
}

struct PublicResolver;

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let public: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| !super::is_private_ip(a.ip()))
                .collect();
            if public.is_empty() {
                return Err("the host resolves only to private addresses".into());
            }
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn client() -> Result<reqwest::Client, FetchError> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects")
        } else if validate_url(attempt.url().as_str()).is_err() {
            attempt.error("redirect to a disallowed address")
        } else {
            attempt.follow()
        }
    });
    crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .redirect(policy)
        .dns_resolver(std::sync::Arc::new(PublicResolver))
        .user_agent("OmniGet-Components")
        .build()
        .map_err(|e| FetchError::new("network", Some(e.to_string())))
}

pub fn parse_body(bytes: &[u8]) -> Result<serde_json::Value, FetchError> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(FetchError::new("too_large", None));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| FetchError::new("not_json", Some(e.to_string())))?;
    if !value.is_object() && !value.is_array() {
        return Err(FetchError::new("not_json", None));
    }
    Ok(value)
}

async fn download(url: &url::Url) -> Result<serde_json::Value, FetchError> {
    let client = client()?;
    let mut response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                FetchError::new("timeout", None)
            } else {
                FetchError::new("network", Some(e.to_string()))
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::new(
            "http_status",
            Some(status.as_u16().to_string()),
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n as usize > MAX_BODY_BYTES)
    {
        return Err(FetchError::new("too_large", None));
    }
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|e| {
            if e.is_timeout() {
                FetchError::new("timeout", None)
            } else {
                FetchError::new("network", Some(e.to_string()))
            }
        })?;
        let Some(chunk) = chunk else { break };
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(FetchError::new("too_large", None));
        }
        buf.extend_from_slice(&chunk);
    }
    parse_body(&buf)
}

pub fn from_entry(
    entry: CacheEntry,
    stale: bool,
    offline_error: Option<FetchError>,
) -> FetchResult {
    FetchResult {
        url: entry.url,
        body: entry.body,
        fetched_at: entry.fetched_at,
        from_cache: true,
        stale,
        offline_error,
    }
}

pub async fn fetch_json(
    raw_url: &str,
    force: bool,
    cache_dir: Option<&Path>,
) -> Result<FetchResult, FetchError> {
    let url = validate_url(raw_url).map_err(|code| FetchError::new(code, None))?;
    let key = url.as_str().to_string();
    let cached = cache_dir.and_then(|d| cache::read(d, &key));
    let now = cache::now_secs();
    if !force {
        if let Some(entry) = cached.as_ref() {
            if cache::is_fresh(entry, now, CACHE_TTL_SECS) {
                return Ok(from_entry(entry.clone(), false, None));
            }
        }
    }
    match download(&url).await {
        Ok(body) => {
            let entry = CacheEntry {
                url: key.clone(),
                fetched_at: now,
                body,
            };
            if let Some(dir) = cache_dir {
                let _ = cache::write(dir, &entry);
            }
            Ok(FetchResult {
                url: entry.url,
                body: entry.body,
                fetched_at: now,
                from_cache: false,
                stale: false,
                offline_error: None,
            })
        }
        Err(err) => match cached {
            Some(entry) if err.code != "too_large" && err.code != "not_json" => {
                let stale = !cache::is_fresh(&entry, now, CACHE_TTL_SECS);
                Ok(from_entry(entry, stale, Some(err)))
            }
            _ => Err(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolver_refuses_hosts_that_only_resolve_privately() {
        use reqwest::dns::Resolve;
        use std::str::FromStr;
        let name = reqwest::dns::Name::from_str("localhost").unwrap();
        assert!(PublicResolver.resolve(name).await.is_err());
    }

    #[test]
    fn parse_body_accepts_objects_and_arrays_only() {
        assert!(parse_body(br#"{"items":[]}"#).is_ok());
        assert!(parse_body(br#"[{"name":"a"}]"#).is_ok());
        assert_eq!(parse_body(b"42").unwrap_err().code, "not_json");
        assert_eq!(parse_body(b"<html>").unwrap_err().code, "not_json");
    }

    #[test]
    fn parse_body_enforces_size_limit() {
        let big = vec![b' '; MAX_BODY_BYTES + 1];
        assert_eq!(parse_body(&big).unwrap_err().code, "too_large");
    }

    #[tokio::test]
    async fn invalid_url_is_rejected_before_network() {
        let err = fetch_json("http://ui.shadcn.com/r/index.json", false, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, "url_not_https");
    }

    #[tokio::test]
    async fn fresh_cache_is_served_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://registry.invalid/r/registry.json";
        cache::write(
            dir.path(),
            &CacheEntry {
                url: url.into(),
                fetched_at: cache::now_secs(),
                body: serde_json::json!({ "items": [] }),
            },
        )
        .unwrap();
        let got = fetch_json(url, false, Some(dir.path())).await.unwrap();
        assert!(got.from_cache);
        assert!(!got.stale);
        assert!(got.offline_error.is_none());
    }
}
