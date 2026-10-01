use serde_json::Value;
use tauri::State;

use crate::AppState;

#[tauri::command]
pub async fn llm_telemetry_snapshot(state: State<'_, AppState>) -> Result<Value, String> {
    serde_json::to_value(state.llm.telemetry()).map_err(|e| e.to_string())
}
