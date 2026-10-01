use std::collections::HashMap;

use omniget_core::core::canvas::export::{self, ExportOptions, ImportResult, Partitura, Selection};
use omniget_core::core::canvas::layout::{self, ArrangeMode};
use omniget_core::core::canvas::model::{
    Point, Snapshot, Workspace, WorkspaceInput, WorkspaceSummary,
};
use omniget_core::core::canvas::notes::{self, NoteContent, NoteSummary};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::{ApplyResult, CanvasStore, ORIGIN_USER};
use serde_json::{Map, Value};
use tauri::AppHandle;

use crate::canvas::doc::host;

fn store(app: &AppHandle) -> std::sync::Arc<CanvasStore> {
    host().bind(app).store().clone()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

#[tauri::command]
pub async fn canvas_workspaces(app: AppHandle) -> Result<Vec<WorkspaceSummary>, String> {
    let s = store(&app);
    blocking(move || Ok(s.workspaces())).await
}

#[tauri::command]
pub async fn canvas_workspace_create(
    app: AppHandle,
    input: WorkspaceInput,
) -> Result<Workspace, String> {
    let s = store(&app);
    blocking(move || s.create_workspace(input)).await
}

#[tauri::command]
pub async fn canvas_workspace_update(
    app: AppHandle,
    workspace_id: String,
    patch: Map<String, Value>,
) -> Result<Workspace, String> {
    canvas_workspace_update_with_files(app, workspace_id, patch)
        .await
        .map(|(ws, _)| ws)
}

pub async fn canvas_workspace_update_with_files(
    app: AppHandle,
    workspace_id: String,
    mut patch: Map<String, Value>,
) -> Result<(Workspace, Option<Value>), String> {
    use super::workspace_meta as meta;
    let s = store(&app);
    blocking(move || {
        let work = meta::take_agent_patch(&workspace_id, &mut patch)?;
        let ws = if patch.is_empty() {
            s.workspace(&workspace_id)?
        } else {
            s.update_workspace(&workspace_id, patch)?
        };
        let files = meta::run_agent_patch(&ws, work)?;
        Ok((ws, files))
    })
    .await
}

#[tauri::command]
pub async fn canvas_workspace_delete(app: AppHandle, workspace_id: String) -> Result<(), String> {
    let s = store(&app);
    blocking(move || s.delete_workspace(&workspace_id)).await
}

#[tauri::command]
pub async fn canvas_load(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
) -> Result<Snapshot, String> {
    let s = store(&app);
    blocking(move || s.snapshot(&workspace_id, &floor_id)).await
}

#[tauri::command]
pub async fn canvas_apply(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    base_rev: Option<u64>,
    ops: Vec<Op>,
    origin: Option<String>,
) -> Result<ApplyResult, String> {
    let s = store(&app);
    let origin = origin.unwrap_or_else(|| ORIGIN_USER.to_string());
    blocking(move || s.apply(&workspace_id, &floor_id, ops, &origin, base_rev)).await
}

#[tauri::command]
pub async fn canvas_flush(app: AppHandle) -> Result<(), String> {
    let s = store(&app);
    blocking(move || s.flush()).await
}

#[tauri::command]
pub async fn canvas_note_read(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_id: String,
) -> Result<NoteContent, String> {
    let s = store(&app);
    blocking(move || notes::read(&s, &workspace_id, &floor_id, &node_id)).await
}

#[tauri::command]
pub async fn canvas_note_write(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_id: String,
    text: String,
    if_revision: Option<String>,
    origin: Option<String>,
) -> Result<NoteContent, String> {
    let s = store(&app);
    let origin = origin.unwrap_or_else(|| ORIGIN_USER.to_string());
    blocking(move || {
        notes::write(
            &s,
            &workspace_id,
            &floor_id,
            &node_id,
            &text,
            if_revision.as_deref(),
            &origin,
        )
    })
    .await
}

#[tauri::command]
pub async fn canvas_note_move(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_id: String,
    dest: String,
) -> Result<NoteContent, String> {
    let s = store(&app);
    blocking(move || notes::move_to(&s, &workspace_id, &floor_id, &node_id, &dest, ORIGIN_USER))
        .await
}

#[tauri::command]
pub async fn canvas_note_list(
    app: AppHandle,
    workspace_id: String,
) -> Result<Vec<NoteSummary>, String> {
    let s = store(&app);
    blocking(move || notes::list(&s, &workspace_id)).await
}

#[tauri::command]
pub async fn canvas_layout_free_spot(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    w: f64,
    h: f64,
    near: Option<Point>,
) -> Result<Point, String> {
    let s = store(&app);
    blocking(move || {
        s.read(&workspace_id, &floor_id, |d| {
            layout::free_spot_in(d, w, h, near, &[])
        })
    })
    .await
}

#[tauri::command]
pub async fn canvas_layout_arrange(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    ids: Vec<String>,
    mode: ArrangeMode,
    origin: Option<String>,
) -> Result<ApplyResult, String> {
    let s = store(&app);
    let origin = origin.unwrap_or_else(|| ORIGIN_USER.to_string());
    blocking(move || {
        let ops = s.read(&workspace_id, &floor_id, |d| {
            layout::arrange_ops(d, &layout::arrange(d, &ids, mode))
        })?;
        s.apply(&workspace_id, &floor_id, ops, &origin, None)
    })
    .await
}

#[tauri::command]
pub async fn canvas_export_selection(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    selection: Selection,
    options: ExportOptions,
) -> Result<Partitura, String> {
    let s = store(&app);
    blocking(move || export::export_from_store(&s, &workspace_id, &floor_id, &selection, &options))
        .await
}

#[tauri::command]
pub async fn canvas_import_partitura(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    partitura: Partitura,
    at: Point,
    origin: Option<String>,
    role_map: Option<HashMap<String, String>>,
) -> Result<ImportResult, String> {
    let s = store(&app);
    let origin = origin.unwrap_or_else(|| ORIGIN_USER.to_string());
    blocking(move || {
        export::import_partitura(
            &s,
            &workspace_id,
            &floor_id,
            &partitura,
            at,
            &origin,
            &role_map.unwrap_or_default(),
        )
    })
    .await
}
