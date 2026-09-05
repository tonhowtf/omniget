//! Comandos da categoria X / Twitter (estudo 67).

use omniget_core::core::tools::x::{
    self, archive, bookmarks, follows, grok, media, profile, search, thread, XPost, XUser,
};
use serde::Serialize;

use super::{err, progress};

#[derive(Debug, Serialize)]
pub struct XSession {
    pub logged_in: bool,
    pub user_id: Option<String>,
    pub user: Option<XUser>,
    pub query_ids_cached: usize,
    pub query_ids_age_secs: Option<i64>,
}

#[tauri::command]
pub async fn tool_x_session() -> Result<XSession, String> {
    let client = x::client::XClient::new().map_err(err)?;
    let logged_in = client.authed();
    let user_id = client.user_id();
    let user = if logged_in {
        follows::me(&client).await.ok()
    } else {
        None
    };
    Ok(XSession {
        logged_in,
        user_id,
        user,
        query_ids_cached: x::query_ids::cached_count(),
        query_ids_age_secs: x::query_ids::cache_age_secs(),
    })
}

#[tauri::command]
pub async fn tool_x_query_ids_refresh() -> Result<usize, String> {
    let client = x::client::XClient::new().map_err(err)?;
    x::query_ids::refresh(&client.http, client.cookie())
        .await
        .map_err(err)
}

#[tauri::command]
pub fn tool_x_cancel(job: String) {
    x::cancel(&job);
}

#[tauri::command]
pub async fn tool_x_post(input: String) -> Result<XPost, String> {
    let id = x::post_id_from(&input)
        .ok_or_else(|| format!("nao reconheci um post do X em: {}", input))?;
    x::fx::status(&id).await.map_err(err)
}

#[tauri::command]
pub async fn tool_x_thread(input: String) -> Result<thread::Thread, String> {
    thread::unroll(&input).await.map_err(err)
}

#[tauri::command]
pub fn tool_x_export_posts(
    posts: Vec<XPost>,
    format: String,
    dest: String,
    title: String,
) -> Result<String, String> {
    x::export::write_posts(&posts, &format, std::path::Path::new(&dest), &title).map_err(err)
}

#[tauri::command]
pub fn tool_x_export_users(
    users: Vec<XUser>,
    format: String,
    dest: String,
) -> Result<String, String> {
    x::export::write_users(&users, &format, std::path::Path::new(&dest)).map_err(err)
}

#[tauri::command]
pub fn tool_x_render_posts(
    posts: Vec<XPost>,
    format: String,
    title: String,
) -> Result<String, String> {
    Ok(match format.as_str() {
        "md" | "markdown" => x::export::posts_markdown(&title, &posts),
        "html" => x::export::posts_html(&title, &posts),
        "txt" | "text" => x::export::posts_text(&posts),
        "csv" => x::export::posts_csv(&posts),
        _ => serde_json::to_string_pretty(&posts).map_err(err)?,
    })
}

#[tauri::command]
pub async fn tool_x_profile(
    input: String,
    limit: Option<usize>,
    with_replies: Option<bool>,
) -> Result<profile::ProfileReport, String> {
    profile::analyze(&input, limit.unwrap_or(200), with_replies.unwrap_or(false))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_x_profile_lookup(input: String) -> Result<XUser, String> {
    let handle = x::handle_from(&input)
        .ok_or_else(|| format!("nao reconheci um perfil do X em: {}", input))?;
    x::fx::profile(&handle).await.map_err(err)
}

#[tauri::command]
pub async fn tool_x_media(
    app: tauri::AppHandle,
    input: String,
    dest: String,
    limit: Option<usize>,
    photos: Option<bool>,
    videos: Option<bool>,
) -> Result<media::MediaResult, String> {
    media::download_profile(
        &input,
        &dest,
        limit.unwrap_or(0),
        photos.unwrap_or(true),
        videos.unwrap_or(true),
        progress(&app),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn tool_x_media_posts(
    app: tauri::AppHandle,
    posts: Vec<XPost>,
    dest: String,
    job: String,
) -> Result<media::MediaResult, String> {
    x::clear_cancel(&job);
    media::download_posts(
        &posts,
        std::path::Path::new(&dest),
        true,
        true,
        &job,
        &progress(&app),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn tool_x_search(
    query: String,
    feed: Option<String>,
    cursor: Option<String>,
) -> Result<search::SearchPage, String> {
    search::search(
        &query,
        feed.as_deref().unwrap_or("latest"),
        cursor.as_deref(),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn tool_x_trends() -> Result<Vec<x::fx::Trend>, String> {
    search::trends().await.map_err(err)
}

#[tauri::command]
pub async fn tool_x_bookmarks_export(
    app: tauri::AppHandle,
    dest: String,
    formats: Vec<String>,
    with_media: Option<bool>,
    max: Option<usize>,
) -> Result<bookmarks::BookmarksResult, String> {
    bookmarks::export(
        &dest,
        &formats,
        with_media.unwrap_or(false),
        max.unwrap_or(0),
        progress(&app),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn tool_x_follows_audit(
    app: tauri::AppHandle,
    limit: Option<usize>,
) -> Result<follows::Audit, String> {
    follows::audit(limit.unwrap_or(0), progress(&app))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_x_unfollow(
    app: tauri::AppHandle,
    ids: Vec<String>,
    min_delay: Option<u64>,
    max_delay: Option<u64>,
    daily_cap: Option<usize>,
) -> Result<follows::UnfollowResult, String> {
    follows::unfollow(
        &ids,
        min_delay.unwrap_or(15),
        max_delay.unwrap_or(40),
        daily_cap.unwrap_or(100),
        progress(&app),
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub fn tool_x_whitelist_get() -> Vec<String> {
    follows::whitelist()
}

#[tauri::command]
pub fn tool_x_whitelist_set(handles: Vec<String>) -> Result<Vec<String>, String> {
    follows::set_whitelist(&handles).map_err(err)
}

#[tauri::command]
pub async fn tool_x_archive_open(path: String) -> Result<archive::ArchiveSummary, String> {
    tokio::task::spawn_blocking(move || archive::open(&path))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn tool_x_archive_export(
    path: String,
    dest: String,
    what: String,
    format: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || archive::export(&path, &dest, &what, &format))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub fn tool_x_grok_config() -> grok::GrokConfigView {
    grok::view()
}

#[tauri::command]
pub fn tool_x_grok_config_set(
    xai_key: Option<String>,
    xai_model: Option<String>,
    x_model: Option<String>,
) -> Result<grok::GrokConfigView, String> {
    grok::set(xai_key, xai_model, x_model).map_err(err)
}

#[tauri::command]
pub async fn tool_x_grok_ask(request: grok::GrokRequest) -> Result<grok::GrokAnswer, String> {
    grok::ask(request).await.map_err(err)
}

/// Imagem remota como data URL (avatar e midia para o canvas do card).
#[tauri::command]
pub async fn tool_x_data_url(url: String) -> Result<String, String> {
    use base64::Engine;
    let client = omniget_core::core::tools::client().map_err(err)?;
    let resp = client
        .get(&url)
        .header("Referer", "https://x.com/")
        .send()
        .await
        .map_err(err)?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mime = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .to_string();
    let bytes = resp.bytes().await.map_err(err)?;
    Ok(format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// Grava um PNG (data URL) vindo do canvas.
#[tauri::command]
pub async fn tool_x_save_data_url(data_url: String, dest: String) -> Result<String, String> {
    use base64::Engine;
    let b64 = data_url.split(',').nth(1).ok_or("data URL invalida")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(err)?;
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    tokio::fs::write(&dest, bytes).await.map_err(err)?;
    Ok(dest)
}

/// Grava texto (relatorios JSON gerados na UI).
#[tauri::command]
pub async fn tool_x_write_text(dest: String, content: String) -> Result<String, String> {
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    tokio::fs::write(&dest, content).await.map_err(err)?;
    Ok(dest)
}
