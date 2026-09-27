//! The washmybelly database: plans, receipts, setting snapshots and pending
//! approvals, in `<app_data>/washmybelly/wmb.db`.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Where washmybelly keeps its state. Production uses [`Ctx::app`]; tests
/// hand in a temporary folder so they never touch the real app data.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub dir: PathBuf,
}

impl Ctx {
    pub fn app() -> Result<Ctx, String> {
        Ok(Ctx {
            dir: super::data_dir()?,
        })
    }

    pub fn at(dir: &Path) -> Ctx {
        let _ = std::fs::create_dir_all(dir);
        Ctx {
            dir: dir.to_path_buf(),
        }
    }

    pub fn quarantine_dir(&self) -> PathBuf {
        self.dir.join("quarantine")
    }

    pub fn open(&self) -> Result<Connection, String> {
        let db = Connection::open(self.dir.join("wmb.db")).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS plans (id TEXT PRIMARY KEY, kind TEXT NOT NULL, created INTEGER NOT NULL, expires INTEGER NOT NULL, status TEXT NOT NULL, body TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS receipts (id TEXT PRIMARY KEY, plan_id TEXT NOT NULL, created INTEGER NOT NULL, body TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS snapshots (id TEXT PRIMARY KEY, receipt_id TEXT NOT NULL, module TEXT NOT NULL, key TEXT NOT NULL, created INTEGER NOT NULL, body TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS approvals (id TEXT PRIMARY KEY, plan_id TEXT NOT NULL, client TEXT NOT NULL, created INTEGER NOT NULL, status TEXT NOT NULL, receipt_id TEXT);
             CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, body TEXT NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        Ok(db)
    }

    /// A small JSON value by key (module state: holds, perf sessions index...).
    pub fn kv_get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let db = self.open().ok()?;
        let body: Option<String> = db
            .query_row("SELECT body FROM kv WHERE key=?1", params![key], |r| {
                r.get(0)
            })
            .optional()
            .ok()?;
        body.and_then(|b| serde_json::from_str(&b).ok())
    }

    pub fn kv_set<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let body = serde_json::to_string(value).map_err(|e| e.to_string())?;
        self.open()?
            .execute(
                "INSERT INTO kv(key, body) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET body=excluded.body",
                params![key, body],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Save the state a module had before it changed it, for undo.
    pub fn snapshot_put<T: Serialize>(
        &self,
        receipt_id: &str,
        module: &str,
        key: &str,
        value: &T,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let body = serde_json::to_string(value).map_err(|e| e.to_string())?;
        self.open()?
            .execute(
                "INSERT INTO snapshots(id, receipt_id, module, key, created, body) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, receipt_id, module, key, now(), body],
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn snapshot_get<T: DeserializeOwned>(&self, id: &str) -> Result<T, String> {
        let body: String = self
            .open()?
            .query_row("SELECT body FROM snapshots WHERE id=?1", params![id], |r| {
                r.get(0)
            })
            .map_err(|_| format!("snapshot {id} not found"))?;
        serde_json::from_str(&body).map_err(|e| e.to_string())
    }
}

/// Seconds since the epoch.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
