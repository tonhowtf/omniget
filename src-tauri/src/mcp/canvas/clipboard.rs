use serde_json::{json, Value};
use tauri::AppHandle;

use omniget_core::core::canvas::clipboard::ClipSelection;
use omniget_core::core::canvas::model::Point;

use super::{blocking, list, locate, num, ready, st, ws_floor};
use crate::canvas::clipboard::{self as clip, Where};

async fn window_state(app: &AppHandle) -> Option<Value> {
    super::super::canvas_ui::ui(app, "state", json!({}))
        .await
        .ok()
        .filter(|v| v["workspaceId"].is_string())
}

fn centre(v: &Value) -> Option<Point> {
    let (x, y, z) = (
        v["camera"]["x"].as_f64()?,
        v["camera"]["y"].as_f64()?,
        v["camera"]["zoom"].as_f64().filter(|z| *z > 0.0)?,
    );
    let (w, h) = (v["viewport"]["w"].as_f64()?, v["viewport"]["h"].as_f64()?);
    Some(Point {
        x: x + w / (2.0 * z),
        y: y + h / (2.0 * z),
    })
}

pub async fn call(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let s = ready(app);
    let action = st(a, "action").unwrap_or_else(|| "paste".into());
    match action.as_str() {
        "copy" | "cut" => {
            let keys = list(a, "nodeIds");
            let drawings = list(a, "drawingIds");
            let (ws, floor, node_ids, drawing_ids) = if keys.is_empty() {
                let v = window_state(app).await.ok_or(
                    "MISSING_ARGUMENT: nodeIds (no canvas open in the window to take the selection from)",
                )?;
                let sel: Vec<String> = list(&v, "selection");
                let (ws, floor) = (
                    v["workspaceId"].as_str().unwrap_or_default().to_string(),
                    v["floorId"].as_str().unwrap_or_default().to_string(),
                );
                let doc = s.read(&ws, &floor, |d| d.clone())?;
                let nodes = sel
                    .iter()
                    .filter(|id| doc.node(id).is_some())
                    .cloned()
                    .collect();
                let mut dr: Vec<String> = sel
                    .iter()
                    .filter(|id| doc.drawings.iter().any(|d| &d.id == *id))
                    .cloned()
                    .collect();
                dr.extend(drawings);
                (ws, floor, nodes, dr)
            } else {
                let mut place: Option<(String, String)> = None;
                let mut ids = Vec::new();
                for k in &keys {
                    let (ws, floor, n) = locate(&s, k)?;
                    match &place {
                        None => place = Some((ws, floor)),
                        Some((w, f)) if *w != ws || *f != floor => {
                            return Err("INVALID_ARGUMENT: nodeIds must all be on one floor".into())
                        }
                        _ => {}
                    }
                    ids.push(n.id);
                }
                let (ws, floor) = place.expect("at least one node");
                (ws, floor, ids, drawings)
            };
            if node_ids.is_empty() && drawing_ids.is_empty() {
                return Err("CANVAS_INVALID: nothing selected to copy".into());
            }
            let sel = ClipSelection {
                node_ids,
                drawing_ids,
            };
            let cut = action == "cut";
            let app2 = app.clone();
            let mut r = blocking(move || clip::copy(&app2, &ws, &floor, &sel, cut)).await?;
            r["copied"] = r["clip"]["nodes"].clone();
            Ok(r)
        }
        "paste" => {
            let win = window_state(app).await;
            let (ws, floor) = if st(a, "workspaceId").is_some() || st(a, "floorId").is_some() {
                ws_floor(&s, a)?
            } else if let Some(v) = &win {
                (
                    v["workspaceId"].as_str().unwrap_or_default().to_string(),
                    v["floorId"].as_str().unwrap_or_default().to_string(),
                )
            } else {
                ws_floor(&s, a)?
            };
            let at = match (num(a, "x"), num(a, "y")) {
                (Some(x), Some(y)) => Some(Point { x, y }),
                _ => None,
            };
            let center = win
                .as_ref()
                .filter(|v| v["workspaceId"] == json!(ws) && v["floorId"] == json!(floor))
                .and_then(centre);
            let app2 = app.clone();
            let (w2, f2) = (ws.clone(), floor.clone());
            let mut r =
                blocking(move || clip::paste(&app2, &w2, &f2, Where { at, center })).await?;
            r["workspaceId"] = json!(ws);
            r["floorId"] = json!(floor);
            Ok(r)
        }
        "read" => {
            let app2 = app.clone();
            blocking(move || Ok(clip::read(&app2))).await
        }
        other => Err(format!("INVALID_ARGUMENT: action '{other}'")),
    }
}
