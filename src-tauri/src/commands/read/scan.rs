use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use omniget_core::core::read::{extract, format::BookFormat, kosync};

use super::db;

static SCANNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn is_scanning() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

const THUMB_W: u32 = 320;
const THUMB_H: u32 = 480;

pub fn store_cover(data: &[u8]) -> Option<String> {
    let dir = db::covers_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let hash = hex::encode(&Sha256::digest(data)[..10]);
    let ext = if data.starts_with(b"\x89PNG") {
        "png"
    } else if data.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if data.starts_with(b"GIF8") {
        "gif"
    } else {
        "jpg"
    };
    let full = dir.join(format!("{hash}.{ext}"));
    if !full.exists() {
        std::fs::write(&full, data).ok()?;
    }
    let thumb = dir.join(format!("{hash}-t.jpg"));
    if !thumb.exists() {
        if let Ok(img) = image::load_from_memory(data) {
            let t = img.thumbnail(THUMB_W, THUMB_H).to_rgb8();
            let _ = t.save_with_format(&thumb, image::ImageFormat::Jpeg);
        }
    }
    Some(hash)
}

pub fn cover_file(hash: &str, thumb: bool) -> Option<PathBuf> {
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) || hash.len() > 64 {
        return None;
    }
    let dir = db::covers_dir()?;
    if thumb {
        let t = dir.join(format!("{hash}-t.jpg"));
        if t.exists() {
            return Some(t);
        }
    }
    for ext in ["jpg", "png", "webp", "gif"] {
        let p = dir.join(format!("{hash}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn sidecar_cover(path: &Path) -> Option<Vec<u8>> {
    let dir = path.parent()?;
    let stem = path.file_stem()?.to_str()?;
    for name in [
        format!("{stem}.jpg"),
        format!("{stem}.png"),
        "cover.jpg".into(),
        "cover.png".into(),
        "folder.jpg".into(),
    ] {
        let p = dir.join(&name);
        if p.is_file() && p != path {
            return std::fs::read(p).ok();
        }
    }
    None
}

fn stamp(meta: &std::fs::Metadata) -> (i64, i64) {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    (mtime_ms, meta.len() as i64)
}

pub fn is_book_path(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with("._") || name.starts_with('.') {
        return false;
    }
    match BookFormat::from_path(p) {
        Some(BookFormat::Html | BookFormat::Md | BookFormat::Txt) => false,
        Some(_) => true,
        None => false,
    }
}

pub struct IndexOpts {
    pub source: String,
    pub source_url: String,
    pub series_id: Option<i64>,
    pub chapter_id: Option<i64>,
}

impl Default for IndexOpts {
    fn default() -> Self {
        Self {
            source: "local".into(),
            source_url: String::new(),
            series_id: None,
            chapter_id: None,
        }
    }
}

pub fn index_file(path: &Path, opts: &IndexOpts) -> Result<i64, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let (mtime_ms, size) = stamp(&meta);
    let ex = extract::extract(path)?;
    let cover = ex.cover.or_else(|| sidecar_cover(path));
    let cover_hash = cover.as_deref().and_then(store_cover);
    let hash = if path.is_file() {
        kosync::partial_md5_path(path).unwrap_or_default()
    } else {
        String::new()
    };
    let p = path.to_string_lossy().to_string();
    let folder = path
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    db::upsert_book(&db::NewBook {
        path: p,
        folder,
        format: format!("{:?}", ex.format).to_lowercase(),
        size,
        mtime_ms,
        hash,
        meta: ex.meta,
        cover: cover_hash,
        source: opts.source.clone(),
        source_url: opts.source_url.clone(),
        series_id: opts.series_id,
        chapter_id: opts.chapter_id,
    })
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
    current: String,
}

#[derive(Serialize, Clone, Default)]
pub struct ScanSummary {
    pub added_or_updated: usize,
    pub removed: usize,
    pub failed: usize,
    pub total: usize,
}

fn walk(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !e
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with('.') || n == "__MACOSX")
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| is_book_path(p))
        .collect()
}

pub async fn scan(app: AppHandle, full: bool) -> Result<ScanSummary, String> {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return Err("a scan is already running".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let res = scan_inner(&app, full).await;
    SCANNING.store(false, Ordering::SeqCst);
    let _ = app.emit("read-scan-done", res.as_ref().ok());
    res
}

async fn scan_inner(app: &AppHandle, full: bool) -> Result<ScanSummary, String> {
    let roots = db::roots()?;
    let mut summary = ScanSummary::default();
    for root in roots {
        let root_path = PathBuf::from(&root);
        if !root_path.exists() {
            continue;
        }
        let known = db::stamps_under(&root)?;
        let files = tokio::task::spawn_blocking({
            let r = root_path.clone();
            move || walk(&r)
        })
        .await
        .map_err(|e| e.to_string())?;
        summary.total += files.len();
        let present: HashSet<String> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let todo: Vec<PathBuf> = files
            .into_iter()
            .filter(|p| {
                if full {
                    return true;
                }
                let key = p.to_string_lossy().to_string();
                match (known.get(&key), std::fs::metadata(p).ok()) {
                    (Some((mt, sz)), Some(meta)) => stamp(&meta) != (*mt, *sz),
                    _ => true,
                }
            })
            .collect();

        let total = todo.len();
        let mut done = 0usize;
        let mut last_emit = std::time::Instant::now();
        let mut stream = futures::stream::iter(todo.into_iter().map(|p| {
            tokio::task::spawn_blocking(move || {
                let r = index_file(&p, &IndexOpts::default());
                (p, r)
            })
        }))
        .buffer_unordered(4);
        while let Some(joined) = stream.next().await {
            if CANCEL.load(Ordering::SeqCst) {
                break;
            }
            done += 1;
            if let Ok((p, r)) = joined {
                match r {
                    Ok(_) => summary.added_or_updated += 1,
                    Err(e) => {
                        summary.failed += 1;
                        tracing::debug!("[read] skip {}: {e}", p.display());
                    }
                }
                if last_emit.elapsed().as_millis() > 150 || done == total {
                    last_emit = std::time::Instant::now();
                    let _ = app.emit(
                        "read-scan-progress",
                        Progress {
                            done,
                            total,
                            current: p
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        },
                    );
                }
            }
        }

        let gone: Vec<String> = known
            .keys()
            .filter(|k| !present.contains(*k))
            .filter(|k| !Path::new(k).exists())
            .cloned()
            .collect();
        if !gone.is_empty() && !CANCEL.load(Ordering::SeqCst) {
            summary.removed += gone.len();
            db::with_mut(|c| {
                let tx = c.transaction()?;
                {
                    let mut st = tx.prepare("DELETE FROM books WHERE path = ?1")?;
                    for g in &gone {
                        st.execute([g])?;
                    }
                }
                tx.commit()
            })?;
        }
    }
    Ok(summary)
}
