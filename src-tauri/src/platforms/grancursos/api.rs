use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const BASE_URL: &str = "https://www.grancursosonline.com.br";

#[derive(Clone)]
pub struct GranCursosSession {
    pub cookies: String,
    pub contract_id: Option<String>,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub cookies: String,
    pub contract_id: Option<String>,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GranCursosCourse {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GranCursosDiscipline {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GranCursosModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<GranCursosLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GranCursosLesson {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub video_id: Option<String>,
}

fn build_client(cookies: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Cookie",
        HeaderValue::from_str(cookies)?,
    );
    headers.insert("Accept", HeaderValue::from_static("application/json"));
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://www.grancursosonline.com.br/"),
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
    Ok(data_dir.join("omniget").join("grancursos_session.json"))
}

pub async fn validate_token(session: &GranCursosSession) -> anyhow::Result<bool> {
    let resp = session
        .client
        .post(format!("{}/aluno/api/assinatura/buscar-cursos", BASE_URL))
        .json(&serde_json::json!({"termo": ""}))
        .send()
        .await?;

    Ok(resp.status().is_success())
}

pub async fn search_courses(session: &GranCursosSession, query: &str) -> anyhow::Result<(Vec<GranCursosCourse>, Option<String>)> {
    let resp = session
        .client
        .post(format!("{}/aluno/api/assinatura/buscar-cursos", BASE_URL))
        .json(&serde_json::json!({"termo": query}))
        .send()
        .await?;

    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "search_courses returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let mut courses = Vec::new();
    let mut contract_id = None;

    let items = body
        .get("data")
        .or(Some(&body))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            body.get("cursos")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        });

    for item in &items {
        let id = item
            .get("codigo")
            .or_else(|| item.get("id"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let name = item
            .get("nome")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if contract_id.is_none() {
            contract_id = item
                .get("contrato_id")
                .or_else(|| item.get("contratoId"))
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty());
        }

        if id > 0 {
            courses.push(GranCursosCourse { id, name });
        }
    }

    Ok((courses, contract_id))
}

pub async fn list_courses(session: &GranCursosSession) -> anyhow::Result<Vec<GranCursosCourse>> {
    let (courses, _) = search_courses(session, "").await?;
    Ok(courses)
}

pub async fn get_disciplines(
    session: &GranCursosSession,
    course_id: i64,
) -> anyhow::Result<Vec<GranCursosDiscipline>> {
    let url = format!(
        "{}/aluno/curso/listar-conteudo-aula/codigo/{}/tipo/video",
        BASE_URL, course_id
    );

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_disciplines returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let items = body
        .get("data")
        .or(Some(&body))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            body.get("disciplinas")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        });

    let mut disciplines = Vec::new();

    for item in &items {
        let id = item
            .get("id")
            .or_else(|| item.get("codigo"))
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            })
            .unwrap_or_default();

        let name = item
            .get("nome")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !id.is_empty() {
            disciplines.push(GranCursosDiscipline { id, name });
        }
    }

    Ok(disciplines)
}

pub async fn get_discipline_content(
    session: &GranCursosSession,
    course_id: i64,
    discipline_id: &str,
) -> anyhow::Result<Vec<GranCursosModule>> {
    let url = format!(
        "{}/aluno/curso/listar-conteudo-aula/codigo/{}/tipo/video/disciplina/{}",
        BASE_URL, course_id, discipline_id
    );

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_discipline_content returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let items = body
        .get("data")
        .or(Some(&body))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            body.get("conteudos")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        });

    let mut modules = Vec::new();

    for (ci, content) in items.iter().enumerate() {
        let content_id = content
            .get("id")
            .or_else(|| content.get("codigo"))
            .map(|v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => format!("{}", ci),
            })
            .unwrap_or_else(|| format!("{}", ci));

        let content_name = content
            .get("nome")
            .or_else(|| content.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Content")
            .to_string();

        let aulas = content
            .get("aulas")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut lessons = Vec::new();

        for (ai, aula) in aulas.iter().enumerate() {
            let aula_id = aula
                .get("id")
                .or_else(|| aula.get("codigo"))
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => format!("{}-{}", ci, ai),
                })
                .unwrap_or_else(|| format!("{}-{}", ci, ai));

            let aula_name = aula
                .get("nome")
                .or_else(|| aula.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let video_id = aula
                .get("video_id")
                .or_else(|| aula.get("videoId"))
                .map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty());

            lessons.push(GranCursosLesson {
                id: aula_id,
                name: aula_name,
                order: ai as i64,
                video_id,
            });
        }

        modules.push(GranCursosModule {
            id: content_id,
            name: content_name,
            order: ci as i64,
            lessons,
        });
    }

    Ok(modules)
}

pub async fn get_video_url(
    session: &GranCursosSession,
    course_id: i64,
    video_id: &str,
    contract_id: &str,
) -> anyhow::Result<String> {
    let url = format!(
        "{}/aluno/sala-de-aula/video/co/{}/a/{}/c/{}",
        BASE_URL, course_id, video_id, contract_id
    );

    let resp = session.client.get(&url).send().await?;
    let status = resp.status();
    let body_text = resp.text().await?;

    if !status.is_success() {
        return Err(anyhow!(
            "get_video_url returned status {}: {}",
            status,
            &body_text[..body_text.len().min(300)]
        ));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)?;

    let sources = body
        .get("player")
        .and_then(|p| p.get("sources"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            body.get("sources")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        });

    for source in &sources {
        let file = source
            .get("file")
            .or_else(|| source.get("src"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !file.is_empty() {
            return Ok(file.to_string());
        }
    }

    let direct_url = body
        .get("url")
        .or_else(|| body.get("video_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !direct_url.is_empty() {
        return Ok(direct_url.to_string());
    }

    Err(anyhow!("No video URL found in response"))
}

pub async fn save_session(session: &GranCursosSession) -> anyhow::Result<()> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let saved = SavedSession {
        cookies: session.cookies.clone(),
        contract_id: session.contract_id.clone(),
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let json = serde_json::to_string_pretty(&saved)?;
    tokio::fs::write(&path, json).await?;
    tracing::info!("[grancursos] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<GranCursosSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.cookies)?;

    tracing::info!("[grancursos] session loaded");

    Ok(Some(GranCursosSession {
        cookies: saved.cookies,
        contract_id: saved.contract_id,
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
