use std::path::PathBuf;

use omniget_core::core::guard::{self, GuardOptions, GuardReport};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("GUARD_TASK: {e}"))
}

#[tauri::command]
pub async fn guard_scan_path(
    path: String,
    kind: Option<String>,
    origin_tool: Option<String>,
    options: Option<GuardOptions>,
) -> Result<Vec<GuardReport>, String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("GUARD_NOT_FOUND: {path}"));
    }
    let opts = options.unwrap_or_default();
    blocking(move || guard::scan_path(&p, kind.as_deref(), origin_tool.as_deref(), &opts)).await
}
