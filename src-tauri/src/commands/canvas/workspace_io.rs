use std::path::PathBuf;
use std::sync::Arc;

use omniget_core::core::canvas::model::Workspace;
use omniget_core::core::canvas::store::CanvasStore;
use omniget_core::core::canvas::workspace_io::{
    self as wio, BackupInfo, ExportOptions, ExportReport, ImportOptions, ImportPreview,
    ImportResult, RestoreDiff,
};
use tauri::AppHandle;

use crate::canvas::doc::host;

fn store(app: &AppHandle) -> Arc<CanvasStore> {
    host().bind(app).store().clone()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

fn path_arg(p: &str) -> Result<PathBuf, String> {
    let p = p.trim();
    if p.is_empty() {
        return Err("MISSING_ARGUMENT: path".into());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return Ok(home.join(rest));
        }
    }
    Ok(PathBuf::from(p))
}

pub fn close_workspace_terminals(s: &CanvasStore, ws: &str) {
    let Ok(w) = s.workspace(ws) else { return };
    for f in &w.floors {
        crate::canvas::floors::close_floor_terminals(s, ws, &f.id);
    }
}

pub async fn export(
    s: Arc<CanvasStore>,
    ws: String,
    path: String,
    opts: ExportOptions,
) -> Result<ExportReport, String> {
    let mut path = path_arg(&path)?;
    if path.extension().is_none() {
        path.set_extension(wio::ARCHIVE_EXTENSION);
    }
    blocking(move || wio::export_to(&s, &ws, &path, &opts)).await
}

pub async fn import_preview(s: Arc<CanvasStore>, path: String) -> Result<ImportPreview, String> {
    let path = path_arg(&path)?;
    blocking(move || {
        let a = wio::read_archive(&path)?;
        Ok(wio::preview(&s, &a))
    })
    .await
}

pub async fn import(
    s: Arc<CanvasStore>,
    path: String,
    opts: ImportOptions,
) -> Result<ImportResult, String> {
    let path = path_arg(&path)?;
    blocking(move || {
        let a = wio::read_archive(&path)?;
        wio::import(&s, &a, &opts)
    })
    .await
}

pub async fn restore(s: Arc<CanvasStore>, ws: String, backup: String) -> Result<Workspace, String> {
    blocking(move || {
        let running = running_terminals(&s, &ws);
        close_workspace_terminals(&s, &ws);
        let meta = wio::restore_backup(&s, &ws, &backup)?;
        for (floor, node) in running {
            let still = s
                .read(&ws, &floor, |d| {
                    d.node(&node).is_some_and(|n| n.is_terminal())
                })
                .unwrap_or(false);
            if still {
                crate::canvas::launch::spawn_ensure(&ws, &floor, &node);
            }
        }
        Ok(meta)
    })
    .await
}

fn running_terminals(s: &CanvasStore, ws: &str) -> Vec<(String, String)> {
    let Ok(w) = s.workspace(ws) else {
        return Vec::new();
    };
    let term = crate::pty::term();
    let mut out = Vec::new();
    for f in &w.floors {
        let nodes = s
            .read(ws, &f.id, |d| {
                d.nodes
                    .iter()
                    .filter(|n| n.is_terminal())
                    .filter_map(|n| {
                        n.data_str("terminalId")
                            .map(|t| (n.id.clone(), t.to_string()))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (node, tid) in nodes {
            if term.info(&tid).is_some_and(|i| i.alive) {
                out.push((f.id.clone(), node));
            }
        }
    }
    out
}

pub fn start_backup_scheduler(s: Arc<CanvasStore>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ON: AtomicBool = AtomicBool::new(false);
    if ON.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("canvas-backups".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5 * 60));
            loop {
                let n = wio::backup_all(&s, "hourly");
                if n > 0 {
                    tracing::info!("[canvas] hourly backup: {n} workspace(s)");
                }
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        });
}

#[tauri::command]
pub async fn canvas_workspace_export(
    app: AppHandle,
    workspace_id: String,
    path: String,
    include_secrets: Option<bool>,
    include_chat: Option<bool>,
) -> Result<ExportReport, String> {
    let opts = ExportOptions {
        include_secrets: include_secrets.unwrap_or(false),
        include_chat: include_chat.unwrap_or(true),
    };
    export(store(&app), workspace_id, path, opts).await
}

#[tauri::command]
pub async fn canvas_workspace_import_preview(
    app: AppHandle,
    path: String,
) -> Result<ImportPreview, String> {
    import_preview(store(&app), path).await
}

#[tauri::command]
pub async fn canvas_workspace_import(
    app: AppHandle,
    path: String,
    name: Option<String>,
    dir: Option<String>,
) -> Result<ImportResult, String> {
    import(store(&app), path, ImportOptions { name, dir }).await
}

#[tauri::command]
pub async fn canvas_workspace_backups(
    app: AppHandle,
    workspace_id: String,
) -> Result<Vec<BackupInfo>, String> {
    let s = store(&app);
    blocking(move || wio::list_backups(&s, &workspace_id)).await
}

#[tauri::command]
pub async fn canvas_workspace_backup_now(
    app: AppHandle,
    workspace_id: String,
) -> Result<BackupInfo, String> {
    let s = store(&app);
    blocking(move || wio::create_backup(&s, &workspace_id, "manual")).await
}

#[tauri::command]
pub async fn canvas_workspace_backup_diff(
    app: AppHandle,
    workspace_id: String,
    backup_id: String,
) -> Result<RestoreDiff, String> {
    let s = store(&app);
    blocking(move || wio::restore_diff(&s, &workspace_id, &backup_id)).await
}

#[tauri::command]
pub async fn canvas_workspace_backup_restore(
    app: AppHandle,
    workspace_id: String,
    backup_id: String,
) -> Result<Workspace, String> {
    restore(store(&app), workspace_id, backup_id).await
}
