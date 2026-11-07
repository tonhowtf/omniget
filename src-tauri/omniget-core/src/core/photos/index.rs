use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

use super::library::{entry_for, Entry, Library};
use super::{meta, thumb};

const MAX_WORKERS: usize = 4;
const FLUSH_EVERY: usize = 24;
const SCAN_REPORT_EVERY: usize = 400;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Idle,
    Scanning,
    Indexing,
    Done,
    Cancelled,
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Progress {
    pub stage: Stage,
    pub found: usize,
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub error: Option<String>,
}

impl Progress {
    pub fn idle() -> Progress {
        Progress {
            stage: Stage::Idle,
            found: 0,
            total: 0,
            done: 0,
            failed: 0,
            error: None,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.stage, Stage::Scanning | Stage::Indexing)
    }
}

#[derive(Debug, Clone)]
struct Found {
    path: PathBuf,
    format: &'static str,
    size: u64,
    modified_at: i64,
}

fn hidden(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with('.') || name == "$RECYCLE.BIN" || name == "@eaDir"
}

fn scan(
    roots: &[PathBuf],
    skip: &Path,
    token: &CancellationToken,
    report: &dyn Fn(usize),
) -> Vec<Found> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for root in roots {
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || (!hidden(e.file_name()) && e.path() != skip));
        for entry in walker.filter_map(Result::ok) {
            if token.is_cancelled() {
                return found;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(format) = super::format_of(entry.path()) else {
                continue;
            };
            if !seen.insert(entry.path().to_path_buf()) {
                continue;
            }
            let Ok(md) = entry.metadata() else { continue };
            found.push(Found {
                path: entry.into_path(),
                format,
                size: md.len(),
                modified_at: md.modified().map(meta::wall_clock).unwrap_or(0),
            });
            if found.len() % SCAN_REPORT_EVERY == 0 {
                report(found.len());
            }
        }
    }
    found
}

pub fn thumb_key(path: &Path, size: u64, modified_at: i64) -> String {
    let mut h = blake3::Hasher::new();
    h.update(path.to_string_lossy().as_bytes());
    h.update(&size.to_le_bytes());
    h.update(&modified_at.to_le_bytes());
    h.finalize().to_hex()[..24].to_string()
}

fn process(lib: &Library, f: &Found) -> Entry {
    let mut entry = entry_for(&f.path, f.format, f.size, f.modified_at);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let exif = meta::read_exif(&f.path);
        let raw = (f.format == "raw").then(|| meta::raw_info(&f.path, exif.orientation));
        let img = thumb::decode_quick(&f.path, f.format, exif.orientation)?;
        let made = thumb::make(&img)?;
        let key = thumb_key(&f.path, f.size, f.modified_at);
        std::fs::write(lib.thumb_path(&key), &made.jpeg)
            .map_err(|e| format!("The preview could not be saved: {e}."))?;
        let taken = exif.taken_at.or(raw.and_then(|r| r.taken_at));
        let (width, height) = raw
            .and_then(|r| r.size)
            .unwrap_or((img.width(), img.height()));
        Ok::<_, String>((taken, width, height, key, made.dhash))
    }));
    match result {
        Ok(Ok((taken, width, height, key, dhash))) => {
            entry.taken_at = taken;
            entry.width = width;
            entry.height = height;
            entry.thumb = Some(key);
            entry.dhash = Some(dhash);
        }
        Ok(Err(e)) => entry.error = Some(e),
        Err(_) => {
            entry.error = Some(
                "This photo could not be read. It may be damaged or use a variant OmniGet can't open yet."
                    .to_string(),
            )
        }
    }
    entry
}

fn flush(lib: &Library, conn: &mut rusqlite::Connection, batch: &mut Vec<(Entry, Option<String>)>) {
    if batch.is_empty() {
        return;
    }
    if let Ok(tx) = conn.transaction() {
        for (entry, _) in batch.iter() {
            let _ = lib.upsert(&tx, entry);
        }
        let _ = tx.commit();
    }
    for (entry, old) in batch.drain(..) {
        if let Some(old) = old.filter(|o| Some(o) != entry.thumb.as_ref()) {
            let _ = std::fs::remove_file(lib.thumb_path(&old));
        }
    }
}

pub fn run(
    lib: &Library,
    roots: &[PathBuf],
    token: &CancellationToken,
    report: &(dyn Fn(&Progress) + Sync),
) -> Progress {
    let mut progress = Progress {
        stage: Stage::Scanning,
        ..Progress::idle()
    };
    report(&progress);
    let fail = |mut p: Progress, e: String| {
        p.stage = Stage::Failed;
        p.error = Some(e);
        report(&p);
        p
    };
    let mut conn = match lib.open() {
        Ok(c) => c,
        Err(e) => return fail(progress, e),
    };
    let found = scan(roots, lib.dir(), token, &|n| {
        report(&Progress {
            found: n,
            ..progress.clone()
        })
    });
    progress.found = found.len();
    if token.is_cancelled() {
        progress.stage = Stage::Cancelled;
        report(&progress);
        return progress;
    }
    let stamps = match lib.stamps(&conn) {
        Ok(s) => s,
        Err(e) => return fail(progress, e),
    };
    let seen: HashSet<String> = found
        .iter()
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect();
    let offline: Vec<&PathBuf> = roots.iter().filter(|r| !r.is_dir()).collect();
    let gone: Vec<String> = stamps
        .keys()
        .filter(|k| !seen.contains(*k))
        .filter(|k| !offline.iter().any(|r| Path::new(k).starts_with(r)))
        .cloned()
        .collect();
    if !gone.is_empty() {
        if let Ok(tx) = conn.transaction() {
            let _ = lib.forget(&tx, &gone);
            let _ = tx.commit();
        }
    }
    let todo: Vec<(Found, Option<String>)> = found
        .into_iter()
        .filter_map(|f| {
            let key = f.path.to_string_lossy();
            match stamps.get(key.as_ref()) {
                Some(s)
                    if s.size == f.size
                        && s.modified_at == f.modified_at
                        && s.done
                        && s.thumb.as_ref().is_none_or(|t| lib.thumb_path(t).exists()) =>
                {
                    None
                }
                Some(s) => Some((f, s.thumb.clone())),
                None => Some((f, None)),
            }
        })
        .collect();
    progress.stage = Stage::Indexing;
    progress.total = todo.len();
    report(&progress);

    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, MAX_WORKERS);
    let (tx, rx) = mpsc::channel::<(Entry, Option<String>)>();
    std::thread::scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let todo = &todo;
            let next = &next;
            s.spawn(move || loop {
                if token.is_cancelled() {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((f, old)) = todo.get(i) else { break };
                if tx.send((process(lib, f), old.clone())).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let mut batch = Vec::new();
        for (entry, old) in rx {
            progress.done += 1;
            if entry.error.is_some() {
                progress.failed += 1;
            }
            batch.push((entry, old));
            if batch.len() >= FLUSH_EVERY {
                flush(lib, &mut conn, &mut batch);
                report(&progress);
            }
        }
        flush(lib, &mut conn, &mut batch);
    });
    progress.stage = if token.is_cancelled() {
        Stage::Cancelled
    } else {
        Stage::Done
    };
    report(&progress);
    progress
}
