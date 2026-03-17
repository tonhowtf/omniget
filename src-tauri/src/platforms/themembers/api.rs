use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const API_BASE: &str = "https://api.themembers.com.br/api";

#[derive(Clone)]
pub struct TheMembersSession {
    pub token: String,
    pub tenant_id: String,
    pub org_id: String,
    pub domain: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub tenant_id: String,
    pub org_id: String,
    pub domain: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheMembersCourse {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheMembersModule {
    pub id: i64,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<TheMembersLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheMembersLesson {
    pub id: i64,
    pub name: String,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheMembersLessonDetail {
    pub id: i64,
    pub name: String,
    pub video_url: Option<String>,
    pub host: String,
    pub files: Vec<TheMembersFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheMembersFile {
    pub name: String,
    pub url: String,
}

fn build_client(token: &str, tenant_id: &str, org_id: &str, domain: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    headers.insert("x-platform-id", HeaderValue::from_str(tenant_id)?);
    headers.insert("x-tenant-id", HeaderValue::from_str(tenant_id)?);
    headers.insert("Tenant-ID", HeaderValue::from_str(tenant_id)?);
    headers.insert("orgId", HeaderValue::from_str(org_id)?);
    headers.insert("Accept", HeaderValue::from_static("application/json"));
    headers.insert(
        "Origin",
        HeaderValue::from_str(&format!("https://{}", domain))?,
    );
    headers.insert(
        "Referer",
        HeaderValue::from_str(&format!("https://{}/", domain))?,
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
    Ok(data_dir.join("omniget").join("themembers_session.json"))
}

pub async fn resolve_tenant(domain: &str) -> anyhow::Result<(String, String)> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let url = format!("{}/getTenant?domain={}", API_BASE, domain);
    let resp = temp_client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "resolve_tenant returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let tenant = body.get("tenant").unwrap_or(&body);

    let tenant_id = tenant
        .get("id")
        .map(|v| match v {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            _ => String::new(),
        })
        .unwrap_or_default();

    let org_id = tenant
        .get("organization_id")
        .map(|v| match v {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            _ => String::new(),
        })
        .unwrap_or_default();

    if tenant_id.is_empty() {
        return Err(anyhow!("Could not resolve tenant_id for domain {}", domain));
    }

    Ok((tenant_id, org_id))
}

pub async fn authenticate(email: &str, password: &str, domain: &str) -> anyhow::Result<TheMembersSession> {
    let (tenant_id, org_id) = resolve_tenant(domain).await?;

    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let payload = serde_json::json!({
        "email": email,
        "password": password,
        "tenant_id": tenant_id,
    });

    let resp = temp_client
        .post(format!("{}/auth/login", API_BASE))
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
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("access_token not found in login response"))?
        .to_string();

    let client = build_client(&token, &tenant_id, &org_id, domain)?;

    Ok(TheMembersSession {
        token,
        tenant_id,
        org_id,
        domain: domain.to_string(),
        client,
    })
}

pub async fn authenticate_token(token: &str, domain: &str) -> anyhow::Result<TheMembersSession> {
    let (tenant_id, org_id) = resolve_tenant(domain).await?;
    let client = build_client(token, &tenant_id, &org_id, domain)?;

    let session = TheMembersSession {
        token: token.to_string(),
        tenant_id,
        org_id,
        domain: domain.to_string(),
        client,
    };

    let valid = validate_token(&session).await?;
    if !valid {
        return Err(anyhow!("Invalid token"));
    }

    Ok(session)
}

pub async fn validate_token(session: &TheMembersSession) -> anyhow::Result<bool> {
    let url = format!(
        "{}/auth/sideList/{}/courses",
        API_BASE, session.tenant_id
    );

    let resp = session.client.get(&url).send().await?;
    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &TheMembersSession) -> anyhow::Result<Vec<TheMembersCourse>> {
    let url = format!(
        "{}/auth/sideList/{}/courses",
        API_BASE, session.tenant_id
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

    let data_arr = body
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut courses = Vec::new();
    for item in &data_arr {
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let name = item
            .get("title")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if id != 0 {
            courses.push(TheMembersCourse { id, name });
        }
    }

    Ok(courses)
}

pub async fn get_course_content(
    session: &TheMembersSession,
    course_id: i64,
) -> anyhow::Result<Vec<TheMembersModule>> {
    let modules_url = format!(
        "{}/auth/sideList/{}/modules",
        API_BASE, course_id
    );

    let resp = session.client.get(&modules_url).send().await?;
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

    let modules_arr = body
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut modules = Vec::new();

    for (mi, module_val) in modules_arr.iter().enumerate() {
        let module_id = module_val.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let module_name = module_val
            .get("title")
            .or_else(|| module_val.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let lessons_url = format!(
            "{}/auth/sideList/{}/lessons",
            API_BASE, module_id
        );

        let lessons = match session.client.get(&lessons_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let lessons_text = resp.text().await.unwrap_or_default();
                let lessons_body: serde_json::Value =
                    serde_json::from_str(&lessons_text).unwrap_or(serde_json::Value::Null);

                let lessons_arr = lessons_body
                    .get("data")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                lessons_arr
                    .iter()
                    .enumerate()
                    .map(|(li, lesson_val)| {
                        let lesson_id = lesson_val
                            .get("id")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);

                        let lesson_name = lesson_val
                            .get("title")
                            .or_else(|| lesson_val.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        TheMembersLesson {
                            id: lesson_id,
                            name: lesson_name,
                            order: li as i64,
                        }
                    })
                    .collect()
            }
            _ => vec![],
        };

        modules.push(TheMembersModule {
            id: module_id,
            name: module_name,
            order: mi as i64,
            lessons,
        });
    }

    Ok(modules)
}

pub async fn get_lesson_detail(
    session: &TheMembersSession,
    lesson_id: i64,
) -> anyhow::Result<TheMembersLessonDetail> {
    let url = format!(
        "{}/auth/home/class/{}/{}",
        API_BASE, lesson_id, session.tenant_id
    );

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_lesson_detail returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let class_obj = body.get("class").unwrap_or(&body);

    let name = class_obj
        .get("title")
        .or_else(|| class_obj.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let host = class_obj
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let raw_video_url = class_obj
        .get("url_video")
        .and_then(|v| v.as_str())
        .map(String::from);

    let video_url = match (host.as_str(), &raw_video_url) {
        ("the-player-ai", Some(url)) => {
            resolve_the_player_ai_url(url).await.ok()
        }
        (_, Some(url)) if !url.is_empty() => Some(url.clone()),
        _ => None,
    };

    let materials_url = format!(
        "{}/auth/home/materials/{}",
        API_BASE, lesson_id
    );

    let files = match session.client.get(&materials_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let files_text = resp.text().await.unwrap_or_default();
            let files_body: serde_json::Value =
                serde_json::from_str(&files_text).unwrap_or(serde_json::Value::Array(vec![]));

            let files_arr = match &files_body {
                serde_json::Value::Array(arr) => arr.clone(),
                _ => files_body
                    .get("data")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
            };

            files_arr
                .iter()
                .filter_map(|f| {
                    let file_name = f
                        .get("title")
                        .or_else(|| f.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("file")
                        .to_string();

                    let file_url = f
                        .get("url")
                        .or_else(|| f.get("file_url"))
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    file_url.map(|url| TheMembersFile {
                        name: file_name,
                        url,
                    })
                })
                .collect()
        }
        _ => vec![],
    };

    Ok(TheMembersLessonDetail {
        id: lesson_id,
        name,
        video_url,
        host,
        files,
    })
}

async fn resolve_the_player_ai_url(video_url: &str) -> anyhow::Result<String> {
    let video_id = video_url
        .rsplit('/')
        .next()
        .unwrap_or(video_url)
        .split('?')
        .next()
        .unwrap_or(video_url);

    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let url = format!("https://player.themembers.com.br/api/video/{}", video_id);
    let resp = temp_client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "resolve_the_player_ai_url returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let hls_url = body
        .get("data")
        .and_then(|d| d.get("urls"))
        .and_then(|u| u.get("hls"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("No HLS URL found in the-player-ai response"))?
        .to_string();

    Ok(hls_url)
}

pub async fn save_session(session: &TheMembersSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        token: session.token.clone(),
        tenant_id: session.tenant_id.clone(),
        org_id: session.org_id.clone(),
        domain: session.domain.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[themembers] session saved for {}", session.domain);
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<TheMembersSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token, &saved.tenant_id, &saved.org_id, &saved.domain)?;

    tracing::info!("[themembers] session loaded for {}", saved.domain);

    Ok(Some(TheMembersSession {
        token: saved.token,
        tenant_id: saved.tenant_id,
        org_id: saved.org_id,
        domain: saved.domain,
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
