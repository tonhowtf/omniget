use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";
const API_BASE: &str = "https://api.estrategiaconcursos.com.br/api/aluno";

#[derive(Clone)]
pub struct EstrategiaConcursosSession {
    pub token: String,
    pub client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub token: String,
    pub saved_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstrategiaConcursosCourse {
    pub id: i64,
    pub name: String,
    pub total_aulas: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstrategiaConcursosModule {
    pub id: String,
    pub name: String,
    pub order: i64,
    pub lessons: Vec<EstrategiaConcursosLesson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstrategiaConcursosLesson {
    pub id: i64,
    pub name: String,
    pub order: i64,
    pub is_disponivel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstrategiaConcursosLessonDetail {
    pub id: i64,
    pub name: String,
    pub videos: Vec<EstrategiaConcursosVideo>,
    pub pdf_url: Option<String>,
    pub pdf_grifado_url: Option<String>,
    pub pdf_simplificado_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstrategiaConcursosVideo {
    pub quality: String,
    pub url: String,
}

fn build_client(token: &str) -> anyhow::Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    headers.insert(
        "Cookie",
        HeaderValue::from_str(&format!("PHPSESSID={}; __Secure-SID={}", token, token))?,
    );
    headers.insert("Accept", HeaderValue::from_static("application/json"));
    headers.insert("Personificado", HeaderValue::from_static("false"));
    headers.insert(
        "Origin",
        HeaderValue::from_static("https://www.estrategiaconcursos.com.br"),
    );
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://www.estrategiaconcursos.com.br/"),
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
    Ok(data_dir.join("omniget").join("estrategia_concursos_session.json"))
}

pub async fn validate_token(session: &EstrategiaConcursosSession) -> anyhow::Result<bool> {
    let resp = session
        .client
        .get(format!("{}/perfil/detalhes", API_BASE))
        .send()
        .await?;

    Ok(resp.status().is_success())
}

pub async fn list_courses(session: &EstrategiaConcursosSession) -> anyhow::Result<Vec<EstrategiaConcursosCourse>> {
    let resp = session
        .client
        .get(format!("{}/curso", API_BASE))
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

    let concursos = body
        .get("data")
        .and_then(|d| d.get("concursos"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for concurso in &concursos {
        let cursos = concurso
            .get("cursos")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for curso in &cursos {
            let id = curso
                .get("id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let name = curso
                .get("nome")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let total_aulas = curso
                .get("total_aulas")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            courses.push(EstrategiaConcursosCourse {
                id,
                name,
                total_aulas,
            });
        }
    }

    Ok(courses)
}

pub async fn get_course_content(
    session: &EstrategiaConcursosSession,
    course_id: i64,
) -> anyhow::Result<Vec<EstrategiaConcursosModule>> {
    let resp = session
        .client
        .get(format!("{}/curso/{}", API_BASE, course_id))
        .send()
        .await?;

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

    let aulas = body
        .get("data")
        .and_then(|d| d.get("aulas"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut lessons = Vec::new();

    for (i, aula) in aulas.iter().enumerate() {
        let id = aula
            .get("id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let name = aula
            .get("nome")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let is_disponivel = aula
            .get("is_disponivel")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        lessons.push(EstrategiaConcursosLesson {
            id,
            name,
            order: i as i64,
            is_disponivel,
        });
    }

    let module = EstrategiaConcursosModule {
        id: course_id.to_string(),
        name: "Aulas".to_string(),
        order: 0,
        lessons,
    };

    Ok(vec![module])
}

pub async fn get_lesson_detail(
    session: &EstrategiaConcursosSession,
    lesson_id: i64,
) -> anyhow::Result<EstrategiaConcursosLessonDetail> {
    let resp = session
        .client
        .get(format!("{}/aula/{}", API_BASE, lesson_id))
        .send()
        .await?;

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

    let data = body.get("data").unwrap_or(&body);

    let name = data
        .get("nome")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut videos = Vec::new();

    let videos_arr = data
        .get("videos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for video in &videos_arr {
        let resolucoes = video
            .get("resolucoes")
            .and_then(|v| v.as_object());

        if let Some(res_map) = resolucoes {
            let best = pick_best_quality(res_map);
            if let Some((quality, url)) = best {
                videos.push(EstrategiaConcursosVideo {
                    quality,
                    url,
                });
            }
        }
    }

    let pdf_url = data
        .get("pdf")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let pdf_grifado_url = data
        .get("pdf_grifado")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let pdf_simplificado_url = data
        .get("pdf_simplificado")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    Ok(EstrategiaConcursosLessonDetail {
        id: lesson_id,
        name,
        videos,
        pdf_url,
        pdf_grifado_url,
        pdf_simplificado_url,
    })
}

fn pick_best_quality(resolucoes: &serde_json::Map<String, serde_json::Value>) -> Option<(String, String)> {
    let priority = ["720", "480", "360", "1080", "240"];

    for q in &priority {
        if let Some(url) = resolucoes.get(*q).and_then(|v| v.as_str()) {
            if !url.is_empty() {
                return Some((q.to_string(), url.to_string()));
            }
        }
    }

    resolucoes.iter().next().and_then(|(k, v)| {
        v.as_str().map(|u| (k.clone(), u.to_string()))
    })
}

pub async fn save_session(session: &EstrategiaConcursosSession) -> anyhow::Result<()> {
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
    tracing::info!("[estrategia_concursos] session saved");
    Ok(())
}

pub async fn load_session() -> anyhow::Result<Option<EstrategiaConcursosSession>> {
    let path = session_file_path()?;
    let json = match tokio::fs::read_to_string(&path).await {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };

    let saved: SavedSession = serde_json::from_str(&json)?;
    let client = build_client(&saved.token)?;

    tracing::info!("[estrategia_concursos] session loaded");

    Ok(Some(EstrategiaConcursosSession {
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
