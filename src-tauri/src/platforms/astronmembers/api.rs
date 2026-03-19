use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";

#[derive(Clone)]
pub struct AstronSession {
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
pub struct AstronCourse {
    pub id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstronModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<AstronLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstronLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub url: String,
}

fn build_client(cookies: &str, site_url: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Cookie",
        HeaderValue::from_str(cookies)?,
    );
    headers.insert(
        "Referer",
        HeaderValue::from_str(&format!("{}/", site_url))?,
    );
    headers.insert("Accept", HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"));

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
    Ok(data_dir.join("omniget").join("astron_session.json"))
}

pub async fn authenticate(site_url: &str, login: &str, password: &str) -> anyhow::Result<AstronSession> {
    let temp_client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .user_agent(USER_AGENT)
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()?;

    let login_url = format!("{}/entrar", site_url);
    let resp = temp_client
        .post(&login_url)
        .form(&[("login", login), ("senha", password)])
        .send()
        .await?;

    if !resp.status().is_success() && !resp.status().is_redirection() {
        return Err(anyhow!("Authentication failed (status {})", resp.status()));
    }

    let mut cookie_strings = Vec::new();
    for cookie in resp.cookies() {
        cookie_strings.push(format!("{}={}", cookie.name(), cookie.value()));
    }
    let cookies = cookie_strings.join("; ");

    if cookies.is_empty() {
        return Err(anyhow!("No cookies received from login"));
    }

    let client = build_client(&cookies, site_url)?;

    Ok(AstronSession {
        cookies,
        site_url: site_url.to_string(),
        client,
    })
}

pub async fn validate_token(session: &AstronSession) -> anyhow::Result<bool> {
    let resp = session.client.get(&session.site_url).send().await?;
    let body = resp.text().await?;
    Ok(!body.contains("/entrar") || body.contains("box-slider-cursos"))
}

pub async fn list_courses(session: &AstronSession) -> anyhow::Result<Vec<AstronCourse>> {
    let resp = session.client.get(&session.site_url).send().await?;

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

    for line in body_text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("href=") && trimmed.contains("curso/") {
            if let Some(href) = extract_attr(trimmed, "href") {
                if href.contains("curso/") {
                    let slug = href.rsplit("curso/").next().unwrap_or("").trim_matches('/').to_string();
                    if !slug.is_empty() && !courses.iter().any(|c: &AstronCourse| c.slug == slug) {
                        let name = extract_text_content(trimmed)
                            .unwrap_or_else(|| slug.replace('-', " "));
                        courses.push(AstronCourse {
                            id: slug.clone(),
                            name,
                            slug,
                        });
                    }
                }
            }
        }
    }

    Ok(courses)
}

pub async fn get_course_content(
    session: &AstronSession,
    course_slug: &str,
) -> anyhow::Result<Vec<AstronModule>> {
    let url = format!("{}/curso/{}", session.site_url, course_slug);
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

    let mut modules = Vec::new();
    let mut current_module: Option<(String, Vec<AstronLesson>)> = None;
    let mut module_order = 0i64;
    let mut lesson_order = 0i64;

    for line in body_text.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<dt") {
            if let Some((name, lessons)) = current_module.take() {
                modules.push(AstronModule {
                    id: format!("mod-{}", module_order),
                    name,
                    order: module_order,
                    lessons,
                });
            }
            module_order += 1;
            lesson_order = 0;
            let mod_name = extract_text_content(trimmed)
                .unwrap_or_else(|| format!("Module {}", module_order));
            current_module = Some((mod_name, Vec::new()));
        }

        if trimmed.contains("aulabox") && trimmed.contains("href=") {
            if let Some(href) = extract_attr(trimmed, "href") {
                lesson_order += 1;
                let lesson_id = href.rsplit('/').next().unwrap_or(&href).to_string();
                let lesson_name = extract_text_content(trimmed)
                    .unwrap_or_else(|| format!("Lesson {}", lesson_order));
                let full_url = if href.starts_with("http") {
                    href
                } else {
                    format!("{}{}", session.site_url, href)
                };
                if let Some(ref mut m) = current_module {
                    m.1.push(AstronLesson {
                        id: lesson_id,
                        name: lesson_name,
                        order: lesson_order,
                        url: full_url,
                    });
                }
            }
        }
    }

    if let Some((name, lessons)) = current_module.take() {
        modules.push(AstronModule {
            id: format!("mod-{}", module_order),
            name,
            order: module_order,
            lessons,
        });
    }

    Ok(modules)
}

pub async fn get_lesson_video_url(
    session: &AstronSession,
    lesson_url: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let resp = session.client.get(lesson_url).send().await?;
    let body = resp.text().await?;

    let mut video_url = None;

    if let Some(pos) = body.find("streaming-video-url") {
        let rest = &body[pos..];
        if let Some(src) = extract_attr(rest, "src") {
            if src.contains("panda") {
                video_url = Some(convert_panda_url(&src));
            } else {
                video_url = Some(src);
            }
        }
    }

    if video_url.is_none() {
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.contains("<iframe") {
                if let Some(src) = extract_attr(trimmed, "src") {
                    if src.contains("youtube") || src.contains("vimeo") || src.contains("player") || src.contains("panda") || src.contains("embed") {
                        if src.contains("panda") {
                            video_url = Some(convert_panda_url(&src));
                        } else {
                            video_url = Some(src);
                        }
                        break;
                    }
                }
            }
        }
    }

    if video_url.is_none() {
        if let Some(pos) = body.find("__NEXT_DATA__") {
            let rest = &body[pos..];
            if let Some(start) = rest.find('{') {
                let json_str = &rest[start..];
                if let Some(end) = find_matching_brace(json_str) {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&json_str[..=end]) {
                        if let Some(url) = extract_video_from_next_data(&data) {
                            video_url = Some(url);
                        }
                    }
                }
            }
        }
    }

    let description = extract_lesson_description(&body);

    Ok((video_url, description))
}

fn extract_lesson_description(html: &str) -> Option<String> {
    if let Some(start) = html.find("class=\"conteudo-aula\"").or_else(|| html.find("class=\"lesson-content\"")).or_else(|| html.find("class=\"aula-conteudo\"")) {
        let rest = &html[start..];
        if let Some(tag_end) = rest.find('>') {
            let content = &rest[tag_end + 1..];
            if let Some(end) = content.find("</div>") {
                let text = content[..end].trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn convert_panda_url(url: &str) -> String {
    url.to_string()
}

fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_video_from_next_data(data: &serde_json::Value) -> Option<String> {
    if let Some(url) = data.get("videoUrl").and_then(|v| v.as_str()) {
        return Some(url.to_string());
    }
    if let Some(props) = data.get("props") {
        if let Some(page_props) = props.get("pageProps") {
            if let Some(url) = page_props.get("videoUrl").and_then(|v| v.as_str()) {
                return Some(url.to_string());
            }
        }
    }
    None
}

fn extract_attr(html: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    if let Some(pos) = html.find(&pattern) {
        let rest = &html[pos + pattern.len()..];
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    let pattern2 = format!("{}='", attr);
    if let Some(pos) = html.find(&pattern2) {
        let rest = &html[pos + pattern2.len()..];
        if let Some(end) = rest.find('\'') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn extract_text_content(html: &str) -> Option<String> {
    if let Some(pos) = html.rfind('>') {
        let rest = &html[pos + 1..];
        if let Some(end) = rest.find('<') {
            let text = rest[..end].trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

pub async fn save_session(session: &AstronSession) -> anyhow::Result<()> {
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
    tracing::info!("[astron] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<AstronSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.cookies, &saved.site_url)?;

    tracing::info!("[astron] session loaded");

    Ok(Some(AstronSession {
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
