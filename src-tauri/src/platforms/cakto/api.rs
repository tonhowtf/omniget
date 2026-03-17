use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const API_BASE: &str = "https://aluno.cakto.com.br";

#[derive(Clone)]
pub struct CaktoSession {
    pub cookie: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub cookie: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaktoCourse {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaktoModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<CaktoLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaktoLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub video_url: Option<String>,
    pub files: Vec<CaktoFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaktoFile {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
}

fn build_client(cookie: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Cookie",
        HeaderValue::from_str(&format!("__Secure-better-auth.session_token={}", cookie))?,
    );
    headers.insert("Accept", HeaderValue::from_static("application/json"));
    headers.insert(
        "Origin",
        HeaderValue::from_static(API_BASE),
    );
    headers.insert(
        "Referer",
        HeaderValue::from_str(&format!("{}/", API_BASE))?,
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
    Ok(data_dir.join("omniget").join("cakto_session.json"))
}

pub async fn authenticate(email: &str, password: &str) -> anyhow::Result<CaktoSession> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "email": email,
        "password": password,
        "callbackURL": "/app",
    });

    let resp = temp_client
        .post(format!("{}/api/auth/sign-in/email", API_BASE))
        .json(&payload)
        .send()
        .await?;

    let status = resp.status();

    let mut cookie_value = String::new();
    for cookie in resp.cookies() {
        if cookie.name().contains("better-auth.session_token") {
            cookie_value = cookie.value().to_string();
            break;
        }
    }

    if cookie_value.is_empty() {
        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!(
                "Authentication failed (status {}): {}",
                status,
                &body_text[..body_text.len().min(300)]
            ));
        }

        if let Ok(body) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if let Some(token) = body.get("token").and_then(|v| v.as_str()) {
                cookie_value = token.to_string();
            } else if let Some(session) = body.get("session") {
                if let Some(token) = session.get("token").and_then(|v| v.as_str()) {
                    cookie_value = token.to_string();
                }
            }
        }

        if cookie_value.is_empty() {
            return Err(anyhow!("No session token received from login"));
        }
    }

    let client = build_client(&cookie_value)?;

    Ok(CaktoSession {
        cookie: cookie_value,
        client,
    })
}

pub async fn validate_token(session: &CaktoSession) -> anyhow::Result<bool> {
    let url = format!("{}/api/user/courses?page=1&limit=1", API_BASE);
    let resp = session.client.get(&url).send().await?;
    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &CaktoSession) -> anyhow::Result<Vec<CaktoCourse>> {
    let mut all_courses = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!("{}/api/user/courses?page={}&limit=50", API_BASE, page);
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
            .get("courses")
            .or_else(|| body.get("data"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if courses_arr.is_empty() {
            break;
        }

        for item in &courses_arr {
            let id = item
                .get("courseId")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let name = item
                .get("title")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let image_url = item
                .get("image")
                .or_else(|| item.get("thumbnail"))
                .and_then(|v| v.as_str())
                .map(String::from);

            if !id.is_empty() {
                all_courses.push(CaktoCourse {
                    id,
                    name,
                    image_url,
                });
            }
        }

        page += 1;
        if courses_arr.len() < 50 {
            break;
        }
    }

    Ok(all_courses)
}

pub async fn get_course_content(
    session: &CaktoSession,
    course_id: &str,
) -> anyhow::Result<Vec<CaktoModule>> {
    let url = format!("{}/api/courses/{}", API_BASE, course_id);
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

    let course_data = body.get("course").unwrap_or(&body);

    let modules_arr = course_data
        .get("modules")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut modules = Vec::new();

    for (mi, module_val) in modules_arr.iter().enumerate() {
        let mod_id = module_val
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mod_name = module_val
            .get("title")
            .or_else(|| module_val.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let lessons_arr = module_val
            .get("lessons")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut lessons = Vec::new();

        for (li, lesson_val) in lessons_arr.iter().enumerate() {
            let lesson_id = lesson_val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let lesson_name = lesson_val
                .get("title")
                .or_else(|| lesson_val.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let video_url = lesson_val
                .get("videoUrl")
                .or_else(|| lesson_val.get("video_url"))
                .and_then(|v| v.as_str())
                .map(String::from);

            let files = lesson_val
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|f| {
                            let fid = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let fname = f.get("name").and_then(|v| v.as_str()).unwrap_or("file").to_string();
                            let furl = f.get("url").and_then(|v| v.as_str()).map(String::from);
                            CaktoFile { id: fid, name: fname, url: furl }
                        })
                        .collect()
                })
                .unwrap_or_default();

            lessons.push(CaktoLesson {
                id: if lesson_id.is_empty() { format!("{}", li) } else { lesson_id },
                name: lesson_name,
                order: li as i64,
                video_url,
                files,
            });
        }

        modules.push(CaktoModule {
            id: if mod_id.is_empty() { format!("{}", mi) } else { mod_id },
            name: mod_name,
            order: mi as i64,
            lessons,
        });
    }

    Ok(modules)
}

pub async fn get_file_download_url(
    session: &CaktoSession,
    lesson_id: &str,
    file_id: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/api/lessons/{}/files/{}/download", API_BASE, lesson_id, file_id);
    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_file_download_url returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let download_url = body
        .get("url")
        .or_else(|| body.get("download_url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("No download URL in file response"))?
        .to_string();

    Ok(download_url)
}

pub async fn save_session(session: &CaktoSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        cookie: session.cookie.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[cakto] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<CaktoSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.cookie)?;

    tracing::info!("[cakto] session loaded");

    Ok(Some(CaktoSession {
        cookie: saved.cookie,
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
