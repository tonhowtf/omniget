#[cfg(windows)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Clone)]
pub struct UdemySession {
    pub access_token: String,
    pub email: String,
    pub client: reqwest::Client,
    pub cookies: Vec<(String, String)>,
    pub portal_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub access_token: String,
    pub email: String,
    pub cookies: Vec<(String, String)>,
    pub saved_at: u64,
    #[serde(default = "default_portal")]
    pub portal_name: String,
}

fn default_portal() -> String {
    "www".into()
}

fn session_file_path() -> anyhow::Result<PathBuf> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| anyhow!("Could not find app data directory"))?;
    Ok(data_dir.join("omniget").join("udemy_session.json"))
}

const UDEMY_CLIENT_ID: &str = "TH96Ov3Ebo3OtgoSH5mOYzYolcowM3ycedWQDDce";
const UDEMY_CLIENT_SECRET: &str = "f2lgDUDxjFiOlVHUpwQNFUfCQPyMO0tJQMaud53PF01UKueW8enYjeEYoyVeP0bb2XVEDkJ5GLJaVTfM5QgMVz6yyXyydZdA5QhzgvG9UmCPUYaCrIVf7VpmiilfbLJc";

pub fn build_client_from_saved(saved: &SavedSession) -> anyhow::Result<reqwest::Client> {
    if saved.portal_name != "www" {
        return build_enterprise_client(saved);
    }

    let mut default_headers = HeaderMap::new();

    if !saved.access_token.is_empty() {
        let bearer = format!("Bearer {}", saved.access_token);
        default_headers.insert(
            "Authorization",
            HeaderValue::from_str(&bearer)?,
        );
        default_headers.insert(
            "X-Udemy-Authorization",
            HeaderValue::from_str(&bearer)?,
        );
    }

    default_headers.insert(
        "Accept",
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    default_headers.insert(
        "x-udemy-client-id",
        HeaderValue::from_static(UDEMY_CLIENT_ID),
    );
    default_headers.insert(
        "x-udemy-client-secret",
        HeaderValue::from_static(UDEMY_CLIENT_SECRET),
    );
    default_headers.insert(
        "accept-language",
        HeaderValue::from_static("en_US"),
    );

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent("okhttp/4.12.0 UdemyAndroid 9.51.2(594) (phone)")
        .default_headers(default_headers)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .build()?;

    Ok(client)
}

fn build_enterprise_client(saved: &SavedSession) -> anyhow::Result<reqwest::Client> {
    let mut default_headers = HeaderMap::new();

    let cookie_header = saved
        .cookies
        .iter()
        .map(|(name, value)| format!("{}={}", name, value))
        .collect::<Vec<_>>()
        .join("; ");

    if !cookie_header.is_empty() {
        default_headers.insert("Cookie", HeaderValue::from_str(&cookie_header)?);
    }

    if let Some(csrf) = saved.cookies.iter().find(|(n, _)| n == "csrftoken") {
        default_headers.insert("X-CSRFToken", HeaderValue::from_str(&csrf.1)?);
    }

    let origin = format!("https://{}.udemy.com", saved.portal_name);
    default_headers.insert("Referer", HeaderValue::from_str(&format!("{}/", origin))?);
    default_headers.insert("Origin", HeaderValue::from_str(&origin)?);
    default_headers.insert(
        "x-requested-with",
        HeaderValue::from_static("XMLHttpRequest"),
    );
    default_headers.insert(
        "Accept",
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    default_headers.insert(
        "accept-language",
        HeaderValue::from_static("en_US"),
    );

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .default_headers(default_headers)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .build()?;

    Ok(client)
}

pub async fn save_session(session: &UdemySession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        access_token: session.access_token.clone(),
        email: session.email.clone(),
        cookies: session.cookies.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        portal_name: session.portal_name.clone(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[udemy] session saved for {}, {} cookies", session.email, session.cookies.len());
    Ok(())
}

pub async fn load_saved_session() -> anyhow::Result<UdemySession> {
    let path = session_file_path()?;
    let json = tokio::fs::read_to_string(&path).await?;
    let saved: SavedSession = serde_json::from_str(&json)?;

    tracing::info!("[udemy] session loaded for {}, {} cookies", saved.email, saved.cookies.len());

    let client = build_client_from_saved(&saved)?;

    Ok(UdemySession {
        access_token: saved.access_token,
        email: saved.email,
        cookies: saved.cookies,
        client,
        portal_name: saved.portal_name,
    })
}

pub async fn delete_saved_session() -> anyhow::Result<()> {
    let path = session_file_path()?;
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

const COOKIE_URIS: &[&str] = &[
    "https://www.udemy.com",
    "https://udemy.com",
];

#[cfg(windows)]
async fn extract_webview_cookies_for_uri(
    window: &tauri::WebviewWindow,
    uri: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    use webview2_com::GetCookiesCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use windows_core::{Interface, HSTRING, PCWSTR, PWSTR};

    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<(String, String)>>();
    let uri_owned = uri.to_string();

    window
        .with_webview(move |webview| {
            unsafe {
                let core = webview.controller().CoreWebView2().unwrap();
                let core2: ICoreWebView2_2 = core.cast().unwrap();
                let manager = core2.CookieManager().unwrap();
                let uri_hstring = HSTRING::from(uri_owned);

                let _ = manager.GetCookies(
                    PCWSTR::from_raw(uri_hstring.as_ptr()),
                    &GetCookiesCompletedHandler::create(Box::new(
                        move |error_code, cookie_list| {
                            let mut result = Vec::new();
                            if error_code.is_ok() {
                                if let Some(list) = cookie_list {
                                    let mut count = 0u32;
                                    list.Count(&mut count)?;
                                    for i in 0..count {
                                        let cookie = list.GetValueAtIndex(i)?;
                                        let mut name = PWSTR::null();
                                        let mut value = PWSTR::null();
                                        cookie.Name(&mut name)?;
                                        cookie.Value(&mut value)?;
                                        result.push((
                                            webview2_com::take_pwstr(name),
                                            webview2_com::take_pwstr(value),
                                        ));
                                    }
                                }
                            }
                            let _ = tx.send(result);
                            Ok(())
                        },
                    )),
                );
            }
        })
        .map_err(|e| anyhow!("{}", e))?;

    let cookies = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| anyhow!("Timeout getting cookies from WebView"))?
        .map_err(|_| anyhow!("Cookie channel closed"))?;

    Ok(cookies)
}

#[cfg(windows)]
async fn extract_webview_cookies(
    window: &tauri::WebviewWindow,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut seen = HashMap::<String, String>::new();

    for uri in COOKIE_URIS {
        match extract_webview_cookies_for_uri(window, uri).await {
            Ok(cookies) => {
                let names: Vec<&str> = cookies.iter().map(|(n, _)| n.as_str()).collect();
                tracing::info!("[udemy] cookies {} → {} cookies: {:?}", uri, cookies.len(), names);
                for (name, value) in cookies {
                    seen.insert(name, value);
                }
            }
            Err(e) => {
                tracing::warn!("[udemy] cookies {} → error: {}", uri, e);
            }
        }
    }

    let cookies: Vec<(String, String)> = seen.into_iter().collect();
    tracing::info!("[udemy] total unique cookies: {}", cookies.len());

    Ok(cookies)
}

#[allow(dead_code)]
fn parse_document_cookie(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let (name, value) = pair.split_once('=')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

#[cfg(not(windows))]
async fn extract_webview_cookies_js(
    window: &tauri::WebviewWindow,
    cookie_data: &Arc<std::sync::Mutex<Option<String>>>,
) -> anyhow::Result<Vec<(String, String)>> {
    *cookie_data.lock().unwrap() = None;

    window
        .eval(
            "window.location.href = 'https://omniget-udemy-cookie-extract.local/?cookies=' + encodeURIComponent(document.cookie)",
        )
        .map_err(|e| anyhow!("JS eval failed: {}", e))?;

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(5);
    loop {
        if let Some(cookie_str) = cookie_data.lock().unwrap().take() {
            let cookies = parse_document_cookie(&cookie_str);
            let names: Vec<&str> = cookies.iter().map(|(n, _)| n.as_str()).collect();
            tracing::info!("[udemy] document.cookie → {} cookies: {:?}", cookies.len(), names);
            return Ok(cookies);
        }
        if start.elapsed() > timeout {
            tracing::warn!("[udemy] timeout waiting for document.cookie response");
            return Ok(Vec::new());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn strip_cookie_quotes(value: &str) -> &str {
    let v = value.trim();
    if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

pub async fn authenticate(
    app: &tauri::AppHandle,
    email: &str,
) -> anyhow::Result<UdemySession> {
    use tauri::Emitter;

    let post_login_detected = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let post_login_flag = post_login_detected.clone();

    #[cfg(not(windows))]
    let cookie_data = Arc::new(std::sync::Mutex::new(Option::<String>::None));
    #[cfg(not(windows))]
    let cookie_data_nav = cookie_data.clone();

    let email_json = serde_json::to_string(email)?;

    let init_script = format!(
        r#"(function() {{
            if (window.location.hostname !== 'www.udemy.com') return;
            var filled = false;
            function fillEmail() {{
                if (filled) return;
                var input = document.querySelector('input[name="email"], input[name="Email"], input[type="email"]');
                if (!input) return;
                filled = true;
                var s = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                s.call(input, {email_json});
                input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                input.dispatchEvent(new Event('change', {{ bubbles: true }}));
                setTimeout(function() {{
                    var btn = document.querySelector('button[type="submit"], form button');
                    if (btn) btn.click();
                }}, 500);
            }}
            setTimeout(fillEmail, 1500);
            setInterval(fillEmail, 500);
        }})()"#
    );

    if let Some(existing) = app.get_webview_window("udemy-login") {
        let _ = existing.close();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let login_url = "https://www.udemy.com/join/passwordless-auth/?locale=en_US&next=https%3A%2F%2Fwww.udemy.com%2F&response_type=html&action=login";

    let login_window = tauri::WebviewWindowBuilder::new(
        app,
        "udemy-login",
        tauri::WebviewUrl::External(login_url.parse().unwrap()),
    )
    .title("Udemy Login")
    .inner_size(500.0, 700.0)
    .initialization_script(&init_script)
    .on_navigation(move |url| {
        let host = url.host_str().unwrap_or("");
        let url_str = url.as_str();

        tracing::info!("[udemy] navigation → {} (host={})", url_str, host);

        if (host == "www.udemy.com" || host == "udemy.com")
            && !url_str.contains("/join/")
            && !url_str.contains("/passwordless-auth/")
        {
            post_login_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        #[cfg(not(windows))]
        {
            if host == "omniget-udemy-cookie-extract.local" {
                for (key, value) in url.query_pairs() {
                    if key == "cookies" {
                        *cookie_data_nav.lock().unwrap() = Some(value.to_string());
                        break;
                    }
                }
                return false;
            }
        }

        true
    })
    .build()
    .map_err(|e| anyhow!("Failed to create login window: {}", e))?;

    let _ = app.emit("udemy-auth-waiting-code", ());

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(180);
    let poll_start_delay = Duration::from_secs(5);
    let mut last_log_secs: u64 = 0;

    loop {
        if start.elapsed() > timeout {
            let _ = login_window.close();
            return Err(anyhow!("Timeout waiting for login. Check your email for the verification code."));
        }

        let nav_detected = post_login_detected.load(std::sync::atomic::Ordering::Relaxed);
        let should_poll = nav_detected || start.elapsed() > poll_start_delay;

        if should_poll {
            #[cfg(windows)]
            let cookies = extract_webview_cookies(&login_window).await?;
            #[cfg(not(windows))]
            let cookies = extract_webview_cookies_js(&login_window, &cookie_data).await?;

            let has_access_token = cookies.iter().any(|(name, _)| name == "access_token");
            let has_logged_in = cookies
                .iter()
                .any(|(name, value)| name == "ud_cache_logged_in" && value == "1");

            if has_access_token || has_logged_in {
                tracing::info!(
                    "[udemy] login detected after {:.1}s (access_token={}, ud_cache_logged_in={}, nav_detected={})",
                    start.elapsed().as_secs_f64(),
                    has_access_token,
                    has_logged_in,
                    nav_detected
                );
                let _ = login_window.close();

                let access_token = cookies
                    .iter()
                    .find(|(name, _)| name == "access_token")
                    .map(|(_, value)| strip_cookie_quotes(value).to_string())
                    .unwrap_or_default();

                let saved = SavedSession {
                    access_token: access_token.clone(),
                    email: email.to_string(),
                    cookies: cookies.clone(),
                    saved_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    portal_name: "www".into(),
                };

                let client = build_client_from_saved(&saved)?;

                return Ok(UdemySession {
                    access_token,
                    email: email.to_string(),
                    client,
                    cookies,
                    portal_name: "www".into(),
                });
            }

            let elapsed_secs = start.elapsed().as_secs();
            if elapsed_secs >= last_log_secs + 5 {
                last_log_secs = elapsed_secs;
                let cookie_names: Vec<&str> =
                    cookies.iter().map(|(n, _)| n.as_str()).collect();
                tracing::info!(
                    "[udemy] polling at {}s, {} cookies: {:?}",
                    elapsed_secs,
                    cookies.len(),
                    cookie_names
                );
            }
        }

        let sleep_ms = if nav_detected { 500 } else { 2000 };
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
    }
}

pub async fn authenticate_with_cookie_json(cookie_json_str: &str) -> anyhow::Result<UdemySession> {
    #[derive(Deserialize)]
    struct CookieEntry {
        name: String,
        value: String,
        #[allow(dead_code)]
        domain: Option<String>,
    }

    #[derive(Deserialize)]
    struct CookieExport {
        url: Option<String>,
        cookies: Vec<CookieEntry>,
    }

    let export: CookieExport = serde_json::from_str(cookie_json_str)
        .map_err(|e| anyhow!("Invalid cookie JSON: {}", e))?;

    if export.cookies.is_empty() {
        return Err(anyhow!("No cookies provided"));
    }

    // Detect portal from URL or cookie domains
    let portal_name = if let Some(ref url) = export.url {
        detect_portal_from_url(url)
    } else {
        export
            .cookies
            .iter()
            .filter_map(|c| c.domain.as_deref())
            .find_map(|d| {
                let d = d.trim_start_matches('.');
                d.strip_suffix(".udemy.com")
                    .map(|sub| sub.to_string())
            })
            .unwrap_or_else(|| "www".into())
    };

    let cookies: Vec<(String, String)> = export
        .cookies
        .iter()
        .map(|c| (c.name.clone(), c.value.clone()))
        .collect();

    let access_token = cookies
        .iter()
        .find(|(name, _)| name == "access_token")
        .map(|(_, value)| strip_cookie_quotes(value).to_string())
        .unwrap_or_default();

    // Try to extract email from ud_user_jwt cookie (base64url-encoded JWT)
    let email = cookies
        .iter()
        .find(|(name, _)| name == "ud_user_jwt")
        .and_then(|(_, value)| decode_jwt_email(value))
        .unwrap_or_else(|| format!("enterprise@{}.udemy.com", portal_name));

    let saved = SavedSession {
        access_token: access_token.clone(),
        email: email.clone(),
        cookies: cookies.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        portal_name: portal_name.clone(),
    };

    let client = build_client_from_saved(&saved)?;

    tracing::info!(
        "[udemy] cookie login for portal={}, email={}, {} cookies",
        portal_name,
        email,
        cookies.len()
    );

    Ok(UdemySession {
        access_token,
        email,
        client,
        cookies,
        portal_name,
    })
}

fn detect_portal_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .and_then(|host| {
            host.strip_suffix(".udemy.com")
                .map(|sub| sub.to_string())
        })
        .unwrap_or_else(|| "www".into())
}

fn decode_jwt_email(jwt: &str) -> Option<String> {
    use base64::Engine;

    // JWT has 3 parts: header.payload.signature
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(parts[1]))
        .ok()?;

    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    claims
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
