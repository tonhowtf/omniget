use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use ed25519_dalek::{Signature, VerifyingKey};
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{mpsc, Notify};
use tokio::time::Instant;
use tracing::{info, warn};

const OP_WORLD_DIFF: u8 = 40;
const OP_WORLD_INPUT: u8 = 41;
const OP_INTEREST: u8 = 42;
const OP_CHAT: u8 = 43;

const MAX_FRAME: usize = 512 * 1024;
const MAX_HELLO: usize = 4096;
const MAX_VISITOR_FRAME: usize = 4096;
const QUEUE: usize = 32;
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PING_EVERY: Duration = Duration::from_secs(30);
const SILENCE_LIMIT: Duration = Duration::from_secs(90);
const CLOCK_SKEW_MS: u64 = 120_000;
const INPUT_PER_S: u32 = 20;
const CHAT_PER_S: u32 = 5;
const NAME_MAX: usize = 32;
const CHAT_MAX: usize = 500;
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Copy)]
struct Config {
    max_visitors: usize,
    max_rooms: usize,
    max_rooms_per_ip: usize,
    ttl_s: u64,
}

#[derive(Clone)]
struct AppState {
    rooms: Arc<RwLock<HashMap<String, Arc<Room>>>>,
    cfg: Config,
    started: Instant,
}

struct Visitor {
    name: String,
    tx: mpsc::Sender<Message>,
    kick: Arc<Notify>,
}

#[derive(Default)]
struct RoomInner {
    visitors: HashMap<u32, Visitor>,
    closed: bool,
}

struct Room {
    code: String,
    host_name: String,
    house: String,
    host_tx: mpsc::Sender<Message>,
    ip: IpAddr,
    inner: RwLock<RoomInner>,
    next_id: AtomicU32,
    expires: Instant,
}

struct Reject(&'static str, &'static str);

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Hello {
    Open {
        pubkey: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        house: String,
        ts: u64,
        sig: String,
    },
    Join {
        code: String,
        pubkey: String,
        #[serde(default)]
        name: String,
        ts: u64,
        sig: String,
    },
}

enum Role {
    Host,
    Visitor(u32),
}

struct RateLimit {
    per_s: u32,
    window: Instant,
    count: u32,
}

impl RateLimit {
    fn new(per_s: u32) -> Self {
        Self {
            per_s,
            window: Instant::now(),
            count: 0,
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= self.per_s
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn text(v: serde_json::Value) -> Message {
    Message::Text(v.to_string().into())
}

fn clean(raw: &str, max: usize) -> String {
    let no_ctl: String = raw.chars().filter(|c| !c.is_control()).collect();
    no_ctl
        .trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn clean_name(raw: &str) -> String {
    let n = clean(raw, NAME_MAX);
    if n.is_empty() {
        "guest".to_string()
    } else {
        n
    }
}

fn new_code() -> String {
    let bits =
        (rand::random::<u64>() ^ now_ms().wrapping_mul(0x9E37_79B9_7F4A_7C15)) & ((1u64 << 40) - 1);
    let mut s = String::with_capacity(9);
    for i in 0..8 {
        if i == 4 {
            s.push('-');
        }
        s.push(CROCKFORD[((bits >> (35 - 5 * i)) & 31) as usize] as char);
    }
    s
}

fn parse_code(raw: &str) -> Option<String> {
    let mut s = String::with_capacity(9);
    for c in raw.trim().chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !CROCKFORD.contains(&(c as u8)) {
            return None;
        }
        if s.len() == 4 {
            s.push('-');
        }
        s.push(c);
    }
    (s.len() == 9).then_some(s)
}

fn ip_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

fn frame_limit(is_host: bool) -> usize {
    if is_host {
        MAX_FRAME
    } else {
        MAX_VISITOR_FRAME
    }
}

fn verify(pubkey_hex: &str, sig_hex: &str, ts: u64, signed: &str) -> Result<String, Reject> {
    let bad = || {
        Reject(
            "ERR_ROOM_BAD_HELLO",
            "pubkey must be 32 hex bytes and sig 64 hex bytes",
        )
    };
    let pk: [u8; 32] = hex::decode(pubkey_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(bad)?;
    let sg: [u8; 64] = hex::decode(sig_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(bad)?;
    if now_ms().abs_diff(ts) > CLOCK_SKEW_MS {
        return Err(Reject(
            "ERR_ROOM_CLOCK",
            "timestamp is more than 120 s away from server time",
        ));
    }
    let key = VerifyingKey::from_bytes(&pk)
        .map_err(|_| Reject("ERR_ROOM_BAD_SIGNATURE", "invalid public key"))?;
    key.verify_strict(signed.as_bytes(), &Signature::from_bytes(&sg))
        .map_err(|_| Reject("ERR_ROOM_BAD_SIGNATURE", "signature does not match"))?;
    Ok(hex::encode(pk))
}

impl Room {
    fn send_host(&self, msg: Message) {
        let _ = self.host_tx.try_send(msg);
    }

    fn broadcast(&self, msg: &Message, except: Option<u32>) -> Vec<u32> {
        let inner = self.inner.read().unwrap();
        inner
            .visitors
            .iter()
            .filter(|(id, _)| Some(**id) != except)
            .filter_map(|(id, v)| v.tx.try_send(msg.clone()).is_err().then_some(*id))
            .collect()
    }

    fn fan_out(&self, msg: &Message, except: Option<u32>) {
        let slow = self.broadcast(msg, except);
        if !slow.is_empty() {
            self.remove_visitors(slow);
        }
    }

    fn remove_visitors(&self, mut ids: Vec<u32>) {
        while let Some(id) = ids.pop() {
            let (removed, left) = {
                let mut inner = self.inner.write().unwrap();
                (inner.visitors.remove(&id), inner.visitors.len())
            };
            let Some(v) = removed else { continue };
            v.kick.notify_one();
            info!(room = %self.code, visitor_id = id, visitors = left, "visitor left");
            let msg = text(json!({"op": "visitor_left", "visitor_id": id}));
            self.send_host(msg.clone());
            ids.extend(self.broadcast(&msg, None));
        }
    }

    fn join(
        &self,
        max: usize,
        name: &str,
        pubkey: &str,
        tx: mpsc::Sender<Message>,
        kick: Arc<Notify>,
    ) -> Result<u32, Reject> {
        let (id, count) = {
            let mut inner = self.inner.write().unwrap();
            if inner.closed {
                return Err(Reject("ERR_ROOM_NOT_FOUND", "no open house with that code"));
            }
            if inner.visitors.len() >= max {
                return Err(Reject("ERR_ROOM_FULL", "the house is full"));
            }
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let mut others: Vec<_> = inner
                .visitors
                .iter()
                .map(|(i, v)| (*i, v.name.clone()))
                .collect();
            others.sort();
            let others: Vec<_> = others
                .into_iter()
                .map(|(i, n)| json!({"visitor_id": i, "name": n}))
                .collect();
            let _ = tx.try_send(text(json!({
                "op": "joined",
                "visitor_id": id,
                "ent": 1000 + id,
                "host_name": self.host_name,
                "house": self.house,
                "visitors": others,
            })));
            inner.visitors.insert(
                id,
                Visitor {
                    name: name.to_string(),
                    tx,
                    kick,
                },
            );
            (id, inner.visitors.len())
        };
        info!(room = %self.code, visitor_id = id, visitors = count, "visitor joined");
        let msg =
            text(json!({"op": "visitor_joined", "visitor_id": id, "name": name, "pubkey": pubkey}));
        self.send_host(msg.clone());
        self.fan_out(&msg, Some(id));
        Ok(id)
    }

    fn chat(&self, from: u32, name: &str, raw: &[u8]) {
        let Ok(raw) = std::str::from_utf8(raw) else {
            return;
        };
        let body = clean(raw, CHAT_MAX);
        if body.is_empty() {
            return;
        }
        let mut frame = vec![OP_CHAT];
        frame.extend_from_slice(
            json!({"from": from, "name": name, "text": body, "ts": now_ms()})
                .to_string()
                .as_bytes(),
        );
        let msg = Message::Binary(Bytes::from(frame));
        self.send_host(msg.clone());
        self.fan_out(&msg, None);
    }
}

impl AppState {
    fn open_room(
        &self,
        ip: IpAddr,
        host_name: String,
        house: String,
        host_tx: mpsc::Sender<Message>,
    ) -> Result<Arc<Room>, Reject> {
        let mut rooms = self.rooms.write().unwrap();
        if rooms.len() >= self.cfg.max_rooms {
            return Err(Reject(
                "ERR_ROOM_SERVER_FULL",
                "the server is at its room limit",
            ));
        }
        if rooms.values().filter(|r| r.ip == ip).count() >= self.cfg.max_rooms_per_ip {
            return Err(Reject(
                "ERR_ROOM_TOO_MANY",
                "this address already has the maximum number of open rooms",
            ));
        }
        let mut code = new_code();
        while rooms.contains_key(&code) {
            code = new_code();
        }
        let _ = host_tx.try_send(text(json!({
            "op": "opened",
            "code": code,
            "ttl_s": self.cfg.ttl_s,
            "max_visitors": self.cfg.max_visitors,
        })));
        let room = Arc::new(Room {
            code: code.clone(),
            host_name,
            house,
            host_tx,
            ip,
            inner: RwLock::new(RoomInner::default()),
            next_id: AtomicU32::new(1),
            expires: Instant::now() + Duration::from_secs(self.cfg.ttl_s),
        });
        rooms.insert(code, room.clone());
        info!(room = %room.code, rooms = rooms.len(), "room opened");
        Ok(room)
    }

    fn close_room(&self, room: &Arc<Room>, reason: &'static str) {
        let rooms_left = {
            let mut rooms = self.rooms.write().unwrap();
            if rooms.get(&room.code).is_some_and(|r| Arc::ptr_eq(r, room)) {
                rooms.remove(&room.code);
            }
            rooms.len()
        };
        let visitors: Vec<Visitor> = {
            let mut inner = room.inner.write().unwrap();
            inner.closed = true;
            inner.visitors.drain().map(|(_, v)| v).collect()
        };
        let msg = text(json!({"op": "closed", "reason": reason}));
        for v in &visitors {
            if v.tx.try_send(msg.clone()).is_err() || v.tx.try_send(Message::Close(None)).is_err() {
                v.kick.notify_one();
            }
        }
        info!(room = %room.code, reason, visitors = visitors.len(), rooms = rooms_left, "room closed");
    }
}

async fn reject(sink: &mut SplitSink<WebSocket, Message>, r: Reject) {
    warn!(code = r.0, "hello rejected");
    let _ = sink
        .send(text(json!({"op": "error", "code": r.0, "message": r.1})))
        .await;
    let _ = sink.send(Message::Close(None)).await;
}

async fn first_text(stream: &mut SplitStream<WebSocket>) -> Option<Result<String, Reject>> {
    loop {
        match stream.next().await? {
            Ok(Message::Text(t)) if t.len() <= MAX_HELLO => {
                return Some(Ok(t.as_str().to_string()))
            }
            Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {
                return Some(Err(Reject(
                    "ERR_ROOM_BAD_HELLO",
                    "the first frame must be a small JSON text frame",
                )))
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => return None,
        }
    }
}

async fn writer(mut sink: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Message>) {
    let mut ping = tokio::time::interval_at(Instant::now() + PING_EVERY, PING_EVERY);
    loop {
        tokio::select! {
            m = rx.recv() => {
                let m = m.unwrap_or(Message::Close(None));
                let last = matches!(m, Message::Close(_));
                if sink.send(m).await.is_err() || last {
                    break;
                }
            }
            _ = ping.tick() => {
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, ip: IpAddr) {
    let (mut sink, mut stream) = socket.split();
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, first_text(&mut stream)).await {
        Ok(Some(Ok(t))) => t,
        Ok(Some(Err(r))) => return reject(&mut sink, r).await,
        Ok(None) => return,
        Err(_) => {
            let _ = sink.send(Message::Close(None)).await;
            return;
        }
    };
    let hello: Hello = match serde_json::from_str(&hello) {
        Ok(h) => h,
        Err(_) => return reject(&mut sink, Reject("ERR_ROOM_BAD_HELLO", "malformed hello")).await,
    };

    let (tx, rx) = mpsc::channel::<Message>(QUEUE);
    let kick = Arc::new(Notify::new());
    let setup = match hello {
        Hello::Open {
            pubkey,
            name,
            house,
            ts,
            sig,
        } => verify(&pubkey, &sig, ts, &format!("omniworld:open:{ts}")).and_then(|_| {
            let house = clean(&house, NAME_MAX);
            let house = if house.is_empty() {
                "casa-v1".to_string()
            } else {
                house
            };
            let name = clean_name(&name);
            let room = state.open_room(ip, name.clone(), house, tx.clone())?;
            Ok((room, Role::Host, name))
        }),
        Hello::Join {
            code,
            pubkey,
            name,
            ts,
            sig,
        } => {
            let signed = format!("omniworld:join:{}:{ts}", code.to_ascii_uppercase());
            verify(&pubkey, &sig, ts, &signed).and_then(|pubkey| {
                let not_found = || Reject("ERR_ROOM_NOT_FOUND", "no open house with that code");
                let canonical = parse_code(&code).ok_or_else(not_found)?;
                let room = state
                    .rooms
                    .read()
                    .unwrap()
                    .get(&canonical)
                    .cloned()
                    .ok_or_else(not_found)?;
                let name = clean_name(&name);
                let id = room.join(
                    state.cfg.max_visitors,
                    &name,
                    &pubkey,
                    tx.clone(),
                    kick.clone(),
                )?;
                Ok((room, Role::Visitor(id), name))
            })
        }
    };
    let (room, role, name) = match setup {
        Ok(s) => s,
        Err(r) => return reject(&mut sink, r).await,
    };

    let mut writer_task = tokio::spawn(writer(sink, rx));
    let mut writer_done = false;
    let mut kicked = false;
    let mut reason = "host_left";
    let is_host = matches!(role, Role::Host);
    let from = match role {
        Role::Host => 0,
        Role::Visitor(id) => id,
    };
    let mut input_limit = RateLimit::new(INPUT_PER_S);
    let mut interest_limit = RateLimit::new(INPUT_PER_S);
    let mut chat_limit = RateLimit::new(CHAT_PER_S);
    let ttl = tokio::time::sleep_until(room.expires);
    tokio::pin!(ttl);

    loop {
        let frame = tokio::select! {
            f = tokio::time::timeout(SILENCE_LIMIT, stream.next()) => f,
            _ = &mut writer_task => { writer_done = true; break; }
            _ = kick.notified() => { kicked = true; break; }
            _ = &mut ttl, if is_host => { reason = "ttl"; break; }
        };
        let data = match frame {
            Ok(Some(Ok(Message::Binary(b)))) => b,
            Ok(Some(Ok(Message::Text(t)))) if t.len() > frame_limit(is_host) => break,
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        };
        if data.len() > frame_limit(is_host) {
            break;
        }
        let Some(&op) = data.first() else { continue };
        match (op, is_host) {
            (OP_WORLD_DIFF, true) => room.fan_out(&Message::Binary(data), None),
            (OP_WORLD_INPUT | OP_INTEREST, false) => {
                let limit = if op == OP_WORLD_INPUT {
                    &mut input_limit
                } else {
                    &mut interest_limit
                };
                if !limit.allow() {
                    continue;
                }
                let mut out = Vec::with_capacity(data.len() + 4);
                out.push(op);
                out.extend_from_slice(&from.to_be_bytes());
                out.extend_from_slice(&data[1..]);
                room.send_host(Message::Binary(Bytes::from(out)));
            }
            (OP_CHAT, _) if chat_limit.allow() => {
                room.chat(from, &name, &data[1..]);
            }
            _ => {}
        }
    }

    if is_host {
        state.close_room(&room, reason);
        if reason == "ttl" {
            let _ = tx.try_send(text(json!({"op": "closed", "reason": "ttl"})));
        }
    } else {
        room.remove_visitors(vec![from]);
    }
    drop(room);

    if !writer_done {
        if !kicked {
            let _ = tx.try_send(Message::Close(None));
        }
        if kicked
            || tokio::time::timeout(Duration::from_secs(2), &mut writer_task)
                .await
                .is_err()
        {
            writer_task.abort();
        }
    }
}

async fn room_endpoint(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let ip = ip_key(addr.ip());
    ws.max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| handle_socket(socket, state, ip))
}

async fn stats(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (rooms, visitors) = {
        let rooms = state.rooms.read().unwrap();
        let visitors: usize = rooms
            .values()
            .map(|r| r.inner.read().unwrap().visitors.len())
            .sum();
        (rooms.len(), visitors)
    };
    Json(
        json!({"rooms": rooms, "visitors": visitors, "uptime_s": state.started.elapsed().as_secs()}),
    )
}

fn env_or<T: FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config {
        max_visitors: env_or("OMNIWORLD_MAX_VISITORS", 8),
        max_rooms: env_or("OMNIWORLD_MAX_ROOMS", 200),
        max_rooms_per_ip: env_or("OMNIWORLD_MAX_ROOMS_PER_IP", 3),
        ttl_s: env_or("OMNIWORLD_TTL_S", 21_600),
    };
    let bind: String = env_or("OMNIWORLD_BIND", "0.0.0.0:7878".to_string());
    let state = AppState {
        rooms: Arc::default(),
        cfg,
        started: Instant::now(),
    };

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/stats", get(stats))
        .route("/v1/room", get(room_endpoint))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(%bind, error = %e, "cannot bind");
            std::process::exit(1);
        }
    };
    info!(%bind, max_visitors = cfg.max_visitors, max_rooms = cfg.max_rooms, max_rooms_per_ip = cfg.max_rooms_per_ip, ttl_s = cfg.ttl_s, "omniworld-server listening");

    tokio::select! {
        r = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()) => {
            if let Err(e) = r {
                tracing::error!(error = %e, "server stopped");
            }
        }
        _ = shutdown_signal() => info!("shutting down"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(max_rooms: usize, max_rooms_per_ip: usize) -> AppState {
        AppState {
            rooms: Arc::default(),
            cfg: Config {
                max_visitors: 8,
                max_rooms,
                max_rooms_per_ip,
                ttl_s: 60,
            },
            started: Instant::now(),
        }
    }

    fn open(state: &AppState, ip: &str) -> Result<Arc<Room>, Reject> {
        let (tx, _rx) = mpsc::channel(QUEUE);
        let ip = ip_key(ip.parse().expect("test ip"));
        state.open_room(ip, "host".to_string(), "casa-v1".to_string(), tx)
    }

    #[test]
    fn one_address_cannot_open_more_than_its_share_of_rooms() {
        let state = state(200, 2);
        assert!(open(&state, "203.0.113.7").is_ok());
        assert!(open(&state, "203.0.113.7").is_ok());
        assert!(matches!(
            open(&state, "203.0.113.7"),
            Err(Reject("ERR_ROOM_TOO_MANY", _))
        ));
        assert!(open(&state, "203.0.113.8").is_ok());
        assert_eq!(state.rooms.read().unwrap().len(), 3);
    }

    #[test]
    fn closing_a_room_frees_the_slot_of_its_address() {
        let state = state(200, 1);
        let Ok(room) = open(&state, "203.0.113.7") else {
            panic!("first room must open");
        };
        assert!(open(&state, "203.0.113.7").is_err());
        state.close_room(&room, "host_left");
        assert!(open(&state, "203.0.113.7").is_ok());
    }

    #[test]
    fn the_server_room_limit_still_applies_across_addresses() {
        let state = state(1, 3);
        assert!(open(&state, "203.0.113.7").is_ok());
        assert!(matches!(
            open(&state, "203.0.113.8"),
            Err(Reject("ERR_ROOM_SERVER_FULL", _))
        ));
    }

    #[test]
    fn ipv6_addresses_of_one_subnet_share_a_slot() {
        let state = state(200, 1);
        assert!(open(&state, "2001:db8:1:2::1").is_ok());
        assert!(open(&state, "2001:db8:1:2:ffff::9").is_err());
        assert!(open(&state, "2001:db8:1:3::1").is_ok());
        assert_eq!(
            ip_key("::ffff:203.0.113.7".parse().expect("test ip")),
            "203.0.113.7".parse::<IpAddr>().expect("test ip")
        );
    }

    #[test]
    fn visitor_frames_have_a_small_cap_and_host_frames_keep_the_scene_size() {
        assert_eq!(frame_limit(true), MAX_FRAME);
        assert_eq!(frame_limit(false), MAX_VISITOR_FRAME);
        assert!(500 * 4 < frame_limit(false));
        assert!(400 * 1024 < frame_limit(true));
    }
}
