use omniget_core::core::tools::{calameo, gallery, gdocs, slides};

use super::{err, progress};

#[tauri::command]
pub async fn tool_slideshare(app: tauri::AppHandle, url: String, dest: String) -> Result<slides::SlidesResult, String> {
    slides::download(&url, &dest, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub fn tool_gdocs_parse(url: String) -> Option<gdocs::GdocInfo> {
    gdocs::parse(&url)
}

#[tauri::command]
pub async fn tool_gdocs_download(app: tauri::AppHandle, url: String, format: String, dest: String) -> Result<String, String> {
    gdocs::download(&url, &format, &dest, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_calameo(app: tauri::AppHandle, url: String, dest: String) -> Result<calameo::CalameoResult, String> {
    calameo::download(&url, &dest, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_gallery_status() -> gallery::GalleryStatus {
    gallery::status().await
}

#[tauri::command]
pub async fn tool_gallery_install() -> Result<String, String> {
    gallery::install().await.map_err(err)
}

#[tauri::command]
pub async fn tool_gallery_download(app: tauri::AppHandle, url: String, dest: String, cookies_file: Option<String>) -> Result<gallery::GalleryResult, String> {
    gallery::download(&url, &dest, cookies_file.as_deref(), progress(&app)).await.map_err(err)
}
