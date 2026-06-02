use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

pub const DEFAULT_REFERER: &str = "https://www.bilibili.com";
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

const REQUEST_TIMEOUT_SECS: u64 = 20;
const MAX_RETRIES: u32 = 3;
const RETRY_BACKOFF_MS: u64 = 600;

#[derive(Debug, Error)]
pub enum BilibiliError {
    #[error("errors.bilibili.network_failed")]
    Network(#[source] reqwest::Error),
    #[error("errors.bilibili.http_status")]
    HttpStatus(StatusCode),
    #[error("errors.bilibili.invalid_json")]
    InvalidJson(#[source] serde_json::Error),
    #[error("errors.bilibili.api_code")]
    ApiCode { code: i64, message: String },
    #[error("errors.bilibili.not_logged_in")]
    NotLoggedIn,
    #[error("errors.bilibili.geo_blocked")]
    GeoBlocked,
    #[error("errors.bilibili.rate_limited")]
    RateLimited,
    #[error("errors.bilibili.premium_required")]
    PremiumRequired,
    #[error("errors.bilibili.content_unavailable")]
    ContentUnavailable,
    #[error("errors.bilibili.cookie_missing")]
    CookieMissing,
    #[error("errors.bilibili.cancelled")]
    Cancelled,
}

impl BilibiliError {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            BilibiliError::Network(_) => "errors.bilibili.network_failed",
            BilibiliError::HttpStatus(_) => "errors.bilibili.http_status",
            BilibiliError::InvalidJson(_) => "errors.bilibili.invalid_json",
            BilibiliError::ApiCode { .. } => "errors.bilibili.api_code",
            BilibiliError::NotLoggedIn => "errors.bilibili.not_logged_in",
            BilibiliError::GeoBlocked => "errors.bilibili.geo_blocked",
            BilibiliError::RateLimited => "errors.bilibili.rate_limited",
            BilibiliError::PremiumRequired => "errors.bilibili.premium_required",
            BilibiliError::ContentUnavailable => "errors.bilibili.content_unavailable",
            BilibiliError::CookieMissing => "errors.bilibili.cookie_missing",
            BilibiliError::Cancelled => "errors.bilibili.cancelled",
        }
    }
}

pub type Result<T> = std::result::Result<T, BilibiliError>;

#[derive(Clone)]
pub struct ApiClient {
    inner: Client,
    referer: String,
    user_agent: String,
    cookie_header: Option<String>,
    account_slug: Option<String>,
}

impl ApiClient {
    pub fn new() -> Result<Self> {
        let inner = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(BilibiliError::Network)?;
        Ok(Self {
            inner,
            referer: DEFAULT_REFERER.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            cookie_header: None,
            account_slug: None,
        })
    }

    pub fn with_referer(mut self, referer: impl Into<String>) -> Self {
        self.referer = referer.into();
        self
    }

    pub fn with_user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    pub fn with_account(mut self, slug: impl Into<String>) -> Self {
        let slug = slug.into();
        self.cookie_header = build_cookie_header_for_account(&slug);
        if self.cookie_header.is_some() {
            self.account_slug = Some(slug);
        }
        self
    }

    pub fn with_anonymous_cookies(mut self) -> Self {
        self.cookie_header = build_cookie_header_for_account("_anonymous");
        self
    }

    pub fn with_raw_cookies(mut self, header: impl Into<String>) -> Self {
        self.cookie_header = Some(header.into());
        self
    }

    pub fn account_slug(&self) -> Option<&str> {
        self.account_slug.as_deref()
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub fn cookie_header(&self) -> Option<&str> {
        self.cookie_header.as_deref()
    }

    fn build_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.referer) {
            h.insert(REFERER, v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.user_agent) {
            h.insert(USER_AGENT, v);
        }
        if let Some(c) = self.cookie_header.as_ref() {
            if let Ok(v) = HeaderValue::from_str(c) {
                h.insert(COOKIE, v);
            }
        }
        h
    }

    pub async fn get_json(&self, url: &str) -> Result<Value> {
        let mut last_err: Option<BilibiliError> = None;
        for attempt in 0..MAX_RETRIES {
            match self
                .inner
                .get(url)
                .headers(self.build_headers())
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let text = resp.text().await.map_err(BilibiliError::Network)?;
                        let json: Value =
                            serde_json::from_str(&text).map_err(BilibiliError::InvalidJson)?;
                        return Ok(json);
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS {
                        last_err = Some(BilibiliError::RateLimited);
                    } else if status.is_server_error() {
                        last_err = Some(BilibiliError::HttpStatus(status));
                    } else {
                        return Err(BilibiliError::HttpStatus(status));
                    }
                }
                Err(e) => {
                    last_err = Some(BilibiliError::Network(e));
                }
            }
            tokio::time::sleep(Duration::from_millis(RETRY_BACKOFF_MS * 2u64.pow(attempt))).await;
        }
        Err(last_err.unwrap_or(BilibiliError::ContentUnavailable))
    }

    pub async fn post_json(&self, url: &str, body: &Value) -> Result<Value> {
        let resp = self
            .inner
            .post(url)
            .headers(self.build_headers())
            .json(body)
            .send()
            .await
            .map_err(BilibiliError::Network)?;
        let status = resp.status();
        let text = resp.text().await.map_err(BilibiliError::Network)?;
        if !status.is_success() {
            return Err(BilibiliError::HttpStatus(status));
        }
        serde_json::from_str(&text).map_err(BilibiliError::InvalidJson)
    }

    pub async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<Value> {
        let resp = self
            .inner
            .post(url)
            .headers(self.build_headers())
            .form(form)
            .send()
            .await
            .map_err(BilibiliError::Network)?;
        let status = resp.status();
        let text = resp.text().await.map_err(BilibiliError::Network)?;
        if !status.is_success() {
            return Err(BilibiliError::HttpStatus(status));
        }
        serde_json::from_str(&text).map_err(BilibiliError::InvalidJson)
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .inner
            .get(url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(BilibiliError::Network)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(BilibiliError::HttpStatus(status));
        }
        let bytes = resp.bytes().await.map_err(BilibiliError::Network)?;
        Ok(bytes.to_vec())
    }

    pub async fn head_content_length(&self, url: &str) -> Result<Option<u64>> {
        let resp = self
            .inner
            .head(url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(BilibiliError::Network)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(BilibiliError::HttpStatus(status));
        }
        Ok(resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok()))
    }

    pub async fn resolve_redirect(&self, url: &str) -> Result<String> {
        let no_follow = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(BilibiliError::Network)?;
        let resp = no_follow
            .get(url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(BilibiliError::Network)?;
        if let Some(loc) = resp.headers().get(reqwest::header::LOCATION) {
            if let Ok(s) = loc.to_str() {
                return Ok(s.to_string());
            }
        }
        Ok(url.to_string())
    }
}

pub fn check_api_response(value: &Value) -> Result<&Value> {
    let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code == 0 {
        if let Some(data) = value.get("data") {
            return Ok(data);
        }
        if let Some(result) = value.get("result") {
            return Ok(result);
        }
        return Ok(value);
    }
    let message = value
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Err(map_api_code(code, message))
}

pub fn map_api_code(code: i64, message: String) -> BilibiliError {
    match code {
        -101 | -400 => BilibiliError::NotLoggedIn,
        -403 | 62002 => BilibiliError::ContentUnavailable,
        -404 | 6002003 => BilibiliError::ContentUnavailable,
        -412 => BilibiliError::RateLimited,
        87007 | 87008 => BilibiliError::PremiumRequired,
        62004 => BilibiliError::GeoBlocked,
        _ => BilibiliError::ApiCode { code, message },
    }
}

pub fn parse_data<T: DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(BilibiliError::InvalidJson)
}

fn build_cookie_header_for_account(slug: &str) -> Option<String> {
    let path = crate::cookies::account_path_for_consumer("bilibili.com", Some(slug))?;
    let content = std::fs::read_to_string(&path).ok()?;
    cookie_header_from_netscape(&content)
}

fn cookie_header_from_netscape(content: &str) -> Option<String> {
    let mut pairs: Vec<String> = Vec::new();
    for raw in content.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        let effective = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            rest
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };
        let mut parts = effective.split('\t');
        let _domain = parts.next();
        let _include = parts.next();
        let _path = parts.next();
        let _secure = parts.next();
        let _expires = parts.next();
        let name = parts.next();
        let value = parts.next();
        if let (Some(n), Some(v)) = (name, value) {
            pairs.push(format!("{}={}", n, v));
        }
    }
    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_header_keeps_httponly_netscape_cookies() {
        let raw = "# Netscape HTTP Cookie File\n#HttpOnly_.bilibili.com\tTRUE\t/\tTRUE\t1830000000\tSESSDATA\tabc123\n.bilibili.com\tTRUE\t/\tTRUE\t1830000000\tbili_jct\tcsrf\n";
        let header = cookie_header_from_netscape(raw).unwrap();

        assert!(header.contains("SESSDATA=abc123"));
        assert!(header.contains("bili_jct=csrf"));
        assert!(!header.contains("#HttpOnly_"));
    }

    #[test]
    fn cookie_header_skips_regular_comments() {
        let raw = "# comment\n.bilibili.com\tTRUE\t/\tTRUE\t1830000000\tDedeUserID\t42\n";
        let header = cookie_header_from_netscape(raw).unwrap();

        assert_eq!(header, "DedeUserID=42");
    }
}
