//! Downloads › Tools › Proxy check: does this proxy work, which address and
//! country do sites see through it, and can a given link be read through it.
//!
//! The idea comes from ytdlpgui, whose window has a proxy box (pre-filled with
//! `127.0.0.1:7890`) that goes to yt-dlp's `--proxy` on every download. Here
//! the proxy is tested before it is used, and can then be saved as the app's
//! proxy. Reimplemented; ytdlpgui has no license, so nothing was copied.

use std::time::{Duration, Instant};

use serde::Serialize;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

/// Cloudflare's trace page answers with `ip=` and `loc=` lines.
const TRACE_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RouteResult {
    pub ok: bool,
    pub ip: Option<String>,
    /// Two-letter country code.
    pub country: Option<String>,
    pub ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SiteResult {
    pub ok: bool,
    pub title: Option<String>,
    pub ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProxyCheck {
    /// The proxy that was tested, with the password hidden.
    pub proxy: String,
    pub via_proxy: RouteResult,
    pub direct: RouteResult,
    pub site: Option<SiteResult>,
}

/// `127.0.0.1:7890` → `http://127.0.0.1:7890`, as yt-dlp reads a proxy without a scheme.
pub fn normalize_proxy(raw: &str) -> Result<String, String> {
    let p = raw.trim();
    if p.is_empty() {
        return Err("Type a proxy, like socks5://127.0.0.1:7890".to_string());
    }
    let with_scheme = if p.contains("://") {
        p.to_string()
    } else {
        format!("http://{}", p)
    };
    let scheme = with_scheme.split("://").next().unwrap_or_default();
    if !matches!(scheme, "http" | "https" | "socks5" | "socks5h" | "socks4") {
        return Err(format!("Unknown proxy type \"{}\"", scheme));
    }
    let rest = with_scheme.split("://").nth(1).unwrap_or_default();
    let host_port = rest.rsplit('@').next().unwrap_or_default();
    let port_ok = host_port
        .rsplit_once(':')
        .map(|(h, port)| !h.is_empty() && port.parse::<u16>().is_ok())
        .unwrap_or(false);
    if !port_ok {
        return Err("The proxy needs a host and a port, like 127.0.0.1:7890".to_string());
    }
    Ok(with_scheme)
}

/// Hides the password of `scheme://user:pass@host:port`.
pub fn redact(proxy: &str) -> String {
    match proxy.split_once("://") {
        Some((scheme, rest)) => match rest.rsplit_once('@') {
            Some((auth, host)) => {
                let user = auth.split(':').next().unwrap_or_default();
                format!("{}://{}:***@{}", scheme, user, host)
            }
            None => proxy.to_string(),
        },
        None => proxy.to_string(),
    }
}

pub fn parse_trace(body: &str) -> (Option<String>, Option<String>) {
    let field = |k: &str| {
        body.lines()
            .find_map(|l| l.strip_prefix(k))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    (field("ip="), field("loc="))
}

async fn trace(proxy: Option<&str>) -> RouteResult {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(12));
    builder = match proxy {
        Some(p) => match reqwest::Proxy::all(p) {
            Ok(px) => builder.proxy(px),
            Err(e) => {
                return RouteResult {
                    error: Some(e.to_string()),
                    ..Default::default()
                }
            }
        },
        None => builder.no_proxy(),
    };
    let client = match builder.build() {
        Ok(c) => c,
        Err(e) => {
            return RouteResult {
                error: Some(e.to_string()),
                ..Default::default()
            }
        }
    };
    let t0 = Instant::now();
    match client.get(TRACE_URL).send().await {
        Ok(resp) => {
            let ms = t0.elapsed().as_millis() as u64;
            let body = resp.text().await.unwrap_or_default();
            let (ip, country) = parse_trace(&body);
            RouteResult {
                ok: ip.is_some(),
                ip,
                country,
                ms: Some(ms),
                error: None,
            }
        }
        Err(e) => RouteResult {
            error: Some(if e.is_timeout() {
                "No answer in 12 s".to_string()
            } else if e.is_connect() {
                "Could not connect to the proxy".to_string()
            } else {
                e.to_string()
            }),
            ..Default::default()
        },
    }
}

/// `proxy`: what to test; empty means the proxy saved in Settings.
/// `url`: optional link to read through the proxy with yt-dlp.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn check_proxy(proxy: Option<String>, url: Option<String>) -> Result<ProxyCheck, String> {
    let proxy = match proxy.filter(|p| !p.trim().is_empty()) {
        Some(p) => normalize_proxy(&p)?,
        None => omniget_core::core::http_client::proxy_url().ok_or_else(|| {
            "No proxy is saved in Settings › Network. Type one to test it.".to_string()
        })?,
    };
    let (via_proxy, direct) = tokio::join!(trace(Some(&proxy)), trace(None));

    let site = match url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()) {
        Some(u) => {
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err("Paste a web address that starts with http:// or https://".to_string());
            }
            omniget_core::core::platform_optout::ensure_allowed(&u)?;
            let ytdlp_path = ytdlp::ensure_ytdlp()
                .await
                .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
            let t0 = Instant::now();
            let flags = vec!["--proxy".to_string(), proxy.clone()];
            Some(match ytdlp::get_video_info(&ytdlp_path, &u, &flags).await {
                Ok(json) => SiteResult {
                    ok: true,
                    title: json
                        .get("title")
                        .and_then(|t| t.as_str())
                        .map(str::to_string),
                    ms: Some(t0.elapsed().as_millis() as u64),
                    error: None,
                },
                Err(e) => SiteResult {
                    ok: false,
                    title: None,
                    ms: Some(t0.elapsed().as_millis() as u64),
                    error: Some(e.to_string()),
                },
            })
        }
        None => None,
    };

    Ok(ProxyCheck {
        proxy: redact(&proxy),
        via_proxy,
        direct,
        site,
    })
}
