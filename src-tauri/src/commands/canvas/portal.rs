use base64::Engine as _;
use serde_json::Value;
use tauri::{AppHandle, Webview};

use crate::portal::{self, automation, Bounds, OpenSpec, PortalState, Scope};

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn canvas_portal_open(
    app: AppHandle,
    node_id: String,
    workspace_id: String,
    url: Option<String>,
    storage_scope: Option<String>,
    user_agent: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<PortalState, String> {
    let viewport = match (width, height) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((w, h)),
        _ => None,
    };
    let saved = automation::spec_from_node(&node_id);
    portal::open(
        &app,
        OpenSpec {
            node_id,
            workspace_id,
            url,
            scope: Scope::parse(storage_scope.as_deref()),
            user_agent,
            viewport,
            trusted_hosts: saved
                .as_ref()
                .map(|s| s.trusted_hosts.clone())
                .unwrap_or_default(),
            muted: saved.is_some_and(|s| s.muted),
        },
    )
}

#[tauri::command]
pub async fn canvas_portal_trust_cert(
    app: AppHandle,
    node_id: String,
) -> Result<PortalState, String> {
    portal::bind(&app);
    portal::trust_cert(&app, &node_id).await
}

#[tauri::command]
pub fn canvas_portal_inspect(app: AppHandle, node_id: String) -> Result<(), String> {
    portal::bind(&app);
    portal::open_inspector(&app, &node_id)
}

#[tauri::command]
pub async fn canvas_portal_copy_screenshot(
    app: AppHandle,
    node_id: String,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    portal::bind(&app);
    automation::ensure_open(&app, &node_id)?;
    let png = portal::snapshot_png(&app, &node_id).await?;
    let rgba = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|e| format!("PORTAL_COPY: {e}"))?
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    let img = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
    app.clipboard()
        .write_image(&img)
        .map_err(|e| format!("PORTAL_COPY: {e}"))?;
    Ok(serde_json::json!({ "copied": true, "width": w, "height": h }))
}

#[tauri::command]
pub fn canvas_portal_close(app: AppHandle, node_id: String) -> Result<(), String> {
    portal::bind(&app);
    portal::close(&app, &node_id)
}

#[tauri::command]
pub fn canvas_portal_set_bounds(
    app: AppHandle,
    node_id: String,
    bounds: Bounds,
) -> Result<(), String> {
    portal::bind(&app);
    portal::set_bounds(&app, &node_id, Some(bounds))
}

#[tauri::command]
pub fn canvas_portal_set_visible(
    app: AppHandle,
    node_id: Option<String>,
    visible: bool,
    bounds: Option<Bounds>,
) -> Result<(), String> {
    portal::bind(&app);
    match node_id {
        None => {
            portal::hide_all(&app);
            Ok(())
        }
        Some(id) if visible => portal::set_bounds(&app, &id, bounds),
        Some(id) => portal::set_bounds(&app, &id, None),
    }
}

#[tauri::command]
pub async fn canvas_portal_navigate(
    app: AppHandle,
    node_id: String,
    url: Option<String>,
    action: Option<String>,
) -> Result<PortalState, String> {
    portal::bind(&app);
    automation::ensure_open(&app, &node_id)?;
    match (url.filter(|u| !u.trim().is_empty()), action.as_deref()) {
        (Some(u), _) => portal::navigate(&app, &node_id, &u).await,
        (None, Some(a @ ("back" | "forward" | "reload"))) => {
            portal::history(&app, &node_id, a).await
        }
        _ => Err("PORTAL_ARGS: url or action (back, forward, reload) is required".into()),
    }
}

#[tauri::command]
pub async fn canvas_portal_snapshot_image(
    app: AppHandle,
    node_id: String,
) -> Result<String, String> {
    portal::bind(&app);
    let png = portal::snapshot_png(&app, &node_id).await?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

#[tauri::command]
pub async fn canvas_portal_set_user_agent(
    app: AppHandle,
    node_id: String,
    user_agent: Option<String>,
) -> Result<PortalState, String> {
    portal::bind(&app);
    let ua = user_agent
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty() && u != "desktop")
        .map(|u| portal::user_agent_preset(&u).map(String::from).unwrap_or(u));
    portal::set_user_agent(&app, &node_id, ua).await
}

#[tauri::command]
pub fn canvas_portal_states(app: AppHandle) -> Vec<PortalState> {
    portal::bind(&app);
    portal::host().states()
}

#[tauri::command]
pub fn canvas_portal_report(
    app: AppHandle,
    webview: Webview,
    kind: String,
    id: Option<String>,
    data: Option<Value>,
) {
    portal::report(
        &app,
        webview.label(),
        &kind,
        id,
        data.unwrap_or(Value::Null),
    );
}

#[tauri::command]
pub async fn canvas_portal_design_arm(
    app: AppHandle,
    node_id: String,
    hint: Option<String>,
) -> Result<serde_json::Value, String> {
    crate::portal::bind(&app);
    crate::canvas::design::arm(&node_id, hint).await
}

#[tauri::command]
pub async fn canvas_portal_design_disarm(
    app: AppHandle,
    node_id: String,
) -> Result<serde_json::Value, String> {
    crate::portal::bind(&app);
    crate::canvas::design::disarm(&node_id).await
}

#[tauri::command]
pub async fn canvas_portal_design_send(
    app: AppHandle,
    node_id: String,
    target: String,
    comment: String,
    pick: serde_json::Value,
    screenshot_path: Option<String>,
    attachments: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    crate::portal::bind(&app);
    crate::canvas::design::send(
        &node_id,
        &target,
        &comment,
        pick,
        screenshot_path,
        attachments.unwrap_or_default(),
    )
    .await
}
