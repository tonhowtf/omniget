use tauri::AppHandle;

#[tauri::command]
pub async fn canvas_lead_set(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_id: String,
    on: bool,
) -> Result<(), String> {
    crate::canvas::routines::start(&app);
    crate::canvas::doc::host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || {
        crate::canvas::lead::set_lead(&workspace_id, &floor_id, &node_id, on)
    })
    .await
    .map_err(|e| format!("CANVAS_TASK: {e}"))?
}
