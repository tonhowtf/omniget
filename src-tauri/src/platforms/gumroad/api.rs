use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "okhttp/4.8.1";
const CLIENT_ID: &str = "46410c2fb9aa741c1f03cdea099929c795d20de0282b352eac881dfa46b2b89c";
const CLIENT_SECRET: &str = "e2fa7dc5bc347d09820a3931d4ce10e1137a02577ce647b33c60670a72b1acd5";
const MOBILE_TOKEN: &str = "ps407sr3rno1561ro2o4n360q21248s4o24oq33770rpro59o11q9r5469ososoo";

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "mp3", "wav", "flac", "aac", "ogg", "m4a", "wma",
];

#[derive(Clone)]
pub struct GumroadSession {
    pub token: String,
    pub email: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub email: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GumroadProduct {
    pub id: String,
    pub name: String,
    pub creator_name: String,
    pub preview_url: Option<String>,
    pub url_redirect_token: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GumroadFile {
    pub id: String,
    pub name: String,
    pub filetype: String,
    pub filegroup: String,
    pub download_url: Option<String>,
    pub size: u64,
}

fn build_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );

    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()?;

    Ok(client)
}

fn session_file_path() -> anyhow::Result<PathBuf> {
    let data_dir =
        dirs::data_dir().ok_or_else(|| anyhow!("Could not find app data directory"))?;
    Ok(data_dir.join("omniget").join("gumroad_session.json"))
}

pub async fn authenticate(email: &str, password: &str) -> anyhow::Result<GumroadSession> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let params = [
        ("grant_type", "password"),
        ("username", email),
        ("password", password),
        ("scope", "mobile_api creator_api"),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
    ];

    let resp = temp_client
        .post("https://gumroad.com/oauth/token")
        .form(&params)
        .send()
        .await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "Authentication failed (status {}): {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("access_token not found in login response"))?
        .to_string();

    let client = build_client(&token)?;

    Ok(GumroadSession {
        token,
        email: email.to_string(),
        client,
    })
}

pub async fn validate_token(session: &GumroadSession) -> anyhow::Result<bool> {
    let url = format!(
        "https://api.gumroad.com/mobile/purchases/index.json?include_subscriptions=true&include_mobile_unfriendly_products=true&mobile_token={}",
        MOBILE_TOKEN
    );

    let resp = session.client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Ok(false);
    }

    let body: serde_json::Value = resp.json().await?;
    Ok(body.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
}

pub async fn list_products(session: &GumroadSession) -> anyhow::Result<Vec<GumroadProduct>> {
    let url = format!(
        "https://api.gumroad.com/mobile/purchases/index.json?include_subscriptions=true&include_mobile_unfriendly_products=true&mobile_token={}",
        MOBILE_TOKEN
    );

    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "list_products returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let success = body.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    if !success {
        return Err(anyhow!("API returned success=false"));
    }

    let products = body
        .get("products")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut result = Vec::new();

    for p in &products {
        if p.get("refunded").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        if p.get("partially_refunded").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        if p.get("chargedback").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        if p.get("is_archived").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }

        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed").to_string();
        let creator_name = p.get("creator_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let preview_url = p.get("preview_url").and_then(|v| v.as_str()).map(String::from);
        let url_redirect_token = p
            .get("url_redirect_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let file_count = p
            .get("file_data")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        result.push(GumroadProduct {
            id,
            name,
            creator_name,
            preview_url,
            url_redirect_token,
            file_count,
        });
    }

    Ok(result)
}

pub async fn get_product_files(
    _session: &GumroadSession,
    product_json: &serde_json::Value,
) -> Vec<GumroadFile> {
    let file_data = product_json
        .get("file_data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut files = Vec::new();

    for f in &file_data {
        let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("file").to_string();
        let filetype = f
            .get("filetype")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let filegroup = f
            .get("filegroup")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let download_url = f.get("download_url").and_then(|v| v.as_str()).map(String::from);
        let size = f.get("size").and_then(|v| v.as_u64()).unwrap_or(0);

        files.push(GumroadFile {
            id,
            name,
            filetype,
            filegroup,
            download_url,
            size,
        });
    }

    files
}

pub fn is_media_file(file: &GumroadFile) -> bool {
    if file.filegroup == "video" || file.filegroup == "audio" {
        return true;
    }
    MEDIA_EXTENSIONS.contains(&file.filetype.as_str())
}

pub async fn get_stream_url(
    session: &GumroadSession,
    url_redirect_token: &str,
    file_id: &str,
) -> anyhow::Result<String> {
    let url = format!(
        "https://api.gumroad.com/mobile/url_redirects/stream/{}/{}?mobile_token={}",
        url_redirect_token, file_id, MOBILE_TOKEN
    );

    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_stream_url returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let playlist_url = body
        .get("playlist_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("No playlist_url in stream response"))?
        .to_string();

    Ok(playlist_url)
}

pub fn get_direct_download_url(url_redirect_token: &str, file_id: &str) -> String {
    format!(
        "https://api.gumroad.com/mobile/url_redirects/download/{}/{}?mobile_token={}",
        url_redirect_token, file_id, MOBILE_TOKEN
    )
}

pub async fn save_session(session: &GumroadSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        token: session.token.clone(),
        email: session.email.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[gumroad] session saved for {}", session.email);
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<GumroadSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[gumroad] session loaded for {}", saved.email);

    Ok(Some(GumroadSession {
        token: saved.token,
        email: saved.email,
        client,
    }))
}

pub async fn delete_saved_session() -> anyhow::Result<()> {
    let path = session_file_path()?;
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}
