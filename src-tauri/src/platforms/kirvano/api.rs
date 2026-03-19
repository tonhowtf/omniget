use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";

#[derive(Clone)]
pub struct KirvanoSession {
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
pub struct KirvanoCourse {
    pub id: String,
    pub name: String,
    pub seller: String,
    pub image_url: Option<String>,
    pub course_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KirvanoModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<KirvanoLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KirvanoLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub available: bool,
    pub video_url: Option<String>,
    pub description: Option<String>,
}

fn build_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    headers.insert("Accept", HeaderValue::from_static("application/json"));
    headers.insert(
        "Origin",
        HeaderValue::from_static("https://app.kirvano.com"),
    );
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://app.kirvano.com/"),
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
    Ok(data_dir.join("omniget").join("kirvano_session.json"))
}

pub async fn authenticate(email: &str, password: &str) -> anyhow::Result<KirvanoSession> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "email": email,
        "password": password,
        "source": "app-web",
        "fingerprint": null,
    });

    let resp = temp_client
        .post("https://app-api.kirvano.com/users/login/otp")
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

    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("token not found in login response"))?
        .to_string();

    let client = build_client(&token)?;

    Ok(KirvanoSession {
        token,
        email: email.to_string(),
        client,
    })
}

pub async fn validate_token(session: &KirvanoSession) -> anyhow::Result<bool> {
    let resp = session
        .client
        .get("https://app-api.kirvano.com/purchases?page=1&pageSize=1")
        .send()
        .await?;

    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &KirvanoSession) -> anyhow::Result<Vec<KirvanoCourse>> {
    let mut all_courses = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!(
            "https://app-api.kirvano.com/purchases?page={}&pageSize=99",
            page
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

        let items = body
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if items.is_empty() {
            break;
        }

        for item in &items {
            let purchase_uuid = item
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let name = item
                .get("product")
                .and_then(|v| v.as_str())
                .unwrap_or("Unnamed")
                .to_string();

            let image_url = item.get("photo").and_then(|v| v.as_str()).map(String::from);

            let seller = item
                .get("sellerName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let course_uuid = item
                .get("courseUuid")
                .or_else(|| item.get("productUuid"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if course_uuid.is_empty() {
                let detail_url = format!(
                    "https://app-api.kirvano.com/purchases/{}",
                    purchase_uuid
                );
                if let Ok(detail_resp) = session.client.get(&detail_url).send().await {
                    if detail_resp.status().is_success() {
                        if let Ok(detail_text) = detail_resp.text().await {
                            if let Ok(detail_body) = serde_json::from_str::<serde_json::Value>(&detail_text) {
                                let resolved_uuid = detail_body
                                    .get("courseUuid")
                                    .or_else(|| detail_body.get("productUuid"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let resolved_seller = detail_body
                                    .get("sellerName")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&seller)
                                    .to_string();

                                if !resolved_uuid.is_empty() {
                                    all_courses.push(KirvanoCourse {
                                        id: purchase_uuid,
                                        name: name.clone(),
                                        seller: resolved_seller,
                                        image_url: image_url.clone(),
                                        course_uuid: resolved_uuid,
                                    });
                                    continue;
                                }
                            }
                        }
                    }
                }
                continue;
            }

            all_courses.push(KirvanoCourse {
                id: purchase_uuid,
                name,
                seller,
                image_url,
                course_uuid,
            });
        }

        let meta = body.get("meta");
        let current_page = meta
            .and_then(|m| m.get("page"))
            .and_then(|v| v.as_u64())
            .unwrap_or(page as u64);
        let total_pages = meta
            .and_then(|m| m.get("pages"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);

        if current_page >= total_pages {
            break;
        }

        page += 1;
    }

    Ok(all_courses)
}

pub async fn get_course_modules(
    session: &KirvanoSession,
    course_uuid: &str,
) -> anyhow::Result<Vec<KirvanoModule>> {
    let mut all_modules = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!(
            "https://members-api.kirvano.com/v1/courses/{}/lessons/modules?page={}&pageSize=99",
            course_uuid, page
        );

        let resp = session.client.get(&url).send().await?;

        let status = resp.status();
        let body_text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "get_course_modules returned status {}: {}",
                status,
                &body_text[..body_text.len().min(300)]
            ));
        }

        let body: serde_json::Value = serde_json::from_str(&body_text)?;

        let items = body
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if items.is_empty() {
            break;
        }

        for item in &items {
            let mod_uuid = item
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let mod_name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Module")
                .to_string();

            let mod_order = item
                .get("order")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let lessons = get_module_lessons(session, course_uuid, &mod_uuid).await?;

            all_modules.push(KirvanoModule {
                id: mod_uuid,
                name: mod_name,
                order: mod_order,
                lessons,
            });
        }

        let meta = body.get("meta");
        let current_page = meta
            .and_then(|m| m.get("page"))
            .and_then(|v| v.as_u64())
            .unwrap_or(page as u64);
        let total_pages = meta
            .and_then(|m| m.get("pages"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);

        if current_page >= total_pages {
            break;
        }

        page += 1;
    }

    all_modules.sort_by_key(|m| m.order);

    Ok(all_modules)
}

async fn get_module_lessons(
    session: &KirvanoSession,
    course_uuid: &str,
    module_uuid: &str,
) -> anyhow::Result<Vec<KirvanoLesson>> {
    let mut all_lessons = Vec::new();
    let mut page = 1u32;

    loop {
        let url = format!(
            "https://members-api.kirvano.com/v1/courses/{}/lessons/modules/{}/contents?page={}&pageSize=99",
            course_uuid, module_uuid, page
        );

        let resp = session.client.get(&url).send().await?;

        let status = resp.status();
        let body_text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "get_module_lessons returned status {}: {}",
                status,
                &body_text[..body_text.len().min(300)]
            ));
        }

        let body: serde_json::Value = serde_json::from_str(&body_text)?;

        let items = body
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if items.is_empty() {
            break;
        }

        for item in &items {
            let lesson_uuid = item
                .get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let lesson_name = item
                .get("name")
                .or_else(|| item.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Lesson")
                .to_string();

            let lesson_order = item
                .get("order")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let available = item
                .get("available")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let video_url = item
                .get("videoUrl")
                .and_then(|v| v.as_str())
                .map(String::from);

            let description = item
                .get("description")
                .or_else(|| item.get("content"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(String::from);

            all_lessons.push(KirvanoLesson {
                id: lesson_uuid,
                name: lesson_name,
                order: lesson_order,
                available,
                video_url,
                description,
            });
        }

        let meta = body.get("meta");
        let current_page = meta
            .and_then(|m| m.get("page"))
            .and_then(|v| v.as_u64())
            .unwrap_or(page as u64);
        let total_pages = meta
            .and_then(|m| m.get("pages"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);

        if current_page >= total_pages {
            break;
        }

        page += 1;
    }

    all_lessons.sort_by_key(|l| l.order);

    Ok(all_lessons)
}

pub async fn save_session(session: &KirvanoSession) -> anyhow::Result<()> {
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
    tracing::info!("[kirvano] session saved for {}", session.email);
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<KirvanoSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[kirvano] session loaded for {}", saved.email);

    Ok(Some(KirvanoSession {
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
