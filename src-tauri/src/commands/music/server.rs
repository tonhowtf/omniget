//! Localhost HTTP server the player's `<audio>` elements read from.
//!
//! The webview can't open arbitrary local files, can't read Icecast titles
//! and can't route non-CORS radio through Web Audio (it would go silent), so
//! every source goes through here. Bound to 127.0.0.1 on a random port; every
//! request must carry the per-launch token (`k=`), so other local programs
//! and web pages can't use it to read files.
//!
//! * `GET /t/<id>`          track file, with Range (seeking)
//! * `GET /x/<id>?ss=<s>`   same track transcoded to MP3 by ffmpeg, for
//!   formats the webview can't decode (APE, WMA, ...)
//! * `GET /c/<hash>?t=1`    cover art (thumbnail with `t=1`)
//! * `GET /r?u=<url>&s=<id>` radio proxy: strips ICY metadata, emits titles
//! * `GET /p?u=<url>`       plain passthrough with CORS (HLS playlists/segments)

use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::OnceCell;
use tower_http::cors::{Any, CorsLayer};

use omniget_core::core::music::icy::{self, IcyDemuxer};
use omniget_core::core::music::playlist_file;

use super::{db, scan};

#[derive(Clone, Serialize)]
pub struct ServerInfo {
    pub port: u16,
    pub token: String,
}

#[derive(Clone)]
struct Ctx {
    app: AppHandle,
    token: Arc<String>,
}

static SERVER: OnceCell<ServerInfo> = OnceCell::const_new();

pub async fn ensure(app: &AppHandle) -> Result<ServerInfo, String> {
    SERVER
        .get_or_try_init(|| async {
            let token = crate::local_bridge::generate_token();
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| e.to_string())?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            let ctx = Ctx {
                app: app.clone(),
                token: Arc::new(token.clone()),
            };
            let router = Router::new()
                .route("/t/{id}", get(track))
                .route("/x/{id}", get(transcode))
                .route("/c/{hash}", get(cover))
                .route("/r", get(radio))
                .route("/p", get(passthrough))
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
                    tracing::error!("[music] server stopped: {e}");
                }
            });
            tracing::info!("[music] server on 127.0.0.1:{port}");
            Ok(ServerInfo { port, token })
        })
        .await
        .cloned()
}

fn authorized(ctx: &Ctx, q: &HashMap<String, String>) -> bool {
    q.get("k")
        .map(|k| constant_time_eq::constant_time_eq(k.as_bytes(), ctx.token.as_bytes()))
        .unwrap_or(false)
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

pub fn audio_mime(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "m4b" | "mp4" | "alac" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" | "spx" => "audio/ogg",
        "webm" => "audio/webm",
        "wav" | "wave" => "audio/wav",
        "aif" | "aiff" | "aifc" => "audio/aiff",
        "mka" => "audio/x-matroska",
        _ => "application/octet-stream",
    }
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

async fn track(
    State(ctx): State<Ctx>,
    Path(id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&ctx, &q) {
        return status(StatusCode::FORBIDDEN);
    }
    let Ok(path) = db::track_path(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    serve_file(&path, audio_mime(&path), &headers).await
}

pub(crate) async fn serve_file(path: &str, mime: &str, headers: &HeaderMap) -> Response {
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
    let (start, end, code) = match range {
        Some((s, e)) => (s, e, StatusCode::PARTIAL_CONTENT),
        None => (0, len.saturating_sub(1), StatusCode::OK),
    };
    if headers.contains_key(header::RANGE) && range.is_none() && len > 0 {
        let mut r = status(StatusCode::RANGE_NOT_SATISFIABLE);
        r.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes */{len}"))
                .unwrap_or(HeaderValue::from_static("")),
        );
        return r;
    }
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let count = if len == 0 { 0 } else { end - start + 1 };
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file.take(count)));
    let mut resp = Response::new(body);
    *resp.status_mut() = code;
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
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

async fn transcode(
    State(ctx): State<Ctx>,
    Path(id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if !authorized(&ctx, &q) {
        return status(StatusCode::FORBIDDEN);
    }
    let Ok(path) = db::track_path(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    let ss = q
        .get("ss")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(0.0);
    let mut cmd = omniget_core::core::process::command("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    if ss > 0.0 {
        cmd.args(["-ss", &format!("{ss:.3}")]);
    }
    cmd.args([
        "-i",
        &path,
        "-map",
        "0:a:0",
        "-vn",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "320k",
        "-f",
        "mp3",
        "pipe:1",
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
    let Ok(mut child) = cmd.spawn() else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Some(stdout) = child.stdout.take() else {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    };
    // the child rides along with the stream so ffmpeg dies when the player
    // drops the connection (seek, skip, stop)
    let stream = futures::stream::unfold(
        (tokio_util::io::ReaderStream::new(stdout), child),
        |(mut rs, child)| async move { rs.next().await.map(|chunk| (chunk, (rs, child))) },
    );
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/mpeg"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

async fn cover(
    State(ctx): State<Ctx>,
    Path(hash): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&ctx, &q) {
        return status(StatusCode::FORBIDDEN);
    }
    let thumb = q.get("t").map(|v| v == "1").unwrap_or(false);
    let Some(p) = scan::cover_file(&hash, thumb) else {
        return status(StatusCode::NOT_FOUND);
    };
    let path = p.to_string_lossy().to_string();
    let mime = match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    };
    let mut r = serve_file(&path, mime, &headers).await;
    // content-addressed, safe to cache for good
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    r
}

fn upstream_url(q: &HashMap<String, String>) -> Option<url::Url> {
    let u = url::Url::parse(q.get("u")?).ok()?;
    matches!(u.scheme(), "http" | "https").then_some(u)
}

fn radio_client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default()
    })
}

#[derive(Serialize, Clone)]
pub struct RadioMeta {
    pub session: String,
    pub title: String,
    pub artist: Option<String>,
    pub raw: String,
}

async fn radio(State(ctx): State<Ctx>, Query(q): Query<HashMap<String, String>>) -> Response {
    if !authorized(&ctx, &q) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(url) = upstream_url(&q) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let session = q.get("s").cloned().unwrap_or_default();
    let upstream = match radio_client()
        .get(url)
        .header("Icy-MetaData", "1")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            return status(
                StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            )
        }
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let metaint = upstream
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mime = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|m| {
            m.starts_with("audio/") || m.contains("ogg") || m.contains("aac") || m.contains("mpeg")
        })
        .unwrap_or("audio/mpeg")
        .to_string();
    let app = ctx.app.clone();
    let mut demux = IcyDemuxer::new(metaint);
    let mut last_title = String::new();
    let stream = upstream.bytes_stream().map(move |chunk| {
        let chunk = chunk.map_err(std::io::Error::other)?;
        let mut audio = Vec::with_capacity(chunk.len());
        for m in demux.push(&chunk, &mut audio) {
            if let Some(t) = m.stream_title {
                if t != last_title {
                    last_title = t.clone();
                    let (artist, title) = icy::split_title(&t);
                    let _ = app.emit(
                        "music-radio-meta",
                        RadioMeta {
                            session: session.clone(),
                            title,
                            artist,
                            raw: t,
                        },
                    );
                }
            }
        }
        Ok::<_, std::io::Error>(axum::body::Bytes::from(audio))
    });
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

async fn passthrough(
    State(ctx): State<Ctx>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&ctx, &q) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(url) = upstream_url(&q) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let mut req = radio_client().get(url);
    if let Some(r) = headers.get(header::RANGE) {
        req = req.header(header::RANGE, r.clone());
    }
    let upstream = match req.send().await {
        Ok(r) => r,
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let code = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let final_url = upstream.url().clone();
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // HLS playlists are small: rewrite their URIs against the address they
    // really came from (after redirects) so the player can resolve them
    if playlist_file::looks_like_hls(&content_type, final_url.as_str(), b"") {
        let Ok(body) = upstream.bytes().await else {
            return status(StatusCode::BAD_GATEWAY);
        };
        if playlist_file::looks_like_hls(&content_type, final_url.as_str(), &body) {
            let text = String::from_utf8_lossy(&body);
            let rewritten = playlist_file::absolutize_hls(&text, &final_url);
            let mut resp = Response::new(Body::from(rewritten));
            *resp.status_mut() = code;
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/vnd.apple.mpegurl"),
            );
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            return resp;
        }
        let mut resp = Response::new(Body::from(body));
        *resp.status_mut() = code;
        if let Ok(v) = HeaderValue::from_str(&content_type) {
            resp.headers_mut().insert(header::CONTENT_TYPE, v);
        }
        return resp;
    }
    let mut out_headers = HeaderMap::new();
    for key in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
    ] {
        if let Some(v) = upstream.headers().get(&key) {
            out_headers.insert(key, v.clone());
        }
    }
    let stream = upstream
        .bytes_stream()
        .map(|c| c.map_err(std::io::Error::other));
    let mut resp = Response::new(Body::from_stream(stream));
    *resp.status_mut() = code;
    resp.headers_mut().extend(out_headers);
    resp
}
