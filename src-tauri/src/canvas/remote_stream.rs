use std::convert::Infallible;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Request};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::StoreEvent;
use serde_json::{json, Value};
use tauri::{Emitter, Listener};
use tokio::sync::mpsc;

use crate::local_bridge_remote::{self as lbr, Peer, Scope};

pub const EVENT_PRESENCE: &str = "canvas://presence";
pub const EVENT_REMOTE_CAMERA: &str = "canvas://remote-camera";

const KEEPALIVE: Duration = Duration::from_secs(15);
const QUEUE: usize = 512;

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/remote/api/canvas/stream", get(api_stream))
        .route("/remote/api/canvas/feed", post(api_feed))
        .route(
            "/remote/api/canvas/nodes/{id}/stream",
            get(api_terminal_stream),
        )
}

pub fn changes(ops: &[Op]) -> Vec<Value> {
    ops.iter()
        .filter_map(|o| {
            let v = serde_json::to_value(o).ok()?;
            let op = v["op"].as_str().unwrap_or("").to_string();
            let id = v["id"]
                .as_str()
                .or_else(|| v["node"]["id"].as_str())
                .or_else(|| v["connection"]["id"].as_str())
                .or_else(|| v["drawing"]["id"].as_str())
                .map(str::to_string);
            Some(json!({ "op": op, "id": id }))
        })
        .collect()
}

pub fn frame(v: &Value) -> String {
    format!("data: {}\n\n", v)
}

struct Subs {
    store_id: u64,
    app_ids: Vec<tauri::EventId>,
}

impl Drop for Subs {
    fn drop(&mut self) {
        super::doc::host().store().unsubscribe(self.store_id);
        if let Some(app) = super::companion::app_handle() {
            for id in self.app_ids.drain(..) {
                app.unlisten(id);
            }
        }
    }
}

fn query_of(req: &Request, key: &str) -> Option<String> {
    req.uri().query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    })
}

async fn wake_floor(ws: &str, floor: &str) -> usize {
    let nodes: Vec<String> = super::doc::host()
        .store()
        .read(ws, floor, |d| {
            d.nodes
                .iter()
                .filter(|n| n.is_terminal())
                .map(|n| n.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut n = 0;
    for id in nodes {
        if super::launch::ensure_running(ws, floor, &id).await.is_ok() {
            n += 1;
        }
    }
    n
}

async fn api_stream(ConnectInfo(peer): ConnectInfo<Peer>, req: Request) -> Response {
    let headers = req.headers().clone();
    let d = match super::remote::gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let only_ws = query_of(&req, "workspaceId");
    let only_floor = query_of(&req, "floorId");
    let wake = query_of(&req, "wake").is_some_and(|w| w == "1" || w == "true");
    let mut woke = None;
    if wake && d.scope == Scope::Drive {
        if let Some(ws) = &only_ws {
            let floor = match &only_floor {
                Some(f) => Some(f.clone()),
                None => super::doc::host()
                    .store()
                    .workspace(ws)
                    .ok()
                    .map(|m| m.active_floor_id),
            };
            if let Some(f) = floor {
                woke = Some(wake_floor(ws, &f).await);
            }
        }
    }
    let (tx, rx) = mpsc::channel::<String>(QUEUE);
    let _ = tx.try_send(frame(&json!({
        "type": "hello",
        "device": { "id": d.id, "name": d.name },
        "scope": d.scope,
        "filter": { "workspaceId": only_ws, "floorId": only_floor },
        "woke": woke,
    })));
    let tx_store = tx.clone();
    let (fw, ff) = (only_ws.clone(), only_floor.clone());
    let store_id = super::doc::host().store().subscribe(move |ev| {
        if let StoreEvent::Patch(p) = ev {
            if fw.as_deref().is_some_and(|w| w != p.workspace_id)
                || ff.as_deref().is_some_and(|f| f != p.floor_id)
            {
                return;
            }
            let mutation = p.origin.strip_prefix("remote:").map(str::to_string);
            let _ = tx_store.try_send(frame(&json!({
                "type": "patch",
                "workspaceId": p.workspace_id,
                "floorId": p.floor_id,
                "epoch": p.epoch,
                "seq": p.seq,
                "rev": p.rev,
                "origin": p.origin,
                "mutationId": mutation,
                "changes": changes(&p.ops),
            })));
        }
    });
    let mut app_ids = Vec::new();
    if let Some(app) = super::companion::app_handle() {
        let t = tx.clone();
        app_ids.push(app.listen_any("term://state", move |ev| {
            let Ok(v) = serde_json::from_str::<Value>(ev.payload()) else {
                return;
            };
            let tid = v["id"].as_str().unwrap_or("").to_string();
            let node = super::doc::host()
                .store()
                .locate_terminal(&tid)
                .map(|(_, _, n)| n);
            let _ = t.try_send(frame(&json!({
                "type": "state",
                "terminalId": tid,
                "nodeId": node,
                "state": v["state"],
                "message": v["message"],
            })));
        }));
        let t = tx.clone();
        app_ids.push(app.listen_any(EVENT_PRESENCE, move |ev| {
            if let Ok(v) = serde_json::from_str::<Value>(ev.payload()) {
                let _ = t.try_send(frame(&v));
            }
        }));
    }
    let t = tx.clone();
    tokio::spawn(async move {
        let mut rx = super::cords::host().changes_rx();
        let mut last = usize::MAX;
        loop {
            let n = super::cords::host().inbox(None).len();
            if n != last {
                last = n;
                if t.send(frame(&json!({ "type": "inbox", "count": n })))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            tokio::select! {
                r = rx.changed() => if r.is_err() { break },
                _ = t.closed() => break,
            }
        }
    });
    let subs = Subs { store_id, app_ids };
    drop(tx);
    let device_id = d.id.clone();
    let stream = futures::stream::unfold(
        (rx, subs, device_id),
        |(mut rx, subs, device_id)| async move {
            let deadline = tokio::time::Instant::now() + KEEPALIVE;
            let msg = loop {
                if !lbr::hub().stream_allowed(&device_id) {
                    return None;
                }
                match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
                    Ok(Some(m)) => break m,
                    Ok(None) => return None,
                    Err(_) if tokio::time::Instant::now() >= deadline => {
                        break ": keepalive\n\n".to_string();
                    }
                    Err(_) => {}
                }
            };
            Some((
                Ok::<Bytes, Infallible>(Bytes::from(msg)),
                (rx, subs, device_id),
            ))
        },
    );
    let mut r = Response::new(Body::from_stream(stream));
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    lbr::tag_device(r, &d)
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v[k].as_f64().filter(|x| x.is_finite())
}

async fn api_feed(ConnectInfo(peer): ConnectInfo<Peer>, req: Request) -> Response {
    let headers = req.headers().clone();
    let d = match super::remote::gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let body = match axum::body::to_bytes(req.into_body(), 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return lbr::fail_response(StatusCode::BAD_REQUEST, "bad_body", "body too large"),
    };
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(app) = super::companion::app_handle() else {
        return lbr::fail_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            "app not ready",
        );
    };
    match v["type"].as_str() {
        Some("presence") => {
            let msg = json!({
                "type": "presence",
                "device": { "id": d.id, "name": d.name },
                "nodeId": v["nodeId"].as_str(),
                "x": num(&v, "x"),
                "y": num(&v, "y"),
                "at": chrono::Utc::now().timestamp_millis(),
            });
            let _ = app.emit(EVENT_PRESENCE, msg);
            lbr::tag_device(Json(json!({ "ok": true })).into_response(), &d)
        }
        Some("camera") => {
            if d.scope != Scope::Drive {
                return lbr::tag_device(
                    lbr::fail_response(
                        StatusCode::FORBIDDEN,
                        "insufficient_scope",
                        "read-only device",
                    ),
                    &d,
                );
            }
            let zoom = num(&v, "zoom").map(|z| z.clamp(0.25, 4.0));
            let msg = json!({
                "dx": num(&v, "dx").unwrap_or(0.0),
                "dy": num(&v, "dy").unwrap_or(0.0),
                "zoom": zoom,
                "device": d.name,
            });
            let _ = app.emit(EVENT_REMOTE_CAMERA, msg);
            lbr::tag_device(Json(json!({ "ok": true })).into_response(), &d)
        }
        Some("resize") => {
            if d.scope != Scope::Drive {
                return lbr::tag_device(
                    lbr::fail_response(
                        StatusCode::FORBIDDEN,
                        "insufficient_scope",
                        "read-only device: input dropped",
                    ),
                    &d,
                );
            }
            let cols = num(&v, "cols").unwrap_or(0.0).clamp(0.0, 1000.0) as u16;
            let rows = num(&v, "rows").unwrap_or(0.0).clamp(0.0, 1000.0) as u16;
            let r = terminal_id(v["nodeId"].as_str().unwrap_or("")).and_then(|tid| {
                if cols < 10 || rows < 3 {
                    return Err("CANVAS_INVALID: cols >= 10 and rows >= 3".to_string());
                }
                crate::pty::term().resize(&tid, cols, rows)?;
                Ok(json!({ "ok": true, "cols": cols, "rows": rows }))
            });
            lbr::tag_device(result(r), &d)
        }
        Some("input") => {
            if d.scope != Scope::Drive {
                return lbr::tag_device(
                    lbr::fail_response(
                        StatusCode::FORBIDDEN,
                        "insufficient_scope",
                        "read-only device: input dropped",
                    ),
                    &d,
                );
            }
            let text: String = v["text"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(8_000)
                .collect();
            let r = terminal_id(v["nodeId"].as_str().unwrap_or("")).and_then(|tid| {
                crate::pty::term().input(&tid, text.as_bytes())?;
                Ok(json!({ "ok": true, "bytes": text.len() }))
            });
            lbr::tag_device(result(r), &d)
        }
        _ => lbr::tag_device(
            lbr::fail_response(
                StatusCode::BAD_REQUEST,
                "bad_type",
                "type must be presence, camera, resize or input",
            ),
            &d,
        ),
    }
}

fn result(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let code = e.split(':').next().unwrap_or("ERR").trim().to_string();
            let status = if code.ends_with("NOT_FOUND") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            lbr::fail_response(status, &code, e)
        }
    }
}

fn terminal_id(node_id: &str) -> Result<String, String> {
    let s = super::doc::host().store();
    let (ws, floor) = s
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    let tid = s
        .read(&ws, &floor, |d| {
            d.node(node_id)
                .and_then(|n| n.data_str("terminalId").map(String::from))
        })?
        .filter(|t| !t.is_empty())
        .ok_or("CANVAS_INVALID: not a terminal node")?;
    if crate::pty::term().info(&tid).is_none() {
        return Err("CANVAS_NOT_RUNNING: the terminal is not running".into());
    }
    Ok(tid)
}

async fn api_terminal_stream(
    ConnectInfo(peer): ConnectInfo<Peer>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    let d = match super::remote::gate(&headers, &peer, false) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let tid = match terminal_id(&id) {
        Ok(t) => t,
        Err(e) => {
            let status = if e.starts_with("CANVAS_NOT_RUNNING") {
                StatusCode::CONFLICT
            } else if e.starts_with("CANVAS_NOT_FOUND") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            return lbr::tag_device(lbr::fail_response(status, "terminal", e), &d);
        }
    };
    let (tx, rx) = mpsc::channel::<String>(64);
    let node = id.clone();
    let device_id = d.id.clone();
    tokio::spawn(async move {
        let term = crate::pty::term();
        let mut last_geo = (0u16, 0u16);
        let mut last_changed = i64::MIN;
        let mut idle_keepalive = tokio::time::Instant::now();
        loop {
            if !lbr::hub().stream_allowed(&device_id) {
                break;
            }
            let Some(info) = term.info(&tid) else {
                let _ = tx
                    .send(frame(&json!({ "type": "exit", "nodeId": node })))
                    .await;
                break;
            };
            let geo = (info.cols, info.rows);
            if geo != last_geo {
                last_geo = geo;
                if tx
                    .send(frame(&json!({ "type": "geometry", "nodeId": node, "cols": geo.0, "rows": geo.1 })))
                    .await
                    .is_err()
                {
                    break;
                }
                last_changed = i64::MIN;
            }
            if let Ok(snap) = term.screen_text(&tid, geo.1.max(1) as usize) {
                if snap.changed_ms != last_changed {
                    last_changed = snap.changed_ms;
                    idle_keepalive = tokio::time::Instant::now();
                    if tx
                        .send(frame(&json!({
                            "type": "screen", "nodeId": node, "text": snap.text,
                            "altScreen": snap.alt_screen, "changedMs": snap.changed_ms,
                            "alive": info.alive,
                        })))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            if idle_keepalive.elapsed() > KEEPALIVE {
                idle_keepalive = tokio::time::Instant::now();
                if tx.send(": keepalive\n\n".to_string()).await.is_err() {
                    break;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(150)) => {}
                _ = tx.closed() => break,
            }
        }
    });
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        let m = rx.recv().await?;
        Some((Ok::<Bytes, Infallible>(Bytes::from(m)), rx))
    });
    let mut r = Response::new(Body::from_stream(stream));
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    lbr::tag_device(r, &d)
}
