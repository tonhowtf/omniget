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
        Platform::ShortPost => "x.com",
        Platform::PhotoFeed => "instagram.com",
        Platform::ClipFeed => "tiktok.com",
        Platform::Forums => "reddit.com",
        Platform::PinBoard => "pinterest.com",
        Platform::CodeHub => "github.com",
    }
}

pub fn session_cookie(p: Platform) -> &'static str {
    match p {
        Platform::ShortPost => "auth_token",
        Platform::PhotoFeed => "sessionid",
        Platform::ClipFeed => "sessionid",
        Platform::Forums => "reddit_session",
        Platform::PinBoard => "_pinterest_sess",
        Platform::CodeHub => "user_session",
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
    if cookie(b, p, session_cookie(p)).await.is_some() {
        return true;
    }
    match p {
        Platform::Forums => cookie(b, p, "token_v2")
            .await
            .is_some_and(|t| super::forums::token_is_user(&t)),
        _ => false,
    }
}

pub fn site_origin(urls: &[String], p: Platform) -> Option<String> {
    let site = host(p);
    urls.iter().find_map(|raw| {
        let url = url::Url::parse(raw).ok()?;
        let name = url.host_str()?;
        (url.scheme() == "https" && (name == site || name.ends_with(&format!(".{site}"))))
            .then(|| url.origin().ascii_serialization())
    })
}

pub fn live_origin(b: &Browser, p: Platform) -> Option<String> {
    let urls: Vec<String> = b.tabs().into_iter().map(|t| t.url).collect();
    site_origin(&urls, p)
}

pub async fn origin(p: Platform) -> String {
    match running().await.and_then(|b| live_origin(&b, p)) {
        Some(found) if p == Platform::PinBoard => found,
        _ => p.origin().to_string(),
    }
}

pub async fn ensure_tab(b: &Browser, p: Platform) -> Result<(), String> {
    let probe = format!("{}/", p.origin());
    let open = |b: &Browser| match p {
        Platform::PinBoard => live_origin(b, p).is_some(),
        _ => b.tab_for_origin(&probe).is_some(),
    };
    if open(b) {
        return Ok(());
    }
    browser::warm_up(&probe, false).await;
    for _ in 0..20 {
        if open(b) {
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
    eval_for(b, p, expression, Duration::from_secs(60)).await
}

pub async fn eval_for(
    b: &Browser,
    p: Platform,
    expression: &str,
    timeout: Duration,
) -> Result<Value, String> {
    let probe = format!("{}/", p.origin());
    let tab = b
        .tab_for_origin(&probe)
        .ok_or("BROWSER_UNAVAILABLE: the site tab closed")?;
    b.eval(&tab.id, expression, timeout).await
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

#[cfg(test)]
mod origin_tests {
    use super::*;

    #[test]
    fn a_regional_pinboard_tab_gives_the_origin_to_call() {
        let tabs = vec![
            "about:blank".to_string(),
            "https://x.com/home".to_string(),
            "https://br.pinterest.com/pin/1/".to_string(),
        ];
        assert_eq!(
            site_origin(&tabs, Platform::PinBoard).as_deref(),
            Some("https://br.pinterest.com")
        );
        assert_eq!(site_origin(&tabs, Platform::Forums), None);
        let fake = vec!["https://notpinterest.com/".to_string()];
        assert_eq!(site_origin(&fake, Platform::PinBoard), None);
    }
}
