use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{cap_chars, output_file, write_new, DevError};

pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
pub const DEFAULT_BODY_CHARS: usize = 4000;
pub const MASK: &str = "***";
pub const DEFAULT_TIMEOUT_MS: u64 = 20_000;
pub const MAX_TIMEOUT_MS: u64 = 60_000;
pub const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;
pub(super) const USER_AGENT: &str = concat!("OmniGet-DevTools/", env!("CARGO_PKG_VERSION"));
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_body_chars: Option<usize>,
    pub save_body_to: Option<String>,
    pub parse_as: ParseAs,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParseAs {
    #[default]
    Auto,
    Json,
    Text,
    Base64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyPreview {
    pub preview: String,
    pub truncated: bool,
    pub total: usize,
    pub json: bool,
    pub binary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Redirect {
    pub status: u16,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    pub total_ms: u64,
    pub first_byte_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EchoedRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub redirects: Vec<Redirect>,
    pub final_url: String,
    pub timing: Timing,
    pub body: BodyPreview,
    pub body_bytes: usize,
    pub body_complete: bool,
    pub body_path: Option<String>,
    pub request: EchoedRequest,
    pub curl: String,
    pub untrusted: bool,
}

pub fn is_secret_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "api-key",
        "apikey",
        "token",
        "secret",
        "password",
    ]
    .iter()
    .any(|mark| name.contains(mark))
}

pub fn masked(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let shown = if is_secret_header(name) { MASK } else { value };
            (name.clone(), shown.to_string())
        })
        .collect()
}

fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn curl(
    method: &str,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<&str>,
) -> String {
    let mut out = format!("curl -X {method} {}", quoted(url));
    for (name, value) in masked(headers) {
        out.push_str(&format!(" -H {}", quoted(&format!("{name}: {value}"))));
    }
    if let Some(body) = body {
        out.push_str(&format!(" --data-raw {}", quoted(body)));
    }
    out
}

pub fn preview(bytes: &[u8], max_chars: usize) -> BodyPreview {
    preview_as(bytes, max_chars, ParseAs::Auto)
}

pub fn preview_as(bytes: &[u8], max_chars: usize, parse_as: ParseAs) -> BodyPreview {
    if parse_as == ParseAs::Base64 {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let (preview, truncated, total) = cap_chars(&encoded, max_chars);
        return BodyPreview {
            preview,
            truncated,
            total,
            json: false,
            binary: true,
            parse_error: None,
        };
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return BodyPreview {
            preview: String::new(),
            truncated: !bytes.is_empty(),
            total: 0,
            json: false,
            binary: true,
            parse_error: None,
        };
    };
    let parsed = match parse_as {
        ParseAs::Text => None,
        _ => Some(serde_json::from_str::<serde_json::Value>(text)),
    };
    let (shown, json, parse_error) = match parsed {
        Some(Ok(value)) => (format!("{value:#}"), true, None),
        Some(Err(e)) if parse_as == ParseAs::Json => (text.to_string(), false, Some(e.to_string())),
        _ => (text.to_string(), false, None),
    };
    let (preview, truncated, total) = cap_chars(&shown, max_chars);
    BodyPreview {
        preview,
        truncated,
        total,
        json,
        binary: false,
        parse_error,
    }
}

pub fn expand_env(
    text: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    used: &mut Vec<String>,
) -> Result<String, DevError> {
    const OPEN: &str = "{{env.";
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find("}}") else { break };
        let name = &after[..end];
        let value = lookup(name).filter(|v| !v.is_empty()).ok_or_else(|| {
            DevError::InvalidInput(format!(
                "the environment variable {name} is not set for the OmniGet app"
            ))
        })?;
        out.push_str(&rest[..start]);
        out.push_str(&value);
        used.push(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

pub fn scrub(text: &str, secrets: &[String]) -> String {
    secrets.iter().fold(text.to_string(), |shown, secret| {
        shown.replace(secret, MASK)
    })
}

pub fn checked_url(raw: &str) -> Result<url::Url, DevError> {
    let url = url::Url::parse(raw)
        .map_err(|e| DevError::InvalidInput(format!("invalid url \"{raw}\": {e}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(DevError::InvalidInput(format!(
            "only http and https URLs are supported, got \"{raw}\""
        )));
    }
    Ok(url)
}

pub fn checked(request: &HttpRequest) -> Result<(reqwest::Method, url::Url), DevError> {
    let method = request.method.to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(DevError::InvalidInput(format!(
            "unknown method \"{}\"; use one of {}",
            request.method,
            METHODS.join(", ")
        )));
    }
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| DevError::InvalidInput(e.to_string()))?;
    Ok((method, checked_url(&request.url)?))
}

pub(super) fn network_error(error: reqwest::Error) -> DevError {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(inner) = source {
        message = format!("{message}: {inner}");
        source = inner.source();
    }
    DevError::Network {
        message,
        timeout: error.is_timeout(),
    }
}

pub(super) async fn read_capped(
    response: &mut reqwest::Response,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), DevError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        let room = max_bytes - bytes.len();
        if chunk.len() > room {
            bytes.extend_from_slice(&chunk[..room]);
            return Ok((bytes, false));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, true))
}

pub async fn send(original: HttpRequest) -> Result<HttpResponse, DevError> {
    let env = |name: &str| std::env::var(name).ok();
    let mut secrets = Vec::new();
    let mut request = original.clone();
    request.url = expand_env(&original.url, &env, &mut secrets)?;
    for value in request.headers.values_mut() {
        *value = expand_env(value, &env, &mut secrets)?;
    }
    if let Some(body) = &original.body {
        request.body = Some(expand_env(body, &env, &mut secrets)?);
    }
    let (method, url) = checked(&request)?;
    let shown_url =
        checked_url(&original.url).map_or_else(|_| original.url.clone(), |u| u.to_string());
    let timeout = request
        .timeout_ms
        .map_or(DEFAULT_TIMEOUT_MS, |ms| ms.min(MAX_TIMEOUT_MS));
    let hops: Arc<Mutex<Vec<Redirect>>> = Arc::default();
    let seen = hops.clone();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout))
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if let Ok(mut hops) = seen.lock() {
                hops.push(Redirect {
                    status: attempt.status().as_u16(),
                    url: attempt.url().to_string(),
                });
            }
            attempt.follow()
        }))
        .build()
        .map_err(network_error)?;
    let mut builder = client.request(method.clone(), url.clone());
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = &request.body {
        builder = builder.body(body.clone());
    }
    let started = Instant::now();
    let mut response = builder.send().await.map_err(network_error)?;
    let first_byte_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in response.headers() {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        headers
            .entry(name.to_string())
            .and_modify(|joined| *joined = format!("{joined}, {value}"))
            .or_insert(value);
    }
    let (bytes, body_complete) = read_capped(&mut response, MAX_BODY_BYTES).await?;
    let total_ms = started.elapsed().as_millis() as u64;
    let mut body = preview_as(
        &bytes,
        request.max_body_chars.unwrap_or(DEFAULT_BODY_CHARS),
        request.parse_as,
    );
    body.preview = scrub(&body.preview, &secrets);
    let body_path = match &request.save_body_to {
        Some(path) => Some(PathBuf::from(path)),
        None if body.truncated => Some(output_file("http-body", "bin")),
        None => None,
    };
    if let Some(path) = &body_path {
        write_new(path, &bytes)?;
    }
    let redirects = hops
        .lock()
        .map(|hops| hops.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|hop| Redirect {
            status: hop.status,
            url: scrub(&hop.url, &secrets),
        })
        .collect();
    Ok(HttpResponse {
        status,
        headers: masked(&headers),
        redirects,
        final_url: scrub(&final_url, &secrets),
        timing: Timing {
            total_ms,
            first_byte_ms,
        },
        body,
        body_bytes: bytes.len(),
        body_complete,
        body_path: body_path.map(|path| path.to_string_lossy().into_owned()),
        curl: curl(
            method.as_str(),
            &shown_url,
            &original.headers,
            original.body.as_deref(),
        ),
        request: EchoedRequest {
            method: method.to_string(),
            url: shown_url,
            headers: masked(&original.headers),
        },
        untrusted: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn credential_headers_are_masked_and_the_rest_is_kept() {
        let out = masked(&headers(&[
            ("Authorization", "Bearer abc"),
            ("Cookie", "sid=1"),
            ("X-Api-Key", "k"),
            ("Set-Cookie", "sid=2"),
            ("X-Auth-Token", "t"),
            ("Accept", "application/json"),
        ]));
        assert_eq!(out["Authorization"], MASK);
        assert_eq!(out["Cookie"], MASK);
        assert_eq!(out["X-Api-Key"], MASK);
        assert_eq!(out["Set-Cookie"], MASK);
        assert_eq!(out["X-Auth-Token"], MASK);
        assert_eq!(out["Accept"], "application/json");
    }

    #[test]
    fn curl_export_quotes_for_the_shell_and_never_shows_a_credential() {
        let text = curl(
            "POST",
            "https://api.test/v1?q=it's",
            &headers(&[
                ("Authorization", "Bearer abc"),
                ("Content-Type", "application/json"),
            ]),
            Some("{\"a\":\"it's\"}"),
        );
        assert_eq!(
            text,
            "curl -X POST 'https://api.test/v1?q=it'\\''s' -H 'Authorization: ***' -H 'Content-Type: application/json' --data-raw '{\"a\":\"it'\\''s\"}'"
        );
        assert_eq!(
            curl("GET", "https://a.test/", &BTreeMap::new(), None),
            "curl -X GET 'https://a.test/'"
        );
    }

    #[test]
    fn a_json_body_is_pretty_printed_then_capped() {
        let out = preview(b"{\"a\":[1,2]}", 100);
        assert_eq!(out.preview, "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
        assert!(out.json && !out.truncated && !out.binary);
        let out = preview(b"{\"a\":[1,2]}", 5);
        assert_eq!(out.preview, "{\n  \"");
        assert!(out.truncated);
        assert_eq!(out.total, 29);
    }

    #[test]
    fn text_is_kept_as_is_and_binary_is_not_previewed() {
        let out = preview("olá mundo".as_bytes(), 3);
        assert_eq!(out.preview, "olá");
        assert!(out.truncated && !out.json);
        assert_eq!(out.total, 9);
        let out = preview(&[0xff, 0xd8, 0x00, 0x10], 100);
        assert!(out.binary);
        assert_eq!(out.preview, "");
    }

    #[test]
    fn env_references_are_filled_in_and_a_missing_one_is_named() {
        let env = |name: &str| (name == "API_KEY").then(|| "s3cret".to_string());
        let mut used = Vec::new();
        assert_eq!(
            expand_env("Bearer {{env.API_KEY}}", &env, &mut used).unwrap(),
            "Bearer s3cret"
        );
        assert_eq!(used, vec!["s3cret".to_string()]);
        assert_eq!(
            expand_env("plain {{ text }}", &env, &mut used).unwrap(),
            "plain {{ text }}"
        );
        let missing = expand_env("{{env.NOPE}}", &env, &mut used).unwrap_err();
        assert!(missing.to_string().contains("NOPE"));
    }

    #[test]
    fn a_secret_that_came_from_the_environment_is_scrubbed_from_what_is_shown() {
        let secrets = vec!["s3cret".to_string()];
        assert_eq!(
            scrub("https://a.test/?k=s3cret&x=1", &secrets),
            "https://a.test/?k=***&x=1"
        );
        assert_eq!(scrub("nothing here", &secrets), "nothing here");
    }

    #[test]
    fn the_body_can_be_forced_to_text_json_or_base64() {
        let json = br#"{"a":1}"#;
        let as_text = preview_as(json, 100, ParseAs::Text);
        assert_eq!(as_text.preview, r#"{"a":1}"#);
        assert!(!as_text.json);
        let forced = preview_as(b"not json", 100, ParseAs::Json);
        assert!(!forced.json);
        assert!(forced.parse_error.is_some());
        assert_eq!(forced.preview, "not json");
        let b64 = preview_as(&[0xff, 0x00, 0x01], 100, ParseAs::Base64);
        assert_eq!(b64.preview, "/wAB");
        assert!(b64.binary);
        assert!(!b64.truncated);
        let auto = preview_as(json, 100, ParseAs::Auto);
        assert!(auto.json);
        assert_eq!(auto.parse_error, None);
    }

    #[test]
    fn only_http_urls_and_known_methods_are_sent() {
        let request = |method: &str, url: &str| HttpRequest {
            method: method.into(),
            url: url.into(),
            ..Default::default()
        };
        assert!(checked(&request("get", "https://example.com")).is_ok());
        assert!(matches!(
            checked(&request("GET", "file:///etc/passwd")),
            Err(DevError::InvalidInput(_))
        ));
        assert!(matches!(
            checked(&request("BREW", "https://example.com")),
            Err(DevError::InvalidInput(_))
        ));
        assert!(matches!(
            checked(&request("GET", "not a url")),
            Err(DevError::InvalidInput(_))
        ));
    }

    async fn serve(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap();
                seen.push(String::from_utf8_lossy(&buffer[..read]).to_string());
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            }
            seen
        });
        (base, handle)
    }

    #[tokio::test]
    async fn a_loopback_request_reports_redirects_timing_and_a_masked_echo() {
        let body = "{\"ok\":true,\"items\":[1,2,3]}";
        let (base, server) = serve(vec![
            "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: sid=secret\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let saved = dir.path().join("body.json");
        let out = send(HttpRequest {
            method: "GET".into(),
            url: format!("{base}/start"),
            headers: headers(&[("Authorization", "Bearer abc")]),
            max_body_chars: Some(12),
            save_body_to: Some(saved.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
        let seen = server.await.unwrap();
        assert!(seen[0].to_lowercase().contains("authorization: bearer abc"));
        assert_eq!(out.status, 200);
        assert_eq!(
            out.redirects,
            vec![Redirect {
                status: 302,
                url: format!("{base}/final")
            }]
        );
        assert_eq!(out.final_url, format!("{base}/final"));
        assert!(out.body.json && out.body.truncated);
        assert_eq!(out.body.preview.chars().count(), 12);
        assert_eq!(out.body_bytes, body.len());
        assert!(out.body_complete);
        assert_eq!(out.headers["set-cookie"], MASK);
        assert_eq!(out.request.headers["Authorization"], MASK);
        assert!(out.curl.contains("-H 'Authorization: ***'"));
        assert!(out.timing.total_ms >= out.timing.first_byte_ms);
        assert!(out.untrusted);
        assert_eq!(out.body_path.as_deref(), saved.to_str());
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), body);
    }

    #[tokio::test]
    async fn a_truncated_body_is_saved_to_a_new_file_even_when_no_path_was_given() {
        let body = "x".repeat(300);
        let (base, server) = serve(vec![format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 300\r\nConnection: close\r\n\r\n{body}"
        )])
        .await;
        let out = send(HttpRequest {
            method: "POST".into(),
            url: base,
            body: Some("payload".into()),
            max_body_chars: Some(10),
            ..Default::default()
        })
        .await
        .unwrap();
        let seen = server.await.unwrap();
        assert!(seen[0].ends_with("payload"));
        assert_eq!(out.status, 500);
        let path = out.body_path.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn a_refused_connection_is_a_retryable_network_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let error = send(HttpRequest {
            method: "GET".into(),
            url,
            ..Default::default()
        })
        .await
        .unwrap_err();
        assert!(matches!(error, DevError::Network { timeout: false, .. }));
        assert!(error.retryable());
    }

    #[tokio::test]
    #[ignore = "network: run by hand with --ignored"]
    async fn example_com_answers_over_the_real_network() {
        let out = send(HttpRequest {
            method: "GET".into(),
            url: "https://example.com".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(out.status, 200);
        assert!(out.body.preview.contains("Example Domain"));
    }
}
