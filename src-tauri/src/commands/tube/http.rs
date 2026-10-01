//! Network transport for OmniTube. Every request to YouTube, YouTube Music
//! and the side services (SponsorBlock, DeArrow, RYD, LRCLIB) goes through
//! the clients here.
//!
//! * `client()`        JSON/text calls: gzip/brotli, 20 s timeout, no cookie
//!   store (the account jar lives in `account` and is sent by hand)
//! * `media_client()`  googlevideo byte ranges for the proxy: no decompression
//!   and no total timeout (the proxy has its own stall guard)
//!
//! `innertube()` signs requests with the account (SAPISIDHASH) when asked,
//! folds `Set-Cookie` back into the jar, and renews the `visitorData` once
//! when YouTube rejects the session.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use omniget_core::core::tube::innertube::{self, ClientDef};
use omniget_core::models::settings::{AppSettings, TubeSettings};
use serde_json::Value;

use super::account;

/// Desktop Safari: sw.js_data and the watch page answer it like a browser.
pub const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15";

const VISITOR_TTL: Duration = Duration::from_secs(6 * 3600);
const SETTINGS_TTL: Duration = Duration::from_secs(3);

pub fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        omniget_core::core::http_client::apply_global_proxy(
            reqwest::Client::builder()
                .user_agent(concat!(
                    "OmniGet/",
                    env!("CARGO_PKG_VERSION"),
                    " (https://github.com/tonhowtf/omniget)"
                ))
                .gzip(true)
                .brotli(true)
                .timeout(Duration::from_secs(20))
                .connect_timeout(Duration::from_secs(10)),
        )
        .build()
        .unwrap_or_default()
    })
}

pub fn media_client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        omniget_core::core::http_client::apply_global_proxy(
            reqwest::Client::builder()
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .connect_timeout(Duration::from_secs(10))
                .pool_idle_timeout(Duration::from_secs(60)),
        )
        .build()
        .unwrap_or_default()
    })
}

// ---------- settings ----------

/// The app settings, re-read at most every few seconds: commands call this
/// on every request and the store is a JSON file.
pub fn app_settings() -> AppSettings {
    static CACHE: Mutex<Option<(AppSettings, Instant)>> = Mutex::new(None);
    if let Ok(g) = CACHE.lock() {
        if let Some((s, at)) = g.as_ref() {
            if at.elapsed() < SETTINGS_TTL {
                return s.clone();
            }
        }
    }
    let s = crate::storage::config::load_settings_standalone();
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((s.clone(), Instant::now()));
    }
    s
}

pub fn settings() -> TubeSettings {
    app_settings().tube
}

/// `(hl, gl)` from the app language: "pt-BR" -> ("pt", "BR"), "en" -> ("en", "US").
pub fn locale() -> (String, String) {
    locale_of(&app_settings().appearance.language)
}

fn locale_of(lang: &str) -> (String, String) {
    let lang = lang.trim().replace('_', "-");
    let mut parts = lang.split('-');
    let primary = parts.next().unwrap_or("").to_ascii_lowercase();
    let region = parts
        .find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|p| p.to_ascii_uppercase());
    if primary.is_empty() {
        return ("en".into(), "US".into());
    }
    let gl = region.clone().unwrap_or_else(|| {
        match primary.as_str() {
            "en" => "US",
            "pt" => "BR",
            "ja" => "JP",
            "ko" => "KR",
            "zh" => "CN",
            "uk" => "UA",
            "cs" => "CZ",
            "el" => "GR",
            "sv" => "SE",
            "da" => "DK",
            "he" => "IL",
            "hi" => "IN",
            "vi" => "VN",
            "ar" => "SA",
            other if other.len() == 2 => return other.to_ascii_uppercase(),
            _ => "US",
        }
        .to_string()
    });
    // YouTube knows zh-CN / zh-TW as their own hl values
    let hl = match (primary.as_str(), region.as_deref()) {
        ("zh", Some(r)) => format!("zh-{r}"),
        _ => primary,
    };
    (hl, gl)
}

// ---------- visitorData ----------

fn visitor_cache() -> &'static tokio::sync::Mutex<Option<(String, Instant)>> {
    static V: OnceLock<tokio::sync::Mutex<Option<(String, Instant)>>> = OnceLock::new();
    V.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Anonymous `visitorData`, reused for 6 h. Without it the player answers
/// "Sign in to confirm you're not a bot".
pub async fn visitor_data() -> Result<String, String> {
    let mut g = visitor_cache().lock().await;
    if let Some((v, at)) = g.as_ref() {
        if at.elapsed() < VISITOR_TTL {
            return Ok(v.clone());
        }
    }
    let v = fetch_visitor_data().await?;
    *g = Some((v.clone(), Instant::now()));
    Ok(v)
}

/// Forgets the cached visitorData so the next call fetches a fresh one.
pub async fn reset_visitor_data() {
    *visitor_cache().lock().await = None;
}

async fn fetch_visitor_data() -> Result<String, String> {
    let from_sw = async {
        let body = client()
            .get("https://www.youtube.com/sw.js_data")
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .header(reqwest::header::COOKIE, "SOCS=CAI")
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()?;
        innertube::parse_visitor_data_swjs(&body)
    }
    .await;
    if let Some(v) = from_sw {
        return Ok(v);
    }
    tracing::debug!("[tube] sw.js_data without visitorData, trying visitor_id");
    let c = innertube::client("WEB").ok_or("WEB client missing")?;
    let (hl, gl) = locale();
    let body = innertube::context(&c, None, &hl, &gl);
    let json = post(&c, "visitor_id", &body, None, None).await?;
    json.pointer("/responseContext/visitorData")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "no visitorData from YouTube".to_string())
}

// ---------- innertube ----------

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Account headers for a client that takes cookies: Cookie, Authorization
/// (SAPISIDHASH for the client's origin) and X-Goog-AuthUser.
pub fn auth_headers(
    jar: &innertube::CookieJar,
    origin: &str,
) -> Option<Vec<(&'static str, String)>> {
    let auth = innertube::auth_header(jar, origin, now_s())?;
    Some(vec![
        ("cookie", jar.header()),
        ("authorization", auth),
        ("x-goog-authuser", "0".to_string()),
    ])
}

fn set_cookies(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect()
}

/// Folds `Set-Cookie` values of a response from `host` into the account jar
/// and writes it back when a value changed (Google rotates `__Secure-*PSIDTS`).
pub fn absorb_cookies(host: &str, headers: &reqwest::header::HeaderMap) {
    let values = set_cookies(headers);
    if values.is_empty() {
        return;
    }
    let Some(mut jar) = account::jar() else {
        return;
    };
    if jar.merge_set_cookie_from(host, &values, now_s() as i64) {
        account::save_jar(&jar);
    }
}

async fn post(
    c: &ClientDef,
    endpoint: &str,
    body: &Value,
    visitor: Option<&str>,
    jar: Option<&innertube::CookieJar>,
) -> Result<Value, String> {
    let url = innertube::endpoint_url(c, endpoint);
    let mut req = client().post(&url).json(body);
    for (k, v) in innertube::headers(c, visitor) {
        req = req.header(k, v);
    }
    let signed = jar.and_then(|j| auth_headers(j, &c.origin()));
    match &signed {
        Some(h) => {
            for (k, v) in h {
                req = req.header(*k, v);
            }
        }
        // skip the EU consent interstitial on anonymous calls
        None => req = req.header(reqwest::header::COOKIE, "SOCS=CAI"),
    }
    let resp = req.send().await.map_err(|e| {
        tracing::debug!("[tube] {endpoint} failed: {e}");
        format!("network error ({endpoint})")
    })?;
    if signed.is_some() {
        absorb_cookies(&c.host, resp.headers());
    }
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("YouTube answered {} ({endpoint})", status.as_u16()));
    }
    resp.json::<Value>()
        .await
        .map_err(|_| format!("invalid response ({endpoint})"))
}

fn set_visitor(body: &mut Value, visitor: &str) {
    if let Some(client) = body.pointer_mut("/context/client") {
        if let Some(obj) = client.as_object_mut() {
            obj.insert("visitorData".into(), Value::String(visitor.to_string()));
        }
    }
}

/// POST `youtubei/v1/{endpoint}` with client `c`. With `auth`, a client that
/// takes cookies and an account in the vault, the call is signed. A 401/403
/// renews the visitorData and retries once.
pub async fn innertube(
    c: &ClientDef,
    endpoint: &str,
    body: Value,
    auth: bool,
) -> Result<Value, String> {
    let jar = if auth && c.supports_cookies {
        account::jar()
    } else {
        None
    };
    let visitor = visitor_data().await.ok();
    match post(c, endpoint, &body, visitor.as_deref(), jar.as_ref()).await {
        Err(e) if e.contains(" 401 ") || e.contains(" 403 ") => {
            tracing::debug!("[tube] {endpoint}: session rejected, renewing visitorData");
            reset_visitor_data().await;
            let visitor = visitor_data().await.ok();
            let mut body = body;
            if let Some(v) = visitor.as_deref() {
                set_visitor(&mut body, v);
            }
            post(c, endpoint, &body, visitor.as_deref(), jar.as_ref()).await
        }
        other => other,
    }
}

async fn with_client(
    name: &str,
    endpoint: &str,
    auth: bool,
    body_fn: impl FnOnce(&ClientDef, Option<&str>, &str, &str) -> Value + Send,
) -> Result<Value, String> {
    let c = innertube::client(name).ok_or_else(|| format!("{name} client missing"))?;
    let visitor = visitor_data().await.ok();
    let (hl, gl) = locale();
    let body = body_fn(&c, visitor.as_deref(), &hl, &gl);
    innertube(&c, endpoint, body, auth).await
}

/// WEB client (www.youtube.com), signed with the account when there is one.
pub async fn web(
    endpoint: &str,
    body_fn: impl FnOnce(&ClientDef, Option<&str>, &str, &str) -> Value + Send,
) -> Result<Value, String> {
    with_client("WEB", endpoint, true, body_fn).await
}

/// WEB_REMIX client (music.youtube.com), signed with the account when there is one.
pub async fn music(
    endpoint: &str,
    body_fn: impl FnOnce(&ClientDef, Option<&str>, &str, &str) -> Value + Send,
) -> Result<Value, String> {
    with_client("WEB_REMIX", endpoint, true, body_fn).await
}

/// Same as `web`/`music` but never signed (typeahead, lyrics lookups).
pub async fn anonymous(
    client_name: &str,
    endpoint: &str,
    body_fn: impl FnOnce(&ClientDef, Option<&str>, &str, &str) -> Value + Send,
) -> Result<Value, String> {
    with_client(client_name, endpoint, false, body_fn).await
}

/// `{"context": ...}` plus the given fields, for endpoints without a body
/// builder in the core (like, subscribe, edit_playlist, feedback...).
pub fn body_with(c: &ClientDef, visitor: Option<&str>, hl: &str, gl: &str, extra: Value) -> Value {
    let mut body = innertube::context(c, visitor, hl, gl);
    if let (Some(obj), Value::Object(more)) = (body.as_object_mut(), extra) {
        obj.extend(more);
    }
    body
}

// ---------- plain GETs ----------

pub async fn get_json(url: &str) -> Result<Value, String> {
    let resp = client()
        .get(url)
        .send()
        .await
        .map_err(|_| "network error".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    resp.json::<Value>()
        .await
        .map_err(|_| "invalid JSON".to_string())
}

pub async fn get_text(url: &str) -> Result<String, String> {
    let resp = client()
        .get(url)
        .send()
        .await
        .map_err(|_| "network error".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    resp.text()
        .await
        .map_err(|_| "invalid response".to_string())
}

/// Truncated URL for debug logs: googlevideo URLs are credentials.
pub fn redact(url: &str) -> String {
    let base = url.split('?').next().unwrap_or(url);
    base.chars().take(48).collect()
}
