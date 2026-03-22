use crate::external_url::ExternalUrlEvent;

#[tauri::command]
pub async fn register_external_frontend(
    app: tauri::AppHandle,
) -> Result<Vec<ExternalUrlEvent>, String> {
    Ok(crate::external_url::register_frontend(&app).await)
}
