use std::path::{Path, PathBuf};
use std::sync::Arc;

use omniget_core::core::canvas::assets::{self, AssetRef};
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{json, Map, Value};
use tauri::AppHandle;

use super::{
    any, arr, b, def, destructive, flag, floor, i, n, need, num, out, s, spawn, st, ty, ws,
    ws_floor, ws_of, Def,
};
use crate::commands::canvas::images::{self as cmd, Source};

const SNAPSHOT_THUMBS_MAX: usize = 24;

pub const NAMES: &[&str] = &[
    "canvas_image_save",
    "canvas_asset_add",
    "canvas_asset_list",
    "canvas_asset_use",
    "canvas_asset_remove",
];

fn asset_out() -> Value {
    json!({"type": "object", "properties": {"id": ty("string"), "path": ty("string"), "w": ty("integer"), "h": ty("integer"), "bytes": ty("integer")}, "required": ["id", "path"]})
}

pub fn defs() -> Vec<Def> {
    let save_props = json!({
        "workspaceId": ws(), "floorId": floor(),
        "base64": s("Image bytes as base64 or a data: URL (PNG, JPEG, GIF, WebP; up to 25 MB)"),
        "url": s("http(s) link to download (public sites only, up to 25 MB, 20 s)"),
        "path": s("File on this computer; it is copied into the workspace library"),
        "name": s("Name shown in the library and as the node title"),
        "place": b("Also put it on the canvas as an image node (default true)"),
        "x": n("Left (world units)"), "y": n("Top (world units)"), "w": n("Width (default: fits the image in 480)"), "h": n("Height"),
        "connectFrom": s("Node id to cord the new image from")
    });
    let save_out = out(
        json!({"asset": asset_out(), "node": any(), "connection": any()}),
        &["asset"],
    );
    vec![
        def("canvas_image_save", "Save an image into the workspace library (files/assets, deduplicated by content) from base64, a URL or a file path, and by default place it on the canvas as an image node. Returns the asset ({id, path, w, h, bytes}) and the node.", true, save_props.clone(), &[], save_out.clone()),
        def("canvas_asset_add", "Same as canvas_image_save.", true, save_props, &[], save_out),
        def("canvas_asset_list", "Images in the workspace library (newest first), with the nodes that use each one.", false, json!({
            "workspaceId": ws(), "query": s("Filter by name, source URL or id prefix"), "cursor": s("nextCursor of the previous page"), "limit": i("Page size (default 50)", 1, 500)
        }), &[], out(json!({"assets": arr(ty("object")), "nextCursor": any(), "total": ty("integer")}), &["assets"])),
        def("canvas_asset_use", "Put an image from the workspace library on the canvas as an image node.", true, json!({
            "workspaceId": ws(), "floorId": floor(), "assetId": s("Asset id (or a unique prefix, or files/assets/<id>.<ext>)"),
            "x": n("Left (world units)"), "y": n("Top (world units)"), "w": n("Width"), "h": n("Height"), "name": s("Node title"), "connectFrom": s("Node id to cord the new image from")
        }), &["assetId"], out(json!({"node": ty("object"), "connection": any()}), &["node"])),
        destructive(def("canvas_asset_remove", "Remove an image from the workspace library. Refuses while a node uses it (and lists them); force moves it to files/.trash anyway, and those nodes show a placeholder.", true, json!({
            "workspaceId": ws(), "assetId": s("Asset id (or a unique prefix)"), "force": b("Remove even when nodes use it")
        }), &["assetId"], out(json!({"removed": ty("boolean"), "asset": any(), "usedBy": arr(ty("object")), "trashedTo": any()}), &["removed"]))),
    ]
}

fn short(r: &AssetRef) -> Value {
    json!({ "id": r.id, "path": r.path, "w": r.w, "h": r.h, "bytes": r.bytes })
}

fn users(s: &CanvasStore, ws: &str, r: &AssetRef) -> Vec<Value> {
    let Ok(w) = s.workspace(ws) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in &w.floors {
        let Ok(nodes) = s.read(ws, &f.id, |d| d.nodes.clone()) else {
            continue;
        };
        for nd in nodes.iter().filter(|nd| assets::uses(&nd.data, r)) {
            out.push(json!({ "nodeId": nd.id, "floorId": f.id, "title": nd.title }));
        }
    }
    out
}

fn source_of(a: &Value) -> Result<Source, String> {
    if let Some(b) = st(a, "base64") {
        return Ok(Source::Base64(b));
    }
    if let Some(p) = st(a, "path") {
        return Ok(Source::Path(p));
    }
    if let Some(u) = st(a, "url") {
        return Ok(Source::Url(u));
    }
    if let Some(id) = st(a, "asset").or_else(|| st(a, "assetId")) {
        return Ok(Source::Asset(id));
    }
    Err("MISSING_ARGUMENT: base64, url or path".into())
}

fn warm_thumb(dir: PathBuf, r: AssetRef) {
    tauri::async_runtime::spawn(async move {
        let _ = cmd::thumb_of(&dir, &r).await;
    });
}

async fn place(
    app: &AppHandle,
    s: &Arc<CanvasStore>,
    a: &Value,
    r: &AssetRef,
    title: Option<String>,
) -> Result<Value, String> {
    let mut args = Map::new();
    args.insert("kind".into(), json!("image"));
    args.insert("asset".into(), json!(r.id));
    for k in ["workspaceId", "floorId", "x", "y", "w", "h", "connectFrom"] {
        if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
            args.insert(k.into(), v.clone());
        }
    }
    if let Some(t) = title {
        args.insert("title".into(), json!(t));
    }
    spawn(app, s, &Value::Object(args)).await
}

pub async fn call(
    app: &AppHandle,
    s: &Arc<CanvasStore>,
    name: &str,
    a: &Value,
) -> Result<Value, String> {
    let ws_id = ws_of(a);
    let dir = cmd::ws_dir(s, &ws_id)?;
    match name {
        "canvas_image_save" | "canvas_asset_add" => {
            if flag(a, "place") != Some(false) {
                ws_floor(s, a)?;
            }
            let (r, source_url) = cmd::import_source(app, &dir, source_of(a)?).await?;
            let label = st(a, "name");
            assets::remember(&dir, &r.id, label.as_deref(), source_url.as_deref())?;
            warm_thumb(dir.clone(), r.clone());
            if flag(a, "place") == Some(false) {
                return Ok(json!({ "asset": short(&r) }));
            }
            let title = label.or_else(|| {
                st(a, "path").and_then(|p| {
                    Path::new(&p)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
            });
            let placed = place(app, s, a, &r, title).await?;
            Ok(
                json!({ "asset": short(&r), "node": placed["node"], "connection": placed["connection"] }),
            )
        }
        "canvas_asset_use" => {
            let id = need(a, "assetId")?;
            let d = dir.clone();
            let r = assets::blocking(move || assets::find(&d, &id)).await?;
            warm_thumb(dir.clone(), r.clone());
            let title = st(a, "name").or_else(|| r.name.clone());
            place(app, s, a, &r, title).await
        }
        "canvas_asset_list" => {
            let d = dir.clone();
            let all = assets::blocking(move || Ok(assets::list(&d))).await?;
            let q = st(a, "query").map(|q| q.to_lowercase());
            let hits: Vec<AssetRef> = all
                .into_iter()
                .filter(|r| match &q {
                    None => true,
                    Some(q) => {
                        r.id.starts_with(q.as_str())
                            || r.name
                                .as_deref()
                                .is_some_and(|n| n.to_lowercase().contains(q))
                            || r.source_url
                                .as_deref()
                                .is_some_and(|u| u.to_lowercase().contains(q))
                    }
                })
                .collect();
            let total = hits.len();
            let start = st(a, "cursor")
                .and_then(|c| c.parse::<usize>().ok())
                .unwrap_or(0)
                .min(total);
            let limit = num(a, "limit")
                .map(|l| l as usize)
                .unwrap_or(50)
                .clamp(1, 500);
            let end = (start + limit).min(total);
            let page: Vec<Value> = hits[start..end]
                .iter()
                .map(|r| {
                    let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
                    if let Some(o) = v.as_object_mut() {
                        o.remove("abs");
                        o.insert("usedBy".into(), json!(users(s, &ws_id, r)));
                    }
                    v
                })
                .collect();
            Ok(json!({
                "assets": page,
                "total": total,
                "nextCursor": if end < total { json!(end.to_string()) } else { Value::Null },
            }))
        }
        "canvas_asset_remove" => {
            let id = need(a, "assetId")?;
            let d = dir.clone();
            let r = assets::blocking(move || assets::find(&d, &id)).await?;
            let used = users(s, &ws_id, &r);
            if !used.is_empty() && flag(a, "force") != Some(true) {
                let ids: Vec<&str> = used.iter().filter_map(|u| u["nodeId"].as_str()).collect();
                return Err(format!(
                    "ASSET_IN_USE: asset {} is used by node(s) {}; remove those nodes first, or pass force:true to move it to files/.trash anyway",
                    &r.id[..12],
                    ids.join(", ")
                ));
            }
            let d = dir.clone();
            let r2 = r.clone();
            let to = assets::blocking(move || assets::trash(&d, &r2)).await?;
            Ok(json!({
                "removed": true,
                "asset": short(&r),
                "usedBy": used,
                "trashedTo": to.to_string_lossy(),
            }))
        }
        _ => Err("TOOL_NOT_AUTHORIZED".into()),
    }
}

pub async fn spawn_data(
    app: &AppHandle,
    s: &CanvasStore,
    ws: &str,
    a: &Value,
) -> Result<Map<String, Value>, String> {
    let dir = cmd::ws_dir(s, ws)?;
    let src = source_of(a)?;
    let raw_path = match &src {
        Source::Path(p) => assets::resolve_path(&dir, p),
        _ => None,
    };
    match cmd::import_source(app, &dir, src).await {
        Ok((r, source_url)) => {
            if source_url.is_some() {
                assets::remember(&dir, &r.id, None, source_url.as_deref())?;
            }
            warm_thumb(dir.clone(), r.clone());
            Ok(assets::node_data(&r, source_url.as_deref()))
        }
        Err(e) if e.starts_with("ASSET_UNSUPPORTED") => match raw_path {
            Some(p) if assets::inside(s.root(), &p) => {
                let mut d = Map::new();
                d.insert("path".into(), json!(p.to_string_lossy()));
                Ok(d)
            }
            _ => Err(e),
        },
        Err(e) => Err(e),
    }
}

pub fn natural(data: &Value) -> Option<(u32, u32)> {
    let w = data["naturalW"].as_u64()? as u32;
    let h = data["naturalH"].as_u64()? as u32;
    (w > 0 && h > 0).then_some((w, h))
}

pub async fn snapshot_thumbs(s: &CanvasStore, ws: &str, snapshot: &mut Value) {
    let Ok(dir) = cmd::ws_dir(s, ws) else {
        return;
    };
    let Some(nodes) = snapshot.get_mut("nodes").and_then(Value::as_array_mut) else {
        return;
    };
    let mut done = 0;
    for nd in nodes.iter_mut().filter(|nd| nd["kind"] == "image") {
        if done >= SNAPSHOT_THUMBS_MAX {
            nd["thumbnail"] = json!({ "skipped": "limit" });
            continue;
        }
        let key = nd["data"]["asset"]
            .as_str()
            .or_else(|| nd["data"]["path"].as_str())
            .map(str::to_string);
        let Some(key) = key else { continue };
        let d = dir.clone();
        let tiny = assets::blocking(move || {
            let r = assets::find(&d, &key)?;
            assets::thumb_for_asset(&d, &r)?;
            Ok(assets::tiny_thumb(&d, &r.id))
        })
        .await
        .ok()
        .flatten();
        if let Some(t) = tiny {
            nd["thumbnail"] = json!(t);
            done += 1;
        }
    }
}
