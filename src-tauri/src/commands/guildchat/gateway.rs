use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_util::sync::CancellationToken;

use super::model::{self, Message, Snapshot};
use super::zlib::ZlibStream;
use super::{props, ws};

pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub enum Command {
    Subscribe {
        guild_id: String,
        channel_id: String,
    },
}

pub struct Hooks {
    pub emit: Emit,
    pub snapshot: Arc<RwLock<Snapshot>>,
    pub on_token: Arc<dyn Fn(String) + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Stopped,
    Fatal(String),
    Reconnect,
    NewSession,
    Retry,
}

pub fn close_action(code: u16) -> End {
    match code {
        4004 => End::Fatal("unauthorized".into()),
        4010..=4014 => End::Fatal(format!("refused_{code}")),
        4007 | 4009 => End::NewSession,
        4008 => End::Retry,
        _ => End::Reconnect,
    }
}

pub const BACKOFF_START: Duration = Duration::from_secs(2);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);

pub fn next_backoff(current: Duration, worked: bool) -> Duration {
    if worked {
        BACKOFF_START
    } else {
        (current * 2).min(BACKOFF_MAX)
    }
}

pub fn resume_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/?v=9&encoding=json&compress=zlib-stream")
}

#[derive(Default)]
struct Session {
    id: Option<String>,
    url: Option<String>,
    seq: Option<u64>,
    me: String,
    subscribed: HashSet<String>,
    ready: bool,
}

pub async fn run(
    token: String,
    hooks: Hooks,
    mut cmds: mpsc::UnboundedReceiver<Command>,
    cancel: CancellationToken,
) {
    let mut session = Session::default();
    let mut backoff = BACKOFF_START;
    let mut first = true;
    loop {
        let state = if first || session.id.is_none() {
            "connecting"
        } else {
            "reconnecting"
        };
        first = false;
        (hooks.emit)("guildchat-status", json!({ "state": state }));
        let (end, worked) = connect_once(&token, &hooks, &mut session, &mut cmds, &cancel).await;
        session.ready = false;
        session.subscribed.clear();
        let wait = match end {
            End::Stopped => {
                (hooks.emit)("guildchat-status", json!({ "state": "offline" }));
                return;
            }
            End::Fatal(reason) => {
                tracing::warn!("[guildchat] gateway stopped: {}", reason);
                (hooks.emit)(
                    "guildchat-status",
                    json!({ "state": "failed", "reason": reason }),
                );
                return;
            }
            End::Retry => backoff,
            End::Reconnect => {
                if worked {
                    Duration::from_secs(1)
                } else {
                    backoff
                }
            }
            End::NewSession => {
                session.id = None;
                session.url = None;
                session.seq = None;
                Duration::from_millis(1000 + rand::random_range(0..4000u64))
                    + if worked { Duration::ZERO } else { backoff }
            }
        };
        backoff = next_backoff(backoff, worked);
        (hooks.emit)(
            "guildchat-status",
            json!({ "state": "reconnecting", "retry_in_ms": wait.as_millis() as u64 }),
        );
        tokio::select! {
            _ = cancel.cancelled() => {
                (hooks.emit)("guildchat-status", json!({ "state": "offline" }));
                return;
            }
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

async fn send(sock: &mut ws::Ws, v: &Value) -> bool {
    sock.send(WsMessage::Text(v.to_string().into()))
        .await
        .is_ok()
}

async fn close(sock: &mut ws::Ws, code: u16) {
    let frame = CloseFrame {
        code: CloseCode::from(code),
        reason: "".into(),
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), sock.close(Some(frame))).await;
}

fn identify(token: &str) -> Value {
    json!({
        "op": 2,
        "d": {
            "token": token,
            "capabilities": props::CAPABILITIES,
            "properties": props::properties(),
            "presence": { "status": "unknown", "since": 0, "activities": [], "afk": false },
            "compress": false,
            "client_state": props::client_state(),
        }
    })
}

fn subscribe_payload(guild_id: &str, channel_id: &str) -> Value {
    let mut subs = serde_json::Map::new();
    subs.insert(
        guild_id.to_string(),
        json!({
            "typing": false,
            "threads": true,
            "activities": false,
            "channels": { channel_id: [[0, 99]] },
        }),
    );
    json!({ "op": 37, "d": { "subscriptions": subs } })
}

async fn connect_once(
    token: &str,
    hooks: &Hooks,
    s: &mut Session,
    cmds: &mut mpsc::UnboundedReceiver<Command>,
    cancel: &CancellationToken,
) -> (End, bool) {
    let url = match (&s.id, &s.url) {
        (Some(_), Some(u)) => resume_url(u),
        _ => props::GATEWAY_URL.to_string(),
    };
    let mut sock = tokio::select! {
        _ = cancel.cancelled() => return (End::Stopped, false),
        r = ws::connect(&url) => match r {
            Ok(sock) => sock,
            Err(e) => {
                tracing::warn!("[guildchat] gateway connect failed: {}", e);
                return (End::Retry, false);
            }
        },
    };
    let mut z = ZlibStream::new();
    let mut interval = Duration::ZERO;
    let mut next_beat = Instant::now() + Duration::from_secs(20);
    let mut acked = true;
    let mut worked = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                close(&mut sock, 1000).await;
                return (End::Stopped, worked);
            }
            _ = tokio::time::sleep_until(next_beat) => {
                if interval.is_zero() || !acked {
                    tracing::warn!("[guildchat] gateway went quiet, reconnecting");
                    close(&mut sock, 4000).await;
                    return (End::Reconnect, worked);
                }
                if !send(&mut sock, &json!({"op": 1, "d": s.seq})).await {
                    return (End::Reconnect, worked);
                }
                acked = false;
                next_beat = Instant::now() + interval;
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else {
                    close(&mut sock, 1000).await;
                    return (End::Stopped, worked);
                };
                match cmd {
                    Command::Subscribe { guild_id, channel_id } => {
                        let key = format!("{guild_id}:{channel_id}");
                        if s.ready && !s.subscribed.contains(&key) {
                            s.subscribed.insert(key);
                            let _ = send(&mut sock, &subscribe_payload(&guild_id, &channel_id)).await;
                        }
                    }
                }
            }
            msg = sock.next() => {
                let text = match msg {
                    None => return (End::Reconnect, worked),
                    Some(Err(e)) => {
                        tracing::warn!("[guildchat] gateway read failed: {}", e);
                        return (End::Reconnect, worked);
                    }
                    Some(Ok(WsMessage::Close(frame))) => {
                        let code = frame.map(|f| u16::from(f.code)).unwrap_or(1006);
                        tracing::info!("[guildchat] gateway closed with {}", code);
                        return (close_action(code), worked);
                    }
                    Some(Ok(WsMessage::Binary(b))) => match z.push(&b) {
                        Ok(Some(t)) => t,
                        Ok(None) => continue,
                        Err(e) => {
                            tracing::warn!("[guildchat] gateway decode failed: {}", e);
                            close(&mut sock, 4000).await;
                            return (End::Reconnect, worked);
                        }
                    },
                    Some(Ok(WsMessage::Text(t))) => t.to_string(),
                    Some(Ok(_)) => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                drop(text);
                if let Some(seq) = v.get("s").and_then(|x| x.as_u64()) {
                    s.seq = Some(seq);
                }
                match v.get("op").and_then(|x| x.as_u64()).unwrap_or(99) {
                    10 => {
                        let ms = v["d"]["heartbeat_interval"].as_u64().unwrap_or(41_250).max(1_000);
                        interval = Duration::from_millis(ms);
                        next_beat = Instant::now() + Duration::from_millis(rand::random_range(0..ms));
                        acked = true;
                        let hello = match (&s.id, s.seq) {
                            (Some(id), Some(seq)) => json!({"op": 6, "d": {"token": token, "session_id": id, "seq": seq}}),
                            _ => identify(token),
                        };
                        if !send(&mut sock, &hello).await {
                            return (End::Reconnect, worked);
                        }
                    }
                    11 => acked = true,
                    1 => {
                        let _ = send(&mut sock, &json!({"op": 1, "d": s.seq})).await;
                    }
                    7 => {
                        close(&mut sock, 4000).await;
                        return (End::Reconnect, worked);
                    }
                    9 => {
                        close(&mut sock, 4000).await;
                        if v.get("d").and_then(|x| x.as_bool()) == Some(true) && s.id.is_some() {
                            return (End::Reconnect, worked);
                        }
                        return (End::NewSession, worked);
                    }
                    0 => {
                        let t = v.get("t").and_then(|x| x.as_str()).unwrap_or("");
                        let d = v.get("d").unwrap_or(&Value::Null);
                        if t == "READY" || t == "RESUMED" {
                            worked = true;
                            s.ready = true;
                        }
                        dispatch(t, d, hooks, s);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn mentions_me(m: &Message, me: &str) -> bool {
    m.mention_everyone || m.mentions.iter().any(|u| u.id == me)
}

fn dispatch(t: &str, d: &Value, hooks: &Hooks, s: &mut Session) {
    let emit = &hooks.emit;
    match t {
        "READY" => {
            s.id = d
                .get("session_id")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            s.url = d
                .get("resume_gateway_url")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            if let Some(fresh) = d.get("auth_token").and_then(|x| x.as_str()) {
                (hooks.on_token)(fresh.to_string());
            }
            let snap = model::parse_ready(d);
            s.me = snap.me.id.clone();
            tracing::info!(
                "[guildchat] ready: {} servers, {} direct messages",
                snap.guilds.len(),
                snap.dms.len()
            );
            if let Ok(mut w) = hooks.snapshot.write() {
                *w = snap.clone();
            }
            emit(
                "guildchat-ready",
                serde_json::to_value(&snap).unwrap_or(Value::Null),
            );
            emit("guildchat-status", json!({ "state": "ready" }));
        }
        "RESUMED" => emit(
            "guildchat-status",
            json!({ "state": "ready", "resumed": true }),
        ),
        "MESSAGE_CREATE" => {
            let Some(m) = Message::from_value(d) else {
                return;
            };
            if let Ok(mut w) = hooks.snapshot.write() {
                let me = w.me.id.clone();
                if let Some(c) = w.channel_mut(&m.channel_id) {
                    c.last_message_id = Some(m.id.clone());
                }
                let entry = w.read_states.entry(m.channel_id.clone()).or_default();
                if m.author.id == me {
                    entry.last_message_id = Some(m.id.clone());
                    entry.mention_count = 0;
                } else if mentions_me(&m, &me) {
                    entry.mention_count += 1;
                }
            }
            emit(
                "guildchat-message-create",
                json!({ "message": m, "guild_id": d.get("guild_id") }),
            );
        }
        "MESSAGE_UPDATE" => {
            let full = d.get("content").is_some() && d.get("author").is_some();
            let mut m = Message::from_value(d).unwrap_or_default();
            if m.id.is_empty() {
                return;
            }
            if !full {
                m.author = Default::default();
            }
            emit(
                "guildchat-message-update",
                json!({ "message": m, "partial": !full }),
            );
        }
        "MESSAGE_DELETE" => {
            emit(
                "guildchat-message-delete",
                json!({ "id": d.get("id"), "channel_id": d.get("channel_id") }),
            );
        }
        "MESSAGE_DELETE_BULK" => {
            emit(
                "guildchat-message-delete-bulk",
                json!({ "ids": d.get("ids"), "channel_id": d.get("channel_id") }),
            );
        }
        "MESSAGE_ACK" => {
            let (Some(cid), Some(mid)) = (
                d.get("channel_id").and_then(model::id_of),
                d.get("message_id").and_then(model::id_of),
            ) else {
                return;
            };
            if let Ok(mut w) = hooks.snapshot.write() {
                let e = w.read_states.entry(cid.clone()).or_default();
                e.last_message_id = Some(mid.clone());
                e.mention_count =
                    d.get("mention_count").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            }
            emit(
                "guildchat-read",
                json!({ "channel_id": cid, "message_id": mid, "mention_count": d.get("mention_count") }),
            );
        }
        "GUILD_CREATE" => {
            let Some(g) = model::parse_guild(d, &s.me, None) else {
                return;
            };
            if let Ok(mut w) = hooks.snapshot.write() {
                w.upsert_guild(g.clone());
            }
            emit(
                "guildchat-guild",
                serde_json::to_value(&g).unwrap_or(Value::Null),
            );
        }
        "GUILD_DELETE" => {
            if d.get("unavailable").and_then(|x| x.as_bool()) == Some(true) {
                return;
            }
            let Some(id) = d.get("id").and_then(model::id_of) else {
                return;
            };
            if let Ok(mut w) = hooks.snapshot.write() {
                w.guilds.retain(|g| g.id != id);
            }
            emit("guildchat-guild-remove", json!({ "id": id }));
        }
        "CHANNEL_CREATE" if d.get("guild_id").is_none() => {
            let Some(c) = model::parse_dm(d, &Default::default()) else {
                return;
            };
            if let Ok(mut w) = hooks.snapshot.write() {
                if !w.dms.iter().any(|x| x.id == c.id) {
                    w.dms.insert(0, c.clone());
                }
            }
            emit(
                "guildchat-channel",
                serde_json::to_value(&c).unwrap_or(Value::Null),
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_codes_map_to_actions() {
        assert_eq!(close_action(4004), End::Fatal("unauthorized".into()));
        assert_eq!(close_action(4013), End::Fatal("refused_4013".into()));
        assert_eq!(close_action(4009), End::NewSession);
        assert_eq!(close_action(4007), End::NewSession);
        assert_eq!(close_action(4008), End::Retry);
        assert_eq!(close_action(1006), End::Reconnect);
        assert_eq!(close_action(4000), End::Reconnect);
    }

    #[test]
    fn backoff_doubles_to_a_cap_and_resets() {
        let mut b = BACKOFF_START;
        for _ in 0..10 {
            b = next_backoff(b, false);
        }
        assert_eq!(b, BACKOFF_MAX);
        assert_eq!(next_backoff(b, true), BACKOFF_START);
        assert_eq!(next_backoff(BACKOFF_START, false), Duration::from_secs(4));
    }

    #[test]
    fn identify_carries_omniget_properties() {
        let v = identify("tok");
        assert_eq!(v["op"], 2);
        assert_eq!(v["d"]["capabilities"], props::CAPABILITIES);
        assert_eq!(v["d"]["properties"]["browser"], "OmniGet");
        assert_eq!(v["d"]["compress"], false);
        assert_eq!(
            resume_url("wss://gateway-us-east1-b.discord.gg"),
            "wss://gateway-us-east1-b.discord.gg/?v=9&encoding=json&compress=zlib-stream"
        );
        let sub = subscribe_payload("1", "2");
        assert_eq!(sub["d"]["subscriptions"]["1"]["channels"]["2"][0][1], 99);
    }

    #[test]
    fn dispatch_tracks_reads_and_mentions() {
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let seen2 = seen.clone();
        let hooks = Hooks {
            emit: Arc::new(move |e, _| seen2.lock().unwrap().push(e.to_string())),
            snapshot: snapshot.clone(),
            on_token: Arc::new(|_| {}),
        };
        let mut s = Session::default();
        dispatch(
            "READY",
            &json!({"session_id": "abc", "resume_gateway_url": "wss://x", "user": {"id": "1", "username": "me"},
                "private_channels": [{"id": "5", "type": 1, "recipients": [{"id": "2", "username": "b"}]}]}),
            &hooks,
            &mut s,
        );
        assert_eq!(s.id.as_deref(), Some("abc"));
        dispatch(
            "MESSAGE_CREATE",
            &json!({"id": "10", "channel_id": "5", "author": {"id": "2"}, "mentions": [{"id": "1"}]}),
            &hooks,
            &mut s,
        );
        assert_eq!(snapshot.read().unwrap().read_states["5"].mention_count, 1);
        assert_eq!(
            snapshot.read().unwrap().dms[0].last_message_id.as_deref(),
            Some("10")
        );
        dispatch(
            "MESSAGE_CREATE",
            &json!({"id": "11", "channel_id": "5", "author": {"id": "1"}}),
            &hooks,
            &mut s,
        );
        let rs = snapshot.read().unwrap().read_states["5"].clone();
        assert_eq!(rs.mention_count, 0);
        assert_eq!(rs.last_message_id.as_deref(), Some("11"));
        let events = seen.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                "guildchat-ready",
                "guildchat-status",
                "guildchat-message-create",
                "guildchat-message-create"
            ]
        );
    }
}
