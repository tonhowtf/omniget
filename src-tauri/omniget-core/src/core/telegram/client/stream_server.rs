use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use grammers_client::grammers_tl_types as tl;
use rand::RngExt;
use serde::Deserialize;
use tokio::sync::Mutex;

use super::auth::TelegramSessionHandle;
use super::parallel_download;

pub struct StreamServerInfo {
    pub port: u16,
    pub token: String,
}

#[derive(Clone)]
struct ServerState {
    session: TelegramSessionHandle,
    token: Arc<String>,
    port: u16,
}

#[derive(Deserialize)]
struct StreamQuery {
    token: Option<String>,
}

fn generate_token() -> String {
    let bytes: [u8; 24] = rand::rng().random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// `(start, end, is_range)`, or `None` when the range cannot be served
/// (416). A missing or malformed header means the whole file.
fn parse_range_header(header: Option<&HeaderValue>, total_size: u64) -> Option<(u64, u64, bool)> {
    let whole = Some((0, total_size.saturating_sub(1), false));
    let Some(value) = header.and_then(|v| v.to_str().ok()) else {
        return whole;
    };
    let Some(stripped) = value.trim().strip_prefix("bytes=") else {
        return whole;
    };
    let first = stripped.split(',').next().unwrap_or("").trim();
    let mut parts = first.splitn(2, '-');
    let start_str = parts.next().unwrap_or("").trim();
    let end_str = parts.next().unwrap_or("").trim();
    if total_size == 0 {
        return None;
    }
    let last = total_size - 1;
    let (start, end) = match (start_str.is_empty(), end_str.is_empty()) {
        (true, false) => {
            let suffix: u64 = end_str.parse().ok()?;
            if suffix == 0 {
                return None;
            }
            (total_size.saturating_sub(suffix), last)
        }
        (false, true) => (start_str.parse().ok()?, last),
        (false, false) => {
            let s: u64 = start_str.parse().ok()?;
            let e: u64 = end_str.parse().ok()?;
            (s, e.min(last))
        }
        (true, true) => return whole,
    };
    if start > last || start > end {
        return None;
    }
    Some((start, end, true))
}

/// A message's media, kept for a few minutes: a player sends many range
/// requests for one file and each would otherwise refetch the message.
#[derive(Clone)]
pub struct CachedMedia {
    pub media: tl::enums::MessageMedia,
    pub date: i32,
    pub file_name: String,
    pub file_size: u64,
    pub mime: String,
    pub media_type: String,
}

const MEDIA_TTL: Duration = Duration::from_secs(10 * 60);

type MediaCache = std::sync::Mutex<HashMap<(i64, i32), (CachedMedia, Instant)>>;

fn media_cache() -> &'static MediaCache {
    static CACHE: OnceLock<MediaCache> = OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub fn forget_media(chat_id: i64, message_id: i32) {
    if let Ok(mut c) = media_cache().lock() {
        c.remove(&(chat_id, message_id));
    }
}

/// Mime type from the file name, for media Telegram stores as
/// `application/octet-stream` (players refuse to guess).
pub fn mime_from_name(name: &str) -> Option<&'static str> {
    Some(match super::template::ext_of(name).as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "ts" => "video/mp2t",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

/// Fetches (or reuses) the media of one message.
pub async fn media_for(
    session: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
) -> anyhow::Result<(grammers_client::Client, CachedMedia)> {
    let (client, _, access_hash) = super::history::peer_for(session, chat_id, chat_type).await?;
    if let Ok(c) = media_cache().lock() {
        if let Some((m, at)) = c.get(&(chat_id, message_id)) {
            if at.elapsed() < MEDIA_TTL {
                return Ok((client, m.clone()));
            }
        }
    }
    let is_channel = chat_type == "channel" || (chat_type == "group" && access_hash != 0);
    let (media, date) =
        parallel_download::fetch_raw_media(&client, chat_id, access_hash, is_channel, message_id)
            .await?;
    let (file_name, file_size, media_type) =
        super::api::extract_raw_media_info(&media, super::fix_file_extensions())
            .ok_or_else(|| anyhow::anyhow!("Message {} has no file", message_id))?;
    let mut mime = parallel_download::mime_type_from_media(&media);
    if mime.is_empty() || mime == "application/octet-stream" {
        if let Some(m) = mime_from_name(&file_name) {
            mime = m.to_string();
        }
    }
    let cached = CachedMedia {
        media,
        date,
        file_name,
        file_size,
        mime,
        media_type,
    };
    if let Ok(mut c) = media_cache().lock() {
        c.retain(|_, (_, at)| at.elapsed() < MEDIA_TTL);
        c.insert((chat_id, message_id), (cached.clone(), Instant::now()));
    }
    Ok((client, cached))
}

fn plain(status: StatusCode, msg: impl Into<String>) -> Response {
    Response::builder()
        .status(status)
        .header("access-control-allow-origin", "*")
        .body(Body::from(msg.into()))
        .unwrap()
}

async fn handle_stream(
    State(state): State<ServerState>,
    method: Method,
    Path((chat_type, chat_id, message_id)): Path<(String, i64, i32)>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    serve(state, method, chat_type, chat_id, message_id, q, headers).await
}

/// Same as `handle_stream`; the trailing file name only helps external
/// players guess the format and show a title.
async fn handle_stream_named(
    State(state): State<ServerState>,
    method: Method,
    Path((chat_type, chat_id, message_id, _name)): Path<(String, i64, i32, String)>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    serve(state, method, chat_type, chat_id, message_id, q, headers).await
}

async fn serve(
    state: ServerState,
    method: Method,
    chat_type: String,
    chat_id: i64,
    message_id: i32,
    q: StreamQuery,
    headers: HeaderMap,
) -> Response {
    if q.token.as_deref() != Some(state.token.as_str()) {
        return plain(StatusCode::FORBIDDEN, "invalid token");
    }
    super::restore_session(&state.session).await;
    if state.session.lock().await.client.is_none() {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "not authenticated");
    }

    let (client, cached) = match media_for(&state.session, chat_id, &chat_type, message_id).await {
        Ok(t) => t,
        Err(e) => return plain(StatusCode::NOT_FOUND, format!("media: {}", e)),
    };
    let Some(media_loc) = parallel_download::media_to_location(&cached.media) else {
        return plain(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported media");
    };

    let mime = cached.mime.clone();
    let total_size = media_loc.size;
    let etag = format!("\"tg-{}-{}-{}\"", chat_id, message_id, total_size);

    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if if_none_match == etag {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .header(header::CACHE_CONTROL, "private, max-age=86400, immutable")
                .header("access-control-allow-origin", "*")
                .body(Body::empty())
                .unwrap();
        }
    }

    let range = parse_range_header(headers.get(header::RANGE), total_size);
    let (start, end, has_range) = match range {
        Some(r) => r,
        None => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", total_size))
                .header(header::ACCEPT_RANGES, "bytes")
                .header("access-control-allow-origin", "*")
                .body(Body::empty())
                .unwrap();
        }
    };
    let chunk_len = if total_size == 0 {
        0
    } else {
        end.saturating_sub(start).saturating_add(1)
    };

    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, &mime)
        .header(header::CONTENT_LENGTH, chunk_len.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, max-age=86400, immutable")
        .header(header::ETAG, &etag)
        .header(header::LAST_MODIFIED, "Thu, 01 Jan 1970 00:00:00 GMT")
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "inline; filename=\"{}\"",
                ascii_file_name(&cached.file_name)
            ),
        )
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
        .header("access-control-allow-headers", "*")
        .header(
            "access-control-expose-headers",
            "Content-Length, Content-Range, Accept-Ranges, Content-Type",
        );

    if has_range {
        builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{}", start, end, total_size),
        );
    } else {
        builder = builder.status(StatusCode::OK);
    }

    if method == Method::HEAD || chunk_len == 0 {
        return builder.body(Body::empty()).unwrap();
    }

    let inner = parallel_download::stream_file_range(client, media_loc, start, end);
    // A failed read usually means an expired file reference: drop the
    // cached message so the player's retry fetches a fresh one.
    let stream = async_stream::stream! {
        let mut inner = std::pin::pin!(inner);
        while let Some(chunk) = futures::StreamExt::next(&mut inner).await {
            if chunk.is_err() {
                forget_media(chat_id, message_id);
            }
            yield chunk;
        }
    };
    builder.body(Body::from_stream(stream)).unwrap()
}

/// A quoted-string-safe version of the name for `Content-Disposition`.
fn ascii_file_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

async fn handle_options() -> Response {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
        .header("access-control-allow-headers", "*")
        .body(Body::empty())
        .unwrap()
}

pub async fn start(session: TelegramSessionHandle) -> anyhow::Result<Arc<Mutex<StreamServerInfo>>> {
    let token = generate_token();
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let port = listener.local_addr()?.port();
    let state = ServerState {
        session,
        token: Arc::new(token.clone()),
        port,
    };

    let app = Router::new()
        .route(
            "/stream/{chat_type}/{chat_id}/{message_id}",
            get(handle_stream).options(handle_options),
        )
        .route(
            "/stream/{chat_type}/{chat_id}/{message_id}/{file_name}",
            get(handle_stream_named).options(handle_options),
        )
        .route(
            "/remux/{chat_type}/{chat_id}/{message_id}/index.m3u8",
            get(handle_remux).options(handle_options),
        )
        .route(
            "/hs/{job}/{file}",
            get(handle_hls_segment).options(handle_options),
        )
        .with_state(state);
    tracing::info!(
        "[tg-stream] streaming server listening on 127.0.0.1:{}",
        port
    );

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("[tg-stream] server error: {}", e);
        }
    });

    tokio::spawn(async move {
        let tracker = super::parallel_download::global_stream_tracker().clone();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let rate = tracker.current_rate_bytes_per_sec().await;
            if rate > 0 {
                tracing::debug!(
                    "[tg-stream] speed={:.2} MB/s ({} B/s)",
                    rate as f64 / (1024.0 * 1024.0),
                    rate
                );
            }
        }
    });

    Ok(Arc::new(Mutex::new(StreamServerInfo { port, token })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(v: &str, total: u64) -> Option<(u64, u64, bool)> {
        parse_range_header(Some(&HeaderValue::from_str(v).unwrap()), total)
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range_header(None, 100), Some((0, 99, false)));
        assert_eq!(r("bytes=0-", 100), Some((0, 99, true)));
        assert_eq!(r("bytes=0-1", 100), Some((0, 1, true)));
        assert_eq!(r("bytes=90-200", 100), Some((90, 99, true)));
        assert_eq!(r("bytes=-10", 100), Some((90, 99, true)));
        assert_eq!(r("bytes=100-", 100), None);
        assert_eq!(r("bytes=5-2", 100), None);
    }
}

// ---- remux to HLS (MKV/AVI/WebM in the WKWebView) ---------------------------
//
// ffmpeg reads the file from this same server (`/stream/...`, with Range)
// and writes an HLS EVENT playlist with fMP4 segments to
// `data_dir/hls/<job>/`, the way /video does for local files. WebKit plays
// HLS natively; the playlist grows while ffmpeg works.

#[derive(Deserialize)]
struct RemuxQuery {
    token: Option<String>,
    /// `copy` (video and audio copied), `aac` (video copied, audio to AAC;
    /// the default) or `transcode` (H.264 + AAC).
    mode: Option<String>,
    ss: Option<f64>,
}

struct HlsJob {
    key: String,
    dir: std::path::PathBuf,
    child: tokio::process::Child,
}

fn hls_jobs() -> &'static Mutex<HashMap<(i64, i32), HlsJob>> {
    static J: OnceLock<Mutex<HashMap<(i64, i32), HlsJob>>> = OnceLock::new();
    J.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hls_root() -> std::path::PathBuf {
    static CLEARED: std::sync::Once = std::sync::Once::new();
    let d = crate::core::telegram::host::data_dir().join("hls");
    // Jobs never survive a restart: what is left is useless.
    CLEARED.call_once(|| {
        let _ = std::fs::remove_dir_all(&d);
    });
    let _ = std::fs::create_dir_all(&d);
    d
}

fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !s.contains("..")
}

/// Video codec of the input (`hevc`, `h264`, …), to tag HEVC as `hvc1`.
async fn video_codec(input: &str) -> String {
    let out = crate::core::process::command("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(input)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(20), out).await {
        Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

fn ffmpeg_args(
    input: &str,
    vcodec: &str,
    mode: &str,
    ss: f64,
    dir: &std::path::Path,
) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if ss > 0.0 {
        a.extend(["-ss".into(), format!("{ss:.3}")]);
    }
    a.extend([
        "-fflags".into(),
        "+genpts".into(),
        "-i".into(),
        input.into(),
    ]);
    a.extend([
        "-map".into(),
        "0:v:0?".into(),
        "-map".into(),
        "0:a:0?".into(),
        "-sn".into(),
        "-dn".into(),
        "-map_metadata".into(),
        "-1".into(),
    ]);
    if mode == "transcode" {
        a.extend([
            "-vf".into(),
            "scale=-2:'min(1080,ih)',format=yuv420p".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-crf".into(),
            "21".into(),
            "-g".into(),
            "60".into(),
        ]);
    } else {
        a.extend(["-c:v".into(), "copy".into()]);
        if vcodec == "hevc" {
            a.extend(["-tag:v".into(), "hvc1".into()]);
        }
    }
    if mode == "copy" {
        a.extend(["-c:a".into(), "copy".into()]);
    } else {
        a.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-ac".into(),
            "2".into(),
        ]);
    }
    a.extend([
        "-strict".into(),
        "-2".into(),
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
    ]);
    a
}

/// Waits until the playlist lists a segment. `None` when ffmpeg exits
/// first (unreadable input, codec the container can't take).
async fn wait_playlist(job: &mut HlsJob) -> Option<String> {
    let pl = job.dir.join("index.m3u8");
    for _ in 0..900 {
        if let Ok(text) = tokio::fs::read_to_string(&pl).await {
            if text.contains("#EXTINF") {
                return Some(text);
            }
        }
        if let Ok(Some(st)) = job.child.try_wait() {
            if st.success() {
                return tokio::fs::read_to_string(&pl).await.ok();
            }
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

fn rewrite_playlist(text: &str, job: &str, token: &str) -> String {
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
            out.push_str(&format!(
                "#EXT-X-MAP:URI=\"/hs/{job}/{name}?token={token}\"\n"
            ));
            continue;
        }
        if t.starts_with('#') {
            out.push_str(t);
        } else {
            let name = t.rsplit('/').next().unwrap_or(t);
            out.push_str(&format!("/hs/{job}/{name}?token={token}"));
        }
        out.push('\n');
    }
    out
}

async fn handle_remux(
    State(state): State<ServerState>,
    Path((chat_type, chat_id, message_id)): Path<(String, i64, i32)>,
    Query(q): Query<RemuxQuery>,
) -> Response {
    if q.token.as_deref() != Some(state.token.as_str()) {
        return plain(StatusCode::FORBIDDEN, "invalid token");
    }
    if !safe_name(&chat_type) {
        return plain(StatusCode::BAD_REQUEST, "bad chat type");
    }
    let mode = match q.mode.as_deref() {
        Some("copy") => "copy",
        Some("transcode") => "transcode",
        _ => "aac",
    };
    let ss = q.ss.unwrap_or(0.0).max(0.0);
    let key = format!(
        "{}-{}-{}-{}",
        chat_id,
        message_id,
        mode,
        (ss * 1000.0).round() as i64
    );
    let input = format!(
        "http://127.0.0.1:{}/stream/{}/{}/{}?token={}",
        state.port, chat_type, chat_id, message_id, state.token
    );
    let mut jobs = hls_jobs().lock().await;
    let slot = (chat_id, message_id);
    let reuse = jobs.get(&slot).map(|j| j.key == key).unwrap_or(false);
    if !reuse {
        if let Some(mut old) = jobs.remove(&slot) {
            let _ = old.child.kill().await;
            let _ = tokio::fs::remove_dir_all(&old.dir).await;
        }
        // One conversion at a time across the app: a new video stops the
        // previous one.
        let others: Vec<(i64, i32)> = jobs.keys().copied().collect();
        for k in others {
            if let Some(mut old) = jobs.remove(&k) {
                let _ = old.child.kill().await;
                let _ = tokio::fs::remove_dir_all(&old.dir).await;
            }
        }
        let dir = hls_root().join(&key);
        let _ = tokio::fs::remove_dir_all(&dir).await;
        if tokio::fs::create_dir_all(&dir).await.is_err() {
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "no cache folder");
        }
        let vcodec = if mode == "transcode" {
            String::new()
        } else {
            video_codec(&input).await
        };
        let child = crate::core::process::command("ffmpeg")
            .args(ffmpeg_args(&input, &vcodec, mode, ss, &dir))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let Ok(child) = child else {
            return plain(StatusCode::UNPROCESSABLE_ENTITY, "ffmpeg is not available");
        };
        let mut job = HlsJob {
            key: key.clone(),
            dir,
            child,
        };
        if wait_playlist(&mut job).await.is_none() {
            let _ = job.child.kill().await;
            return plain(
                StatusCode::UNPROCESSABLE_ENTITY,
                "conversion failed; try mode=transcode",
            );
        }
        jobs.insert(slot, job);
    }
    let Some(job) = jobs.get_mut(&slot) else {
        return plain(StatusCode::UNPROCESSABLE_ENTITY, "conversion failed");
    };
    let Some(text) = wait_playlist(job).await else {
        return plain(StatusCode::UNPROCESSABLE_ENTITY, "conversion failed");
    };
    let body = rewrite_playlist(&text, &key, &state.token);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-store")
        .header("access-control-allow-origin", "*")
        .body(Body::from(body))
        .unwrap()
}

async fn handle_hls_segment(
    State(state): State<ServerState>,
    Path((job, file)): Path<(String, String)>,
    Query(q): Query<StreamQuery>,
) -> Response {
    if q.token.as_deref() != Some(state.token.as_str()) {
        return plain(StatusCode::FORBIDDEN, "invalid token");
    }
    if !safe_name(&job) || !safe_name(&file) {
        return plain(StatusCode::BAD_REQUEST, "bad name");
    }
    let p = hls_root().join(&job).join(&file);
    // A player can ask for the next segment a moment before ffmpeg renames it.
    for _ in 0..100 {
        if tokio::fs::try_exists(&p).await.unwrap_or(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    match tokio::fs::read(&p).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .header(header::CONTENT_LENGTH, bytes.len().to_string())
            .header(header::CACHE_CONTROL, "no-store")
            .header("access-control-allow-origin", "*")
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => plain(StatusCode::NOT_FOUND, "segment not found"),
    }
}

/// Containers the WKWebView `<video>` can't open (they play after the
/// remux to HLS).
pub fn needs_remux(file_name: &str, mime: &str) -> bool {
    let ext = super::template::ext_of(file_name);
    matches!(
        ext.as_str(),
        "mkv"
            | "avi"
            | "webm"
            | "flv"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "ts"
            | "m2ts"
            | "ogv"
            | "divx"
            | "rmvb"
    ) || matches!(
        mime,
        "video/x-matroska" | "video/x-msvideo" | "video/webm" | "video/x-flv" | "video/x-ms-wmv"
    )
}
