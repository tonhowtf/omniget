//! Localhost HTTP server the player's `<video>` reads from.
//!
//! Same design as `/music`: 127.0.0.1 on a random port, a per-launch token
//! (`k=`) on every request, and the `Host` header must be 127.0.0.1 or
//! localhost so a web page can't reach it through DNS rebinding.
//!
//! * `GET /f/{id}`               the file itself, with Range
//! * `GET /m/{id}/index.m3u8?ss=&ai=`  remux to HLS (fMP4 segments, `-c copy`)
//! * `GET /a/{id}/index.m3u8?ss=&ai=`  same, audio to AAC
//! * `GET /t/{id}/index.m3u8?ss=&ai=`  full transcode to H.264 + AAC
//! * `GET /hs/{job}/{file}`      segments of those playlists
//!
//! The ffmpeg tiers write HLS to disk instead of piping MP4: WebKit only
//! plays progressive MP4 when it can ask for byte ranges, and a pipe has no
//! length. HLS plays natively in WKWebView and through hls.js elsewhere.
//! * `GET /s/{id}/{stream}.vtt`  embedded text subtitle as WebVTT (`o=` shifts, ms)
//! * `GET /x/{id}.vtt?p=&o=`     external subtitle next to the video as WebVTT
//! * `GET /thumb/{id}?t=`        480px JPEG (10 % in, or at `t` ms), cached
//! * `GET /frame/{id}?t=`        full-size JPEG of one frame
//! * `GET /img/{file}`           frames saved with notes
//! * `GET /r/{rid}?u=`           remote proxy with the headers given to
//!   `register_remote`; HLS playlists are rewritten to come back here
//!
//! `ss` is in seconds; the stream then starts there and the player adds the
//! offset to `currentTime`.

use std::collections::{HashMap, VecDeque};
use std::io::SeekFrom;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{OnceCell, Semaphore};
use tower_http::cors::{Any, CorsLayer};

use omniget_core::core::music::playlist_file;
use omniget_core::core::video::subs;

use super::{db, probe};

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
                .route("/f/{id}", get(file))
                .route("/m/{id}/index.m3u8", get(remux))
                .route("/a/{id}/index.m3u8", get(audio_fix))
                .route("/t/{id}/index.m3u8", get(transcode))
                .route("/hs/{job}/{file}", get(hls_segment))
                .route("/s/{id}/{stream}", get(embedded_sub))
                .route("/x/{id}", get(external_sub))
                .route("/thumb/{id}", get(thumb))
                .route("/frame/{id}", get(frame))
                .route("/img/{file}", get(img))
                .route("/r/{rid}", get(remote))
                .layer(
                    CorsLayer::new()
                        .allow_origin(Any)
                        .allow_methods(Any)
                        .allow_headers(Any)
                        .expose_headers(Any),
                )
                .with_state(ctx);
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, router).await {
                    tracing::error!("[video] server stopped: {e}");
                }
            });
            tracing::info!("[video] server on 127.0.0.1:{port}");
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

/// Absolute URL for a server path, token included. Returns the path as is
/// when the server is not running yet (call `ensure` first).
pub fn url(path: &str) -> String {
    let Some(info) = SERVER.get() else {
        return path.to_string();
    };
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{}{path}{sep}k={}", base(info), info.token)
}

// ---------- remote proxy registry ----------

#[derive(Clone)]
struct Remote {
    url: String,
    headers: Vec<(String, String)>,
}

const REMOTE_CAP: usize = 4000;

/// Registered remotes and their insertion order (oldest evicted first).
type RemoteTable = (HashMap<String, Remote>, VecDeque<String>);

fn remotes() -> &'static Mutex<RemoteTable> {
    static R: OnceLock<Mutex<RemoteTable>> = OnceLock::new();
    R.get_or_init(|| Mutex::new((HashMap::new(), VecDeque::new())))
}

/// Registers a remote URL and the headers it needs (User-Agent, Referer,
/// cookies...) and returns the absolute proxy URL `/r/<rid>`. Range is passed
/// through, and HLS playlists are rewritten so every variant, segment and key
/// also goes through the proxy with the same headers.
pub fn register_remote(url: &str, headers: Vec<(String, String)>) -> String {
    let Ok(mut g) = remotes().lock() else {
        return url.to_string();
    };
    let (map, order) = &mut *g;
    if let Some((rid, _)) = map
        .iter()
        .find(|(_, r)| r.url == url && r.headers == headers)
    {
        return self::url(&format!("/r/{rid}"));
    }
    let rid = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
    map.insert(
        rid.clone(),
        Remote {
            url: url.to_string(),
            headers,
        },
    );
    order.push_back(rid.clone());
    while order.len() > REMOTE_CAP {
        if let Some(old) = order.pop_front() {
            map.remove(&old);
        }
    }
    self::url(&format!("/r/{rid}"))
}

fn remote_get(rid: &str) -> Option<Remote> {
    remotes().lock().ok()?.0.get(rid).cloned()
}

// ---------- guards and helpers ----------

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

fn with_type(mut r: Response, mime: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(mime) {
        r.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    r
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n`.
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

async fn serve_file(path: &str, mime: &str, headers: &HeaderMap) -> Response {
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return status(StatusCode::NOT_FOUND);
    };
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(_) => return status(StatusCode::NOT_FOUND),
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| parse_range(h, len));
    if headers.contains_key(header::RANGE) && range.is_none() && len > 0 {
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
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let count = if len == 0 { 0 } else { end - start + 1 };
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file.take(count)));
    let mut resp = Response::new(body);
    *resp.status_mut() = code;
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
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

fn q_f64(q: &HashMap<String, String>, k: &str) -> f64 {
    q.get(k)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(0.0)
}

fn q_i64(q: &HashMap<String, String>, k: &str) -> i64 {
    q.get(k).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0)
}

/// Strips a trailing `.vtt`/`.jpg` so `/s/3/2.vtt` and `/x/3.vtt` parse.
fn id_of(seg: &str) -> Option<i64> {
    seg.split('.').next()?.parse().ok()
}

// ---------- local files ----------

async fn file(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some((path, ..)) = id_of(&id).and_then(db::media_file) else {
        return status(StatusCode::NOT_FOUND);
    };
    serve_file(&path, probe::video_mime(&path), &headers).await
}

// ---------- ffmpeg streaming (remux / transcode) ----------

#[derive(Clone, Copy, PartialEq)]
enum Pipe {
    Remux,
    AudioFix,
    Transcode,
}

/// macOS: set once VideoToolbox failed, so later transcodes go straight to
/// libx264.
static VT_BROKEN: AtomicBool = AtomicBool::new(false);

fn ffmpeg_args(
    path: &str,
    vcodec: &str,
    ss: f64,
    ai: i64,
    pipe: Pipe,
    encoder: &str,
) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if ss > 0.0 {
        a.extend(["-ss".into(), format!("{ss:.3}")]);
    }
    a.extend(["-fflags".into(), "+genpts".into(), "-i".into(), path.into()]);
    a.extend([
        "-map".into(),
        "0:v:0?".into(),
        "-map".into(),
        format!("0:a:{}?", ai.max(0)),
        "-sn".into(),
        "-dn".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-map_chapters".into(),
        "-1".into(),
    ]);
    match pipe {
        Pipe::Remux | Pipe::AudioFix => {
            a.extend(["-c:v".into(), "copy".into()]);
            if vcodec == "hevc" {
                // WebKit plays HEVC only when tagged hvc1
                a.extend(["-tag:v".into(), "hvc1".into()]);
            }
        }
        Pipe::Transcode => {
            a.extend([
                "-vf".into(),
                "scale=-2:'min(1080,ih)',format=yuv420p".into(),
                "-c:v".into(),
                encoder.into(),
            ]);
            if encoder == "h264_videotoolbox" {
                a.extend([
                    "-b:v".into(),
                    "8M".into(),
                    "-maxrate".into(),
                    "10M".into(),
                    "-allow_sw".into(),
                    "1".into(),
                    "-realtime".into(),
                    "1".into(),
                ]);
            } else {
                a.extend([
                    "-preset".into(),
                    "veryfast".into(),
                    "-crf".into(),
                    "21".into(),
                    "-profile:v".into(),
                    "high".into(),
                ]);
            }
            a.extend(["-g".into(), "60".into()]);
        }
    }
    match pipe {
        Pipe::Remux => a.extend(["-c:a".into(), "copy".into()]),
        Pipe::AudioFix | Pipe::Transcode => a.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-ac".into(),
            "2".into(),
        ]),
    }
    a.extend(["-strict".into(), "-2".into()]);
    a
}

/// HLS output for the ffmpeg tiers: an EVENT playlist that grows while
/// ffmpeg works (remux runs far ahead of playback), fMP4 segments so HEVC
/// and AC-3 copies stay valid.
fn hls_output(dir: &FsPath) -> Vec<String> {
    vec![
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        "4".into(),
        "-hls_list_size".into(),
        "0".into(),
        "-hls_playlist_type".into(),
        "event".into(),
        "-hls_segment_type".into(),
        "fmp4".into(),
        "-hls_flags".into(),
        "independent_segments+temp_file".into(),
        "-hls_fmp4_init_filename".into(),
        "init.mp4".into(),
        "-hls_segment_filename".into(),
        dir.join("seg%05d.m4s").to_string_lossy().to_string(),
        dir.join("index.m3u8").to_string_lossy().to_string(),
    ]
}

struct HlsJob {
    key: String,
    dir: PathBuf,
    child: tokio::process::Child,
}

/// One running ffmpeg per media id: a seek or a tier change replaces it.
fn hls_jobs() -> &'static tokio::sync::Mutex<HashMap<i64, HlsJob>> {
    static J: OnceLock<tokio::sync::Mutex<HashMap<i64, HlsJob>>> = OnceLock::new();
    J.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

fn hls_root() -> Option<PathBuf> {
    static CLEARED: std::sync::Once = std::sync::Once::new();
    let d = db::cache_dir("hls")?;
    // leftovers from a previous run are useless: jobs never survive a restart
    CLEARED.call_once(|| {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    });
    Some(d)
}

/// Waits until the playlist lists at least one segment. `None` when ffmpeg
/// exits first (bad encoder, unreadable file).
async fn wait_playlist(job: &mut HlsJob) -> Option<String> {
    let pl = job.dir.join("index.m3u8");
    for _ in 0..600 {
        if let Ok(text) = tokio::fs::read_to_string(&pl).await {
            if text.contains("#EXTINF") {
                return Some(text);
            }
        }
        if let Ok(Some(st)) = job.child.try_wait() {
            // it may have finished a very short file in one go
            if st.success() {
                return tokio::fs::read_to_string(&pl).await.ok();
            }
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

fn rewrite_local_playlist(text: &str, job: &str, token: &str) -> String {
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t == "#EXTM3U" {
            out.push_str("#EXTM3U\n#EXT-X-START:TIME-OFFSET=0,PRECISE=YES\n");
            continue;
        }
        if let Some(rest) = t.strip_prefix("#EXT-X-MAP:URI=\"") {
            let name = rest.split('"').next().unwrap_or("init.mp4");
            out.push_str(&format!("#EXT-X-MAP:URI=\"/hs/{job}/{name}?k={token}\"\n"));
            continue;
        }
        if t.starts_with('#') {
            out.push_str(t);
        } else {
            let name = t.rsplit('/').next().unwrap_or(t);
            out.push_str(&format!("/hs/{job}/{name}?k={token}"));
        }
        out.push('\n');
    }
    out
}

async fn spawn_hls(args: Vec<String>, dir: &FsPath) -> Option<tokio::process::Child> {
    let _ = tokio::fs::remove_dir_all(dir).await;
    tokio::fs::create_dir_all(dir).await.ok()?;
    let mut cmd = omniget_core::core::process::command("ffmpeg");
    cmd.args(&args)
        .args(hls_output(dir))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    cmd.spawn().ok()
}

pub fn encoders() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(target_os = "macos") && !VT_BROKEN.load(Ordering::SeqCst) {
        v.push("h264_videotoolbox");
    }
    v.push("libx264");
    v
}

async fn pipe_route(
    ctx: Ctx,
    id: String,
    q: HashMap<String, String>,
    headers: HeaderMap,
    pipe: Pipe,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(mid) = id_of(&id) else {
        return status(StatusCode::NOT_FOUND);
    };
    let Some((path, _, vcodec, _)) = db::media_file(mid) else {
        return status(StatusCode::NOT_FOUND);
    };
    let Some(root) = hls_root() else {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let ss = q_f64(&q, "ss");
    let ai = q_i64(&q, "ai");
    let tag = match pipe {
        Pipe::Remux => "m",
        Pipe::AudioFix => "a",
        Pipe::Transcode => "t",
    };
    let key = format!("{mid}-{tag}-{ai}-{}", (ss * 1000.0).round() as i64);
    let mut jobs = hls_jobs().lock().await;
    let reuse = jobs.get(&mid).map(|j| j.key == key).unwrap_or(false);
    if !reuse {
        if let Some(mut old) = jobs.remove(&mid) {
            let _ = old.child.kill().await;
            let _ = tokio::fs::remove_dir_all(&old.dir).await;
        }
        let dir = root.join(&key);
        let encs: Vec<&str> = if pipe == Pipe::Transcode {
            encoders()
        } else {
            vec![""]
        };
        let mut started = None;
        for enc in encs {
            let args = ffmpeg_args(&path, &vcodec, ss, ai, pipe, enc);
            let Some(child) = spawn_hls(args, &dir).await else {
                continue;
            };
            let mut job = HlsJob {
                key: key.clone(),
                dir: dir.clone(),
                child,
            };
            if wait_playlist(&mut job).await.is_some() {
                started = Some(job);
                break;
            }
            let _ = job.child.kill().await;
            if enc == "h264_videotoolbox" {
                tracing::warn!("[video] videotoolbox failed, using libx264");
                VT_BROKEN.store(true, Ordering::SeqCst);
            }
        }
        match started {
            Some(job) => {
                jobs.insert(mid, job);
            }
            None => return status(StatusCode::UNPROCESSABLE_ENTITY),
        }
    }
    let Some(job) = jobs.get_mut(&mid) else {
        return status(StatusCode::UNPROCESSABLE_ENTITY);
    };
    let Some(text) = wait_playlist(job).await else {
        return status(StatusCode::UNPROCESSABLE_ENTITY);
    };
    let body = rewrite_local_playlist(&text, &key, &ctx.token);
    let mut r = Response::new(Body::from(body));
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apple.mpegurl"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn hls_segment(
    State(ctx): State<Ctx>,
    Path((job, file)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    if !safe_file_name(&job) || !safe_file_name(&file) {
        return status(StatusCode::BAD_REQUEST);
    }
    let Some(root) = hls_root() else {
        return status(StatusCode::NOT_FOUND);
    };
    let p = root.join(&job).join(&file);
    // a player can ask for the next segment a moment before ffmpeg renames it
    for _ in 0..100 {
        if tokio::fs::try_exists(&p).await.unwrap_or(false) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mut r = serve_file(&p.to_string_lossy(), "video/mp4", &headers).await;
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn remux(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    pipe_route(ctx, id, q, headers, Pipe::Remux).await
}

async fn audio_fix(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    pipe_route(ctx, id, q, headers, Pipe::AudioFix).await
}

async fn transcode(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    pipe_route(ctx, id, q, headers, Pipe::Transcode).await
}

// ---------- subtitles ----------

fn vtt_response(text: String) -> Response {
    let mut r = Response::new(Body::from(text));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/vtt; charset=utf-8"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// Extracts one embedded text subtitle stream to a cached WebVTT file.
pub async fn embedded_vtt_file(path: &str, hash: &str, stream: i64) -> Option<PathBuf> {
    let dir = db::cache_dir("subs")?;
    let out = dir.join(format!("{hash}-{stream}.vtt"));
    if out.exists() {
        return Some(out);
    }
    let tmp = dir.join(format!("{hash}-{stream}.{}.tmp", std::process::id()));
    let ok = omniget_core::core::process::command("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(path)
        .args(["-map", &format!("0:{stream}"), "-f", "webvtt"])
        .arg(&tmp)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    std::fs::rename(&tmp, &out).ok()?;
    Some(out)
}

fn shifted_vtt(bytes: &[u8], offset_ms: i64) -> String {
    let cues = subs::parse_auto(bytes);
    subs::to_vtt(&cues, offset_ms)
}

async fn embedded_sub(
    State(ctx): State<Ctx>,
    Path((id, stream)): Path<(i64, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let (Some((path, hash, ..)), Some(stream)) = (db::media_file(id), id_of(&stream)) else {
        return status(StatusCode::NOT_FOUND);
    };
    let Some(vtt) = embedded_vtt_file(&path, &hash, stream).await else {
        return status(StatusCode::UNPROCESSABLE_ENTITY);
    };
    let Ok(bytes) = tokio::fs::read(&vtt).await else {
        return status(StatusCode::NOT_FOUND);
    };
    let o = q_i64(&q, "o");
    if o == 0 {
        return vtt_response(String::from_utf8_lossy(&bytes).into_owned());
    }
    vtt_response(shifted_vtt(&bytes, o))
}

/// True when `sub` sits in the same folder as `video` (no path tricks).
pub fn same_folder(video: &str, sub: &str) -> bool {
    let (Ok(v), Ok(s)) = (std::fs::canonicalize(video), std::fs::canonicalize(sub)) else {
        return false;
    };
    v.parent().is_some() && v.parent() == s.parent()
}

async fn external_sub(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let (Some((path, ..)), Some(sub)) = (id_of(&id).and_then(db::media_file), q.get("p")) else {
        return status(StatusCode::NOT_FOUND);
    };
    if !same_folder(&path, sub)
        || !probe::SUB_EXTS.contains(&probe::ext_of(FsPath::new(sub)).as_str())
    {
        return status(StatusCode::FORBIDDEN);
    }
    let Ok(bytes) = tokio::fs::read(sub).await else {
        return status(StatusCode::NOT_FOUND);
    };
    vtt_response(shifted_vtt(&bytes, q_i64(&q, "o")))
}

// ---------- images ----------

fn thumb_gate() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(3))
}

async fn ffmpeg_frame(path: &str, at_ms: i64, width: Option<u32>) -> Option<Vec<u8>> {
    let mut cmd = omniget_core::core::process::command("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    if at_ms > 0 {
        cmd.args(["-ss", &format!("{:.3}", at_ms as f64 / 1000.0)]);
    }
    cmd.arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-an", "-sn"]);
    if let Some(w) = width {
        cmd.args(["-vf", &format!("scale={w}:-2")]);
    }
    let out = cmd
        .args(["-q:v", "3", "-f", "image2pipe", "-c:v", "mjpeg", "pipe:1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// Captures one full-size frame to a file (notes).
pub async fn capture_frame(source: &str, at_ms: i64, out: &FsPath) -> Result<(), String> {
    let bytes = ffmpeg_frame(source, at_ms, None)
        .await
        .ok_or("could not capture the frame")?;
    tokio::fs::write(out, bytes)
        .await
        .map_err(|e| e.to_string())
}

fn jpeg(bytes: Vec<u8>, cache: bool) -> Response {
    let mut r = Response::new(Body::from(bytes));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if cache {
            "private, max-age=86400"
        } else {
            "no-store"
        }),
    );
    r
}

async fn thumb(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some((path, hash, _, dur)) = id_of(&id).and_then(db::media_file) else {
        return status(StatusCode::NOT_FOUND);
    };
    let Some(dir) = db::cache_dir("thumbs") else {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let t = q_i64(&q, "t");
    let name = if t > 0 {
        format!("{hash}-{t}")
    } else {
        hash.clone()
    };
    let file = dir.join(format!("{name}.jpg"));
    let miss = dir.join(format!("{name}.none"));
    if let Ok(b) = tokio::fs::read(&file).await {
        return jpeg(b, true);
    }
    if miss.exists() {
        return status(StatusCode::NOT_FOUND);
    }
    let Ok(_permit) = thumb_gate().acquire().await else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    // another request may have made it while this one waited
    if let Ok(b) = tokio::fs::read(&file).await {
        return jpeg(b, true);
    }
    let at = if t > 0 {
        t
    } else if dur > 0 {
        dur / 10
    } else {
        5_000
    };
    let bytes = match ffmpeg_frame(&path, at, Some(480)).await {
        Some(b) => Some(b),
        // too close to the end or a broken index: try the first frame
        None => ffmpeg_frame(&path, 0, Some(480)).await,
    };
    match bytes {
        Some(b) => {
            let _ = tokio::fs::write(&file, &b).await;
            jpeg(b, true)
        }
        None => {
            // only remember the failure when the file is really there
            if FsPath::new(&path).exists() {
                let _ = tokio::fs::write(&miss, b"").await;
            }
            status(StatusCode::NOT_FOUND)
        }
    }
}

async fn frame(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some((path, ..)) = id_of(&id).and_then(db::media_file) else {
        return status(StatusCode::NOT_FOUND);
    };
    match ffmpeg_frame(&path, q_i64(&q, "t"), None).await {
        Some(b) => jpeg(b, false),
        None => status(StatusCode::UNPROCESSABLE_ENTITY),
    }
}

pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

async fn img(
    State(ctx): State<Ctx>,
    Path(name): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let (true, Some(dir)) = (safe_file_name(&name), db::img_dir()) else {
        return status(StatusCode::NOT_FOUND);
    };
    let p = dir.join(&name).to_string_lossy().to_string();
    let mime = match probe::ext_of(FsPath::new(&name)).as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };
    serve_file(&p, mime, &headers).await
}

// ---------- remote proxy ----------

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .use_rustls_tls()
            .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(12))
            .build()
            .unwrap_or_default()
    })
}

/// Rewrites every URI of an HLS playlist to `/r/<rid>?u=<absolute>` so the
/// player fetches segments, keys and variants through the proxy too.
fn rewrite_hls(text: &str, final_url: &url::Url, rid: &str) -> String {
    let proxied = |u: &str| -> String {
        let abs = final_url
            .join(u.trim())
            .map(|x| x.to_string())
            .unwrap_or_else(|_| u.to_string());
        url(&format!("/r/{rid}?u={}", urlencoding::encode(&abs)))
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
                if inner.starts_with("data:") || inner.starts_with("skd:") {
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

async fn remote(
    State(ctx): State<Ctx>,
    Path(rid): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !allowed(&ctx, &q, &headers) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(entry) = remote_get(&rid) else {
        return status(StatusCode::NOT_FOUND);
    };
    let target = q.get("u").cloned().unwrap_or_else(|| entry.url.clone());
    let Ok(target) = url::Url::parse(&target) else {
        return status(StatusCode::BAD_REQUEST);
    };
    if !matches!(target.scheme(), "http" | "https") {
        return status(StatusCode::BAD_REQUEST);
    }
    let mut req = client().get(target.clone());
    for (k, v) in &entry.headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            req = req.header(name, val);
        }
    }
    if let Some(r) = headers.get(header::RANGE) {
        req = req.header(reqwest::header::RANGE, r.as_bytes());
    }
    let upstream = match req.send().await {
        Ok(r) => r,
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let code = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let final_url = upstream.url().clone();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let small = upstream
        .content_length()
        .map(|l| l < 4 * 1024 * 1024)
        .unwrap_or(true);
    if small && playlist_file::looks_like_hls(&content_type, final_url.as_str(), b"") {
        let Ok(body) = upstream.bytes().await else {
            return status(StatusCode::BAD_GATEWAY);
        };
        if playlist_file::looks_like_hls(&content_type, final_url.as_str(), &body)
            && body.starts_with(b"#EXTM3U")
        {
            let text = String::from_utf8_lossy(&body);
            let mut r = Response::new(Body::from(rewrite_hls(&text, &final_url, &rid)));
            *r.status_mut() = code;
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            return with_type(r, "application/vnd.apple.mpegurl");
        }
        let mut r = Response::new(Body::from(body));
        *r.status_mut() = code;
        return with_type(r, &content_type);
    }
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
