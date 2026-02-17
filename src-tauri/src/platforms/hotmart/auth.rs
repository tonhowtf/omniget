use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Clone)]
pub struct HotmartSession {
    pub token: String,
    pub email: String,
    pub client: reqwest::Client,
    pub cookies: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub email: String,
    pub cookies: Vec<(String, String)>,
    pub saved_at: u64,
}

fn session_file_path() -> anyhow::Result<PathBuf> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| anyhow!("Não foi possível encontrar diretório de dados do app"))?;
    Ok(data_dir.join("omniget").join("hotmart_session.json"))
}

pub fn build_client_from_saved(saved: &SavedSession) -> anyhow::Result<reqwest::Client> {
    let jar = Jar::default();
    let domains = [
        "https://hotmart.com",
        "https://api-sec-vlc.hotmart.com",
        "https://api-hub.cb.hotmart.com",
        "https://api-club-course-consumption-gateway-ga.cb.hotmart.com",
        "https://consumer.hotmart.com",
        "https://api-club-hot-club-api.cb.hotmart.com",
    ];
    for (name, value) in &saved.cookies {
        let cookie_str = format!("{}={}; Domain=.hotmart.com; Path=/", name, value);
        for domain in &domains {
            jar.add_cookie_str(&cookie_str, &domain.parse().unwrap());
        }
    }

    let mut default_headers = HeaderMap::new();
    default_headers.insert(
        "Accept",
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    default_headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", saved.token))?,
    );
    default_headers.insert(
        "Origin",
        HeaderValue::from_static("https://consumer.hotmart.com"),
    );
    default_headers.insert(
        "Referer",
        HeaderValue::from_static("https://consumer.hotmart.com"),
    );
    default_headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    default_headers.insert("cache-control", HeaderValue::from_static("no-cache"));

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .cookie_provider(Arc::new(jar))
        .default_headers(default_headers)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    Ok(client)
}

pub async fn save_session(session: &HotmartSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        token: session.token.clone(),
        email: session.email.clone(),
        cookies: session.cookies.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

pub async fn load_saved_session() -> anyhow::Result<HotmartSession> {
    let path = session_file_path()?;
    let json = tokio::fs::read_to_string(&path).await?;
    let saved: SavedSession = serde_json::from_str(&json)?;

    let client = build_client_from_saved(&saved)?;

    Ok(HotmartSession {
        token: saved.token,
        email: saved.email,
        cookies: saved.cookies,
        client,
    })
}

pub async fn delete_saved_session() -> anyhow::Result<()> {
    let path = session_file_path()?;
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

#[cfg(windows)]
async fn extract_webview_cookies(
    window: &tauri::WebviewWindow,
) -> anyhow::Result<Vec<(String, String)>> {
    use webview2_com::GetCookiesCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use windows_core::{Interface, HSTRING, PCWSTR, PWSTR};

    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<(String, String)>>();

    window
        .with_webview(move |webview| {
            unsafe {
                let core = webview.controller().CoreWebView2().unwrap();
                let core2: ICoreWebView2_2 = core.cast().unwrap();
                let manager = core2.CookieManager().unwrap();
                let uri = HSTRING::from("https://hotmart.com");

                let _ = manager.GetCookies(
                    PCWSTR::from_raw(uri.as_ptr()),
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
        .map_err(|_| anyhow!("Timeout ao obter cookies do WebView."))?
        .map_err(|_| anyhow!("Canal de cookies fechado."))?;

    Ok(cookies)
}

pub async fn authenticate(
    app: &tauri::AppHandle,
    email: &str,
    password: &str,
) -> anyhow::Result<HotmartSession> {
    let post_login_detected = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let post_login_flag = post_login_detected.clone();

    let email_json = serde_json::to_string(email)?;
    let password_json = serde_json::to_string(password)?;

    let init_script = format!(
        r#"(function() {{
            if (window.location.hostname !== 'sso.hotmart.com') return;
            var filled = false;
            function dismissCookies() {{
                try {{
                    var el = document.querySelector('#hotmart-cookie-policy');
                    if (el && el.shadowRoot) {{
                        var btn = el.shadowRoot.querySelector('button.cookie-policy-accept-all');
                        if (btn) btn.click();
                    }}
                }} catch(e) {{}}
            }}
            function fillForm() {{
                if (filled) return;
                var u = document.querySelector('#username');
                var p = document.querySelector('#password');
                if (!u || !p) return;
                filled = true;
                var s = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                s.call(u, {email_json});
                u.dispatchEvent(new Event('input', {{ bubbles: true }}));
                u.dispatchEvent(new Event('change', {{ bubbles: true }}));
                s.call(p, {password_json});
                p.dispatchEvent(new Event('input', {{ bubbles: true }}));
                p.dispatchEvent(new Event('change', {{ bubbles: true }}));
                setTimeout(function() {{
                    var btn = document.querySelector('[name=submit]');
                    if (btn) btn.click();
                }}, 500);
            }}
            dismissCookies();
            setTimeout(dismissCookies, 1000);
            setTimeout(fillForm, 1500);
            setInterval(fillForm, 500);
        }})()"#
    );

    if let Some(existing) = app.get_webview_window("hotmart-login") {
        let _ = existing.close();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let login_window = tauri::WebviewWindowBuilder::new(
        app,
        "hotmart-login",
        tauri::WebviewUrl::External("https://sso.hotmart.com/login".parse().unwrap()),
    )
    .title("Hotmart Login")
    .inner_size(500.0, 700.0)
    .initialization_script(&init_script)
    .on_navigation(move |url| {
        let host = url.host_str().unwrap_or("");
        if host == "consumer.hotmart.com" {
            post_login_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        true
    })
    .build()
    .map_err(|e| anyhow!("Falha ao criar janela de login: {}", e))?;

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(120);

    loop {
        if start.elapsed() > timeout {
            let _ = login_window.close();
            return Err(anyhow!("Timeout esperando login. Verifique credenciais."));
        }

        if post_login_detected.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let cookies = extract_webview_cookies(&login_window).await?;
            let _ = login_window.close();

            let token = cookies
                .iter()
                .find(|(name, _)| name == "hmVlcIntegration")
                .map(|(_, value)| value.clone())
                .ok_or_else(|| anyhow!("Cookie hmVlcIntegration não encontrado."))?;

            let saved = SavedSession {
                token: token.clone(),
                email: email.to_string(),
                cookies: cookies.clone(),
                saved_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            };

            let client = build_client_from_saved(&saved)?;

            return Ok(HotmartSession {
                token,
                email: email.to_string(),
                client,
                cookies,
            });
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
