use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use rusqlite::{Connection, Transaction};

pub const ERR_ASSIST_DB: &str = "ERR_ASSIST_DB";
pub const DB_FILE: &str = "assist.db";
pub const KEEP_BACKUPS: usize = 3;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub module: &'static str,
    pub version: u32,
    pub sql: &'static str,
}

const READING_MIGRATIONS: &[Migration] = &[Migration {
    module: "reading",
    version: 1,
    sql: "CREATE TABLE reading_journeys (
            id TEXT PRIMARY KEY,
            principal TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            title TEXT NOT NULL,
            author TEXT,
            edition TEXT,
            language TEXT,
            external_id TEXT,
            status TEXT NOT NULL DEFAULT 'active'
              CHECK (status IN ('active','paused','finished','abandoned')),
            spoiler_terms_json TEXT NOT NULL DEFAULT '[]',
            notes TEXT,
            started_ms INTEGER NOT NULL,
            finished_ms INTEGER,
            updated_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_journeys_bot ON reading_journeys(bot_id, status);
          CREATE TABLE reading_progress (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            position_kind TEXT NOT NULL CHECK (position_kind IN ('chapter','page','percent','locator')),
            value TEXT NOT NULL,
            value_num REAL,
            edition TEXT,
            origin TEXT NOT NULL CHECK (origin IN ('manual','reader','bot')),
            note TEXT,
            recorded_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_progress_journey ON reading_progress(journey_id, recorded_ms);
          CREATE TABLE reading_contexts (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            text TEXT NOT NULL,
            created_ms INTEGER NOT NULL,
            expires_ms INTEGER NOT NULL
          );
          CREATE TABLE reading_access_prefs (
            principal TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            region TEXT NOT NULL DEFAULT 'BR',
            subscriptions_json TEXT NOT NULL DEFAULT '[]',
            access_kinds_json TEXT NOT NULL DEFAULT '[\"subscription\",\"free\",\"rent\",\"buy\"]',
            only_confirmed INTEGER NOT NULL DEFAULT 0,
            freshness_days INTEGER NOT NULL DEFAULT 7,
            updated_ms INTEGER NOT NULL,
            PRIMARY KEY (principal, bot_id)
          );
          CREATE TABLE reading_movies (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            original_title TEXT,
            year INTEGER NOT NULL,
            director TEXT,
            external_ids_json TEXT NOT NULL DEFAULT '{}',
            ident_key TEXT NOT NULL UNIQUE,
            created_ms INTEGER NOT NULL
          );
          CREATE TABLE reading_availability (
            id TEXT PRIMARY KEY,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id) ON DELETE CASCADE,
            region TEXT NOT NULL,
            platform TEXT NOT NULL,
            platform_key TEXT NOT NULL,
            access_kind TEXT NOT NULL CHECK (access_kind IN ('subscription','rent','buy','free')),
            url TEXT,
            status TEXT NOT NULL CHECK (status IN ('confirmed','inferred','absent','unknown')),
            source_kind TEXT NOT NULL CHECK (source_kind IN ('platform','aggregator','search','user')),
            quote TEXT,
            fetch_id TEXT,
            page_date_ms INTEGER,
            note TEXT,
            bot_id TEXT,
            checked_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_availability_movie ON reading_availability(movie_id, platform_key, access_kind, checked_ms);
          CREATE TABLE reading_subtitles (
            id TEXT PRIMARY KEY,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id) ON DELETE CASCADE,
            availability_id TEXT REFERENCES reading_availability(id) ON DELETE SET NULL,
            platform_key TEXT NOT NULL,
            language TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('subtitle','audio')),
            status TEXT NOT NULL CHECK (status IN ('confirmed','inferred','absent','unknown')),
            url TEXT,
            source_kind TEXT NOT NULL CHECK (source_kind IN ('platform','aggregator','search','user')),
            quote TEXT,
            fetch_id TEXT,
            login_required INTEGER NOT NULL DEFAULT 0,
            note TEXT,
            bot_id TEXT,
            checked_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_subtitles_movie ON reading_subtitles(movie_id, platform_key, checked_ms);
          CREATE TABLE reading_rounds (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            progress_id TEXT REFERENCES reading_progress(id) ON DELETE SET NULL,
            reason TEXT,
            context_text TEXT,
            requested_count INTEGER,
            shortfall_reason TEXT,
            only_confirmed INTEGER NOT NULL DEFAULT 0,
            skill_name TEXT,
            skill_hash TEXT,
            look_for_json TEXT NOT NULL DEFAULT '[]',
            bot_id TEXT,
            created_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_rounds_journey ON reading_rounds(journey_id, created_ms);
          CREATE TABLE reading_round_items (
            id TEXT PRIMARY KEY,
            round_id TEXT NOT NULL REFERENCES reading_rounds(id) ON DELETE CASCADE,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id),
            position INTEGER NOT NULL,
            role TEXT NOT NULL CHECK (role IN ('entry','shift','surprise')),
            connection TEXT NOT NULL,
            why TEXT NOT NULL,
            moods_json TEXT NOT NULL DEFAULT '[]',
            pace TEXT NOT NULL CHECK (pace IN ('easy','attentive')),
            heavy INTEGER NOT NULL DEFAULT 0,
            depends_on_ending INTEGER NOT NULL DEFAULT 0,
            resumes_item_id TEXT,
            questions_json TEXT NOT NULL DEFAULT '[]',
            evidence_event_ids_json TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL CHECK (status IN ('confirmed','probable')),
            missing_json TEXT NOT NULL DEFAULT '[]',
            availability_id TEXT,
            subtitle_id TEXT
          );
          CREATE TABLE reading_viewing_events (
            id TEXT PRIMARY KEY,
            journey_id TEXT NOT NULL REFERENCES reading_journeys(id) ON DELETE CASCADE,
            movie_id TEXT NOT NULL REFERENCES reading_movies(id),
            round_item_id TEXT,
            kind TEXT NOT NULL CHECK (kind IN ('recommended','planned','watched','abandoned','declined')),
            reaction TEXT,
            reasons_json TEXT NOT NULL DEFAULT '[]',
            origin TEXT NOT NULL CHECK (origin IN ('user','bot','app')),
            created_ms INTEGER NOT NULL
          );
          CREATE INDEX reading_events_journey ON reading_viewing_events(journey_id, movie_id, created_ms);",
}];

const LEARNING_MIGRATIONS: &[Migration] = &[Migration {
    module: "learning",
    version: 1,
    sql: "
CREATE TABLE learn_settings (
    bot_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1,
    auto_promote_local INTEGER NOT NULL DEFAULT 0,
    policy TEXT NOT NULL DEFAULT '{}',
    updated_ms INTEGER NOT NULL
);
CREATE TABLE learn_observations (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL DEFAULT '',
    skill_hash TEXT,
    overlay_version TEXT,
    task_ref TEXT NOT NULL DEFAULT '',
    scope TEXT NOT NULL,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    polarity INTEGER NOT NULL DEFAULT 0,
    outcome TEXT NOT NULL DEFAULT 'unverified',
    evidence_ref TEXT,
    content_hash TEXT NOT NULL,
    dup_of TEXT,
    revoked_ms INTEGER,
    revoked_reason TEXT,
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_observations_bot ON learn_observations(bot_id, skill, created_ms DESC);
CREATE INDEX learn_observations_hash ON learn_observations(content_hash);
CREATE TABLE learn_candidates (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'overlay',
    title TEXT NOT NULL,
    state TEXT NOT NULL,
    current_version INTEGER NOT NULL DEFAULT 1,
    reason TEXT,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX learn_candidates_bot ON learn_candidates(bot_id, skill, state);
CREATE TABLE learn_candidate_versions (
    candidate_id TEXT NOT NULL REFERENCES learn_candidates(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    overlay TEXT NOT NULL,
    diff TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT '',
    observations TEXT NOT NULL DEFAULT '[]',
    created_ms INTEGER NOT NULL,
    PRIMARY KEY (candidate_id, version)
);
CREATE TABLE learn_eval_cases (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    split TEXT NOT NULL CHECK (split IN ('dev','holdout')),
    name TEXT NOT NULL,
    input TEXT NOT NULL,
    checks TEXT NOT NULL DEFAULT '[]',
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_eval_cases_bot ON learn_eval_cases(bot_id, skill, split);
CREATE TABLE learn_fixtures (
    case_id TEXT NOT NULL REFERENCES learn_eval_cases(id) ON DELETE CASCADE,
    variant TEXT NOT NULL,
    output TEXT NOT NULL,
    recorded_ms INTEGER NOT NULL,
    PRIMARY KEY (case_id, variant)
);
CREATE TABLE learn_eval_runs (
    id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL,
    candidate_version INTEGER NOT NULL,
    baseline TEXT NOT NULL,
    case_set TEXT NOT NULL,
    mode TEXT NOT NULL,
    policy TEXT NOT NULL,
    results TEXT NOT NULL,
    summary TEXT NOT NULL,
    decision TEXT NOT NULL,
    reasons TEXT NOT NULL DEFAULT '[]',
    meta TEXT NOT NULL DEFAULT '{}',
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_eval_runs_candidate ON learn_eval_runs(candidate_id, created_ms DESC);
CREATE TABLE learn_active (
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    scope TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    since_ms INTEGER NOT NULL,
    PRIMARY KEY (bot_id, skill, scope)
);
CREATE TABLE learn_promotions (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL,
    skill TEXT NOT NULL,
    scope TEXT NOT NULL,
    action TEXT NOT NULL,
    candidate_id TEXT,
    version INTEGER,
    previous TEXT,
    eval_run_id TEXT,
    by_whom TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_ms INTEGER NOT NULL
);
CREATE INDEX learn_promotions_bot ON learn_promotions(bot_id, skill, scope, created_ms DESC);
",
}];

const PACKS_MIGRATIONS: &[Migration] = &[Migration {
    module: "packs",
    version: 1,
    sql: "
CREATE TABLE packs_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    origin TEXT NOT NULL,
    origin_sha TEXT,
    source_path TEXT NOT NULL,
    hash TEXT NOT NULL,
    installed_hash TEXT,
    license TEXT,
    attribution TEXT,
    capabilities TEXT NOT NULL DEFAULT '[]',
    deps TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    content TEXT,
    installed_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    UNIQUE(kind, name)
);
CREATE TABLE packs_history (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    action TEXT NOT NULL,
    hash TEXT NOT NULL,
    content TEXT,
    backup_path TEXT,
    created_ms INTEGER NOT NULL
);
CREATE INDEX packs_history_item ON packs_history(item_id, version DESC);
",
}];

pub fn all_migrations() -> Vec<Migration> {
    let mut out = Vec::new();
    out.extend_from_slice(super::bots::MIGRATIONS);
    out.extend_from_slice(super::runs::MIGRATIONS);
    out.extend_from_slice(super::memory::MIGRATIONS);
    out.extend_from_slice(READING_MIGRATIONS);
    out.extend_from_slice(super::groups::MIGRATIONS);
    out.extend_from_slice(super::web::MIGRATIONS);
    out.extend_from_slice(super::projection::MIGRATIONS);
    out.extend_from_slice(super::missions::MIGRATIONS);
    out.extend_from_slice(LEARNING_MIGRATIONS);
    out.extend_from_slice(PACKS_MIGRATIONS);
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
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with(path, &all_migrations())
    }

    pub fn open_with(path: &Path, migrations: &[Migration]) -> Result<Self, String> {
        static OPENING: Mutex<()> = Mutex::new(());
        let _turn = OPENING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| err("creating the folder", e))?;
        }
        let conn = Connection::open(path).map_err(|e| err("opening", e))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| err("opening", e))?;
        let db = Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        };
        db.pragmas()?;
        db.migrate(migrations)?;
        Ok(db)
    }

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
        if self.path.is_some() && self.has_user_tables() {
            self.backup_now()?;
        }
        for m in pending {
            let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| err("begin", e))?;
            let done: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE module=?1 AND version=?2)",
                    rusqlite::params![m.module, m.version],
                    |r| r.get(0),
                )
                .map_err(|e| err("reading the migrations", e))?;
            if done {
                continue;
            }
            (|| -> Result<(), String> {
                tx.execute_batch(m.sql)
                    .map_err(|e| err(&format!("migration {}#{}", m.module, m.version), e))?;
                tx.execute(
                    "INSERT INTO schema_migrations(module, version, applied_ms) VALUES (?1, ?2, ?3)",
                    rusqlite::params![m.module, m.version, super::now_ms()],
                )
                .map_err(|e| err("recording the migration", e))?;
                Ok(())
            })()?;
            tx.commit().map_err(|e| err("commit", e))?;
        }
        Ok(())
    }

    pub fn backup_now(&self) -> Result<PathBuf, String> {
        let Some(path) = self.path.clone() else {
            return Err(err("backup", "in-memory database"));
        };
        let dest = PathBuf::from(format!("{}.bak-{}", path.display(), super::now_ms()));
        self.backup_to(&dest)?;
        prune_backups(&path);
        Ok(dest)
    }

    pub fn backup_to(&self, dest: &Path) -> Result<(), String> {
        if dest.exists() {
            return Err(err("backup", format!("{} already exists", dest.display())));
        }
        let target = dest.to_string_lossy().replace('\'', "''");
        self.with(|c| c.execute_batch(&format!("VACUUM INTO '{target}'")))
    }

    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn).map_err(|e| err("query", e))
    }

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

static GLOBAL: OnceLock<RwLock<Option<Arc<AssistDb>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<AssistDb>>> {
    GLOBAL.get_or_init(|| RwLock::new(None))
}

pub fn set_global(db: Arc<AssistDb>) {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = Some(db);
}

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
