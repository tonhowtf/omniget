use std::sync::LazyLock;
use std::sync::RwLock;

use crate::models::settings::ProxySettings;

static GLOBAL_PROXY: LazyLock<RwLock<ProxySettings>> =
    LazyLock::new(|| RwLock::new(ProxySettings::default()));

pub fn init_proxy(proxy: ProxySettings) {
    if let Ok(mut guard) = GLOBAL_PROXY.write() {
        *guard = proxy;
    }
}

pub fn get_proxy_snapshot() -> ProxySettings {
    GLOBAL_PROXY.read().map(|g| g.clone()).unwrap_or_default()
}

pub fn proxy_url() -> Option<String> {
    let proxy = get_proxy_snapshot();
    if !proxy.enabled || proxy.host.is_empty() {
        return None;
    }
    let scheme = match proxy.proxy_type.as_str() {
        "socks5" => "socks5",
        "https" => "https",
        _ => "http",
    };
    if !proxy.username.is_empty() {
        Some(format!(
            "{}://{}:{}@{}:{}",
            scheme, proxy.username, proxy.password, proxy.host, proxy.port
        ))
    } else {
        Some(format!("{}://{}:{}", scheme, proxy.host, proxy.port))
    }
}

pub fn apply_proxy(
    builder: reqwest::ClientBuilder,
    proxy: &ProxySettings,
) -> reqwest::ClientBuilder {
    if !proxy.enabled || proxy.host.is_empty() {
        return builder;
    }
    let scheme = match proxy.proxy_type.as_str() {
        "socks5" => "socks5",
        "https" => "https",
        _ => "http",
    };
    let proxy_url = if !proxy.username.is_empty() {
        format!(
            "{}://{}:{}@{}:{}",
            scheme, proxy.username, proxy.password, proxy.host, proxy.port
        )
    } else {
        format!("{}://{}:{}", scheme, proxy.host, proxy.port)
    };
    match reqwest::Proxy::all(&proxy_url) {
        Ok(p) => builder.proxy(p),
        Err(e) => {
            tracing::warn!("Invalid proxy URL: {}", e);
            builder
        }
    }
}

pub fn apply_global_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    // Pin rustls (webpki roots). Workspace feature unification turns on
    // reqwest's native-tls (librqbit default-tls, livekit), which silently
    // becomes the default backend; under the worker's Seatbelt profile trustd
    // is unreachable and every handshake fails with "bad certificate format".
    let builder = builder.use_rustls_tls();
    let proxy = get_proxy_snapshot();
    apply_proxy(builder, &proxy)
}

pub fn inject_ua_header(headers: &mut reqwest::header::HeaderMap, opts_ua: Option<&str>) {
    if let Some(ua) = opts_ua {
        if let Ok(v) = reqwest::header::HeaderValue::from_str(ua) {
            headers.insert(reqwest::header::USER_AGENT, v);
        }
    }
}

pub fn ua_header_map(opts_ua: Option<&str>) -> Option<reqwest::header::HeaderMap> {
    let ua = opts_ua?;
    let value = reqwest::header::HeaderValue::from_str(ua).ok()?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::USER_AGENT, value);
    Some(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `-p omniget-core -p omniget` test binary has reqwest's native-tls
    /// unified in, like the workspace-built worker (librqbit/livekit). A server
    /// answering the ClientHello with plain HTTP makes each backend fail with
    /// its own wording: rustls says "corrupt message", SecureTransport says
    /// something else. Under the worker Seatbelt (no trustd) native-tls fails
    /// every handshake with "bad certificate format" (benchmark 26/09).
    #[tokio::test]
    async fn global_builder_is_pinned_to_rustls() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut b = [0u8; 512];
            let _ = s.read(&mut b).await;
            let _ = s
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .await;
        });
        let client = apply_global_proxy(reqwest::Client::builder().no_proxy())
            .build()
            .unwrap();
        let err = client
            .get(format!("https://127.0.0.1:{}/", addr.port()))
            .send()
            .await
            .unwrap_err();
        let mut chain = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(e) = source {
            chain.push_str(&format!(" | {e}"));
            source = e.source();
        }
        server.abort();
        assert!(!chain.contains("bad certificate format"), "{chain}");
        assert!(chain.to_lowercase().contains("corrupt message"), "{chain}");
    }

    /// Every reqwest builder the worker can reach through a platform must go
    /// through apply_global_proxy (rustls pin); a bare builder falls back to
    /// native-tls and dies under the worker Seatbelt ("bad certificate
    /// format", bench 26/09 reddit-1). Scans the platform sources.
    #[test]
    fn platform_builders_go_through_apply_global_proxy() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platforms");
        let mut files = vec![root.clone()];
        let mut offenders = Vec::new();
        while let Some(p) = files.pop() {
            if p.is_dir() {
                for e in std::fs::read_dir(&p).unwrap().flatten() {
                    files.push(e.path());
                }
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            let src = src.split("#[cfg(test)]").next().unwrap_or("");
            let lines: Vec<&str> = src.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                // `Client::` itself, not a wrapper like `ApiClient::new()`.
                let bare = ["Client::builder()", "Client::new()"].iter().any(|pat| {
                    line.match_indices(pat).any(|(at, _)| {
                        !line[..at]
                            .chars()
                            .next_back()
                            .is_some_and(|c| c.is_alphanumeric() || c == '_')
                    })
                });
                if !bare {
                    continue;
                }
                let lo = i.saturating_sub(3);
                let hi = (i + 12).min(lines.len());
                let ctx = lines[lo..hi].join("\n");
                if !ctx.contains("apply_global_proxy") && !ctx.contains("use_rustls_tls") {
                    offenders.push(format!("{}:{}", p.display(), i + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "bare reqwest builders: {offenders:?}");
    }
}
