use omniget_core::core::tools::kdeconnect;

use super::err;

#[tauri::command]
pub async fn tool_kde_status() -> kdeconnect::KdeStatus {
    kdeconnect::status().await
}

#[tauri::command]
pub async fn tool_kde_share(device: String, kind: String, value: String) -> Result<String, String> {
    match kind.as_str() {
        "file" => kdeconnect::share_file(&device, &value).await,
        "url" => kdeconnect::share_url(&device, &value).await,
        _ => kdeconnect::share_text(&device, &value).await,
    }
    .map_err(err)
}

#[tauri::command]
pub async fn tool_kde_ping(device: String, message: Option<String>) -> Result<String, String> {
    kdeconnect::ping(&device, message.as_deref().unwrap_or("")).await.map_err(err)
}

#[tauri::command]
pub async fn tool_kde_refresh() -> Result<String, String> {
    kdeconnect::refresh().await.map_err(err)
}
