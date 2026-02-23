#[tauri::command]
pub fn get_rate_limit_stats() -> serde_json::Value {
    crate::core::ytdlp::get_rate_limit_stats()
}
