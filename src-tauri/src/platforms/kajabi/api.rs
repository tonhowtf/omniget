use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "KajabiMobileApp";
const BASE_URL: &str = "https://mobile-api.kajabi.com/api/mobile/v3";
const AUTH_BASE: &str = "https://app.kajabi.com/api/mobile/v2";

#[derive(Clone)]
pub struct KajabiSession {
    pub token: String,
    pub site_id: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub site_id: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KajabiSite {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KajabiCourse {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub site_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KajabiModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<KajabiLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KajabiLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KajabiLessonDetail {
    pub id: String,
    pub name: String,
    pub video_url: Option<String>,
    pub files: Vec<KajabiFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KajabiFile {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
}

fn build_client(token: &str, site_id: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    headers.insert(
        "Kjb-App-Id",
        HeaderValue::from_static("Kajabi"),
    );
    headers.insert(
        "KJB-DP",
        HeaderValue::from_static("ANDROID"),
    );
    headers.insert(
        "KJB-SITE-ID",
        HeaderValue::from_str(site_id)?,
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

fn build_auth_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    headers.insert(
        "Kjb-App-Id",
        HeaderValue::from_static("Kajabi"),
    );
    headers.insert(
        "KJB-DP",
        HeaderValue::from_static("ANDROID"),
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
    Ok(data_dir.join("omniget").join("kajabi_session.json"))
}

pub async fn request_login_link(email: &str) -> anyhow::Result<String> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "email": email,
    });

    let resp = temp_client
        .post(&format!("{}/login_links", AUTH_BASE))
        .json(&payload)
        .send()
        .await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "Login link request failed (status {}): {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let login_token = body
        .get("token")
        .or_else(|| body.get("login_token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("token not found in login_links response"))?
        .to_string();

    Ok(login_token)
}

pub async fn verify_login(email: &str, confirmation_code: &str, login_token: &str) -> anyhow::Result<String> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "email": email,
        "confirmation_code": confirmation_code,
        "token": login_token,
    });

    let resp = temp_client
        .post(&format!("{}/authentication", AUTH_BASE))
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

    let bearer_token = body
        .get("bearerToken")
        .or_else(|| body.get("bearer_token"))
        .or_else(|| body.get("token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("bearerToken not found in authentication response"))?
        .to_string();

    Ok(bearer_token)
}

pub async fn validate_token(session: &KajabiSession) -> anyhow::Result<bool> {
    let client = build_auth_client(&session.token)?;

    let resp = client
        .get(&format!("{}/sites", AUTH_BASE))
        .send()
        .await?;

    Ok(resp.status().is_success())
}

pub async fn list_sites(token: &str) -> anyhow::Result<Vec<KajabiSite>> {
    let client = build_auth_client(token)?;

    let resp = client
        .get(&format!("{}/sites", AUTH_BASE))
        .send()
        .await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "list_sites returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let sites_arr = body
        .as_array()
        .or_else(|| body.get("sites").and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default();

    let mut sites = Vec::new();

    for item in &sites_arr {
        let id = item
            .get("id")
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            })
            .unwrap_or_default();

        let title = item
            .get("title")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        sites.push(KajabiSite { id, title });
    }

    Ok(sites)
}

pub async fn list_courses(session: &KajabiSession) -> anyhow::Result<Vec<KajabiCourse>> {
    let mut all_courses = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!(
            "{}/sites/{}/courses?page={}&per_page=50",
            BASE_URL, session.site_id, page
        );

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

        let courses_arr = body
            .get("data")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
            .cloned()
            .unwrap_or_default();

        if courses_arr.is_empty() {
            break;
        }

        for item in &courses_arr {
            let id = item
                .get("id")
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();

            let name = item
                .get("name")
                .or_else(|| item.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let thumbnail_url = item
                .get("thumbnail_url")
                .or_else(|| item.get("image_url"))
                .and_then(|v| v.as_str())
                .map(String::from);

            all_courses.push(KajabiCourse {
                id,
                name,
                thumbnail_url,
                site_id: session.site_id.clone(),
            });
        }

        page += 1;
    }

    Ok(all_courses)
}

pub async fn get_categories(
    session: &KajabiSession,
    product_id: &str,
) -> anyhow::Result<Vec<KajabiModule>> {
    let url = format!(
        "{}/sites/{}/products/{}/categories",
        BASE_URL, session.site_id, product_id
    );

    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_categories returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let categories_arr = body
        .get("data")
        .and_then(|v| v.get("categories"))
        .and_then(|v| v.as_array())
        .or_else(|| body.get("categories").and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default();

    let mut modules = Vec::new();

    for (i, cat) in categories_arr.iter().enumerate() {
        let cat_id = cat
            .get("id")
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => format!("{}", i),
            })
            .unwrap_or_else(|| format!("{}", i));

        let cat_name = cat
            .get("name")
            .or_else(|| cat.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let posts_arr = cat
            .get("posts")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut lessons = Vec::new();

        for (j, post) in posts_arr.iter().enumerate() {
            let post_id = post
                .get("id")
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => format!("{}", j),
                })
                .unwrap_or_else(|| format!("{}", j));

            let post_name = post
                .get("name")
                .or_else(|| post.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            lessons.push(KajabiLesson {
                id: post_id,
                name: post_name,
                order: j as i64,
            });
        }

        modules.push(KajabiModule {
            id: cat_id,
            name: cat_name,
            order: i as i64,
            lessons,
        });
    }

    Ok(modules)
}

pub async fn get_post_detail(
    session: &KajabiSession,
    product_id: &str,
    post_id: &str,
) -> anyhow::Result<KajabiLessonDetail> {
    let url = format!(
        "{}/sites/{}/products/{}/posts/{}",
        BASE_URL, session.site_id, product_id, post_id
    );

    let mut last_err = anyhow!("Failed after retries");

    for attempt in 0..3u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        let resp = match session.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = e.into();
                continue;
            }
        };

        let status = resp.status();

        if status.as_u16() == 403 || status.as_u16() == 429 {
            let body_text = resp.text().await.unwrap_or_default();
            last_err = anyhow!(
                "get_post_detail returned status {}: {}",
                status,
                &body_text[..body_text.len().min(300)]
            );
            continue;
        }

        let body_text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "get_post_detail returned status {}: {}",
                status,
                &body_text[..body_text.len().min(300)]
            ));
        }

        let body: serde_json::Value = serde_json::from_str(&body_text)?;

        let data = body.get("data").unwrap_or(&body);

        let id = data
            .get("id")
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => post_id.to_string(),
            })
            .unwrap_or_else(|| post_id.to_string());

        let name = data
            .get("name")
            .or_else(|| data.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let video_url = data
            .get("video_url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                data.get("video")
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .or_else(|| {
                data.get("wistia_video")
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .or_else(|| {
                data.get("content_videos")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .or_else(|| {
                data.get("media")
                    .and_then(|m| m.get("assets"))
                    .and_then(|a| a.as_array())
                    .and_then(|arr| {
                        arr.iter().find(|a| {
                            a.get("type")
                                .and_then(|v| v.as_str())
                                .map(|t| t == "HdMp4VideoFile")
                                .unwrap_or(false)
                        })
                    })
                    .and_then(|a| a.get("url"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });

        let mut files = Vec::new();

        if let Some(downloads) = data.get("downloads").and_then(|v| v.as_array()) {
            for dl in downloads {
                let file_id = dl
                    .get("id")
                    .map(|v| match v {
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::String(s) => s.clone(),
                        _ => String::new(),
                    })
                    .unwrap_or_default();

                let file_name = dl
                    .get("name")
                    .or_else(|| dl.get("filename"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("file")
                    .to_string();

                let file_url = dl
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                files.push(KajabiFile {
                    id: file_id,
                    name: file_name,
                    url: file_url,
                });
            }
        }

        return Ok(KajabiLessonDetail {
            id,
            name,
            video_url,
            files,
        });
    }

    Err(last_err)
}

pub async fn save_session(session: &KajabiSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        token: session.token.clone(),
        site_id: session.site_id.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[kajabi] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<KajabiSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token, &saved.site_id)?;

    tracing::info!("[kajabi] session loaded");

    Ok(Some(KajabiSession {
        token: saved.token,
        site_id: saved.site_id,
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
