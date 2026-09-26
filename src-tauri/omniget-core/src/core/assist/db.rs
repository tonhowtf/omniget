//! The assistant database: one SQLite file, versioned migrations owned by
//! each module, a consistent backup before any pending migration runs.
//!
//! Every module declares `pub const MIGRATIONS: &[Migration]` with its own
//! `module` name and increasing `version`s. [`AssistDb::open`] applies what is
//! missing, one migration per transaction, and records it in
//! `schema_migrations`. A migration that fails rolls back alone and `open`
//! returns the error: the file keeps the previous, usable schema, and the
//! backup taken before the first pending migration is still on disk.
//!
//! Never edit a migration that shipped; add a new version.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use rusqlite::{Connection, Transaction};

pub const ERR_ASSIST_DB: &str = "ERR_ASSIST_DB";
pub const DB_FILE: &str = "assist.db";
/// Backups kept next to the database (`assist.db.bak-<ms>`), newest first.
pub const KEEP_BACKUPS: usize = 3;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub module: &'static str,
    pub version: u32,
    pub sql: &'static str,
}

/// Every module's migrations, in the order they are applied on a new file.
pub fn all_migrations() -> Vec<Migration> {
    let mut out = Vec::new();
    out.extend_from_slice(super::bots::MIGRATIONS);
    out.extend_from_slice(super::runs::MIGRATIONS);
    out.extend_from_slice(super::memory::MIGRATIONS);
    out.extend_from_slice(super::reading::MIGRATIONS);
    out.extend_from_slice(super::groups::MIGRATIONS);
    out.extend_from_slice(super::web::MIGRATIONS);
    out.extend_from_slice(super::projection::MIGRATIONS);
    out.extend_from_slice(super::missions::MIGRATIONS);
    out.extend_from_slice(super::learning::MIGRATIONS);
    out.extend_from_slice(super::packs::MIGRATIONS);
    out.extend_from_slice(super::authority::MIGRATIONS);
    out.extend_from_slice(super::external_config::MIGRATIONS);
    out.extend_from_slice(super::media_provider::MIGRATIONS);
    out.extend_from_slice(super::media_tools::MIGRATIONS);
    out
}

pub struct AssistDb {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for AssistDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistDb")
            .field("path", &self.path)
            .finish()
    }
}

fn err<E: std::fmt::Display>(what: &str, e: E) -> String {
    format!("{ERR_ASSIST_DB}: {what}: {e}")
}

impl AssistDb {
    /// Opens (creating) the file and applies every pending migration.
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with(path, &all_migrations())
    }

    /// [`open`] with an explicit migration list (tests inject failures here).
    pub fn open_with(path: &Path, migrations: &[Migration]) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| err("creating the folder", e))?;
        }
        let conn = Connection::open(path).map_err(|e| err("opening", e))?;
        let db = Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        };
        db.pragmas()?;
        db.migrate(migrations)?;
        Ok(db)
    }

    /// A private database in memory, with every migration applied.
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| err("opening", e))?;
        let db = Self {
            conn: Mutex::new(conn),
            path: None,
        };
        db.pragmas()?;
        db.migrate(&all_migrations())?;
        Ok(db)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn pragmas(&self) -> Result<(), String> {
        self.with(|c| {
            c.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA foreign_keys=ON;
                 PRAGMA busy_timeout=5000;
                 CREATE TABLE IF NOT EXISTS schema_migrations (
                    module TEXT NOT NULL, version INTEGER NOT NULL,
                    applied_ms INTEGER NOT NULL, PRIMARY KEY(module, version));",
            )
        })
    }

    fn applied(&self) -> Result<std::collections::HashSet<(String, u32)>, String> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT module, version FROM schema_migrations")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
            rows.collect()
        })
    }

    fn has_user_tables(&self) -> bool {
        self.with(|c| {
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name <> 'schema_migrations' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get::<_, i64>(0),
            )
        })
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    fn migrate(&self, migrations: &[Migration]) -> Result<(), String> {
        let applied = self.applied()?;
        let pending: Vec<&Migration> = migrations
            .iter()
            .filter(|m| !applied.contains(&(m.module.to_string(), m.version)))
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        // Old data exists: copy it aside before touching the schema.
        if self.path.is_some() && self.has_user_tables() {
            self.backup_now()?;
        }
        for m in pending {
            self.tx(|tx| {
                tx.execute_batch(m.sql)
                    .map_err(|e| err(&format!("migration {}#{}", m.module, m.version), e))?;
                tx.execute(
                    "INSERT INTO schema_migrations(module, version, applied_ms) VALUES (?1, ?2, ?3)",
                    rusqlite::params![m.module, m.version, super::now_ms()],
                )
                .map_err(|e| err("recording the migration", e))?;
                Ok(())
            })?;
        }
        Ok(())
    }

    /// A consistent copy of the whole database (`VACUUM INTO`), kept next to
    /// it; only the newest [`KEEP_BACKUPS`] survive. Returns the file.
    pub fn backup_now(&self) -> Result<PathBuf, String> {
        let Some(path) = self.path.clone() else {
            return Err(err("backup", "in-memory database"));
        };
        let dest = PathBuf::from(format!("{}.bak-{}", path.display(), super::now_ms()));
        self.backup_to(&dest)?;
        prune_backups(&path);
        Ok(dest)
    }

    /// `VACUUM INTO` a file of the caller's choice (exports, tests).
    pub fn backup_to(&self, dest: &Path) -> Result<(), String> {
        if dest.exists() {
            return Err(err("backup", format!("{} already exists", dest.display())));
        }
        let target = dest.to_string_lossy().replace('\'', "''");
        self.with(|c| c.execute_batch(&format!("VACUUM INTO '{target}'")))
    }

    /// Runs `f` with the connection. Keep it short: every caller shares it.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn).map_err(|e| err("query", e))
    }

    /// Runs `f` inside one transaction; any `Err` rolls everything back.
    pub fn tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, String>) -> Result<T, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction().map_err(|e| err("begin", e))?;
        let out = f(&tx)?;
        tx.commit().map_err(|e| err("commit", e))?;
        Ok(out)
    }
}

fn prune_backups(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    found.reverse();
    for old in found.into_iter().skip(KEEP_BACKUPS) {
        let _ = std::fs::remove_file(old);
    }
}

// ── The process-wide instance ─────────────────────────────────────────────

static GLOBAL: OnceLock<RwLock<Option<Arc<AssistDb>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<AssistDb>>> {
    GLOBAL.get_or_init(|| RwLock::new(None))
}

/// Installs the database the tools and commands use (the app does this at
/// boot with `<llm>/assist.db`; tests with a temp file or memory).
pub fn set_global(db: Arc<AssistDb>) {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = Some(db);
}

/// The installed database. Opens `<llm dir>/assist.db` on first use when no
/// one installed one.
pub fn global() -> Result<Arc<AssistDb>, String> {
    if let Some(db) = slot().read().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(db);
    }
    let dir = crate::core::llm::roster_store::llm_dir()
        .ok_or_else(|| err("opening", "no app data folder"))?;
    let db = Arc::new(AssistDb::open(&dir.join(DB_FILE))?);
    let mut w = slot().write().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = w.clone() {
        return Ok(existing);
    }
    *w = Some(db.clone());
    Ok(db)
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A fresh file database under the system temp dir.
    pub fn temp_db() -> (Arc<AssistDb>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("omniget-assist-{}", super::super::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(DB_FILE);
        (Arc::new(AssistDb::open(&path).unwrap()), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-assist-db-{}", super::super::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(DB_FILE)
    }

    const A1: Migration = Migration {
        module: "t",
        version: 1,
        sql: "CREATE TABLE t_items(id TEXT PRIMARY KEY, v TEXT NOT NULL);",
    };
    const A2_BROKEN: Migration = Migration {
        module: "t",
        version: 2,
        sql: "ALTER TABLE t_items ADD COLUMN w TEXT; INSERT INTO nowhere VALUES (1);",
    };
    const A2_FIXED: Migration = Migration {
        module: "t",
        version: 2,
        sql: "ALTER TABLE t_items ADD COLUMN w TEXT;",
    };

    #[test]
    fn every_module_migration_applies_on_a_new_file_and_reopening_is_a_no_op() {
        let path = temp_path();
        drop(AssistDb::open(&path).unwrap());
        let db = AssistDb::open(&path).unwrap();
        let n: i64 = db
            .with(|c| c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n as usize, all_migrations().len());
    }

    #[test]
    fn a_failing_migration_keeps_old_data_and_a_backup_and_retry_does_not_duplicate() {
        let path = temp_path();
        {
            let db = AssistDb::open_with(&path, &[A1]).unwrap();
            db.with(|c| c.execute("INSERT INTO t_items VALUES ('a','keep')", []))
                .unwrap();
        }
        let failed = AssistDb::open_with(&path, &[A1, A2_BROKEN]);
        assert!(failed.is_err(), "the broken migration must surface");
        // Old schema and row intact, and the pre-migration backup exists.
        let db = AssistDb::open_with(&path, &[A1]).unwrap();
        let v: String = db
            .with(|c| c.query_row("SELECT v FROM t_items WHERE id='a'", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(v, "keep");
        let backups = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .count();
        assert!(backups >= 1);
        drop(db);
        // The fixed migration applies once; opening again changes nothing.
        let db = AssistDb::open_with(&path, &[A1, A2_FIXED]).unwrap();
        drop(db);
        let db = AssistDb::open_with(&path, &[A1, A2_FIXED]).unwrap();
        let rows: i64 = db
            .with(|c| {
                c.query_row(
                    "SELECT count(*) FROM schema_migrations WHERE module='t'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(rows, 2);
        let items: i64 = db
            .with(|c| c.query_row("SELECT count(*) FROM t_items", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(items, 1);
    }
}
