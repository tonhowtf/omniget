use omniget_core::core::canvas::model::Point;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::{
    arr, blocking, def, e, locate_in, n, need, num, out, ready, s, st, ty, ws, ws_floor, Def,
    ORIGIN,
};
use crate::canvas::artifacts;

pub fn defs() -> Vec<Def> {
    let placed = out(
        json!({"nodeId": ty("string"), "node": ty("object"), "artifact": ty("object")}),
        &["nodeId", "artifact"],
    );
    vec![
        def("canvas_artifact_save", "Keep an artifact (what a Claude agent produced: an HTML page, a Markdown doc, an SVG) in the workspace library and place it on the canvas. Exactly one of content (the text), path (a .html, .md or .svg file, up to 5 MB) or url (a link, e.g. https://claude.ai/artifact/..., opened as a web portal; session = a named portal session to open it in). HTML opens in a portal served read-only from the workspace, Markdown becomes a note, SVG an image. connectFrom cords it to a node (e.g. the agent that made it).", true, json!({
            "workspaceId": ws(), "floorId": s("Floor id (default: the workspace's active floor)"),
            "name": s("Artifact name"), "kind": e(&["html", "md", "svg"], "Kind (default: from the file extension or the content)"),
            "content": s("The artifact's text"), "path": s("File to copy (.html, .md, .svg)"), "url": s("Link artifact (http/https)"),
            "session": s("Named portal session for a link artifact"),
            "connectFrom": s("Node to cord it to (id or unique title)"), "place": super::b("Place it on the canvas (default true)"),
            "x": n("Left"), "y": n("Top")
        }), &["name"], out(json!({"artifact": ty("object"), "nodeId": ty("string"), "node": ty("object")}), &["artifact"])),
        def("canvas_artifact_list", "The workspace's artifact library, newest first: id, name, kind (html, md, svg, link), createdAt, source, origin, path or url.", false, json!({
            "workspaceId": ws()
        }), &[], out(json!({"artifacts": arr(ty("object"))}), &["artifacts"])),
        def("canvas_artifact_open", "Place a saved artifact on the canvas again (HTML as a portal, Markdown as a note, SVG as an image, link as a web portal).", true, json!({
            "workspaceId": ws(), "floorId": s("Floor id (default: the workspace's active floor)"), "artifactId": s("Artifact id from canvas_artifact_list"),
            "connectFrom": s("Node to cord it to (id or unique title)"), "x": n("Left"), "y": n("Top")
        }), &["artifactId"], placed),
    ]
}

pub async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    let store = ready(app);
    let (ws, floor) = ws_floor(&store, a)?;
    let connect_from = match st(a, "connectFrom") {
        Some(k) => {
            let (w2, f2, n) = locate_in(&store, &k, Some(&ws))?;
            if w2 != ws || f2 != floor {
                return Err("CANVAS_INVALID: connectFrom must be on the same floor".into());
            }
            Some(n.id)
        }
        None => None,
    };
    let at = match (num(a, "x"), num(a, "y")) {
        (Some(x), Some(y)) => Some(Point { x, y }),
        _ => None,
    };
    let result = match name {
        "canvas_artifact_list" => {
            let w = ws.clone();
            return blocking(move || {
                let list = artifacts::list(&w)?;
                Ok(json!({ "artifacts": list.iter().map(|m| artifacts::view(&w, m)).collect::<Vec<_>>() }))
            })
            .await;
        }
        "canvas_artifact_save" => {
            let spec = artifacts::SaveSpec {
                name: need(a, "name")?,
                kind: st(a, "kind"),
                content: a.get("content").and_then(Value::as_str).map(str::to_string),
                path: st(a, "path"),
                url: st(a, "url"),
                origin: ORIGIN.to_string(),
                session: st(a, "session"),
            };
            let place = a.get("place").and_then(Value::as_bool) != Some(false);
            let (w, f) = (ws.clone(), floor.clone());
            blocking(move || {
                let m = artifacts::save(&w, spec)?;
                if !place {
                    return Ok(json!({ "artifact": artifacts::view(&w, &m) }));
                }
                artifacts::place(&w, &f, &m, at, connect_from.as_deref())
            })
            .await?
        }
        "canvas_artifact_open" => {
            let id = need(a, "artifactId")?;
            let (w, f) = (ws.clone(), floor.clone());
            blocking(move || {
                let m = artifacts::get(&w, &id)?;
                artifacts::place(&w, &f, &m, at, connect_from.as_deref())
            })
            .await?
        }
        _ => return Err("TOOL_NOT_AUTHORIZED".into()),
    };
    if let Some(id) = result["nodeId"].as_str() {
        super::navigate_window(app, &ws, &floor, Some(id));
    }
    Ok(result)
}
