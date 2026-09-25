//! The city: a persistent, server-authoritative world on an OmniDisc
//! instance. Where `house.rs` relays a house the host simulates, this joins a
//! region the server simulates. Nothing here opens a socket until the user
//! enters the city, and leaving drops the only socket there is.
//!
//! Wire: `omnidisc-server/src/world/protocol.rs`. Binary frames from the
//! server go to the route's channel untouched (`[opcode][tag][blob]`), so the
//! TypeScript decoder sees exactly what the server sent; control text frames
//! become `city://control` events; chat becomes `city://chat`; acks become
//! `city://ack`.
//!
//! The account token comes from the OmniDisc session store for the instance
//! URL, so the server's `actor_id` is the signed-in user, never a field of a
//! payload. No token means "sign in to OmniDisc first".

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

pub const OP_SNAPSHOT: u8 = 50;
pub const OP_DIFF: u8 = 51;
pub const OP_INPUT: u8 = 52;
pub const OP_ACK: u8 = 53;
pub const OP_INTEREST: u8 = 54;
pub const OP_CHAT: u8 = 55;
pub const OP_RESYNC: u8 = 56;
pub const OP_NOTICE: u8 = 57;

pub const EVENT_CONTROL: &str = "city://control";
pub const EVENT_CHAT: &str = "city://chat";
pub const EVENT_ACK: &str = "city://ack";
pub const EVENT_CLOSED: &str = "city://closed";

pub const ERR_CITY: &str = "ERR_CITY";
pub const DEFAULT_SERVER: &str = "https://chat.tonho.wtf";
const RECONNECT_ATTEMPTS: u32 = 5;

struct Session {
    city: String,
    base: String,
    out: mpsc::Sender<Message>,
    cancel: CancellationToken,
    seq: Arc<AtomicU32>,
    ready: Value,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn session() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

fn state_json() -> Value {
    match session().as_ref() {
        Some(s) => json!({
            "connected": true,
            "city": s.city,
            "server": s.base,
            "ready": s.ready,
        }),
        None => json!({ "connected": false }),
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The instance base URL: the argument, then Settings, then the public one.
pub fn resolve_base(raw: Option<String>, app: &AppHandle) -> Result<String, String> {
    let raw = raw
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let saved = crate::storage::config::load_settings(app).world.city_server;
            (!saved.trim().is_empty()).then(|| saved.trim().to_string())
        })
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());
    crate::commands::omnidisc::normalize_instance_url(&raw)
}

fn ws_url(base: &str) -> String {
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}/world/v1")
}

fn token_for(base: &str) -> Result<String, String> {
    crate::commands::omnidisc::store::load_token(base)?
        .ok_or_else(|| format!("{ERR_CITY}_NO_SESSION: sign in to OmniDisc at {base} first"))
}

/// Connect, say hello, wait for `ready` (or an error).
async fn handshake(base: &str, city: &str) -> Result<(Ws, Value), String> {
    let token = token_for(base)?;
    let url = ws_url(base);
    let connect = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_tungstenite::connect_async(&url),
    );
    let (mut ws, _) = connect
        .await
        .map_err(|_| format!("{ERR_CITY}_UNREACHABLE: {url} did not answer"))?
        .map_err(|e| format!("{ERR_CITY}_UNREACHABLE: {e}"))?;
    let hello = json!({ "op": "hello", "token": token, "city": city, "wire": 1 });
    ws.send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| format!("{ERR_CITY}_UNREACHABLE: {e}"))?;
    let reply = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(Message::Text(t)) => return serde_json::from_str::<Value>(&t).ok(),
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
    .ok_or_else(|| format!("{ERR_CITY}_PROTOCOL: no answer to the hello"))?;
    if reply["op"] == "error" {
        return Err(format!(
            "{}: {}",
            reply["code"].as_str().unwrap_or("ERR_CITY"),
            reply["message"].as_str().unwrap_or("refused")
        ));
    }
    if reply["op"] != "ready" {
        return Err(format!(
            "{ERR_CITY}_PROTOCOL: expected ready, got {}",
            reply["op"]
        ));
    }
    Ok((ws, reply))
}

fn on_binary(app: &AppHandle, bytes: &[u8], channel: &Channel<InvokeResponseBody>) {
    let Some(&op) = bytes.first() else { return };
    match op {
        OP_SNAPSHOT | OP_DIFF => {
            let _ = channel.send(InvokeResponseBody::Raw(bytes.to_vec()));
        }
        OP_ACK => {
            if bytes.len() >= 6 {
                let seq = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
                let _ = app.emit(
                    EVENT_ACK,
                    json!({
                        "seq": seq,
                        "status": bytes[5],
                        "code": String::from_utf8_lossy(&bytes[6..]),
                    }),
                );
            }
        }
        OP_CHAT | OP_NOTICE => {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes[1..]) {
                let _ = app.emit(
                    if op == OP_CHAT {
                        EVENT_CHAT
                    } else {
                        EVENT_CONTROL
                    },
                    v,
                );
            }
        }
        _ => {}
    }
}

fn end(app: &AppHandle, reason: &str) {
    if let Some(s) = session().take() {
        s.cancel.cancel();
    }
    let _ = app.emit(EVENT_CLOSED, json!({ "reason": reason }));
}

/// The socket loop, with reconnection: a dropped connection is retried with
/// backoff and a fresh hello; the server answers with a snapshot, which is
/// the client's resync. The route learns about it through `city://control`
/// (`op: "reconnected"`) so it can drop its replica and wait for that one.
fn run(
    app: AppHandle,
    mut ws: Ws,
    mut out_rx: mpsc::Receiver<Message>,
    cancel: CancellationToken,
    channel: Channel<InvokeResponseBody>,
    base: String,
    city: String,
) {
    tauri::async_runtime::spawn(async move {
        let mut attempts = 0u32;
        let reason = 'outer: loop {
            let lost = loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        let _ = ws.send(Message::Close(None)).await;
                        break 'outer "left";
                    }
                    Some(msg) = out_rx.recv() => {
                        if ws.send(msg).await.is_err() {
                            break true;
                        }
                    }
                    incoming = ws.next() => {
                        match incoming {
                            Some(Ok(Message::Binary(bytes))) => on_binary(&app, &bytes, &channel),
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                    if v["op"] == "error" {
                                        let _ = app.emit(EVENT_CONTROL, v);
                                        break 'outer "error";
                                    }
                                    let _ = app.emit(EVENT_CONTROL, v);
                                }
                            }
                            Some(Ok(Message::Ping(p))) => {
                                let _ = ws.send(Message::Pong(p)).await;
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break true,
                            _ => {}
                        }
                    }
                }
            };
            if !lost {
                break "left";
            }
            // Reconnect with backoff.
            loop {
                attempts += 1;
                if attempts > RECONNECT_ATTEMPTS {
                    break 'outer "connection_lost";
                }
                let wait = std::time::Duration::from_secs(1u64 << (attempts - 1).min(4));
                let _ = app.emit(EVENT_CONTROL, json!({ "op": "reconnecting", "attempt": attempts, "wait_ms": wait.as_millis() as u64 }));
                tokio::select! {
                    _ = cancel.cancelled() => break 'outer "left",
                    _ = tokio::time::sleep(wait) => {}
                }
                match handshake(&base, &city).await {
                    Ok((new_ws, ready)) => {
                        ws = new_ws;
                        attempts = 0;
                        if let Some(s) = session().as_mut() {
                            s.ready = ready.clone();
                        }
                        let mut v = ready;
                        v["op"] = json!("reconnected");
                        let _ = app.emit(EVENT_CONTROL, v);
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("[city] reconnect failed: {e}");
                    }
                }
            }
        };
        end(&app, reason);
    });
}

/// Enter the city. Answers with the `ready` message; the first snapshot and
/// every frame after it arrive on `channel`.
#[tauri::command]
pub async fn city_join(
    app: AppHandle,
    city: String,
    server: Option<String>,
    channel: Channel<InvokeResponseBody>,
) -> Result<Value, String> {
    if session().is_some() {
        return Err(format!("{ERR_CITY}_BUSY: leave the current city first"));
    }
    let city = city.trim().to_string();
    if city.is_empty() || city.len() > 64 {
        return Err(format!("{ERR_CITY}_BAD_CITY"));
    }
    let base = resolve_base(server, &app)?;
    let (ws, ready) = handshake(&base, &city).await?;
    let (out, out_rx) = mpsc::channel::<Message>(512);
    let cancel = CancellationToken::new();
    *session() = Some(Session {
        city: city.clone(),
        base: base.clone(),
        out,
        cancel: cancel.clone(),
        seq: Arc::new(AtomicU32::new(1)),
        ready: ready.clone(),
    });
    run(app.clone(), ws, out_rx, cancel, channel, base, city);
    Ok(ready)
}

#[tauri::command]
pub async fn city_leave(app: AppHandle) -> Result<Value, String> {
    end(&app, "left");
    Ok(state_json())
}

fn send_frame(op: u8, payload: &[u8]) -> Result<(), String> {
    let s = session();
    let Some(s) = s.as_ref() else {
        return Err(format!("{ERR_CITY}_NOT_CONNECTED"));
    };
    let mut bytes = Vec::with_capacity(payload.len() + 1);
    bytes.push(op);
    bytes.extend_from_slice(payload);
    s.out
        .try_send(Message::Binary(bytes.into()))
        .map_err(|_| format!("{ERR_CITY}_BUSY: outgoing queue is full"))
}

/// A `ClientInput` of the server protocol, as JSON. Returns the sequence
/// number, which the ack will carry.
#[tauri::command]
pub async fn city_input(input: Value) -> Result<u32, String> {
    let seq = {
        let s = session();
        let Some(s) = s.as_ref() else {
            return Err(format!("{ERR_CITY}_NOT_CONNECTED"));
        };
        s.seq.fetch_add(1, Ordering::Relaxed)
    };
    let mut payload = seq.to_be_bytes().to_vec();
    payload.extend_from_slice(input.to_string().as_bytes());
    send_frame(OP_INPUT, &payload)?;
    Ok(seq)
}

#[tauri::command]
pub async fn city_interest(cx: i32, cy: i32, radius: u16) -> Result<(), String> {
    let mut payload = Vec::with_capacity(10);
    payload.extend_from_slice(&cx.to_be_bytes());
    payload.extend_from_slice(&cy.to_be_bytes());
    payload.extend_from_slice(&radius.to_be_bytes());
    send_frame(OP_INTEREST, &payload)
}

#[tauri::command]
pub async fn city_chat(text: String) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    send_frame(OP_CHAT, text.as_bytes())
}

#[tauri::command]
pub async fn city_resync() -> Result<(), String> {
    send_frame(OP_RESYNC, &[])
}

/// An authenticated call to `/api/world/*` on the city's instance (or the
/// one in `server` before joining): claim a plot, patch a home, list plots.
#[tauri::command]
pub async fn city_api(
    app: AppHandle,
    method: String,
    path: String,
    body: Option<Value>,
    server: Option<String>,
) -> Result<Value, String> {
    let base = match session().as_ref() {
        Some(s) => s.base.clone(),
        None => resolve_base(server, &app)?,
    };
    if !path.starts_with("/api/world/") || path.contains("..") {
        return Err(format!("{ERR_CITY}_BAD_PATH"));
    }
    let token = token_for(&base)?;
    let method = reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
        .map_err(|_| format!("{ERR_CITY}_BAD_METHOD"))?;
    let client = crate::commands::omnidisc::http::http_client(std::time::Duration::from_secs(20))?;
    let mut req = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("{ERR_CITY}_UNREACHABLE: {e}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    let value: Value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text.clone()))
    };
    if status.is_success() {
        return Ok(value);
    }
    let code = value["code"].as_str().unwrap_or("").to_string();
    Err(crate::commands::omnidisc::http::map_error(status, &code))
}

/// Is there an OmniDisc session for the city's instance. The chat shell that
/// used to sign in is retired, so the city has its own sign-in below.
#[tauri::command]
pub async fn city_session(app: AppHandle, server: Option<String>) -> Result<Value, String> {
    let base = resolve_base(server, &app)?;
    let has = crate::commands::omnidisc::store::load_token(&base)?.is_some();
    Ok(json!({ "server": base, "signed_in": has }))
}

async fn auth_call(
    base: &str,
    path: &str,
    username: &str,
    password: &str,
) -> Result<Value, String> {
    let client = crate::commands::omnidisc::http::http_client(std::time::Duration::from_secs(20))?;
    let res = client
        .post(format!("{base}{path}"))
        .json(&json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|e| format!("{ERR_CITY}_UNREACHABLE: {e}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let code = value["code"].as_str().unwrap_or("").to_string();
        return Err(crate::commands::omnidisc::http::map_error(status, &code));
    }
    let token = value["token"]
        .as_str()
        .ok_or_else(|| format!("{ERR_CITY}_PROTOCOL: no token in the answer"))?;
    crate::commands::omnidisc::store::save_token(base, token)?;
    Ok(json!({ "server": base, "signed_in": true, "user": value["user"] }))
}

/// Sign in to the instance and keep the session token in the OS store.
#[tauri::command]
pub async fn city_login(
    app: AppHandle,
    username: String,
    password: String,
    server: Option<String>,
) -> Result<Value, String> {
    let base = resolve_base(server, &app)?;
    auth_call(&base, "/api/auth/login", username.trim(), &password).await
}

/// Create an account on the instance (when registration is open) and sign in.
#[tauri::command]
pub async fn city_register(
    app: AppHandle,
    username: String,
    password: String,
    server: Option<String>,
) -> Result<Value, String> {
    let base = resolve_base(server, &app)?;
    auth_call(&base, "/api/auth/register", username.trim(), &password).await
}

#[tauri::command]
pub async fn city_logout(app: AppHandle, server: Option<String>) -> Result<Value, String> {
    let base = resolve_base(server, &app)?;
    if session().as_ref().map(|s| s.base == base).unwrap_or(false) {
        end(&app, "left");
    }
    crate::commands::omnidisc::store::delete_token(&base)?;
    Ok(json!({ "server": base, "signed_in": false }))
}
