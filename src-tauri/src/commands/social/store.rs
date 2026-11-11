use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;

pub fn dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("social"))
}

fn open() -> Option<Connection> {
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("social.db")).ok()?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Some(conn)
}

fn handle() -> Option<&'static Mutex<Connection>> {
    static CONN: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();
    CONN.get_or_init(|| open().map(Mutex::new)).as_ref()
}

fn applied() -> &'static Mutex<HashSet<&'static str>> {
    static DONE: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    DONE.get_or_init(Default::default)
}

pub fn with<T>(
    schema: &'static str,
    f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
) -> Result<T, String> {
    let m = handle().ok_or("STORE_UNAVAILABLE: the social database could not be opened")?;
    let mut conn = m
        .lock()
        .map_err(|_| "STORE_UNAVAILABLE: the social database lock is poisoned")?;
    let mut done = applied()
        .lock()
        .map_err(|_| "STORE_UNAVAILABLE: the social database lock is poisoned")?;
    if !done.contains(schema) {
        conn.execute_batch(schema)
            .map_err(|e| format!("STORE_UNAVAILABLE: {e}"))?;
        done.insert(schema);
    }
    drop(done);
    f(&mut conn).map_err(|e| format!("STORE_FAILED: {e}"))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub fn memory(schema: &str) -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory database");
    conn.execute_batch(schema).expect("schema");
    conn
}
