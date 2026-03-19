use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const API_BASE: &str = "https://cms.medway.com.br/api/v2";

#[derive(Clone)]
pub struct MedwaySession {
    pub token: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedwayCourse {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedwayModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<MedwayLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedwayLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub item_type: String,
    pub video_url: Option<String>,
    pub document_id: Option<String>,
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
    Ok(data_dir.join("omniget").join("medway_session.json"))
}

pub async fn validate_token(session: &MedwaySession) -> anyhow::Result<bool> {
    let resp = session
        .client
        .get(format!("{}/student_group/", API_BASE))
        .send()
        .await?;

    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &MedwaySession) -> anyhow::Result<Vec<MedwayCourse>> {
    let mut courses = Vec::new();
    let mut next_url: Option<String> = Some(format!("{}/student_group/", API_BASE));

    while let Some(url) = next_url {
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

        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for item in &results {
            let id = item
                .get("id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Curso")
                .to_string();

            courses.push(MedwayCourse { id, name });
        }

        next_url = body
            .get("next")
            .and_then(|v| v.as_str())
            .map(String::from);
    }

    Ok(courses)
}

pub async fn get_modules(session: &MedwaySession, course_id: i64) -> anyhow::Result<Vec<MedwayModule>> {
    let subjects = fetch_subjects(session, course_id).await?;
    let mut modules = Vec::new();

    for (si, subject) in subjects.iter().enumerate() {
        let subject_id = subject
            .get("id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let subject_name = subject
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let subject_order = subject
            .get("order")
            .and_then(|v| v.as_i64())
            .unwrap_or(si as i64 + 1);

        let sub_modules = fetch_subject_modules(session, subject_id).await?;

        for module in &sub_modules {
            let module_id = module
                .get("id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let module_name = module
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Module")
                .to_string();

            let module_order = module
                .get("order")
                .or_else(|| module.get("lesson_order"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let title = if module_order > 0 {
                format!("{}. {} - {}. {}", subject_order, subject_name, module_order, module_name)
            } else {
                format!("{}. {} - {}", subject_order, subject_name, module_name)
            };

            let lessons = fetch_module_lessons(session, module_id).await?;

            modules.push(MedwayModule {
                id: module_id.to_string(),
                name: title,
                order: subject_order * 1000 + module_order,
                lessons,
            });
        }
    }

    modules.sort_by_key(|m| m.order);

    Ok(modules)
}

async fn fetch_subjects(session: &MedwaySession, course_id: i64) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut subjects = Vec::new();
    let mut next_url: Option<String> = Some(format!(
        "{}/lesson-subject/?ordering=order&studentgroup={}",
        API_BASE, course_id
    ));

    while let Some(url) = next_url {
        let resp = session.client.get(&url).send().await?;
        let status = resp.status();
        let body_text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "fetch_subjects returned status {}: {}",
                status,
                &body_text[..body_text.len().min(300)]
            ));
        }

        let body: serde_json::Value = serde_json::from_str(&body_text)?;

        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        subjects.extend(results);

        next_url = body
            .get("next")
            .and_then(|v| v.as_str())
            .map(String::from);
    }

    Ok(subjects)
}

async fn fetch_subject_modules(session: &MedwaySession, subject_id: i64) -> anyhow::Result<Vec<serde_json::Value>> {
    let url = format!("{}/lesson-subject/{}/modules/", API_BASE, subject_id);

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "fetch_subject_modules returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    match body {
        serde_json::Value::Array(arr) => Ok(arr),
        _ => Ok(vec![body]),
    }
}

async fn fetch_module_lessons(session: &MedwaySession, module_id: i64) -> anyhow::Result<Vec<MedwayLesson>> {
    let url = format!("{}/lesson-module/{}/", API_BASE, module_id);

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "fetch_module_lessons returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let items = body
        .get("module_items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut lessons = Vec::new();

    for (i, item) in items.iter().enumerate() {
        let id = item
            .get("id")
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => format!("{}-{}", module_id, i),
            })
            .unwrap_or_else(|| format!("{}-{}", module_id, i));

        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let order = item
            .get("order")
            .and_then(|v| v.as_i64())
            .unwrap_or(i as i64 + 1);

        let item_type = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let video_url = item
            .get("url_lesson")
            .and_then(|v| v.as_str())
            .map(String::from);

        let document_id = item
            .get("object_id")
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            });

        let description = item
            .get("description")
            .or_else(|| item.get("content"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);

        lessons.push(MedwayLesson {
            id,
            name,
            order,
            item_type,
            video_url,
            document_id,
            description,
        });
    }

    lessons.sort_by_key(|l| l.order);

    Ok(lessons)
}

pub async fn get_document_url(session: &MedwaySession, document_id: &str) -> anyhow::Result<(String, String)> {
    let url = format!("{}/lesson-document/{}/", API_BASE, document_id);

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_document_url returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let doc_url = body
        .get("document")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let doc_name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Document")
        .to_string();

    if doc_url.is_empty() {
        return Err(anyhow!("No document URL found in response"));
    }

    Ok((doc_url, doc_name))
}

pub fn resolve_video_url(url: &str) -> String {
    if url.contains("vimeo.com") && !url.contains("player.vimeo.com") {
        let video_id = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        if !video_id.is_empty() {
            return format!("https://player.vimeo.com/video/{}?autoplay=1&app_id=122963", video_id);
        }
    }
    url.to_string()
}

pub async fn save_session(session: &MedwaySession) -> anyhow::Result<()> {
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
    tracing::info!("[medway] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<MedwaySession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[medway] session loaded");

    Ok(Some(MedwaySession {
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
