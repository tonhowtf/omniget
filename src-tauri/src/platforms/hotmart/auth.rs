use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

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
        "https://api-club-course-consumption-gateway.hotmart.com",
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
        .cookie_provider(Arc::new(jar))
        .default_headers(default_headers)
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
    tracing::info!("Sessão salva em {:?}", path);
    Ok(())
}

pub async fn load_saved_session() -> anyhow::Result<HotmartSession> {
    let path = session_file_path()?;
    let json = tokio::fs::read_to_string(&path).await?;
    let saved: SavedSession = serde_json::from_str(&json)?;

    tracing::info!(
        "Sessão restaurada do disco para {} (salva em {})",
        saved.email,
        saved.saved_at
    );

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
        tracing::info!("Sessão removida do disco: {:?}", path);
    }
    Ok(())
}

pub async fn authenticate(email: &str, password: &str) -> anyhow::Result<HotmartSession> {
    tracing::info!("Iniciando autenticação Hotmart para {}", email);

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .with_head()
            .build()
            .map_err(|e| anyhow!("Falha ao configurar browser: {}", e))?,
    )
    .await?;
    tokio::spawn(async move {
        while handler.next().await.is_some() {}
    });
    tracing::info!("Browser iniciado");

    let page = browser.new_page("https://sso.hotmart.com/login").await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    tracing::info!("Página carregada, verificando estado...");

    let url = page.url().await?.unwrap_or_default();

    let already_logged_in = url.contains("consumer.hotmart.com")
        || url.contains("dashboard")
        || url.contains("club.hotmart.com");

    if already_logged_in {
        tracing::info!("Sessão existente detectada, reutilizando cookies");
    } else if url.contains("sso.hotmart.com") {
        tracing::info!("Página de login detectada, preenchendo formulário");

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
        tracing::info!("Cookie banner tratado");

        page.find_element("#username")
            .await?
            .click()
            .await?
            .type_str(email)
            .await?;
        page.find_element("#password")
            .await?
            .click()
            .await?
            .type_str(password)
            .await?;
        tracing::info!("Credenciais preenchidas");

        page.find_element("[name=submit]").await?.click().await?;
        tracing::info!("Formulário enviado, aguardando redirect...");

        let start = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let current = page.url().await?.unwrap_or_default();
            if current.contains("consumer.hotmart.com") || current.contains("dashboard") {
                tracing::info!("Redirect detectado: {}", current);
                break;
            }
            if current.contains("captcha") {
                return Err(anyhow!("Captcha detectado. Tente novamente."));
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err(anyhow!(
                    "Timeout esperando login. Verifique credenciais."
                ));
            }
        }
    } else {
        tracing::info!("URL inesperada: {}, navegando para consumer.hotmart.com", url);
        page.goto("https://consumer.hotmart.com").await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    let cookies = page.get_cookies().await?;
    let token = cookies
        .iter()
        .find(|c| c.name == "hmVlcIntegration")
        .map(|c| c.value.clone())
        .ok_or_else(|| anyhow!("Cookie hmVlcIntegration não encontrado"))?;
    tracing::info!("Token extraído: {}...", &token[..20.min(token.len())]);

    let cookie_names: Vec<&str> = cookies.iter().map(|c| c.name.as_str()).collect();
    tracing::info!("Cookies extraídos do browser: {:?}", cookie_names);

    let jar = Jar::default();
    let domains = [
        "https://hotmart.com",
        "https://api-sec-vlc.hotmart.com",
        "https://api-hub.cb.hotmart.com",
        "https://api-club-course-consumption-gateway.hotmart.com",
        "https://consumer.hotmart.com",
        "https://api-club-hot-club-api.cb.hotmart.com",
    ];
    for c in &cookies {
        let cookie_str = format!("{}={}; Domain=.hotmart.com; Path=/", c.name, c.value);
        for domain in &domains {
            jar.add_cookie_str(&cookie_str, &domain.parse().unwrap());
        }
    }
    tracing::info!("Cookies adicionados ao jar para {} domínios", domains.len());

    let mut default_headers = HeaderMap::new();
    default_headers.insert("Accept", HeaderValue::from_static("application/json, text/plain, */*"));
    default_headers.insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", token))?);
    default_headers.insert("Origin", HeaderValue::from_static("https://consumer.hotmart.com"));
    default_headers.insert("Referer", HeaderValue::from_static("https://consumer.hotmart.com"));
    default_headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    default_headers.insert("cache-control", HeaderValue::from_static("no-cache"));

    let client = reqwest::Client::builder()
        .cookie_provider(Arc::new(jar))
        .default_headers(default_headers)
        .build()?;

    tracing::info!("Login Hotmart concluído com sucesso (client com headers globais)");

    Ok(HotmartSession {
        token,
        email: email.to_string(),
        client,
        cookies: cookies
            .iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect(),
    })
}
