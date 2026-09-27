//! Library scan: walks the music folders, reads tags of new or changed files
//! (by mtime + size), caches cover art and drops rows whose file is gone.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use omniget_core::core::music::tags::{self, Cover};

use super::db;

static SCANNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn is_scanning() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

pub fn covers_dir() -> Option<PathBuf> {
    db::music_dir().map(|d| d.join("covers"))
}

const FOLDER_COVERS: &[&str] = &["cover", "folder", "front", "album", "albumart"];
const THUMB_PX: u32 = 360;

/// Stores the picture once per content hash, plus a small JPEG thumbnail
/// for grids. Returns the hash used by `/c/<hash>`.
pub fn store_cover(c: &Cover) -> Option<String> {
    let dir = covers_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let hash = hex::encode(&Sha256::digest(&c.data)[..10]);
    let ext = match c.mime.as_str() {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    };
    let full = dir.join(format!("{hash}.{ext}"));
    if !full.exists() {
        std::fs::write(&full, &c.data).ok()?;
    }
    let thumb = dir.join(format!("{hash}-t.jpg"));
    if !thumb.exists() {
        if let Ok(img) = image::load_from_memory(&c.data) {
            let t = img.thumbnail(THUMB_PX, THUMB_PX).to_rgb8();
            let _ = t.save_with_format(&thumb, image::ImageFormat::Jpeg);
        }
    }
    Some(hash)
}

/// `/c/<hash>` resolution: thumbnail when asked and present, else original.
pub fn cover_file(hash: &str, thumb: bool) -> Option<PathBuf> {
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) || hash.len() > 64 {
        return None;
    }
    let dir = covers_dir()?;
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

fn folder_cover(dir: &Path) -> Option<Cover> {
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        let stem = p.file_stem()?.to_str()?.to_ascii_lowercase();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if FOLDER_COVERS.contains(&stem.as_str())
            && ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str())
        {
            let data = std::fs::read(&p).ok()?;
            let mime = tags::sniff_mime(&data).to_string();
            return Some(Cover { data, mime });
        }
    }
    None
}

pub fn sidecar_lrc(path: &Path) -> Option<PathBuf> {
    let p = path.with_extension("lrc");
    p.exists().then_some(p)
}

fn stamp(meta: &std::fs::Metadata) -> db::FileStamp {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    db::FileStamp {
        mtime_ms,
        size: meta.len() as i64,
    }
}

/// Reads one file and writes its row. Used by the scan and after edits.
pub fn index_file(path: &Path) -> Result<i64, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let st = stamp(&meta);
    let (m, cover) = tags::read(path).map_err(|e| e.to_string())?;
    let cover = cover.or_else(|| path.parent().and_then(folder_cover));
    let cover_hash = cover.as_ref().and_then(store_cover);
    let has_lyrics = !m.lyrics.trim().is_empty() || sidecar_lrc(path).is_some();
    let p = path.to_string_lossy().to_string();
    let folder = path
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    db::with(|c| db::upsert_track(c, &p, &folder, &st, &m, cover_hash.as_deref(), has_lyrics))
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
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| tags::is_audio_path(p))
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("._"))
                .unwrap_or(false)
        })
        .collect()
}

pub async fn scan(app: AppHandle, full: bool) -> Result<ScanSummary, String> {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return Err("a scan is already running".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let res = scan_inner(&app, full).await;
    SCANNING.store(false, Ordering::SeqCst);
    let _ = app.emit("music-scan-done", res.as_ref().ok());
    res
}

async fn scan_inner(app: &AppHandle, full: bool) -> Result<ScanSummary, String> {
    let roots = db::roots()?;
    let mut summary = ScanSummary::default();
    for root in roots {
        let root_path = PathBuf::from(&root);
        if !root_path.exists() {
            // an unplugged drive: keep its tracks until it comes back
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
                    (Some((mt, sz)), Some(meta)) => {
                        let s = stamp(&meta);
                        s.mtime_ms != *mt || s.size != *sz
                    }
                    _ => true,
                }
            })
            .collect();

        let total = todo.len();
        let mut done = 0usize;
        let mut last_emit = std::time::Instant::now();
        let mut stream = futures::stream::iter(todo.into_iter().map(|p| {
            tokio::task::spawn_blocking(move || {
                let r = index_file(&p);
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
                        tracing::debug!("[music] skip {}: {e}", p.display());
                    }
                }
                if last_emit.elapsed().as_millis() > 150 || done == total {
                    last_emit = std::time::Instant::now();
                    let _ = app.emit(
                        "music-scan-progress",
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
            .cloned()
            .collect();
        if !gone.is_empty() && !CANCEL.load(Ordering::SeqCst) {
            summary.removed += gone.len();
            db::with_mut(|c| {
                let tx = c.transaction()?;
                {
                    let mut st = tx.prepare("DELETE FROM tracks WHERE path = ?1")?;
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
