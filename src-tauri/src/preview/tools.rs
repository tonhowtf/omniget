//! Preview tools for agents (embedded MCP + LLM broker). The table entries
//! live in `omniget-core/src/core/llm/tool_table.rs` as `ToolImpl::Host`; the
//! host half in `mcp.rs` forwards every `preview_*` name here:
//!
//! ```ignore
//! n if n.starts_with("preview_") => crate::preview::tools::dispatch(app, n, a).await,
//! ```
//!
//! Every tool takes an optional `threadId`; without it the last preview used
//! is the target. `preview_navigate` with a `url` opens a hidden preview when
//! the thread has none, so an agent can work without the tab being open.

use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::AppHandle;

use super::{manager, script};

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
}

fn b(v: &Value, k: &str) -> Option<bool> {
    v.get(k).and_then(|x| x.as_bool())
}

fn n(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64())
}

fn target(a: &Value) -> Result<String, String> {
    manager()
        .resolve_label(s(a, "threadId").as_deref())
        .ok_or_else(|| "PREVIEW_NOT_OPEN: no preview is open; call preview_navigate with a url (or open the Browser tab of the thread)".into())
}

/// Tool router. Names: preview_servers, preview_navigate, preview_snapshot,
/// preview_click, preview_type, preview_console, preview_screenshot.
pub async fn dispatch(app: &AppHandle, name: &str, a: Value) -> Result<Value, String> {
    match name {
        "preview_servers" => preview_servers(a).await,
        "preview_navigate" => preview_navigate(app, a).await,
        "preview_snapshot" => preview_snapshot(app, a).await,
        "preview_click" => preview_click(app, a).await,
        "preview_type" => preview_type(app, a).await,
        "preview_console" => preview_console(app, a).await,
        "preview_screenshot" => preview_screenshot(app, a).await,
        other => Err(format!("PREVIEW_UNKNOWN_TOOL: {other}")),
    }
}

/// `{cwd?, all?}` → dev servers listening from inside `cwd`.
pub async fn preview_servers(a: Value) -> Result<Value, String> {
    let cwd = s(&a, "cwd").map(PathBuf::from);
    let all = b(&a, "all").unwrap_or(cwd.is_none());
    let list = super::ports::discover(cwd.as_deref(), all).await?;
    Ok(json!({ "servers": list }))
}

/// `{url?, action?: back|forward|reload, threadId?}`.
pub async fn preview_navigate(app: &AppHandle, a: Value) -> Result<Value, String> {
    let action = s(&a, "action");
    let url = s(&a, "url");
    if let Some(act) = action
        .as_deref()
        .filter(|x| matches!(*x, "back" | "forward" | "reload"))
    {
        let label = target(&a)?;
        let st = super::history(app, &label, act).await?;
        return Ok(json!(st));
    }
    let url = url.ok_or("PREVIEW_ARGS: url or action is required")?;
    let label = match manager().resolve_label(s(&a, "threadId").as_deref()) {
        Some(l) => l,
        None => {
            let thread = s(&a, "threadId").unwrap_or_else(|| "agent".to_string());
            let st = super::open(app, &thread, None, None)?;
            st.label
        }
    };
    let st = super::navigate(app, &label, &url).await?;
    Ok(json!(st))
}

/// `{threadId?, maxNodes?, maxChars?, includeHtml?}` → `{url, title, tree,
/// text, html?, nodes, truncated, viewport}`. Refs (`e12`) feed click/type.
pub async fn preview_snapshot(app: &AppHandle, a: Value) -> Result<Value, String> {
    let label = target(&a)?;
    let args = json!({
        "maxNodes": n(&a, "maxNodes").unwrap_or(1500).clamp(50, 10_000),
        "maxChars": n(&a, "maxChars").unwrap_or(20_000).clamp(500, 200_000),
        "includeHtml": b(&a, "includeHtml").unwrap_or(false),
    });
    super::eval_fn(app, &label, &script::snapshot_fn(), args).await
}

/// `{selector, double?, threadId?}`; selector = CSS, `e12` from the snapshot,
/// or `text=Label`.
pub async fn preview_click(app: &AppHandle, a: Value) -> Result<Value, String> {
    let label = target(&a)?;
    let selector = s(&a, "selector").ok_or("PREVIEW_ARGS: selector is required")?;
    let v = super::eval_fn(
        app,
        &label,
        &script::click_fn(),
        json!({ "selector": selector, "double": b(&a, "double").unwrap_or(false) }),
    )
    .await?;
    // A click often navigates; give the page a beat so the next snapshot sees it.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    Ok(json!({ "result": v, "page": super::state_json(&label) }))
}

/// `{selector, text, submit?, clear?, threadId?}`.
pub async fn preview_type(app: &AppHandle, a: Value) -> Result<Value, String> {
    let label = target(&a)?;
    let selector = s(&a, "selector").ok_or("PREVIEW_ARGS: selector is required")?;
    let text = a
        .get("text")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    super::eval_fn(
        app,
        &label,
        &script::type_fn(),
        json!({ "selector": selector, "text": text, "submit": b(&a, "submit").unwrap_or(false), "clear": b(&a, "clear").unwrap_or(true) }),
    )
    .await
}

/// `{level?: error|warn|log|info|debug, limit?, clear?, threadId?}`. Reads and
/// (by default) drains the page's console ring (500 entries, since the last
/// page load).
pub async fn preview_console(app: &AppHandle, a: Value) -> Result<Value, String> {
    let label = target(&a)?;
    super::eval_fn(
        app,
        &label,
        script::CONSOLE_FN,
        json!({ "level": s(&a, "level"), "limit": n(&a, "limit").unwrap_or(200).clamp(1, 500), "clear": b(&a, "clear").unwrap_or(true) }),
    )
    .await
}

/// `{threadId?}` → `{path, width, height, bytes}` (PNG under
/// `<app_data>/snapshots/`).
pub async fn preview_screenshot(app: &AppHandle, a: Value) -> Result<Value, String> {
    let label = target(&a)?;
    let shot = super::screenshot(app, &label).await?;
    Ok(
        json!({ "path": shot.path, "width": shot.width, "height": shot.height, "bytes": shot.bytes, "page": super::state_json(&label) }),
    )
}
