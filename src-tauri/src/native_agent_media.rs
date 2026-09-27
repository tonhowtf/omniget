//! Native pixels in a bounded, principal-owned managed directory.
use crate::mcp::{
    artifacts,
    policy::{self, Principal},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io::Cursor, path::PathBuf};
use tauri::{AppHandle, Manager};
static SNAPSHOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Capture {
    window_id: Option<String>,
    region: Option<Region>,
}
pub(super) fn directory(p: &Principal) -> Result<PathBuf, String> {
    policy::active(p)?;
    if !p.scopes.iter().any(|s| s == "app_capture") || !p.scopes.iter().any(|s| s == "transfer") {
        return Err("PERMISSION_REQUIRED: app_capture and transfer required".into());
    }
    uuid::Uuid::parse_str(&p.id).map_err(|_| "PERMISSION_REQUIRED")?;
    let root = crate::core::paths::app_data_dir()
        .ok_or("CAPTURE_FAILED: data directory")?
        .join("native-agent-artifacts")
        .join(&p.id);
    std::fs::create_dir_all(&root).map_err(|e| format!("CAPTURE_FAILED: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "CAPTURE_FAILED: permissions")?;
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(&root).map_err(|_| "CAPTURE_FAILED")? {
        let entry = entry.map_err(|_| "CAPTURE_FAILED")?;
        let meta = entry.metadata().map_err(|_| "CAPTURE_FAILED")?;
        if meta.is_file()
            && meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d.as_secs() > 86400)
        {
            let _ = std::fs::remove_file(entry.path());
        } else if meta.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name
                .strip_suffix("-frames")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                if meta
                    .modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|d| d.as_secs() > 3600)
                {
                    let _ = std::fs::remove_dir_all(entry.path());
                } else {
                    for frame in std::fs::read_dir(entry.path())
                        .map_err(|_| "CAPTURE_FAILED")?
                        .flatten()
                    {
                        total =
                            total.saturating_add(frame.metadata().map(|m| m.len()).unwrap_or(0));
                    }
                }
            }
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    if total > 512 * 1024 * 1024 {
        return Err("CAPTURE_FAILED: managed storage exceeds 512 MiB".into());
    }
    Ok(root)
}
pub(super) async fn pixels(app: &AppHandle, window_id: &str) -> Result<(Vec<u8>, Value), String> {
    let _snapshot = tokio::time::timeout(std::time::Duration::from_secs(1), SNAPSHOT.lock())
        .await
        .map_err(|_| "SESSION_BUSY: snapshot acquisition")?;
    let before = super::native_agent::bridge(app, window_id, "inspect", json!({})).await?;
    if before["sensitive"] != false {
        return Err("PERMISSION_REQUIRED: sensitive surface".into());
    }
    if before["readiness"] != "ready" {
        return Err("FRONTEND_NOT_READY: surface is loading".into());
    }
    let window = app
        .get_webview_window(window_id)
        .ok_or("WINDOW_NOT_FOUND")?;
    if window.is_minimized().unwrap_or(true) || !window.is_visible().unwrap_or(false) {
        return Err("CAPTURE_FAILED: window is hidden or minimized".into());
    }
    let png = super::native_capture::snapshot(&window).await?;
    let after = super::native_agent::bridge(app, window_id, "inspect", json!({})).await?;
    if after["readiness"] != "ready"
        || after["sensitive"] != false
        || before["route"] != after["route"]
        || before["stateRevision"] != after["stateRevision"]
    {
        return Err("STALE_TARGET: surface changed during capture".into());
    }
    Ok((png, before))
}
pub async fn capture(app: &AppHandle, p: &Principal, args: Value) -> Result<Value, String> {
    let options: Capture = serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT")?;
    let window_id = options.window_id.unwrap_or_else(|| "main".into());
    let root = directory(p)?;
    let (bytes, state) = pixels(app, &window_id).await?;
    let mut img = image::load_from_memory(&bytes).map_err(|_| "CAPTURE_FAILED: invalid PNG")?;
    let original = json!({"width":img.width(),"height":img.height()});
    let css_width = state["viewport"]["width"]
        .as_f64()
        .filter(|n| *n > 0.)
        .ok_or("CAPTURE_FAILED: viewport")?;
    let scale = img.width() as f64 / css_width;
    let mut crop = Value::Null;
    if let Some(r) = options.region {
        if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
            || r.x < 0.
            || r.y < 0.
            || r.width <= 0.
            || r.height <= 0.
        {
            return Err("INVALID_ARGUMENT: crop".into());
        }
        let (x, y, w, h) = (
            (r.x * scale).floor() as u32,
            (r.y * scale).floor() as u32,
            (r.width * scale).ceil() as u32,
            (r.height * scale).ceil() as u32,
        );
        if x.saturating_add(w) > img.width() || y.saturating_add(h) > img.height() {
            return Err("INVALID_ARGUMENT: crop outside image".into());
        }
        img = img.crop_imm(x, y, w, h);
        crop = json!({"x":x,"y":y,"width":w,"height":h,"coordinateSpace":"physical-pixels"});
    }
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.png"));
    if crop.is_null() {
        // Preserve the native PNG, including any colour metadata, byte for byte.
        std::fs::write(&path, &bytes).map_err(|e| format!("CAPTURE_FAILED: {e}"))?;
    } else {
        img.save_with_format(&path, image::ImageFormat::Png)
            .map_err(|e| format!("CAPTURE_FAILED: {e}"))?;
    }
    policy::active(p)?;
    artifacts::grant_root(&p.id, &root)?;
    let artifact = artifacts::register(p, &path)?;
    let mut preview = Cursor::new(Vec::new());
    img.thumbnail(640, 480)
        .write_to(&mut preview, image::ImageFormat::Png)
        .map_err(|_| "CAPTURE_FAILED: preview")?;
    if preview.get_ref().len() > 256 * 1024 {
        preview = Cursor::new(Vec::new());
        img.thumbnail(320, 240)
            .write_to(&mut preview, image::ImageFormat::Png)
            .map_err(|_| "CAPTURE_FAILED: bounded preview")?;
    }
    if preview.get_ref().len() > 384 * 1024 {
        return Err("CAPTURE_FAILED: preview limit".into());
    }
    let metadata = json!({"schemaVersion":1,"windowId":window_id,"route":state["route"],"stateRevision":state["stateRevision"],"source":"WKWebView.takeSnapshot","appVersion":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"width":img.width(),"height":img.height(),"originalDimensions":original,"scale":scale,"crop":crop,"observedAt":chrono::Utc::now().to_rfc3339(),"color":"native snapshot; no explicit profile conversion","profile":"current app profile","artifact":artifact});
    std::fs::write(
        root.join(format!("{id}.json")),
        serde_json::to_vec_pretty(&metadata).map_err(|_| "CAPTURE_FAILED")?,
    )
    .map_err(|_| "CAPTURE_FAILED: metadata")?;
    let mut result = metadata;
    result["_mcpImage"] = json!({"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(preview.into_inner())});
    Ok(result)
}
