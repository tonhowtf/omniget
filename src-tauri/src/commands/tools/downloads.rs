use omniget_core::core::tools::{aria2, manifest_dl};

use super::{err, progress};

#[tauri::command]
pub async fn tool_aria2_status() -> aria2::Aria2Status {
    aria2::status().await
}

#[tauri::command]
pub async fn tool_aria2_download(
    app: tauri::AppHandle,
    opts: aria2::Aria2Options,
) -> Result<aria2::Aria2Result, String> {
    aria2::download(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_manifest_download(
    app: tauri::AppHandle,
    opts: manifest_dl::ManifestOptions,
) -> Result<manifest_dl::ManifestResult, String> {
    manifest_dl::download(opts, progress(&app))
        .await
        .map_err(err)
}
