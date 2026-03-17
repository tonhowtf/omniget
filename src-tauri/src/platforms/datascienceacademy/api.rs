use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const API_BASE: &str = "https://www.datascienceacademy.com.br";

#[derive(Clone)]
pub struct DsaSession {
    pub token: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsaCourse {
    pub id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsaModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<DsaLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsaLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub item_type: String,
    pub vimeo_id: Option<String>,
}

fn build_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Token",
        HeaderValue::from_str(token)?,
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
    Ok(data_dir.join("omniget").join("dsa_session.json"))
}

pub async fn validate_token(session: &DsaSession) -> anyhow::Result<bool> {
    let url = format!("{}/api/products_all", API_BASE);

    let resp = session.client.get(&url).send().await?;

    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &DsaSession) -> anyhow::Result<Vec<DsaCourse>> {
    let url = format!("{}/api/products_all", API_BASE);

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

    let courses_map = body
        .get("courses")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let allowed_ids: Vec<String> = body
        .get("allowedCourseIds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from).or_else(|| Some(v.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let mut enrolled_ids: Vec<String> = Vec::new();
    let progress_url = format!("{}/api/user/courses-progress", API_BASE);
    if let Ok(progress_resp) = session.client.get(&progress_url).send().await {
        if progress_resp.status().is_success() {
            if let Ok(progress_text) = progress_resp.text().await {
                if let Ok(progress_body) = serde_json::from_str::<serde_json::Value>(&progress_text) {
                    if let Some(user_courses) = progress_body.get("userCourses").and_then(|v| v.as_array()) {
                        for uc in user_courses {
                            let me = uc.get("me").unwrap_or(&serde_json::Value::Null);
                            let is_premium = me.get("premium").and_then(|v| v.as_bool()).unwrap_or(false);
                            let is_registered = me.get("registered").and_then(|v| v.as_bool()).unwrap_or(false);
                            if is_premium || is_registered {
                                if let Some(cid) = uc.get("courseId") {
                                    enrolled_ids.push(cid.to_string().trim_matches('"').to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut result = Vec::new();

    for (cid, course) in &courses_map {
        let me = course.get("me").unwrap_or(&serde_json::Value::Null);
        let course_id = course
            .get("id")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_else(|| cid.clone());

        let is_registered = me.get("registered").and_then(|v| v.as_bool()).unwrap_or(false);
        let is_allowed = allowed_ids.contains(&course_id) || allowed_ids.contains(cid);
        let is_free = course.get("status").and_then(|v| v.as_str()) == Some("free");
        let is_enrolled = enrolled_ids.contains(&course_id);

        if !is_registered && !is_allowed && !is_free && !is_enrolled {
            continue;
        }

        let name = course
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Course")
            .to_string();

        let slug = course
            .get("titleId")
            .and_then(|v| v.as_str())
            .unwrap_or(cid)
            .to_string();

        result.push(DsaCourse {
            id: course_id,
            name,
            slug,
        });
    }

    result.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(result)
}

pub async fn get_course_content(
    session: &DsaSession,
    slug: &str,
) -> anyhow::Result<Vec<DsaModule>> {
    let url = format!("{}/api/course/{}?contents", API_BASE, slug);

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
    let sections = course_data.get("sections");
    let videos = course_data
        .get("videos")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let sections_list: Vec<serde_json::Value> = match sections {
        Some(serde_json::Value::Object(map)) => map.values().cloned().collect(),
        Some(serde_json::Value::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };

    let mut modules = Vec::new();

    for (section_index, section) in sections_list.iter().enumerate() {
        let title = resolve_title(
            section.get("titles").or_else(|| section.get("title")),
            &format!("Module {}", section_index + 1),
        );

        let learning_path = section
            .get("learningPath")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut lessons = Vec::new();

        for (lesson_index, item) in learning_path.iter().enumerate() {
            let unit_id = item
                .get("id")
                .map(|v| v.to_string().trim_matches('"').to_string())
                .unwrap_or_else(|| format!("{}-{}", section_index + 1, lesson_index + 1));

            let unit_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();

            let mut lesson_title = String::new();

            if (unit_type == "ivideo" || unit_type == "video") && videos.contains_key(&unit_id) {
                if let Some(vid) = videos.get(&unit_id) {
                    lesson_title = vid
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }

            if lesson_title.is_empty() || lesson_title == "Untitled" {
                lesson_title = resolve_title(
                    item.get("titles").or_else(|| item.get("unitTitle")),
                    &format!("Lesson {}", lesson_index + 1),
                );
            }

            let vimeo_id = if (unit_type == "ivideo" || unit_type == "video") && videos.contains_key(&unit_id) {
                videos
                    .get(&unit_id)
                    .and_then(|vid| vid.get("vimeoid"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            };

            lessons.push(DsaLesson {
                id: unit_id,
                name: lesson_title,
                order: (lesson_index + 1) as i64,
                item_type: unit_type,
                vimeo_id,
            });
        }

        let section_id = section
            .get("titleId")
            .or_else(|| section.get("id"))
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_else(|| format!("{}", section_index + 1));

        modules.push(DsaModule {
            id: section_id,
            name: title,
            order: (section_index + 1) as i64,
            lessons,
        });
    }

    Ok(modules)
}

fn resolve_title(raw: Option<&serde_json::Value>, fallback: &str) -> String {
    match raw {
        Some(serde_json::Value::Object(map)) => {
            for key in &["pt-BR", "pt", "en", "title"] {
                if let Some(val) = map.get(*key).and_then(|v| v.as_str()) {
                    let trimmed = val.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
            for val in map.values() {
                if let Some(s) = val.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
            fallback.to_string()
        }
        Some(serde_json::Value::Array(arr)) => {
            for entry in arr {
                if let Some(s) = entry.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
            fallback.to_string()
        }
        Some(serde_json::Value::String(s)) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                trimmed.to_string()
            } else {
                fallback.to_string()
            }
        }
        _ => fallback.to_string(),
    }
}

pub async fn save_session(session: &DsaSession) -> anyhow::Result<()> {
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
    tracing::info!("[dsa] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<DsaSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[dsa] session loaded");

    Ok(Some(DsaSession {
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
