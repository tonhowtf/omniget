use omniget_core::core::llm::audit::Actor;
use serde_json::Value;

use crate::llm_budget::{self, ResolveBody};

#[tauri::command]
pub async fn llm_budget_state() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(llm_budget::state)
        .await
        .map_err(|e| format!("ERR_LLM_BUDGET: {e}"))?
}

#[tauri::command]
pub async fn llm_budget_resolve(
    id: String,
    action: String,
    amount: Option<f64>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        llm_budget::resolve(
            &id,
            &ResolveBody { action, amount },
            Actor::via("app", "user"),
        )
    })
    .await
    .map_err(|e| format!("ERR_LLM_BUDGET: {e}"))?
}
