use std::sync::Arc;

use axum::extract::{ConnectInfo, Path, Request};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use omniget_core::core::canvas::model::{CanvasNode, TerminalData};
use omniget_core::core::canvas::store::CanvasStore;
use omniget_core::core::term::agent::AgentState;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use super::rpc::{NodeCtx, RpcResult};
use crate::local_bridge_remote::{self as lbr, Peer, RemoteDevice, Scope};

fn store() -> Arc<CanvasStore> {
    super::doc::host().store().clone()
}

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/remote/canvas",
            get(|| async { Redirect::permanent("/remote/canvas/") }),
        )
        .route("/remote/canvas/", get(page))
        .route("/remote/api/canvas/overview", get(api_overview))
        .route(
            "/remote/api/canvas/workspaces/{id}/icon",
            get(api_workspace_icon),
        )
        .route("/remote/api/canvas/nodes/{id}/screen", get(api_screen))
        .route("/remote/api/canvas/nodes/{id}/prompt", post(api_prompt))
        .route("/remote/api/canvas/nodes/{id}/approve", post(api_approve))
        .route("/remote/api/canvas/nodes/{id}/deny", post(api_deny))
        .route("/remote/api/canvas/nodes/{id}/seen", post(api_seen))
        .route("/remote/api/canvas/nodes/{id}/focus", post(api_focus))
        .route(
            "/remote/api/canvas/nodes/{id}/attachments",
            post(api_attachment).layer(axum::extract::DefaultBodyLimit::max(ATTACH_MAX + 1024)),
        )
        .route("/remote/api/canvas/nodes/{id}/files", get(api_files))
        .route(
            "/remote/api/canvas/nodes/{id}/file/preview",
            get(api_file_preview),
        )
        .route(
            "/remote/api/canvas/workspaces/{id}/nodes",
            post(api_node_create),
        )
        .route("/remote/api/canvas/mutations", post(api_mutations))
        .route("/remote/api/info", get(api_info))
        .route("/remote/api/canvas/info", get(api_info))
        .merge(super::remote_stream::routes())
        .merge(super::remote_notes::routes())
}

pub fn host_allowed(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    let name = if let Some(rest) = h.strip_prefix('[') {
        return rest
            .split(']')
            .next()
            .is_some_and(|ip| ip.parse::<std::net::Ipv6Addr>().is_ok());
    } else {
        h.rsplit_once(':')
            .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
            .map(|(n, _)| n.to_string())
            .unwrap_or(h.clone())
    };
    let name = name.trim_end_matches('.');
    name.parse::<std::net::IpAddr>().is_ok()
        || name == "localhost"
        || name.ends_with(".local")
        || name.ends_with(".ts.net")
}

#[allow(clippy::result_large_err)]
pub(super) fn gate(
    headers: &HeaderMap,
    peer: &Peer,
    drive: bool,
) -> Result<RemoteDevice, Response> {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !host_allowed(host) {
        return Err(lbr::fail_response(
            StatusCode::FORBIDDEN,
            "host_forbidden",
            "open this page by IP, .local or .ts.net address",
        ));
    }
    let d = lbr::device_of(headers, peer).ok_or_else(|| {
        lbr::fail_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid device token",
        )
    })?;
    if drive && d.scope != Scope::Drive {
        return Err(lbr::tag_device(
            lbr::fail_response(
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                "read-only device",
            ),
            &d,
        ));
    }
    Ok(d)
}

fn reply(r: Result<Value, String>, d: &RemoteDevice) -> Response {
    let resp = match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let code = e.split(':').next().unwrap_or("ERR").trim().to_string();
            let status = if code.ends_with("NOT_FOUND") {
                StatusCode::NOT_FOUND
            } else if code.ends_with("NOT_RUNNING") || code.ends_with("NOTHING_PENDING") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            lbr::fail_response(status, &code, e)
        }
    };
    lbr::tag_device(resp, d)
}

async fn blocking(
    f: impl FnOnce() -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

fn state_str(s: AgentState) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

fn terminal_of(node: &CanvasNode) -> Option<String> {
    node.data_as::<TerminalData>()
        .map(|d| d.terminal_id)
        .filter(|t| !t.is_empty())
}

fn pending_permission(node_id: &str) -> Option<String> {
    let app = super::companion::app_handle()?;
    let broker = app.state::<crate::AppState>().llm.broker();
    let request = format!("canvas:{node_id}");
    broker
        .pending_calls()
        .into_iter()
        .find(|(r, _, _)| *r == request)
        .map(|(_, call, _)| call)
}

pub fn overview() -> Result<Value, String> {
    let s = store();
    let term = crate::pty::term();
    let inbox = super::cords::host().inbox(None);
    let mut workspaces = Vec::new();
    for summary in s.workspaces() {
        let ws = summary.workspace;
        let mut floors = Vec::new();
        for floor in &ws.floors {
            let nodes = s
                .read(&ws.id, &floor.id, |d| {
                    d.nodes
                        .iter()
                        .filter(|n| n.is_terminal())
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let nodes: Vec<Value> = nodes
                .iter()
                .map(|n| {
                    let tid = terminal_of(n);
                    let info = tid.as_deref().and_then(|t| term.info(t));
                    let sig = tid.as_deref().and_then(|t| term.agent_state(t));
                    let needs = inbox.iter().find(|i| i["nodeId"] == n.id.as_str());
                    let blocked = sig.as_ref().is_some_and(|s| s.state == AgentState::Blocked);
                    let prompt = tid
                        .as_deref()
                        .filter(|_| blocked || info.as_ref().is_some_and(|i| i.alive))
                        .and_then(|t| term.screen_text(t, 40).ok())
                        .and_then(|s| pending_prompt(&s.text));
                    json!({
                        "id": n.id,
                        "name": super::cords::node_name(n),
                        "agent": n.data_str("agent").unwrap_or("shell"),
                        "floorId": floor.id,
                        "alive": info.as_ref().is_some_and(|i| i.alive),
                        "state": sig.as_ref().map(|s| state_str(s.state)).unwrap_or_else(|| "unknown".into()),
                        "stateSource": sig.as_ref().map(|s| format!("{:?}", s.source).to_lowercase()),
                        "message": sig.as_ref().and_then(|s| s.message.clone()),
                        "sinceMs": sig.as_ref().map(|s| s.since_ms),
                        "needsYou": needs.map(|i| i["reason"].clone()),
                        "permissionPending": pending_permission(&n.id).is_some(),
                        "pendingPrompt": prompt,
                    })
                })
                .collect();
            floors.push(json!({ "id": floor.id, "name": floor.name, "nodes": nodes }));
        }
        let attention = inbox
            .iter()
            .filter(|i| i["workspaceId"].as_str() == Some(ws.id.as_str()))
            .count();
        workspaces.push(json!({
            "id": ws.id, "name": ws.name, "icon": ws.icon, "activeFloorId": ws.active_floor_id,
            "floors": floors, "attentionCount": attention,
        }));
    }
    let companion: Vec<Value> = super::companion::history(None, 12, false)
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "workspaces": workspaces, "inbox": inbox, "companion": companion }))
}

fn locate(node_id: &str) -> Result<(String, String, CanvasNode), String> {
    let s = store();
    let (ws, floor) = s
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    let node = s
        .read(&ws, &floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    Ok((ws, floor, node))
}

pub fn screen(node_id: &str, lines: usize) -> Result<Value, String> {
    let (_, _, node) = locate(node_id)?;
    let tid = terminal_of(&node).ok_or("CANVAS_INVALID: not a terminal node")?;
    let snap = crate::pty::term().screen_text(&tid, lines.clamp(5, 300))?;
    Ok(json!({
        "nodeId": node_id, "name": super::cords::node_name(&node),
        "text": snap.text, "altScreen": snap.alt_screen, "changedMs": snap.changed_ms,
    }))
}

pub fn answer(node_id: &str, allow: bool) -> Result<Value, String> {
    let (_, _, node) = locate(node_id)?;
    if let Some(call) = pending_permission(node_id) {
        use omniget_core::core::llm::broker::Answer;
        let app = super::companion::app_handle().ok_or("CANVAS_NOT_RUNNING: app not ready")?;
        let broker = app.state::<crate::AppState>().llm.broker();
        let ok = broker.answer_with(&call, if allow { Answer::Once } else { Answer::Deny });
        let _ = app.emit(
            "omni://tool-resolved",
            json!({ "request_id": format!("canvas:{node_id}"), "tool_call_id": call, "allow": allow }),
        );
        return Ok(json!({ "ok": ok, "via": "permission" }));
    }
    let tid = terminal_of(&node).ok_or("CANVAS_INVALID: not a terminal node")?;
    let term = crate::pty::term();
    let blocked = term
        .agent_state(&tid)
        .is_some_and(|s| s.state == AgentState::Blocked);
    let screen = term
        .screen_text(&tid, 40)
        .map(|s| s.text)
        .unwrap_or_default();
    if !blocked && pending_prompt(&screen).is_none() {
        return Err("CANVAS_NOTHING_PENDING: the agent is not waiting for an answer".into());
    }
    let choice = choose_answer(&screen, allow);
    match &choice {
        Choice::Keys(bytes, _) => term.input(&tid, bytes)?,
        Choice::Enter => term.submit(&tid)?,
        Choice::Escape => term.key(
            &tid,
            &omniget_core::core::term::input::KeyInput {
                key: "Escape".into(),
                ..Default::default()
            },
        )?,
    }
    super::cords::host().mark_seen(node_id);
    Ok(json!({ "ok": true, "via": "terminal", "answer": choice.label() }))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Keys(Vec<u8>, String),
    Enter,
    Escape,
}

impl Choice {
    pub fn label(&self) -> String {
        match self {
            Choice::Keys(_, l) => l.clone(),
            Choice::Enter => "enter".into(),
            Choice::Escape => "escape".into(),
        }
    }
}

pub fn pending_prompt(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail = &lines[lines.len().saturating_sub(14)..];
    let yn = regex::Regex::new(r"(?i)[\[(]\s*y(es)?\s*/\s*no?\s*[\])]").ok()?;
    if let Some(l) = tail.iter().rev().find(|l| yn.is_match(l)) {
        return Some(l.trim().to_string());
    }
    let opts = menu_options(tail);
    if opts.len() >= 2 {
        let first = tail
            .iter()
            .position(|l| menu_line(l).is_some())
            .unwrap_or(0);
        let question = tail[..first]
            .iter()
            .rev()
            .find(|l| l.contains('?'))
            .map(|l| {
                l.trim()
                    .trim_matches(|c: char| "│|╭╰─".contains(c))
                    .trim()
                    .to_string()
            });
        let list = opts
            .iter()
            .map(|(n, t)| format!("{n}. {t}"))
            .collect::<Vec<_>>()
            .join(" / ");
        return Some(match question {
            Some(q) if !q.is_empty() => format!("{q} {list}"),
            _ => list,
        });
    }
    None
}

fn menu_line(l: &str) -> Option<(String, String)> {
    let re = regex::Regex::new(r"^[\s│|]*[❯>›▶→•]?\s*(\d{1,2})[.)]\s+(.+?)\s*[│|]?\s*$").ok()?;
    let c = re.captures(l)?;
    Some((c[1].to_string(), c[2].trim().to_string()))
}

fn menu_options(lines: &[&str]) -> Vec<(String, String)> {
    lines.iter().filter_map(|l| menu_line(l)).collect()
}

pub fn choose_answer(screen: &str, allow: bool) -> Choice {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail = &lines[lines.len().saturating_sub(14)..];
    let opts = menu_options(tail);
    if opts.len() >= 2 {
        let lower = |t: &str| t.to_ascii_lowercase();
        let sticky = |t: &str| {
            let t = lower(t);
            t.contains("always")
                || t.contains("don't ask")
                || t.contains("dont ask")
                || t.contains("all ")
                || t.contains("session")
                || t.contains("every")
        };
        let pick = if allow {
            opts.iter()
                .find(|(_, t)| lower(t).starts_with("yes") && !sticky(t))
                .or_else(|| opts.iter().find(|(_, t)| lower(t).starts_with("yes")))
                .or_else(|| opts.iter().find(|(_, t)| !lower(t).starts_with("no")))
        } else {
            opts.iter().find(|(_, t)| {
                let t = lower(t);
                t.starts_with("no")
                    || t.contains("deny")
                    || t.contains("reject")
                    || t.contains("cancel")
            })
        };
        return match pick {
            Some((n, t)) => Choice::Keys(n.as_bytes().to_vec(), format!("{n}. {t}")),
            None if allow => Choice::Enter,
            None => Choice::Escape,
        };
    }
    let yn = regex::Regex::new(r"(?i)[\[(]\s*y(es)?\s*/\s*no?\s*[\])]").expect("static regex");
    if tail.iter().rev().take(4).any(|l| yn.is_match(l)) {
        return if allow {
            Choice::Keys(b"y\r".to_vec(), "y".into())
        } else {
            Choice::Keys(b"n\r".to_vec(), "n".into())
        };
    }
    if allow {
        Choice::Enter
    } else {
        Choice::Escape
    }
}

async fn api_overview(ConnectInfo(peer): ConnectInfo<Peer>, headers: HeaderMap) -> Response {
    let d = match gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let mut r = blocking(overview).await;
    if let Ok(v) = r.as_mut() {
        v["scope"] = json!(d.scope);
        v["device"] = json!({ "id": d.id, "name": d.name });
    }
    reply(r, &d)
}

async fn api_workspace_icon(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let d = match gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = tauri::async_runtime::spawn_blocking(move || {
        crate::commands::canvas::workspace_meta::icon_bytes(&id)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    let resp = match r {
        Ok(Some((bytes, mime, rev))) => {
            let etag = format!("\"{rev}\"");
            let hit = headers
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v == etag);
            let mut resp = if hit {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                ([(header::CONTENT_TYPE, mime)], bytes).into_response()
            };
            if let Ok(v) = HeaderValue::from_str(&etag) {
                resp.headers_mut().insert(header::ETAG, v);
            }
            resp
        }
        Ok(None) => lbr::fail_response(
            StatusCode::NOT_FOUND,
            "no_icon",
            "the workspace has no image icon",
        ),
        Err(e) => return reply(Err(e), &d),
    };
    lbr::tag_device(resp, &d)
}

pub fn preview_mime(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "tif" | "tiff" => "image/tiff",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => return None,
    })
}

pub fn file_flags(data: &mut serde_json::Map<String, Value>) {
    let Some(path) = data.get("path").and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    let mime = preview_mime(&path).unwrap_or("");
    for (k, on) in [
        ("isImage", mime.starts_with("image/")),
        ("isVideo", mime.starts_with("video/")),
        ("isPdf", mime == "application/pdf"),
    ] {
        if on && !data.contains_key(k) {
            data.insert(k.into(), json!(true));
        }
    }
}

const PREVIEW_MAX: u64 = 32 * 1024 * 1024;

async fn api_file_preview(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let d = match gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r =
        tauri::async_runtime::spawn_blocking(move || -> Result<(Vec<u8>, &'static str), String> {
            let (_, _, node) = locate(&id)?;
            if node.kind != omniget_core::core::canvas::model::NodeKind::File {
                return Err("CANVAS_INVALID: not a file node".into());
            }
            let path = node
                .data_str("path")
                .map(str::to_string)
                .ok_or("CANVAS_INVALID: the file node has no path")?;
            let mime =
                preview_mime(&path).ok_or("CANVAS_INVALID: no preview for this kind of file")?;
            let meta = std::fs::metadata(&path).map_err(|e| format!("CANVAS_NOT_FOUND: {e}"))?;
            if meta.len() > PREVIEW_MAX {
                return Err("CANVAS_INVALID: the file is too large to preview".into());
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("CANVAS_NOT_FOUND: {e}"))?;
            Ok((bytes, mime))
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match r {
        Ok((bytes, mime)) => lbr::tag_device(
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "private, max-age=60"),
                ],
                bytes,
            )
                .into_response(),
            &d,
        ),
        Err(e) => reply(Err(e), &d),
    }
}

async fn api_screen(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let d = match gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let lines = req
        .uri()
        .query()
        .and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "lines")
                .and_then(|(_, v)| v.parse::<usize>().ok())
        })
        .unwrap_or(60);
    reply(blocking(move || screen(&id, lines)).await, &d)
}

async fn api_prompt(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let text: String = match body["segments"].as_array() {
        Some(segs) => {
            let mut out = String::new();
            for seg in segs {
                if let Some(t) = seg["text"].as_str() {
                    out.push_str(t);
                } else if let Some(a) = seg["attachmentId"].as_str() {
                    match attachment_path(a) {
                        Some(p) => {
                            if !out.is_empty() && !out.ends_with(char::is_whitespace) {
                                out.push(' ');
                            }
                            out.push_str(&p);
                            out.push(' ');
                        }
                        None => {
                            return reply(
                                Err(format!(
                                    "CANVAS_NOT_FOUND: attachment {a} (expired or unknown)"
                                )),
                                &d,
                            )
                        }
                    }
                }
            }
            out
        }
        None => body["text"].as_str().unwrap_or("").to_string(),
    };
    let text: String = text.chars().take(8_000).collect();
    if text.trim().is_empty() {
        return reply(Err("CANVAS_INVALID: empty prompt".into()), &d);
    }
    let mutation = body["mutationId"].as_str().map(str::to_string);
    let r = blocking(move || {
        super::companion::send_to_node(&id, &text)?;
        super::cords::host().mark_seen(&id);
        Ok(json!({ "ok": true, "mutationId": mutation }))
    })
    .await;
    reply(r, &d)
}

async fn api_approve(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    reply(blocking(move || answer(&id, true)).await, &d)
}

async fn api_deny(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    reply(blocking(move || answer(&id, false)).await, &d)
}

async fn api_seen(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    super::cords::host().mark_seen(&id);
    let term = crate::pty::term();
    let attention = locate(&id)
        .ok()
        .and_then(|(_, _, n)| terminal_of(&n))
        .and_then(|t| term.agent_state(&t))
        .is_some_and(|s| s.state == AgentState::Blocked);
    reply(Ok(json!({ "ok": true, "stillBlocked": attention })), &d)
}

async fn api_focus(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = async {
        let (ws, floor, node) = locate(&id)?;
        let app = super::companion::app_handle().ok_or("CANVAS_NOT_RUNNING: app not ready")?;
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        let _ = app.emit(
            crate::mcp::canvas::EVENT_NAVIGATE,
            json!({ "workspaceId": ws, "floorId": floor, "nodeId": id }),
        );
        let ui = crate::mcp::canvas_ui::ui(
            app,
            "focus",
            json!({ "nodeId": id, "workspaceId": ws, "floorId": floor }),
        )
        .await
        .unwrap_or(Value::Null);
        super::cords::host().mark_seen(&id);
        Ok::<Value, String>(
            json!({ "ok": true, "nodeId": id, "name": super::cords::node_name(&node), "ui": ui }),
        )
    }
    .await;
    reply(r, &d)
}

pub const ATTACH_MAX: usize = 8 * 1024 * 1024;
const ATTACH_LIVE: usize = 64;
const ATTACH_TTL_MS: i64 = 15 * 60 * 1000;

fn attach_dir() -> Option<std::path::PathBuf> {
    let d = omniget_core::core::paths::app_data_dir()?
        .join("remote")
        .join("attachments");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn attach_sweep() -> Vec<std::path::PathBuf> {
    let Some(dir) = attach_dir() else {
        return vec![];
    };
    let now = std::time::SystemTime::now();
    let mut live: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        let t = e.metadata().and_then(|m| m.modified()).unwrap_or(now);
        let age = now
            .duration_since(t)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if age > ATTACH_TTL_MS {
            let _ = std::fs::remove_file(&p);
        } else {
            live.push((t, p));
        }
    }
    live.sort();
    live.into_iter().map(|(_, p)| p).collect()
}

fn attachment_path(id: &str) -> Option<String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) || id.is_empty() {
        return None;
    }
    attach_sweep()
        .into_iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{id}-")))
        })
        .map(|p| p.to_string_lossy().into_owned())
}

fn clean_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("file");
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    if s.trim_matches('.').is_empty() {
        "file".into()
    } else {
        s
    }
}

async fn api_attachment(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    if let Err(e) = locate(&id) {
        return reply(Err(e), &d);
    }
    let name = req
        .uri()
        .query()
        .and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "filename")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_else(|| "attachment".into());
    let bytes = match axum::body::to_bytes(req.into_body(), ATTACH_MAX).await {
        Ok(b) => b,
        Err(_) => {
            return reply(
                Err("CANVAS_TOO_LARGE: attachments are up to 8 MiB".into()),
                &d,
            );
        }
    };
    if bytes.is_empty() {
        return reply(Err("CANVAS_INVALID: empty attachment".into()), &d);
    }
    let r = blocking(move || {
        let live = attach_sweep();
        if live.len() >= ATTACH_LIVE {
            let _ = std::fs::remove_file(&live[0]);
        }
        let dir = attach_dir().ok_or("CANVAS_IO: no app data dir")?;
        let aid = {
            let mut b = [0u8; 8];
            rand::Rng::fill_bytes(&mut rand::rng(), &mut b);
            hex::encode(b)
        };
        let path = dir.join(format!("{aid}-{}", clean_filename(&name)));
        std::fs::write(&path, &bytes).map_err(|e| format!("CANVAS_IO: {e}"))?;
        let expires = chrono::Utc::now() + chrono::Duration::milliseconds(ATTACH_TTL_MS);
        Ok(json!({
            "attachmentId": aid,
            "path": path.to_string_lossy(),
            "size": bytes.len(),
            "expiresAt": expires.to_rfc3339(),
        }))
    })
    .await;
    reply(r, &d)
}

async fn api_files(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let d = match gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let query = req
        .uri()
        .query()
        .and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == "query")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_default();
    let r = blocking(move || files_for(&id, &query, 50)).await;
    reply(r, &d)
}

fn node_dir(node_id: &str) -> Result<std::path::PathBuf, String> {
    let (ws, _, node) = locate(node_id)?;
    let term_cwd = terminal_of(&node)
        .and_then(|t| crate::pty::term().info(&t))
        .and_then(|i| i.osc_cwd.or(i.cwd));
    let dir = term_cwd
        .or_else(|| node.data_str("cwd").map(String::from))
        .or_else(|| {
            store()
                .workspace(&ws)
                .ok()
                .and_then(|w| w.working_directory)
        })
        .ok_or("CANVAS_INVALID: this terminal has no folder")?;
    let dir = dir.strip_prefix("file://").map(String::from).unwrap_or(dir);
    Ok(std::path::PathBuf::from(dir))
}

pub fn files_for(node_id: &str, query: &str, limit: usize) -> Result<Value, String> {
    let root = node_dir(node_id)?;
    if !root.is_dir() {
        return Err(format!("CANVAS_NOT_FOUND: folder {}", root.display()));
    }
    let q = query.to_lowercase();
    let mut hits: Vec<String> = Vec::new();
    let git = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .ok()
        .filter(|o| o.status.success());
    match git {
        Some(o) => {
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                if l.to_lowercase().contains(&q) {
                    hits.push(l.to_string());
                    if hits.len() >= limit {
                        break;
                    }
                }
            }
        }
        None => {
            let mut stack = vec![(root.clone(), 0usize)];
            let mut seen = 0usize;
            while let Some((dir, depth)) = stack.pop() {
                for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    seen += 1;
                    if seen > 20_000 || hits.len() >= limit {
                        break;
                    }
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') || name == "node_modules" || name == "target" {
                        continue;
                    }
                    let rel = p
                        .strip_prefix(&root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .into_owned();
                    if p.is_dir() {
                        if depth < 8 {
                            stack.push((p, depth + 1));
                        }
                    } else if rel.to_lowercase().contains(&q) {
                        hits.push(rel);
                    }
                }
            }
        }
    }
    hits.sort_by_key(|h| {
        let name = h.rsplit('/').next().unwrap_or(h).to_lowercase();
        (!name.contains(&q), h.len())
    });
    Ok(json!({ "root": root.to_string_lossy(), "files": hits }))
}

fn mutation_origin(id: Option<&str>) -> (String, String) {
    let id = id
        .filter(|m| {
            !m.is_empty()
                && m.len() <= 64
                && m.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        })
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut b = [0u8; 8];
            rand::Rng::fill_bytes(&mut rand::rng(), &mut b);
            hex::encode(b)
        });
    (format!("remote:{id}"), id)
}

async fn api_node_create(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(ws): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = blocking(move || {
        let s = store();
        let meta = s.workspace(&ws)?;
        let floor = body["floorId"]
            .as_str()
            .map(str::to_string)
            .unwrap_or(meta.active_floor_id.clone());
        let (origin, mid) = mutation_origin(body["mutationId"].as_str());
        let src = if body["node"].is_object() { body["node"].clone() } else { body.clone() };
        let kind = src["kind"].as_str().unwrap_or("note").to_string();
        if matches!(kind.as_str(), "terminal" | "agent") {
            return Err("CANVAS_INVALID: terminals are started from the app or the Lead".into());
        }
        let (w, h) = (
            src["w"].as_f64().unwrap_or(320.0).clamp(40.0, 4096.0),
            src["h"].as_f64().unwrap_or(220.0).clamp(40.0, 4096.0),
        );
        let (x, y, z) = s.read(&ws, &floor, |doc| {
            let z = doc.nodes.iter().map(|n| n.z).fold(0.0, f64::max) + 1.0;
            let spot = omniget_core::core::canvas::layout::free_spot_in(doc, w, h, None, &[]);
            (src["x"].as_f64().unwrap_or(spot.x), src["y"].as_f64().unwrap_or(spot.y), z)
        })?;
        let id = omniget_core::core::canvas::model::new_id();
        let node: CanvasNode = serde_json::from_value(json!({
            "id": id, "kind": kind, "x": x, "y": y, "w": w, "h": h, "z": z,
            "title": src["title"], "data": if src["data"].is_object() { src["data"].clone() } else { json!({}) },
        }))
        .map_err(|e| format!("CANVAS_INVALID: node: {e}"))?;
        let res = s.apply(
            &ws,
            &floor,
            vec![omniget_core::core::canvas::ops::Op::AddNode { node }],
            &origin,
            None,
        )?;
        Ok(json!({ "nodeId": id, "workspaceId": ws, "floorId": floor, "mutationId": mid, "rev": res.rev, "seq": res.seq }))
    })
    .await;
    reply(r, &d)
}

fn starts_a_process(op: &omniget_core::core::canvas::ops::Op) -> bool {
    use omniget_core::core::canvas::ops::Op;
    match op {
        Op::AddNode { node } => node.is_terminal(),
        Op::UpdateNode { patch, .. } => {
            patch.contains_key("kind")
                || patch
                    .get("data")
                    .and_then(Value::as_object)
                    .is_some_and(|data| {
                        ["command", "cwd", "env", "terminalId"]
                            .iter()
                            .any(|k| data.contains_key(*k))
                    })
        }
        _ => false,
    }
}

async fn api_mutations(
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let d = match gate(&headers, &peer, true) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = blocking(move || {
        let s = store();
        let ws = body["workspaceId"].as_str().ok_or("CANVAS_INVALID: workspaceId")?.to_string();
        let meta = s.workspace(&ws)?;
        let floor = body["floorId"].as_str().map(str::to_string).unwrap_or(meta.active_floor_id.clone());
        let ops: Vec<omniget_core::core::canvas::ops::Op> =
            serde_json::from_value(body["ops"].clone()).map_err(|e| format!("CANVAS_INVALID: ops: {e}"))?;
        if ops.is_empty() || ops.len() > 200 {
            return Err("CANVAS_INVALID: 1 to 200 ops".into());
        }
        if ops.iter().any(starts_a_process) {
            return Err("CANVAS_INVALID: terminals are started from the app or the Lead".into());
        }
        let (origin, mid) = mutation_origin(body["mutationId"].as_str());
        let res = s.apply(&ws, &floor, ops, &origin, body["baseRev"].as_u64())?;
        Ok(json!({ "workspaceId": ws, "floorId": floor, "mutationId": mid, "rev": res.rev, "seq": res.seq }))
    })
    .await;
    reply(r, &d)
}

async fn api_info(headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !host_allowed(host) {
        return lbr::fail_response(
            StatusCode::FORBIDDEN,
            "host_forbidden",
            "open this page by IP, .local or .ts.net address",
        );
    }
    let tls = lbr::tls_info();
    Json(json!({
        "app": "OmniGet",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": 1,
        "hostName": whoami_host(),
        "auth": { "type": "bearer", "pairing": ["code6", "qr"], "roles": ["guest", "owner"] },
        "tls": tls,
        "limits": { "requestBytes": ATTACH_MAX, "attachmentTtlSecs": ATTACH_TTL_MS / 1000, "attachmentsLive": ATTACH_LIVE },
        "capabilities": [
            "overview", "screen", "prompt", "segments", "attachments", "files", "approve", "deny",
            "seen", "focus", "nodes.create", "mutations", "mutationId", "stream", "terminalStream",
            "resize", "presence", "cameraControl", "workspaceIcon", "floorWake"
        ],
    }))
    .into_response()
}

fn whoami_host() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

async fn page(headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !host_allowed(host) {
        return (
            StatusCode::FORBIDDEN,
            "open this page by IP, .local or .ts.net address",
        )
            .into_response();
    }
    let nonce = {
        let mut b = [0u8; 16];
        rand::Rng::fill_bytes(&mut rand::rng(), &mut b);
        hex::encode(b)
    };
    let csp = format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; \
connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    let body = PAGE.replace("{{NONCE}}", &nonce);
    let mut r = body.into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&csp) {
        h.insert("content-security-policy", v);
    }
    r
}

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    match method {
        "env.cli" => Some(env_cli(ctx, args).await),
        "env.get" => Some(env_get(&ctx.workspace_id, Some(&ctx.node_id))),
        "env.test" => Some(env_test(&ctx.workspace_id, Some(&ctx.node_id)).await),
        "env.providers" => Some(Ok(json!(envs::providers(&store())))),
        "env.set" => {
            let manager = locate(&ctx.node_id)
                .map(|(_, _, n)| n.data.get("isManager").and_then(|v| v.as_bool()) == Some(true))
                .unwrap_or(false);
            if !manager {
                return Some(Err(
                    "CANVAS_FORBIDDEN: only the Lead (or the user) changes environments".into(),
                ));
            }
            let target = args
                .get("terminal")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let value = args
                .get("argv")
                .and_then(|v| v.get(0))
                .cloned()
                .or_else(|| args.get("environment").cloned())
                .unwrap_or(Value::Null);
            let mut v = value;
            for k in ["dir", "user", "runtime", "sshOptions"] {
                if let (Some(x), Some(o)) = (args.get(k).cloned(), v.as_str().map(str::to_string)) {
                    let mut rec = serde_json::to_value(
                        envs::record_from_value(&Value::String(o)).unwrap_or_default(),
                    )
                    .unwrap_or(Value::Null);
                    rec[k] = x;
                    v = rec;
                } else if let (Some(x), Some(obj)) = (args.get(k).cloned(), v.as_object_mut()) {
                    obj.insert(k.into(), x);
                }
            }
            Some(match target {
                Some(t) => {
                    let id = if t == "self" || t == "me" {
                        ctx.node_id.clone()
                    } else {
                        store()
                            .read(&ctx.workspace_id, &ctx.floor_id, |d| {
                                d.nodes
                                    .iter()
                                    .find(|n| {
                                        n.id == t
                                            || super::cords::node_name(n).eq_ignore_ascii_case(&t)
                                    })
                                    .map(|n| n.id.clone())
                            })
                            .ok()
                            .flatten()
                            .unwrap_or(t)
                    };
                    env_set_terminal(&id, v).await
                }
                None => env_set_workspace(&ctx.workspace_id, v),
            })
        }
        _ => None,
    }
}

use omniget_core::core::canvas::envs;

pub fn env_get(ws: &str, node: Option<&str>) -> Result<Value, String> {
    let s = store();
    s.workspace(ws)?;
    let rec = envs::load(&s, ws);
    let mut out = json!({
        "workspaceId": ws,
        "environment": rec,
        "preview": envs::preview(&envs::resolve(&s, rec.clone())),
        "providers": envs::providers(&s),
    });
    if let Some(n) = node {
        let own = locate(n)
            .ok()
            .and_then(|(_, _, node)| node.data.get("environment").cloned());
        let eff = envs::load_for(&s, ws, Some(n));
        out["terminal"] = json!({
            "nodeId": n,
            "override": own.unwrap_or(Value::Null),
            "effective": eff,
            "notice": eff.notice,
        });
    }
    Ok(out)
}

pub fn env_set_workspace(ws: &str, value: Value) -> Result<Value, String> {
    let s = store();
    let rec = envs::record_from_value(&value)?;
    let saved = envs::save(&s, ws, rec)?;
    Ok(json!({ "workspaceId": ws, "environment": saved }))
}

pub async fn env_set_terminal(node_id: &str, value: Value) -> Result<Value, String> {
    let s = store();
    let (ws, floor, node) = locate(node_id)?;
    if !node.is_terminal() {
        return Err("CANVAS_INVALID: only terminals have an environment".into());
    }
    let stored = match &value {
        Value::Null => Value::Null,
        Value::String(x) if x == "workspace" || x.is_empty() => Value::Null,
        Value::String(x) if x == "local" => json!("local"),
        other => {
            let rec = envs::record_from_value(other)?;
            envs::validate(&s, &rec)?;
            if rec.kind == envs::EnvKind::Local {
                json!("local")
            } else {
                serde_json::to_value(&rec).map_err(|e| e.to_string())?
            }
        }
    };
    if node.data.get("environment").cloned().unwrap_or(Value::Null) == stored {
        return Ok(json!({ "nodeId": node_id, "environment": stored, "restarted": false }));
    }
    let op = omniget_core::core::canvas::ops::Op::update_node(
        node_id,
        json!({ "data": { "environment": stored } }),
    );
    s.apply(&ws, &floor, vec![op], "system:env", None)?;
    let restarted = restart_terminal(&ws, &floor, node_id).await?;
    Ok(json!({ "nodeId": node_id, "environment": stored, "restarted": restarted }))
}

async fn restart_terminal(ws: &str, floor: &str, node_id: &str) -> Result<bool, String> {
    let (_, _, node) = locate(node_id)?;
    let Some(tid) = terminal_of(&node) else {
        return Ok(false);
    };
    let term = crate::pty::term();
    if term.info(&tid).is_none() {
        return Ok(false);
    }
    term.close(&tid, false)?;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    super::launch::ensure_running(ws, floor, node_id).await?;
    Ok(true)
}

pub async fn env_test(ws: &str, node: Option<&str>) -> Result<Value, String> {
    let s = store();
    let rec = envs::load_for(&s, ws, node);
    let port = super::companion::app_handle()
        .and_then(|app| crate::local_bridge::ensure_running_for_canvas(app).ok());
    let t = envs::test_bridge(&rec, port).await;
    Ok(json!({ "environment": rec, "test": t, "notice": rec.notice }))
}

fn host_cli() -> std::path::PathBuf {
    let name = if cfg!(windows) {
        "omniget.exe"
    } else {
        "omniget"
    };
    if let Some(p) = omniget_core::core::paths::app_data_dir()
        .map(|d| d.join("bin").join(name))
        .filter(|p| p.is_file())
    {
        return p;
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()))
        .map(|d| {
            let cli = d.join(if cfg!(windows) {
                "omniget-cli.exe"
            } else {
                "omniget-cli"
            });
            if cli.is_file() {
                cli
            } else {
                d.join(name)
            }
        })
        .unwrap_or_else(|| name.into())
}

async fn env_cli(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    use base64::Engine as _;
    let argv: Vec<String> = args
        .get("argv")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .ok_or("CANVAS_INVALID: argv is required")?;
    if argv.first().map(String::as_str) != Some("canvas") {
        return Err("CANVAS_INVALID: only `omniget canvas ...` runs through the bridge".into());
    }
    if argv.iter().any(|a| a.len() > 64 * 1024) || argv.len() > 256 {
        return Err("CANVAS_INVALID: argv too large".into());
    }
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .ok_or("CANVAS_INVALID: token is required")?
        .to_string();
    let app = super::companion::app_handle().ok_or("CANVAS_NOT_RUNNING: app not ready")?;
    let port = crate::local_bridge::ensure_running_for_canvas(app)?;
    let mut cmd = tokio::process::Command::new(host_cli());
    cmd.args(&argv)
        .env("OMNIGET_CANVAS", "1")
        .env("OMNIGET_CANVAS_NODE", &ctx.node_id)
        .env("OMNIGET_CANVAS_WORKSPACE", &ctx.workspace_id)
        .env("OMNIGET_CANVAS_FLOOR", &ctx.floor_id)
        .env("OMNIGET_CANVAS_TOKEN", &token)
        .env("OMNIGET_BRIDGE_URL", format!("http://127.0.0.1:{port}"))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(900), cmd.output())
        .await
        .map_err(|_| "CANVAS_TIMEOUT: the command ran for more than 15 minutes".to_string())?
        .map_err(|e| format!("CANVAS_CLI: {e}"))?;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    Ok(json!({
        "code": out.status.code().unwrap_or(1),
        "out": b64(&out.stdout),
        "err": b64(&out.stderr),
    }))
}

const PAGE: &str = r###"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="color-scheme" content="light dark">
<title>OmniGet Canvas</title>
<style nonce="{{NONCE}}">
:root{--bg:#f5f5f7;--card:#fff;--text:#1d1d1f;--muted:#6e6e73;--border:#d2d2d7;--accent:#0a84ff;--bad:#d96a62;--ok:#5ca97a;--warn:#dcab3c}
@media (prefers-color-scheme:dark){:root{--bg:#000;--card:#1c1c1e;--text:#f5f5f7;--muted:#98989d;--border:#38383a}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.4 -apple-system,system-ui,sans-serif;padding:12px 16px calc(24px + env(safe-area-inset-bottom))}
h1{font-size:20px;margin:4px 0 12px;display:flex;justify-content:space-between;align-items:center;gap:8px}
h2{font-size:13px;text-transform:uppercase;letter-spacing:.04em;color:var(--muted);margin:18px 0 6px}
.card{background:var(--card);border:1px solid var(--border);border-radius:12px;padding:10px 12px;margin:6px 0}
.row{display:flex;align-items:center;gap:8px;justify-content:space-between}
.node{cursor:pointer}
.dot{width:10px;height:10px;border-radius:50%;flex:none;background:#8a8a8a}
.name{font-weight:600;flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.muted{color:var(--muted);font-size:13px}
.badge{font-size:11px;padding:2px 7px;border-radius:99px;border:1px solid var(--border);color:var(--muted)}
.badge.warn{border-color:var(--bad);color:var(--bad)}
button{font:inherit;border:0;border-radius:10px;padding:9px 14px;background:var(--accent);color:#fff;min-height:40px}
button.ghost{background:transparent;color:var(--accent);border:1px solid var(--border)}
button.bad{background:var(--bad)}
button:disabled{opacity:.5}
input,textarea{font:inherit;width:100%;border:1px solid var(--border);border-radius:10px;padding:10px;background:var(--card);color:var(--text)}
textarea{min-height:80px;resize:vertical}
pre{white-space:pre-wrap;word-break:break-word;font:12px/1.35 ui-monospace,Menlo,monospace;background:var(--card);border:1px solid var(--border);border-radius:10px;padding:10px;max-height:55vh;overflow:auto;margin:8px 0}
.actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:8px}
.sugg{background:transparent;color:var(--text);border:1px solid var(--border);text-align:left;width:100%;margin-top:6px}
.err{color:var(--bad);font-size:13px;min-height:1em}
.hidden{display:none}
</style>
</head>
<body>
<h1><span>OmniGet Canvas</span><span id="scope" class="badge"></span></h1>
<section id="pair" class="hidden">
  <div class="card">
    <p>Type the 6-digit code shown in OmniGet (Canvas, Share, Phone).</p>
    <input id="code" inputmode="numeric" autocomplete="one-time-code" maxlength="6" placeholder="123456">
    <p></p>
    <input id="devname" placeholder="Device name (optional)" maxlength="60">
    <div class="actions"><button id="pairbtn">Pair</button></div>
    <p id="pairerr" class="err"></p>
  </div>
</section>
<section id="main" class="hidden">
  <p id="err" class="err"></p>
  <h2>Needs you</h2><div id="inbox"></div>
  <h2>Omni</h2><div id="omni"></div>
  <div id="wss"></div>
  <div class="actions"><button id="logout" class="ghost">Forget this device</button></div>
</section>
<section id="detail" class="hidden">
  <div class="row"><button id="back" class="ghost">Back</button><span id="dstate" class="badge"></span></div>
  <h2 id="dname"></h2>
  <pre id="screen"></pre>
  <div id="drive">
    <div class="actions" id="answer"><button id="approve">Approve</button><button id="deny" class="bad">Deny</button></div>
    <textarea id="prompt" placeholder="Message to the agent"></textarea>
    <div class="actions"><button id="send">Send</button><button id="seen" class="ghost">Mark seen</button></div>
  </div>
  <p id="derr" class="err"></p>
</section>
<script nonce="{{NONCE}}">
(function(){
"use strict";
var KEY="omniget.canvasRemote.token";
var token=null; try{token=localStorage.getItem(KEY);}catch(e){}
var scope="read", current=null, timer=null, data=null;
var COLORS={idle:"#a199ab",thinking:"#4f9faf",working:"#dcab3c",waiting_peer:"#7f8fc9",blocked:"#d96a62",success:"#5ca97a",failed:"#c0504d",compacting:"#b48ee0",looping:"#e8a33c",unknown:"#8a8a8a"};
function $(id){return document.getElementById(id);}
function el(tag,cls,text){var e=document.createElement(tag);if(cls)e.className=cls;if(text!=null)e.textContent=text;return e;}
function show(id){["pair","main","detail"].forEach(function(s){$(s).classList.toggle("hidden",s!==id);});}
function api(method,path,body){
  var h={"Content-Type":"application/json"}; if(token)h.Authorization="Bearer "+token;
  return fetch(path,{method:method,headers:h,body:body?JSON.stringify(body):undefined}).then(function(r){
    return r.json().catch(function(){return {};}).then(function(j){
      if(r.status===401){forget();throw new Error("This device is no longer paired.");}
      if(!r.ok)throw new Error(j.message||("HTTP "+r.status)); return j;});
  });
}
function forget(){token=null;try{localStorage.removeItem(KEY);}catch(e){} stop(); show("pair");}
function stop(){if(timer){clearInterval(timer);timer=null;}}
function pair(){
  var code=$("code").value.replace(/\D/g,""); $("pairerr").textContent="";
  if(code.length!==6){$("pairerr").textContent="The code has 6 digits.";return;}
  api("POST","/remote/api/pair",{secret:code,name:$("devname").value||undefined}).then(function(j){
    token=j.token; try{localStorage.setItem(KEY,token);}catch(e){} start();
  }).catch(function(e){$("pairerr").textContent=e.message;});
}
function nodeRow(n,ws){
  var c=el("div","card node"); var r=el("div","row");
  var d=el("span","dot"); d.style.background=COLORS[n.state]||COLORS.unknown; r.appendChild(d);
  r.appendChild(el("span","name",n.name));
  if(n.needsYou||n.permissionPending)r.appendChild(el("span","badge warn",n.permissionPending?"permission":String(n.needsYou)));
  else r.appendChild(el("span","badge",n.alive?n.state.replace("_"," "):"stopped"));
  c.appendChild(r);
  if(n.message)c.appendChild(el("div","muted",n.message));
  c.addEventListener("click",function(){open(n);});
  return c;
}
function findNode(id){
  var out=null; (data&&data.workspaces||[]).forEach(function(w){w.floors.forEach(function(f){f.nodes.forEach(function(n){if(n.id===id)out=n;});});}); return out;
}
function render(){
  $("scope").textContent=scope==="drive"?"full control":"read-only";
  var inbox=$("inbox"); inbox.textContent="";
  (data.inbox||[]).forEach(function(i){var n=findNode(i.nodeId); if(n)inbox.appendChild(nodeRow(n));});
  if(!inbox.childNodes.length)inbox.appendChild(el("div","muted","Nothing needs you."));
  var omni=$("omni"); omni.textContent="";
  (data.companion||[]).slice(0,5).forEach(function(e){
    var c=el("div","card"); c.appendChild(el("div","name",e.name));
    c.appendChild(el("div",null,e.summary));
    if(scope==="drive")(e.suggestions||[]).forEach(function(s){
      var b=el("button","sugg",s); b.addEventListener("click",function(){
        b.disabled=true; api("POST","/remote/api/canvas/nodes/"+encodeURIComponent(e.nodeId)+"/prompt",{text:s}).then(function(){b.textContent="Sent: "+s;}).catch(function(x){b.disabled=false;$("err").textContent=x.message;});
      }); c.appendChild(b);});
    omni.appendChild(c);
  });
  if(!omni.childNodes.length)omni.appendChild(el("div","muted","No summaries yet."));
  var wss=$("wss"); wss.textContent="";
  (data.workspaces||[]).forEach(function(w){
    w.floors.forEach(function(f){
      if(!f.nodes.length)return;
      wss.appendChild(el("h2",null,w.floors.length>1?(w.name+" / "+f.name):w.name));
      f.nodes.forEach(function(n){wss.appendChild(nodeRow(n));});
    });
  });
}
function refresh(){
  return api("GET","/remote/api/canvas/overview").then(function(j){
    data=j; scope=j.scope||"read"; $("err").textContent="";
    if(current){var n=findNode(current.id); if(n){current=n; renderDetail();}}
    else render();
  }).catch(function(e){$("err").textContent=e.message;});
}
function renderDetail(){
  $("dname").textContent=current.name;
  $("dstate").textContent=current.permissionPending?"permission":(current.needsYou?String(current.needsYou):current.state.replace("_"," "));
  $("drive").classList.toggle("hidden",scope!=="drive");
  $("answer").classList.toggle("hidden",!(current.permissionPending||current.state==="blocked"));
}
function loadScreen(){
  if(!current)return;
  api("GET","/remote/api/canvas/nodes/"+encodeURIComponent(current.id)+"/screen?lines=80").then(function(j){
    var pre=$("screen"); var atEnd=pre.scrollTop+pre.clientHeight>=pre.scrollHeight-8;
    pre.textContent=j.text||""; if(atEnd)pre.scrollTop=pre.scrollHeight;
  }).catch(function(e){$("derr").textContent=e.message;});
}
function open(n){current=n;$("derr").textContent="";$("screen").textContent="";show("detail");renderDetail();loadScreen();}
function act(path,body){
  $("derr").textContent="";
  return api("POST","/remote/api/canvas/nodes/"+encodeURIComponent(current.id)+"/"+path,body).then(function(){refresh();loadScreen();}).catch(function(e){$("derr").textContent=e.message;});
}
function start(){
  show("main"); refresh(); stop();
  timer=setInterval(function(){if(document.hidden)return; refresh(); if(current)loadScreen();},3000);
}
$("pairbtn").addEventListener("click",pair);
$("back").addEventListener("click",function(){current=null;show("main");render();});
$("approve").addEventListener("click",function(){act("approve");});
$("deny").addEventListener("click",function(){act("deny");});
$("seen").addEventListener("click",function(){act("seen");});
$("send").addEventListener("click",function(){var t=$("prompt").value.trim(); if(!t)return; act("prompt",{text:t}).then(function(){$("prompt").value="";});});
$("logout").addEventListener("click",function(){api("POST","/remote/api/logout").finally(forget);});
if(token)start(); else show("pair");
})();
</script>
</body>
</html>
"###;
