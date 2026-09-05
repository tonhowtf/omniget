use std::sync::{Mutex, OnceLock};

use omniget_core::core::tools::{dupes, file_search, rename};
use serde::Serialize;

use super::{err, progress};

#[tauri::command]
pub async fn tool_dupes_scan(app: tauri::AppHandle, opts: dupes::DupesOptions) -> Result<dupes::DupesResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || dupes::scan(&opts, &p)).await.map_err(err)
}

#[derive(Serialize)]
pub struct DeleteOut {
    pub deleted: Vec<String>,
    pub failed: Vec<String>,
}

#[tauri::command]
pub fn tool_dupes_delete(paths: Vec<String>) -> DeleteOut {
    let (deleted, failed) = dupes::delete(&paths);
    DeleteOut { deleted, failed }
}

#[tauri::command]
pub fn tool_rename_plan(opts: rename::RenameOptions) -> Result<Vec<rename::RenamePlan>, String> {
    rename::plan(&opts)
}

#[derive(Serialize)]
pub struct RenameOut {
    pub renamed: usize,
    pub failed: Vec<String>,
}

#[tauri::command]
pub fn tool_rename_apply(plans: Vec<rename::RenamePlan>) -> RenameOut {
    let (renamed, failed) = rename::apply(&plans);
    RenameOut { renamed, failed }
}

#[tauri::command]
pub async fn tool_file_search_backend() -> file_search::SearchBackend {
    file_search::backend().await
}

#[tauri::command]
pub async fn tool_file_search(query: String, folder: Option<String>, limit: Option<usize>) -> Result<Vec<file_search::Hit>, String> {
    file_search::search(&query, folder.as_deref().unwrap_or(""), limit.unwrap_or(300)).await.map_err(err)
}

// ── Manter acordado (estudo 29, PowerToys Awake) ──
// Guard próprio, independente da opção "impedir suspensão durante downloads".
static AWAKE: OnceLock<Mutex<Option<keepawake::KeepAwake>>> = OnceLock::new();

#[tauri::command]
pub fn tool_awake_set(active: bool) -> Result<bool, String> {
    let cell = AWAKE.get_or_init(|| Mutex::new(None));
    let mut held = cell.lock().map_err(|_| "lock".to_string())?;
    if active && held.is_none() {
        let g = keepawake::Builder::default()
            .display(true)
            .idle(true)
            .sleep(true)
            .app_name("OmniGet")
            .reason("Tools: keep awake")
            .create()
            .map_err(|e| e.to_string())?;
        *held = Some(g);
    } else if !active {
        *held = None;
    }
    Ok(held.is_some())
}

#[tauri::command]
pub fn tool_awake_get() -> bool {
    AWAKE.get().and_then(|c| c.lock().ok()).map(|h| h.is_some()).unwrap_or(false)
}
