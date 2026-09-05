use omniget_core::core::tools::{ryd, sponsorblock};

use super::err;

#[tauri::command]
pub async fn tool_sponsorblock(url: String, categories: Option<Vec<String>>) -> Result<sponsorblock::SponsorResult, String> {
    sponsorblock::segments(&url, &categories.unwrap_or_default()).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ryd(url: String) -> Result<ryd::Votes, String> {
    ryd::votes(&url).await.map_err(err)
}

#[tauri::command]
pub fn tool_yt_video_id(url: String) -> Option<String> {
    sponsorblock::video_id(&url)
}

/// Baixa uma URL simples (thumbnail, frame) para um arquivo.
#[tauri::command]
pub async fn tool_save_url(url: String, dest: String) -> Result<String, String> {
    let client = omniget_core::core::tools::client().map_err(err)?;
    let resp = client.get(&url).send().await.map_err(err)?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(err)?;
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    tokio::fs::write(&dest, &bytes).await.map_err(err)?;
    Ok(dest)
}
