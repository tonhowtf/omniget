use std::collections::HashMap;
use std::io::Cursor;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::core::assist::web::{check_url, fetch_raw, forbidden_ip, WebPolicy};

pub const MAX_IMPORT_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_LOCAL_BYTES: u64 = 200 * 1024 * 1024;
pub const THUMB_SIDE: u32 = 1024;
pub const TINY_SIDE: u32 = 128;
const TINY_MIN_SIDE: u32 = 32;
const TINY_MAX_BYTES: usize = 4 * 1024;
pub const URL_TIMEOUT: Duration = Duration::from_secs(20);
pub const MAX_REDIRECTS: usize = 3;
pub const ASSETS_REL: &str = "files/assets";
const MAX_DECODE_SIDE: u32 = 30_000;
const MAX_DECODE_ALLOC: u64 = 768 * 1024 * 1024;
const NODE_MAX_SIDE: f64 = 480.0;
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub id: String,
    pub path: String,
    pub abs: String,
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbInfo {
    pub hash: String,
    pub thumb: String,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThumbMeta {
    w: u32,
    h: u32,
    mime: String,
    src_bytes: u64,
}

pub fn assets_dir(ws_dir: &Path) -> PathBuf {
    ws_dir.join("files").join("assets")
}

pub fn thumbs_dir(ws_dir: &Path) -> PathBuf {
    ws_dir.join(".cache").join("thumbs")
}

pub fn trash_dir(ws_dir: &Path) -> PathBuf {
    ws_dir.join("files").join(".trash")
}

pub fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    match image::guess_format(bytes).ok()? {
        ImageFormat::Png => Some(("png", "image/png")),
        ImageFormat::Jpeg => Some(("jpg", "image/jpeg")),
        ImageFormat::Gif => Some(("gif", "image/gif")),
        ImageFormat::WebP => Some(("webp", "image/webp")),
        _ => None,
    }
}

pub fn supported_ext(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["png", "jpg", "jpeg", "gif", "webp"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

fn mime_of_ext(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_DECODE_SIDE);
    l.max_image_height = Some(MAX_DECODE_SIDE);
    l.max_alloc = Some(MAX_DECODE_ALLOC);
    l
}

fn unsupported() -> String {
    "ASSET_UNSUPPORTED: only PNG, JPEG, GIF and WebP images can be added".into()
}

pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let mut r = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("ASSET_DECODE: {e}"))?;
    r.limits(limits());
    r.into_dimensions()
        .map_err(|e| format!("ASSET_DECODE: {e}"))
}

fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let mut r = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("ASSET_DECODE: {e}"))?;
    r.limits(limits());
    r.decode().map_err(|e| format!("ASSET_DECODE: {e}"))
}

fn asset_ref(ws_dir: &Path, id: &str, ext: &str, w: u32, h: u32, bytes: u64) -> AssetRef {
    let file = format!("{id}.{ext}");
    AssetRef {
        id: id.to_string(),
        path: format!("{ASSETS_REL}/{file}"),
        abs: assets_dir(ws_dir)
            .join(&file)
            .to_string_lossy()
            .into_owned(),
        w,
        h,
        bytes,
        mime: mime_of_ext(ext).to_string(),
        name: None,
        source_url: None,
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or("ASSET_IO: path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("ASSET_IO: {e}"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, bytes).map_err(|e| format!("ASSET_IO: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("ASSET_IO: {e}")
    })
}

fn hash_lock(hash: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut m = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    m.retain(|_, l| Arc::strong_count(l) > 1);
    m.entry(hash.to_string()).or_default().clone()
}

fn index_file(ws_dir: &Path) -> PathBuf {
    assets_dir(ws_dir).join("index.json")
}

fn index_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn read_index(ws_dir: &Path) -> Map<String, Value> {
    std::fs::read(index_file(ws_dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Map<String, Value>>(&b).ok())
        .unwrap_or_default()
}

pub fn remember(
    ws_dir: &Path,
    id: &str,
    name: Option<&str>,
    source_url: Option<&str>,
) -> Result<(), String> {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    if name.is_none() && source_url.is_none() {
        return Ok(());
    }
    let _g = index_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut idx = read_index(ws_dir);
    let entry = idx.entry(id.to_string()).or_insert_with(|| json!({}));
    if let Some(n) = name {
        entry["name"] = json!(n);
    }
    if let Some(u) = source_url {
        entry["sourceUrl"] = json!(u);
    }
    write_file(
        &index_file(ws_dir),
        &serde_json::to_vec_pretty(&idx).map_err(|e| e.to_string())?,
    )
}

fn forget(ws_dir: &Path, id: &str) {
    let _g = index_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut idx = read_index(ws_dir);
    if idx.remove(id).is_some() {
        if let Ok(b) = serde_json::to_vec_pretty(&idx) {
            let _ = write_file(&index_file(ws_dir), &b);
        }
    }
}

fn put_inner(ws_dir: &Path, bytes: &[u8]) -> Result<AssetRef, String> {
    let (ext, _) = sniff(bytes).ok_or_else(unsupported)?;
    let (w, h) = dimensions(bytes)?;
    let id = hash_bytes(bytes);
    let r = asset_ref(ws_dir, &id, ext, w, h, bytes.len() as u64);
    let dest = PathBuf::from(&r.abs);
    let lock = hash_lock(&id);
    let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
    if !dest.is_file() {
        write_file(&dest, bytes)?;
    }
    Ok(r)
}

pub fn put_bytes(ws_dir: &Path, bytes: &[u8]) -> Result<AssetRef, String> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(too_large(bytes.len() as u64));
    }
    put_inner(ws_dir, bytes)
}

fn too_large(n: u64) -> String {
    format!(
        "ASSET_TOO_LARGE: {} MB is over the {} MB limit",
        n / (1024 * 1024),
        MAX_IMPORT_BYTES / (1024 * 1024)
    )
}

pub fn put_path(ws_dir: &Path, path: &Path) -> Result<AssetRef, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("ASSET_NOT_FOUND: {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("ASSET_NOT_FOUND: {} is not a file", path.display()));
    }
    if meta.len() > MAX_LOCAL_BYTES {
        return Err(too_large(meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("ASSET_IO: {e}"))?;
    put_inner(ws_dir, &bytes)
}

pub fn decode_base64(raw: &str) -> Result<Vec<u8>, String> {
    let t = raw.trim();
    let body = match t.strip_prefix("data:") {
        Some(rest) => rest
            .split_once(',')
            .filter(|(head, _)| head.ends_with(";base64"))
            .map(|(_, b)| b)
            .ok_or("ASSET_BASE64: a data URL must be base64 encoded")?,
        None => t,
    };
    let clean: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() / 4 * 3 > MAX_IMPORT_BYTES + 3 {
        return Err(too_large((clean.len() / 4 * 3) as u64));
    }
    let e = base64::engine::general_purpose::STANDARD;
    e.decode(clean.as_bytes())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean.as_bytes()))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(clean.as_bytes()))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(clean.as_bytes()))
        .map_err(|e| format!("ASSET_BASE64: {e}"))
}

pub fn url_guard(raw: &str) -> Result<url::Url, String> {
    let u = check_url(raw).map_err(|e| e.replacen("ERR_WEB_URL", "ASSET_URL", 1))?;
    let host = u.host_str().unwrap_or_default().trim_matches(['[', ']']);
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err(format!(
            "ASSET_URL_BLOCKED: `{host}` points at this computer"
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if forbidden_ip(ip) {
            return Err(format!(
                "ASSET_URL_BLOCKED: `{host}` is a private or local address"
            ));
        }
    }
    Ok(u)
}

pub fn content_type_ok(ct: &str) -> bool {
    ct.trim().to_ascii_lowercase().starts_with("image/")
}

pub async fn fetch_url(raw: &str) -> Result<(Vec<u8>, String), String> {
    let u = url_guard(raw)?;
    let policy = WebPolicy {
        max_bytes: MAX_IMPORT_BYTES + 1,
        timeout: URL_TIMEOUT,
        max_redirects: MAX_REDIRECTS,
        allow_private: false,
        ..WebPolicy::default()
    };
    let r = tokio::time::timeout(URL_TIMEOUT, fetch_raw(&policy, u.as_str(), UA))
        .await
        .map_err(|_| "ASSET_URL_TIMEOUT: the site did not send the image in 20 s".to_string())?
        .map_err(|e| {
            e.replacen("ERR_WEB_BLOCKED_HOST", "ASSET_URL_BLOCKED", 1)
                .replacen("ERR_WEB_", "ASSET_URL_", 1)
        })?;
    if r.truncated || r.body.len() > MAX_IMPORT_BYTES {
        return Err(too_large(r.body.len() as u64));
    }
    if !(200..300).contains(&r.status) {
        return Err(format!("ASSET_URL_HTTP: the site answered {}", r.status));
    }
    if !content_type_ok(&r.content_type) {
        return Err(format!(
            "ASSET_URL_TYPE: the link is not an image ({})",
            if r.content_type.is_empty() {
                "no type"
            } else {
                r.content_type.as_str()
            }
        ));
    }
    if sniff(&r.body).is_none() {
        return Err(unsupported());
    }
    Ok((r.body, r.final_url))
}

pub async fn put_url(ws_dir: PathBuf, raw: &str) -> Result<(AssetRef, String), String> {
    let (bytes, final_url) = fetch_url(raw).await?;
    let r = blocking(move || put_bytes(&ws_dir, &bytes)).await?;
    Ok((r, final_url))
}

pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    static SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let sem = SEM.get_or_init(|| tokio::sync::Semaphore::new(2));
    let _permit = sem.acquire().await.map_err(|e| format!("ASSET_IO: {e}"))?;
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("ASSET_IO: {e}"))?
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

fn ref_of_file(ws_dir: &Path, p: &Path) -> Option<AssetRef> {
    let id = p.file_stem()?.to_str()?;
    if !is_hash(id) {
        return None;
    }
    let ext = ext_of(p)?;
    let bytes = std::fs::metadata(p).ok()?.len();
    let (w, h) = match read_meta(ws_dir, id) {
        Some(m) => (m.w, m.h),
        None => ImageReader::open(p)
            .ok()
            .and_then(|r| r.with_guessed_format().ok())
            .and_then(|r| r.into_dimensions().ok())
            .unwrap_or((0, 0)),
    };
    Some(asset_ref(ws_dir, id, &ext, w, h, bytes))
}

pub fn list(ws_dir: &Path) -> Vec<AssetRef> {
    let Ok(rd) = std::fs::read_dir(assets_dir(ws_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<(SystemTime, AssetRef)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let p = e.path();
            let t = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            ref_of_file(ws_dir, &p).map(|r| (t, r))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let idx = read_index(ws_dir);
    out.into_iter()
        .map(|(_, mut r)| {
            if let Some(e) = idx.get(&r.id) {
                r.name = e["name"].as_str().map(str::to_string);
                r.source_url = e["sourceUrl"].as_str().map(str::to_string);
            }
            r
        })
        .collect()
}

pub fn id_of(key: &str) -> String {
    let k = key.trim();
    let base = k.rsplit(['/', '\\']).next().unwrap_or(k);
    base.split('.').next().unwrap_or(base).to_ascii_lowercase()
}

pub fn find(ws_dir: &Path, key: &str) -> Result<AssetRef, String> {
    let id = id_of(key);
    if id.len() < 6 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("ASSET_NOT_FOUND: '{key}' is not an asset id"));
    }
    let hits: Vec<AssetRef> = list(ws_dir)
        .into_iter()
        .filter(|r| r.id.starts_with(&id))
        .collect();
    match hits.len() {
        0 => Err(format!(
            "ASSET_NOT_FOUND: no asset '{key}' in this workspace"
        )),
        1 => Ok(hits.into_iter().next().expect("one")),
        n => Err(format!(
            "ASSET_AMBIGUOUS: {n} assets start with '{id}'; pass more of the id"
        )),
    }
}

pub fn trash(ws_dir: &Path, r: &AssetRef) -> Result<PathBuf, String> {
    let src = PathBuf::from(&r.abs);
    let dir = trash_dir(ws_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("ASSET_IO: {e}"))?;
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| r.id.clone());
    let dest = dir.join(name);
    std::fs::rename(&src, &dest)
        .or_else(|_| std::fs::copy(&src, &dest).and_then(|_| std::fs::remove_file(&src)))
        .map_err(|e| format!("ASSET_IO: {e}"))?;
    let _ = std::fs::remove_file(thumb_file(ws_dir, &r.id));
    let _ = std::fs::remove_file(meta_file(ws_dir, &r.id));
    forget(ws_dir, &r.id);
    Ok(dest)
}

pub fn thumb_file(ws_dir: &Path, hash: &str) -> PathBuf {
    thumbs_dir(ws_dir).join(format!("{hash}-{THUMB_SIDE}.webp"))
}

fn meta_file(ws_dir: &Path, hash: &str) -> PathBuf {
    thumbs_dir(ws_dir).join(format!("{hash}.json"))
}

fn read_meta(ws_dir: &Path, hash: &str) -> Option<ThumbMeta> {
    let bytes = std::fs::read(meta_file(ws_dir, hash)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn cached_thumb(ws_dir: &Path, hash: &str) -> Option<ThumbInfo> {
    let t = thumb_file(ws_dir, hash);
    if !t.is_file() {
        return None;
    }
    let m = read_meta(ws_dir, hash)?;
    Some(ThumbInfo {
        hash: hash.to_string(),
        thumb: t.to_string_lossy().into_owned(),
        w: m.w,
        h: m.h,
    })
}

fn encode_webp(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let rgba = img.to_rgba8();
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("ASSET_THUMB: {e}"))?;
    Ok(out)
}

pub fn make_thumb(ws_dir: &Path, hash: &str, bytes: &[u8]) -> Result<ThumbInfo, String> {
    if let Some(t) = cached_thumb(ws_dir, hash) {
        return Ok(t);
    }
    let lock = hash_lock(hash);
    let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = cached_thumb(ws_dir, hash) {
        return Ok(t);
    }
    let img = decode(bytes)?;
    let (w, h) = (img.width(), img.height());
    let small = if w.max(h) > THUMB_SIDE {
        img.resize(
            THUMB_SIDE,
            THUMB_SIDE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let webp = encode_webp(&small)?;
    let t = thumb_file(ws_dir, hash);
    write_file(&t, &webp)?;
    let mime = sniff(bytes).map(|(_, m)| m).unwrap_or("image/unknown");
    let meta = ThumbMeta {
        w,
        h,
        mime: mime.to_string(),
        src_bytes: bytes.len() as u64,
    };
    write_file(
        &meta_file(ws_dir, hash),
        &serde_json::to_vec(&meta).map_err(|e| e.to_string())?,
    )?;
    Ok(ThumbInfo {
        hash: hash.to_string(),
        thumb: t.to_string_lossy().into_owned(),
        w,
        h,
    })
}

pub fn thumb_for_asset(ws_dir: &Path, r: &AssetRef) -> Result<ThumbInfo, String> {
    if let Some(t) = cached_thumb(ws_dir, &r.id) {
        return Ok(t);
    }
    let bytes = std::fs::read(&r.abs).map_err(|e| format!("ASSET_NOT_FOUND: {e}"))?;
    make_thumb(ws_dir, &r.id, &bytes)
}

type HashKey = (PathBuf, u64, Option<SystemTime>);

fn path_hashes() -> &'static Mutex<HashMap<HashKey, String>> {
    static M: OnceLock<Mutex<HashMap<HashKey, String>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn thumb_for_file(ws_dir: &Path, path: &Path) -> Result<ThumbInfo, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("ASSET_NOT_FOUND: {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("ASSET_NOT_FOUND: {} is not a file", path.display()));
    }
    if meta.len() > MAX_LOCAL_BYTES {
        return Err(too_large(meta.len()));
    }
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    let known = path_hashes().lock().ok().and_then(|m| m.get(&key).cloned());
    if let Some(t) = known.as_deref().and_then(|h| cached_thumb(ws_dir, h)) {
        return Ok(t);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("ASSET_IO: {e}"))?;
    if sniff(&bytes).is_none() {
        return Err(unsupported());
    }
    let hash = hash_bytes(&bytes);
    if let Ok(mut m) = path_hashes().lock() {
        if m.len() > 4096 {
            m.clear();
        }
        m.insert(key, hash.clone());
    }
    make_thumb(ws_dir, &hash, &bytes)
}

pub fn tiny_thumb(ws_dir: &Path, hash: &str) -> Option<String> {
    let t = cached_thumb(ws_dir, hash)?;
    let bytes = std::fs::read(&t.thumb).ok()?;
    let img = image::load_from_memory_with_format(&bytes, ImageFormat::WebP).ok()?;
    let largest = img.width().max(img.height());
    let mut side = TINY_SIDE.min(largest).max(1);
    loop {
        let out = tiny_jpeg(&img, side)?;
        if out.len() <= TINY_MAX_BYTES || side <= TINY_MIN_SIDE {
            return Some(format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&out)
            ));
        }
        side = (side / 2).max(TINY_MIN_SIDE);
    }
}

fn tiny_jpeg(img: &image::DynamicImage, side: u32) -> Option<Vec<u8>> {
    let small = img.thumbnail(side, side).to_rgba8();
    let mut rgb = image::RgbImage::new(small.width(), small.height());
    for (x, y, p) in small.enumerate_pixels() {
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])]));
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 60)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

pub fn clear_thumbs(canvas_root: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(canvas_root) else {
        return 0;
    };
    let mut freed = 0;
    for e in rd.flatten() {
        let dir = thumbs_dir(&e.path());
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for f in files.flatten() {
            freed += f.metadata().map(|m| m.len()).unwrap_or(0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    freed
}

pub fn resolve_path(ws_dir: &Path, p: &str) -> Option<PathBuf> {
    let path = Path::new(p.trim());
    if p.trim().is_empty() {
        return None;
    }
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(ws_dir.join(path))
}

pub fn inside(ws_dir: &Path, p: &Path) -> bool {
    let (Ok(root), Ok(abs)) = (ws_dir.canonicalize(), p.canonicalize()) else {
        return false;
    };
    abs.starts_with(root)
}

pub fn fit_size(w: u32, h: u32) -> (f64, f64) {
    if w == 0 || h == 0 {
        return (320.0, 240.0);
    }
    let (w, h) = (w as f64, h as f64);
    let k = (NODE_MAX_SIDE / w.max(h)).min(1.0);
    ((w * k).max(40.0), (h * k).max(40.0))
}

pub fn node_data(r: &AssetRef, source_url: Option<&str>) -> Map<String, Value> {
    let mut d = Map::new();
    d.insert("asset".into(), json!(r.id));
    d.insert("path".into(), json!(r.path));
    if r.w > 0 && r.h > 0 {
        d.insert("naturalW".into(), json!(r.w));
        d.insert("naturalH".into(), json!(r.h));
    }
    if let Some(u) = source_url {
        d.insert("sourceUrl".into(), json!(u));
    }
    d
}

pub fn uses(node_data: &Value, r: &AssetRef) -> bool {
    node_data["asset"].as_str() == Some(r.id.as_str())
        || node_data["path"].as_str() == Some(r.path.as_str())
        || node_data["path"].as_str() == Some(r.abs.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn sniff_accepts_only_the_raster_formats() {
        assert_eq!(sniff(&png(2, 2)), Some(("png", "image/png")));
        assert_eq!(
            sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]).map(|s| s.0),
            Some("jpg")
        );
        assert_eq!(sniff(b"GIF89a\x01\x00\x01\x00").map(|s| s.0), Some("gif"));
        assert_eq!(
            sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 ").map(|s| s.0),
            Some("webp")
        );
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff(b"hello"), None);
    }

    #[test]
    fn base64_takes_plain_and_data_urls() {
        let raw = png(3, 1);
        let b = base64::engine::general_purpose::STANDARD.encode(&raw);
        assert_eq!(decode_base64(&b).unwrap(), raw);
        assert_eq!(
            decode_base64(&format!("data:image/png;base64,{b}")).unwrap(),
            raw
        );
        assert_eq!(
            decode_base64(&format!(" {}\n{} ", &b[..8], &b[8..])).unwrap(),
            raw
        );
        assert!(decode_base64("data:image/png,abc").is_err());
        let huge = "A".repeat(MAX_IMPORT_BYTES / 3 * 4 + 64);
        assert!(decode_base64(&huge)
            .unwrap_err()
            .starts_with("ASSET_TOO_LARGE"));
    }

    #[test]
    fn url_guard_blocks_local_and_odd_schemes() {
        assert!(url_guard("https://example.com/a.png").is_ok());
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/a.png",
            "http://127.0.0.1/a.png",
            "http://10.1.2.3/a.png",
            "http://192.168.0.10:8080/a.png",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/a.png",
            "http://[fd00::1]/a.png",
            "http://localhost/a.png",
            "http://api.localhost/a.png",
            "http://user:pw@example.com/a.png",
            "not a url",
        ] {
            assert!(url_guard(bad).is_err(), "{bad} should be refused");
        }
        assert!(content_type_ok("image/png"));
        assert!(content_type_ok("Image/WebP; charset=binary"));
        assert!(!content_type_ok("text/html"));
        assert!(!content_type_ok(""));
    }

    #[test]
    fn put_dedups_by_hash_and_makes_a_thumb() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let raw = png(2000, 1000);
        let a = put_bytes(ws, &raw).unwrap();
        let b = put_bytes(ws, &raw).unwrap();
        assert_eq!(a, b);
        assert_eq!((a.w, a.h), (2000, 1000));
        assert_eq!(a.path, format!("files/assets/{}.png", a.id));
        assert_eq!(list(ws).len(), 1);
        assert_eq!(find(ws, &a.id[..10]).unwrap().id, a.id);
        assert_eq!(find(ws, &a.path).unwrap().id, a.id);
        remember(ws, &a.id, Some("red"), Some("https://example.com/r.png")).unwrap();
        assert_eq!(list(ws)[0].name.as_deref(), Some("red"));
        let t = thumb_for_asset(ws, &a).unwrap();
        assert_eq!((t.w, t.h), (2000, 1000));
        let thumb = image::open(&t.thumb).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (1024, 512));
        assert!(tiny_thumb(ws, &a.id)
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        assert!(put_bytes(ws, b"<svg/>")
            .unwrap_err()
            .starts_with("ASSET_UNSUPPORTED"));
        let moved = trash(ws, &a).unwrap();
        assert!(moved.is_file());
        assert!(list(ws).is_empty());
        assert!(cached_thumb(ws, &a.id).is_none());
    }

    #[test]
    fn relative_paths_stay_in_the_workspace() {
        let ws = Path::new("/data/canvas/home");
        assert_eq!(
            resolve_path(ws, "files/assets/x.png"),
            Some(ws.join("files/assets/x.png"))
        );
        assert_eq!(resolve_path(ws, "../other/x.png"), None);
        assert_eq!(resolve_path(ws, ""), None);
        assert_eq!(fit_size(2000, 1000), (480.0, 240.0));
        assert_eq!(fit_size(100, 50), (100.0, 50.0));
        assert_eq!(id_of("files/assets/ABCdef12.png"), "abcdef12");
    }
}
