use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";

#[derive(Clone)]
pub struct WondriumSession {
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
pub struct WondriumCourse {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WondriumModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<WondriumLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WondriumLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub video_id: Option<String>,
    pub guidebook_url: Option<String>,
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
    Ok(data_dir.join("omniget").join("wondrium_session.json"))
}

pub async fn authenticate(email: &str, password: &str) -> anyhow::Result<WondriumSession> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "username": email,
        "password": password,
    });

    let resp = temp_client
        .post("https://www.thegreatcoursesplus.com/rest/V1/integration/customer/token")
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

    let token = body_text.trim().trim_matches('"').to_string();

    if token.is_empty() {
        return Err(anyhow!("Empty token in login response"));
    }

    let client = build_client(&token)?;

    Ok(WondriumSession {
        token,
        email: email.to_string(),
        client,
    })
}

pub async fn validate_token(session: &WondriumSession) -> anyhow::Result<bool> {
    let resp = session
        .client
        .get("https://www.thegreatcoursesplus.com/rest/all/V2/watchlist/mine/items")
        .send()
        .await?;

    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &WondriumSession) -> anyhow::Result<Vec<WondriumCourse>> {
    let resp = session
        .client
        .get("https://www.thegreatcoursesplus.com/rest/all/V2/watchlist/mine/items")
        .send()
        .await?;

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

    let mut courses = Vec::new();

    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default();

    for item in &items {
        let id = item
            .get("product_id")
            .or_else(|| item.get("id"))
            .or_else(|| item.get("sku"))
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

        if !id.is_empty() && !name.is_empty() {
            courses.push(WondriumCourse { id, name });
        }
    }

    Ok(courses)
}

pub async fn get_course_content(
    session: &WondriumSession,
    course_id: &str,
) -> anyhow::Result<Vec<WondriumModule>> {
    let url = format!(
        "https://m2api.thegreatcourses.com/rest/all/V1/dlo_products/{}",
        course_id
    );

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

    let lectures = body
        .get("lectures")
        .or_else(|| body.get("episodes"))
        .or_else(|| body.get("items"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut modules = Vec::new();
    let mut current_module: Option<WondriumModule> = None;
    let mut module_index = 0;

    for (li, lecture) in lectures.iter().enumerate() {
        let lecture_id = lecture
            .get("id")
            .or_else(|| lecture.get("lecture_id"))
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => format!("{}", li),
            })
            .unwrap_or_else(|| format!("{}", li));

        let lecture_name = lecture
            .get("title")
            .or_else(|| lecture.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let video_id = lecture
            .get("video_id")
            .or_else(|| lecture.get("videoId"))
            .or_else(|| lecture.get("media_id"))
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty());

        let guidebook_url = lecture
            .get("guidebook_url")
            .or_else(|| lecture.get("pdf_url"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let section = lecture
            .get("section")
            .or_else(|| lecture.get("part"))
            .and_then(|v| v.as_str())
            .unwrap_or("Lectures");

        let lesson = WondriumLesson {
            id: lecture_id,
            name: lecture_name,
            order: li as i64,
            video_id,
            guidebook_url,
        };

        match &mut current_module {
            Some(m) if m.name == section => {
                m.lessons.push(lesson);
            }
            _ => {
                if let Some(m) = current_module.take() {
                    modules.push(m);
                }
                module_index += 1;
                current_module = Some(WondriumModule {
                    id: format!("{}", module_index),
                    name: section.to_string(),
                    order: module_index as i64 - 1,
                    lessons: vec![lesson],
                });
            }
        }
    }

    if let Some(m) = current_module {
        modules.push(m);
    }

    if modules.is_empty() && !lectures.is_empty() {
        let mut all_lessons = Vec::new();
        for (li, lecture) in lectures.iter().enumerate() {
            let lecture_id = lecture
                .get("id")
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => format!("{}", li),
                })
                .unwrap_or_else(|| format!("{}", li));

            let lecture_name = lecture
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let video_id = lecture
                .get("video_id")
                .or_else(|| lecture.get("media_id"))
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty());

            let guidebook_url = lecture
                .get("guidebook_url")
                .and_then(|v| v.as_str())
                .map(String::from);

            all_lessons.push(WondriumLesson {
                id: lecture_id,
                name: lecture_name,
                order: li as i64,
                video_id,
                guidebook_url,
            });
        }

        modules.push(WondriumModule {
            id: "1".to_string(),
            name: "Lectures".to_string(),
            order: 0,
            lessons: all_lessons,
        });
    }

    Ok(modules)
}

pub fn get_video_url(video_id: &str) -> String {
    format!(
        "https://link.theplatform.com/s/jESqeC/media/guid/2140042498/{}?manifest=m3u",
        video_id
    )
}

pub async fn save_session(session: &WondriumSession) -> anyhow::Result<()> {
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
    tracing::info!("[wondrium] session saved for {}", session.email);
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<WondriumSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[wondrium] session loaded for {}", saved.email);

    Ok(Some(WondriumSession {
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
