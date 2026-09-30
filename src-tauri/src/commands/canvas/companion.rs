//! Tauri commands of Omni as the canvas companion. Logic and the
//! `canvas://companion` event: `crate::canvas::companion`.

use omniget_core::core::canvas::companion::{CompanionEntry, CompanionSettings};
use serde_json::Value;
use tauri::AppHandle;

use crate::canvas::companion;

fn ready(app: &AppHandle) {
    crate::canvas::doc::host().bind(app);
    companion::start(app);
}

/// Current settings (also starts the watcher).
#[tauri::command]
pub async fn canvas_companion_settings_get(app: AppHandle) -> Result<CompanionSettings, String> {
    ready(&app);
    Ok(companion::settings())
}

#[tauri::command]
pub async fn canvas_companion_settings_set(
    app: AppHandle,
    settings: CompanionSettings,
) -> Result<CompanionSettings, String> {
    ready(&app);
    Ok(companion::set_settings(settings))
}

/// Newest first. `workspaceId` filters; dismissed entries only when asked.
#[tauri::command]
pub async fn canvas_companion_history(
    app: AppHandle,
    workspace_id: Option<String>,
    limit: Option<usize>,
    include_dismissed: Option<bool>,
) -> Result<Vec<CompanionEntry>, String> {
    ready(&app);
    Ok(companion::history(
        workspace_id.as_deref(),
        limit.unwrap_or(40),
        include_dismissed.unwrap_or(false),
    ))
}

#[tauri::command]
pub async fn canvas_companion_dismiss(app: AppHandle, id: String) -> Result<bool, String> {
    ready(&app);
    Ok(companion::dismiss(&id))
}

/// Sends suggestion `index` (0-2, 3 = the direct command) or `text` of
/// entry `id` to its agent (paste + Enter), then dismisses the entry.
#[tauri::command]
pub async fn canvas_companion_act(
    app: AppHandle,
    id: String,
    index: Option<usize>,
    text: Option<String>,
) -> Result<Value, String> {
    ready(&app);
    tauri::async_runtime::spawn_blocking(move || companion::act(&id, index, text))
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

/// "Ask Omni": a summary of `nodeId` right now.
#[tauri::command]
pub async fn canvas_companion_summarize(
    app: AppHandle,
    node_id: String,
) -> Result<CompanionEntry, String> {
    ready(&app);
    companion::summarize_node(&app, &node_id).await
}

/// Installed local (Ollama) chat models, smallest first, for the settings.
#[tauri::command]
pub async fn canvas_companion_models(app: AppHandle) -> Result<Vec<String>, String> {
    ready(&app);
    let url = companion::settings().ollama_url;
    let mut m = omniget_core::core::canvas::companion::ollama_models(&url)
        .await
        .unwrap_or_default();
    m.sort_by_key(|(_, s)| *s);
    Ok(m.into_iter().map(|(n, _)| n).collect())
}

/// A free question to Omni about a workspace ("what is B doing?", "note
/// that ...", "summarize my notes"). `nodeId` puts that terminal first.
#[tauri::command]
pub async fn canvas_companion_ask(
    app: AppHandle,
    workspace_id: String,
    question: String,
    node_id: Option<String>,
) -> Result<Value, String> {
    ready(&app);
    companion::ask(&app, &workspace_id, &question, node_id.as_deref()).await
}

/// Omni's own floating window (⌘⇧O): `show` true/false, or toggle.
#[tauri::command]
pub async fn canvas_omni_window(app: AppHandle, show: Option<bool>) -> Result<bool, String> {
    crate::canvas::companion::start(&app);
    crate::canvas::companion::toggle_window(&app, show)
}
