//! Lift, dock and the canvas menus (Maestri M-B11, M-B12, `app.menu`).
//!
//! Lift/dock state lives in the workspace meta (`panels`), so these tools
//! work with the window anywhere: they write the state, the window follows
//! `canvas://workspaces` and shows the node at real size, and a navigate
//! event brings the window to that workspace and floor. `app_menu` lists and
//! invokes the same menu model the right-click menu renders
//! (`src/lib/canvas/menu/model.ts`) through the window driver.

use std::sync::Arc;
use std::time::Duration;

use omniget_core::core::canvas::model::{DockedPanel, WorkspacePanels};
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{json, Map, Value};
use tauri::AppHandle;

use super::{blocking, flag, locate, locate_arg, navigate_window, num, st};

const DOCK_DEFAULT_W: f64 = 520.0;
const DOCK_MIN_W: f64 = 280.0;
const GRID: f64 = 20.0;

fn panels_of(s: &CanvasStore, ws: &str) -> Result<WorkspacePanels, String> {
    s.workspaces()
        .into_iter()
        .find(|w| w.workspace.id == ws)
        .map(|w| w.workspace.panels.unwrap_or_default())
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: workspace {ws}"))
}

async fn save(
    s: &Arc<CanvasStore>,
    ws: &str,
    p: WorkspacePanels,
) -> Result<WorkspacePanels, String> {
    let empty = p.lifted.is_none() && p.docked.is_empty();
    let mut patch = Map::new();
    patch.insert(
        "panels".into(),
        if empty {
            Value::Null
        } else {
            serde_json::to_value(&p).map_err(|e| e.to_string())?
        },
    );
    let s = s.clone();
    let ws = ws.to_string();
    blocking(move || s.update_workspace(&ws, patch)).await?;
    Ok(p)
}

/// Width on the 20 pt grid, back to the default when close to it.
fn snap_width(w: f64) -> f64 {
    let v = (w.max(DOCK_MIN_W) / GRID).round() * GRID;
    if (v - DOCK_DEFAULT_W).abs() <= 30.0 {
        DOCK_DEFAULT_W
    } else {
        v
    }
}

fn apply_dock(p: &mut WorkspacePanels, node: &str, edge: &str, width: Option<f64>) {
    let prev = p.docked.iter().find(|d| d.node_id == node).map(|d| d.width);
    p.docked
        .retain(|d| d.node_id != node && (edge == "none" || d.edge != edge));
    if edge != "none" {
        p.docked.push(DockedPanel {
            node_id: node.to_string(),
            edge: edge.to_string(),
            width: snap_width(width.or(prev).unwrap_or(DOCK_DEFAULT_W)),
        });
        if p.lifted.as_deref() == Some(node) {
            p.lifted = None;
        }
    }
}

/// What the window shows now (best effort: the window may be closed).
async fn shown(app: &AppHandle) -> Value {
    match super::super::canvas_ui::ui_timeout(app, "state", json!({}), Duration::from_secs(4)).await
    {
        Ok(v) => {
            json!({ "lifted": v["lifted"], "docked": v["docked"], "workspaceId": v["workspaceId"] })
        }
        Err(e) => json!({ "error": e }),
    }
}

pub async fn call(
    app: &AppHandle,
    s: &Arc<CanvasStore>,
    name: &str,
    a: &Value,
) -> Result<Value, String> {
    match name {
        "canvas_node_lift" => {
            let (ws, floor, node) = locate_arg(s, a, "nodeId")?;
            let lift = flag(a, "lifted").unwrap_or(true);
            let mut p = panels_of(s, &ws)?;
            if lift {
                p.lifted = Some(node.id.clone());
                p.docked.retain(|d| d.node_id != node.id);
            } else if p.lifted.as_deref() == Some(node.id.as_str()) {
                p.lifted = None;
            }
            let p = save(s, &ws, p).await?;
            navigate_window(app, &ws, &floor, Some(&node.id));
            tokio::time::sleep(Duration::from_millis(600)).await;
            Ok(json!({
                "workspaceId": ws,
                "floorId": floor,
                "nodeId": node.id,
                "lifted": p.lifted,
                "docked": p.docked,
                "window": shown(app).await,
            }))
        }
        "canvas_node_dock" => {
            let (ws, floor, node) = locate_arg(s, a, "nodeId")?;
            let edge = st(a, "edge").unwrap_or_else(|| "right".into());
            if !matches!(edge.as_str(), "left" | "right" | "none") {
                return Err(format!(
                    "INVALID_ARGUMENT: edge '{edge}' (left, right or none)"
                ));
            }
            let mut p = panels_of(s, &ws)?;
            apply_dock(&mut p, &node.id, &edge, num(a, "width"));
            let p = save(s, &ws, p).await?;
            navigate_window(app, &ws, &floor, Some(&node.id));
            tokio::time::sleep(Duration::from_millis(600)).await;
            Ok(json!({
                "workspaceId": ws,
                "floorId": floor,
                "nodeId": node.id,
                "lifted": p.lifted,
                "docked": p.docked,
                "window": shown(app).await,
            }))
        }
        "app_menu" => {
            let mut args = json!({});
            if let Some(path) = st(a, "path") {
                args["path"] = json!(path);
            }
            if let Some(l) = flag(a, "list") {
                args["list"] = json!(l);
            }
            if let Some(at) = a.get("at").filter(|v| v.is_object()) {
                args["at"] = at.clone();
            }
            if let Some(key) = st(a, "nodeId") {
                let (ws, floor, node) = locate(s, &key)?;
                // The menu acts on the open floor: bring the window there first.
                let here = super::super::canvas_ui::ui_timeout(
                    app,
                    "state",
                    json!({}),
                    Duration::from_secs(4),
                )
                .await
                .ok();
                let open = here
                    .as_ref()
                    .map(|v| v["workspaceId"] == json!(ws) && v["floorId"] == json!(floor))
                    .unwrap_or(false);
                if !open {
                    navigate_window(app, &ws, &floor, Some(&node.id));
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
                args["nodeId"] = json!(node.id);
            }
            super::super::canvas_ui::ui(app, "menu", args).await
        }
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_replaces_the_edge_and_unlifts() {
        let mut p = WorkspacePanels {
            lifted: Some("a".into()),
            docked: vec![DockedPanel {
                node_id: "b".into(),
                edge: "right".into(),
                width: 400.0,
            }],
        };
        apply_dock(&mut p, "a", "right", Some(333.0));
        assert_eq!(p.lifted, None);
        assert_eq!(p.docked.len(), 1);
        assert_eq!(p.docked[0].node_id, "a");
        assert_eq!(p.docked[0].width, 340.0);
        apply_dock(&mut p, "a", "none", None);
        assert!(p.docked.is_empty());
        assert_eq!(snap_width(510.0), DOCK_DEFAULT_W);
        assert_eq!(snap_width(100.0), DOCK_MIN_W);
    }
}
