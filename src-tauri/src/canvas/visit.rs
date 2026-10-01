//! Read-only canvas visits (study 98, 03 §A.6) through the existing
//! `omniworld-server` relay, with no change on the server:
//!
//! - the host opens a room with `"house": "canvas-v1"` (the hello is signed
//!   with the local profile's ed25519 key, as the old house visits were);
//! - frame `40` carries the scene JSON: nodes (position, size, kind, name,
//!   agent, role, canonical state), cords and recent reactions. Never a
//!   terminal's content, unless the host shares that node, and then only its
//!   rendered screen text, never input;
//! - frame `43` is the room chat, as before;
//! - frame `41` from a visitor is only "wave", "point" or "react": it shows
//!   on the host's canvas and touches no agent;
//! - frame `42` (interest) asks the host for a fresh scene.
//!
//! An old app joining a `canvas-v1` room fails its own codec check; this one
//! checks `joined.house` before decoding anything.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use omniget_core::core::canvas::model::{CanvasNode, NodeKind, TerminalData};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

pub const OP_SCENE: u8 = 40;
pub const OP_INPUT: u8 = 41;
pub const OP_INTEREST: u8 = 42;
pub const OP_CHAT: u8 = 43;

pub const HOUSE: &str = "canvas-v1";
pub const SCENE_VERSION: u64 = 1;
pub const DEFAULT_SERVER: &str = "wss://chat.tonho.wtf/v1/room";

pub const EVENT_STATE: &str = "canvas://visit-state";
pub const EVENT_SCENE: &str = "canvas://visit-scene";
pub const EVENT_CHAT: &str = "canvas://visit-chat";
pub const EVENT_CLOSED: &str = "canvas://visit-closed";
pub const EVENT_REACTION: &str = "canvas://visit-reaction";

pub const ERR: &str = "ERR_VISIT";
/// The relay drops frames over 512 KiB; stay well below.
const SCENE_MAX_BYTES: usize = 400 * 1024;
const SHARED_SCREEN_LINES: usize = 40;
const REACTION_TTL_MS: i64 = 6_000;

struct Session {
    role: &'static str,
    code: String,
    server: String,
    out: mpsc::Sender<Message>,
    cancel: CancellationToken,
    visitors: Vec<(u32, String)>,
    host_name: Option<String>,
    workspace_id: Option<String>,
    floor_id: Option<String>,
    shared: HashSet<String>,
    reactions: VecDeque<Value>,
    force: Arc<AtomicBool>,
    /// Visitor: the last scene received (what `remote_status`/MCP shows, so a
    /// visit is testable without the canvas UI).
    last_scene: Option<Value>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn session() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn state_json() -> Value {
    match session().as_ref() {
        Some(s) => json!({
            "role": s.role,
            "code": s.code,
            "server": s.server,
            "hostName": s.host_name,
            "workspaceId": s.workspace_id,
            "floorId": s.floor_id,
            "shared": s.shared.iter().collect::<Vec<_>>(),
            "visitors": s.visitors.iter().map(|(id, name)| json!({ "visitorId": id, "name": name })).collect::<Vec<_>>(),
            "scene": s.last_scene,
        }),
        None => json!({ "role": "none", "visitors": [], "shared": [] }),
    }
}

fn emit_state(app: &AppHandle) {
    let _ = app.emit(EVENT_STATE, state_json());
}

/// `host`, `host:port`, `http(s)://…` or `ws(s)://…` → a `ws(s)://…/v1/room`.
pub fn normalize_server(raw: Option<String>) -> String {
    let raw = raw
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());
    let with_scheme = if let Some(rest) = raw.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = raw.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if raw.starts_with("ws://") || raw.starts_with("wss://") {
        raw
    } else {
        format!("wss://{raw}")
    };
    let after = with_scheme.split_once("://").map(|(_, r)| r).unwrap_or("");
    if after.trim_end_matches('/').contains('/') {
        with_scheme
    } else {
        format!("{}/v1/room", with_scheme.trim_end_matches('/'))
    }
}

fn frame(op: u8, payload: &[u8]) -> Message {
    let mut bytes = Vec::with_capacity(payload.len() + 1);
    bytes.push(op);
    bytes.extend_from_slice(payload);
    Message::Binary(bytes.into())
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn handshake(server: &str, hello: Value) -> Result<(Ws, Value), String> {
    let (mut ws, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(server),
    )
    .await
    .map_err(|_| format!("{ERR}_UNREACHABLE: {server} did not answer"))?
    .map_err(|e| format!("{ERR}_UNREACHABLE: {e}"))?;
    ws.send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| format!("{ERR}_UNREACHABLE: {e}"))?;
    let reply = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(msg) = ws.next().await {
            if let Ok(Message::Text(t)) = msg {
                return serde_json::from_str::<Value>(&t).ok();
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| format!("{ERR}_PROTOCOL: no answer to the hello"))?;
    if reply["op"] == "error" {
        return Err(format!(
            "{}: {}",
            reply["code"].as_str().unwrap_or("ERR_ROOM"),
            reply["message"].as_str().unwrap_or("refused")
        ));
    }
    Ok((ws, reply))
}

fn signed_hello(app: &AppHandle, op: &str, what: &str, extra: Value) -> Result<Value, String> {
    let profiles = app.state::<crate::AppState>().profile.clone();
    let profile = profiles.get()?;
    let ts = now_ms() as u64;
    let sig = profiles.sign(format!("omniworld:{what}:{ts}").as_bytes())?;
    let mut hello = json!({
        "op": op,
        "pubkey": hex::encode(profile.public_key),
        "name": profile.nickname,
        "ts": ts,
        "sig": hex::encode(sig),
    });
    if let (Some(h), Some(e)) = (hello.as_object_mut(), extra.as_object()) {
        h.extend(e.clone());
    }
    Ok(hello)
}

// ---------------------------------------------------------------- scene

fn node_state(node: &CanvasNode) -> Option<String> {
    let tid = node.data_as::<TerminalData>()?.terminal_id;
    let sig = crate::pty::term().agent_state(&tid)?;
    serde_json::to_value(sig.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

/// The scene a visitor sees. Nothing private: no paths, commands, env, URLs
/// or note bodies; a terminal's screen only when `shared` names it.
pub fn build_scene(
    ws: &str,
    floor: &str,
    shared: &HashSet<String>,
    reactions: &[Value],
    host: &str,
) -> Result<Value, String> {
    let store = super::doc::host().store().clone();
    let workspace = store.workspace(ws)?;
    let floor_name = workspace
        .floors
        .iter()
        .find(|f| f.id == floor)
        .map(|f| f.name.clone())
        .unwrap_or_default();
    let (nodes, cords) = store.read(ws, floor, |d| (d.nodes.clone(), d.connections.clone()))?;
    let term = crate::pty::term();
    let mut with_screens = true;
    loop {
        let out_nodes: Vec<Value> = nodes
            .iter()
            .filter(|n| n.is_on_canvas())
            .map(|n| {
                let terminal = n.is_terminal();
                let is_shared = shared.contains(&n.id);
                let mut v = json!({
                    "id": n.id,
                    "kind": n.kind.as_str(),
                    "x": n.x, "y": n.y, "w": n.w, "h": n.h, "z": n.z,
                    "name": super::cords::node_name(n),
                    "color": n.color,
                    "shared": is_shared,
                });
                if terminal {
                    v["agent"] = json!(n.data_str("agent").unwrap_or("shell"));
                    v["roleId"] = json!(n.data_str("roleId"));
                    v["state"] = json!(node_state(n));
                    if is_shared && with_screens {
                        if let Some(tid) = n.data_as::<TerminalData>().map(|d| d.terminal_id) {
                            if let Ok(s) = term.screen_text(&tid, SHARED_SCREEN_LINES) {
                                v["screen"] = json!(s.text);
                            }
                        }
                    }
                }
                if n.kind == NodeKind::Stack {
                    v["name"] = json!(n.data_str("name").unwrap_or("Notes"));
                }
                v
            })
            .collect();
        let out_cords: Vec<Value> = cords
            .iter()
            .map(|c| json!({ "id": c.id, "from": c.from, "to": c.to, "kind": c.kind.as_str(), "label": c.label }))
            .collect();
        let scene = json!({
            "v": SCENE_VERSION,
            "house": HOUSE,
            "host": host,
            "workspace": { "name": workspace.name, "icon": workspace.icon },
            "floor": { "id": floor, "name": floor_name },
            "nodes": out_nodes,
            "cords": out_cords,
            "reactions": reactions,
        });
        let size = scene.to_string().len();
        if size <= SCENE_MAX_BYTES || !with_screens {
            if size > SCENE_MAX_BYTES {
                return Err(format!("{ERR}_TOO_BIG: this floor is too big to share"));
            }
            return Ok(scene);
        }
        with_screens = false;
    }
}

/// Validates a scene from the network before the UI sees it.
pub fn accept_scene(bytes: &[u8]) -> Option<Value> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    (v["v"].as_u64() == Some(SCENE_VERSION) && v["house"] == HOUSE && v["nodes"].is_array())
        .then_some(v)
}

/// Host loop: rebuilds the scene every second and sends it when it changed
/// (or when a visitor asked for a fresh one).
fn spawn_pump(app: AppHandle, cancel: CancellationToken, force: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<String> = None;
        let mut tick = tokio::time::interval(Duration::from_millis(1_000));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tick.tick() => {}
            }
            let snapshot = {
                let mut g = session();
                let Some(s) = g.as_mut() else { break };
                let now = now_ms();
                while s
                    .reactions
                    .front()
                    .is_some_and(|r| now - r["at"].as_i64().unwrap_or(0) > REACTION_TTL_MS)
                {
                    s.reactions.pop_front();
                }
                (
                    s.workspace_id.clone().unwrap_or_default(),
                    s.floor_id.clone().unwrap_or_default(),
                    s.shared.clone(),
                    s.reactions.iter().cloned().collect::<Vec<_>>(),
                    s.out.clone(),
                    s.visitors.is_empty(),
                )
            };
            let (ws, floor, shared, reactions, out, empty) = snapshot;
            let forced = force.swap(false, Ordering::SeqCst);
            if empty && !forced {
                continue;
            }
            let host = app
                .state::<crate::AppState>()
                .profile
                .get()
                .map(|p| p.nickname)
                .unwrap_or_default();
            let scene = match tauri::async_runtime::spawn_blocking(move || {
                build_scene(&ws, &floor, &shared, &reactions, &host)
            })
            .await
            {
                Ok(Ok(s)) => s.to_string(),
                Ok(Err(e)) => {
                    tracing::warn!("[visit] scene: {e}");
                    continue;
                }
                Err(_) => continue,
            };
            if !forced && last.as_deref() == Some(scene.as_str()) {
                continue;
            }
            match out.try_send(frame(OP_SCENE, scene.as_bytes())) {
                Ok(()) => last = Some(scene),
                // Full: the next tick retries (last stays stale on purpose).
                Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    });
}

// ---------------------------------------------------------------- socket

fn end(app: &AppHandle, reason: &str) {
    if let Some(s) = session().take() {
        s.cancel.cancel();
        let _ = app.emit(EVENT_CLOSED, json!({ "reason": reason, "role": s.role }));
    }
    emit_state(app);
}

fn run(app: AppHandle, mut ws: Ws, mut out_rx: mpsc::Receiver<Message>, cancel: CancellationToken) {
    tauri::async_runtime::spawn(async move {
        let reason = loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = ws.send(Message::Close(None)).await;
                    break "left";
                }
                Some(msg) = out_rx.recv() => {
                    if ws.send(msg).await.is_err() {
                        break "connection_lost";
                    }
                }
                incoming = ws.next() => match incoming {
                    Some(Ok(Message::Binary(b))) if !b.is_empty() => on_binary(&app, b[0], &b[1..]),
                    Some(Ok(Message::Text(t))) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&t) {
                            if v["op"] == "closed" {
                                break if v["reason"] == "ttl" { "ttl" } else { "host_left" };
                            }
                            on_control(&app, &v);
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = ws.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break "connection_lost",
                    _ => {}
                },
            }
        };
        end(&app, reason);
    });
}

fn on_control(app: &AppHandle, v: &Value) {
    let id = v["visitor_id"].as_u64().unwrap_or(0) as u32;
    match v["op"].as_str().unwrap_or("") {
        "visitor_joined" => {
            let name = v["name"].as_str().unwrap_or("guest").to_string();
            if let Some(s) = session().as_mut() {
                s.visitors.retain(|(i, _)| *i != id);
                s.visitors.push((id, name));
                // The newcomer has no picture yet.
                s.force.store(true, Ordering::SeqCst);
            }
        }
        "visitor_left" => {
            if let Some(s) = session().as_mut() {
                s.visitors.retain(|(i, _)| *i != id);
            }
        }
        _ => return,
    }
    emit_state(app);
}

fn on_binary(app: &AppHandle, op: u8, payload: &[u8]) {
    let role = session().as_ref().map(|s| s.role).unwrap_or("none");
    match (op, role) {
        (OP_SCENE, "visitor") => {
            if let Some(scene) = accept_scene(payload) {
                if let Some(s) = session().as_mut() {
                    s.last_scene = serde_json::to_value(&scene).ok();
                }
                let _ = app.emit(EVENT_SCENE, scene);
            }
        }
        (OP_CHAT, _) => {
            if let Ok(v) = serde_json::from_slice::<Value>(payload) {
                let _ = app.emit(EVENT_CHAT, v);
            }
        }
        (OP_INTEREST, "host") => {
            if let Some(s) = session().as_ref() {
                s.force.store(true, Ordering::SeqCst);
            }
        }
        // A visitor waves, points or reacts. Nothing else is accepted, and
        // none of it reaches an agent.
        (OP_INPUT, "host") if payload.len() > 4 => {
            let id = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let Ok(v) = serde_json::from_slice::<Value>(&payload[4..]) else {
                return;
            };
            if let Some(r) = reaction_of(id, &v) {
                let mut g = session();
                let Some(s) = g.as_mut() else { return };
                let Some(name) = s
                    .visitors
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map(|(_, n)| n.clone())
                else {
                    return;
                };
                let mut r = r;
                r["name"] = json!(name);
                s.reactions.push_back(r.clone());
                while s.reactions.len() > 16 {
                    s.reactions.pop_front();
                }
                s.force.store(true, Ordering::SeqCst);
                drop(g);
                let _ = app.emit(EVENT_REACTION, r);
            }
        }
        _ => {}
    }
}

/// `{type: wave|point|react, x?, y?, emoji?}` → a sanitized reaction.
pub fn reaction_of(visitor_id: u32, v: &Value) -> Option<Value> {
    let kind = v["type"].as_str()?;
    if !matches!(kind, "wave" | "point" | "react") {
        return None;
    }
    let num = |k: &str| v[k].as_f64().filter(|x| x.is_finite() && x.abs() < 1e7);
    let emoji: Option<String> = v["emoji"]
        .as_str()
        .map(|e| e.chars().filter(|c| !c.is_control()).take(4).collect())
        .filter(|e: &String| !e.is_empty());
    Some(json!({
        "visitorId": visitor_id, "type": kind, "x": num("x"), "y": num("y"),
        "emoji": emoji, "at": now_ms(),
    }))
}

fn send(msg: Message) -> Result<(), String> {
    let out = session()
        .as_ref()
        .map(|s| s.out.clone())
        .ok_or_else(|| format!("{ERR}_CLOSED: not in a visit"))?;
    out.try_send(msg)
        .map_err(|_| format!("{ERR}_BUSY: the connection is saturated"))
}

// ---------------------------------------------------------------- API

/// Opens `ws`/`floor` for read-only visits; returns the state with the code.
pub async fn open(
    app: &AppHandle,
    workspace_id: String,
    floor_id: String,
    server: Option<String>,
) -> Result<Value, String> {
    if session().is_some() {
        return Err(format!("{ERR}_BUSY: close the current visit first"));
    }
    // Fails early on an unknown floor.
    super::doc::host()
        .store()
        .read(&workspace_id, &floor_id, |_| ())?;
    let server = normalize_server(server);
    let hello = signed_hello(app, "open", "open", json!({ "house": HOUSE }))?;
    let (ws, reply) = handshake(&server, hello).await?;
    let code = reply["code"]
        .as_str()
        .ok_or_else(|| format!("{ERR}_PROTOCOL: no code"))?
        .to_string();
    let (out, out_rx) = mpsc::channel::<Message>(64);
    let cancel = CancellationToken::new();
    let force = Arc::new(AtomicBool::new(true));
    *session() = Some(Session {
        role: "host",
        code,
        server,
        out,
        cancel: cancel.clone(),
        visitors: Vec::new(),
        host_name: None,
        workspace_id: Some(workspace_id),
        floor_id: Some(floor_id),
        shared: HashSet::new(),
        reactions: VecDeque::new(),
        force: force.clone(),
        last_scene: None,
    });
    run(app.clone(), ws, out_rx, cancel.clone());
    spawn_pump(app.clone(), cancel, force);
    emit_state(app);
    Ok(state_json())
}

pub fn close(app: &AppHandle) -> Value {
    end(app, "left");
    state_json()
}

/// Host: shows (or hides) one terminal's screen text to the visitors.
pub fn share(app: &AppHandle, node_id: &str, shared: bool) -> Result<Value, String> {
    {
        let mut g = session();
        let s = g
            .as_mut()
            .filter(|s| s.role == "host")
            .ok_or_else(|| format!("{ERR}_CLOSED: the canvas is not open for visits"))?;
        if shared {
            let store = super::doc::host().store().clone();
            let ws = s.workspace_id.clone().unwrap_or_default();
            let floor = s.floor_id.clone().unwrap_or_default();
            let ok = store
                .read(&ws, &floor, |d| {
                    d.node(node_id).is_some_and(|n| n.is_terminal())
                })
                .unwrap_or(false);
            if !ok {
                return Err(format!(
                    "{ERR}_INVALID: only a terminal of the shared floor can be shared"
                ));
            }
            s.shared.insert(node_id.to_string());
        } else {
            s.shared.remove(node_id);
        }
        s.force.store(true, Ordering::SeqCst);
    }
    emit_state(app);
    Ok(state_json())
}

/// Visits a canvas. Answers with the state and the first scene; later
/// scenes arrive as `canvas://visit-scene`.
pub async fn join(app: &AppHandle, code: String, server: Option<String>) -> Result<Value, String> {
    if session().is_some() {
        return Err(format!("{ERR}_BUSY: close the current visit first"));
    }
    let code = code.trim().to_uppercase();
    let server = normalize_server(server);
    let hello = signed_hello(
        app,
        "join",
        &format!("join:{code}"),
        json!({ "code": code }),
    )?;
    let (mut ws, reply) = handshake(&server, hello).await?;
    if reply["house"] != HOUSE {
        let _ = ws.send(Message::Close(None)).await;
        return Err(format!(
            "{ERR}_NOT_CANVAS: this code opens a house, not a canvas"
        ));
    }
    let first = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(Message::Binary(b)) if b.first() == Some(&OP_SCENE) => {
                    if let Some(scene) = accept_scene(&b[1..]) {
                        return Some(scene);
                    }
                }
                Ok(Message::Ping(p)) => {
                    let _ = ws.send(Message::Pong(p)).await;
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                _ => {}
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| format!("{ERR}_NO_SCENE: the host sent no canvas"))?;
    let (out, out_rx) = mpsc::channel::<Message>(32);
    let cancel = CancellationToken::new();
    *session() = Some(Session {
        role: "visitor",
        code,
        server,
        out,
        cancel: cancel.clone(),
        visitors: reply["visitors"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                (
                    v["visitor_id"].as_u64().unwrap_or(0) as u32,
                    v["name"].as_str().unwrap_or("guest").to_string(),
                )
            })
            .collect(),
        host_name: reply["host_name"].as_str().map(str::to_string),
        workspace_id: None,
        floor_id: None,
        shared: HashSet::new(),
        reactions: VecDeque::new(),
        force: Arc::new(AtomicBool::new(false)),
        last_scene: Some(first.clone()),
    });
    run(app.clone(), ws, out_rx, cancel);
    emit_state(app);
    Ok(json!({ "state": state_json(), "scene": first }))
}

pub fn leave(app: &AppHandle) -> Value {
    end(app, "left");
    state_json()
}

pub fn chat(text: &str) -> Result<(), String> {
    let text: String = text
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect();
    if text.is_empty() {
        return Ok(());
    }
    send(frame(OP_CHAT, text.as_bytes()))
}

/// Visitor: wave, point at `(x, y)` (world units) or react with an emoji.
pub fn react(
    kind: &str,
    x: Option<f64>,
    y: Option<f64>,
    emoji: Option<String>,
) -> Result<(), String> {
    let v = json!({ "type": kind, "x": x, "y": y, "emoji": emoji });
    reaction_of(0, &v).ok_or_else(|| format!("{ERR}_INVALID: unknown reaction"))?;
    if session().as_ref().map(|s| s.role) != Some("visitor") {
        return Err(format!("{ERR}_CLOSED: not visiting"));
    }
    send(frame(OP_INPUT, v.to_string().as_bytes()))
}

/// Visitor: ask the host for a fresh scene.
pub fn interest() -> Result<(), String> {
    send(frame(OP_INTEREST, b"{\"type\":\"interest\"}"))
}
