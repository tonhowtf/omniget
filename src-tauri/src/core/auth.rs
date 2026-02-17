use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub platform: String,
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    pub cookies: Vec<(String, String)>,
    pub user_info: HashMap<String, String>,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthType {
    OAuth,
    Credentials,
    BrowserCookies,
}

#[async_trait]
pub trait PlatformAuth: Send + Sync {
    fn name(&self) -> &str;
    fn auth_type(&self) -> AuthType;
    async fn is_authenticated(&self) -> bool;
    async fn authenticate(
        &self,
        params: HashMap<String, String>,
    ) -> anyhow::Result<AuthSession>;
    async fn logout(&self) -> anyhow::Result<()>;
    fn get_token(&self) -> Option<String>;
}

fn auth_session_path(platform: &str) -> anyhow::Result<PathBuf> {
    let data_dir =
        dirs::data_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    Ok(data_dir
        .join("omniget")
        .join("auth")
        .join(format!("{}.json", platform)))
}

pub async fn save_auth_session(session: &AuthSession) -> anyhow::Result<()> {
    let path = auth_session_path(&session.platform)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(session)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

pub async fn load_auth_session(platform: &str) -> anyhow::Result<AuthSession> {
    let path = auth_session_path(platform)?;
    let json = tokio::fs::read_to_string(&path).await?;
    let session: AuthSession = serde_json::from_str(&json)?;
    Ok(session)
}

pub async fn delete_auth_session(platform: &str) -> anyhow::Result<()> {
    let path = auth_session_path(platform)?;
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

pub struct SpotifyAuth {
    pub client_id: String,
    pub session: Arc<Mutex<Option<AuthSession>>>,
}

impl SpotifyAuth {
    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            session: Arc::new(Mutex::new(None)),
        }
    }

    fn generate_pkce() -> (String, String) {
        use rand::RngExt;
        use sha2::Digest;

        let mut rng = rand::rng();
        let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
        let verifier: String = (0..128)
            .map(|_| {
                let idx = rng.random_range(0..charset.len());
                charset[idx] as char
            })
            .collect();

        let hash = sha2::Sha256::digest(verifier.as_bytes());
        let challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        (verifier, challenge)
    }

    async fn start_callback_server() -> anyhow::Result<(tokio::net::TcpListener, u16)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        Ok((listener, port))
    }

    async fn wait_for_callback(
        listener: tokio::net::TcpListener,
    ) -> anyhow::Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            listener.accept(),
        )
        .await
        .map_err(|_| anyhow!("OAuth callback timeout"))?
        .map_err(|e| anyhow!("Failed to accept callback: {}", e))?;

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        let first_line = request.lines().next().unwrap_or("");
        let path = first_line.split_whitespace().nth(1).unwrap_or("/");

        let code = url::Url::parse(&format!("http://localhost{}", path))
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.to_string())
            })
            .ok_or_else(|| anyhow!("No authorization code in callback"))?;

        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html><body><h3>Authentication successful. You can close this tab.</h3></body></html>";
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;

        Ok(code)
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<AuthSession> {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://accounts.spotify.com/api/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", &self.client_id),
                ("code_verifier", verifier),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Token exchange failed: {}", body));
        }

        let json: serde_json::Value = resp.json().await?;
        let access_token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("No access_token in response"))?
            .to_string();
        let refresh_token = json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let expires_in = json
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut user_info = HashMap::new();

        let me_resp = client
            .get("https://api.spotify.com/v1/me")
            .bearer_auth(&access_token)
            .send()
            .await;

        if let Ok(me) = me_resp {
            if me.status().is_success() {
                if let Ok(me_json) = me.json::<serde_json::Value>().await {
                    if let Some(name) = me_json.get("display_name").and_then(|v| v.as_str()) {
                        user_info.insert("display_name".to_string(), name.to_string());
                    }
                    if let Some(email) = me_json.get("email").and_then(|v| v.as_str()) {
                        user_info.insert("email".to_string(), email.to_string());
                    }
                    if let Some(id) = me_json.get("id").and_then(|v| v.as_str()) {
                        user_info.insert("id".to_string(), id.to_string());
                    }
                }
            }
        }

        let session = AuthSession {
            platform: "spotify".to_string(),
            token: Some(access_token),
            refresh_token,
            cookies: Vec::new(),
            user_info,
            expires_at: Some(now + expires_in),
        };

        save_auth_session(&session).await?;
        Ok(session)
    }

    pub async fn refresh_token(&self) -> anyhow::Result<AuthSession> {
        let guard = self.session.lock().await;
        let current = guard
            .as_ref()
            .ok_or_else(|| anyhow!("No active session"))?
            .clone();
        drop(guard);

        let refresh = current
            .refresh_token
            .as_ref()
            .ok_or_else(|| anyhow!("No refresh token available"))?;

        let client = reqwest::Client::new();
        let resp = client
            .post("https://accounts.spotify.com/api/token")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", &self.client_id),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Token refresh failed: {}", body));
        }

        let json: serde_json::Value = resp.json().await?;
        let access_token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("No access_token in refresh response"))?
            .to_string();
        let new_refresh = json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(current.refresh_token.clone());
        let expires_in = json
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let session = AuthSession {
            platform: "spotify".to_string(),
            token: Some(access_token),
            refresh_token: new_refresh,
            cookies: Vec::new(),
            user_info: current.user_info,
            expires_at: Some(now + expires_in),
        };

        save_auth_session(&session).await?;
        let mut guard = self.session.lock().await;
        *guard = Some(session.clone());
        Ok(session)
    }
}

#[async_trait]
impl PlatformAuth for SpotifyAuth {
    fn name(&self) -> &str {
        "spotify"
    }

    fn auth_type(&self) -> AuthType {
        AuthType::OAuth
    }

    async fn is_authenticated(&self) -> bool {
        let guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            if session.token.is_some() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if let Some(expires_at) = session.expires_at {
                    return now < expires_at;
                }
                return true;
            }
        }
        drop(guard);

        if let Ok(saved) = load_auth_session("spotify").await {
            if saved.token.is_some() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if let Some(expires_at) = saved.expires_at {
                    if now < expires_at {
                        let mut guard = self.session.lock().await;
                        *guard = Some(saved);
                        return true;
                    }
                }
                if saved.refresh_token.is_some() {
                    let mut guard = self.session.lock().await;
                    *guard = Some(saved);
                    drop(guard);
                    return self.refresh_token().await.is_ok();
                }
            }
        }

        false
    }

    async fn authenticate(
        &self,
        _params: HashMap<String, String>,
    ) -> anyhow::Result<AuthSession> {
        let (verifier, challenge) = Self::generate_pkce();
        let (listener, port) = Self::start_callback_server().await?;
        let redirect_uri = format!("http://127.0.0.1:{}/callback", port);

        let scopes = "user-read-private user-read-email playlist-read-private user-library-read";

        let auth_url = format!(
            "https://accounts.spotify.com/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge_method=S256&code_challenge={}",
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(scopes),
            urlencoding::encode(&challenge),
        );

        open::that(&auth_url).map_err(|e| anyhow!("Failed to open browser: {}", e))?;

        let code = Self::wait_for_callback(listener).await?;
        let session = self.exchange_code(&code, &verifier, &redirect_uri).await?;

        let mut guard = self.session.lock().await;
        *guard = Some(session.clone());
        Ok(session)
    }

    async fn logout(&self) -> anyhow::Result<()> {
        let mut guard = self.session.lock().await;
        *guard = None;
        drop(guard);
        delete_auth_session("spotify").await?;
        Ok(())
    }

    fn get_token(&self) -> Option<String> {
        let guard = self.session.blocking_lock();
        guard.as_ref().and_then(|s| s.token.clone())
    }
}

pub struct BrowserCookieAuth {
    pub platform_name: String,
    pub login_url: String,
    pub success_url_contains: String,
    pub cookie_domain: String,
    pub session: Arc<Mutex<Option<AuthSession>>>,
}

impl BrowserCookieAuth {
    pub fn new(
        platform_name: &str,
        login_url: &str,
        success_url_contains: &str,
        cookie_domain: &str,
    ) -> Self {
        Self {
            platform_name: platform_name.to_string(),
            login_url: login_url.to_string(),
            success_url_contains: success_url_contains.to_string(),
            cookie_domain: cookie_domain.to_string(),
            session: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl PlatformAuth for BrowserCookieAuth {
    fn name(&self) -> &str {
        &self.platform_name
    }

    fn auth_type(&self) -> AuthType {
        AuthType::BrowserCookies
    }

    async fn is_authenticated(&self) -> bool {
        let guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            return !session.cookies.is_empty();
        }
        drop(guard);

        if let Ok(saved) = load_auth_session(&self.platform_name).await {
            if !saved.cookies.is_empty() {
                let mut guard = self.session.lock().await;
                *guard = Some(saved);
                return true;
            }
        }

        false
    }

    async fn authenticate(
        &self,
        _params: HashMap<String, String>,
    ) -> anyhow::Result<AuthSession> {
        use chromiumoxide::browser::{Browser, BrowserConfig};
        use futures::StreamExt;

        let (browser, mut handler) = Browser::launch(
            BrowserConfig::builder()
                .with_head()
                .build()
                .map_err(|e| anyhow!("Failed to configure browser: {}", e))?,
        )
        .await?;
        tokio::spawn(async move {
            while handler.next().await.is_some() {}
        });

        let page = browser.new_page(&self.login_url).await?;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let success_pattern = self.success_url_contains.clone();
        let start = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            let current_url = page.url().await?.unwrap_or_default();
            if current_url.contains(&success_pattern) {
                break;
            }
            if start.elapsed() > std::time::Duration::from_secs(300) {
                return Err(anyhow!("Authentication timeout"));
            }
        }

        let browser_cookies = page.get_cookies().await?;
        let cookies: Vec<(String, String)> = browser_cookies
            .iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect();

        let mut user_info = HashMap::new();
        let current_url = page.url().await?.unwrap_or_default();
        user_info.insert("login_url".to_string(), current_url);

        let session = AuthSession {
            platform: self.platform_name.clone(),
            token: None,
            refresh_token: None,
            cookies,
            user_info,
            expires_at: None,
        };

        save_auth_session(&session).await?;
        let mut guard = self.session.lock().await;
        *guard = Some(session.clone());
        Ok(session)
    }

    async fn logout(&self) -> anyhow::Result<()> {
        let mut guard = self.session.lock().await;
        *guard = None;
        drop(guard);
        delete_auth_session(&self.platform_name).await?;
        Ok(())
    }

    fn get_token(&self) -> Option<String> {
        let guard = self.session.blocking_lock();
        guard.as_ref().and_then(|s| s.token.clone())
    }
}

pub struct AuthRegistry {
    providers: Vec<Arc<dyn PlatformAuth>>,
}

impl AuthRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Arc<dyn PlatformAuth>) {
        self.providers.push(provider);
    }

    pub fn get(&self, platform: &str) -> Option<Arc<dyn PlatformAuth>> {
        self.providers
            .iter()
            .find(|p| p.name() == platform)
            .cloned()
    }

    pub fn list(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }
}
