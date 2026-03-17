use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::platforms::medcel::api as medcel_api;

const ORIGIN_URL: &str = "https://alunos-internato.afya.com.br";
const API_BASE: &str = "https://service.medcel.com.br";

#[derive(Clone)]
pub struct AfyaSession {
    pub token: String,
    pub api_key: String,
    pub student_id: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub api_key: String,
    pub student_id: String,
    pub saved_at: u64,
}

pub type AfyaCourse = medcel_api::MedcelCourse;
pub type AfyaModule = medcel_api::MedcelModule;
pub type AfyaLesson = medcel_api::MedcelLesson;
pub type AfyaMaterial = medcel_api::MedcelMaterial;

fn session_file_path() -> anyhow::Result<PathBuf> {
    let data_dir =
        dirs::data_dir().ok_or_else(|| anyhow!("Could not find app data directory"))?;
    Ok(data_dir.join("omniget").join("afya_session.json"))
}

pub async fn authenticate(email: &str, password: &str, api_key: &str) -> anyhow::Result<AfyaSession> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent("Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0")
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "X-Api-Key",
        reqwest::header::HeaderValue::from_str(api_key)?,
    );
    headers.insert(
        "Referer",
        reqwest::header::HeaderValue::from_str(&format!("{}/", ORIGIN_URL))?,
    );
    headers.insert(
        "Origin",
        reqwest::header::HeaderValue::from_str(ORIGIN_URL)?,
    );

    let payload = serde_json::json!({
        "email": email,
        "password": password,
    });

    let resp = temp_client
        .post(format!("{}/m1/students/auth", API_BASE))
        .headers(headers)
        .json(&payload)
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

    let student_id = body
        .get("_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let session_token = body
        .get("sessionToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("sessionToken not found in auth response"))?
        .to_string();

    let client = medcel_api::build_client_with_origin(&session_token, api_key, ORIGIN_URL)?;

    Ok(AfyaSession {
        token: session_token,
        api_key: api_key.to_string(),
        student_id,
        client,
    })
}

pub fn to_medcel_session(session: &AfyaSession) -> medcel_api::MedcelSession {
    medcel_api::MedcelSession {
        token: session.token.clone(),
        api_key: session.api_key.clone(),
        student_id: session.student_id.clone(),
        client: session.client.clone(),
    }
}

pub async fn validate_token(session: &AfyaSession) -> anyhow::Result<bool> {
    let ms = to_medcel_session(session);
    medcel_api::validate_token(&ms).await
}

pub async fn fetch_student_id(session: &AfyaSession) -> anyhow::Result<String> {
    let ms = to_medcel_session(session);
    medcel_api::fetch_student_id(&ms).await
}

pub async fn list_courses(session: &AfyaSession) -> anyhow::Result<Vec<AfyaCourse>> {
    let ms = to_medcel_session(session);
    medcel_api::list_courses(&ms).await
}

pub async fn get_modules(session: &AfyaSession, product_id: &str) -> anyhow::Result<Vec<AfyaModule>> {
    let ms = to_medcel_session(session);
    medcel_api::get_modules(&ms, product_id).await
}

pub async fn get_video_url(session: &AfyaSession, class_id: &str, product_id: &str) -> anyhow::Result<String> {
    let ms = to_medcel_session(session);
    medcel_api::get_video_url(&ms, class_id, product_id).await
}

pub async fn save_session(session: &AfyaSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        token: session.token.clone(),
        api_key: session.api_key.clone(),
        student_id: session.student_id.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[afya] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<AfyaSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = medcel_api::build_client_with_origin(&saved.token, &saved.api_key, ORIGIN_URL)?;

    tracing::info!("[afya] session loaded");

    Ok(Some(AfyaSession {
        token: saved.token,
        api_key: saved.api_key,
        student_id: saved.student_id,
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
