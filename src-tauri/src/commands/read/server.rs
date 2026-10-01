//! Localhost HTTP server the reader and outside e-readers read from.
//!
//! The webview can't open arbitrary local files, and foliate-js reads books
//! lazily through `Range` requests, so books go through here instead of the
//! IPC. Bound to 127.0.0.1 on a random port; every request carries the
//! per-launch token (`k=`).
//!
//! * `GET /b/<id>`             book file, with Range
//! * `GET /c/<hash>?t=1`       cover (thumbnail with `t=1`)
//! * `GET /pg/<id>/<n>`        page `n` (0-based) of a comic book or folder
//! * `GET /img?u=&h=&x=`       remote page proxy: headers `h` (JSON pairs),
//!   XOR key `x`
//!
//! With "share on the network" on, a second listener on 0.0.0.0 serves the
//! same library to e-readers over OPDS 1.2 (+ page streaming) and speaks the
//! KOReader sync protocol, behind HTTP Basic auth:
//!
//! * `GET /opds...`            catalog, see `opds_*`
//! * `/users/create`, `/users/auth`, `/syncs/progress[/<doc>]`  kosync

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::OnceCell;
use tower_http::cors::{Any, CorsLayer};

use omniget_core::core::read::{comic, format, kosync, opds};

use super::{db, scan};
use crate::commands::music::server::serve_file;

#[derive(Clone, Serialize)]
pub struct ServerInfo {
    pub port: u16,
    pub token: String,
}

#[derive(Clone)]
struct Ctx {
    app: AppHandle,
    token: Arc<String>,
    /// `true` on the LAN listener: token auth off, Basic auth on.
    lan: bool,
}

static SERVER: OnceCell<ServerInfo> = OnceCell::const_new();

#[derive(Clone, Serialize, serde::Deserialize, Default, PartialEq)]
pub struct ShareConfig {
    pub enabled: bool,
    pub port: u16,
    pub username: String,
    pub password: String,
}

pub fn share_config() -> ShareConfig {
    let mut cfg = db::kv_get("share").unwrap_or(ShareConfig {
        enabled: false,
        port: 8580,
        username: "omniget".into(),
        password: String::new(),
    });
    cfg.password = super::secret("share");
    cfg
}

struct Lan {
    port: u16,
    stop: tokio::sync::oneshot::Sender<()>,
}

static LAN: Mutex<Option<Lan>> = Mutex::new(None);

fn router(ctx: Ctx) -> Router {
    let mut r = Router::new()
        .route("/b/{id}", get(book_file))
        .route("/bc/{id}", get(prepared_file))
        .route("/c/{hash}", get(cover))
        .route("/pg/{id}/{n}", get(page))
        .route("/img", get(remote_image))
        .route("/opds", get(opds_root))
        .route("/opds/search.xml", get(opds_search_desc))
        .route("/opds/{kind}", get(opds_list))
        .route("/opds/{kind}/{key}", get(opds_list_key))
        .route("/opds-file/{id}", get(book_file))
        .route("/opds-cover/{hash}", get(cover))
        .route("/opds-page/{id}/{n}", get(page));
    if ctx.lan {
        r = r
            .route("/users/create", post(kosync_create))
            .route("/users/auth", get(kosync_auth))
            .route("/syncs/progress", put(kosync_put))
            .route("/syncs/progress/{doc}", get(kosync_get));
    }
    r.layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
            .expose_headers(Any),
    )
    .with_state(ctx)
}

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
                lan: false,
            };
            let router = router(ctx);
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, router).await {
                    tracing::error!("[read] server stopped: {e}");
                }
            });
            tracing::info!("[read] server on 127.0.0.1:{port}");
            Ok::<_, String>(ServerInfo { port, token })
        })
        .await
        .cloned()
}

pub fn lan_running() -> bool {
    LAN.lock().map(|l| l.is_some()).unwrap_or(false)
}

pub fn lan_port() -> Option<u16> {
    LAN.lock().ok().and_then(|l| l.as_ref().map(|x| x.port))
}

pub async fn start_lan(app: &AppHandle, cfg: &ShareConfig) -> Result<u16, String> {
    stop_lan();
    if cfg.password.is_empty() {
        return Err("set a password before sharing the library".into());
    }
    let info = ensure(app).await?;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cfg.port))
        .await
        .map_err(|e| format!("port {}: {e}", cfg.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let ctx = Ctx {
        app: app.clone(),
        token: Arc::new(info.token),
        lan: true,
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let router = router(ctx);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    if let Ok(mut l) = LAN.lock() {
        *l = Some(Lan { port, stop: tx });
    }
    tracing::info!("[read] sharing library on 0.0.0.0:{port}");
    Ok(port)
}

pub fn stop_lan() {
    if let Ok(mut l) = LAN.lock() {
        if let Some(lan) = l.take() {
            let _ = lan.stop.send(());
        }
    }
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

fn basic_ok(headers: &HeaderMap) -> bool {
    let cfg = share_config();
    if cfg.password.is_empty() {
        return false;
    }
    let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
    else {
        return false;
    };
    let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(v.trim()) else {
        return false;
    };
    let want = format!("{}:{}", cfg.username, cfg.password);
    constant_time_eq::constant_time_eq(&raw, want.as_bytes())
}

/// Token for the app, Basic auth for e-readers on the network. Returns the
/// response to send back when the request is not allowed.
fn denied(ctx: &Ctx, q: &HashMap<String, String>, headers: &HeaderMap) -> Option<Response> {
    if ctx.lan {
        if basic_ok(headers) {
            return None;
        }
        let mut r = status(StatusCode::UNAUTHORIZED);
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"OmniGet\""),
        );
        return Some(r);
    }
    let ok = q
        .get("k")
        .map(|k| constant_time_eq::constant_time_eq(k.as_bytes(), ctx.token.as_bytes()))
        .unwrap_or(false);
    (!ok).then(|| status(StatusCode::FORBIDDEN))
}

async fn book_file(
    State(ctx): State<Ctx>,
    Path(id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let Ok(b) = db::book(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    let mime = format::BookFormat::from_path(std::path::Path::new(&b.path))
        .map(|f| f.mime())
        .unwrap_or("application/octet-stream");
    let mut r = serve_file(&b.path, mime, &headers).await;
    if ctx.lan {
        let name = std::path::Path::new(&b.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("book");
        if let Ok(v) = HeaderValue::from_str(&format!(
            "attachment; filename*=UTF-8''{}",
            urlencoding::encode(name)
        )) {
            r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    r
}

async fn prepared_file(
    State(ctx): State<Ctx>,
    Path(id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let Some(p) = db::book(id).ok().as_ref().and_then(super::prepared_path) else {
        return status(StatusCode::NOT_FOUND);
    };
    serve_file(&p.to_string_lossy(), "application/epub+zip", &headers).await
}

async fn cover(
    State(ctx): State<Ctx>,
    Path(hash): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let thumb = q.get("t").map(|v| v == "1").unwrap_or(false);
    let Some(p) = scan::cover_file(&hash, thumb) else {
        return status(StatusCode::NOT_FOUND);
    };
    let path = p.to_string_lossy().to_string();
    let mime = format::image_mime(&path);
    let mut r = serve_file(&path, mime, &headers).await;
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    r
}

/// Page list of a comic, cached by path + mtime so paging through a CBZ
/// doesn't re-read the central directory for every image.
fn comic_pages(path: &str) -> Option<Arc<Vec<String>>> {
    type Cache = Mutex<HashMap<String, (i64, Arc<Vec<String>>)>>;
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let p = std::path::Path::new(path);
    let mtime = std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Some((m, v)) = cache.lock().ok()?.get(path) {
        if *m == mtime {
            return Some(v.clone());
        }
    }
    let pages = if p.is_dir() {
        comic::folder_pages(p)
    } else {
        let f = std::fs::File::open(p).ok()?;
        let mut z = zip::ZipArchive::new(f).ok()?;
        comic_zip_pages(&mut z)
    };
    let v = Arc::new(pages);
    if let Ok(mut c) = cache.lock() {
        if c.len() > 64 {
            c.clear();
        }
        c.insert(path.to_string(), (mtime, v.clone()));
    }
    Some(v)
}

/// The app crate links zip 4 while omniget-core uses zip 2; list pages here
/// with the same rules as `comic::zip_pages`.
fn comic_zip_pages<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>) -> Vec<String> {
    let mut names: Vec<String> = (0..z.len())
        .filter_map(|i| {
            let f = z.by_index(i).ok()?;
            let n = f.name().to_string();
            let base = n.rsplit('/').next().unwrap_or(&n).to_string();
            (f.is_file()
                && format::is_image_name(&n)
                && !n.starts_with("__MACOSX")
                && !base.starts_with('.'))
            .then_some(n)
        })
        .collect();
    names.sort_by(|a, b| format::natural_cmp(a, b));
    names
}

pub fn page_count(path: &str) -> usize {
    comic_pages(path).map(|p| p.len()).unwrap_or(0)
}

pub fn read_page(path: &str, n: usize) -> Option<(Vec<u8>, &'static str)> {
    let pages = comic_pages(path)?;
    let name = pages.get(n)?;
    let p = std::path::Path::new(path);
    let bytes = if p.is_dir() {
        std::fs::read(p.join(name)).ok()?
    } else {
        let f = std::fs::File::open(p).ok()?;
        let mut z = zip::ZipArchive::new(f).ok()?;
        let mut e = z.by_name(name).ok()?;
        let mut v = Vec::with_capacity(e.size() as usize);
        e.read_to_end(&mut v).ok()?;
        v
    };
    Some((bytes, format::image_mime(name)))
}

async fn page(
    State(ctx): State<Ctx>,
    Path((id, n)): Path<(i64, usize)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let Ok(b) = db::book(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    let width = q.get("w").and_then(|w| w.parse::<u32>().ok());
    let res = tokio::task::spawn_blocking(move || {
        let (bytes, mime) = read_page(&b.path, n)?;
        match width {
            // Resized for e-readers that ask for a max width (OPDS-PSE).
            Some(w) if w > 0 => {
                let img = image::load_from_memory(&bytes).ok()?;
                if img.width() <= w {
                    return Some((bytes, mime));
                }
                let t = img.resize(w, u32::MAX, image::imageops::FilterType::Triangle);
                let mut out = std::io::Cursor::new(Vec::new());
                t.to_rgb8()
                    .write_to(&mut out, image::ImageFormat::Jpeg)
                    .ok()?;
                Some((out.into_inner(), "image/jpeg"))
            }
            _ => Some((bytes, mime)),
        }
    })
    .await
    .ok()
    .flatten();
    let Some((bytes, mime)) = res else {
        return status(StatusCode::NOT_FOUND);
    };
    let mut r = Response::new(Body::from(bytes));
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    r
}

fn proxy_client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(omniget_core::core::read::sources::client)
}

/// XOR-decrypts a MANGA Plus page with its hex key.
pub fn xor_decrypt(data: &mut [u8], hex_key: &str) {
    let Ok(key) = hex::decode(hex_key) else {
        return;
    };
    if key.is_empty() {
        return;
    }
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= key[i % key.len()];
    }
}

async fn remote_image(
    State(ctx): State<Ctx>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let Some(url) = q
        .get("u")
        .and_then(|u| url::Url::parse(u).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
    else {
        return status(StatusCode::BAD_REQUEST);
    };
    let mut req = proxy_client().get(url);
    let extra: Vec<(String, String)> = q
        .get("h")
        .and_then(|h| serde_json::from_str(h).ok())
        .unwrap_or_default();
    for (k, v) in extra {
        // Sources may set Referer, Origin, cookies and tokens, never Host.
        if !k.eq_ignore_ascii_case("host") {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            return status(
                StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            )
        }
        Err(_) => return status(StatusCode::BAD_GATEWAY),
    };
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();
    let Ok(bytes) = resp.bytes().await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let mut data = bytes.to_vec();
    if let Some(k) = q.get("x").filter(|k| !k.is_empty()) {
        xor_decrypt(&mut data, k);
    }
    let mut r = Response::new(Body::from(data));
    if let Ok(v) = HeaderValue::from_str(&ctype) {
        r.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    r
}

// ---------- OPDS ----------

const OPDS_PAGE: usize = 50;

/// An entry's download link, cover link and, for comics, the page-streaming
/// (OPDS-PSE) URL template with its page count.
type OpdsLinks = (String, Option<String>, Option<(String, u32)>);

fn xml_response(body: String, kind: &'static str) -> Response {
    let mut r = Response::new(Body::from(body));
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    r
}

const ATOM: &str = "application/atom+xml;profile=opds-catalog;charset=utf-8";

/// Query suffix that keeps the token on in-app links (`?k=`), empty on LAN.
fn auth_q(ctx: &Ctx) -> String {
    if ctx.lan {
        String::new()
    } else {
        format!("k={}", ctx.token)
    }
}

fn with_q(path: &str, q: &str) -> String {
    if q.is_empty() {
        path.to_string()
    } else if path.contains('?') {
        format!("{path}&{q}")
    } else {
        format!("{path}?{q}")
    }
}

async fn opds_root(
    State(ctx): State<Ctx>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let a = auth_q(&ctx);
    let mut navs: Vec<(String, String, String, String)> = vec![
        (
            "recent".into(),
            "Recently added".into(),
            "/opds/recent".into(),
            "Newest books first".into(),
        ),
        (
            "reading".into(),
            "Continue reading".into(),
            "/opds/reading".into(),
            "Books in progress".into(),
        ),
        (
            "all".into(),
            "All books".into(),
            "/opds/all".into(),
            "Every book, by title".into(),
        ),
        (
            "series".into(),
            "Series".into(),
            "/opds/series".into(),
            "Browse by series".into(),
        ),
        (
            "authors".into(),
            "Authors".into(),
            "/opds/authors".into(),
            "Browse by author".into(),
        ),
        (
            "shelves".into(),
            "Shelves".into(),
            "/opds/shelves".into(),
            "Your shelves".into(),
        ),
        (
            "favorites".into(),
            "Favorites".into(),
            "/opds/favorites".into(),
            "Starred books".into(),
        ),
    ];
    for n in &mut navs {
        n.2 = with_q(&n.2, &a);
    }
    let served: Vec<opds::ServedNav<'_>> = navs
        .iter()
        .map(|(id, t, h, c)| opds::ServedNav {
            id,
            title: t,
            href: h,
            content: c,
        })
        .collect();
    let body = opds::render_feed(
        "urn:omniget:read",
        "OmniGet library",
        &with_q("/opds", &a),
        &with_q("/opds/search.xml", &a),
        &served,
        &[],
        None,
    );
    xml_response(body, ATOM)
}

async fn opds_search_desc(
    State(ctx): State<Ctx>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let tpl = with_q("/opds/search?q={searchTerms}", &auth_q(&ctx));
    xml_response(
        opds::opensearch_description(&tpl),
        "application/opensearchdescription+xml",
    )
}

async fn opds_list(
    State(ctx): State<Ctx>,
    Path(kind): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    opds_feed(ctx, kind, None, q, headers).await
}

async fn opds_list_key(
    State(ctx): State<Ctx>,
    Path((kind, key)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    opds_feed(ctx, kind, Some(key), q, headers).await
}

async fn opds_feed(
    ctx: Ctx,
    kind: String,
    key: Option<String>,
    q: HashMap<String, String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = denied(&ctx, &q, &headers) {
        return r;
    }
    let a = auth_q(&ctx);
    let page: usize = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(0);
    let Ok(mut books) = db::all_books() else {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    };
    // Navigation lists (series, authors, shelves) first.
    let nav_of =
        |items: Vec<(String, usize)>, base: &str| -> Vec<(String, String, String, String)> {
            items
                .into_iter()
                .map(|(name, n)| {
                    (
                        format!("{base}:{name}"),
                        name.clone(),
                        with_q(&format!("/opds/{base}/{}", urlencoding::encode(&name)), &a),
                        format!("{n} books"),
                    )
                })
                .collect()
        };
    let mut navs: Vec<(String, String, String, String)> = Vec::new();
    let title: String;
    match (kind.as_str(), key.as_deref()) {
        ("series", None) => {
            let mut m: std::collections::BTreeMap<String, usize> = Default::default();
            for b in books.iter().filter(|b| !b.series.is_empty()) {
                *m.entry(b.series.clone()).or_default() += 1;
            }
            navs = nav_of(m.into_iter().collect(), "series");
            books.clear();
            title = "Series".into();
        }
        ("authors", None) => {
            let mut m: std::collections::BTreeMap<String, usize> = Default::default();
            for b in &books {
                for au in &b.authors {
                    *m.entry(au.clone()).or_default() += 1;
                }
            }
            navs = nav_of(m.into_iter().collect(), "authors");
            books.clear();
            title = "Authors".into();
        }
        ("shelves", None) => {
            let shelves = super::library::shelves_raw().unwrap_or_default();
            navs = shelves
                .into_iter()
                .map(|(id, name, n)| {
                    (
                        format!("shelf:{id}"),
                        name,
                        with_q(&format!("/opds/shelves/{id}"), &a),
                        format!("{n} books"),
                    )
                })
                .collect();
            books.clear();
            title = "Shelves".into();
        }
        ("series", Some(s)) => {
            books.retain(|b| b.series == s);
            books.sort_by(|x, y| {
                x.series_index
                    .partial_cmp(&y.series_index)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            title = s.to_string();
        }
        ("authors", Some(au)) => {
            books.retain(|b| b.authors.iter().any(|x| x == au));
            title = au.to_string();
        }
        ("shelves", Some(id)) => {
            let ids = id
                .parse::<i64>()
                .ok()
                .and_then(|id| super::library::shelf_book_ids(id).ok())
                .unwrap_or_default();
            books.retain(|b| ids.contains(&b.id));
            title = "Shelf".into();
        }
        ("recent", _) => {
            books.sort_by_key(|b| std::cmp::Reverse(b.added_at));
            title = "Recently added".into();
        }
        ("reading", _) => {
            books.retain(|b| b.status == "reading");
            books.sort_by_key(|b| std::cmp::Reverse(b.last_read_at));
            title = "Continue reading".into();
        }
        ("favorites", _) => {
            books.retain(|b| b.favorite);
            title = "Favorites".into();
        }
        ("search", _) => {
            let term = q.get("q").cloned().unwrap_or_default().to_lowercase();
            books.retain(|b| {
                b.title.to_lowercase().contains(&term)
                    || b.series.to_lowercase().contains(&term)
                    || b.authors.iter().any(|a| a.to_lowercase().contains(&term))
            });
            title = format!("Search: {term}");
        }
        _ => title = "All books".into(),
    }
    let total = books.len();
    let slice: Vec<&db::BookRow> = books
        .iter()
        .skip(page * OPDS_PAGE)
        .take(OPDS_PAGE)
        .collect();
    let files: Vec<OpdsLinks> = slice
        .iter()
        .map(|b| {
            let file = with_q(&format!("/opds-file/{}", b.id), &a);
            let cover = b
                .cover
                .as_ref()
                .map(|c| with_q(&format!("/opds-cover/{c}?x=1"), &a));
            let comic = format::BookFormat::from_path(std::path::Path::new(&b.path))
                .map(|f| f.is_comic())
                .unwrap_or(false);
            let stream = comic.then(|| {
                (
                    with_q(
                        &format!("/opds-page/{}/{{pageNumber}}?w={{maxWidth}}", b.id),
                        &a,
                    ),
                    b.pages.unwrap_or(0).max(0) as u32,
                )
            });
            (file, cover, stream)
        })
        .collect();
    let mimes: Vec<&'static str> = slice
        .iter()
        .map(|b| {
            format::BookFormat::from_path(std::path::Path::new(&b.path))
                .map(|f| f.mime())
                .unwrap_or("application/octet-stream")
        })
        .collect();
    let ids: Vec<String> = slice
        .iter()
        .map(|b| format!("urn:omniget:book:{}", b.id))
        .collect();
    let updated: Vec<String> = slice
        .iter()
        .map(|b| {
            chrono::DateTime::from_timestamp_millis(b.added_at)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default()
        })
        .collect();
    let entries: Vec<opds::ServedEntry<'_>> = slice
        .iter()
        .enumerate()
        .map(|(i, b)| opds::ServedEntry {
            id: &ids[i],
            title: &b.title,
            authors: &b.authors,
            summary: &b.description,
            updated: &updated[i],
            language: &b.language,
            download_href: &files[i].0,
            mime: mimes[i],
            cover_href: files[i].1.as_deref(),
            stream: files[i].2.as_ref().map(|(t, n)| (t.as_str(), *n)),
        })
        .collect();
    let served_navs: Vec<opds::ServedNav<'_>> = navs
        .iter()
        .map(|(id, t, h, c)| opds::ServedNav {
            id,
            title: t,
            href: h,
            content: c,
        })
        .collect();
    let self_href = with_q(&format!("/opds/{kind}"), &a);
    let next = ((page + 1) * OPDS_PAGE < total).then(|| {
        let mut base = format!("/opds/{kind}");
        if let Some(k) = &key {
            base.push('/');
            base.push_str(&urlencoding::encode(k));
        }
        let mut qs = format!("page={}", page + 1);
        if let Some(t) = q.get("q") {
            qs.push_str(&format!("&q={}", urlencoding::encode(t)));
        }
        with_q(&format!("{base}?{qs}"), &a)
    });
    let body = opds::render_feed(
        &format!("urn:omniget:read:{kind}"),
        &title,
        &self_href,
        &with_q("/opds/search.xml", &a),
        &served_navs,
        &entries,
        next.as_deref(),
    );
    xml_response(body, ATOM)
}

// ---------- KOReader sync server ----------

fn kosync_user(headers: &HeaderMap) -> Option<String> {
    let cfg = share_config();
    let user = headers.get("x-auth-user")?.to_str().ok()?;
    let key = headers.get("x-auth-key")?.to_str().ok()?;
    (user == cfg.username
        && !cfg.password.is_empty()
        && constant_time_eq::constant_time_eq(
            key.as_bytes(),
            kosync::user_key(&cfg.password).as_bytes(),
        ))
    .then(|| user.to_string())
}

fn kosync_unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "message": "Unauthorized" })),
    )
        .into_response()
}

/// Accounts are the shared library login; "register" just confirms it.
async fn kosync_create(Json(body): Json<serde_json::Value>) -> Response {
    let cfg = share_config();
    let user = body["username"].as_str().unwrap_or("");
    let key = body["password"].as_str().unwrap_or("");
    if user == cfg.username && key == kosync::user_key(&cfg.password) {
        (
            StatusCode::CREATED,
            Json(serde_json::json!({ "username": user })),
        )
            .into_response()
    } else {
        (
            StatusCode::PAYMENT_REQUIRED,
            Json(serde_json::json!({ "message": "Use the OmniGet library login" })),
        )
            .into_response()
    }
}

async fn kosync_auth(headers: HeaderMap) -> Response {
    match kosync_user(&headers) {
        Some(_) => Json(serde_json::json!({ "authorized": "OK" })).into_response(),
        None => kosync_unauthorized(),
    }
}

async fn kosync_put(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    Json(mut p): Json<kosync::Progress>,
) -> Response {
    if kosync_user(&headers).is_none() {
        return kosync_unauthorized();
    }
    let ts = chrono::Utc::now().timestamp();
    p.timestamp = Some(ts);
    let _ = db::kv_set(&format!("kosync:{}", p.document), &p);
    // Mirror into the library when the document is one of our books.
    let pct = p.percentage.clamp(0.0, 1.0);
    let doc = p.document.clone();
    let updated = db::with(|c| {
        c.execute(
            "UPDATE books SET progress = ?1, last_read_at = ?2,
                status = CASE WHEN ?1 >= 0.995 THEN 'finished' WHEN status = 'unread' THEN 'reading' ELSE status END,
                finished_at = CASE WHEN ?1 >= 0.995 AND finished_at IS NULL THEN ?2 ELSE finished_at END
             WHERE hash = ?3",
            rusqlite::params![pct, db::now_ms(), doc],
        )
    })
    .unwrap_or(0);
    if updated > 0 {
        let _ = ctx.app.emit("read-library-changed", ());
    }
    Json(serde_json::json!({ "document": p.document, "timestamp": ts })).into_response()
}

async fn kosync_get(headers: HeaderMap, Path(doc): Path<String>) -> Response {
    if kosync_user(&headers).is_none() {
        return kosync_unauthorized();
    }
    if let Some(p) = db::kv_get::<kosync::Progress>(&format!("kosync:{doc}")) {
        return Json(p).into_response();
    }
    // Progress made in OmniGet itself, for a book KOReader also has.
    let from_lib = db::with(|c| {
        use rusqlite::OptionalExtension;
        c.query_row(
            "SELECT progress, last_read_at FROM books WHERE hash = ?1 AND progress > 0",
            [&doc],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()
    })
    .ok()
    .flatten();
    match from_lib {
        Some((pct, at)) => Json(serde_json::json!({
            "document": doc,
            "percentage": pct,
            "progress": "",
            "device": "OmniGet",
            "device_id": "omniget",
            "timestamp": at.map(|t| t / 1000),
        }))
        .into_response(),
        None => Json(serde_json::json!({})).into_response(),
    }
}
