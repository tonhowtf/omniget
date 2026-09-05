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

enum AuthSignal {
    Navigated(String),
    CloseRequested,
}

const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
const COOKIE_TICK: std::time::Duration = std::time::Duration::from_secs(1);
const HINT_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

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

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AuthSignal>(8);

    let success_pattern = request.success_url_contains.clone();
    let tx_nav = tx.clone();
    let nav_login_host = login_host.clone();
    let nav_login_path = login_path.clone();

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

            if is_success_navigation(
                &url_str,
                success_pattern.as_deref(),
                &nav_login_host,
                &nav_login_path,
            ) {
                let _ = tx_nav.try_send(AuthSignal::Navigated(url_str));
            }

            true
        })
        .build()
        .map_err(|e| format!("Failed to create auth window: {}", e))?;

    let tx_close = tx.clone();
    drop(tx);

    webview_window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = tx_close.try_send(AuthSignal::CloseRequested);
        }
    });

    let outcome =
        wait_for_login(&webview_window, &request, &mut rx, &login_host, &login_path).await;

    let _ = webview_window.destroy();

    let (cookies, final_url) = outcome?;

    tracing::info!("[auth_webview] extracted {} cookies", cookies.len());
    for c in &cookies {
        tracing::debug!(
            "[auth_webview]   {} (httpOnly={}, domain={})",
            c.name,
            c.http_only,
            c.domain
        );
    }

    Ok(AuthWebviewResult { cookies, final_url })
}

async fn wait_for_login(
    window: &tauri::WebviewWindow,
    request: &AuthWebviewRequest,
    rx: &mut tokio::sync::mpsc::Receiver<AuthSignal>,
    login_host: &str,
    login_path: &str,
) -> Result<(Vec<AuthCookie>, String), String> {
    let default_domain = request.cookie_domains.first().cloned().unwrap_or_default();
    let domains = &request.cookie_domains;
    let targets = request
        .wait_for_cookie
        .as_deref()
        .map(parse_cookie_targets)
        .unwrap_or_default();

    let timeout = tokio::time::sleep(AUTH_TIMEOUT);
    tokio::pin!(timeout);
    let mut ticker = tokio::time::interval(COOKIE_TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut hint: Option<(String, std::time::Instant)> = None;
    let started = std::time::Instant::now();

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(AuthSignal::Navigated(url)) => {
                    if targets.is_empty() {
                        tracing::info!("[auth_webview] signal: {}", url);
                        let cookies = extract_after_signal(window, &default_domain, domains).await;
                        return Ok((cookies, url));
                    }
                    let cookies = extract_cookies(window, &default_domain, domains).await;
                    if has_any_cookie(&cookies, &targets) {
                        tracing::info!(
                            "[auth_webview] signal: {} with session cookie after {:.1}s",
                            url,
                            started.elapsed().as_secs_f64()
                        );
                        return Ok((cookies, url));
                    }
                    if hint.is_none() {
                        tracing::info!(
                            "[auth_webview] url hint {} without {:?} yet, polling the cookie store",
                            url,
                            targets
                        );
                        hint = Some((url, std::time::Instant::now()));
                    }
                }
                Some(AuthSignal::CloseRequested) => {
                    tracing::info!("[auth_webview] signal: user closed window");
                    let cookies = extract_cookies(window, &default_domain, domains).await;
                    let final_url = window.url().map(|u| u.to_string()).unwrap_or_default();
                    if !targets.is_empty() && !has_any_cookie(&cookies, &targets) {
                        return Err(format!(
                            "Login window was closed before a session was detected (no {} cookie)",
                            targets.join("/")
                        ));
                    }
                    if cookies.is_empty() {
                        return Err("Auth cancelled".to_string());
                    }
                    return Ok((cookies, final_url));
                }
                None => return Err("Auth cancelled".to_string()),
            },
            _ = ticker.tick(), if !targets.is_empty() => {
                let current = window.url().map(|u| u.to_string()).unwrap_or_default();
                let past_login = !current.is_empty() && !is_login_page(&current, login_host, login_path);
                let mut cookies = extract_cookies_native_or_empty(window, domains).await;
                // Sites that finish login with an OIDC code exchange keep the
                // session in web storage, not in a cookie (Hotmart's consumer
                // app stores `oidc.user:…` in localStorage). Once the page has
                // left the login flow, look there too.
                if past_login && !has_any_cookie(&cookies, &targets) && wants_storage(&targets) {
                    let js = extract_cookies_js(window, &default_domain).await;
                    merge_storage(&mut cookies, js);
                }
                if past_login && has_any_cookie(&cookies, &targets) {
                    tracing::info!(
                        "[auth_webview] session cookie present at {} after {:.1}s",
                        current,
                        started.elapsed().as_secs_f64()
                    );
                    return Ok((cookies, current));
                }
                if let Some((ref url, at)) = hint {
                    if at.elapsed() > HINT_GRACE {
                        tracing::warn!(
                            "[auth_webview] {:?} never appeared within {:.0}s of {}, returning what the store has",
                            targets,
                            HINT_GRACE.as_secs_f64(),
                            url
                        );
                        let cookies = extract_cookies(window, &default_domain, domains).await;
                        return Ok((cookies, url.clone()));
                    }
                }
            }
            _ = &mut timeout => {
                tracing::warn!("[auth_webview] timed out after {} minutes", AUTH_TIMEOUT.as_secs() / 60);
                return Err("Auth timed out".to_string());
            }
        }
    }
}

async fn extract_after_signal(
    window: &tauri::WebviewWindow,
    default_domain: &str,
    domains: &[String],
) -> Vec<AuthCookie> {
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let cookies = extract_cookies(window, default_domain, domains).await;
    if cookies.is_empty() {
        tracing::warn!("[auth_webview] no cookies on first attempt, retrying in 3s...");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        extract_cookies(window, default_domain, domains).await
    } else {
        cookies
    }
}

fn parse_cookie_targets(spec: &str) -> Vec<String> {
    spec.split(['|', ','])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A target is a cookie (or web-storage key) name, compared case-insensitively.
/// A trailing `*` matches a prefix, so `oidc.user:*` finds the
/// `oidc.user:https://sso.hotmart.com/oidc:<client-id>` entry without the
/// plugin having to hard-code the client id.
fn target_matches(target: &str, name: &str) -> bool {
    match target.strip_suffix('*') {
        Some(prefix) => !prefix.is_empty() && name.starts_with(prefix),
        None => target == name,
    }
}

fn has_any_cookie(cookies: &[AuthCookie], targets: &[String]) -> bool {
    cookies.iter().any(|c| {
        let name = c.name.to_lowercase();
        targets.iter().any(|t| target_matches(t, &name))
    })
}

/// Storage keys carry a dot, colon or wildcard; plain cookie names do not.
/// Used to skip the JS round-trip when every target is an ordinary cookie.
fn wants_storage(targets: &[String]) -> bool {
    targets
        .iter()
        .any(|t| t.ends_with('*') || t.contains(':') || t.contains('.'))
}

/// Native cookies stay authoritative; the JS pass only contributes the
/// web-storage entries (and, when nothing native came back, document.cookie).
fn merge_storage(into: &mut Vec<AuthCookie>, js: JsExtraction) {
    if into.is_empty() {
        into.extend(js.cookies);
    }
    for entry in js.storage {
        if !into.iter().any(|c| c.name == entry.name) {
            into.push(entry);
        }
    }
}

struct JsExtraction {
    cookies: Vec<AuthCookie>,
    storage: Vec<AuthCookie>,
}

fn is_login_like_path(path: &str) -> bool {
    const MARKERS: &[&str] = &[
        "login",
        "log-in",
        "signin",
        "sign-in",
        "sign_in",
        "auth",
        "signup",
        "sign-up",
        "sign_up",
        "register",
        "join",
        "password",
        "mfa",
        "two-factor",
        "2fa",
        "otp",
        "verify",
        "challenge",
        "captcha",
        "consent",
        "sso",
        "logout",
    ];
    let lower = path.to_ascii_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

fn is_login_page(url_str: &str, login_host: &str, login_path: &str) -> bool {
    let Ok(url) = url::Url::parse(url_str) else {
        return true;
    };
    let host = url.host_str().unwrap_or("");
    if host != login_host {
        return false;
    }
    url.path() == login_path || is_login_like_path(url.path())
}

fn host_matches(nav_host: &str, pattern_host: &str) -> bool {
    !pattern_host.is_empty()
        && (nav_host == pattern_host || nav_host.ends_with(&format!(".{pattern_host}")))
}

/// An explicit pattern from the plugin config is `host` or `host/path-prefix`.
/// The host must match exactly or as a subdomain, never as a substring: ad
/// trackers like DoubleClick embed the SSO redirect target in their own path
/// (e.g. /activityi;src=…;~oref=https://consumer.hotmart.com/…).
///
/// A path prefix also accepts the site root, because that is where most sites
/// drop the user after login (Udemy lands on `/`, not the `/home` the older
/// plugin config asked for — #187). On the login host itself, the login page
/// and anything that still looks like the login flow (`/join/…`, `/auth/…`)
/// never counts, so a host-only pattern for the same site cannot fire on the
/// very first navigation.
fn is_success_navigation(
    url_str: &str,
    pattern: Option<&str>,
    login_host: &str,
    login_path: &str,
) -> bool {
    let Ok(nav_url) = url::Url::parse(url_str) else {
        return false;
    };
    let nav_host = nav_url.host_str().unwrap_or("");
    let nav_path = nav_url.path();

    match pattern {
        Some(pattern) => {
            let (pattern_host, pattern_path) = match pattern.split_once('/') {
                Some((h, p)) => (h, Some(p.trim_end_matches('/'))),
                None => (pattern, None),
            };
            if !host_matches(nav_host, pattern_host) {
                return false;
            }
            let is_root = nav_path.trim_matches('/').is_empty();
            let path_ok = match pattern_path {
                Some(p) if !p.is_empty() => {
                    is_root || nav_path.trim_start_matches('/').starts_with(p)
                }
                _ => true,
            };
            if !path_ok {
                return false;
            }
            !is_login_page(url_str, login_host, login_path)
        }
        None => {
            if login_host.is_empty() || !host_matches(nav_host, login_host) {
                return false;
            }
            nav_path != login_path && !is_login_like_path(nav_path)
        }
    }
}

async fn extract_cookies(
    window: &tauri::WebviewWindow,
    default_domain: &str,
    domains: &[String],
) -> Vec<AuthCookie> {
    let mut cookies = extract_cookies_native_or_empty(window, domains).await;
    if cookies.is_empty() {
        tracing::debug!("[auth_webview] native extraction empty, falling back to JS");
    }
    let js = extract_cookies_js(window, default_domain).await;
    merge_storage(&mut cookies, js);
    cookies
}

async fn extract_cookies_native_or_empty(
    window: &tauri::WebviewWindow,
    domains: &[String],
) -> Vec<AuthCookie> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        extract_cookies_native(window, domains).await
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (window, domains);
        Vec::new()
    }
}

async fn extract_cookies_js(
    window: &tauri::WebviewWindow,
    default_domain: &str,
) -> JsExtraction {
    let js = r#"
(function() {
    try {
        var result = { cookies: document.cookie || '', storage: {} };
        var wanted = /token|auth|access|session|jwt|csrf|oidc|cas-js/i;
        try {
            for (var i = 0; i < localStorage.length; i++) {
                var key = localStorage.key(i);
                if (wanted.test(key)) {
                    result.storage[key] = localStorage.getItem(key);
                }
            }
        } catch(e) {}
        try {
            for (var i = 0; i < sessionStorage.length; i++) {
                var key = sessionStorage.key(i);
                if (wanted.test(key)) {
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

    JsExtraction {
        cookies: Vec::new(),
        storage: Vec::new(),
    }
}

fn parse_cookie_data(data_str: &str, default_domain: &str) -> JsExtraction {
    let mut cookies = Vec::new();
    let mut storage = Vec::new();

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

        if let Some(entries) = data["storage"].as_object() {
            for (key, value) in entries {
                if let Some(val) = value.as_str() {
                    if !val.is_empty() {
                        storage.push(AuthCookie {
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

    JsExtraction { cookies, storage }
}

#[cfg(any(windows, target_os = "linux"))]
fn cookie_query_uris(window: &tauri::WebviewWindow, domains: &[String]) -> Vec<String> {
    let normalized = normalize_cookie_domains(domains);
    if normalized.is_empty() {
        return window
            .url()
            .map(|u| vec![u.to_string()])
            .unwrap_or_default();
    }
    normalized.iter().map(|d| format!("https://{d}/")).collect()
}

#[cfg(any(windows, target_os = "linux"))]
fn merge_cookie_batch(all: &mut Vec<AuthCookie>, batch: Vec<AuthCookie>) {
    for c in batch {
        if !all
            .iter()
            .any(|existing| existing.name == c.name && existing.domain == c.domain)
        {
            all.push(c);
        }
    }
}

#[cfg(windows)]
async fn extract_cookies_native(
    window: &tauri::WebviewWindow,
    domains: &[String],
) -> Vec<AuthCookie> {
    let mut all_cookies: Vec<AuthCookie> = Vec::new();

    for uri in &cookie_query_uris(window, domains) {
        match extract_cookies_for_uri(window, uri).await {
            Ok(batch) => {
                tracing::debug!(
                    "[auth_webview] native cookies from {}: {}",
                    uri,
                    batch.len()
                );
                merge_cookie_batch(&mut all_cookies, batch);
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

/// Reads the cookies out of WKWebView's own cookie store.
///
/// `document.cookie` cannot see HttpOnly cookies by definition, so the JS
/// fallback silently returns an authenticated session minus exactly the
/// cookies that prove it is authenticated. Hotmart's `hmVlcIntegration` is
/// one of those, which is why a visibly logged-in webview still reported
/// `not_authenticated` on macOS (#287) while Windows — which has had native
/// extraction all along — worked.
#[cfg(target_os = "macos")]
async fn extract_cookies_native(
    window: &tauri::WebviewWindow,
    domains: &[String],
) -> Vec<AuthCookie> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<AuthCookie>>();
    let wanted = normalize_cookie_domains(domains);

    let result = window.with_webview(move |platform_webview| {
        use block2::RcBlock;
        use objc2::rc::Retained;
        use objc2_foundation::{NSArray, NSHTTPCookie};
        use objc2_web_kit::WKWebView;

        // with_webview hands us the WKWebView on the main thread, which is the
        // only thread WebKit permits here.
        let webview: &WKWebView = unsafe { &*(platform_webview.inner() as *mut WKWebView) };
        let store = unsafe { webview.configuration().websiteDataStore().httpCookieStore() };

        let tx = std::cell::RefCell::new(Some(tx));
        let handler = RcBlock::new(move |cookies: std::ptr::NonNull<NSArray<NSHTTPCookie>>| {
            let cookies: Retained<NSArray<NSHTTPCookie>> =
                unsafe { Retained::retain(cookies.as_ptr()) }.expect("cookie array");
            let mut out = Vec::new();
            for cookie in cookies.iter() {
                let domain = cookie.domain().to_string();
                if !cookie_domain_matches(&domain, &wanted) {
                    continue;
                }
                out.push(AuthCookie {
                    name: cookie.name().to_string(),
                    value: cookie.value().to_string(),
                    domain,
                    path: cookie.path().to_string(),
                    http_only: cookie.isHTTPOnly(),
                    secure: cookie.isSecure(),
                });
            }
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(out);
            }
        });

        unsafe { store.getAllCookies(&handler) };
    });

    if let Err(e) = result {
        tracing::warn!("[auth_webview] with_webview failed: {}", e);
        return Vec::new();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(cookies)) => {
            tracing::debug!(
                "[auth_webview] native cookies from WKWebView: {}",
                cookies.len()
            );
            cookies
        }
        Ok(Err(_)) => {
            tracing::warn!("[auth_webview] cookie channel closed");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("[auth_webview] getAllCookies timed out");
            Vec::new()
        }
    }
}

/// Reads the cookies out of WebKitGTK's own cookie store.
///
/// Same reason as the macOS path: Udemy's `access_token` and `dj_session_id`
/// are HttpOnly, so `document.cookie` on Linux handed the plugin a session
/// that could never validate (#288). `webkit_cookie_manager_get_cookies` is
/// per-URI and returns what a request to that URI would carry, which is
/// exactly the domain-scoped set the plugin needs.
#[cfg(target_os = "linux")]
async fn extract_cookies_native(
    window: &tauri::WebviewWindow,
    domains: &[String],
) -> Vec<AuthCookie> {
    let wanted = normalize_cookie_domains(domains);
    let mut all_cookies: Vec<AuthCookie> = Vec::new();

    for uri in &cookie_query_uris(window, domains) {
        match extract_cookies_for_uri(window, uri, &wanted).await {
            Ok(batch) => {
                tracing::debug!(
                    "[auth_webview] native cookies from {}: {}",
                    uri,
                    batch.len()
                );
                merge_cookie_batch(&mut all_cookies, batch);
            }
            Err(e) => {
                tracing::warn!("[auth_webview] get_cookies({}) failed: {}", uri, e);
            }
        }
    }

    all_cookies
}

#[cfg(target_os = "linux")]
async fn extract_cookies_for_uri(
    window: &tauri::WebviewWindow,
    uri: &str,
    wanted: &[String],
) -> Result<Vec<AuthCookie>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<Vec<AuthCookie>, String>>();
    let uri_owned = uri.to_string();
    let wanted = wanted.to_vec();

    window
        .with_webview(move |platform_webview| {
            use webkit2gtk::{CookieManagerExt, WebContextExt, WebViewExt, WebsiteDataManagerExt};

            let webview = platform_webview.inner();
            let manager = WebViewExt::website_data_manager(&webview)
                .and_then(|m| WebsiteDataManagerExt::cookie_manager(&m))
                .or_else(|| {
                    WebViewExt::web_context(&webview)
                        .and_then(|c| WebContextExt::cookie_manager(&c))
                });
            let Some(manager) = manager else {
                let _ = tx.send(Err("webview has no cookie manager".to_string()));
                return;
            };

            manager.cookies(
                &uri_owned,
                None::<&webkit2gtk::gio::Cancellable>,
                move |result| {
                    let out = match result {
                        Ok(list) => Ok(list
                            .into_iter()
                            .filter_map(|mut cookie| {
                                let name = cookie.name()?.to_string();
                                let value = cookie.value()?.to_string();
                                if name.is_empty() || value.is_empty() {
                                    return None;
                                }
                                let domain =
                                    cookie.domain().map(|d| d.to_string()).unwrap_or_default();
                                if !cookie_domain_matches(&domain, &wanted) {
                                    return None;
                                }
                                Some(AuthCookie {
                                    name,
                                    value,
                                    domain,
                                    path: cookie
                                        .path()
                                        .map(|p| p.to_string())
                                        .unwrap_or_else(|| "/".to_string()),
                                    http_only: cookie.is_http_only(),
                                    secure: cookie.is_secure(),
                                })
                            })
                            .collect()),
                        Err(e) => Err(e.to_string()),
                    };
                    let _ = tx.send(out);
                },
            );
        })
        .map_err(|e| format!("{}", e))?;

    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "get_cookies timed out".to_string())?
        .map_err(|_| "Cookie channel closed".to_string())?
}

/// Strips scheme and leading dot so a configured `.hotmart.com` and a cookie's
/// `hotmart.com` compare equal.
#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
fn normalize_cookie_domains(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .map(|d| {
            d.trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches('.')
                .to_lowercase()
        })
        .filter(|d| !d.is_empty())
        .collect()
}

/// Whether a cookie's domain belongs to one of the requested domains.
///
/// An empty list means "everything this window has" — the auth webview is
/// opened for one login, so there is nothing else in its store to leak.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn cookie_domain_matches(cookie_domain: &str, wanted: &[String]) -> bool {
    if wanted.is_empty() {
        return true;
    }
    let normalized = cookie_domain.trim_start_matches('.').to_lowercase();
    wanted
        .iter()
        .any(|w| normalized == *w || normalized.ends_with(&format!(".{w}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str) -> AuthCookie {
        AuthCookie {
            name: name.into(),
            value: "x".into(),
            domain: ".udemy.com".into(),
            path: "/".into(),
            http_only: true,
            secure: true,
        }
    }

    #[test]
    fn normalizes_scheme_and_leading_dot() {
        let got = normalize_cookie_domains(&[
            "https://consumer.hotmart.com".into(),
            ".hotmart.com".into(),
            "HOTMART.com".into(),
            "".into(),
        ]);
        assert_eq!(
            got,
            vec!["consumer.hotmart.com", "hotmart.com", "hotmart.com"]
        );
    }

    #[test]
    fn matches_exact_and_subdomains() {
        let wanted = normalize_cookie_domains(&[".hotmart.com".into()]);
        assert!(cookie_domain_matches("hotmart.com", &wanted));
        assert!(cookie_domain_matches(".hotmart.com", &wanted));
        assert!(cookie_domain_matches("consumer.hotmart.com", &wanted));
    }

    #[test]
    fn rejects_lookalike_domains() {
        // The substring check this replaced accepted both of these.
        let wanted = normalize_cookie_domains(&["hotmart.com".into()]);
        assert!(!cookie_domain_matches("nothotmart.com", &wanted));
        assert!(!cookie_domain_matches("hotmart.com.evil.net", &wanted));
    }

    #[test]
    fn empty_wanted_keeps_everything() {
        assert!(cookie_domain_matches("anything.example", &[]));
    }

    const UDEMY_LOGIN: (&str, &str) = ("www.udemy.com", "/join/login-popup/");

    #[test]
    fn udemy_real_post_login_urls_succeed_with_host_only_pattern() {
        let (h, p) = UDEMY_LOGIN;
        for url in [
            "https://www.udemy.com/",
            "https://www.udemy.com/?persist_locale=&locale=en_US",
            "https://www.udemy.com/home/my-courses/",
            "https://www.udemy.com/home/my-courses/learning/",
            "https://acme.udemy.com/",
            "https://acme.udemy.com/organization/home/",
            "https://udemy.com/",
        ] {
            assert!(
                is_success_navigation(url, Some("udemy.com"), h, p),
                "{url} should be success"
            );
        }
    }

    #[test]
    fn udemy_login_flow_pages_never_succeed() {
        let (h, p) = UDEMY_LOGIN;
        for url in [
            "https://www.udemy.com/join/login-popup/",
            "https://www.udemy.com/join/login-popup/?next=https%3A%2F%2Fwww.udemy.com%2F",
            "https://www.udemy.com/join/passwordless-auth/",
            "https://www.udemy.com/join/signup-popup/",
            "https://accounts.google.com/o/oauth2/auth?client_id=1",
            "https://www.udemy.com/user/edit-account/password/",
            "https://notudemy.com/",
            "https://udemy.com.evil.net/",
        ] {
            assert!(
                !is_success_navigation(url, Some("udemy.com"), h, p),
                "{url} must not be success"
            );
        }
    }

    #[test]
    fn old_plugin_path_pattern_still_accepts_site_root() {
        let (h, p) = UDEMY_LOGIN;
        assert!(is_success_navigation(
            "https://www.udemy.com/",
            Some("udemy.com/home"),
            h,
            p
        ));
        assert!(is_success_navigation(
            "https://www.udemy.com/home/my-courses/",
            Some("udemy.com/home"),
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://www.udemy.com/course/rust/",
            Some("udemy.com/home"),
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://www.udemy.com/join/login-popup/",
            Some("udemy.com/home"),
            h,
            p
        ));
    }

    #[test]
    fn hotmart_pattern_matches_consumer_host_on_any_path() {
        let h = "sso.hotmart.com";
        let p = "/login";
        assert!(is_success_navigation(
            "https://consumer.hotmart.com/auth/callback?code=1",
            Some("consumer.hotmart.com"),
            h,
            p
        ));
        assert!(is_success_navigation(
            "https://consumer.hotmart.com/",
            Some("consumer.hotmart.com"),
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://sso.hotmart.com/login?redirect=x",
            Some("consumer.hotmart.com"),
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://ad.doubleclick.net/activityi;src=1;~oref=https://consumer.hotmart.com/",
            Some("consumer.hotmart.com"),
            h,
            p
        ));
    }

    #[test]
    fn heuristic_without_pattern_needs_same_host_and_non_login_path() {
        let h = "soundcloud.com";
        let p = "/signin";
        assert!(is_success_navigation(
            "https://soundcloud.com/discover",
            None,
            h,
            p
        ));
        assert!(is_success_navigation(
            "https://api.soundcloud.com/me",
            None,
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://soundcloud.com/signin",
            None,
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://soundcloud.com/oauth/authorize",
            None,
            h,
            p
        ));
        assert!(!is_success_navigation(
            "https://secure.example.com/",
            None,
            h,
            p
        ));
    }

    #[test]
    fn login_page_check_only_applies_to_login_host() {
        let (h, p) = UDEMY_LOGIN;
        assert!(is_login_page(
            "https://www.udemy.com/join/login-popup/",
            h,
            p
        ));
        assert!(is_login_page(
            "https://www.udemy.com/join/passwordless-auth/",
            h,
            p
        ));
        assert!(!is_login_page("https://www.udemy.com/", h, p));
        assert!(!is_login_page("https://acme.udemy.com/join/login/", h, p));
        assert!(is_login_page("not a url", h, p));
    }

    #[test]
    fn cookie_targets_accept_alternatives_case_insensitively() {
        let targets = parse_cookie_targets("access_token|dj_session_id, udemy_session");
        assert_eq!(
            targets,
            vec!["access_token", "dj_session_id", "udemy_session"]
        );
        assert!(has_any_cookie(
            &[cookie("csrftoken"), cookie("Access_Token")],
            &targets
        ));
        assert!(!has_any_cookie(
            &[cookie("csrftoken"), cookie("ud_user_jwt")],
            &targets
        ));
        assert!(parse_cookie_targets(" ").is_empty());
    }

    #[test]
    fn wildcard_target_matches_storage_key_prefix() {
        let targets = parse_cookie_targets("hmVlcIntegration|oidc.user:*");
        assert!(has_any_cookie(
            &[cookie("oidc.user:https://sso.hotmart.com/oidc:0fff6c2a")],
            &targets
        ));
        assert!(has_any_cookie(&[cookie("HMVLCINTEGRATION")], &targets));
        assert!(!has_any_cookie(&[cookie("oidc.abc123")], &targets));
        assert!(!target_matches("*", "anything"));
        assert!(wants_storage(&targets));
        assert!(!wants_storage(&parse_cookie_targets("access_token|dj_session_id")));
    }

    #[test]
    fn storage_entries_merge_without_replacing_native_cookies() {
        let mut native = vec![cookie("access_token")];
        merge_storage(
            &mut native,
            JsExtraction {
                cookies: vec![cookie("csrftoken")],
                storage: vec![cookie("oidc.user:x"), cookie("access_token")],
            },
        );
        assert_eq!(
            native.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["access_token", "oidc.user:x"]
        );

        let mut empty = Vec::new();
        merge_storage(
            &mut empty,
            JsExtraction {
                cookies: vec![cookie("csrftoken")],
                storage: vec![cookie("oidc.user:x")],
            },
        );
        assert_eq!(empty.len(), 2);
    }
}
