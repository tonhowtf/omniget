use serde_json::{json, Value};
use tauri::AppHandle;

use crate::canvas::visit;
use crate::local_bridge_remote::{self as lbr, Scope};

fn ready(app: &AppHandle) {
    crate::canvas::doc::host().bind(app);
    crate::canvas::companion::start(app);
    lbr::hub().set_app(app);
}

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

#[tauri::command]
pub async fn canvas_visit_status() -> Result<Value, String> {
    Ok(visit::state_json())
}

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

#[tauri::command]
pub async fn canvas_visit_share(
    app: AppHandle,
    node_id: String,
    shared: bool,
) -> Result<Value, String> {
    ready(&app);
    visit::share(&app, &node_id, shared)
}

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

#[tauri::command]
pub async fn canvas_visit_react(
    kind: String,
    x: Option<f64>,
    y: Option<f64>,
    emoji: Option<String>,
) -> Result<(), String> {
    visit::react(&kind, x, y, emoji)
}

#[tauri::command]
pub async fn canvas_visit_refresh() -> Result<(), String> {
    visit::interest()
}
