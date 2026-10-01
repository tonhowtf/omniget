use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures::StreamExt;
use omniget_core::core::tube::{mpd, player, Format, StreamSet};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::sync::OnceCell;
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::http;

const CHUNK: u64 = 4 * 1024 * 1024;
const STALL: Duration = Duration::from_secs(20);
const MAX_RECOVERIES: u32 = 3;
const SESSION_CAP: usize = 64;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ServerInfo {
    pub port: u16,
    pub token: String,
}

#[derive(Clone)]
struct Ctx {
    token: Arc<String>,
    port: u16,
}

static SERVER: OnceCell<ServerInfo> = OnceCell::const_new();

pub async fn ensure(_app: &AppHandle) -> Result<ServerInfo, String> {
    start().await
}

pub async fn start() -> Result<ServerInfo, String> {
    SERVER
        .get_or_try_init(|| async {
            let token = crate::local_bridge::generate_token();
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| e.to_string())?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            let ctx = Ctx {
                token: Arc::new(token.clone()),
                port,
            };
            let router = Router::new()
                .route("/yt/{sid}/{file}", get(entry))
                .route("/yt/{sid}/cc/{file}", get(caption))
                .route("/yt/{sid}/sb/{file}", get(storyboard))
                .layer(cors())
                .with_state(ctx);
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, router).await {
                    tracing::error!("[tube] server stopped: {e}");
                }
            });
            tracing::info!("[tube] server on 127.0.0.1:{port}");
            Ok(ServerInfo { port, token })
        })
        .await
        .cloned()
}

pub fn info() -> Option<ServerInfo> {
    SERVER.get().cloned()
}

pub fn base(info: &ServerInfo) -> String {
    format!("http://127.0.0.1:{}", info.port)
}

pub fn session_base(info: &ServerInfo, session: &str) -> String {
    format!("{}/yt/{session}", base(info))
}

// WHY: webview origins differ: tauri://localhost on macOS/Linux, http(s)://tauri.localhost on Windows.
fn app_origins() -> Vec<String> {
    let mut v: Vec<String> = [
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if cfg!(debug_assertions) {
        v.push("http://localhost:1420".into());
        v.push("http://127.0.0.1:1420".into());
        if let Some(o) = std::env::var("OMNIGET_DEV_URL")
            .ok()
            .and_then(|u| url::Url::parse(&u).ok())
            .map(|u| u.origin().ascii_serialization())
        {
            v.push(o);
        }
        if let Ok(host) = std::env::var("TAURI_DEV_HOST") {
            if !host.trim().is_empty() {
                v.push(format!("http://{}:1420", host.trim()));
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

fn cors() -> CorsLayer {
    let origins: Vec<HeaderValue> = app_origins()
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
        .allow_headers([header::RANGE])
        .expose_headers([
            header::CONTENT_LENGTH,
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::CONTENT_TYPE,
        ])
        .allow_private_network(true)
}

#[derive(Clone, Debug, Default)]
pub struct CaptionSrc {
    pub url: String,
    pub raw: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Extras {
    pub music: bool,
    pub hls: Option<String>,
    pub captions: Vec<CaptionSrc>,
    pub storyboard: Option<String>,
}

struct Session {
    video_id: String,
    music: bool,
    ua: String,
    duration_s: u32,
    formats: HashMap<u32, Format>,
    hls: Option<String>,
    captions: Vec<CaptionSrc>,
    storyboard: Option<String>,
    refreshed: Option<Instant>,
    gate: Arc<tokio::sync::Mutex<()>>,
}

type Table = (HashMap<String, Session>, VecDeque<String>);

fn sessions() -> &'static Mutex<Table> {
    static S: OnceLock<Mutex<Table>> = OnceLock::new();
    S.get_or_init(|| Mutex::new((HashMap::new(), VecDeque::new())))
}

pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

pub fn register(set: &StreamSet, formats_with_urls: Vec<Format>, user_agent: String) {
    register_full(set, formats_with_urls, user_agent, Extras::default());
}

pub fn register_full(set: &StreamSet, formats: Vec<Format>, user_agent: String, extras: Extras) {
    let Ok(mut g) = sessions().lock() else {
        return;
    };
    let (map, order) = &mut *g;
    let session = Session {
        video_id: set.video_id.clone(),
        music: extras.music,
        ua: user_agent,
        duration_s: set.duration_s,
        formats: formats
            .into_iter()
            .filter(|f| !f.url.is_empty())
            .map(|f| (f.itag, f))
            .collect(),
        hls: extras.hls,
        captions: extras.captions,
        storyboard: extras.storyboard,
        refreshed: None,
        gate: Arc::new(tokio::sync::Mutex::new(())),
    };
    if map.insert(set.session.clone(), session).is_none() {
        order.push_back(set.session.clone());
    }
    while order.len() > SESSION_CAP {
        if let Some(old) = order.pop_front() {
            map.remove(&old);
        }
    }
}

pub fn has_session(sid: &str) -> bool {
    sessions()
        .lock()
        .map(|g| g.0.contains_key(sid))
        .unwrap_or(false)
}

#[derive(Clone)]
struct Target {
    url: String,
    ua: String,
    len: Option<u64>,
    mime: String,
}

fn target(sid: &str, itag: u32) -> Option<Target> {
    let g = sessions().lock().ok()?;
    let s = g.0.get(sid)?;
    let f = s.formats.get(&itag)?;
    Some(Target {
        url: f.url.clone(),
        ua: s.ua.clone(),
        len: f.content_length.or_else(|| clen(&f.url)),
        mime: f.mime.split(';').next().unwrap_or("").trim().to_string(),
    })
}

fn remember_len(sid: &str, itag: u32, len: u64) {
    if let Ok(mut g) = sessions().lock() {
        if let Some(f) = g.0.get_mut(sid).and_then(|s| s.formats.get_mut(&itag)) {
            f.content_length = Some(len);
        }
    }
}

async fn refresh_session(sid: &str) -> bool {
    let (gate, video_id, music) = {
        let Ok(g) = sessions().lock() else {
            return false;
        };
        let Some(s) = g.0.get(sid) else {
            return false;
        };
        (s.gate.clone(), s.video_id.clone(), s.music)
    };
    let _held = gate.lock().await;
    let recent = sessions()
        .lock()
        .ok()
        .and_then(|g| g.0.get(sid).and_then(|s| s.refreshed))
        .map(|at| at.elapsed() < Duration::from_secs(15))
        .unwrap_or(false);
    if recent {
        return true;
    }
    tracing::info!("[tube] stream expired for {video_id}, resolving again");
    let fresh = match super::streams::refresh(&video_id, music).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[tube] re-resolve failed for {video_id}: {e}");
            return false;
        }
    };
    let Ok(mut g) = sessions().lock() else {
        return false;
    };
    let Some(s) = g.0.get_mut(sid) else {
        return false;
    };
    for f in fresh.formats {
        if f.url.is_empty() {
            continue;
        }
        match s.formats.get_mut(&f.itag) {
            Some(old) => {
                old.url = f.url;
                if f.content_length.is_some() {
                    old.content_length = f.content_length;
                }
            }
            None => {
                s.formats.insert(f.itag, f);
            }
        }
    }
    s.ua = fresh.ua;
    if fresh.hls.is_some() {
        s.hls = fresh.hls;
    }
    s.refreshed = Some(Instant::now());
    true
}

fn host_ok(ctx: &Ctx, headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let (name, port) = match host.rsplit_once(':') {
        Some((n, p)) => (n, p.parse::<u16>().ok()),
        None => (host, None),
    };
    matches!(name, "127.0.0.1" | "localhost") && port.is_none_or(|p| p == ctx.port)
}

fn allowed(ctx: &Ctx, q: &HashMap<String, String>, headers: &HeaderMap) -> bool {
    host_ok(ctx, headers)
        && q.get("k")
            .map(|k| constant_time_eq::constant_time_eq(k.as_bytes(), ctx.token.as_bytes()))
            .unwrap_or(false)
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len.saturating_sub(1))
    } else {
        let s: u64 = a.parse().ok()?;
        let e = if b.is_empty() {
            len.saturating_sub(1)
        } else {
            b.parse::<u64>().ok()?.min(len.saturating_sub(1))
        };
        (s, e)
    };
    (start <= end && start < len).then_some((start, end))
}

fn clen(url: &str) -> Option<u64> {
    let q = url.split_once('?')?.1;
    q.split('&')
        .find_map(|kv| kv.strip_prefix("clen="))
        .and_then(|v| v.parse().ok())
}

fn range_total(v: &str) -> Option<u64> {
    v.rsplit_once('/')?.1.trim().parse().ok()
}

fn is_expired(code: reqwest::StatusCode) -> bool {
    matches!(code.as_u16(), 401 | 403 | 404 | 410)
}

fn youtube_host(u: &url::Url) -> bool {
    let Some(h) = u.host_str() else {
        return false;
    };
    u.scheme() == "https"
        && (h.ends_with(".googlevideo.com")
            || h == "youtube.com"
            || h.ends_with(".youtube.com")
            || h.ends_with(".ytimg.com"))
}

async fn no_stall<F: std::future::Future>(f: F) -> Option<F::Output> {
    tokio::time::timeout(STALL, f).await.ok()
}

async fn probe_len(sid: &str, itag: u32) -> Option<u64> {
    for _ in 0..2 {
        let t = target(sid, itag)?;
        let resp = no_stall(
            http::media_client()
                .get(&t.url)
                .header(reqwest::header::USER_AGENT, &t.ua)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send(),
        )
        .await?
        .ok()?;
        if is_expired(resp.status()) {
            if !refresh_session(sid).await {
                return None;
            }
            continue;
        }
        let total = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(range_total)?;
        remember_len(sid, itag, total);
        return Some(total);
    }
    None
}

async fn entry(
    State(ctx): State<Ctx>,
    Path((sid, file)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    if !has_session(&sid) {
        return status(StatusCode::NOT_FOUND);
    }
    match file.as_str() {
        "manifest.mpd" => manifest(&ctx, &sid, &q),
        "hls.m3u8" => hls_playlist(&ctx, &sid).await,
        "h" => hls_part(&ctx, &sid, &q, &headers).await,
        other => match other.split('.').next().and_then(|s| s.parse::<u32>().ok()) {
            Some(itag) => media(&sid, itag, &headers, method == Method::HEAD).await,
            None => status(StatusCode::NOT_FOUND),
        },
    }
}

async fn media(sid: &str, itag: u32, headers: &HeaderMap, head_only: bool) -> Response {
    let Some(t) = target(sid, itag) else {
        return status(StatusCode::NOT_FOUND);
    };
    let len = match t.len {
        Some(l) => l,
        None => match probe_len(sid, itag).await {
            Some(l) => l,
            None => return status(StatusCode::BAD_GATEWAY),
        },
    };
    let asked = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let range = asked.and_then(|h| parse_range(h, len));
    if asked.is_some() && range.is_none() {
        let mut r = status(StatusCode::RANGE_NOT_SATISFIABLE);
        if let Ok(v) = HeaderValue::from_str(&format!("bytes */{len}")) {
            r.headers_mut().insert(header::CONTENT_RANGE, v);
        }
        return r;
    }
    let (start, end, code) = match range {
        Some((s, e)) => (s, e, StatusCode::PARTIAL_CONTENT),
        None => (0, len.saturating_sub(1), StatusCode::OK),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };

    let body = if head_only || count == 0 {
        Body::empty()
    } else {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
        let sid = sid.to_string();
        tokio::spawn(async move { pump(sid, itag, start, end, tx).await });
        Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        }))
    };
    let mut resp = Response::new(body);
    *resp.status_mut() = code;
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&t.mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(count));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if code == StatusCode::PARTIAL_CONTENT {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{len}")) {
            h.insert(header::CONTENT_RANGE, v);
        }
    }
    resp
}

async fn pump(
    sid: String,
    itag: u32,
    start: u64,
    end: u64,
    tx: tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
) {
    let mut pos = start;
    let mut failures = 0u32;
    let fail = |msg: &str| Err(std::io::Error::other(msg.to_string()));
    while pos <= end {
        if failures > MAX_RECOVERIES {
            let _ = tx.send(fail("upstream failed")).await;
            return;
        }
        let Some(t) = target(&sid, itag) else {
            let _ = tx.send(fail("session gone")).await;
            return;
        };
        let chunk_end = (pos + CHUNK - 1).min(end);
        let sent = no_stall(
            http::media_client()
                .get(&t.url)
                .header(reqwest::header::USER_AGENT, &t.ua)
                .header(reqwest::header::RANGE, format!("bytes={pos}-{chunk_end}"))
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send(),
        )
        .await;
        let resp = match sent {
            Some(Ok(r)) => r,
            _ => {
                failures += 1;
                continue;
            }
        };
        let code = resp.status();
        if is_expired(code) {
            failures += 1;
            tracing::debug!("[tube] {} for itag {itag} at byte {pos}", code.as_u16());
            if !refresh_session(&sid).await {
                let _ = tx.send(fail("stream expired")).await;
                return;
            }
            continue;
        }
        if code != reqwest::StatusCode::PARTIAL_CONTENT
            && !(code == reqwest::StatusCode::OK && pos == 0)
        {
            failures += 1;
            continue;
        }
        let mut stream = resp.bytes_stream();
        loop {
            match no_stall(stream.next()).await {
                Some(Some(Ok(mut bytes))) => {
                    let room = chunk_end + 1 - pos;
                    if bytes.len() as u64 > room {
                        bytes.truncate(room as usize);
                    }
                    if bytes.is_empty() {
                        break;
                    }
                    pos += bytes.len() as u64;
                    if tx.send(Ok(bytes)).await.is_err() {
                        return;
                    }
                    if pos > chunk_end {
                        break;
                    }
                }
                Some(None) => break,
                _ => {
                    failures += 1;
                    break;
                }
            }
        }
    }
}

fn manifest(ctx: &Ctx, sid: &str, q: &HashMap<String, String>) -> Response {
    let list = |k: &str| -> Vec<u32> {
        q.get(k)
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default()
    };
    let (vs, as_) = (list("v"), list("a"));
    let (video, audio, duration) = {
        let Ok(g) = sessions().lock() else {
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let Some(s) = g.0.get(sid) else {
            return status(StatusCode::NOT_FOUND);
        };
        let pick = |ids: &[u32]| -> Vec<Format> {
            ids.iter()
                .filter_map(|i| s.formats.get(i).cloned())
                .collect()
        };
        (pick(&vs), pick(&as_), s.duration_s)
    };
    if video.is_empty() && audio.is_empty() {
        return status(StatusCode::NOT_FOUND);
    }
    let prefix = format!("http://127.0.0.1:{}/yt/{sid}", ctx.port);
    let token = ctx.token.clone();
    let url_for = move |itag: u32| format!("{prefix}/{itag}?k={token}");
    let text = mpd::synthetic_mpd(duration, &video, &audio, &url_for);
    let mut r = Response::new(Body::from(text));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/dash+xml"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn rewrite_hls(text: &str, final_url: &url::Url, prefix: &str, token: &str) -> String {
    let proxied = |u: &str| -> String {
        let abs = final_url
            .join(u.trim())
            .map(|x| x.to_string())
            .unwrap_or_else(|_| u.to_string());
        format!("{prefix}/h?u={}&k={token}", urlencoding::encode(&abs))
    };
    let mut out = String::with_capacity(text.len() * 2);
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            out.push('\n');
            continue;
        }
        if t.starts_with('#') {
            let mut rest = line;
            let mut rewritten = String::new();
            while let Some(i) = rest.find("URI=\"") {
                let start = i + 5;
                let Some(len) = rest[start..].find('"') else {
                    break;
                };
                rewritten.push_str(&rest[..start]);
                let inner = &rest[start..start + len];
                if inner.starts_with("data:") {
                    rewritten.push_str(inner);
                } else {
                    rewritten.push_str(&proxied(inner));
                }
                rest = &rest[start + len..];
            }
            rewritten.push_str(rest);
            out.push_str(&rewritten);
        } else {
            out.push_str(&proxied(t));
        }
        out.push('\n');
    }
    out
}

fn session_ua(sid: &str) -> Option<String> {
    sessions().lock().ok()?.0.get(sid).map(|s| s.ua.clone())
}

async fn fetch_playlist(ctx: &Ctx, sid: &str, url: &str) -> Response {
    let Ok(parsed) = url::Url::parse(url) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let ua = session_ua(sid).unwrap_or_else(|| http::BROWSER_UA.to_string());
    let Ok(resp) = http::client()
        .get(parsed)
        .header(reqwest::header::USER_AGENT, ua)
        .send()
        .await
    else {
        return status(StatusCode::BAD_GATEWAY);
    };
    if !resp.status().is_success() {
        return status(StatusCode::BAD_GATEWAY);
    }
    let final_url = resp.url().clone();
    let Ok(text) = resp.text().await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let prefix = format!("http://127.0.0.1:{}/yt/{sid}", ctx.port);
    let body = rewrite_hls(&text, &final_url, &prefix, &ctx.token);
    let mut r = Response::new(Body::from(body));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apple.mpegurl"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn hls_playlist(ctx: &Ctx, sid: &str) -> Response {
    let upstream = sessions()
        .lock()
        .ok()
        .and_then(|g| g.0.get(sid).and_then(|s| s.hls.clone()));
    match upstream {
        Some(u) => fetch_playlist(ctx, sid, &u).await,
        None => status(StatusCode::NOT_FOUND),
    }
}

async fn hls_part(
    ctx: &Ctx,
    sid: &str,
    q: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Response {
    let Some(u) = q.get("u").and_then(|u| url::Url::parse(u).ok()) else {
        return status(StatusCode::BAD_REQUEST);
    };
    if !youtube_host(&u) {
        return status(StatusCode::FORBIDDEN);
    }
    let path = u.path().to_ascii_lowercase();
    if path.ends_with(".m3u8") || path.contains("/playlist/") || path.contains("hls_variant") {
        return fetch_playlist(ctx, sid, u.as_str()).await;
    }
    let ua = session_ua(sid).unwrap_or_else(|| http::BROWSER_UA.to_string());
    let mut req = http::media_client()
        .get(u)
        .header(reqwest::header::USER_AGENT, ua);
    if let Some(r) = headers.get(header::RANGE) {
        req = req.header(reqwest::header::RANGE, r.as_bytes());
    }
    let Some(Ok(upstream)) = no_stall(req.send()).await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let code = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut out = HeaderMap::new();
    for key in [
        reqwest::header::CONTENT_TYPE,
        reqwest::header::CONTENT_LENGTH,
        reqwest::header::CONTENT_RANGE,
        reqwest::header::ACCEPT_RANGES,
    ] {
        if let Some(v) = upstream.headers().get(&key) {
            if let (Ok(n), Ok(val)) = (
                header::HeaderName::from_bytes(key.as_str().as_bytes()),
                HeaderValue::from_bytes(v.as_bytes()),
            ) {
                out.insert(n, val);
            }
        }
    }
    let stream = upstream
        .bytes_stream()
        .map(|c| c.map_err(std::io::Error::other));
    let mut r = Response::new(Body::from_stream(stream));
    *r.status_mut() = code;
    r.headers_mut().extend(out);
    r
}

async fn caption(
    State(ctx): State<Ctx>,
    Path((sid, file)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(i) = file.split('.').next().and_then(|s| s.parse::<usize>().ok()) else {
        return status(StatusCode::NOT_FOUND);
    };
    let src = sessions()
        .lock()
        .ok()
        .and_then(|g| g.0.get(&sid).and_then(|s| s.captions.get(i).cloned()));
    let Some(src) = src else {
        return status(StatusCode::NOT_FOUND);
    };
    let tlang = q.get("tlang").map(String::as_str).filter(|t| {
        !t.is_empty() && t.len() <= 16 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    let url = if src.raw {
        player::caption_vtt_url(&src.url, tlang)
    } else {
        src.url.clone()
    };
    if !url::Url::parse(&url)
        .map(|u| youtube_host(&u))
        .unwrap_or(false)
    {
        return status(StatusCode::FORBIDDEN);
    }
    match http::get_text(&url).await {
        Ok(text) => {
            let mut r = Response::new(Body::from(text));
            r.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/vtt; charset=utf-8"),
            );
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        Err(_) => status(StatusCode::BAD_GATEWAY),
    }
}

async fn storyboard(
    State(ctx): State<Ctx>,
    Path((sid, file)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(n) = file.split('.').next().and_then(|s| s.parse::<u32>().ok()) else {
        return status(StatusCode::NOT_FOUND);
    };
    let template = sessions()
        .lock()
        .ok()
        .and_then(|g| g.0.get(&sid).and_then(|s| s.storyboard.clone()));
    let Some(template) = template else {
        return status(StatusCode::NOT_FOUND);
    };
    let url = template.replace("$M", &n.to_string());
    if !url::Url::parse(&url)
        .map(|u| youtube_host(&u))
        .unwrap_or(false)
    {
        return status(StatusCode::FORBIDDEN);
    }
    let Ok(resp) = http::client().get(&url).send().await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    if !resp.status().is_success() {
        return status(StatusCode::NOT_FOUND);
    }
    let Ok(bytes) = resp.bytes().await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let mut r = Response::new(Body::from(bytes));
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    r
}
