//! Registry of every file saved from Telegram (`data_dir/downloaded.sqlite`),
//! so the same file (same Telegram file id, or same size and name) is not
//! downloaded again into another folder.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use crate::core::telegram::host;

/// Prefix of the error `download_media` returns for a duplicate; the
/// existing path follows it.
pub const DUPLICATE_PREFIX: &str = "DUPLICATE_OF:";

pub fn duplicate_of(err: &str) -> Option<&str> {
    err.find(DUPLICATE_PREFIX)
        .map(|i| &err[i + DUPLICATE_PREFIX.len()..])
}

fn conn() -> &'static Mutex<Option<Connection>> {
    static C: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
    C.get_or_init(|| {
        let path = host::data_dir().join("downloaded.sqlite");
        let c = Connection::open(path).ok().and_then(|c| {
            c.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS files (
                    file_uid TEXT PRIMARY KEY,
                    size INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    chat_id INTEGER,
                    msg_id INTEGER,
                    saved_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS files_size_name ON files(size, name);
                 CREATE TABLE IF NOT EXISTS stats (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
            )
            .ok()?;
            Some(c)
        });
        Mutex::new(c)
    })
}

fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let g = conn().lock().unwrap_or_else(|p| p.into_inner());
    g.as_ref().and_then(|c| f(c).ok())
}

/// The `dedupe` setting (on by default).
pub fn enabled() -> bool {
    super::setting("dedupe").as_bool().unwrap_or(true)
}

pub fn set_enabled(on: bool) {
    super::save_setting("dedupe", json!(on));
}

fn on_disk(path: &Path, size: u64) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && (size == 0 || m.len() == size))
        .unwrap_or(false)
}

/// Where this file was already saved, if it still is there. Entries whose
/// file is gone are dropped.
pub fn find(uid: Option<&str>, size: u64, name: &str) -> Option<PathBuf> {
    if let Some(uid) = uid {
        let hit: Option<(String, i64)> = with(|c| {
            c.query_row(
                "SELECT path, size FROM files WHERE file_uid = ?1",
                [uid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })
        .flatten();
        if let Some((path, sz)) = hit {
            let p = PathBuf::from(&path);
            if on_disk(&p, sz.max(0) as u64) {
                return Some(p);
            }
            with(|c| c.execute("DELETE FROM files WHERE file_uid = ?1", [uid]));
        }
    }
    // Same size and name: only for files big enough that a clash is not a
    // coincidence.
    if size < 256 * 1024 || name.is_empty() {
        return None;
    }
    let rows: Vec<(String, String)> = with(|c| {
        let mut st = c.prepare("SELECT file_uid, path FROM files WHERE size = ?1 AND name = ?2")?;
        let rows = st
            .query_map(params![size as i64, name], |r| Ok((r.get(0)?, r.get(1)?)))?
            .flatten()
            .collect();
        Ok(rows)
    })
    .unwrap_or_default();
    for (uid, path) in rows {
        let p = PathBuf::from(&path);
        if on_disk(&p, size) {
            return Some(p);
        }
        with(|c| c.execute("DELETE FROM files WHERE file_uid = ?1", [uid]));
    }
    None
}

/// Registers a saved file.
pub fn record(uid: Option<&str>, size: u64, name: &str, path: &Path, chat_id: i64, msg_id: i32) {
    let key = uid
        .map(str::to_string)
        .unwrap_or_else(|| format!("n{}:{}", size, name));
    with(|c| {
        c.execute(
            "INSERT INTO files (file_uid, size, name, path, chat_id, msg_id, saved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(file_uid) DO UPDATE SET size = excluded.size, name = excluded.name,
                path = excluded.path, chat_id = excluded.chat_id, msg_id = excluded.msg_id,
                saved_at = excluded.saved_at",
            params![
                key,
                size as i64,
                name,
                path.to_string_lossy(),
                chat_id,
                msg_id,
                super::now_secs()
            ],
        )
    });
}

fn bump(key: &str, by: i64) {
    with(|c| {
        c.execute(
            "INSERT INTO stats (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = value + excluded.value",
            params![key, by],
        )
    });
}

/// Counts one download that was avoided.
pub fn note_avoided(size: u64) {
    bump("avoided", 1);
    bump("avoided_bytes", size as i64);
}

fn stat(key: &str) -> i64 {
    with(|c| {
        c.query_row("SELECT value FROM stats WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()
    })
    .flatten()
    .unwrap_or(0)
}

/// `{enabled, files, bytes, avoided, avoided_bytes}`.
pub fn stats() -> serde_json::Value {
    let (files, bytes): (i64, i64) = with(|c| {
        c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    })
    .unwrap_or((0, 0));
    json!({
        "enabled": enabled(),
        "files": files,
        "bytes": bytes,
        "avoided": stat("avoided"),
        "avoided_bytes": stat("avoided_bytes"),
    })
}

/// Forgets every registered file and the counters.
pub fn clear() {
    with(|c| c.execute_batch("DELETE FROM files; DELETE FROM stats;"));
}
