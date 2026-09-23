//! Remote access to the Central (plan §3.9, T11): pair a phone with a QR code
//! and follow or drive the threads from a small PWA.
//!
//! The local bridge (`local_bridge.rs`) keeps listening on `127.0.0.1` only.
//! Remote access is an explicit opt-in that starts a **second listener** on
//! one interface the user picks (loopback for `tailscale serve`/`ssh -L`, a
//! LAN address or the Tailscale address). That listener serves only
//! `/remote/*`, authenticated per device; every other path answers 401, so
//! the local routes are never reachable from it.
//!
//! Pairing (the T3 flow): the app mints a one-time secret and shows
//! `http://<host>:<port>/remote/#pair=<secret>&scope=<read|drive>` as a QR
//! code. The secret sits after the `#`, so it never reaches a server log.
//! The PWA posts it to `/remote/api/pair` and gets a per-device bearer; only
//! the bearer's SHA-256 is stored (`<app_data>/remote/devices.json`). The
//! scope comes from the server-side record of the secret, never from the URL.
//! The event socket (`GET /remote/ws?ticket=…&after=<seq>`) takes a 60 s,
//! single-use ticket minted with the bearer (`POST /remote/api/ws-ticket`),
//! since browsers cannot put headers on a WebSocket.
//!
//! Scopes: `read` sees threads, timelines, diffs and outside sessions;
//! `drive` can also send a message, answer an approval or a question, and
//! interrupt a turn. Nothing else is dispatchable from a phone.
//!
//! No relay: the phone reaches the machine over the LAN, over Tailscale
//! (optionally through `tailscale serve`, run only after the user confirms)
//! or through an `ssh -L` tunnel. Nothing runs while remote access is off,
//! and `tailscale` is only spawned when the settings page asks.
//!
//! The WebSocket shares the port with axum: a small accept loop peeks the
//! request line and hands `GET /remote/ws` connections to tungstenite, the
//! rest to axum (the host's axum is built without its `ws` feature).

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{ConnectInfo, FromRequestParts, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::serve::{IncomingStream, Listener};
use axum::{Json, Router};
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use omniget_core::core::threads::{Command, CommandEnvelope, ThreadsEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, Socket, Type};
use tauri::{AppHandle, Emitter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::tungstenite::{self, Message};

/// Default port of the remote listener (the local bridge uses 47720..47730).
pub const DEFAULT_PORT: u16 = 47740;
/// Default HTTPS port `tailscale serve` publishes on (443 is often taken).
pub const DEFAULT_TAILSCALE_HTTPS_PORT: u16 = 8443;
/// Lifetime of a pairing secret.
pub const PAIR_TTL_SECS: i64 = 600;
/// Lifetime of a WebSocket ticket.
pub const TICKET_TTL_SECS: i64 = 60;
/// UI event: devices, access log or listener state changed.
pub const EVENT_REMOTE: &str = "remote://changed";

const LOG_CAP: usize = 300;
const LOG_FILE_CAP: u64 = 256 * 1024;
const PAIR_FAIL_WINDOW_MS: i64 = 5 * 60 * 1000;
const PAIR_FAIL_MAX: usize = 10;
const LAST_USED_PERSIST_MS: i64 = 60 * 1000;
const WS_PING_SECS: u64 = 25;

// ── PWA shell (embedded so dev and release builds serve the same bytes) ──

const PWA_FILES: &[(&str, &str, &str)] = &[
    (
        "index.html",
        "text/html; charset=utf-8",
        include_str!("../../static/remote/index.html"),
    ),
    (
        "app.js",
        "text/javascript; charset=utf-8",
        include_str!("../../static/remote/app.js"),
    ),
    (
        "app.css",
        "text/css; charset=utf-8",
        include_str!("../../static/remote/app.css"),
    ),
    (
        "sw.js",
        "text/javascript; charset=utf-8",
        include_str!("../../static/remote/sw.js"),
    ),
    (
        "manifest.webmanifest",
        "application/manifest+json",
        include_str!("../../static/remote/manifest.webmanifest"),
    ),
    (
        "icon.svg",
        "image/svg+xml",
        include_str!("../../static/remote/icon.svg"),
    ),
];

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
connect-src 'self' ws: wss:; manifest-src 'self'; worker-src 'self'; base-uri 'none'; \
form-action 'none'; frame-ancestors 'none'";

// ── Model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Read,
    Drive,
}

impl Scope {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Scope::Read),
            "drive" => Ok(Scope::Drive),
            other => Err(format!("REMOTE_SCOPE_INVALID: unknown scope {other:?}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Drive => "drive",
        }
    }
}

/// What a phone may dispatch. `read` nothing; `drive` only the four actions
/// of the plan (send, approve/decline, answer, interrupt) plus "visited".
/// `acceptAlways` writes a permanent permission rule, so it stays a desktop
/// decision.
pub fn command_allowed(scope: Scope, command: &Command) -> Result<(), &'static str> {
    if scope != Scope::Drive {
        return Err("read-only device");
    }
    match command {
        Command::TurnStart { .. }
        | Command::TurnInterrupt { .. }
        | Command::UserInputRespond { .. }
        | Command::Visit { .. } => Ok(()),
        Command::ApprovalRespond { decision, .. } => {
            let v = serde_json::to_value(decision).unwrap_or(Value::Null);
            if v.as_str() == Some("acceptAlways") {
                Err("acceptAlways is only available on the desktop")
            } else {
                Ok(())
            }
        }
        _ => Err("command not available remotely"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceRecord {
    id: String,
    name: String,
    scope: Scope,
    token_hash: String,
    created_at: String,
    #[serde(default)]
    last_used_at: Option<String>,
    #[serde(default)]
    last_ip: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub last_ip: Option<String>,
    pub user_agent: Option<String>,
    /// Open event sockets of this device right now.
    pub connected: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DevicesFile {
    version: u32,
    devices: Vec<DeviceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteConfig {
    /// The opt-in: the listener comes back on the next launch when true.
    pub enabled: bool,
    pub bind_ip: String,
    pub port: u16,
    pub tailscale_https_port: u16,
    /// `https://<machine>.<tailnet>.ts.net[:port]` once `tailscale serve`
    /// was turned on from here.
    pub tailscale_serve_url: Option<String>,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_ip: "127.0.0.1".into(),
            port: DEFAULT_PORT,
            tailscale_https_port: DEFAULT_TAILSCALE_HTTPS_PORT,
            tailscale_serve_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessEntry {
    pub at: String,
    pub ip: String,
    pub device_id: Option<String>,
    pub device: Option<String>,
    pub method: String,
    pub path: String,
    pub status: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceOption {
    pub ip: String,
    /// `loopback|lan|tailscale|other`
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    /// `direct|tailscale-serve|ssh`
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub enabled: bool,
    pub running: bool,
    pub bind_ip: String,
    pub port: u16,
    pub bound: Option<String>,
    pub interfaces: Vec<InterfaceOption>,
    pub endpoints: Vec<Endpoint>,
    pub devices: u32,
    pub pending_pairs: u32,
    pub tailscale_https_port: u16,
    pub tailscale_serve_url: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairLink {
    pub url: String,
    pub base: String,
    pub scope: Scope,
    pub label: Option<String>,
    pub expires_at: String,
    pub qr_svg: String,
}

struct PendingPair {
    secret_hash: String,
    scope: Scope,
    label: Option<String>,
    expires_ms: i64,
}

struct Ticket {
    device_id: String,
    expires_ms: i64,
}

struct Running {
    addr: SocketAddr,
    stop: watch::Sender<bool>,
}

// ── Backend (threads engine + diff); the app one and a test one ─────────

#[async_trait::async_trait]
pub trait RemoteBackend: Send + Sync + 'static {
    async fn engine(&self) -> Result<Arc<ThreadsEngine>, String>;
    async fn diff(&self, thread_id: &str, turn: Option<u32>) -> Result<Value, String>;
}

pub struct AppBackend(pub AppHandle);

#[async_trait::async_trait]
impl RemoteBackend for AppBackend {
    async fn engine(&self) -> Result<Arc<ThreadsEngine>, String> {
        Ok(crate::threads_host::get(&self.0)?.engine.clone())
    }

    async fn diff(&self, thread_id: &str, turn: Option<u32>) -> Result<Value, String> {
        let host = crate::threads_host::get(&self.0)?;
        let opts = omniget_core::core::vcs::diff::DiffOptions {
            max_bytes: 768 * 1024,
            max_file_bytes: 192 * 1024,
            ..Default::default()
        };
        let d = host.git.diff(thread_id, turn, &opts).await?;
        serde_json::to_value(d).map_err(|e| format!("REMOTE_JSON: {e}"))
    }
}

// ── Hub ──────────────────────────────────────────────────────────────────

pub struct RemoteHub {
    dir: Option<PathBuf>,
    cfg: Mutex<RemoteConfig>,
    devices: Mutex<Vec<DeviceRecord>>,
    pending: Mutex<Vec<PendingPair>>,
    tickets: Mutex<HashMap<String, Ticket>>,
    log: Mutex<VecDeque<AccessEntry>>,
    pair_failures: Mutex<VecDeque<i64>>,
    connections: Mutex<HashMap<String, u32>>,
    running: Mutex<Option<Running>>,
    last_error: Mutex<Option<String>>,
    revoked: broadcast::Sender<String>,
    app: Mutex<Option<AppHandle>>,
    last_persist_ms: AtomicI64,
}

static HUB: OnceLock<Arc<RemoteHub>> = OnceLock::new();

/// The process-wide hub, loaded from `<app_data>/remote/` on first use.
pub fn hub() -> Arc<RemoteHub> {
    HUB.get_or_init(|| {
        RemoteHub::new(omniget_core::core::paths::app_data_dir().map(|d| d.join("remote")))
    })
    .clone()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn iso_of_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut buf);
    hex::encode(buf)
}

pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.trim().as_bytes()))
}

fn write_private(path: &FsPath, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
}

impl RemoteHub {
    pub fn new(dir: Option<PathBuf>) -> Arc<Self> {
        let cfg: RemoteConfig = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join("config.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let devices: DevicesFile = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join("devices.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut log = VecDeque::with_capacity(LOG_CAP);
        if let Some(text) = dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("access.jsonl")).ok())
        {
            for line in text.lines() {
                if let Ok(e) = serde_json::from_str::<AccessEntry>(line) {
                    if log.len() == LOG_CAP {
                        log.pop_front();
                    }
                    log.push_back(e);
                }
            }
        }
        let (revoked, _) = broadcast::channel(16);
        Arc::new(Self {
            dir,
            cfg: Mutex::new(cfg),
            devices: Mutex::new(devices.devices),
            pending: Mutex::new(Vec::new()),
            tickets: Mutex::new(HashMap::new()),
            log: Mutex::new(log),
            pair_failures: Mutex::new(VecDeque::new()),
            connections: Mutex::new(HashMap::new()),
            running: Mutex::new(None),
            last_error: Mutex::new(None),
            revoked,
            app: Mutex::new(None),
            last_persist_ms: AtomicI64::new(0),
        })
    }

    pub fn set_app(&self, app: &AppHandle) {
        *self.app.lock().unwrap() = Some(app.clone());
    }

    fn notify(&self, kind: &str) {
        if let Some(app) = self.app.lock().unwrap().as_ref() {
            let _ = app.emit(EVENT_REMOTE, json!({ "kind": kind }));
        }
    }

    pub fn config(&self) -> RemoteConfig {
        self.cfg.lock().unwrap().clone()
    }

    fn save_config(&self) {
        let Some(dir) = &self.dir else { return };
        let cfg = self.config();
        if let Ok(bytes) = serde_json::to_vec_pretty(&cfg) {
            if let Err(e) = write_private(&dir.join("config.json"), &bytes) {
                tracing::warn!("[remote] saving config.json failed: {e}");
            }
        }
    }

    fn save_devices(&self) {
        self.last_persist_ms.store(now_ms(), Ordering::Relaxed);
        let Some(dir) = &self.dir else { return };
        let file = DevicesFile {
            version: 1,
            devices: self.devices.lock().unwrap().clone(),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            if let Err(e) = write_private(&dir.join("devices.json"), &bytes) {
                tracing::warn!("[remote] saving devices.json failed: {e}");
            }
        }
    }

    // ── Pairing ──

    /// Mints a one-time secret for `scope` and returns the link for `base`
    /// (`http(s)://host:port`, no path).
    pub fn create_pairing(
        &self,
        scope: Scope,
        base: &str,
        label: Option<String>,
    ) -> Result<PairLink, String> {
        let base = normalize_base(base)?;
        let secret = random_b64(24);
        let expires_ms = now_ms() + PAIR_TTL_SECS * 1000;
        let label = label
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty());
        {
            let mut p = self.pending.lock().unwrap();
            let now = now_ms();
            p.retain(|x| x.expires_ms > now);
            p.push(PendingPair {
                secret_hash: hash_secret(&secret),
                scope,
                label: label.clone(),
                expires_ms,
            });
        }
        let url = format!("{base}/remote/#pair={secret}&scope={}", scope.as_str());
        Ok(PairLink {
            qr_svg: render_qr(&url)?,
            url,
            base,
            scope,
            label,
            expires_at: iso_of_ms(expires_ms),
        })
    }

    pub fn cancel_pairings(&self) -> u32 {
        let mut p = self.pending.lock().unwrap();
        let n = p.len() as u32;
        p.clear();
        n
    }

    fn pair_locked(&self) -> bool {
        let mut f = self.pair_failures.lock().unwrap();
        let cutoff = now_ms() - PAIR_FAIL_WINDOW_MS;
        while f.front().is_some_and(|t| *t < cutoff) {
            f.pop_front();
        }
        f.len() >= PAIR_FAIL_MAX
    }

    /// Trades a pairing secret for a device bearer. Returns `(token, device)`.
    fn redeem_pairing(
        &self,
        secret: &str,
        name: Option<String>,
        ip: &str,
        user_agent: Option<String>,
    ) -> Result<(String, DeviceView), (StatusCode, &'static str)> {
        if self.pair_locked() {
            return Err((StatusCode::TOO_MANY_REQUESTS, "too many failed pairings"));
        }
        let hash = hash_secret(secret);
        let found = {
            let mut p = self.pending.lock().unwrap();
            let now = now_ms();
            p.retain(|x| x.expires_ms > now);
            let idx = p.iter().position(|x| {
                constant_time_eq::constant_time_eq(x.secret_hash.as_bytes(), hash.as_bytes())
            });
            idx.map(|i| p.remove(i))
        };
        let Some(pending) = found else {
            self.pair_failures.lock().unwrap().push_back(now_ms());
            return Err((StatusCode::UNAUTHORIZED, "invalid or expired pairing code"));
        };
        let token = crate::local_bridge::generate_token();
        let name = name
            .map(|n| n.trim().chars().take(60).collect::<String>())
            .filter(|n| !n.is_empty())
            .or(pending.label)
            .unwrap_or_else(|| guess_device_name(user_agent.as_deref()));
        let rec = DeviceRecord {
            id: format!("dev_{}", random_hex(6)),
            name,
            scope: pending.scope,
            token_hash: hash_secret(&token),
            created_at: now_iso(),
            last_used_at: Some(now_iso()),
            last_ip: Some(ip.to_string()),
            user_agent: user_agent.map(|u| u.chars().take(200).collect()),
        };
        let view = self.view_of(&rec);
        self.devices.lock().unwrap().push(rec);
        self.save_devices();
        self.notify("paired");
        Ok((token, view))
    }

    fn view_of(&self, d: &DeviceRecord) -> DeviceView {
        DeviceView {
            id: d.id.clone(),
            name: d.name.clone(),
            scope: d.scope,
            created_at: d.created_at.clone(),
            last_used_at: d.last_used_at.clone(),
            last_ip: d.last_ip.clone(),
            user_agent: d.user_agent.clone(),
            connected: self
                .connections
                .lock()
                .unwrap()
                .get(&d.id)
                .copied()
                .unwrap_or(0),
        }
    }

    // ── Devices ──

    pub fn devices(&self) -> Vec<DeviceView> {
        let list = self.devices.lock().unwrap().clone();
        list.iter().map(|d| self.view_of(d)).collect()
    }

    pub fn revoke(&self, id: &str) -> bool {
        let removed = {
            let mut d = self.devices.lock().unwrap();
            let before = d.len();
            d.retain(|x| x.id != id);
            before != d.len()
        };
        if removed {
            self.tickets
                .lock()
                .unwrap()
                .retain(|_, t| t.device_id != id);
            let _ = self.revoked.send(id.to_string());
            self.save_devices();
            self.notify("revoked");
        }
        removed
    }

    pub fn rename(&self, id: &str, name: &str) -> bool {
        let name: String = name.trim().chars().take(60).collect();
        if name.is_empty() {
            return false;
        }
        let ok = {
            let mut d = self.devices.lock().unwrap();
            match d.iter_mut().find(|x| x.id == id) {
                Some(x) => {
                    x.name = name;
                    true
                }
                None => false,
            }
        };
        if ok {
            self.save_devices();
            self.notify("renamed");
        }
        ok
    }

    /// The device of a bearer, with its last use bumped.
    fn authenticate(&self, token: &str, ip: &str) -> Option<DeviceRecord> {
        let hash = hash_secret(token);
        let (rec, persist) = {
            let mut d = self.devices.lock().unwrap();
            let rec = d.iter_mut().find(|x| {
                constant_time_eq::constant_time_eq(x.token_hash.as_bytes(), hash.as_bytes())
            })?;
            rec.last_used_at = Some(now_iso());
            rec.last_ip = Some(ip.to_string());
            let persist =
                now_ms() - self.last_persist_ms.load(Ordering::Relaxed) > LAST_USED_PERSIST_MS;
            (rec.clone(), persist)
        };
        if persist {
            self.save_devices();
        }
        Some(rec)
    }

    fn device(&self, id: &str) -> Option<DeviceRecord> {
        self.devices
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.id == id)
            .cloned()
    }

    // ── Tickets ──

    fn mint_ticket(&self, device_id: &str) -> (String, i64) {
        let ticket = random_b64(24);
        let expires_ms = now_ms() + TICKET_TTL_SECS * 1000;
        let mut t = self.tickets.lock().unwrap();
        let now = now_ms();
        t.retain(|_, x| x.expires_ms > now);
        t.insert(
            ticket.clone(),
            Ticket {
                device_id: device_id.to_string(),
                expires_ms,
            },
        );
        (ticket, expires_ms)
    }

    /// Single use: the ticket is gone after this call either way.
    fn redeem_ticket(&self, ticket: &str) -> Option<DeviceRecord> {
        let t = self.tickets.lock().unwrap().remove(ticket)?;
        if t.expires_ms <= now_ms() {
            return None;
        }
        self.device(&t.device_id)
    }

    // ── Access log ──

    fn record(&self, entry: AccessEntry) {
        {
            let mut l = self.log.lock().unwrap();
            if l.len() == LOG_CAP {
                l.pop_front();
            }
            l.push_back(entry.clone());
        }
        if let Some(dir) = &self.dir {
            let path = dir.join("access.jsonl");
            let too_big = std::fs::metadata(&path)
                .map(|m| m.len() > LOG_FILE_CAP)
                .unwrap_or(false);
            if too_big {
                let text: String = self
                    .log
                    .lock()
                    .unwrap()
                    .iter()
                    .filter_map(|e| serde_json::to_string(e).ok())
                    .map(|s| s + "\n")
                    .collect();
                let _ = write_private(&path, text.as_bytes());
            } else if let Ok(line) = serde_json::to_string(&entry) {
                let _ = std::fs::create_dir_all(dir);
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
        self.notify("access");
    }

    pub fn access_log(&self, limit: usize) -> Vec<AccessEntry> {
        let l = self.log.lock().unwrap();
        l.iter().rev().take(limit).cloned().collect()
    }

    pub fn clear_access_log(&self) {
        self.log.lock().unwrap().clear();
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join("access.jsonl"));
        }
        self.notify("access");
    }

    // ── Listener ──

    pub fn status(&self) -> RemoteStatus {
        let cfg = self.config();
        let bound = self.running.lock().unwrap().as_ref().map(|r| r.addr);
        let now = now_ms();
        let pending = {
            let mut p = self.pending.lock().unwrap();
            p.retain(|x| x.expires_ms > now);
            p.len() as u32
        };
        let mut interfaces = candidate_interfaces();
        if !cfg.bind_ip.is_empty() && !interfaces.iter().any(|i| i.ip == cfg.bind_ip) {
            if let Ok(ip) = cfg.bind_ip.parse::<IpAddr>() {
                interfaces.push(InterfaceOption {
                    ip: cfg.bind_ip.clone(),
                    kind: classify_ip(ip).into(),
                });
            }
        }
        let mut endpoints = Vec::new();
        if let Some(addr) = bound {
            endpoints.push(Endpoint {
                kind: "direct".into(),
                url: base_for(addr.ip(), addr.port()),
            });
            if let Some(u) = &cfg.tailscale_serve_url {
                endpoints.push(Endpoint {
                    kind: "tailscale-serve".into(),
                    url: u.clone(),
                });
            }
        }
        RemoteStatus {
            enabled: cfg.enabled,
            running: bound.is_some(),
            bind_ip: cfg.bind_ip,
            port: bound.map(|a| a.port()).unwrap_or(cfg.port),
            bound: bound.map(|a| a.to_string()),
            interfaces,
            endpoints,
            devices: self.devices.lock().unwrap().len() as u32,
            pending_pairs: pending,
            tailscale_https_port: cfg.tailscale_https_port,
            tailscale_serve_url: cfg.tailscale_serve_url,
            last_error: self.last_error.lock().unwrap().clone(),
        }
    }

    pub fn bound_addr(&self) -> Option<SocketAddr> {
        self.running.lock().unwrap().as_ref().map(|r| r.addr)
    }

    /// Starts (or restarts) the remote listener on `bind_ip:port`.
    pub async fn start(
        self: &Arc<Self>,
        backend: Arc<dyn RemoteBackend>,
        bind_ip: &str,
        port: u16,
    ) -> Result<SocketAddr, String> {
        let ip: IpAddr = bind_ip
            .trim()
            .parse()
            .map_err(|_| format!("REMOTE_BIND_INVALID: {bind_ip:?} is not an IP address"))?;
        if ip.is_unspecified() {
            return Err(
                "REMOTE_BIND_INVALID: pick one interface, not every interface (0.0.0.0)".into(),
            );
        }
        self.stop_listener();
        let (addr, listener) = match bind_remote(ip, port) {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("REMOTE_BIND_FAILED: {ip}:{port}: {e}");
                *self.last_error.lock().unwrap() = Some(msg.clone());
                return Err(msg);
            }
        };
        let (stop_tx, stop_rx) = watch::channel(false);
        let ctx = Ctx {
            hub: self.clone(),
            backend,
            stop: stop_rx.clone(),
        };
        let (tx, rx) = mpsc::channel::<(TcpStream, SocketAddr)>(64);
        let accept_ctx = ctx.clone();
        let mut accept_stop = stop_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = accept_stop.changed() => break,
                    r = listener.accept() => match r {
                        Ok((stream, peer)) => {
                            let tx = tx.clone();
                            let ctx = accept_ctx.clone();
                            tokio::spawn(async move { route_connection(stream, peer, tx, ctx).await });
                        }
                        Err(e) => {
                            tracing::debug!("[remote] accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            }
        });
        let router = http_router(ctx);
        let mut serve_stop = stop_rx;
        tokio::spawn(async move {
            let listener = SplitListener { rx, local: addr };
            let svc = router.into_make_service_with_connect_info::<Peer>();
            let result = axum::serve(listener, svc)
                .with_graceful_shutdown(async move {
                    let _ = serve_stop.changed().await;
                })
                .await;
            if let Err(e) = result {
                tracing::warn!("[remote] listener stopped: {e}");
            }
        });
        *self.running.lock().unwrap() = Some(Running {
            addr,
            stop: stop_tx,
        });
        *self.last_error.lock().unwrap() = None;
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.enabled = true;
            cfg.bind_ip = ip.to_string();
            cfg.port = addr.port();
        }
        self.save_config();
        tracing::info!("[remote] listening on http://{addr}/remote/");
        self.notify("started");
        Ok(addr)
    }

    fn stop_listener(&self) -> bool {
        match self.running.lock().unwrap().take() {
            Some(r) => {
                let _ = r.stop.send(true);
                true
            }
            None => false,
        }
    }

    /// Stops the listener and turns the opt-in off. Paired devices stay.
    pub fn stop(&self) {
        self.stop_listener();
        self.cancel_pairings();
        self.tickets.lock().unwrap().clear();
        self.cfg.lock().unwrap().enabled = false;
        self.save_config();
        self.notify("stopped");
    }

    pub fn set_tailscale_serve(&self, url: Option<String>, https_port: Option<u16>) {
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.tailscale_serve_url = url;
            if let Some(p) = https_port {
                cfg.tailscale_https_port = p;
            }
        }
        self.save_config();
        self.notify("tailscale");
    }

    fn connection_delta(&self, device_id: &str, up: bool) {
        {
            let mut c = self.connections.lock().unwrap();
            let e = c.entry(device_id.to_string()).or_insert(0);
            if up {
                *e += 1;
            } else {
                *e = e.saturating_sub(1);
            }
        }
        self.notify("connection");
    }
}

fn guess_device_name(ua: Option<&str>) -> String {
    let ua = ua.unwrap_or("");
    let name = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Macintosh") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Linux") {
        "Linux"
    } else if ua.starts_with("curl/") {
        "curl"
    } else {
        "Device"
    };
    name.to_string()
}

pub fn render_qr(content: &str) -> Result<String, String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(content.as_bytes()).map_err(|e| format!("REMOTE_QR: {e}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build())
}

/// `http(s)://host[:port]` with no path or trailing slash.
pub fn normalize_base(base: &str) -> Result<String, String> {
    let u =
        url::Url::parse(base.trim()).map_err(|e| format!("REMOTE_BASE_INVALID: {base:?}: {e}"))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(format!(
            "REMOTE_BASE_INVALID: {base:?} is not an http(s) origin"
        ));
    }
    Ok(u.origin().ascii_serialization())
}

pub fn base_for(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
    }
}

pub fn classify_ip(ip: IpAddr) -> &'static str {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if v4.is_loopback() {
                "loopback"
            } else if o[0] == 100 && (64..128).contains(&o[1]) {
                "tailscale"
            } else if v4.is_private() {
                "lan"
            } else {
                "other"
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                "loopback"
            } else if v6.segments()[0] == 0xfd7a && v6.segments()[1] == 0x115c {
                "tailscale"
            } else {
                "other"
            }
        }
    }
}

/// The address the OS would use to reach `target` (a UDP "connect" sends
/// nothing). Cheap, dependency-free interface discovery.
fn local_ip_towards(target: &str) -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect(target).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

pub fn candidate_interfaces() -> Vec<InterfaceOption> {
    let mut out = vec![InterfaceOption {
        ip: Ipv4Addr::LOCALHOST.to_string(),
        kind: "loopback".into(),
    }];
    for target in [
        "192.168.1.1:9",
        "10.0.0.1:9",
        "8.8.8.8:53",
        "100.100.100.100:53",
    ] {
        if let Some(ip) = local_ip_towards(target) {
            if !out.iter().any(|o| o.ip == ip.to_string()) {
                out.push(InterfaceOption {
                    ip: ip.to_string(),
                    kind: classify_ip(ip).into(),
                });
            }
        }
    }
    out
}

fn bind_one(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    TcpListener::from_std(socket.into())
}

fn bind_remote(ip: IpAddr, port: u16) -> std::io::Result<(SocketAddr, TcpListener)> {
    let mut last = None;
    let first = if port == 0 { DEFAULT_PORT } else { port };
    for p in (first..first.saturating_add(10)).chain(std::iter::once(0)) {
        match bind_one(SocketAddr::new(ip, p)) {
            Ok(l) => {
                let addr = l.local_addr()?;
                return Ok((addr, l));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no port")))
}

// ── Accept loop: WebSocket to tungstenite, the rest to axum ─────────────

#[derive(Clone)]
struct Ctx {
    hub: Arc<RemoteHub>,
    backend: Arc<dyn RemoteBackend>,
    stop: watch::Receiver<bool>,
}

struct SplitListener {
    rx: mpsc::Receiver<(TcpStream, SocketAddr)>,
    local: SocketAddr,
}

impl Listener for SplitListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.rx.recv().await {
            Some(pair) => pair,
            // The accept loop is gone (stopping): graceful shutdown ends us.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

#[derive(Clone, Copy, Debug)]
struct Peer(SocketAddr);

impl axum::extract::connect_info::Connected<IncomingStream<'_, SplitListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, SplitListener>) -> Self {
        Peer(*stream.remote_addr())
    }
}

async fn route_connection(
    stream: TcpStream,
    peer: SocketAddr,
    tx: mpsc::Sender<(TcpStream, SocketAddr)>,
    ctx: Ctx,
) {
    const WS_PREFIX: &[u8] = b"GET /remote/ws";
    let mut buf = [0u8; 16];
    let mut is_ws = false;
    for _ in 0..100 {
        match tokio::time::timeout(Duration::from_secs(10), stream.peek(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
            Ok(Ok(n)) => {
                if n >= WS_PREFIX.len() || buf[..n].contains(&b'\n') {
                    is_ws = buf[..n].starts_with(WS_PREFIX);
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if is_ws {
        serve_ws(stream, peer, ctx).await;
    } else {
        let _ = tx.send((stream, peer)).await;
    }
}

fn query_param(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

struct WsAuth {
    device: DeviceRecord,
    after: Option<i64>,
}

async fn serve_ws(stream: TcpStream, peer: SocketAddr, ctx: Ctx) {
    let ip = peer.ip().to_string();
    let slot: Arc<Mutex<Option<WsAuth>>> = Arc::new(Mutex::new(None));
    let path_seen: Arc<Mutex<String>> = Arc::new(Mutex::new(String::from("/remote/ws")));
    let cb = {
        let hub = ctx.hub.clone();
        let slot = slot.clone();
        let path_seen = path_seen.clone();
        move |req: &tungstenite::handshake::server::Request,
              resp: tungstenite::handshake::server::Response|
              -> Result<
            tungstenite::handshake::server::Response,
            tungstenite::handshake::server::ErrorResponse,
        > {
            let uri = req.uri();
            *path_seen.lock().unwrap() = uri.path().to_string();
            let q = uri.query().unwrap_or("");
            let device = (uri.path() == "/remote/ws")
                .then(|| query_param(q, "ticket"))
                .flatten()
                .and_then(|t| hub.redeem_ticket(&t));
            match device {
                Some(device) => {
                    let after = query_param(q, "after").and_then(|a| a.parse().ok());
                    *slot.lock().unwrap() = Some(WsAuth { device, after });
                    Ok(resp)
                }
                None => {
                    let mut r = tungstenite::handshake::server::ErrorResponse::new(Some(
                        "{\"error\":\"unauthorized\"}".to_string(),
                    ));
                    *r.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED;
                    r.headers_mut().insert(
                        "content-type",
                        tungstenite::http::HeaderValue::from_static("application/json"),
                    );
                    Err(r)
                }
            }
        }
    };
    let handshake = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::accept_hdr_async(stream, cb),
    )
    .await;
    let auth = slot.lock().unwrap().take();
    let path = path_seen.lock().unwrap().clone();
    let ws = match (handshake, auth) {
        (Ok(Ok(ws)), Some(auth)) => {
            ctx.hub.record(AccessEntry {
                at: now_iso(),
                ip: ip.clone(),
                device_id: Some(auth.device.id.clone()),
                device: Some(auth.device.name.clone()),
                method: "WS".into(),
                path,
                status: 101,
            });
            (ws, auth)
        }
        _ => {
            ctx.hub.record(AccessEntry {
                at: now_iso(),
                ip,
                device_id: None,
                device: None,
                method: "WS".into(),
                path,
                status: 401,
            });
            return;
        }
    };
    let (ws, auth) = ws;
    let device_id = auth.device.id.clone();
    ctx.hub.connection_delta(&device_id, true);
    if let Err(e) = pump_events(ws, auth, &ctx).await {
        tracing::debug!("[remote] socket of {device_id} closed: {e}");
    }
    ctx.hub.connection_delta(&device_id, false);
}

type WsStream = tokio_tungstenite::WebSocketStream<TcpStream>;

async fn send_json(
    sink: &mut futures::stream::SplitSink<WsStream, Message>,
    v: &Value,
) -> Result<(), String> {
    sink.send(Message::text(v.to_string()))
        .await
        .map_err(|e| e.to_string())
}

/// Replays `(last, head]` into the socket. Returns the new `last`.
async fn catch_up(
    engine: &Arc<ThreadsEngine>,
    sink: &mut futures::stream::SplitSink<WsStream, Message>,
    mut last: i64,
) -> Result<i64, String> {
    loop {
        let page = engine.events_after(last, 500).await?;
        if page.reset {
            send_json(sink, &json!({ "type": "reset", "head": page.head })).await?;
            return Ok(page.head);
        }
        for ev in &page.events {
            send_json(
                sink,
                &json!({ "type": "event", "sequence": ev.sequence, "event": ev }),
            )
            .await?;
            last = ev.sequence;
        }
        if !page.has_more || page.events.is_empty() {
            return Ok(last);
        }
    }
}

async fn pump_events(ws: WsStream, auth: WsAuth, ctx: &Ctx) -> Result<(), String> {
    let engine = ctx.backend.engine().await?;
    // Subscribe before the replay so nothing falls between the two.
    let mut live = engine.subscribe();
    let mut revoked = ctx.hub.revoked.subscribe();
    let mut stop = ctx.stop.clone();
    let (mut sink, mut source) = ws.split();
    let head = engine.events_after(i64::MAX, 1).await?.head;
    send_json(
        &mut sink,
        &json!({
            "type": "hello",
            "head": head,
            "device": { "id": auth.device.id, "name": auth.device.name, "scope": auth.device.scope },
        }),
    )
    .await?;
    let mut last = match auth.after {
        Some(after) => catch_up(&engine, &mut sink, after).await?,
        None => head,
    };
    let mut ping = tokio::time::interval(Duration::from_secs(WS_PING_SECS));
    ping.tick().await;
    loop {
        tokio::select! {
            ev = live.recv() => match ev {
                Ok(ev) => {
                    if ev.sequence > last {
                        last = ev.sequence;
                        send_json(&mut sink, &json!({ "type": "event", "sequence": ev.sequence, "event": ev })).await?;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    last = catch_up(&engine, &mut sink, last).await?;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            msg = source.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
                    if v.get("type").and_then(Value::as_str) == Some("ping") {
                        send_json(&mut sink, &json!({ "type": "pong", "head": last })).await?;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            id = revoked.recv() => {
                if matches!(id, Ok(ref i) if *i == auth.device.id) {
                    let _ = send_json(&mut sink, &json!({ "type": "revoked" })).await;
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
            _ = stop.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                break;
            },
            _ = ping.tick() => {
                sink.send(Message::Ping(Vec::new().into())).await.map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

// ── HTTP routes ──────────────────────────────────────────────────────────

/// Device name put on a response so the access log can show it.
#[derive(Clone)]
struct LoggedDevice(String, String);

fn fail(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": code, "message": message.into() })),
    )
        .into_response()
}

fn unauthorized() -> Response {
    fail(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "missing or invalid device token",
    )
}

fn bearer_of(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let t = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?
        .trim();
    (!t.is_empty()).then_some(t)
}

fn authed(ctx: &Ctx, headers: &HeaderMap, peer: &Peer) -> Result<DeviceRecord, Response> {
    let token = bearer_of(headers).ok_or_else(unauthorized)?;
    ctx.hub
        .authenticate(token, &peer.0.ip().to_string())
        .ok_or_else(unauthorized)
}

fn tagged(mut r: Response, d: &DeviceRecord) -> Response {
    r.extensions_mut()
        .insert(LoggedDevice(d.id.clone(), d.name.clone()));
    r
}

fn json_result<T: Serialize>(r: Result<T, String>, d: &DeviceRecord) -> Response {
    let resp = match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let code = e.split(':').next().unwrap_or("ERR").trim().to_string();
            let status = if code.ends_with("NOT_FOUND") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            fail(status, &code, e)
        }
    };
    tagged(resp, d)
}

fn http_router(ctx: Ctx) -> Router {
    Router::new()
        .route("/remote", get(|| async { Redirect::permanent("/remote/") }))
        .route("/remote/", get(|| async { static_file("index.html") }))
        .route(
            "/remote/{file}",
            get(|Path(f): Path<String>| async move { static_file(&f) }),
        )
        .route("/remote/api/pair", post(api_pair))
        .route("/remote/api/session", get(api_session))
        .route("/remote/api/logout", post(api_logout))
        .route("/remote/api/ws-ticket", post(api_ws_ticket))
        .route("/remote/api/snapshot", get(api_snapshot))
        .route("/remote/api/events", get(api_events))
        .route("/remote/api/threads/{id}/turns", get(api_turns))
        .route("/remote/api/threads/{id}/diff", get(api_diff))
        .route("/remote/api/dispatch", post(api_dispatch))
        .route("/remote/api/sessions", get(api_sessions))
        .route("/remote/api/sessions/{tool}/{id}", get(api_session_get))
        .fallback(|| async { unauthorized() })
        .layer(middleware::from_fn_with_state(ctx.clone(), access_log))
        .with_state(ctx)
}

async fn access_log(State(ctx): State<Ctx>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let ip = req
        .extensions()
        .get::<ConnectInfo<Peer>>()
        .map(|c| c.0 .0.ip().to_string())
        .unwrap_or_default();
    let mut resp = next.run(req).await;
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
    ] {
        resp.headers_mut().insert(k, HeaderValue::from_static(v));
    }
    let is_shell = method == Method::GET
        && (path == "/remote"
            || path == "/remote/"
            || PWA_FILES
                .iter()
                .any(|(n, _, _)| path == format!("/remote/{n}")));
    if !is_shell {
        let dev = resp.extensions().get::<LoggedDevice>().cloned();
        ctx.hub.record(AccessEntry {
            at: now_iso(),
            ip,
            device_id: dev.as_ref().map(|d| d.0.clone()),
            device: dev.map(|d| d.1),
            method: method.to_string(),
            path,
            status: resp.status().as_u16(),
        });
    }
    resp
}

/// Query string as text pairs (the host's axum has no `query` feature).
struct Qs(HashMap<String, String>);

impl Qs {
    fn text(&self, k: &str) -> Option<String> {
        self.0
            .get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }
    fn num<T: std::str::FromStr>(&self, k: &str) -> Option<T> {
        self.0.get(k).and_then(|v| v.trim().parse().ok())
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Qs {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        let q = parts.uri.query().unwrap_or("");
        Ok(Qs(url::form_urlencoded::parse(q.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()))
    }
}

fn static_file(name: &str) -> Response {
    let Some((_, mime, body)) = PWA_FILES.iter().find(|(n, _, _)| *n == name) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mut r = (*body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    if name == "sw.js" {
        h.insert(
            "service-worker-allowed",
            HeaderValue::from_static("/remote/"),
        );
    }
    r
}

#[derive(Deserialize)]
struct PairBody {
    secret: String,
    #[serde(default)]
    name: Option<String>,
}

async fn api_pair(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Json(body): Json<PairBody>,
) -> Response {
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    match ctx
        .hub
        .redeem_pairing(&body.secret, body.name, &peer.0.ip().to_string(), ua)
    {
        Ok((token, device)) => {
            let mut r = Json(json!({ "token": token, "device": device })).into_response();
            r.extensions_mut()
                .insert(LoggedDevice(device.id.clone(), device.name.clone()));
            r
        }
        Err((status, msg)) => fail(status, "pair_failed", msg),
    }
}

async fn api_session(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_default();
    let view = ctx.hub.view_of(&d);
    tagged(
        Json(json!({
            "device": view,
            "scope": d.scope,
            "version": env!("CARGO_PKG_VERSION"),
            "host": host,
        }))
        .into_response(),
        &d,
    )
}

async fn api_logout(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    ctx.hub.revoke(&d.id);
    tagged(Json(json!({ "ok": true })).into_response(), &d)
}

async fn api_ws_ticket(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let (ticket, exp) = ctx.hub.mint_ticket(&d.id);
    tagged(
        Json(json!({ "ticket": ticket, "expiresAt": iso_of_ms(exp) })).into_response(),
        &d,
    )
}

async fn api_snapshot(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = match ctx.backend.engine().await {
        Ok(e) => e.snapshot().await,
        Err(e) => Err(e),
    };
    json_result(r, &d)
}

async fn api_events(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    q: Qs,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = match ctx.backend.engine().await {
        Ok(e) => {
            e.events_after(q.num("after").unwrap_or(0), q.num("limit").unwrap_or(500))
                .await
        }
        Err(e) => Err(e),
    };
    json_result(r, &d)
}

async fn api_turns(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
    q: Qs,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = match ctx.backend.engine().await {
        Ok(e) => {
            e.turns_page(
                id,
                q.num("before"),
                q.num("limit").unwrap_or(10u32).clamp(1, 50),
            )
            .await
        }
        Err(e) => Err(e),
    };
    json_result(r, &d)
}

async fn api_diff(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path(id): Path<String>,
    q: Qs,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    json_result(ctx.backend.diff(&id, q.num("turn")).await, &d)
}

async fn api_dispatch(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let env: CommandEnvelope = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return tagged(
                fail(
                    StatusCode::BAD_REQUEST,
                    "ERR_THREADS_INVALID",
                    e.to_string(),
                ),
                &d,
            )
        }
    };
    if let Err(why) = command_allowed(d.scope, &env.command) {
        return tagged(fail(StatusCode::FORBIDDEN, "insufficient_scope", why), &d);
    }
    let r = match ctx.backend.engine().await {
        Ok(e) => e.dispatch(env).await,
        Err(e) => Err(e),
    };
    json_result(r, &d)
}

async fn api_sessions(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    q: Qs,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let filter = omniget_core::core::sessions::index::ListFilter {
        tool: q.text("tool"),
        query: q.text("query"),
        project: q.text("project"),
        limit: Some(q.num("limit").unwrap_or(50u32).clamp(1, 200)),
        offset: q.num("offset"),
        ..Default::default()
    };
    json_result(omniget_core::core::sessions::api::list(filter).await, &d)
}

async fn api_session_get(
    State(ctx): State<Ctx>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    headers: HeaderMap,
    Path((tool, id)): Path<(String, String)>,
    q: Qs,
) -> Response {
    let d = match authed(&ctx, &headers, &peer) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let r = omniget_core::core::sessions::api::get(
        tool,
        id,
        Some(q.num("page").unwrap_or(0)),
        Some(q.num("pageSize").unwrap_or(40u32).clamp(1, 200)),
    )
    .await;
    json_result(r, &d)
}

// ── App glue (called from `local_bridge::spawn` and the commands) ───────

/// Brings the remote listener back when the user left it on. Call once at
/// startup (the local bridge's `spawn` is the natural place).
pub async fn boot(app: AppHandle) {
    let h = hub();
    h.set_app(&app);
    let cfg = h.config();
    if !cfg.enabled {
        return;
    }
    let backend: Arc<dyn RemoteBackend> = Arc::new(AppBackend(app));
    if let Err(e) = h.start(backend, &cfg.bind_ip, cfg.port).await {
        tracing::warn!("[remote] could not restore the remote listener: {e}");
    }
}

pub async fn start_for_app(
    app: &AppHandle,
    bind_ip: &str,
    port: Option<u16>,
) -> Result<RemoteStatus, String> {
    let h = hub();
    h.set_app(app);
    let port = port.unwrap_or_else(|| h.config().port);
    let backend: Arc<dyn RemoteBackend> = Arc::new(AppBackend(app.clone()));
    h.start(backend, bind_ip, port).await?;
    Ok(h.status())
}

// ── Tailscale and SSH helpers ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleInfo {
    pub installed: bool,
    pub binary: Option<String>,
    pub running: bool,
    pub backend_state: Option<String>,
    pub dns_name: Option<String>,
    pub ips: Vec<String>,
    pub https_port: u16,
    /// Target the serve proxies to (the remote listener).
    pub target: Option<String>,
    pub serve_args: Vec<String>,
    pub serve_command: Option<String>,
    pub serve_off_args: Vec<String>,
    pub serve_off_command: Option<String>,
    pub serve_url: Option<String>,
    pub serve_enabled: bool,
    pub error: Option<String>,
}

pub fn tailscale_binary() -> Option<PathBuf> {
    if let Ok(p) = which::which("tailscale") {
        return Some(p);
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/Applications/Tailscale.app/Contents/MacOS/Tailscale"]
    } else if cfg!(windows) {
        &["C:\\Program Files\\Tailscale\\tailscale.exe"]
    } else {
        &["/usr/bin/tailscale", "/usr/local/bin/tailscale"]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

fn shell_join(bin: &str, args: &[String]) -> String {
    let q = |s: &str| {
        if s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c))
        {
            s.to_string()
        } else {
            format!("'{}'", s.replace('\'', "'\\''"))
        }
    };
    std::iter::once(q(bin))
        .chain(args.iter().map(|a| q(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn run_tool(bin: &FsPath, args: &[String]) -> Result<(bool, String, String), String> {
    let mut cmd = omniget_core::core::process::command(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| "REMOTE_TAILSCALE_TIMEOUT: tailscale did not answer in 20 s".to_string())?
        .map_err(|e| format!("REMOTE_TAILSCALE_SPAWN: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn serve_target(h: &RemoteHub) -> Option<String> {
    h.bound_addr().map(|a| base_for(a.ip(), a.port()))
}

fn serve_args(https_port: u16, target: &str) -> Vec<String> {
    vec![
        "serve".into(),
        "--bg".into(),
        format!("--https={https_port}"),
        target.to_string(),
    ]
}

fn serve_off_args(https_port: u16) -> Vec<String> {
    vec![
        "serve".into(),
        format!("--https={https_port}"),
        "off".into(),
    ]
}

fn serve_url_for(dns: &str, https_port: u16) -> String {
    if https_port == 443 {
        format!("https://{dns}")
    } else {
        format!("https://{dns}:{https_port}")
    }
}

/// Reads `tailscale status --json` (only when the settings page asks).
pub async fn tailscale_status(https_port: Option<u16>) -> TailscaleInfo {
    let h = hub();
    let cfg = h.config();
    let https_port = https_port.unwrap_or(cfg.tailscale_https_port);
    let target = serve_target(&h);
    let mut info = TailscaleInfo {
        installed: false,
        binary: None,
        running: false,
        backend_state: None,
        dns_name: None,
        ips: Vec::new(),
        https_port,
        target: target.clone(),
        serve_args: Vec::new(),
        serve_command: None,
        serve_off_args: serve_off_args(https_port),
        serve_off_command: None,
        serve_url: None,
        serve_enabled: cfg.tailscale_serve_url.is_some(),
        error: None,
    };
    let Some(bin) = tailscale_binary() else {
        return info;
    };
    let bin_s = bin.to_string_lossy().into_owned();
    info.installed = true;
    info.binary = Some(bin_s.clone());
    info.serve_off_command = Some(shell_join(&bin_s, &info.serve_off_args));
    if let Some(t) = &target {
        info.serve_args = serve_args(https_port, t);
        info.serve_command = Some(shell_join(&bin_s, &info.serve_args));
    }
    match run_tool(&bin, &["status".into(), "--json".into()]).await {
        Ok((_, stdout, stderr)) => match serde_json::from_str::<Value>(&stdout) {
            Ok(v) => {
                let state = v
                    .get("BackendState")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                info.running = state.as_deref() == Some("Running");
                info.backend_state = state;
                let me = v.get("Self");
                info.dns_name = me
                    .and_then(|s| s.get("DNSName"))
                    .and_then(Value::as_str)
                    .map(|s| s.trim_end_matches('.').to_string())
                    .filter(|s| !s.is_empty());
                info.ips = me
                    .and_then(|s| s.get("TailscaleIPs"))
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                info.serve_url = info
                    .dns_name
                    .as_deref()
                    .map(|d| serve_url_for(d, https_port));
            }
            Err(_) => {
                info.error = Some(
                    format!("{} {}", stderr.trim(), stdout.trim())
                        .trim()
                        .chars()
                        .take(400)
                        .collect(),
                )
            }
        },
        Err(e) => info.error = Some(e),
    }
    info
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub ok: bool,
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub serve_url: Option<String>,
}

/// Turns `tailscale serve` of the remote port on or off. Refuses to run
/// without `confirm: true` (the page shows the command first).
pub async fn tailscale_serve(
    enable: bool,
    confirm: bool,
    https_port: Option<u16>,
) -> Result<CommandRun, String> {
    let info = tailscale_status(https_port).await;
    let bin = info
        .binary
        .clone()
        .ok_or_else(|| "REMOTE_TAILSCALE_MISSING: tailscale is not installed".to_string())?;
    let args = if enable {
        if info.target.is_none() {
            return Err("REMOTE_NOT_RUNNING: turn remote access on first".into());
        }
        info.serve_args.clone()
    } else {
        info.serve_off_args.clone()
    };
    let command = shell_join(&bin, &args);
    if !confirm {
        return Err(format!("REMOTE_CONFIRM_REQUIRED: {command}"));
    }
    let (ok, stdout, stderr) = run_tool(FsPath::new(&bin), &args).await?;
    let h = hub();
    let serve_url = if ok && enable {
        info.serve_url.clone()
    } else {
        None
    };
    if ok {
        h.set_tailscale_serve(serve_url.clone(), Some(info.https_port));
    }
    Ok(CommandRun {
        ok,
        command,
        stdout: stdout.chars().take(4000).collect(),
        stderr: stderr.chars().take(4000).collect(),
        serve_url,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshInfo {
    pub command: String,
    pub user: String,
    pub host: String,
    pub local_port: u16,
    pub remote_target: String,
    /// Base to pair with once the tunnel is up (on the other machine).
    pub base: String,
    pub running: bool,
}

/// `ssh -N -L <local>:<bound ip>:<port> user@host` for whoever prefers a
/// tunnel; the phone/laptop then opens `http://127.0.0.1:<local>/remote/`.
pub fn ssh_info(local_port: Option<u16>, host: Option<String>) -> SshInfo {
    let h = hub();
    let bound = h.bound_addr();
    let cfg = h.config();
    let port = bound.map(|a| a.port()).unwrap_or(cfg.port);
    let target_ip = bound
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|| cfg.bind_ip.clone());
    let target_ip = if target_ip.contains(':') {
        format!("[{target_ip}]")
    } else {
        target_ip
    };
    let local_port = local_port.unwrap_or(port);
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    let host = host.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
        candidate_interfaces()
            .into_iter()
            .find(|i| i.kind == "lan")
            .or_else(|| {
                candidate_interfaces()
                    .into_iter()
                    .find(|i| i.kind == "tailscale")
            })
            .map(|i| i.ip)
            .unwrap_or_else(|| "this-machine".into())
    });
    SshInfo {
        command: format!("ssh -N -L {local_port}:{target_ip}:{port} {user}@{host}"),
        user,
        host,
        local_port,
        remote_target: format!("{target_ip}:{port}"),
        base: format!("http://127.0.0.1:{local_port}"),
        running: bound.is_some(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_hub() -> (Arc<RemoteHub>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("omniget-remote-test-{}", random_hex(6)));
        (RemoteHub::new(Some(dir.clone())), dir)
    }

    #[test]
    fn scope_rules() {
        let send: Command = serde_json::from_value(
            json!({"type": "thread.turn.start", "threadId": "t", "text": "hi"}),
        )
        .unwrap();
        let del: Command =
            serde_json::from_value(json!({"type": "thread.delete", "threadId": "t"})).unwrap();
        let always: Command = serde_json::from_value(json!({
            "type": "thread.approval.respond", "threadId": "t", "requestId": "r", "decision": "acceptAlways"
        }))
        .unwrap();
        let accept: Command = serde_json::from_value(json!({
            "type": "thread.approval.respond", "threadId": "t", "requestId": "r", "decision": "accept"
        }))
        .unwrap();
        assert!(command_allowed(Scope::Read, &send).is_err());
        assert!(command_allowed(Scope::Drive, &send).is_ok());
        assert!(command_allowed(Scope::Drive, &accept).is_ok());
        assert!(command_allowed(Scope::Drive, &always).is_err());
        assert!(command_allowed(Scope::Drive, &del).is_err());
    }

    #[test]
    fn pairing_is_single_use_and_scoped_server_side() {
        let (h, dir) = tmp_hub();
        let link = h
            .create_pairing(Scope::Read, "http://192.168.0.5:47740/", None)
            .unwrap();
        assert!(link
            .url
            .starts_with("http://192.168.0.5:47740/remote/#pair="));
        assert!(link.qr_svg.contains("<svg"));
        let secret = link
            .url
            .split("#pair=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();
        let (token, dev) = h
            .redeem_pairing(&secret, Some("Phone".into()), "1.2.3.4", None)
            .unwrap();
        assert_eq!(dev.scope, Scope::Read);
        assert!(h.redeem_pairing(&secret, None, "1.2.3.4", None).is_err());
        assert!(h.authenticate(&token, "1.2.3.4").is_some());
        assert!(h.authenticate("nope", "1.2.3.4").is_none());
        // Only the hash is on disk.
        let disk = std::fs::read_to_string(dir.join("devices.json")).unwrap();
        assert!(!disk.contains(&token));
        assert!(disk.contains(&hash_secret(&token)));
        // Reload keeps the device; revoke drops it.
        let again = RemoteHub::new(Some(dir.clone()));
        assert!(again.authenticate(&token, "x").is_some());
        assert!(again.revoke(&dev.id));
        assert!(again.authenticate(&token, "x").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tickets_are_single_use() {
        let (h, dir) = tmp_hub();
        let link = h
            .create_pairing(Scope::Drive, "http://127.0.0.1:1", None)
            .unwrap();
        let secret = link
            .url
            .split("#pair=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let (_, dev) = h.redeem_pairing(secret, None, "ip", None).unwrap();
        let (t, _) = h.mint_ticket(&dev.id);
        assert_eq!(h.redeem_ticket(&t).map(|d| d.id), Some(dev.id.clone()));
        assert!(h.redeem_ticket(&t).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_pairings_lock_out() {
        let (h, dir) = tmp_hub();
        for _ in 0..PAIR_FAIL_MAX {
            assert_eq!(
                h.redeem_pairing("bad", None, "ip", None).unwrap_err().0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            h.redeem_pairing("bad", None, "ip", None).unwrap_err().0,
            StatusCode::TOO_MANY_REQUESTS
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ip_classes_and_bases() {
        assert_eq!(classify_ip("100.101.2.3".parse().unwrap()), "tailscale");
        assert_eq!(classify_ip("192.168.1.9".parse().unwrap()), "lan");
        assert_eq!(classify_ip("127.0.0.1".parse().unwrap()), "loopback");
        assert_eq!(
            normalize_base("https://box.tail1.ts.net:8443/remote/").unwrap(),
            "https://box.tail1.ts.net:8443"
        );
        assert!(normalize_base("ftp://x").is_err());
        assert_eq!(serve_url_for("box.ts.net", 443), "https://box.ts.net");
    }

    struct EngineBackend(Arc<ThreadsEngine>);

    #[async_trait::async_trait]
    impl RemoteBackend for EngineBackend {
        async fn engine(&self) -> Result<Arc<ThreadsEngine>, String> {
            Ok(self.0.clone())
        }
        async fn diff(&self, _t: &str, _turn: Option<u32>) -> Result<Value, String> {
            Err("ERR_UNSUPPORTED: no git in the harness".into())
        }
    }

    /// Live harness for testing with curl: starts the remote listener over a
    /// temp threads.db with one thread, mints a drive pairing and writes
    /// `{base, secret, threadId}` to `$OMNIGET_REMOTE_HARNESS_OUT`, then
    /// renames the thread every 3 s (events for the socket) for
    /// `$OMNIGET_REMOTE_HARNESS_SECS` (default 90) seconds.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn remote_live_harness() {
        let dir = std::env::temp_dir().join(format!("omniget-remote-live-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = ThreadsEngine::open(&dir.join("threads.db")).unwrap();
        let project = engine
            .dispatch(
                serde_json::from_value(json!({
                    "type": "project.create", "projectId": "prj_remote", "title": "Remote test",
                    "workspaceRoot": dir.to_string_lossy()
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(project.sequence > 0);
        engine
            .dispatch(
                serde_json::from_value(json!({
                    "type": "thread.create", "threadId": "thr_remote", "projectId": "prj_remote",
                    "title": "Hello from the harness", "instanceId": "native", "driver": "native"
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        let h = RemoteHub::new(Some(dir.join("remote")));
        let addr = h
            .start(Arc::new(EngineBackend(engine.clone())), "127.0.0.1", 0)
            .await
            .unwrap();
        let base = base_for(addr.ip(), addr.port());
        let link = h
            .create_pairing(Scope::Drive, &base, Some("curl".into()))
            .unwrap();
        let secret = link
            .url
            .split("#pair=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let out =
            json!({ "base": base, "secret": secret, "threadId": "thr_remote", "url": link.url });
        if let Ok(p) = std::env::var("OMNIGET_REMOTE_HARNESS_OUT") {
            std::fs::write(p, out.to_string()).unwrap();
        }
        println!("{out}");
        let secs: u64 = std::env::var("OMNIGET_REMOTE_HARNESS_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(90);
        for i in 0..(secs / 3) {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let _ = engine
                .dispatch(
                    serde_json::from_value(json!({
                        "type": "thread.rename", "threadId": "thr_remote", "title": format!("tick {i}")
                    }))
                    .unwrap(),
                )
                .await;
        }
        for e in h.access_log(50).iter().rev() {
            println!(
                "{} {} {} {} {:?}",
                e.ip, e.method, e.path, e.status, e.device
            );
        }
        h.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
