use std::sync::Arc;
use std::time::Duration;

use omniget_core::core::global_dl::browser::{self, Browser};
use serde_json::{json, Map, Value};

use super::model::Platform;

pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Result<Value, String> {
        serde_json::from_str(&self.body).map_err(|_| {
            let head: String = self.body.chars().take(160).collect();
            format!(
                "UPSTREAM_CHANGED: the site answered {} with something that is not JSON: {}",
                self.status,
                head.trim()
            )
        })
    }
}

pub fn host(p: Platform) -> &'static str {
    match p {
        Platform::X => "x.com",
        Platform::Instagram => "instagram.com",
        Platform::Tiktok => "tiktok.com",
    }
}

pub fn session_cookie(p: Platform) -> &'static str {
    match p {
        Platform::X => "auth_token",
        Platform::Instagram => "sessionid",
        Platform::Tiktok => "sessionid",
    }
}

pub async fn running() -> Option<Arc<Browser>> {
    browser::current()
}

pub async fn ensure_browser() -> Result<Arc<Browser>, String> {
    browser::start(None, true)
        .await
        .map_err(|e| format!("BROWSER_UNAVAILABLE: {e}"))
}

pub async fn cookie(b: &Browser, p: Platform, name: &str) -> Option<String> {
    b.cookies_for(host(p))
        .await
        .ok()?
        .into_iter()
        .find(|c| c["name"].as_str() == Some(name))
        .and_then(|c| c["value"].as_str().map(str::to_string))
        .filter(|v| !v.is_empty())
}

pub async fn logged_in(b: &Browser, p: Platform) -> bool {
    cookie(b, p, session_cookie(p)).await.is_some()
}

pub async fn ensure_tab(b: &Browser, p: Platform) -> Result<(), String> {
    let probe = format!("{}/", p.origin());
    if b.tab_for_origin(&probe).is_some() {
        return Ok(());
    }
    browser::warm_up(&probe, false).await;
    for _ in 0..20 {
        if b.tab_for_origin(&probe).is_some() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!(
        "BROWSER_UNAVAILABLE: could not open {} in the OmniGet browser",
        p.origin()
    ))
}

pub async fn ready(p: Platform) -> Result<Arc<Browser>, String> {
    let b = ensure_browser().await?;
    if !logged_in(&b, p).await {
        return Err(format!(
            "NOT_LOGGED_IN: sign in to {} in OmniGet first (open the {} screen and press Sign in)",
            p.label(),
            p.label()
        ));
    }
    ensure_tab(&b, p).await?;
    Ok(b)
}

pub async fn fetch(
    p: Platform,
    method: &str,
    url: &str,
    headers: Map<String, Value>,
    body: Option<String>,
) -> Result<Reply, String> {
    let r = browser::fetch(url, method, &headers, body.as_deref()).await?;
    let status = r["status"].as_u64().unwrap_or(0) as u16;
    let body = r["body"].as_str().unwrap_or_default().to_string();
    match status {
        200..=299 => Ok(Reply { status, body }),
        401 => Err(format!(
            "NOT_LOGGED_IN: {} rejected the session (401); sign in again",
            p.label()
        )),
        429 => Err(format!(
            "RATE_LIMITED: {} is limiting requests from this account; wait a few minutes",
            p.label()
        )),
        0 => Err(format!("NETWORK: could not reach {}", p.label())),
        _ => {
            let head: String = body.chars().take(300).collect();
            Err(format!(
                "UPSTREAM_{status}: {} answered {status}: {}",
                p.label(),
                head.trim()
            ))
        }
    }
}

pub async fn eval(b: &Browser, p: Platform, expression: &str) -> Result<Value, String> {
    let probe = format!("{}/", p.origin());
    let tab = b
        .tab_for_origin(&probe)
        .ok_or("BROWSER_UNAVAILABLE: the site tab closed")?;
    b.eval(&tab.id, expression, Duration::from_secs(60)).await
}

pub fn headers(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn query(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
