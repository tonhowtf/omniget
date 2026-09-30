//! Tauri commands of canvas routines. The model and the book are in
//! `omniget_core::core::canvas::routines`; the ticker and the runner in
//! `crate::canvas::routines` (event `canvas://routine`).

use omniget_core::core::canvas::routines::{self, Routine, RoutineInput};
use serde_json::json;
use tauri::{AppHandle, Emitter};

use crate::canvas::routines as host;

fn store(app: &AppHandle) -> std::sync::Arc<omniget_core::core::canvas::store::CanvasStore> {
    host::start(app);
    crate::canvas::doc::host().bind(app).store().clone()
}

#[tauri::command]
pub async fn canvas_routine_list(
    app: AppHandle,
    workspace_id: Option<String>,
) -> Result<Vec<Routine>, String> {
    let s = store(&app);
    Ok(routines::list(&s, workspace_id.as_deref()))
}

#[tauri::command]
pub async fn canvas_routine_save(app: AppHandle, input: RoutineInput) -> Result<Routine, String> {
    let s = store(&app);
    let r = routines::save(&s, input)?;
    host::emit_routine(&r);
    Ok(r)
}

#[tauri::command]
pub async fn canvas_routine_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<Routine, String> {
    let s = store(&app);
    let r = routines::set_enabled(&s, &id, enabled)?;
    host::emit_routine(&r);
    Ok(r)
}

#[tauri::command]
pub async fn canvas_routine_delete(app: AppHandle, id: String) -> Result<(), String> {
    let s = store(&app);
    routines::delete(&s, &id)?;
    let _ = app.emit(host::EVENT_ROUTINE, json!({ "id": id, "deleted": true }));
    Ok(())
}

#[tauri::command]
pub async fn canvas_routine_run_now(app: AppHandle, id: String) -> Result<Routine, String> {
    store(&app);
    host::run_now(&id).await
}

/// History tab: runs of one routine, or of every routine of a workspace
/// (finished `once` routines included), newest first.
#[tauri::command]
pub async fn canvas_routine_history(
    app: AppHandle,
    workspace_id: Option<String>,
    id: Option<String>,
) -> Result<Vec<routines::HistoryItem>, String> {
    let s = store(&app);
    if let Some(i) = id.as_deref() {
        routines::get(&s, i)?;
    }
    Ok(routines::history(
        &s,
        workspace_id.as_deref(),
        id.as_deref(),
    ))
}

/// Clears the history (one routine or a workspace); routines stay.
#[tauri::command]
pub async fn canvas_routine_history_clear(
    app: AppHandle,
    workspace_id: Option<String>,
    id: Option<String>,
) -> Result<usize, String> {
    let s = store(&app);
    let n = routines::clear_history(&s, workspace_id.as_deref(), id.as_deref())?;
    let _ = app.emit(
        host::EVENT_ROUTINE,
        json!({ "historyCleared": true, "workspaceId": workspace_id, "id": id }),
    );
    Ok(n)
}
