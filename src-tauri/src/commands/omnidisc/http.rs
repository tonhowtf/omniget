use super::{normalize_instance_url, store};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

pub const ERR_UNAUTHORIZED: &str = "ERR_UNAUTHORIZED";
pub const ERR_FORBIDDEN: &str = "ERR_FORBIDDEN";
pub const ERR_NOT_FOUND: &str = "ERR_NOT_FOUND";
pub const ERR_RATE_LIMITED: &str = "ERR_RATE_LIMITED";
pub const ERR_UNREACHABLE: &str = "ERR_UNREACHABLE";
pub const ERR_SERVER: &str = "ERR_SERVER";
pub const ERR_BAD_REQUEST: &str = "ERR_BAD_REQUEST";
pub const ERR_NO_SESSION: &str = "ERR_NO_SESSION";

/// Last line of defence for every request this module makes: the assembled path
/// must still be the route we wrote, whatever was interpolated into it.
fn safe_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains("..")
        && !path.contains("//")
        && path.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '@' | '%' | '~')
        })
}

pub fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent(format!("OmniGet/{}", env!("CARGO_PKG_VERSION")))
            .timeout(timeout),
    )
    .build()
    .map_err(|e| format!("OmniDisc: could not build HTTP client: {}", e))
}

pub struct Api {
    pub base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl Api {
    pub fn public(url: &str) -> Result<Self, String> {
        let base = normalize_instance_url(url)?;
        Ok(Self {
            base,
            token: None,
            http: http_client(Duration::from_secs(15))?,
        })
    }

    pub fn authed(url: &str) -> Result<Self, String> {
        let base = normalize_instance_url(url)?;
        let token = store::load_token(&base)?.ok_or_else(|| ERR_NO_SESSION.to_string())?;
        Ok(Self {
            base,
            token: Some(token),
            http: http_client(Duration::from_secs(15))?,
        })
    }

    pub fn with_token(base: String, token: String) -> Result<Self, String> {
        Ok(Self {
            base,
            token: Some(token),
            http: http_client(Duration::from_secs(15))?,
        })
    }

    pub async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<T, String> {
        let text = self.raw(method, path, query, body).await?;
        serde_json::from_str(&text).map_err(|e| {
            tracing::warn!(
                "[omnidisc] {}{} returned unexpected JSON: {}",
                self.base,
                path,
                e
            );
            ERR_SERVER.to_string()
        })
    }

    pub async fn send_empty(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(), String> {
        self.raw(method, path, &[], body).await.map(|_| ())
    }

    async fn raw(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<String, String> {
        if !safe_path(path) {
            tracing::warn!("[omnidisc] refused a request path that is not a plain route");
            return Err(format!("{}:invalid_path", ERR_BAD_REQUEST));
        }
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.request(method.clone(), &url);
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await.map_err(|e| {
            tracing::warn!("[omnidisc] {} {} unreachable: {}", method, url, e);
            ERR_UNREACHABLE.to_string()
        })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(text);
        }
        let code = serde_json::from_str::<omnidisc_proto::rest::ApiError>(&text)
            .map(|e| e.code)
            .unwrap_or_default();
        tracing::warn!(
            "[omnidisc] {} {} -> {} {}",
            method,
            url,
            status.as_u16(),
            code
        );
        Err(map_error(status, &code))
    }
}

pub fn map_error(status: StatusCode, code: &str) -> String {
    match status {
        StatusCode::UNAUTHORIZED => ERR_UNAUTHORIZED.to_string(),
        StatusCode::FORBIDDEN => with_code(ERR_FORBIDDEN, code),
        StatusCode::NOT_FOUND => ERR_NOT_FOUND.to_string(),
        StatusCode::TOO_MANY_REQUESTS => ERR_RATE_LIMITED.to_string(),
        s if s.is_client_error() => with_code(ERR_BAD_REQUEST, code),
        _ => ERR_SERVER.to_string(),
    }
}

fn with_code(base: &str, code: &str) -> String {
    if code.is_empty() {
        base.to_string()
    } else {
        format!("{}:{}", base, code)
    }
}
