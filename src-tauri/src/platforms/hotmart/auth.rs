use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

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

fn is_post_login_url(url: &str) -> bool {
    url.contains("consumer.hotmart.com")
        || url.contains("hotmart.com/buyer")
        || url.contains("club.hotmart.com")
}

fn build_session_from_cookies(
    email: &str,
    token: &str,
    cookies: &[chromiumoxide::cdp::browser_protocol::network::Cookie],
) -> anyhow::Result<HotmartSession> {
    let jar = Jar::default();
    let domains = [
        "https://hotmart.com",
        "https://api-sec-vlc.hotmart.com",
        "https://api-hub.cb.hotmart.com",
        "https://api-club-course-consumption-gateway-ga.cb.hotmart.com",
        "https://consumer.hotmart.com",
        "https://api-club-hot-club-api.cb.hotmart.com",
    ];
    for c in cookies {
        let cookie_str = format!("{}={}; Domain=.hotmart.com; Path=/", c.name, c.value);
        for domain in &domains {
            jar.add_cookie_str(&cookie_str, &domain.parse().unwrap());
        }
    }

    let mut default_headers = HeaderMap::new();
    default_headers.insert("Accept", HeaderValue::from_static("application/json, text/plain, */*"));
    default_headers.insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", token))?);
    default_headers.insert("Origin", HeaderValue::from_static("https://consumer.hotmart.com"));
    default_headers.insert("Referer", HeaderValue::from_static("https://consumer.hotmart.com"));
    default_headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    default_headers.insert("cache-control", HeaderValue::from_static("no-cache"));

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .cookie_provider(Arc::new(jar))
        .default_headers(default_headers)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    Ok(HotmartSession {
        token: token.to_string(),
        email: email.to_string(),
        client,
        cookies: cookies
            .iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect(),
    })
}

pub async fn authenticate(email: &str, password: &str) -> anyhow::Result<HotmartSession> {
    let temp_profile = std::env::temp_dir().join(format!(
        "omniget_chrome_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .with_head()
            .arg(format!("--user-data-dir={}", temp_profile.display()))
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-extensions")
            .build()
            .map_err(|e| anyhow!("Falha ao configurar browser: {}", e))?,
    )
    .await?;
    tokio::spawn(async move {
        while handler.next().await.is_some() {}
    });

    let page = browser.new_page("https://sso.hotmart.com/login").await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let url = page.url().await?.unwrap_or_default();

    if is_post_login_url(&url) {
    } else if url.contains("sso.hotmart.com") {
        page.evaluate(
            r#"
            const el = document.querySelector('#hotmart-cookie-policy');
            if (el && el.shadowRoot) {
                const btn = el.shadowRoot.querySelector('button.cookie-policy-accept-all');
                if (btn) btn.click();
            }
        "#,
        )
        .await
        .ok();
        tokio::time::sleep(Duration::from_secs(1)).await;

        let email_json = serde_json::to_string(email)?;
        let password_json = serde_json::to_string(password)?;
        page.evaluate(format!(
            r#"(function() {{
                var s = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                var u = document.querySelector('#username');
                var p = document.querySelector('#password');
                s.call(u, {});
                u.dispatchEvent(new Event('input', {{ bubbles: true }}));
                u.dispatchEvent(new Event('change', {{ bubbles: true }}));
                s.call(p, {});
                p.dispatchEvent(new Event('input', {{ bubbles: true }}));
                p.dispatchEvent(new Event('change', {{ bubbles: true }}));
            }})()"#,
            email_json, password_json,
        ))
        .await?;
        tokio::time::sleep(Duration::from_millis(300)).await;

        page.find_element("[name=submit]").await?.click().await?;

        let start = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let current = page.url().await?.unwrap_or_default();
            if is_post_login_url(&current) {
                break;
            }
            if current.contains("captcha") {
                let _ = tokio::fs::remove_dir_all(&temp_profile).await;
                return Err(anyhow!("Captcha detectado. Tente novamente."));
            }
            if start.elapsed() > Duration::from_secs(30) {
                let page_html = page
                    .evaluate("document.body ? document.body.innerText : ''")
                    .await
                    .ok()
                    .and_then(|v| v.into_value::<String>().ok())
                    .unwrap_or_default()
                    .to_lowercase();

                let _ = tokio::fs::remove_dir_all(&temp_profile).await;

                if page_html.contains("incorrect username or password")
                    || page_html.contains("senha incorreta")
                    || page_html.contains("credenciais inválidas")
                {
                    return Err(anyhow!("Credenciais inválidas."));
                }

                return Err(anyhow!(
                    "Timeout esperando login. Verifique credenciais."
                ));
            }
        }
    } else {
        page.goto("https://consumer.hotmart.com").await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    page.goto("https://consumer.hotmart.com").await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut cookies = page.get_cookies().await?;
    let mut token = cookies
        .iter()
        .find(|c| c.name == "hmVlcIntegration")
        .map(|c| c.value.clone());

    if token.is_none() {
        tokio::time::sleep(Duration::from_secs(4)).await;
        cookies = page.get_cookies().await?;
        token = cookies
            .iter()
            .find(|c| c.name == "hmVlcIntegration")
            .map(|c| c.value.clone());
    }

    let _ = tokio::fs::remove_dir_all(&temp_profile).await;

    let token = token.ok_or_else(|| anyhow!("Cookie hmVlcIntegration não encontrado após tentativas"))?;

    build_session_from_cookies(email, &token, &cookies)
}
