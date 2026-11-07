use std::path::PathBuf;

use omniget_core::core::motion::{self, LottieFile, RecentTimeline, SavedFile, StageImage};

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("The task stopped unexpectedly: {e}"))?
}

fn data_dir() -> Result<PathBuf, String> {
    omniget_core::core::paths::app_data_dir()
        .ok_or_else(|| "The app data folder could not be found.".to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn motion_lottie_open(path: String) -> Result<LottieFile, String> {
    blocking(move || motion::read_lottie(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn motion_png_save(path: String, data_base64: String) -> Result<SavedFile, String> {
    blocking(move || motion::save_png(&path, &data_base64)).await
}

#[tauri::command]
pub async fn motion_timeline_save(path: String, json: String) -> Result<SavedFile, String> {
    blocking(move || motion::save_timeline(&path, &json)).await
}

#[tauri::command]
pub async fn motion_timeline_load(path: String) -> Result<String, String> {
    blocking(move || motion::load_timeline(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn motion_image_open(path: String) -> Result<StageImage, String> {
    blocking(move || motion::read_image(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn motion_recent_list() -> Result<Vec<RecentTimeline>, String> {
    blocking(|| motion::recent_list(&data_dir()?)).await
}

#[tauri::command]
pub async fn motion_recent_save(
    id: Option<String>,
    name: String,
    json: String,
) -> Result<Vec<RecentTimeline>, String> {
    blocking(move || motion::recent_save(&data_dir()?, id.as_deref(), &name, &json, now_ms())).await
}

#[tauri::command]
pub async fn motion_recent_remove(id: String) -> Result<Vec<RecentTimeline>, String> {
    blocking(move || motion::recent_remove(&data_dir()?, &id)).await
}
