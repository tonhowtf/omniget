//! Tauri commands of the partitura library (`<app_data>/canvas/partituras/`).
//! Logic lives in `omniget_core::core::canvas::partituras`.

use omniget_core::core::canvas::export::ImportResult;
use omniget_core::core::canvas::model::Point;
use omniget_core::core::canvas::partituras::{
    self, ImportPreview, LibraryImport, LibraryPartitura, PartituraSummary, SaveRequest,
    StampOptions,
};
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{Map, Value};
use tauri::AppHandle;

fn store(app: &AppHandle) -> std::sync::Arc<CanvasStore> {
    crate::canvas::routines::start(app);
    crate::canvas::doc::host().bind(app).store().clone()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

/// `scope`: `all` | `global` | a workspace id. `query` searches name,
/// summary, role names and workspace name.
#[tauri::command]
pub async fn canvas_partitura_list(
    app: AppHandle,
    scope: Option<String>,
    query: Option<String>,
) -> Result<Vec<PartituraSummary>, String> {
    let s = store(&app);
    blocking(move || Ok(partituras::list(&s, scope.as_deref(), query.as_deref()))).await
}

#[tauri::command]
pub async fn canvas_partitura_get(app: AppHandle, id: String) -> Result<LibraryPartitura, String> {
    let s = store(&app);
    blocking(move || partituras::get(&s, &id)).await
}

/// Creates a partitura from a selection, or replaces the layout of
/// `request.updateExisting` keeping its identity.
#[tauri::command]
pub async fn canvas_partitura_save(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    request: SaveRequest,
) -> Result<LibraryPartitura, String> {
    let s = store(&app);
    blocking(move || partituras::save(&s, &workspace_id, &floor_id, request)).await
}

/// "Edit…": name, summary, icon, color, scope, budget, permissions, setup.
#[tauri::command]
pub async fn canvas_partitura_edit(
    app: AppHandle,
    id: String,
    patch: Map<String, Value>,
) -> Result<LibraryPartitura, String> {
    let s = store(&app);
    blocking(move || partituras::edit(&s, &id, patch)).await
}

#[tauri::command]
pub async fn canvas_partitura_delete(app: AppHandle, id: String) -> Result<(), String> {
    let s = store(&app);
    blocking(move || partituras::delete(&s, &id)).await
}

/// Import review: commands that will run, role conflicts, hooks, warnings.
/// Give `id` (library) or `partitura` (a file being reviewed).
#[tauri::command]
pub async fn canvas_partitura_preview(
    app: AppHandle,
    workspace_id: String,
    id: Option<String>,
    partitura: Option<LibraryPartitura>,
) -> Result<ImportPreview, String> {
    let s = store(&app);
    blocking(move || {
        let p = partituras::resolve(&s, id.as_deref(), partitura)?;
        Ok(partituras::preview(&s, &workspace_id, &p))
    })
    .await
}

/// Stamps a partitura at `at` after the review (edited commands, role
/// choices). Everything arrives selected-ready in one batch.
#[tauri::command]
pub async fn canvas_partitura_import(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    id: Option<String>,
    partitura: Option<LibraryPartitura>,
    at: Point,
    options: Option<StampOptions>,
) -> Result<ImportResult, String> {
    let s = store(&app);
    blocking(move || {
        let p = partituras::resolve(&s, id.as_deref(), partitura)?;
        partituras::stamp(
            &s,
            &workspace_id,
            &floor_id,
            &p,
            at,
            &options.unwrap_or_default(),
        )
    })
    .await
}

/// Reads a `.json` partitura or bundle from disk (for the review sheet).
#[tauri::command]
pub async fn canvas_partitura_read_file(
    app: AppHandle,
    path: String,
) -> Result<Vec<LibraryPartitura>, String> {
    store(&app);
    blocking(move || partituras::read_file(std::path::Path::new(&path))).await
}

/// Library transfer: adds partituras, skipping ids already there.
#[tauri::command]
pub async fn canvas_partitura_add_to_library(
    app: AppHandle,
    items: Vec<LibraryPartitura>,
) -> Result<LibraryImport, String> {
    let s = store(&app);
    blocking(move || partituras::add_to_library(&s, items)).await
}

/// "Save to…" (one id) or "Export All…" (several ids → bundle).
#[tauri::command]
pub async fn canvas_partitura_export_file(
    app: AppHandle,
    ids: Vec<String>,
    path: String,
) -> Result<(), String> {
    let s = store(&app);
    blocking(move || partituras::export_file(&s, &ids, std::path::Path::new(&path))).await
}
