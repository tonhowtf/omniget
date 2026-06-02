use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthWebviewRequest {
    pub url: String,
    pub title: String,
    pub cookie_domains: Vec<String>,
    pub success_url_contains: Option<String>,
    pub wait_for_cookie: Option<String>,
    pub initialization_script: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthWebviewResult {
    pub cookies: Vec<AuthCookie>,
    pub final_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    #[serde(rename = "httpOnly")]
    pub http_only: bool,
    pub secure: bool,
}

#[tauri::command]
pub async fn open_auth_webview(
    app: AppHandle,
    request: AuthWebviewRequest,
) -> Result<AuthWebviewResult, String> {
    tracing::info!(
        "[auth_webview] opening: url={}, success_pattern={:?}, wait_for={:?}, domains={:?}",
        request.url,
        request.success_url_contains,
        request.wait_for_cookie,
        request.cookie_domains
    );

    let label = format!(
        "auth-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let width = request.width.unwrap_or(900.0);
    let height = request.height.unwrap_or(700.0);

    let parsed_url: url::Url = request
        .url
        .parse()
        .map_err(|e| format!("Invalid URL: {}", e))?;

    let login_path = parsed_url.path().to_string();
    let login_host = parsed_url.host_str().unwrap_or("").to_string();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4);

    let success_pattern = request.success_url_contains.clone();
    let tx_nav = tx.clone();

    let mut builder =
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(parsed_url))
            .title(&request.title)
            .inner_size(width, height)
            .center();

    if let Some(ref script) = request.initialization_script {
        builder = builder.initialization_script(script);
    }

    let webview_window = builder
        .on_navigation(move |url| {
            let url_str = url.to_string();
            tracing::debug!("[auth_webview] navigation: {}", url_str);

            let mut is_success = false;

            if let Some(ref pattern) = success_pattern {
                if url_str.contains(pattern) {
                    is_success = true;
                }
            }

            if !is_success && !login_host.is_empty() {
                if let Ok(nav_url) = url::Url::parse(&url_str) {
                    let nav_host = nav_url.host_str().unwrap_or("");
                    let nav_path = nav_url.path();
                    if nav_host.contains(&login_host) || login_host.contains(nav_host) {
                        if nav_path != login_path
                            && !nav_path.contains("login")
                            && !nav_path.contains("signin")
                            && !nav_path.contains("auth")
                            && !nav_path.contains("oauth")
                            && !nav_path.contains("signup")
                            && !nav_path.contains("register")
                        {
                            is_success = true;
                        }
                    }
                }
            }

            if is_success {
                let _ = tx_nav.try_send(url_str);
            }

            true
        })
        .build()
        .map_err(|e| format!("Failed to create auth window: {}", e))?;

    let tx_close = tx.clone();
    drop(tx);

    let ww_clone = webview_window.clone();
    webview_window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = tx_close.try_send("__CLOSE_REQUESTED__".to_string());
        }
    });

    let final_url = tokio::select! {
        msg = rx.recv() => {
            msg.ok_or_else(|| "Auth cancelled".to_string())?
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {
            tracing::warn!("[auth_webview] timed out after 5 minutes");
            let _ = ww_clone.destroy();
            return Err("Auth timed out".to_string());
        }
    };

    tracing::info!(
        "[auth_webview] signal: {}",
        if final_url == "__CLOSE_REQUESTED__" {
            "user closed window"
        } else {
            &final_url
        }
    );

    let default_domain = request.cookie_domains.first().cloned().unwrap_or_default();

    let cookies = if let Some(ref target_cookie) = request.wait_for_cookie {
        poll_for_cookie(
            &webview_window,
            &default_domain,
            &request.cookie_domains,
            target_cookie,
        )
        .await
    } else {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let cookies =
            extract_cookies(&webview_window, &default_domain, &request.cookie_domains).await;
        if cookies.is_empty() {
            tracing::warn!("[auth_webview] no cookies on first attempt, retrying in 3s...");
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            extract_cookies(&webview_window, &default_domain, &request.cookie_domains).await
        } else {
            cookies
        }
    };

    tracing::info!("[auth_webview] extracted {} cookies", cookies.len());
    for c in &cookies {
        tracing::debug!(
            "[auth_webview]   {} (httpOnly={}, domain={})",
            c.name,
            c.http_only,
            c.domain
        );
    }

    let _ = webview_window.destroy();

    Ok(AuthWebviewResult { cookies, final_url })
}

async fn poll_for_cookie(
    window: &tauri::WebviewWindow,
    default_domain: &str,
    _domains: &[String],
    target_cookie: &str,
) -> Vec<AuthCookie> {
    let target_lower = target_cookie.to_lowercase();
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        let elapsed = start.elapsed();
        if elapsed > timeout {
            tracing::warn!(
                "[auth_webview] cookie polling timed out after {:.1}s",
                elapsed.as_secs_f64()
            );
            return extract_cookies(window, default_domain, _domains).await;
        }

        let cookies = extract_cookies(window, default_domain, _domains).await;

        let found = cookies
            .iter()
            .any(|c| c.name.to_lowercase() == target_lower);
        if found {
            tracing::info!(
                "[auth_webview] target cookie '{}' found after {:.1}s ({} attempts, {} total cookies)",
                target_cookie,
                elapsed.as_secs_f64(),
                attempt,
                cookies.len()
            );
            return cookies;
        }

        if attempt % 4 == 0 {
            tracing::debug!(
                "[auth_webview] polling at {:.1}s, {} cookies, waiting for '{}'",
                elapsed.as_secs_f64(),
                cookies.len(),
                target_cookie
            );
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn extract_cookies(
    window: &tauri::WebviewWindow,
    default_domain: &str,
    domains: &[String],
) -> Vec<AuthCookie> {
    #[cfg(not(windows))]
    let _ = domains;

    #[cfg(windows)]
    {
        let native = extract_cookies_native(window, domains).await;
        if !native.is_empty() {
            return native;
        }
        tracing::debug!("[auth_webview] native extraction empty, falling back to JS");
    }

    extract_cookies_js(window, default_domain).await
}

async fn extract_cookies_js(
    window: &tauri::WebviewWindow,
    default_domain: &str,
) -> Vec<AuthCookie> {
    let js = r#"
(function() {
    try {
        var result = { cookies: document.cookie || '', storage: {} };
        try {
            for (var i = 0; i < localStorage.length; i++) {
                var key = localStorage.key(i);
                if (/token|auth|access|session|jwt|csrf/i.test(key)) {
                    result.storage[key] = localStorage.getItem(key);
                }
            }
        } catch(e) {}
        try {
            for (var i = 0; i < sessionStorage.length; i++) {
                var key = sessionStorage.key(i);
                if (/token|auth|access|session|jwt|csrf/i.test(key)) {
                    result.storage['ss:' + key] = sessionStorage.getItem(key);
                }
            }
        } catch(e) {}
        document.title = '__OMNIGET_COOKIES__' + JSON.stringify(result);
    } catch(err) {
        document.title = '__OMNIGET_COOKIES__{"cookies":"","storage":{}}';
    }
})()
"#;

    for attempt in 0..3 {
        let delay_ms: u64 = match attempt {
            0 => 500,
            1 => 1500,
            _ => 2500,
        };

        match window.eval(js) {
            Ok(()) => {}
            Err(e) => {
                tracing::error!("[auth_webview] eval() error: {}", e);
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                continue;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

        if let Ok(title) = window.title() {
            if let Some(data_str) = title.strip_prefix("__OMNIGET_COOKIES__") {
                return parse_cookie_data(data_str, default_domain);
            }
        }
    }

    Vec::new()
}

fn parse_cookie_data(data_str: &str, default_domain: &str) -> Vec<AuthCookie> {
    let mut cookies = Vec::new();

    if let Ok(data) = serde_json::from_str::<serde_json::Value>(data_str) {
        if let Some(cookie_str) = data["cookies"].as_str() {
            for part in cookie_str.split(';') {
                let part = part.trim();
                if let Some((name, value)) = part.split_once('=') {
                    cookies.push(AuthCookie {
                        name: name.trim().to_string(),
                        value: value.trim().to_string(),
                        domain: default_domain.to_string(),
                        path: "/".to_string(),
                        http_only: false,
                        secure: false,
                    });
                }
            }
        }

        if let Some(storage) = data["storage"].as_object() {
            for (key, value) in storage {
                if let Some(val) = value.as_str() {
                    if !val.is_empty() {
                        cookies.push(AuthCookie {
                            name: key.clone(),
                            value: val.to_string(),
                            domain: default_domain.to_string(),
                            path: "/".to_string(),
                            http_only: false,
                            secure: false,
                        });
                    }
                }
            }
        }
    } else {
        for part in data_str.split(';') {
            let part = part.trim();
            if let Some((name, value)) = part.split_once('=') {
                cookies.push(AuthCookie {
                    name: name.trim().to_string(),
                    value: value.trim().to_string(),
                    domain: default_domain.to_string(),
                    path: "/".to_string(),
                    http_only: false,
                    secure: false,
                });
            }
        }
    }

    cookies
}

#[cfg(windows)]
async fn extract_cookies_native(
    window: &tauri::WebviewWindow,
    domains: &[String],
) -> Vec<AuthCookie> {
    let uris: Vec<String> = if domains.is_empty() {
        vec!["".to_string()]
    } else {
        domains
            .iter()
            .map(|d| {
                if d.starts_with("http") {
                    d.clone()
                } else {
                    format!("https://{}", d.trim_start_matches('.'))
                }
            })
            .collect()
    };

    let mut all_cookies: Vec<AuthCookie> = Vec::new();

    for uri in &uris {
        match extract_cookies_for_uri(window, uri).await {
            Ok(batch) => {
                tracing::debug!(
                    "[auth_webview] native cookies from {}: {}",
                    uri,
                    batch.len()
                );
                for c in batch {
                    if !all_cookies
                        .iter()
                        .any(|existing| existing.name == c.name && existing.domain == c.domain)
                    {
                        all_cookies.push(c);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("[auth_webview] GetCookies({}) failed: {}", uri, e);
            }
        }
    }

    all_cookies
}

#[cfg(windows)]
async fn extract_cookies_for_uri(
    window: &tauri::WebviewWindow,
    uri: &str,
) -> Result<Vec<AuthCookie>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<AuthCookie>>();
    let uri_owned = uri.to_string();

    window
        .with_webview(move |platform_webview| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::*;
            use windows::core::{Interface, BOOL, HSTRING, PCWSTR, PWSTR};

            let core = match platform_webview.controller().CoreWebView2() {
                Ok(c) => c,
                Err(_) => {
                    let _ = tx.send(Vec::new());
                    return;
                }
            };
            let core2: ICoreWebView2_2 = match core.cast() {
                Ok(c) => c,
                Err(_) => {
                    let _ = tx.send(Vec::new());
                    return;
                }
            };
            let manager = match core2.CookieManager() {
                Ok(m) => m,
                Err(_) => {
                    let _ = tx.send(Vec::new());
                    return;
                }
            };

            let uri_hstring = HSTRING::from(uri_owned);

            let _ = manager.GetCookies(
                PCWSTR::from_raw(uri_hstring.as_ptr()),
                &webview2_com::GetCookiesCompletedHandler::create(Box::new(
                    move |hr, cookie_list| {
                        let mut cookies = Vec::new();
                        if hr.is_ok() {
                            if let Some(list) = cookie_list {
                                let mut count: u32 = 0;
                                let _ = list.Count(&mut count);
                                for i in 0..count {
                                    if let Ok(cookie) = list.GetValueAtIndex(i) {
                                        let mut name_pw = PWSTR::null();
                                        let mut value_pw = PWSTR::null();
                                        let mut domain_pw = PWSTR::null();
                                        let mut path_pw = PWSTR::null();
                                        let mut http_only_b = BOOL::default();
                                        let mut secure_b = BOOL::default();

                                        let _ = cookie.Name(&mut name_pw);
                                        let _ = cookie.Value(&mut value_pw);
                                        let _ = cookie.Domain(&mut domain_pw);
                                        let _ = cookie.Path(&mut path_pw);
                                        let _ = cookie.IsHttpOnly(&mut http_only_b);
                                        let _ = cookie.IsSecure(&mut secure_b);

                                        let name = name_pw.to_string().unwrap_or_default();
                                        let value = value_pw.to_string().unwrap_or_default();
                                        let domain = domain_pw.to_string().unwrap_or_default();
                                        let path = path_pw.to_string().unwrap_or_default();

                                        if !name.is_empty() && !value.is_empty() {
                                            cookies.push(AuthCookie {
                                                name,
                                                value,
                                                domain,
                                                path,
                                                http_only: http_only_b.as_bool(),
                                                secure: secure_b.as_bool(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx.send(cookies);
                        Ok(())
                    },
                )),
            );
        })
        .map_err(|e| format!("{}", e))?;

    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "GetCookies timed out".to_string())?
        .map_err(|_| "Cookie channel closed".to_string())
}
