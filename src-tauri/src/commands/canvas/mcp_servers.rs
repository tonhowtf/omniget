use serde_json::Value;

use crate::canvas::mcp_attach::{self, ServerView};

#[tauri::command]
pub async fn canvas_mcp_servers() -> Result<Vec<ServerView>, String> {
    mcp_attach::servers().await
}

#[tauri::command]
pub async fn canvas_mcp_server_set(id: String, enabled: bool) -> Result<Vec<ServerView>, String> {
    mcp_attach::set_enabled(&id, enabled).await
}

#[tauri::command]
pub async fn canvas_mcp_server_test(id: String) -> Result<Value, String> {
    mcp_attach::test(&id).await
}
