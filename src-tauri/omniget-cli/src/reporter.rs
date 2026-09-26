use anyhow::Result;
use std::path::PathBuf;

use omniget_core::core::dependencies::find_tool;
use omniget_core::core::paths::app_data_dir;

pub async fn find_yt_dlp() -> Result<PathBuf> {
    find_tool("yt-dlp")
        .await
        .ok_or_else(|| anyhow::anyhow!("yt-dlp not found in PATH or app data dir"))
}

pub fn default_output_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

pub fn default_cookie_path() -> Option<PathBuf> {
    let app_dir = app_data_dir()?;
    let cookie_file = app_dir.join("cookies").join("cookies.txt");
    if cookie_file.exists() {
        Some(cookie_file)
    } else {
        None
    }
}
