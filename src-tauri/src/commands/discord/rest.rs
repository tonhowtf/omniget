use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode};
use serde_json::Value;

use super::props;

pub const LONG_BLOCK: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: u32 = 4;

pub fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(props::user_agent())
        .cookie_store(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http: {e}"))
}

pub fn web_headers(token: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    let mut put = |k: &'static str, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(k), v);
        }
    };
    if let Some(t) = token {
        put("authorization", t);
    }
    put("origin", props::ORIGIN);
    put("x-discord-locale", "en-US");
    put("accept-language", "en-US,en;q=0.9");
    if let Some(v) = h.get_mut("authorization") {
        v.set_sensitive(true);
    }
    h
}

pub fn route_key(method: &Method, path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let mut out = String::with_capacity(path.len() + 8);
    out.push_str(method.as_str());
    out.push(' ');
    let mut prev = "";
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        let is_id = seg.bytes().all(|b| b.is_ascii_digit());
        let major = matches!(prev, "channels" | "guilds" | "webhooks");
        if is_id && !major {
            out.push_str(":id");
        } else {
            out.push_str(seg);
        }
        prev = seg;
    }
    out
}

#[derive(Default, Debug, Clone)]
pub struct Bucket {
    pub remaining: Option<u32>,
    pub reset_at: Option<Instant>,
}

impl Bucket {
    pub fn wait(&self, now: Instant) -> Duration {
        match (self.remaining, self.reset_at) {
            (Some(0), Some(at)) if at > now => at - now,
            _ => Duration::ZERO,
        }
    }

    pub fn update(&mut self, remaining: Option<u32>, reset_after: Option<f64>, now: Instant) {
        if let Some(r) = remaining {
            self.remaining = Some(r);
        }
        if let Some(secs) = reset_after.filter(|s| s.is_finite() && *s >= 0.0) {
            self.reset_at = Some(now + Duration::from_secs_f64(secs.min(3600.0)));
        }
    }

    pub fn block_for(&mut self, wait: Duration, now: Instant) {
        self.remaining = Some(0);
        self.reset_at = Some(now + wait);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateHeaders {
    pub remaining: Option<u32>,
    pub reset_after: Option<f64>,
    pub retry_after: Option<f64>,
    pub global: bool,
}

pub fn rate_headers(h: &HeaderMap) -> RateHeaders {
    let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::trim);
    RateHeaders {
        remaining: get("x-ratelimit-remaining").and_then(|v| v.parse().ok()),
        reset_after: get("x-ratelimit-reset-after").and_then(|v| v.parse().ok()),
        retry_after: get("retry-after").and_then(|v| v.parse().ok()),
        global: get("x-ratelimit-global").is_some_and(|v| v.eq_ignore_ascii_case("true"))
            || get("x-ratelimit-scope") == Some("global"),
    }
}

pub fn retry_wait(headers: &RateHeaders, body: &Value) -> (Duration, bool) {
    let secs = body
        .get("retry_after")
        .and_then(|v| v.as_f64())
        .or(headers.retry_after)
        .or(headers.reset_after)
        .unwrap_or(1.0)
        .clamp(0.0, 24.0 * 3600.0);
    let global = headers.global || body.get("global").and_then(|v| v.as_bool()) == Some(true);
    (
        Duration::from_secs_f64(secs) + Duration::from_millis(100),
        global,
    )
}

pub fn rate_limited_error(wait: Duration) -> String {
    format!("rate_limited:{}", wait.as_secs().max(1))
}

pub fn error_for(status: StatusCode, body: &Value) -> String {
    let msg = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    match status.as_u16() {
        401 => "unauthorized".into(),
        403 => format!("forbidden:{msg}"),
        404 => format!("not_found:{msg}"),
        code => format!("http_{code}:{msg}"),
    }
}

#[derive(Default)]
struct Limits {
    routes: HashMap<String, Arc<tokio::sync::Mutex<Bucket>>>,
    global_until: Option<Instant>,
}

pub struct Rest {
    http: reqwest::Client,
    headers: HeaderMap,
    limits: Mutex<Limits>,
    on_unauthorized: Box<dyn Fn() + Send + Sync>,
}

impl Rest {
    pub fn new(token: &str, on_unauthorized: Box<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        Ok(Self {
            http: http_client()?,
            headers: web_headers(Some(token)),
            limits: Mutex::new(Limits::default()),
            on_unauthorized,
        })
    }

    pub fn cdn(&self) -> &reqwest::Client {
        &self.http
    }

    fn bucket(&self, key: &str) -> Arc<tokio::sync::Mutex<Bucket>> {
        let mut l = self.limits.lock().unwrap_or_else(|p| p.into_inner());
        if l.routes.len() > 512 {
            l.routes.retain(|_, b| Arc::strong_count(b) > 1);
        }
        l.routes.entry(key.to_string()).or_default().clone()
    }

    fn global_wait(&self, now: Instant) -> Duration {
        let l = self.limits.lock().unwrap_or_else(|p| p.into_inner());
        match l.global_until {
            Some(at) if at > now => at - now,
            _ => Duration::ZERO,
        }
    }

    fn set_global(&self, until: Instant) {
        let mut l = self.limits.lock().unwrap_or_else(|p| p.into_inner());
        l.global_until = Some(l.global_until.map_or(until, |u| u.max(until)));
    }

    pub async fn get(&self, path: &str) -> Result<Value, String> {
        self.request(Method::GET, path, None).await
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let key = route_key(&method, path);
        let bucket = self.bucket(&key);
        let mut bucket = bucket.lock().await;
        let url = format!("{}{}", props::API_BASE, path);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let now = Instant::now();
            let wait = bucket.wait(now).max(self.global_wait(now));
            if wait > LONG_BLOCK {
                return Err(rate_limited_error(wait));
            }
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            let mut req = self
                .http
                .request(method.clone(), &url)
                .headers(self.headers.clone());
            if let Some(b) = &body {
                req = req.json(b);
            } else if method == Method::POST || method == Method::PUT || method == Method::PATCH {
                req = req.header("content-type", "application/json").body("{}");
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e)
                    if method == Method::GET
                        && attempt < MAX_ATTEMPTS
                        && (e.is_timeout() || e.is_connect()) =>
                {
                    tokio::time::sleep(Duration::from_millis(700 * attempt as u64)).await;
                    continue;
                }
                Err(e) => return Err(format!("network: {}", e.without_url())),
            };
            let status = resp.status();
            let rate = rate_headers(resp.headers());
            bucket.update(rate.remaining, rate.reset_after, Instant::now());
            let text = resp.text().await.unwrap_or_default();
            let json: Value = if text.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap_or(Value::Null)
            };
            if status == StatusCode::TOO_MANY_REQUESTS {
                let (wait, global) = retry_wait(&rate, &json);
                let now = Instant::now();
                if global {
                    self.set_global(now + wait);
                } else {
                    bucket.block_for(wait, now);
                }
                tracing::warn!("[discord] rate limited on {} for {:?}", key, wait);
                if wait > LONG_BLOCK || attempt >= MAX_ATTEMPTS {
                    return Err(rate_limited_error(wait));
                }
                continue;
            }
            if status.is_server_error() && method == Method::GET && attempt < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                continue;
            }
            if status == StatusCode::UNAUTHORIZED {
                (self.on_unauthorized)();
            }
            if !status.is_success() {
                return Err(error_for(status, &json));
            }
            return Ok(json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_keys_keep_major_params_and_fold_the_rest() {
        assert_eq!(
            route_key(&Method::GET, "/channels/123/messages?before=999&limit=50"),
            "GET /channels/123/messages"
        );
        assert_eq!(
            route_key(&Method::PATCH, "/channels/123/messages/456"),
            "PATCH /channels/123/messages/:id"
        );
        assert_eq!(
            route_key(&Method::DELETE, "/channels/123/messages/789"),
            route_key(&Method::DELETE, "/channels/123/messages/456")
        );
        assert_ne!(
            route_key(&Method::GET, "/channels/1/messages"),
            route_key(&Method::GET, "/channels/2/messages")
        );
        assert_eq!(route_key(&Method::GET, "/users/@me"), "GET /users/@me");
        assert_eq!(
            route_key(&Method::GET, "/guilds/5/members/77"),
            "GET /guilds/5/members/:id"
        );
    }

    #[test]
    fn bucket_waits_only_when_empty_and_not_reset() {
        let now = Instant::now();
        let mut b = Bucket::default();
        assert_eq!(b.wait(now), Duration::ZERO);
        b.update(Some(3), Some(2.0), now);
        assert_eq!(b.wait(now), Duration::ZERO);
        b.update(Some(0), Some(1.5), now);
        assert_eq!(b.wait(now), Duration::from_millis(1500));
        assert_eq!(
            b.wait(now + Duration::from_secs(1)),
            Duration::from_millis(500)
        );
        assert_eq!(b.wait(now + Duration::from_secs(2)), Duration::ZERO);
        b.update(None, Some(f64::NAN), now);
        assert_eq!(b.wait(now), Duration::from_millis(1500));
        b.block_for(Duration::from_secs(4), now);
        assert_eq!(b.wait(now), Duration::from_secs(4));
    }

    #[test]
    fn reads_rate_limit_headers() {
        let mut h = HeaderMap::new();
        h.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        h.insert("x-ratelimit-reset-after", HeaderValue::from_static("0.837"));
        h.insert("x-ratelimit-global", HeaderValue::from_static("true"));
        let r = rate_headers(&h);
        assert_eq!(r.remaining, Some(0));
        assert_eq!(r.reset_after, Some(0.837));
        assert!(r.global);
        assert!(!rate_headers(&HeaderMap::new()).global);
    }

    #[test]
    fn retry_after_prefers_the_body_and_flags_global() {
        let none = rate_headers(&HeaderMap::new());
        let (w, g) = retry_wait(
            &none,
            &serde_json::json!({"retry_after": 2.5, "global": true}),
        );
        assert_eq!(w, Duration::from_millis(2600));
        assert!(g);
        let mut h = HeaderMap::new();
        h.insert("retry-after", HeaderValue::from_static("7"));
        let (w, g) = retry_wait(&rate_headers(&h), &Value::Null);
        assert_eq!(w, Duration::from_millis(7100));
        assert!(!g);
        assert!(w < LONG_BLOCK);
        let (w, _) = retry_wait(&none, &serde_json::json!({"retry_after": 600.0}));
        assert!(w > LONG_BLOCK);
        assert_eq!(rate_limited_error(w), "rate_limited:600");
    }

    #[test]
    fn errors_never_echo_the_token() {
        let h = web_headers(Some("secret.token.value"));
        assert!(h.get("authorization").unwrap().is_sensitive());
        assert!(format!("{:?}", h).contains("Sensitive"));
        assert_eq!(
            error_for(StatusCode::UNAUTHORIZED, &Value::Null),
            "unauthorized"
        );
        assert_eq!(
            error_for(
                StatusCode::FORBIDDEN,
                &serde_json::json!({"message": "Missing Access"})
            ),
            "forbidden:Missing Access"
        );
    }
}
