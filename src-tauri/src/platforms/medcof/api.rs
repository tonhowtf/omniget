use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const API_BASE: &str = "https://lms-api.medcof.tech";

#[derive(Clone)]
pub struct MedcofSession {
    pub token: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedcofCourse {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedcofModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<MedcofLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedcofLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub vimeo_id: Option<String>,
    pub description: Option<String>,
}

fn build_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    headers.insert("Accept", HeaderValue::from_static("application/json"));

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
    Ok(data_dir.join("omniget").join("medcof_session.json"))
}

pub async fn validate_token(session: &MedcofSession) -> anyhow::Result<bool> {
    let url = format!("{}/auth/v1/me", API_BASE);
    let resp = session.client.get(&url).send().await?;
    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &MedcofSession) -> anyhow::Result<Vec<MedcofCourse>> {
    let url = format!("{}/auth/v1/me", API_BASE);
    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "list_courses returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let products = body
        .get("products")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut courses = Vec::new();

    for item in &products {
        let is_active = item
            .get("isActive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !is_active {
            continue;
        }

        let id = item
            .get("identifier")
            .or_else(|| item.get("id"))
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default();

        let name = item
            .get("name")
            .or_else(|| item.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !id.is_empty() {
            courses.push(MedcofCourse { id, name });
        }
    }

    Ok(courses)
}

pub async fn switch_product(session: &MedcofSession, product_id: &str) -> anyhow::Result<()> {
    let url = format!("{}/user/use/product", API_BASE);
    let payload = serde_json::json!({
        "productId": product_id,
    });

    let resp = session.client.patch(&url).json(&payload).send().await?;

    if !resp.status().is_success() {
        let body_text = resp.text().await?;
        return Err(anyhow!(
            "switch_product failed: {}",
            &body_text[..body_text.len().min(300)]
        ));
    }

    Ok(())
}

pub async fn get_course_content(session: &MedcofSession, product_id: &str) -> anyhow::Result<Vec<MedcofModule>> {
    switch_product(session, product_id).await?;

    let url = format!("{}/product/user/progress", API_BASE);
    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_course_content returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let blocks = body
        .get("blocks")
        .or_else(|| body.get("data"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            if body.is_array() {
                body.as_array().cloned().unwrap_or_default()
            } else {
                Vec::new()
            }
        });

    let mut modules = Vec::new();

    for (bi, block) in blocks.iter().enumerate() {
        let block_number = block
            .get("blockNumber")
            .or_else(|| block.get("number"))
            .or_else(|| block.get("id"))
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => format!("{}", bi + 1),
            })
            .unwrap_or_else(|| format!("{}", bi + 1));

        let block_name = block
            .get("name")
            .or_else(|| block.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or(&block_number)
            .to_string();

        let aqfm_url = format!("{}/aqfm/by-block-number/{}", API_BASE, block_number);
        let aqfm_resp = match session.client.get(&aqfm_url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("[medcof] Failed to fetch block {}: {}", block_number, e);
                continue;
            }
        };

        if !aqfm_resp.status().is_success() {
            continue;
        }

        let aqfm_text = aqfm_resp.text().await?;
        let aqfm_body: serde_json::Value = match serde_json::from_str(&aqfm_text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let specialties = aqfm_body
            .get("specialties")
            .or_else(|| aqfm_body.get("data"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| {
                if aqfm_body.is_array() {
                    aqfm_body.as_array().cloned().unwrap_or_default()
                } else {
                    Vec::new()
                }
            });

        let mut lessons = Vec::new();
        let mut lesson_order = 0i64;

        for specialty in &specialties {
            let aqfms = specialty
                .get("aqfms")
                .or_else(|| specialty.get("items"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for aqfm in &aqfms {
                let videos = aqfm
                    .get("videos")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                for video in &videos {
                    lesson_order += 1;

                    let vid_id = video
                        .get("id")
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => format!("{}", lesson_order),
                        })
                        .unwrap_or_else(|| format!("{}", lesson_order));

                    let vid_name = video
                        .get("name")
                        .or_else(|| video.get("title"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let vimeo_id = video
                        .get("vimeoId")
                        .or_else(|| video.get("externalId"))
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => String::new(),
                        })
                        .filter(|s| !s.is_empty());

                    let description = video
                        .get("description")
                        .or_else(|| video.get("content"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(String::from);

                    lessons.push(MedcofLesson {
                        id: vid_id,
                        name: vid_name,
                        order: lesson_order,
                        vimeo_id,
                        description,
                    });
                }
            }
        }

        if !lessons.is_empty() {
            modules.push(MedcofModule {
                id: block_number,
                name: block_name,
                order: (bi + 1) as i64,
                lessons,
            });
        }
    }

    Ok(modules)
}

pub async fn get_vimeo_playback_url(session: &MedcofSession, vimeo_id: &str) -> anyhow::Result<String> {
    let url = format!("{}/vimeo/get/{}", API_BASE, vimeo_id);
    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_vimeo_playback_url returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let playback_url = body
        .get("playbackUrl")
        .or_else(|| body.get("url"))
        .or_else(|| body.get("link"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("No playback URL in vimeo response"))?
        .to_string();

    Ok(playback_url)
}

pub async fn save_session(session: &MedcofSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        token: session.token.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[medcof] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<MedcofSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[medcof] session loaded");

    Ok(Some(MedcofSession {
        token: saved.token,
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
