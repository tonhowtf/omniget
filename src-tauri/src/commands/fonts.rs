use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use omniget_core::core::fonts::{
    catalog, install, scan, CatalogFont, FileState, FontDetails, InstallOutcome, InstalledFamily,
};

static JOBS: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
struct InstallProgress<'a> {
    job_id: &'a str,
    index: usize,
    total: usize,
    file_name: &'a str,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

fn register(job_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    if let Ok(mut map) = JOBS.lock() {
        map.get_or_insert_with(HashMap::new)
            .insert(job_id.to_string(), token.clone());
    }
    token
}

fn unregister(job_id: &str) {
    if let Ok(mut map) = JOBS.lock() {
        if let Some(m) = map.as_mut() {
            m.remove(job_id);
        }
    }
}

#[tauri::command]
pub async fn fonts_installed(refresh: Option<bool>) -> Result<Vec<InstalledFamily>, String> {
    let refresh = refresh.unwrap_or(false);
    tokio::task::spawn_blocking(move || scan::installed(refresh))
        .await
        .map_err(|e| format!("The font scan stopped unexpectedly: {e}"))
}

#[tauri::command]
pub async fn fonts_catalog(refresh: Option<bool>) -> Result<Vec<CatalogFont>, String> {
    catalog::list(refresh.unwrap_or(false)).await
}

#[tauri::command]
pub async fn fonts_details(id: String) -> Result<FontDetails, String> {
    catalog::details(&id).await
}

#[tauri::command]
pub fn fonts_user_dir() -> Option<String> {
    scan::user_font_dir().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn fonts_install(
    app: AppHandle,
    job_id: String,
    id: String,
    subset: String,
    out_dir: Option<String>,
) -> Result<InstallOutcome, String> {
    let token = register(&job_id);
    let target = out_dir.filter(|d| !d.trim().is_empty()).map(PathBuf::from);
    let details = tokio::select! {
        d = catalog::details(&id) => d,
        _ = token.cancelled() => {
            unregister(&job_id);
            return Ok(InstallOutcome {
                installed: target.is_none(),
                cancelled: true,
                ..Default::default()
            });
        }
    };
    let details = match details {
        Ok(d) => d,
        Err(e) => {
            unregister(&job_id);
            return Err(e);
        }
    };
    let result = install::run(
        &details,
        &subset,
        target,
        token,
        |index, total, file_name, state: FileState, error| {
            let _ = app.emit(
                "fonts-install-progress",
                InstallProgress {
                    job_id: &job_id,
                    index,
                    total,
                    file_name,
                    state: state.as_str(),
                    error,
                },
            );
        },
    )
    .await;
    unregister(&job_id);
    result
}

#[tauri::command]
pub fn fonts_cancel(job_id: String) {
    if let Ok(map) = JOBS.lock() {
        if let Some(token) = map.as_ref().and_then(|m| m.get(&job_id)) {
            token.cancel();
        }
    }
}
