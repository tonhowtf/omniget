use omniget_core::core::tools::{ai_keys, ollama, pricing, usage};
use serde::Serialize;

use super::{err, progress};

#[tauri::command]
pub async fn tool_ollama_status(host: Option<String>) -> ollama::OllamaStatus {
    ollama::status(host.as_deref().unwrap_or("")).await
}

#[tauri::command]
pub fn tool_ollama_recommended() -> Vec<ollama::Recommended> {
    ollama::recommended()
}

#[tauri::command]
pub async fn tool_ollama_pull(app: tauri::AppHandle, host: Option<String>, name: String) -> Result<(), String> {
    ollama::pull(host.as_deref().unwrap_or(""), &name, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_ollama_delete(host: Option<String>, name: String) -> Result<(), String> {
    ollama::delete(host.as_deref().unwrap_or(""), &name).await.map_err(err)
}

#[tauri::command]
pub async fn tool_pricing_info(force: Option<bool>) -> Result<pricing::PricingInfo, String> {
    pricing::info(force.unwrap_or(false)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_pricing_search(query: String, mode: Option<String>, limit: Option<usize>) -> Result<Vec<pricing::ModelPrice>, String> {
    pricing::search(&query, mode.as_deref().unwrap_or(""), limit.unwrap_or(60)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_pricing_for(model: String) -> Option<pricing::ModelPrice> {
    pricing::price_for(&model).await
}

#[tauri::command]
pub async fn tool_usage_report(days: Option<u32>) -> usage::UsageReport {
    usage::report(days.unwrap_or(30)).await
}

#[tauri::command]
pub fn tool_usage_clear() -> Result<(), String> {
    usage::clear().map_err(err)
}

// ── Chaves de API (estudo 24) ──

#[tauri::command]
pub fn tool_keys_kinds() -> Vec<ai_keys::Kind> {
    ai_keys::KINDS.to_vec()
}

#[tauri::command]
pub fn tool_keys_list() -> Vec<ai_keys::KeyView> {
    ai_keys::list()
}

#[tauri::command]
pub fn tool_keys_save(entry: ai_keys::KeyEntry) -> Result<ai_keys::KeyView, String> {
    ai_keys::upsert(entry).map_err(err)
}

#[tauri::command]
pub fn tool_keys_delete(id: String) -> Result<(), String> {
    ai_keys::delete(&id).map_err(err)
}

#[tauri::command]
pub async fn tool_keys_test(id: String) -> Result<ai_keys::KeyView, String> {
    ai_keys::test(&id).await.map_err(err)
}

#[tauri::command]
pub async fn tool_keys_balance(id: String) -> Result<ai_keys::KeyView, String> {
    ai_keys::balance(&id).await.map_err(err)
}

#[tauri::command]
pub async fn tool_keys_models(entry: ai_keys::KeyEntry) -> Result<Vec<String>, String> {
    // Chave salva: a UI manda `key` vazia e o id; buscamos o segredo aqui.
    let e = if entry.key.is_empty() && !entry.id.is_empty() { ai_keys::entry_with_secret(&entry.id).map_err(err)? } else { entry };
    ai_keys::models(&e).await.map_err(err)
}

#[tauri::command]
pub fn tool_keys_export(format: String, ids: Vec<String>) -> Result<String, String> {
    ai_keys::export(&format, &ids).map_err(err)
}

#[tauri::command]
pub fn tool_keys_use(id: String) -> Result<(), String> {
    ai_keys::use_in_app(&id).map_err(err)
}

// ── Servidor MCP ──

#[derive(Serialize)]
pub struct McpStatus {
    pub enabled: bool,
    pub bridge_enabled: bool,
    pub port: u16,
    pub url: String,
    pub token: String,
    pub tools: Vec<crate::mcp::ToolDef>,
    pub snippets: Vec<(String, String)>,
}

#[tauri::command]
pub fn tool_mcp_status(app: tauri::AppHandle) -> McpStatus {
    let settings = crate::storage::config::load_settings(&app);
    let url = if settings.bridge.port == 0 { String::new() } else { format!("http://127.0.0.1:{}/mcp", settings.bridge.port) };
    McpStatus {
        enabled: crate::mcp::enabled(),
        bridge_enabled: settings.bridge.enabled,
        port: settings.bridge.port,
        snippets: crate::mcp::client_snippets(&url, &settings.bridge.token),
        url,
        token: settings.bridge.token,
        tools: crate::mcp::tools(),
    }
}

#[tauri::command]
pub fn tool_mcp_set_enabled(enabled: bool) -> Result<bool, String> {
    crate::mcp::set_enabled(enabled)?;
    Ok(crate::mcp::enabled())
}

/// Faz um `initialize` + `tools/list` de verdade contra o proprio endpoint,
/// para provar que a porta, o token e o servidor respondem.
#[tauri::command]
pub async fn tool_mcp_selftest(app: tauri::AppHandle) -> Result<String, String> {
    let settings = crate::storage::config::load_settings(&app);
    if settings.bridge.port == 0 {
        return Err("bridge sem porta".into());
    }
    let url = format!("http://127.0.0.1:{}/mcp", settings.bridge.port);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().map_err(err)?;
    let init = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": crate::mcp::PROTOCOL, "capabilities": {}, "clientInfo": { "name": "omniget-selftest", "version": "1" } } });
    let r: serde_json::Value = client.post(&url).bearer_auth(&settings.bridge.token).json(&init).send().await.map_err(err)?.error_for_status().map_err(err)?.json().await.map_err(err)?;
    let version = r["result"]["protocolVersion"].as_str().unwrap_or("?").to_string();
    let list = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let r: serde_json::Value = client.post(&url).bearer_auth(&settings.bridge.token).json(&list).send().await.map_err(err)?.json().await.map_err(err)?;
    let n = r["result"]["tools"].as_array().map(|a| a.len()).unwrap_or(0);
    Ok(format!("{} · {} tools · {}", version, n, url))
}
