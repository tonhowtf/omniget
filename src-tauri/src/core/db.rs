use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;

const DB_FILE: &str = "omniget.db";

fn db_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(DB_FILE))
}

fn open() -> Option<Connection> {
    let path = db_path()?;
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("[db] create_dir_all failed: {}", e);
            return None;
        }
    }
    match Connection::open(&path) {
        Ok(conn) => {
            let _ = conn.pragma_update(None, "journal_mode", "WAL");
            let _ = conn.pragma_update(None, "synchronous", "NORMAL");
            let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
            Some(conn)
        }
        Err(e) => {
            tracing::error!("[db] failed to open {}: {}", path.display(), e);
            None
        }
    }
}

fn handle() -> Option<&'static Mutex<Connection>> {
    static CONN: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();
    CONN.get_or_init(|| open().map(Mutex::new)).as_ref()
}

pub fn with_conn<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let mutex = handle()?;
    let guard = match mutex.lock() {
        Ok(g) => g,
        Err(e) => {
            tracing::error!("[db] mutex poisoned: {}", e);
            return None;
        }
    };
    match f(&guard) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("[db] query failed: {}", e);
            None
        }
    }
}
