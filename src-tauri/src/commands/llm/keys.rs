//! AI keys, usage report and the MCP server switch the LLM screens use.

use omniget_core::core::tools::{ai_keys, usage};
use serde::Serialize;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn tool_usage_report(days: Option<u32>) -> usage::UsageReport {
    usage::report(days.unwrap_or(30)).await
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
pub async fn tool_keys_test(id: String) -> Result<ai_keys::KeyView, String> {
    ai_keys::test(&id).await.map_err(err)
}

// ── Sign in with OpenRouter (OAuth PKCE, sem backend) ──
//
// Two steps, because the browser round-trip happens in between: the UI opens
// `url`, OpenRouter sends the user back to `omniget://openrouter-auth?state=…`
// with `code` appended, and the deep-link handler hands both to `_finish`. The
// state is mandatory there: a callback without the one this process generated is
// refused. The verifier stays in the core and the key goes straight into the
// secret store — neither ever reaches the frontend.

/// Starts the flow: returns the URL to open and the state to echo back.
#[tauri::command]
pub fn tool_ai_keys_openrouter_pkce() -> ai_keys::PkceStart {
    ai_keys::pkce_start()
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
    let url = if settings.bridge.port == 0 {
        String::new()
    } else {
        format!("http://127.0.0.1:{}/mcp", settings.bridge.port)
    };
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(err)?;
    let init = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": crate::mcp::PROTOCOL, "capabilities": {}, "clientInfo": { "name": "omniget-selftest", "version": "1" } } });
    let r: serde_json::Value = client
        .post(&url)
        .bearer_auth(&settings.bridge.token)
        .json(&init)
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let version = r["result"]["protocolVersion"]
        .as_str()
        .unwrap_or("?")
        .to_string();
    let list = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let r: serde_json::Value = client
        .post(&url)
        .bearer_auth(&settings.bridge.token)
        .json(&list)
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let n = r["result"]["tools"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(format!("{} · {} tools · {}", version, n, url))
}

// ── Contagem de tokens e custo por modelo (estende ai-prices) ──

// ── arXiv → Markdown ──
