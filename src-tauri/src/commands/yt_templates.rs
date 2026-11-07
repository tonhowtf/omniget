use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtTemplate {
    pub id: String,
    pub name: String,
    pub args: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YtTemplateStore {
    #[serde(default)]
    pub templates: Vec<YtTemplate>,
}

fn templates_path() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("yt_templates.json")
}

fn load() -> YtTemplateStore {
    crate::core::state_file::load_or_quarantine(&templates_path(), "yt-dlp templates")
        .unwrap_or_default()
}

fn save(store: &YtTemplateStore) -> Result<(), String> {
    let path = templates_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let serialized = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, serialized).map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn yt_templates_list() -> Result<Vec<YtTemplate>, String> {
    Ok(load().templates)
}

#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    pub id: Option<String>,
    pub name: String,
    pub args: Vec<String>,
}

#[tauri::command]
pub fn yt_templates_save(request: SaveRequest) -> Result<YtTemplate, String> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err("Template name cannot be empty".to_string());
    }
    let now = now_ms();
    let mut store = load();
    let id = request.id.unwrap_or_else(|| format!("tpl-{}", now));
    let existing = store.templates.iter_mut().find(|t| t.id == id);
    let template = if let Some(t) = existing {
        t.name = name;
        t.args = request.args;
        t.updated_at_ms = now;
        t.clone()
    } else {
        let t = YtTemplate {
            id,
            name,
            args: request.args,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.templates.push(t.clone());
        t
    };
    save(&store)?;
    Ok(template)
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub id: String,
}

#[tauri::command]
pub fn yt_templates_delete(request: DeleteRequest) -> Result<(), String> {
    let mut store = load();
    let before = store.templates.len();
    store.templates.retain(|t| t.id != request.id);
    if store.templates.len() == before {
        return Err(format!("template not found: {}", request.id));
    }
    save(&store)
}
