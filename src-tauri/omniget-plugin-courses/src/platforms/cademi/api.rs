use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";

#[derive(Clone)]
pub struct CademiSession {
    pub cookies: String,
    pub site_url: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub cookies: String,
    pub site_url: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CademiCourse {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CademiModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<CademiLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CademiLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CademiLessonDetail {
    pub id: String,
    pub name: String,
    pub video_url: Option<String>,
    pub description: Option<String>,
}

fn build_client(cookies: &str, site_url: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Cookie",
        HeaderValue::from_str(&format!("app_v4_session={}", cookies))?,
    );
    headers.insert(
        "Referer",
        HeaderValue::from_str(&format!("{}/", site_url))?,
    );

    let client = omniget_core::core::http_client::apply_global_proxy(reqwest::Client::builder())
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
    Ok(data_dir.join("omniget").join("cademi_session.json"))
}

pub async fn authenticate(
    email: &str,
    password: &str,
    site_url: &str,
) -> anyhow::Result<CademiSession> {
    let temp_client = omniget_core::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let login_page_url = format!("{}/login", site_url.trim_end_matches('/'));
    let resp = temp_client.get(&login_page_url).send().await?;
    let login_html = resp.text().await?;

    let csrf_token = extract_input_value(&login_html, "_token")
        .ok_or_else(|| anyhow!("Could not find CSRF token on login page"))?;

    let form_url = format!("{}/acesso", site_url.trim_end_matches('/'));
    let resp = temp_client
        .post(&form_url)
        .form(&[
            ("_token", csrf_token.as_str()),
            ("Acesso[email]", email),
            ("Acesso[senha]", password),
        ])
        .send()
        .await?;

    let status = resp.status();
    let resp_url = resp.url().to_string();
    let _body = resp.text().await?;

    if resp_url.contains("/login") || resp_url.contains("/acesso") {
        if !status.is_success() && !status.is_redirection() {
            return Err(anyhow!("Authentication failed (status {})", status));
        }
        return Err(anyhow!("Authentication failed - invalid credentials"));
    }

    let area_resp = temp_client
        .get(&format!("{}/area/vitrine", site_url.trim_end_matches('/')))
        .send()
        .await?;

    if !area_resp.status().is_success() {
        return Err(anyhow!("Authentication failed - cannot access member area"));
    }

    let cookies_header = area_resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| {
            if s.contains("app_v4_session=") {
                let start = s.find("app_v4_session=")? + 15;
                let end = s[start..].find(';').map(|e| e + start).unwrap_or(s.len());
                Some(s[start..end].to_string())
            } else {
                None
            }
        })
        .last();

    let session_cookie = cookies_header
        .ok_or_else(|| anyhow!("Could not extract session cookie after login"))?;

    let client = build_client(&session_cookie, site_url)?;

    Ok(CademiSession {
        cookies: session_cookie,
        site_url: site_url.trim_end_matches('/').to_string(),
        client,
    })
}

pub async fn validate_token(session: &CademiSession) -> anyhow::Result<bool> {
    let url = format!("{}/area/vitrine", session.site_url);
    let resp = session.client.get(&url).send().await?;

    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &CademiSession) -> anyhow::Result<Vec<CademiCourse>> {
    let url = format!("{}/area/vitrine", session.site_url);
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

    let mut courses = Vec::new();
    let re = regex::Regex::new(r#"/area/produto/(\d+)"#)?;

    for cap in re.captures_iter(&body_text) {
        let id = cap[1].to_string();

        if courses.iter().any(|c: &CademiCourse| c.id == id) {
            continue;
        }

        let name_re = regex::Regex::new(&format!(
            r#"/area/produto/{}[^>]*>([^<]+)"#,
            regex::escape(&id)
        ))?;

        let name = name_re
            .captures(&body_text)
            .and_then(|c| Some(c[1].trim().to_string()))
            .unwrap_or_else(|| format!("Course {}", id));

        courses.push(CademiCourse { id, name });
    }

    Ok(courses)
}

pub async fn get_modules(
    session: &CademiSession,
    course_id: &str,
) -> anyhow::Result<Vec<CademiModule>> {
    let url = format!("{}/area/produto/{}", session.site_url, course_id);
    let resp = session.client.get(&url).send().await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_modules returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let mut modules = Vec::new();

    let section_re = regex::Regex::new(
        r#"(?s)data-secao-id="(\d+)"[^>]*>.*?<[^>]*class="[^"]*section-title[^"]*"[^>]*>([^<]+)"#
    )?;

    let lesson_re = regex::Regex::new(
        r#"/area/produto/item/(\d+)[^>]*>([^<]+)"#
    )?;

    let sections: Vec<(String, String, usize)> = section_re
        .captures_iter(&body_text)
        .enumerate()
        .map(|(i, cap)| (cap[1].to_string(), cap[2].trim().to_string(), i))
        .collect();

    if sections.is_empty() {
        let mut lessons = Vec::new();
        for (i, cap) in lesson_re.captures_iter(&body_text).enumerate() {
            let lesson_id = cap[1].to_string();
            let lesson_name = cap[2].trim().to_string();

            if lessons.iter().any(|l: &CademiLesson| l.id == lesson_id) {
                continue;
            }

            lessons.push(CademiLesson {
                id: lesson_id,
                name: lesson_name,
                order: i as i64,
            });
        }

        if !lessons.is_empty() {
            modules.push(CademiModule {
                id: "0".to_string(),
                name: "Content".to_string(),
                order: 0,
                lessons,
            });
        }
    } else {
        for (section_id, section_name, idx) in &sections {
            let section_start = body_text
                .find(&format!("data-secao-id=\"{}\"", section_id))
                .unwrap_or(0);

            let section_end = if idx + 1 < sections.len() {
                body_text
                    .find(&format!("data-secao-id=\"{}\"", sections[idx + 1].0))
                    .unwrap_or(body_text.len())
            } else {
                body_text.len()
            };

            let section_html = &body_text[section_start..section_end];

            let mut lessons = Vec::new();
            for (i, cap) in lesson_re.captures_iter(section_html).enumerate() {
                let lesson_id = cap[1].to_string();
                let lesson_name = cap[2].trim().to_string();

                if lessons.iter().any(|l: &CademiLesson| l.id == lesson_id) {
                    continue;
                }

                lessons.push(CademiLesson {
                    id: lesson_id,
                    name: lesson_name,
                    order: i as i64,
                });
            }

            modules.push(CademiModule {
                id: section_id.clone(),
                name: section_name.clone(),
                order: *idx as i64,
                lessons,
            });
        }
    }

    Ok(modules)
}

pub async fn get_lesson_detail(
    session: &CademiSession,
    lesson_id: &str,
) -> anyhow::Result<CademiLessonDetail> {
    let url = format!("{}/area/produto/item/{}", session.site_url, lesson_id);
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

    let title_re = regex::Regex::new(r#"<title>([^<]+)</title>"#)?;
    let name = title_re
        .captures(&body_text)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default();

    let mut video_url: Option<String> = None;

    let panda_re = regex::Regex::new(r#"(?s)pandavideo[^>]*src="([^"]+)""#)?;
    if let Some(cap) = panda_re.captures(&body_text) {
        video_url = Some(cap[1].to_string());
    }

    if video_url.is_none() {
        let iframe_re = regex::Regex::new(r#"<iframe[^>]+src="([^"]+)"[^>]*>"#)?;
        for cap in iframe_re.captures_iter(&body_text) {
            let src = &cap[1];
            if src.contains("pandavideo")
                || src.contains("youtube")
                || src.contains("vimeo")
                || src.contains("player")
            {
                video_url = Some(src.to_string());
                break;
            }
        }
    }

    if video_url.is_none() {
        let video_re = regex::Regex::new(r#"<video[^>]+src="([^"]+)""#)?;
        if let Some(cap) = video_re.captures(&body_text) {
            video_url = Some(cap[1].to_string());
        }
    }

    if video_url.is_none() {
        let source_re = regex::Regex::new(r#"<source[^>]+src="([^"]+\.(?:mp4|m3u8)[^"]*)""#)?;
        if let Some(cap) = source_re.captures(&body_text) {
            video_url = Some(cap[1].to_string());
        }
    }

    let mut description: Option<String> = None;
    let article_re = regex::Regex::new(r#"(?s)<article[^>]*>(.*?)</article>"#)?;
    if let Some(cap) = article_re.captures(&body_text) {
        let text = cap[1].trim();
        if !text.is_empty() {
            description = Some(text.to_string());
        }
    }
    if description.is_none() {
        let content_re = regex::Regex::new(r#"(?s)class="conteudo-aula"[^>]*>(.*?)</div>"#)?;
        if let Some(cap) = content_re.captures(&body_text) {
            let text = cap[1].trim();
            if !text.is_empty() {
                description = Some(text.to_string());
            }
        }
    }

    Ok(CademiLessonDetail {
        id: lesson_id.to_string(),
        name,
        video_url,
        description,
    })
}

fn extract_input_value(html: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"name="{}"\s+value="([^"]*)""#, regex::escape(name));
    let re = regex::Regex::new(&pattern).ok()?;
    if let Some(cap) = re.captures(html) {
        return Some(cap[1].to_string());
    }

    let pattern2 = format!(r#"value="([^"]*)"\s+name="{}""#, regex::escape(name));
    let re2 = regex::Regex::new(&pattern2).ok()?;
    if let Some(cap) = re2.captures(html) {
        return Some(cap[1].to_string());
    }

    None
}

pub async fn save_session(session: &CademiSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        cookies: session.cookies.clone(),
        site_url: session.site_url.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[cademi] session saved for {}", session.site_url);
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<CademiSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.cookies, &saved.site_url)?;

    tracing::info!("[cademi] session loaded for {}", saved.site_url);

    Ok(Some(CademiSession {
        cookies: saved.cookies,
        site_url: saved.site_url,
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
