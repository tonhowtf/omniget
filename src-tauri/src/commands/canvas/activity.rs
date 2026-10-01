use tauri::AppHandle;

use crate::canvas::activity::{self, Entry};

#[tauri::command]
pub async fn canvas_activity(
    app: AppHandle,
    workspace_id: String,
    floor_id: Option<String>,
    since: Option<u64>,
    limit: Option<usize>,
) -> Result<Vec<Entry>, String> {
    crate::canvas::doc::host().bind(&app);
    Ok(activity::recent(
        &workspace_id,
        floor_id.as_deref(),
        since,
        limit.unwrap_or(100).clamp(1, 500),
    ))
}
