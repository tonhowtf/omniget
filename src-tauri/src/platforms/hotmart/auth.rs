use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use reqwest::cookie::Jar;

pub struct HotmartSession {
    pub token: String,
    pub client: reqwest::Client,
    pub cookies: Vec<(String, String)>,
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
    tracing::info!("Página de login carregada");

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
        let url = page.url().await?.unwrap_or_default();
        if url.contains("consumer.hotmart.com") || url.contains("dashboard") {
            tracing::info!("Redirect detectado: {}", url);
            break;
        }
        if url.contains("captcha") {
            return Err(anyhow!("Captcha detectado. Tente novamente."));
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err(anyhow!(
                "Timeout esperando login. Verifique credenciais."
            ));
        }
    }

    let cookies = page.get_cookies().await?;
    let token = cookies
        .iter()
        .find(|c| c.name == "hmVlcIntegration")
        .map(|c| c.value.clone())
        .ok_or_else(|| anyhow!("Cookie hmVlcIntegration não encontrado"))?;
    tracing::info!("Token extraído: {}...", &token[..20.min(token.len())]);

    let jar = Jar::default();
    for c in &cookies {
        jar.add_cookie_str(
            &format!("{}={}", c.name, c.value),
            &"https://hotmart.com".parse().unwrap(),
        );
    }
    let client = reqwest::Client::builder()
        .cookie_provider(Arc::new(jar))
        .build()?;

    tracing::info!("Login Hotmart concluído com sucesso");

    Ok(HotmartSession {
        token,
        client,
        cookies: cookies
            .iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect(),
    })
}
