//! Tauri commands for reaching the canvas from outside this window:
//!
//! - phone control (`crate::canvas::remote`, served by the remote listener
//!   of `crate::local_bridge_remote`): a 6-digit pairing code and the page
//!   URLs. Turning the listener on, listing and revoking devices reuse the
//!   Central's `remote_*` commands (same devices, same scopes);
//! - read-only visits (`crate::canvas::visit`) through the room relay.

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::canvas::visit;
use crate::local_bridge_remote::{self as lbr, Scope};

fn ready(app: &AppHandle) {
    crate::canvas::doc::host().bind(app);
    crate::canvas::companion::start(app);
    lbr::hub().set_app(app);
}

/// Listener status plus the canvas page URL on each endpoint.
#[tauri::command]
pub async fn canvas_remote_status(app: AppHandle) -> Result<Value, String> {
    ready(&app);
    let status = lbr::hub().status();
    let urls: Vec<String> = status
        .endpoints
        .iter()
        .map(|e| format!("{}/remote/canvas/", e.url.trim_end_matches('/')))
        .collect();
    Ok(json!({ "status": status, "canvasUrls": urls }))
}

/// A one-time 6-digit code (5 min) for `scope`: `read` (read-only) or
/// `drive` (full control). The phone types it on `/remote/canvas/`.
#[tauri::command]
pub async fn canvas_remote_pair_code(
    app: AppHandle,
    scope: String,
    label: Option<String>,
) -> Result<Value, String> {
    ready(&app);
    let h = lbr::hub();
    let addr = h
        .bound_addr()
        .ok_or_else(|| "REMOTE_NOT_RUNNING: turn remote access on first".to_string())?;
    let scope = Scope::parse(&scope)?;
    let code = h.create_code_pairing(scope, label);
    let mut urls = vec![format!(
        "{}/remote/canvas/",
        lbr::base_for(addr.ip(), addr.port())
    )];
    if let Some(u) = h.config().tailscale_serve_url {
        urls.push(format!("{}/remote/canvas/", u.trim_end_matches('/')));
    }
    let qr = lbr::render_qr(&urls[urls.len() - 1]).ok();
    Ok(json!({ "pair": code, "urls": urls, "qrSvg": qr }))
}

// ---------------------------------------------------------------- visits

#[tauri::command]
pub async fn canvas_visit_status() -> Result<Value, String> {
    Ok(visit::state_json())
}

/// Opens `floorId` of `workspaceId` for read-only visits; the state carries
/// the room code to share.
#[tauri::command]
pub async fn canvas_visit_open(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    server: Option<String>,
) -> Result<Value, String> {
    ready(&app);
    visit::open(&app, workspace_id, floor_id, server).await
}

#[tauri::command]
pub async fn canvas_visit_close(app: AppHandle) -> Result<Value, String> {
    Ok(visit::close(&app))
}

/// Host: shares (or stops sharing) one terminal's screen text.
#[tauri::command]
pub async fn canvas_visit_share(
    app: AppHandle,
    node_id: String,
    shared: bool,
) -> Result<Value, String> {
    ready(&app);
    visit::share(&app, &node_id, shared)
}

/// Visits a canvas by code: `{state, scene}`; later scenes arrive as
/// `canvas://visit-scene`.
#[tauri::command]
pub async fn canvas_visit_join(
    app: AppHandle,
    code: String,
    server: Option<String>,
) -> Result<Value, String> {
    ready(&app);
    visit::join(&app, code, server).await
}

#[tauri::command]
pub async fn canvas_visit_leave(app: AppHandle) -> Result<Value, String> {
    Ok(visit::leave(&app))
}

#[tauri::command]
pub async fn canvas_visit_chat(text: String) -> Result<(), String> {
    visit::chat(&text)
}

/// Visitor: `kind` = wave | point | react (with `x`, `y` in world units and
/// an optional `emoji`). Shows on the host's canvas, touches no agent.
#[tauri::command]
pub async fn canvas_visit_react(
    kind: String,
    x: Option<f64>,
    y: Option<f64>,
    emoji: Option<String>,
) -> Result<(), String> {
    visit::react(&kind, x, y, emoji)
}

/// Visitor: asks the host for a fresh scene.
#[tauri::command]
pub async fn canvas_visit_refresh() -> Result<(), String> {
    visit::interest()
}
