//! Chromecast, written from scratch: mDNS discovery over plain UDP, the
//! CASTV2 channel over TLS with a hand-rolled `CastMessage` protobuf, and a
//! LAN listener that exists only while a cast session runs.
//!
//! Flow: discover (`_googlecast._tcp.local` PTR, 3 s) → TLS to :8009 →
//! CONNECT + heartbeat → LAUNCH the Default Media Receiver (CC1AD845) →
//! CONNECT to its transport → LOAD. The TV can't reach 127.0.0.1, so the
//! local player server is mirrored on the LAN address that routes to the
//! device, under a per-session token, with `Access-Control-Allow-Origin: *`
//! (the receiver fetches WebVTT and HLS by XHR). HLS playlists are rewritten
//! on the way so their absolute 127.0.0.1 URLs point at the mirror too.
//!
//! Reference behaviour: pychromecast `socket_client.py` and catt
//! `get_local_ip` (see estudos/93-watch/re/lote-5).

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Path, RawQuery, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::rustls;
use tower_http::cors::{Any, CorsLayer};

use super::{server, sources, PlayMode, PlaybackInfo, SourceRef};

const NS_CONN: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HB: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECV: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
const PLATFORM: &str = "receiver-0";
const SENDER: &str = "sender-0";
const DEFAULT_RECEIVER: &str = "CC1AD845";
const EVENT: &str = "video-cast-status";
const MAX_FRAME: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const LOAD_TIMEOUT: Duration = Duration::from_secs(25);

// =====================================================================
// CastMessage protobuf (cast_channel.proto, fields 1-7, proto2)
// =====================================================================

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CastMessage {
    pub source_id: String,
    pub destination_id: String,
    pub namespace: String,
    /// `payload_utf8` (field 6). Binary payloads (device auth) are not used.
    pub payload: String,
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn get_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn put_str(out: &mut Vec<u8>, field: u64, s: &str) {
    put_varint(out, (field << 3) | 2);
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

impl CastMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.payload.len() + 96);
        put_varint(&mut out, 1 << 3); // protocol_version = CASTV2_1_0
        put_varint(&mut out, 0);
        put_str(&mut out, 2, &self.source_id);
        put_str(&mut out, 3, &self.destination_id);
        put_str(&mut out, 4, &self.namespace);
        put_varint(&mut out, 5 << 3); // payload_type = STRING
        put_varint(&mut out, 0);
        put_str(&mut out, 6, &self.payload);
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut m = CastMessage::default();
        let mut pos = 0;
        while pos < buf.len() {
            let key = get_varint(buf, &mut pos)?;
            let (field, wire) = (key >> 3, key & 7);
            match wire {
                0 => {
                    get_varint(buf, &mut pos)?;
                }
                1 => pos = pos.checked_add(8)?,
                2 => {
                    let len = get_varint(buf, &mut pos)? as usize;
                    let end = pos.checked_add(len)?;
                    let bytes = buf.get(pos..end)?;
                    pos = end;
                    let text = || String::from_utf8_lossy(bytes).into_owned();
                    match field {
                        2 => m.source_id = text(),
                        3 => m.destination_id = text(),
                        4 => m.namespace = text(),
                        6 => m.payload = text(),
                        _ => {}
                    }
                }
                5 => pos = pos.checked_add(4)?,
                _ => return None,
            }
        }
        (pos == buf.len()).then_some(m)
    }

    /// Length-prefixed frame as it goes on the wire.
    fn frame(&self) -> Vec<u8> {
        let body = self.encode();
        let mut out = Vec::with_capacity(body.len() + 4);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }
}

// =====================================================================
// mDNS discovery
// =====================================================================

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub host: String,
    pub port: u16,
    /// Always "chromecast" for now.
    pub kind: String,
}

const SERVICE: &str = "_googlecast._tcp.local";
const MDNS_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

fn put_name(out: &mut Vec<u8>, name: &str) {
    for label in name.split('.').filter(|l| !l.is_empty()) {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
}

/// One PTR question for the Cast service.
fn mdns_query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    put_name(&mut q, SERVICE);
    q.extend_from_slice(&[0, 12, 0, 1]); // PTR, IN
    q
}

/// Reads a (possibly compressed) name; returns it and the offset after it.
fn read_name(buf: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = start;
    let mut end = None;
    for _ in 0..128 {
        let len = *buf.get(pos)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(pos + 1)));
        }
        if len & 0xc0 == 0xc0 {
            let ptr = ((len & 0x3f) << 8) | *buf.get(pos + 1)? as usize;
            end.get_or_insert(pos + 2);
            pos = ptr;
            continue;
        }
        let label = buf.get(pos + 1..pos + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        pos += 1 + len;
    }
    None
}

#[derive(Default, Debug)]
struct MdnsRecords {
    ptr: Vec<String>,
    srv: HashMap<String, (u16, String)>,
    txt: HashMap<String, HashMap<String, String>>,
    a: HashMap<String, Ipv4Addr>,
}

fn norm(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// Collects PTR/SRV/TXT/A records of one packet into `rec`.
fn parse_mdns(buf: &[u8], rec: &mut MdnsRecords) -> Option<()> {
    if buf.len() < 12 {
        return None;
    }
    let count = |i: usize| u16::from_be_bytes([buf[i], buf[i + 1]]) as usize;
    let (qd, an, ns, ar) = (count(4), count(6), count(8), count(10));
    let mut pos = 12;
    for _ in 0..qd {
        let (_, p) = read_name(buf, pos)?;
        pos = p + 4;
    }
    for _ in 0..(an + ns + ar) {
        let (name, p) = read_name(buf, pos)?;
        let hdr = buf.get(p..p + 10)?;
        let typ = u16::from_be_bytes([hdr[0], hdr[1]]);
        let rdlen = u16::from_be_bytes([hdr[8], hdr[9]]) as usize;
        let rstart = p + 10;
        let rdata = buf.get(rstart..rstart + rdlen)?;
        let name = norm(&name);
        match typ {
            12 if name == SERVICE.to_ascii_lowercase() => {
                if let Some((target, _)) = read_name(buf, rstart) {
                    rec.ptr.push(norm(&target));
                }
            }
            33 if rdlen >= 6 => {
                let port = u16::from_be_bytes([rdata[4], rdata[5]]);
                if let Some((target, _)) = read_name(buf, rstart + 6) {
                    rec.srv.insert(name, (port, norm(&target)));
                }
            }
            16 => {
                let mut kv = HashMap::new();
                let mut i = 0;
                while i < rdata.len() {
                    let l = rdata[i] as usize;
                    let Some(s) = rdata.get(i + 1..i + 1 + l) else {
                        break;
                    };
                    let s = String::from_utf8_lossy(s);
                    if let Some((k, v)) = s.split_once('=') {
                        kv.insert(k.to_ascii_lowercase(), v.to_string());
                    }
                    i += 1 + l;
                }
                rec.txt.entry(name).or_default().extend(kv);
            }
            1 if rdlen == 4 => {
                rec.a
                    .insert(name, Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3]));
            }
            _ => {}
        }
        pos = rstart + rdlen;
    }
    Some(())
}

/// Turns the records into devices. `from` is the address each instance's
/// packet came from, used when the A record is missing.
fn devices_from(rec: &MdnsRecords, from: &HashMap<String, IpAddr>) -> Vec<CastDevice> {
    let mut names: Vec<&String> = rec.ptr.iter().collect();
    for k in rec.srv.keys().chain(rec.txt.keys()) {
        if k.ends_with(&SERVICE.to_ascii_lowercase()) && !names.contains(&k) {
            names.push(k);
        }
    }
    let mut out: Vec<CastDevice> = Vec::new();
    for inst in names {
        let txt = rec.txt.get(inst).cloned().unwrap_or_default();
        let (port, target) = rec.srv.get(inst).cloned().unwrap_or((8009, String::new()));
        // Other ports are speaker groups (audio only).
        if port != 8009 {
            continue;
        }
        // `ca` bit 0 = video out; a Chromecast Audio has no screen.
        if let Some(ca) = txt.get("ca").and_then(|c| c.parse::<u64>().ok()) {
            if ca & 1 == 0 {
                continue;
            }
        }
        let host = rec
            .a
            .get(&target)
            .map(|ip| IpAddr::V4(*ip))
            .or_else(|| from.get(inst).copied());
        let Some(host) = host else { continue };
        let id = txt
            .get("id")
            .cloned()
            .unwrap_or_else(|| inst.split('.').next().unwrap_or(inst).to_string());
        if out.iter().any(|d| d.id == id) {
            continue;
        }
        out.push(CastDevice {
            name: txt
                .get("fn")
                .cloned()
                .unwrap_or_else(|| inst.split("._").next().unwrap_or(inst).to_string()),
            model: txt
                .get("md")
                .cloned()
                .unwrap_or_else(|| "Chromecast".into()),
            host: host.to_string(),
            port,
            kind: "chromecast".into(),
            id,
        });
    }
    out
}

/// A socket that joins the mDNS group on 5353, when the OS lets two
/// programs share the port (it usually does; the system responder is there).
fn passive_socket() -> Option<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).ok()?;
    s.set_reuse_address(true).ok()?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5353)).into())
        .ok()?;
    s.join_multicast_v4(&MDNS_ADDR, &Ipv4Addr::UNSPECIFIED)
        .ok()?;
    s.set_nonblocking(true).ok()?;
    Some(s.into())
}

/// Sends the query a few times and listens for `window`. Answers come back
/// as legacy unicast to the query socket and, from some devices, multicast.
async fn discover(window: Duration) -> Vec<CastDevice> {
    let query = mdns_query();
    let active = match std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[video] cast discovery socket: {e}");
            return Vec::new();
        }
    };
    let _ = active.set_multicast_ttl_v4(255);
    let _ = active.set_nonblocking(true);
    let Ok(active) = tokio::net::UdpSocket::from_std(active) else {
        return Vec::new();
    };
    let passive = passive_socket().and_then(|s| tokio::net::UdpSocket::from_std(s).ok());

    let mut rec = MdnsRecords::default();
    let mut from: HashMap<String, IpAddr> = HashMap::new();
    let deadline = tokio::time::Instant::now() + window;
    let mut next_send = tokio::time::Instant::now();
    let mut sent = 0;
    let (mut b1, mut b2) = (vec![0u8; 9000], vec![0u8; 9000]);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        if sent < 3 && now >= next_send {
            let _ = active.send_to(&query, (MDNS_ADDR, 5353)).await;
            sent += 1;
            next_send = now + Duration::from_millis(900);
        }
        let wake = if sent < 3 {
            next_send.min(deadline)
        } else {
            deadline
        };
        let got = tokio::select! {
            r = active.recv_from(&mut b1) => r.ok().map(|(n, a)| (b1[..n].to_vec(), a)),
            r = async {
                match &passive {
                    Some(p) => p.recv_from(&mut b2).await.ok().map(|(n, a)| (b2[..n].to_vec(), a)),
                    None => std::future::pending().await,
                }
            } => r,
            _ = tokio::time::sleep_until(wake) => None,
        };
        if let Some((pkt, addr)) = got {
            let mut one = MdnsRecords::default();
            if parse_mdns(&pkt, &mut one).is_some() {
                for inst in one.ptr.iter().chain(one.srv.keys()).chain(one.txt.keys()) {
                    from.entry(inst.clone()).or_insert(addr.ip());
                }
                rec.ptr.extend(one.ptr);
                rec.srv.extend(one.srv);
                for (k, v) in one.txt {
                    rec.txt.entry(k).or_default().extend(v);
                }
                rec.a.extend(one.a);
            }
        }
    }
    rec.ptr.sort();
    rec.ptr.dedup();
    let mut list = devices_from(&rec, &from);
    list.sort_by_key(|d| d.name.to_lowercase());
    list
}

fn devices() -> &'static Mutex<HashMap<String, CastDevice>> {
    static D: OnceLock<Mutex<HashMap<String, CastDevice>>> = OnceLock::new();
    D.get_or_init(Default::default)
}

// =====================================================================
// CASTV2 channel over TLS
// =====================================================================

/// The Chromecast presents a self-signed certificate; like every sender we
/// don't verify it (the receiver is the one that proves identity, and only
/// to Google's own senders).
#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

/// Something the device said that nobody was waiting for.
enum Incoming {
    Message(String, Value),
    Closed(String),
}

struct Channel {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    pending: Mutex<HashMap<u32, oneshot::Sender<Value>>>,
    next_id: AtomicU32,
    open: Mutex<HashSet<String>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    last_seen: Mutex<Instant>,
}

impl Channel {
    async fn connect(
        host: IpAddr,
        port: u16,
        events: mpsc::UnboundedSender<Incoming>,
    ) -> Result<Arc<Self>, String> {
        let tcp = tokio::time::timeout(
            Duration::from_secs(6),
            tokio::net::TcpStream::connect((host, port)),
        )
        .await
        .map_err(|_| "The TV did not answer".to_string())?
        .map_err(|_| "Can't reach the TV".to_string())?;
        let _ = tcp.set_nodelay(true);
        let connector = tokio_rustls::TlsConnector::from(tls_config()?);
        let tls = tokio::time::timeout(
            Duration::from_secs(6),
            connector.connect(rustls::pki_types::ServerName::IpAddress(host.into()), tcp),
        )
        .await
        .map_err(|_| "The TV did not answer".to_string())?
        .map_err(|e| format!("Secure connection to the TV failed: {e}"))?;
        let (mut rd, mut wr) = tokio::io::split(tls);
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let chan = Arc::new(Channel {
            tx,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
            open: Mutex::new(HashSet::new()),
            tasks: Mutex::new(Vec::new()),
            last_seen: Mutex::new(Instant::now()),
        });

        let writer = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if wr.write_all(&frame).await.is_err() || wr.flush().await.is_err() {
                    break;
                }
            }
            let _ = wr.shutdown().await;
        });

        let me = Arc::downgrade(&chan);
        let reader = tokio::spawn(async move {
            let reason = loop {
                let mut len = [0u8; 4];
                if let Err(e) = rd.read_exact(&mut len).await {
                    break format!("Connection to the TV lost ({e})");
                }
                let n = u32::from_be_bytes(len) as usize;
                if n > MAX_FRAME {
                    break "The TV sent a broken message".to_string();
                }
                let mut buf = vec![0u8; n];
                if let Err(e) = rd.read_exact(&mut buf).await {
                    break format!("Connection to the TV lost ({e})");
                }
                let Some(msg) = CastMessage::decode(&buf) else {
                    continue;
                };
                let Some(chan) = me.upgrade() else {
                    break String::new();
                };
                if let Ok(mut t) = chan.last_seen.lock() {
                    *t = Instant::now();
                }
                let v: Value = serde_json::from_str(&msg.payload).unwrap_or(Value::Null);
                if msg.namespace == NS_HB {
                    if v["type"] == "PING" {
                        chan.send_raw(&msg.source_id, NS_HB, json!({ "type": "PONG" }));
                    }
                    continue;
                }
                if let Some(id) = v["requestId"].as_u64().filter(|id| *id != 0) {
                    let waiter = chan
                        .pending
                        .lock()
                        .ok()
                        .and_then(|mut p| p.remove(&(id as u32)));
                    if let Some(w) = waiter {
                        let _ = w.send(v.clone());
                    }
                }
                let _ = events.send(Incoming::Message(msg.namespace, v));
            };
            let _ = events.send(Incoming::Closed(reason));
        });
        if let Ok(mut t) = chan.tasks.lock() {
            t.push(writer);
            t.push(reader);
        }
        chan.send_raw(PLATFORM, NS_CONN, connect_payload());
        if let Ok(mut o) = chan.open.lock() {
            o.insert(PLATFORM.into());
        }
        Ok(chan)
    }

    fn send_raw(&self, dest: &str, ns: &str, v: Value) {
        let msg = CastMessage {
            source_id: SENDER.into(),
            destination_id: dest.into(),
            namespace: ns.into(),
            payload: v.to_string(),
        };
        let _ = self.tx.send(msg.frame());
    }

    /// Sends, opening the virtual connection to `dest` first if needed.
    fn send(&self, dest: &str, ns: &str, v: Value) {
        let first = self
            .open
            .lock()
            .map(|mut o| o.insert(dest.to_string()))
            .unwrap_or(false);
        if first {
            self.send_raw(dest, NS_CONN, connect_payload());
        }
        self.send_raw(dest, ns, v);
    }

    async fn request(
        &self,
        dest: &str,
        ns: &str,
        mut v: Value,
        limit: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        v["requestId"] = json!(id);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        self.send(dest, ns, v);
        let r = tokio::time::timeout(limit, rx).await;
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
        r.map_err(|_| "The TV did not answer".to_string())?
            .map_err(|_| "Connection to the TV lost".to_string())
    }

    fn close(&self) {
        let dests: Vec<String> = self
            .open
            .lock()
            .map(|o| o.iter().cloned().collect())
            .unwrap_or_default();
        for d in dests {
            self.send_raw(&d, NS_CONN, json!({ "type": "CLOSE" }));
        }
        let tasks = self
            .tasks
            .lock()
            .map(|mut t| std::mem::take(&mut *t))
            .unwrap_or_default();
        // Give the writer a moment to flush the CLOSE frames.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            for t in tasks {
                t.abort();
            }
        });
    }
}

fn connect_payload() -> Value {
    json!({
        "type": "CONNECT",
        "origin": {},
        "userAgent": "OmniGet",
        "senderInfo": {
            "sdkType": 2,
            "version": env!("CARGO_PKG_VERSION"),
            "browserVersion": "OmniGet",
            "platform": 4,
            "systemVersion": std::env::consts::OS,
            "connectionType": 1
        }
    })
}

// =====================================================================
// LAN mirror of the local player server
// =====================================================================

#[derive(Clone)]
struct LanCtx {
    token: Arc<String>,
    /// `http://127.0.0.1:<port>` of the player server.
    upstream: Arc<String>,
    /// `http://<lan ip>:<port>/c/<token>/p`
    public: Arc<String>,
    alive: Arc<AtomicBool>,
    client: reqwest::Client,
    /// Inline (`data:`) subtitles, served as `/c/<token>/s/<n>.vtt`.
    subs: Arc<Mutex<Vec<String>>>,
}

struct Lan {
    public: String,
    upstream: String,
    alive: Arc<AtomicBool>,
    subs: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Lan {
    /// The LAN URL for a player-server URL; other URLs go through the
    /// server's remote proxy first so the TV never needs our headers.
    fn map(&self, url: &str) -> String {
        if let Some(data) = url.strip_prefix("data:") {
            use base64::Engine;
            let (meta, body) = data.split_once(',').unwrap_or(("", data));
            let text = if meta.ends_with(";base64") {
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default()
            } else {
                urlencoding::decode(body)
                    .map(|c| c.into_owned())
                    .unwrap_or_default()
            };
            let n = self
                .subs
                .lock()
                .map(|mut v| {
                    v.push(text);
                    v.len() - 1
                })
                .unwrap_or(0);
            let root = self.public.trim_end_matches("/p");
            return format!("{root}/s/{n}.vtt");
        }
        for local in [
            self.upstream.clone(),
            self.upstream.replace("127.0.0.1", "localhost"),
        ] {
            if let Some(rest) = url.strip_prefix(&local) {
                return format!("{}{}", self.public, rest);
            }
        }
        let proxied = server::register_remote(url, Vec::new());
        match proxied.strip_prefix(&self.upstream) {
            Some(rest) => format!("{}{}", self.public, rest),
            None => proxied,
        }
    }

    fn shutdown(&self) {
        self.alive.store(false, Ordering::SeqCst);
        self.task.abort();
    }
}

/// The local address the OS would use to reach the device (catt's trick:
/// a connected UDP socket sends nothing but picks the route).
fn local_ip_for(device: IpAddr) -> Result<IpAddr, String> {
    let s = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
    s.connect((device, 8009))
        .map_err(|_| "No network route to the TV".to_string())?;
    let ip = s.local_addr().map_err(|e| e.to_string())?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        return Err("No network route to the TV".into());
    }
    Ok(ip)
}

async fn start_lan(local: IpAddr, upstream: String) -> Result<Lan, String> {
    let listener = tokio::net::TcpListener::bind((local, 0))
        .await
        .map_err(|e| format!("Can't open the cast server: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let public = format!("http://{local}:{port}/c/{token}/p");
    let alive = Arc::new(AtomicBool::new(true));
    let subs = Arc::new(Mutex::new(Vec::new()));
    let ctx = LanCtx {
        subs: subs.clone(),
        token: Arc::new(token),
        upstream: Arc::new(upstream.clone()),
        public: Arc::new(public.clone()),
        alive: alive.clone(),
        // No total timeout: a movie streams for hours.
        client: reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(8))
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?,
    };
    let router = Router::new()
        .route("/c/{token}/p/{*rest}", get(lan_proxy))
        .route("/c/{token}/s/{file}", get(lan_subtitle))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
                .expose_headers(Any),
        )
        .with_state(ctx);
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::warn!("[video] cast server stopped: {e}");
        }
    });
    tracing::info!("[video] cast server on {local}:{port}");
    Ok(Lan {
        public,
        upstream,
        alive,
        subs,
        task,
    })
}

async fn lan_subtitle(
    State(ctx): State<LanCtx>,
    Path((token, file)): Path<(String, String)>,
) -> Response {
    if !ctx.alive.load(Ordering::SeqCst)
        || !constant_time_eq::constant_time_eq(token.as_bytes(), ctx.token.as_bytes())
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let n: usize = match file.trim_end_matches(".vtt").parse() {
        Ok(n) => n,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let body = ctx.subs.lock().ok().and_then(|v| v.get(n).cloned());
    match body {
        Some(b) => ([(header::CONTENT_TYPE, "text/vtt; charset=utf-8")], b).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn lan_proxy(
    State(ctx): State<LanCtx>,
    Path((token, rest)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if !ctx.alive.load(Ordering::SeqCst)
        || !constant_time_eq::constant_time_eq(token.as_bytes(), ctx.token.as_bytes())
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let url = match query {
        Some(q) => format!("{}/{}?{}", ctx.upstream, rest, q),
        None => format!("{}/{}", ctx.upstream, rest),
    };
    let mut req = ctx.client.get(&url);
    if let Some(r) = headers.get(header::RANGE) {
        req = req.header(header::RANGE, r.clone());
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_playlist = ctype.contains("mpegurl") || rest.ends_with(".m3u8");
    let mut out = Response::builder().status(status);
    for h in [
        header::CONTENT_TYPE,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::CACHE_CONTROL,
    ] {
        if let Some(v) = resp.headers().get(&h) {
            out = out.header(h, v.clone());
        }
    }
    if is_playlist {
        let text = resp.text().await.unwrap_or_default();
        let local = format!("{}/", ctx.upstream);
        let body = text.replace(&local, &format!("{}/", ctx.public)).replace(
            &local.replace("127.0.0.1", "localhost"),
            &format!("{}/", ctx.public),
        );
        return out
            .body(Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    if let Some(len) = resp.headers().get(header::CONTENT_LENGTH) {
        out = out.header(header::CONTENT_LENGTH, len.clone());
    }
    if rest.ends_with(".vtt") && ctype.is_empty() {
        out = out.header(header::CONTENT_TYPE, HeaderValue::from_static("text/vtt"));
    }
    let stream = resp
        .bytes_stream()
        .map(|r| r.map_err(std::io::Error::other));
    out.body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// =====================================================================
// Session and status
// =====================================================================

#[derive(Serialize, Clone, Debug)]
pub struct CastStatus {
    pub device: Option<String>,
    /// "idle" | "connecting" | "buffering" | "playing" | "paused" | "error"
    pub state: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub volume: f64,
    pub muted: bool,
    pub title: String,
    pub error: Option<String>,
}

impl Default for CastStatus {
    fn default() -> Self {
        Self {
            device: None,
            state: "idle".into(),
            position_ms: 0,
            duration_ms: 0,
            volume: 1.0,
            muted: false,
            title: String::new(),
            error: None,
        }
    }
}

/// Status plus what is needed to interpolate the position between the
/// device's own reports (it only sends them on changes).
struct Live {
    status: CastStatus,
    media_session: Option<i64>,
    app_session: String,
    transport: String,
    /// Position reported by the device, in the stream's own clock.
    reported_ms: i64,
    reported_at: Instant,
    rate: f64,
    /// Added to the device position when the stream was reloaded at an offset.
    offset_ms: i64,
    generation: u64,
}

impl Live {
    fn position(&self) -> i64 {
        let mut p = self.offset_ms + self.reported_ms;
        if self.status.state == "playing" {
            p += (self.reported_at.elapsed().as_millis() as f64 * self.rate) as i64;
        }
        if self.status.duration_ms > 0 {
            p = p.min(self.status.duration_ms);
        }
        p.max(0)
    }

    fn snapshot(&self) -> CastStatus {
        let mut s = self.status.clone();
        s.position_ms = self.position();
        s
    }
}

fn live() -> &'static Mutex<Live> {
    static L: OnceLock<Mutex<Live>> = OnceLock::new();
    L.get_or_init(|| {
        Mutex::new(Live {
            status: CastStatus::default(),
            media_session: None,
            app_session: String::new(),
            transport: String::new(),
            reported_ms: 0,
            reported_at: Instant::now(),
            rate: 1.0,
            offset_ms: 0,
            generation: 0,
        })
    })
}

struct Session {
    app: AppHandle,
    chan: Arc<Channel>,
    lan: Lan,
    info: PlaybackInfo,
    generation: u64,
    ticker: tokio::task::JoinHandle<()>,
    listener: tokio::task::JoinHandle<()>,
}

fn session() -> &'static tokio::sync::Mutex<Option<Session>> {
    static S: OnceLock<tokio::sync::Mutex<Option<Session>>> = OnceLock::new();
    S.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn emit(app: &AppHandle) {
    let snap = match live().lock() {
        Ok(l) => l.snapshot(),
        Err(_) => return,
    };
    let _ = app.emit(EVENT, snap);
}

fn set_state(app: &AppHandle, f: impl FnOnce(&mut Live)) {
    if let Ok(mut l) = live().lock() {
        f(&mut l);
    }
    emit(app);
}

/// Applies a `MEDIA_STATUS` / `RECEIVER_STATUS` / `CLOSE` from the device.
fn on_message(app: &AppHandle, generation: u64, ns: &str, v: &Value) {
    let mut l = match live().lock() {
        Ok(l) => l,
        Err(_) => return,
    };
    if l.generation != generation {
        return;
    }
    match (ns, v["type"].as_str().unwrap_or("")) {
        (NS_MEDIA, "MEDIA_STATUS") => {
            let Some(st) = v["status"].as_array().and_then(|a| a.first()) else {
                // Empty status: the media ended or was unloaded.
                if l.media_session.is_some() && l.status.state != "connecting" {
                    l.status.state = "idle".into();
                }
                drop(l);
                emit(app);
                return;
            };
            if let Some(id) = st["mediaSessionId"].as_i64() {
                l.media_session = Some(id);
            }
            if let Some(t) = st["currentTime"].as_f64() {
                l.reported_ms = (t * 1000.0) as i64;
                l.reported_at = Instant::now();
            }
            if let Some(r) = st["playbackRate"].as_f64() {
                l.rate = r;
            }
            if let Some(d) = st["media"]["duration"].as_f64().filter(|d| *d > 0.0) {
                // A reloaded stream reports the remaining length only.
                l.status.duration_ms = l.status.duration_ms.max(l.offset_ms + (d * 1000.0) as i64);
            }
            match st["playerState"].as_str().unwrap_or("") {
                "PLAYING" => l.status.state = "playing".into(),
                "PAUSED" => l.status.state = "paused".into(),
                "BUFFERING" | "LOADING" => l.status.state = "buffering".into(),
                "IDLE" => match st["idleReason"].as_str() {
                    Some("ERROR") => {
                        l.status.state = "error".into();
                        l.status.error = Some("The TV couldn't play this video".into());
                    }
                    Some("FINISHED") => {
                        l.reported_ms = l.status.duration_ms - l.offset_ms;
                        l.status.state = "idle".into();
                    }
                    Some(_) => l.status.state = "idle".into(),
                    None => {}
                },
                _ => {}
            }
        }
        (NS_MEDIA, "LOAD_FAILED" | "LOAD_CANCELLED" | "INVALID_REQUEST") => {
            l.status.state = "error".into();
            l.status.error = Some("The TV couldn't play this video".into());
        }
        (NS_RECV, "RECEIVER_STATUS") => {
            let st = &v["status"];
            if let Some(level) = st["volume"]["level"].as_f64() {
                l.status.volume = level;
            }
            if let Some(m) = st["volume"]["muted"].as_bool() {
                l.status.muted = m;
            }
            if let Some(apps) = st["applications"].as_array() {
                let ours = !l.app_session.is_empty()
                    && apps
                        .iter()
                        .any(|a| a["sessionId"].as_str() == Some(l.app_session.as_str()));
                if !l.app_session.is_empty() && !ours && l.status.state != "connecting" {
                    // Someone else took over the TV.
                    l.status.state = "idle".into();
                    l.app_session.clear();
                }
            }
        }
        (NS_CONN, "CLOSE") => {
            if l.status.state != "connecting" {
                l.status.state = "idle".into();
            }
        }
        _ => return,
    }
    drop(l);
    emit(app);
}

fn with_ss(url: &str, ms: i64) -> String {
    let secs = format!("{:.3}", ms as f64 / 1000.0);
    match url::Url::parse(url) {
        Ok(mut u) => {
            let pairs: Vec<(String, String)> = u
                .query_pairs()
                .filter(|(k, _)| k != "ss")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            {
                let mut q = u.query_pairs_mut();
                q.clear();
                for (k, v) in &pairs {
                    q.append_pair(k, v);
                }
                q.append_pair("ss", &secs);
            }
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

fn content_type(info: &PlaybackInfo) -> &'static str {
    let lower = info.url.to_ascii_lowercase();
    if info.mode == PlayMode::Hls || lower.contains(".m3u8") {
        return "application/x-mpegURL";
    }
    match info.mode {
        PlayMode::Remux | PlayMode::TranscodeAudio | PlayMode::Transcode => "video/mp4",
        _ => {
            let c = info.info.container.to_ascii_lowercase();
            let path = lower.split('?').next().unwrap_or("");
            if c.contains("webm") || path.ends_with(".webm") {
                "video/webm"
            } else if c.contains("matroska") || c.contains("mkv") || path.ends_with(".mkv") {
                "video/x-matroska"
            } else if c == "ts" || c.contains("mpegts") || path.ends_with(".ts") {
                "video/mp2t"
            } else {
                "video/mp4"
            }
        }
    }
}

fn load_payload(info: &PlaybackInfo, lan: &Lan, start_ms: i64, app_session: &str) -> (Value, i64) {
    let reload = info.seek_reloads && start_ms > 0;
    let url = if reload {
        lan.map(&with_ss(&info.url, start_ms))
    } else {
        lan.map(&info.url)
    };
    let offset = if reload { start_ms } else { 0 };
    let current = if reload || info.is_live {
        0.0
    } else {
        start_ms as f64 / 1000.0
    };

    let mut tracks = Vec::new();
    let mut active: Vec<i64> = Vec::new();
    for (n, sub) in info.subtitles.iter().enumerate() {
        let track_id = n as i64 + 1;
        tracks.push(json!({
            "trackId": track_id,
            "type": "TEXT",
            "subtype": "SUBTITLES",
            "trackContentId": lan.map(&sub.url),
            "trackContentType": "text/vtt",
            "language": sub.lang,
            "name": sub.label,
        }));
        let wanted = match (&info.prefs.subtitle_id, &info.prefs.subtitle_lang) {
            (Some(id), _) => *id == sub.id,
            (None, Some(lang)) => sub.lang.eq_ignore_ascii_case(lang),
            (None, None) => sub.forced,
        };
        if wanted && active.is_empty() {
            active.push(track_id);
        }
    }
    let mut media = json!({
        "contentId": url,
        "contentType": content_type(info),
        "streamType": if info.is_live { "LIVE" } else { "BUFFERED" },
        "metadata": { "metadataType": 0, "title": info.title, "subtitle": info.subtitle },
    });
    if info.duration_ms > 0 && !info.is_live {
        media["duration"] = json!((info.duration_ms - offset).max(0) as f64 / 1000.0);
    }
    if !tracks.is_empty() {
        media["tracks"] = json!(tracks);
        media["textTrackStyle"] = json!({
            "backgroundColor": if info.prefs.subtitle_bg { "#00000099" } else { "#00000000" },
            "foregroundColor": "#FFFFFFFF",
            "edgeType": "OUTLINE",
            "edgeColor": "#000000FF",
            "fontScale": info.prefs.subtitle_size as f64 / 100.0,
            "fontGenericFamily": "SANS_SERIF",
            "windowType": "NONE"
        });
    }
    (
        json!({
            "type": "LOAD",
            "sessionId": app_session,
            "media": media,
            "autoplay": true,
            "currentTime": current,
            "activeTrackIds": active,
            "customData": {}
        }),
        offset,
    )
}

/// Finds the Default Media Receiver on the device, launching it if needed.
async fn launch(chan: &Channel) -> Result<(String, String), String> {
    fn find(v: &Value) -> Option<(String, String)> {
        v["status"]["applications"]
            .as_array()?
            .iter()
            .find_map(|a| {
                (a["appId"].as_str() == Some(DEFAULT_RECEIVER)).then(|| {
                    (
                        a["sessionId"].as_str().unwrap_or_default().to_string(),
                        a["transportId"].as_str().unwrap_or_default().to_string(),
                    )
                })
            })
    }
    let st = chan
        .request(
            PLATFORM,
            NS_RECV,
            json!({ "type": "GET_STATUS" }),
            REQUEST_TIMEOUT,
        )
        .await?;
    if let Some(found) = find(&st).filter(|f| !f.1.is_empty()) {
        return Ok(found);
    }
    let r = chan
        .request(
            PLATFORM,
            NS_RECV,
            json!({ "type": "LAUNCH", "appId": DEFAULT_RECEIVER }),
            REQUEST_TIMEOUT,
        )
        .await?;
    if r["type"] == "LAUNCH_ERROR" {
        return Err("The TV refused to open the player".into());
    }
    if let Some(found) = find(&r).filter(|f| !f.1.is_empty()) {
        return Ok(found);
    }
    for _ in 0..16 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let st = chan
            .request(
                PLATFORM,
                NS_RECV,
                json!({ "type": "GET_STATUS" }),
                REQUEST_TIMEOUT,
            )
            .await?;
        if let Some(found) = find(&st).filter(|f| !f.1.is_empty()) {
            return Ok(found);
        }
    }
    Err("The TV did not open the player".into())
}

async fn send_load(chan: &Channel, transport: &str, payload: Value) -> Result<i64, String> {
    let r = chan
        .request(transport, NS_MEDIA, payload, LOAD_TIMEOUT)
        .await?;
    match r["type"].as_str() {
        Some("MEDIA_STATUS") => Ok(r["status"][0]["mediaSessionId"].as_i64().unwrap_or(0)),
        Some("LOAD_FAILED") => Err("The TV couldn't play this video".into()),
        Some("LOAD_CANCELLED") => Err("Playback was cancelled on the TV".into()),
        _ => Err("The TV couldn't play this video".into()),
    }
}

fn source_key(info: &PlaybackInfo) -> (String, SourceRef) {
    (info.key.clone(), info.source.clone())
}

/// Records progress on the PC side while the TV plays.
async fn save_progress(info: &PlaybackInfo, jf_event: Option<&str>) {
    let (pos, dur, playing) = match live().lock() {
        Ok(l) => (
            l.position(),
            l.status.duration_ms,
            l.status.state == "playing",
        ),
        Err(_) => return,
    };
    if info.is_live || pos <= 0 {
        return;
    }
    let (key, source) = source_key(info);
    let _ = super::progress_record(&key, pos, dur.max(info.duration_ms), playing);
    if let (Some(ev), SourceRef::Jellyfin { source, item }) = (jf_event, source) {
        let _ = super::jellyfin::report(source, &item, ev, pos, !playing).await;
    }
}

fn spawn_ticker(
    app: AppHandle,
    chan: Arc<Channel>,
    info: PlaybackInfo,
    generation: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick += 1;
            let (state, transport, media_session) = match live().lock() {
                Ok(l) if l.generation == generation => {
                    (l.status.state.clone(), l.transport.clone(), l.media_session)
                }
                _ => break,
            };
            if matches!(state.as_str(), "playing" | "paused" | "buffering") {
                emit(&app);
            }
            if tick.is_multiple_of(5) {
                chan.send_raw(PLATFORM, NS_HB, json!({ "type": "PING" }));
                if let Some(id) = media_session {
                    chan.send(
                        &transport,
                        NS_MEDIA,
                        json!({ "type": "GET_STATUS", "mediaSessionId": id, "requestId": 0 }),
                    );
                }
                let stale = chan
                    .last_seen
                    .lock()
                    .map(|t| t.elapsed() > Duration::from_secs(30))
                    .unwrap_or(false);
                if stale {
                    set_state(&app, |l| {
                        l.status.state = "error".into();
                        l.status.error = Some("The TV stopped answering".into());
                    });
                    break;
                }
                let jf = if tick.is_multiple_of(10) {
                    Some("progress")
                } else {
                    None
                };
                save_progress(&info, jf).await;
            }
        }
    })
}

fn spawn_listener(
    app: AppHandle,
    mut rx: mpsc::UnboundedReceiver<Incoming>,
    generation: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                Incoming::Message(ns, v) => on_message(&app, generation, &ns, &v),
                Incoming::Closed(reason) => {
                    let mine = live()
                        .lock()
                        .map(|l| l.generation == generation)
                        .unwrap_or(false);
                    if mine {
                        set_state(&app, |l| {
                            if l.status.state != "idle" {
                                l.status.state = "error".into();
                                l.status.error = Some(reason);
                            }
                        });
                    }
                    break;
                }
            }
        }
    })
}

/// Ends the current session: stops the media, closes the receiver app and
/// the LAN mirror.
async fn end_session(close_app: bool) {
    let Some(s) = session().lock().await.take() else {
        return;
    };
    save_progress(&s.info, Some("stop")).await;
    let (transport, media_session, app_session) = live()
        .lock()
        .map(|l| (l.transport.clone(), l.media_session, l.app_session.clone()))
        .unwrap_or_default();
    if close_app {
        if let Some(id) = media_session {
            s.chan.send(
                &transport,
                NS_MEDIA,
                json!({ "type": "STOP", "mediaSessionId": id, "requestId": 0 }),
            );
        }
        if !app_session.is_empty() {
            s.chan.send_raw(
                PLATFORM,
                NS_RECV,
                json!({ "type": "STOP", "sessionId": app_session, "requestId": 0 }),
            );
        }
    }
    s.chan.close();
    s.ticker.abort();
    s.listener.abort();
    s.lan.shutdown();
    if let Ok(mut l) = live().lock() {
        l.generation += 1;
        l.media_session = None;
        l.app_session.clear();
        l.transport.clear();
        l.status = CastStatus {
            volume: l.status.volume,
            muted: l.status.muted,
            ..CastStatus::default()
        };
    }
    emit(&s.app);
}

// =====================================================================
// Commands
// =====================================================================

#[tauri::command]
pub async fn video_cast_discover() -> Result<Vec<CastDevice>, String> {
    let found = discover(Duration::from_secs(3)).await;
    if let Ok(mut d) = devices().lock() {
        for dev in &found {
            d.insert(dev.id.clone(), dev.clone());
        }
    }
    Ok(found)
}

#[tauri::command]
pub async fn video_cast_start(
    app: AppHandle,
    device_id: String,
    source: SourceRef,
    start_ms: Option<i64>,
) -> Result<CastStatus, String> {
    let mut device = devices()
        .lock()
        .ok()
        .and_then(|d| d.get(&device_id).cloned());
    if device.is_none() {
        let _ = video_cast_discover().await;
        device = devices()
            .lock()
            .ok()
            .and_then(|d| d.get(&device_id).cloned());
    }
    let device = device.ok_or("TV not found on the network")?;
    let host: IpAddr = device
        .host
        .parse()
        .map_err(|_| "TV not found on the network")?;

    end_session(false).await;
    let generation = {
        let mut l = live().lock().map_err(|e| e.to_string())?;
        l.generation += 1;
        l.status = CastStatus {
            device: Some(device.name.clone()),
            state: "connecting".into(),
            ..CastStatus::default()
        };
        l.media_session = None;
        l.app_session.clear();
        l.transport.clear();
        l.reported_ms = 0;
        l.offset_ms = 0;
        l.generation
    };
    emit(&app);

    let fail = |app: &AppHandle, e: String| {
        set_state(app, |l| {
            if l.generation == generation {
                l.status.state = "error".into();
                l.status.error = Some(e.clone());
            }
        });
        e
    };

    let info = match sources::resolve(&app, source, true).await {
        Ok(i) => i,
        Err(e) => return Err(fail(&app, e)),
    };
    let local = match server::ensure(&app).await {
        Ok(s) => server::base(&s),
        Err(e) => return Err(fail(&app, e)),
    };
    let lan_ip = match local_ip_for(host) {
        Ok(ip) => ip,
        Err(e) => return Err(fail(&app, e)),
    };
    let lan = match start_lan(lan_ip, local).await {
        Ok(l) => l,
        Err(e) => return Err(fail(&app, e)),
    };

    let (tx, rx) = mpsc::unbounded_channel();
    let chan = match Channel::connect(host, device.port, tx).await {
        Ok(c) => c,
        Err(e) => {
            lan.shutdown();
            return Err(fail(&app, e));
        }
    };
    let listener = spawn_listener(app.clone(), rx, generation);
    let start = if info.is_live {
        0
    } else {
        start_ms.filter(|s| *s > 0).unwrap_or(info.start_ms).max(0)
    };

    let result = async {
        let (app_session, transport) = launch(&chan).await?;
        if let Ok(mut l) = live().lock() {
            l.app_session = app_session.clone();
            l.transport = transport.clone();
            l.status.title = info.title.clone();
            l.status.duration_ms = info.duration_ms;
            l.status.state = "buffering".into();
        }
        let (payload, offset) = load_payload(&info, &lan, start, &app_session);
        if let Ok(mut l) = live().lock() {
            l.offset_ms = offset;
            l.reported_ms = start - offset;
            l.reported_at = Instant::now();
        }
        let media_session = send_load(&chan, &transport, payload).await?;
        if let Ok(mut l) = live().lock() {
            if media_session > 0 {
                l.media_session = Some(media_session);
            }
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(e) = result {
        chan.close();
        listener.abort();
        lan.shutdown();
        return Err(fail(&app, e));
    }

    if let SourceRef::Jellyfin { source, item } = &info.source {
        let _ = super::jellyfin::report(*source, item, "start", start, false).await;
    }
    let ticker = spawn_ticker(app.clone(), chan.clone(), info.clone(), generation);
    *session().lock().await = Some(Session {
        app: app.clone(),
        chan,
        lan,
        info,
        generation,
        ticker,
        listener,
    });
    emit(&app);
    video_cast_status().await
}

#[tauri::command]
pub async fn video_cast_control(action: String, value: Option<f64>) -> Result<(), String> {
    let guard = session().lock().await;
    let s = guard.as_ref().ok_or("Nothing is casting")?;
    let (transport, media_session, muted) = {
        let l = live().lock().map_err(|e| e.to_string())?;
        if l.generation != s.generation {
            return Err("Nothing is casting".into());
        }
        (l.transport.clone(), l.media_session, l.status.muted)
    };
    let media = |v: Value| -> Result<Value, String> {
        let id = media_session.ok_or("The TV is still loading")?;
        let mut v = v;
        v["mediaSessionId"] = json!(id);
        Ok(v)
    };
    match action.as_str() {
        "play" => {
            s.chan
                .request(
                    &transport,
                    NS_MEDIA,
                    media(json!({ "type": "PLAY" }))?,
                    REQUEST_TIMEOUT,
                )
                .await?;
        }
        "pause" => {
            s.chan
                .request(
                    &transport,
                    NS_MEDIA,
                    media(json!({ "type": "PAUSE" }))?,
                    REQUEST_TIMEOUT,
                )
                .await?;
            save_progress(&s.info, Some("progress")).await;
        }
        "seek" => {
            let target = value.ok_or("Missing position")?.max(0.0) as i64;
            if s.info.seek_reloads {
                // Piped streams can't seek; reload at the new point.
                let app_session = live()
                    .lock()
                    .map(|l| l.app_session.clone())
                    .unwrap_or_default();
                let (payload, offset) = load_payload(&s.info, &s.lan, target, &app_session);
                if let Ok(mut l) = live().lock() {
                    l.offset_ms = offset;
                    l.reported_ms = target - offset;
                    l.reported_at = Instant::now();
                    l.status.state = "buffering".into();
                }
                let id = send_load(&s.chan, &transport, payload).await?;
                if let Ok(mut l) = live().lock() {
                    if id > 0 {
                        l.media_session = Some(id);
                    }
                }
            } else {
                let offset = live().lock().map(|l| l.offset_ms).unwrap_or(0);
                s.chan
                    .request(
                        &transport,
                        NS_MEDIA,
                        media(json!({
                            "type": "SEEK",
                            "currentTime": (target - offset).max(0) as f64 / 1000.0,
                            "resumeState": "PLAYBACK_START"
                        }))?,
                        REQUEST_TIMEOUT,
                    )
                    .await?;
                if let Ok(mut l) = live().lock() {
                    l.reported_ms = target - offset;
                    l.reported_at = Instant::now();
                }
            }
        }
        "volume" => {
            let level = value.ok_or("Missing volume")?.clamp(0.0, 1.0);
            s.chan
                .request(
                    PLATFORM,
                    NS_RECV,
                    json!({ "type": "SET_VOLUME", "volume": { "level": level } }),
                    REQUEST_TIMEOUT,
                )
                .await?;
            if let Ok(mut l) = live().lock() {
                l.status.volume = level;
            }
        }
        "mute" => {
            let mute = value.map(|v| v != 0.0).unwrap_or(!muted);
            s.chan
                .request(
                    PLATFORM,
                    NS_RECV,
                    json!({ "type": "SET_VOLUME", "volume": { "muted": mute } }),
                    REQUEST_TIMEOUT,
                )
                .await?;
            if let Ok(mut l) = live().lock() {
                l.status.muted = mute;
            }
        }
        _ => return Err("Unknown action".into()),
    }
    emit(&s.app);
    Ok(())
}

#[tauri::command]
pub async fn video_cast_stop() -> Result<(), String> {
    end_session(true).await;
    Ok(())
}

#[tauri::command]
pub async fn video_cast_status() -> Result<CastStatus, String> {
    live()
        .lock()
        .map(|l| l.snapshot())
        .map_err(|e| e.to_string())
}
