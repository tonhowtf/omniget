use omniget_core::core::canvas::media::{Actor, ModelSchema, ModelSummary};
use serde_json::Value;

use crate::canvas::generate;

#[tauri::command]
pub async fn canvas_generate_models() -> Result<Vec<ModelSummary>, String> {
    generate::models().await
}

#[tauri::command]
pub async fn canvas_generate_schema(model: String) -> Result<ModelSchema, String> {
    generate::schema_of(&model).await
}

#[tauri::command]
pub async fn canvas_generate_quote(node_id: String) -> Result<Value, String> {
    generate::requested_by(&node_id, "user");
    generate::quote(&node_id, Actor::Person).await
}

#[tauri::command]
pub async fn canvas_generate_submit(
    node_id: String,
    quote_id: String,
    idempotency_key: Option<String>,
) -> Result<Value, String> {
    generate::requested_by(&node_id, "user");
    generate::submit(&node_id, &quote_id, idempotency_key, Actor::Person).await
}

#[tauri::command]
pub async fn canvas_generate_reconcile(node_id: String) -> Result<Value, String> {
    generate::reconcile(&node_id).await
}

#[tauri::command]
pub async fn canvas_generate_release(node_id: String) -> Result<Value, String> {
    generate::release(&node_id)
}

#[tauri::command]
pub async fn canvas_generate_budget(
    workspace_id: String,
    ceiling: Option<f64>,
    auto_limit: Option<f64>,
    auto_per_hour: Option<i64>,
) -> Result<Value, String> {
    generate::budget(&workspace_id, ceiling, auto_limit, auto_per_hour, true).await
}
