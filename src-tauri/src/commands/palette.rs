use std::path::PathBuf;

use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use omniget_core::core::palette::{self, Extraction, LoadOutcome};

const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;

#[tauri::command]
pub async fn palette_projects_load() -> Result<LoadOutcome, String> {
    tokio::task::spawn_blocking(palette::load)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn palette_projects_save(data: Value) -> Result<(), String> {
    tokio::task::spawn_blocking(move || palette::save(&data))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn palette_extract(path: String, count: u32) -> Result<Extraction, String> {
    let path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || palette::extract_from_path(&path, count as usize))
        .await
        .map_err(|e| e.to_string())?
}

fn extension_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| matches!(ext.as_str(), "css" | "scss" | "json"))
}

#[tauri::command]
pub async fn palette_save_file(
    app: AppHandle,
    contents: String,
    file_name: String,
) -> Result<Option<String>, String> {
    if contents.len() > MAX_EXPORT_BYTES {
        return Err("The export is larger than 16 MB, so it was not saved.".to_string());
    }
    let name = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("palette.css")
        .to_string();
    let ext = extension_of(&name)
        .ok_or_else(|| "Only .css, .scss and .json files can be saved from Palette.".to_string())?;
    tokio::task::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .set_file_name(&name)
            .add_filter(ext.to_uppercase(), &[ext.as_str()])
            .blocking_save_file();
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked
            .into_path()
            .map_err(|e| format!("The chosen location is not a local file: {e}."))?;
        if path.is_dir() {
            return Err("The chosen location is a folder. Pick a file name instead.".to_string());
        }
        palette::store::write_atomic(&path, contents.as_bytes())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
