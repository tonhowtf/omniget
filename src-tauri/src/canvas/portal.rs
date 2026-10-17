use std::collections::{HashSet, VecDeque};

use omniget_core::core::canvas::model::{CanvasNode, FloorDoc, NodeKind};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::{layout, model, store};
use serde_json::{json, Value};
use tauri::AppHandle;

use super::rpc::{NodeCtx, RpcResult};
use crate::portal::{self, automation, Scope};

#[derive(Debug, Clone)]
struct Reach {
    node_id: String,
    name: String,
    url: Option<String>,
    kind: String,
    scope: Scope,
    perms: Vec<String>,
}

fn portal_name(n: &CanvasNode) -> String {
    n.data_str("name")
        .or(n.title.as_deref())
        .map(String::from)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            n.data_str("url")
                .and_then(|u| url::Url::parse(u).ok())
                .and_then(|u| u.host_str().map(String::from))
        })
        .unwrap_or_else(|| "Portal".into())
}

fn reachable(doc: &FloorDoc, from: &str) -> Vec<Reach> {
    let is_portal = |id: &str| doc.node(id).is_some_and(|n| n.kind == NodeKind::Portal);
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();
    for c in doc.connections.iter().filter(|c| c.touches(from)) {
        if let Some(other) = c.other_end(from) {
            if is_portal(other) && seen.insert(other.to_string()) {
                queue.push_back((other.to_string(), c.perms.clone()));
            }
        }
    }
    while let Some((id, perms)) = queue.pop_front() {
        let Some(n) = doc.node(&id) else { continue };
        out.push(Reach {
            node_id: id.clone(),
            name: portal_name(n),
            url: n.data_str("url").map(String::from),
            kind: n.data_str("portalKind").unwrap_or("web").to_string(),
            scope: Scope::parse(n.data_str("storageScope")),
            perms: perms.clone(),
        });
        for c in doc.connections.iter().filter(|c| c.touches(&id)) {
            if let Some(other) = c.other_end(&id) {
                if other != from && is_portal(other) && seen.insert(other.to_string()) {
                    queue.push_back((other.to_string(), perms.clone()));
                }
            }
        }
    }
    out
}

fn pick<'a>(list: &'a [Reach], want: Option<&str>) -> Result<&'a Reach, String> {
    let names = || {
        list.iter()
            .map(|r| format!("\"{}\"", r.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match want.map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => list
            .iter()
            .find(|r| r.node_id == w)
            .or_else(|| list.iter().find(|r| r.name.eq_ignore_ascii_case(w)))
            .ok_or_else(|| {
                if list.is_empty() {
                    "PORTAL_NOT_CONNECTED: no portal is corded to you; ask the user to connect one, or call portal.open with a url".to_string()
                } else {
                    format!("PORTAL_NOT_CONNECTED: no corded portal named \"{w}\"; corded portals: {}", names())
                }
            }),
        None => match list {
            [] => Err("PORTAL_NOT_CONNECTED: no portal is corded to you; ask the user to connect one, or call portal.open with a url".into()),
            [one] => Ok(one),
            _ => Err(format!("PORTAL_AMBIGUOUS: several corded portals, pass `portal`: {}", names())),
        },
    }
}

fn arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn may_eval(r: &Reach) -> bool {
    r.scope == Scope::Isolated || r.perms.iter().any(|p| p == "eval" || p == "raw")
}

fn app_handle() -> Result<AppHandle, String> {
    portal::app().ok_or_else(|| {
        "PORTAL_NOT_READY: the canvas has not started yet; open the World canvas once".into()
    })
}

fn reach_of(ctx: &NodeCtx) -> Result<Vec<Reach>, String> {
    super::doc::host()
        .store()
        .read(&ctx.workspace_id, &ctx.floor_id, |d| {
            reachable(d, &ctx.node_id)
        })
}

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    let verb = method.strip_prefix("portal.")?;
    Some(run(ctx, verb, args).await)
}

async fn run(ctx: &NodeCtx, verb: &str, a: Value) -> Result<Value, String> {
    let list = reach_of(ctx)?;
    if verb == "list" {
        let portals: Vec<Value> = list
            .iter()
            .map(|r| {
                let st = portal::host().state(&r.node_id);
                json!({
                    "nodeId": r.node_id,
                    "name": r.name,
                    "kind": r.kind,
                    "url": st.as_ref().map(|s| s.url.clone()).or(r.url.clone()),
                    "title": st.as_ref().map(|s| s.title.clone()),
                    "open": st.is_some(),
                    "scope": r.scope.as_str(),
                    "canEvaluate": may_eval(r),
                })
            })
            .collect();
        return Ok(json!({ "portals": portals }));
    }
    let app = app_handle()?;
    if verb == "open" {
        return open(&app, ctx, &list, &a).await;
    }
    let target = pick(&list, arg(&a, "portal"))?.clone();
    if target.kind != "web" {
        return Err(format!(
            "PORTAL_UNSUPPORTED: {} portals are device nodes, driven with the canvas_device_* MCP tools",
            target.kind
        ));
    }
    let node = target.node_id.as_str();
    if OWNER_VERBS.contains(&verb) {
        return Err(format!(
            "PORTAL_OWNER_ONLY: portal.{verb} is for the user (portal menu or MCP), not agents"
        ));
    }
    if !KNOWN_VERBS.contains(&verb) {
        return Err(format!("unknown canvas method `portal.{verb}`"));
    }
    if verb == "evaluate" && !may_eval(&target) {
        return Err(format!(
            "PORTAL_EVAL_DENIED: \"{}\" shares its logins ({} storage); evaluate needs the `eval` permission on your cord. Use snapshot/click/fill instead, or ask the user to grant it.",
            target.name,
            target.scope.as_str()
        ));
    }
    let a = if verb == "upload" {
        absolute_paths(ctx, a)?
    } else {
        a
    };
    let by = store::agent_origin(&ctx.node_id);
    drive(&app, &ctx.workspace_id, &ctx.floor_id, node, &by, verb, a).await
}

fn absolute_paths(ctx: &NodeCtx, mut a: Value) -> Result<Value, String> {
    let cwd = super::doc::host()
        .store()
        .read(&ctx.workspace_id, &ctx.floor_id, |d| {
            d.node(&ctx.node_id)
                .and_then(|n| n.data_str("cwd").map(std::path::PathBuf::from))
        })?;
    let home = dirs::home_dir();
    let fix = |p: &str| -> Result<String, String> {
        let raw = std::path::Path::new(p);
        if raw.is_absolute() {
            return Ok(p.to_string());
        }
        if let (Some(rest), Some(h)) = (p.strip_prefix("~/"), home.as_ref()) {
            return Ok(h.join(rest).to_string_lossy().into_owned());
        }
        cwd.as_ref()
            .map(|c| c.join(raw).to_string_lossy().into_owned())
            .ok_or_else(|| {
                format!("PORTAL_ARGS: `{p}` is relative and your terminal has no folder; pass an absolute path")
            })
    };
    let key = if a.get("paths").is_some() {
        "paths"
    } else if a.get("files").is_some() {
        "files"
    } else {
        "path"
    };
    let fixed = match a.get(key) {
        Some(Value::Array(v)) => Value::Array(
            v.iter()
                .map(|x| match x.as_str() {
                    Some(p) => fix(p.trim()).map(Value::String),
                    None => Ok(x.clone()),
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(Value::String(p)) if !p.trim().is_empty() => Value::String(fix(p.trim())?),
        _ => return Ok(a),
    };
    if let Some(o) = a.as_object_mut() {
        o.insert(key.into(), fixed);
    }
    Ok(a)
}

pub const KNOWN_VERBS: &[&str] = &[
    "snapshot",
    "click",
    "fill",
    "type",
    "press",
    "evaluate",
    "navigate",
    "back",
    "resize",
    "ua",
    "screenshot",
    "console",
    "hover",
    "scroll",
    "select",
    "check",
    "uncheck",
    "text",
    "html",
    "wait",
    "trust",
    "mute",
    "unmute",
    "copy_screenshot",
    "inspect",
    "upload",
];

pub const OWNER_VERBS: &[&str] = &["trust", "copy_screenshot", "inspect"];

pub async fn drive(
    app: &AppHandle,
    ws: &str,
    floor: &str,
    node: &str,
    by: &str,
    verb: &str,
    a: Value,
) -> Result<Value, String> {
    if !KNOWN_VERBS.contains(&verb) {
        return Err(format!("unknown canvas method `portal.{verb}`"));
    }
    automation::ensure_open(app, node)?;
    portal::driving(app, node, by, verb);
    let out = match verb {
        "snapshot" => automation::snapshot(app, node, &a).await,
        "click" => automation::click(app, node, &a).await,
        "fill" => automation::fill(app, node, &a).await,
        "type" => automation::type_text(app, node, &a).await,
        "press" => automation::press(app, node, &a).await,
        "evaluate" => automation::evaluate(app, node, &a).await,
        "console" => automation::console(app, node, &a).await,
        "upload" => automation::upload(app, node, &a).await,
        "hover" | "scroll" | "select" | "check" | "uncheck" | "text" | "html" | "wait" => {
            automation::element(app, node, verb, &a).await
        }
        "navigate" => {
            let url = arg(&a, "url").ok_or("PORTAL_ARGS: url is required")?;
            portal::navigate(app, node, url).await.map(|s| json!(s))
        }
        "back" => {
            let action = arg(&a, "action").unwrap_or("back");
            if !matches!(action, "back" | "forward" | "reload") {
                return Err("PORTAL_ARGS: action is back, forward or reload".into());
            }
            portal::history(app, node, action).await.map(|s| json!(s))
        }
        "resize" => resize(app, ws, floor, by, node, &a),
        "ua" => user_agent(app, ws, floor, by, node, &a).await,
        "trust" => portal::trust_cert(app, node).await.map(|s| json!(s)),
        "mute" | "unmute" => {
            let m = verb == "mute";
            let op = Op::update_node(node, json!({ "data": { "muted": m } }));
            super::doc::host()
                .store()
                .apply(ws, floor, vec![op], by, None)?;
            portal::set_muted(app, node, m).await.map(|s| json!(s))
        }
        "copy_screenshot" => {
            crate::commands::canvas::portal::canvas_portal_copy_screenshot(
                app.clone(),
                node.to_string(),
            )
            .await
        }
        "inspect" => portal::open_inspector(app, node).map(|_| json!({ "inspector": true })),
        "screenshot" => portal::screenshot(app, node).await.map(|c| {
            json!({ "path": c.path, "width": c.width, "height": c.height, "bytes": c.bytes, "page": portal::state_json(node) })
        }),
        _ => unreachable!(),
    };
    portal::driving(app, node, by, &format!("{verb}:done"));
    out
}

async fn open(app: &AppHandle, ctx: &NodeCtx, list: &[Reach], a: &Value) -> Result<Value, String> {
    let url = arg(a, "url");
    let existing = match arg(a, "portal") {
        Some(_) => Some(pick(list, arg(a, "portal"))?.clone()),
        None if url.is_none() => Some(pick(list, None)?.clone()),
        None => None,
    };
    let node_id = match existing {
        Some(r) => {
            if r.kind != "web" {
                return Err(format!(
                    "PORTAL_UNSUPPORTED: {} portals are device nodes, driven with the canvas_device_* MCP tools",
                    r.kind
                ));
            }
            r.node_id
        }
        None => create(ctx, url.unwrap_or_default(), a)?,
    };
    automation::ensure_open(app, &node_id)?;
    portal::driving(app, &node_id, &store::agent_origin(&ctx.node_id), "open");
    let st = match url {
        Some(u) => portal::navigate(app, &node_id, u).await?,
        None => portal::host()
            .state(&node_id)
            .ok_or("PORTAL_OPEN: session vanished")?,
    };
    Ok(json!({ "nodeId": node_id, "page": st }))
}

fn create(ctx: &NodeCtx, url: &str, a: &Value) -> Result<String, String> {
    let u = portal::normalize_url(url)?;
    let base = model::default_size(&NodeKind::Portal);
    let (dw, dh) = portal::app()
        .and_then(|app| super::prefs::default_size(&app, "portal"))
        .unwrap_or((base.w, base.h));
    let w = a
        .get("width")
        .and_then(|v| v.as_f64())
        .unwrap_or(dw)
        .clamp(240.0, 4096.0);
    let h = a
        .get("height")
        .and_then(|v| v.as_f64())
        .map(|h| h + automation::NODE_HEADER)
        .unwrap_or(dh)
        .clamp(180.0, 4096.0);
    let beside = portal::app()
        .is_some_and(|app| super::prefs::get(&app).defaults.portal_placement == "beside");
    let st = super::doc::host().store();
    let (ws, floor) = (ctx.workspace_id.as_str(), ctx.floor_id.as_str());
    let spot = st.read(ws, floor, |d| {
        let near = d.node(&ctx.node_id).map(|n| model::Point {
            x: n.x + n.w + 40.0,
            y: n.y,
        });
        let z = d.nodes.iter().map(|n| n.z).fold(0.0, f64::max) + 1.0;
        match near.filter(|_| beside) {
            Some(p) => (p, z),
            None => (layout::free_spot_in(d, w, h, near, &[]), z),
        }
    })?;
    let id = model::new_id();
    let name = arg(a, "name")
        .map(String::from)
        .or_else(|| u.host_str().map(String::from));
    let mut data = json!({ "url": u.to_string(), "portalKind": "web", "storageScope": "isolated" });
    if let Some(n) = &name {
        data["name"] = json!(n);
    }
    let node: CanvasNode = serde_json::from_value(json!({
        "id": id, "kind": "portal", "x": spot.0.x, "y": spot.0.y, "w": w, "h": h, "z": spot.1,
        "title": name, "data": data,
    }))
    .map_err(|e| format!("PORTAL_CREATE: {e}"))?;
    let cord: model::Connection = serde_json::from_value(json!({
        "id": model::new_id(), "from": ctx.node_id, "to": id, "kind": "portal", "perms": [],
    }))
    .map_err(|e| format!("PORTAL_CREATE: {e}"))?;
    st.apply(
        ws,
        floor,
        vec![Op::AddNode { node }, Op::AddConnection { connection: cord }],
        &store::agent_origin(&ctx.node_id),
        None,
    )?;
    Ok(id)
}

fn resize(
    app: &AppHandle,
    ws: &str,
    floor: &str,
    by: &str,
    node: &str,
    a: &Value,
) -> Result<Value, String> {
    let w = a
        .get("width")
        .and_then(|v| v.as_f64())
        .ok_or("PORTAL_ARGS: width is required")?;
    let h = a
        .get("height")
        .and_then(|v| v.as_f64())
        .ok_or("PORTAL_ARGS: height is required")?;
    let (w, h) = (w.clamp(100.0, 4096.0), h.clamp(100.0, 4096.0));
    let op = Op::update_node(node, json!({ "w": w, "h": h + automation::NODE_HEADER }));
    super::doc::host()
        .store()
        .apply(ws, floor, vec![op], by, None)?;
    portal::set_viewport(app, node, w, h)?;
    Ok(json!({ "width": w, "height": h, "page": portal::state_json(node) }))
}

async fn user_agent(
    app: &AppHandle,
    ws: &str,
    floor: &str,
    by: &str,
    node: &str,
    a: &Value,
) -> Result<Value, String> {
    let ua: Option<String> = if a.get("reset").and_then(|v| v.as_bool()) == Some(true) {
        None
    } else if let Some(v) = arg(a, "value") {
        if v.len() > 512 {
            return Err("PORTAL_ARGS: user agent longer than 512 chars".into());
        }
        Some(v.to_string())
    } else {
        match arg(a, "preset") {
            Some("desktop") | Some("default") => None,
            Some(p) => Some(
                portal::user_agent_preset(p)
                    .ok_or_else(|| format!("PORTAL_ARGS: unknown preset `{p}`"))?
                    .to_string(),
            ),
            None => return Err("PORTAL_ARGS: preset, value or reset is required".into()),
        }
    };
    let op = Op::update_node(
        node,
        json!({ "data": { "userAgent": ua.clone().map(Value::String).unwrap_or(Value::Null) } }),
    );
    super::doc::host()
        .store()
        .apply(ws, floor, vec![op], by, None)?;
    let st = portal::set_user_agent(app, node, ua).await?;
    Ok(json!(st))
}
