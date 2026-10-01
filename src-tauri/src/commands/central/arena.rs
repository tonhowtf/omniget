use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use omniget_core::core::threads::model::{CommandEnvelope, DomainEvent};
use omniget_core::core::threads::store;
use omniget_core::core::threads::ThreadsEngine;
use omniget_core::core::vcs::diff::{self, DiffOptions, DiffResult};
use omniget_core::core::vcs::repo::{self, GitCtx};
use omniget_core::core::vcs::runner::{self, Invocation};
use omniget_core::core::vcs::worktree;
use omniget_core::core::vcs::{integrate, job_worktree};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::threads_host;

mod compare;
pub use compare::{ArenaCompare, ArenaEstimate};

pub const ARENA_EVENT: &str = "central://arena";
pub const ERR_ARENA: &str = "ERR_ARENA";
pub const MIN_ENTRIES: usize = 2;
pub const MAX_ENTRIES: usize = 5;
const DEFAULT_VERIFY_TIMEOUT_SECS: u64 = 600;
const REF_ROOT: &str = "refs/omniget/arena";

fn aerr(code: &str, msg: impl std::fmt::Display) -> String {
    format!("{code}: {msg}")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn short_id(prefix: &str, n: usize) -> String {
    let s = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}{}", &s[..n.min(s.len())])
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySpec {
    pub instance_id: String,
    pub driver: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArenaCreate {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub verify_command: Option<String>,
    #[serde(default)]
    pub verify_timeout_secs: Option<u64>,
    #[serde(default)]
    pub runtime_mode: Option<String>,
    pub entries: Vec<EntrySpec>,
    #[serde(default)]
    pub source_thread_id: Option<String>,
    #[serde(default)]
    pub source_plan_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArenaEntry {
    pub entry_id: String,
    pub arena_id: String,
    pub position: i64,
    pub thread_id: String,
    pub instance_id: String,
    pub driver: String,
    pub model: Option<String>,
    pub label: String,
    pub status: String,
    pub started_at: Option<String>,
    pub turn_started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_usd: Option<f64>,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub commit_sha: Option<String>,
    pub files_changed: i64,
    pub additions: i64,
    pub deletions: i64,
    pub test_status: String,
    pub test_exit: Option<i64>,
    pub test_ms: Option<i64>,
    pub test_tail: Option<String>,
    pub votes: i64,
    pub winner: bool,
    pub error: Option<String>,
    pub cleaned_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArenaView {
    pub arena_id: String,
    pub title: String,
    pub prompt: String,
    pub project_id: String,
    pub repo_root: String,
    pub base_branch: Option<String>,
    pub base_commit: String,
    pub verify_command: Option<String>,
    pub verify_timeout_secs: i64,
    pub runtime_mode: String,
    pub source_thread_id: Option<String>,
    pub source_plan_id: Option<String>,
    pub status: String,
    pub winner_entry_id: Option<String>,
    pub applied_strategy: Option<String>,
    pub applied_commit: Option<String>,
    pub applied_branch: Option<String>,
    pub applied_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub entries: Vec<ArenaEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreRow {
    pub driver: String,
    pub model: Option<String>,
    pub runs: i64,
    pub wins: i64,
    pub votes: i64,
    pub tests_passed: i64,
    pub tests_run: i64,
    pub failures: i64,
    pub avg_duration_ms: Option<f64>,
    pub avg_cost_usd: Option<f64>,
    pub avg_tokens: Option<f64>,
    pub avg_lines: Option<f64>,
    pub last_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPreview {
    pub arena_id: String,
    pub entry_id: String,
    pub target_branch: String,
    pub target_head: String,
    pub winner_commit: String,
    pub checked_out_at: Option<String>,
    pub checkout_dirty: bool,
    pub blocker: Option<String>,
    pub already_applied: bool,
    pub conflicts: Vec<String>,
    pub files_changed: i64,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub arena: ArenaView,
    pub commit: String,
    pub target_branch: String,
    pub strategy: String,
    pub updated: String,
    pub took_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub arena: ArenaView,
    pub archived: Vec<String>,
    pub worktrees_removed: Vec<String>,
    pub saved: Vec<String>,
    pub branches_deleted: Vec<String>,
    pub branches_kept: Vec<String>,
    pub errors: Vec<String>,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS arenas (
    arena_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    prompt TEXT NOT NULL,
    project_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    base_branch TEXT,
    base_commit TEXT NOT NULL,
    verify_command TEXT,
    verify_timeout_secs INTEGER NOT NULL DEFAULT 600,
    runtime_mode TEXT NOT NULL,
    source_thread_id TEXT,
    source_plan_id TEXT,
    status TEXT NOT NULL,
    winner_entry_id TEXT,
    applied_strategy TEXT,
    applied_commit TEXT,
    applied_branch TEXT,
    applied_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entries (
    entry_id TEXT PRIMARY KEY,
    arena_id TEXT NOT NULL REFERENCES arenas(arena_id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    thread_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    driver TEXT NOT NULL,
    model TEXT,
    label TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT,
    turn_started_at TEXT,
    finished_at TEXT,
    duration_ms INTEGER,
    turns INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    branch TEXT,
    worktree_path TEXT,
    commit_sha TEXT,
    files_changed INTEGER NOT NULL DEFAULT 0,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    test_status TEXT NOT NULL DEFAULT 'none',
    test_exit INTEGER,
    test_ms INTEGER,
    test_tail TEXT,
    votes INTEGER NOT NULL DEFAULT 0,
    winner INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    cleaned_at TEXT
);
CREATE INDEX IF NOT EXISTS entries_arena ON entries(arena_id, position);
CREATE INDEX IF NOT EXISTS entries_thread ON entries(thread_id);
CREATE INDEX IF NOT EXISTS entries_driver ON entries(driver, model);
"#;

struct Db {
    conn: Mutex<Connection>,
}

fn db() -> Result<&'static Db, String> {
    static DB: OnceLock<Db> = OnceLock::new();
    if let Some(d) = DB.get() {
        return Ok(d);
    }
    let dir = omniget_core::core::llm::roster_store::llm_dir()
        .ok_or_else(|| aerr(ERR_ARENA, "no app data dir"))?;
    std::fs::create_dir_all(&dir).map_err(|e| aerr(ERR_ARENA, e))?;
    let conn = open_db(&dir.join("arena.db"))?;
    let _ = DB.set(Db {
        conn: Mutex::new(conn),
    });
    DB.get().ok_or_else(|| aerr(ERR_ARENA, "db init"))
}

pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| aerr("ERR_ARENA_DB", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| aerr("ERR_ARENA_DB", e))?;
    conn.execute_batch(SCHEMA)
        .map_err(|e| aerr("ERR_ARENA_DB", e))?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| aerr("ERR_ARENA_DB", e))?;
    Ok(conn)
}

fn with_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let d = db()?;
    let c = d.conn.lock().unwrap_or_else(|e| e.into_inner());
    f(&c).map_err(|e| aerr("ERR_ARENA_DB", e))
}

const ENTRY_COLS: &str = "entry_id, arena_id, position, thread_id, instance_id, driver, model, label, status, \
    started_at, turn_started_at, finished_at, duration_ms, turns, input_tokens, output_tokens, cached_tokens, \
    reasoning_tokens, cost_usd, branch, worktree_path, commit_sha, files_changed, additions, deletions, \
    test_status, test_exit, test_ms, test_tail, votes, winner, error, cleaned_at";

fn row_entry(r: &rusqlite::Row) -> rusqlite::Result<ArenaEntry> {
    Ok(ArenaEntry {
        entry_id: r.get(0)?,
        arena_id: r.get(1)?,
        position: r.get(2)?,
        thread_id: r.get(3)?,
        instance_id: r.get(4)?,
        driver: r.get(5)?,
        model: r.get(6)?,
        label: r.get(7)?,
        status: r.get(8)?,
        started_at: r.get(9)?,
        turn_started_at: r.get(10)?,
        finished_at: r.get(11)?,
        duration_ms: r.get(12)?,
        turns: r.get(13)?,
        input_tokens: r.get(14)?,
        output_tokens: r.get(15)?,
        cached_tokens: r.get(16)?,
        reasoning_tokens: r.get(17)?,
        cost_usd: r.get(18)?,
        branch: r.get(19)?,
        worktree_path: r.get(20)?,
        commit_sha: r.get(21)?,
        files_changed: r.get(22)?,
        additions: r.get(23)?,
        deletions: r.get(24)?,
        test_status: r.get(25)?,
        test_exit: r.get(26)?,
        test_ms: r.get(27)?,
        test_tail: r.get(28)?,
        votes: r.get(29)?,
        winner: r.get::<_, i64>(30)? != 0,
        error: r.get(31)?,
        cleaned_at: r.get(32)?,
    })
}

const ARENA_COLS: &str = "arena_id, title, prompt, project_id, repo_root, base_branch, base_commit, verify_command, \
    verify_timeout_secs, runtime_mode, source_thread_id, source_plan_id, status, winner_entry_id, applied_strategy, \
    applied_commit, applied_branch, applied_at, created_at, updated_at";

fn row_arena(r: &rusqlite::Row) -> rusqlite::Result<ArenaView> {
    Ok(ArenaView {
        arena_id: r.get(0)?,
        title: r.get(1)?,
        prompt: r.get(2)?,
        project_id: r.get(3)?,
        repo_root: r.get(4)?,
        base_branch: r.get(5)?,
        base_commit: r.get(6)?,
        verify_command: r.get(7)?,
        verify_timeout_secs: r.get(8)?,
        runtime_mode: r.get(9)?,
        source_thread_id: r.get(10)?,
        source_plan_id: r.get(11)?,
        status: r.get(12)?,
        winner_entry_id: r.get(13)?,
        applied_strategy: r.get(14)?,
        applied_commit: r.get(15)?,
        applied_branch: r.get(16)?,
        applied_at: r.get(17)?,
        created_at: r.get(18)?,
        updated_at: r.get(19)?,
        entries: Vec::new(),
    })
}

fn load_entries(c: &Connection, arena_id: &str) -> rusqlite::Result<Vec<ArenaEntry>> {
    let mut st = c.prepare(&format!(
        "SELECT {ENTRY_COLS} FROM entries WHERE arena_id = ?1 ORDER BY position ASC"
    ))?;
    let rows = st.query_map([arena_id], row_entry)?;
    rows.collect()
}

fn load_arena(arena_id: &str) -> Result<ArenaView, String> {
    let found = with_db(|c| {
        let a = c
            .query_row(
                &format!("SELECT {ARENA_COLS} FROM arenas WHERE arena_id = ?1"),
                [arena_id],
                row_arena,
            )
            .optional()?;
        match a {
            Some(mut a) => {
                a.entries = load_entries(c, arena_id)?;
                Ok(Some(a))
            }
            None => Ok(None),
        }
    })?;
    found.ok_or_else(|| aerr("ERR_ARENA_NOT_FOUND", format!("no arena {arena_id}")))
}

fn load_entry(entry_id: &str) -> Result<ArenaEntry, String> {
    with_db(|c| {
        c.query_row(
            &format!("SELECT {ENTRY_COLS} FROM entries WHERE entry_id = ?1"),
            [entry_id],
            row_entry,
        )
        .optional()
    })?
    .ok_or_else(|| aerr("ERR_ARENA_NOT_FOUND", format!("no entry {entry_id}")))
}

fn entry_of(arena: &ArenaView, entry_id: &str) -> Result<ArenaEntry, String> {
    arena
        .entries
        .iter()
        .find(|e| e.entry_id == entry_id)
        .cloned()
        .ok_or_else(|| aerr("ERR_ARENA_NOT_FOUND", format!("no entry {entry_id}")))
}

fn touch_arena(c: &Connection, arena_id: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE arenas SET updated_at = ?2 WHERE arena_id = ?1",
        params![arena_id, now_iso()],
    )?;
    Ok(())
}

fn set_entry_fields(entry_id: &str, sets: &[(&str, Value)]) -> Result<(), String> {
    if sets.is_empty() {
        return Ok(());
    }
    with_db(|c| {
        let assign: Vec<String> = sets
            .iter()
            .enumerate()
            .map(|(i, (k, _))| format!("{k} = ?{}", i + 2))
            .collect();
        let sql = format!(
            "UPDATE entries SET {} WHERE entry_id = ?1",
            assign.join(", ")
        );
        let mut vals: Vec<rusqlite::types::Value> = vec![entry_id.to_string().into()];
        for (_, v) in sets {
            vals.push(json_to_sql(v));
        }
        c.execute(&sql, rusqlite::params_from_iter(vals))?;
        let arena: Option<String> = c
            .query_row(
                "SELECT arena_id FROM entries WHERE entry_id = ?1",
                [entry_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(a) = arena {
            touch_arena(c, &a)?;
        }
        Ok(())
    })
}

fn json_to_sql(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        Value::Null => S::Null,
        Value::Bool(b) => S::Integer(*b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                S::Integer(i)
            } else {
                S::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => S::Text(s.clone()),
        other => S::Text(other.to_string()),
    }
}

#[derive(Default)]
struct Live {
    threads: HashMap<String, String>,
    busy: HashSet<String>,
}

fn live() -> &'static Mutex<Live> {
    static L: OnceLock<Mutex<Live>> = OnceLock::new();
    L.get_or_init(Default::default)
}

fn watch(thread_id: &str, entry_id: &str) {
    live()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .threads
        .insert(thread_id.to_string(), entry_id.to_string());
}

fn entry_for_thread(thread_id: &str) -> Option<String> {
    live()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .threads
        .get(thread_id)
        .cloned()
}

fn is_busy(entry_id: &str) -> bool {
    live()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .busy
        .contains(entry_id)
}

fn load_watch_set() {
    let rows: Vec<(String, String)> = with_db(|c| {
        let mut st = c.prepare(
            "SELECT e.thread_id, e.entry_id FROM entries e JOIN arenas a ON a.arena_id = e.arena_id
             WHERE a.status IN ('running','review') AND e.cleaned_at IS NULL",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })
    .unwrap_or_default();
    let mut l = live().lock().unwrap_or_else(|e| e.into_inner());
    for (t, e) in rows {
        l.threads.insert(t, e);
    }
}

fn emit_arena(app: &AppHandle, arena_id: &str) {
    if let Ok(a) = load_arena(arena_id) {
        let _ = app.emit(ARENA_EVENT, json!({ "arenaId": arena_id, "arena": a }));
    }
}

pub type Notify = Arc<dyn Fn(&str) + Send + Sync>;

fn notifier(app: &AppHandle) -> Notify {
    let app = app.clone();
    Arc::new(move |id: &str| emit_arena(&app, id))
}

fn ensure_watcher(engine: &Arc<ThreadsEngine>, notify: &Notify) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    load_watch_set();
    let mut rx = engine.subscribe();
    let notify = notify.clone();
    let engine = engine.clone();
    tauri::async_runtime::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("[arena] lagged by {n} events");
                    continue;
                }
                Err(RecvError::Closed) => break,
            };
            match ev.event {
                DomainEvent::TurnStarted {
                    thread_id,
                    started_at,
                    ..
                } => {
                    if let Some(entry) = entry_for_thread(&thread_id) {
                        let _ = set_entry_fields(
                            &entry,
                            &[
                                ("status", json!("running")),
                                ("turn_started_at", json!(started_at)),
                                ("finished_at", Value::Null),
                            ],
                        );
                        if let Ok(e) = load_entry(&entry) {
                            reopen_arena(&e.arena_id);
                            notify(&e.arena_id);
                        }
                    }
                }
                DomainEvent::WorktreeUpdated {
                    thread_id,
                    state,
                    path,
                    branch,
                    error,
                    ..
                } => {
                    let Some(entry) = entry_for_thread(&thread_id) else {
                        continue;
                    };
                    let mut sets: Vec<(&str, Value)> = Vec::new();
                    match state.as_str() {
                        "ready" => {
                            if let Some(p) = path {
                                sets.push(("worktree_path", json!(p)));
                            }
                            if let Some(b) = branch {
                                sets.push(("branch", json!(b)));
                            }
                        }
                        "failed" => {
                            sets.push(("status", json!("failed")));
                            sets.push(("error", json!(error)));
                            sets.push(("finished_at", json!(now_iso())));
                        }
                        _ => {}
                    }
                    if !sets.is_empty() {
                        let _ = set_entry_fields(&entry, &sets);
                        if let Ok(e) = load_entry(&entry) {
                            settle_arena(&e.arena_id);
                            notify(&e.arena_id);
                        }
                    }
                }
                DomainEvent::TurnCompleted { thread_id, .. } => {
                    if let Some(entry) = entry_for_thread(&thread_id) {
                        let notify = notify.clone();
                        let engine = engine.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(1200)).await;
                            finalize(&engine, &notify, &entry, true).await;
                        });
                    }
                }
                _ => {}
            }
        }
    });
}

fn reopen_arena(arena_id: &str) {
    let _ = with_db(|c| {
        c.execute(
            "UPDATE arenas SET status = 'running', updated_at = ?2 WHERE arena_id = ?1 AND status = 'review'",
            params![arena_id, now_iso()],
        )
    });
}

fn settle_arena(arena_id: &str) {
    let _ = with_db(|c| {
        let active: i64 = c.query_row(
            "SELECT COUNT(*) FROM entries WHERE arena_id = ?1 AND status IN ('preparing','running','testing')",
            [arena_id],
            |r| r.get(0),
        )?;
        if active == 0 {
            c.execute(
                "UPDATE arenas SET status = 'review', updated_at = ?2 WHERE arena_id = ?1 AND status = 'running'",
                params![arena_id, now_iso()],
            )?;
        }
        Ok(())
    });
}

fn gctx(path: &Path) -> GitCtx {
    GitCtx::Repo {
        work_tree: path.to_path_buf(),
    }
}

async fn git_out(ctx: &GitCtx, args: &[&str]) -> Result<String, String> {
    ctx.git()
        .args(args)
        .run()
        .await
        .map(|o| o.trimmed())
        .map_err(|e| e.to_string())
}

async fn rev_parse(ctx: &GitCtx, rev: &str) -> Option<String> {
    let o = ctx
        .git()
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .unchecked()
        .run()
        .await
        .ok()?;
    o.ok().then(|| o.trimmed()).filter(|s| !s.is_empty())
}

async fn identity_env(ctx: &GitCtx, inv: Invocation) -> Invocation {
    let has = repo::config_get(ctx, "user.email")
        .await
        .filter(|s| !s.trim().is_empty())
        .is_some();
    if has {
        inv
    } else {
        inv.env("GIT_AUTHOR_NAME", "OmniGet Arena")
            .env("GIT_AUTHOR_EMAIL", "arena@omniget.local")
            .env("GIT_COMMITTER_NAME", "OmniGet Arena")
            .env("GIT_COMMITTER_EMAIL", "arena@omniget.local")
    }
}

async fn commit_worktree(path: &Path, message: &str) -> Result<String, String> {
    let ctx = gctx(path);
    let mut last = String::new();
    for attempt in 0..4 {
        match ctx
            .git()
            .args(["add", "-A", "--", "."])
            .timeout(Some(Duration::from_secs(300)))
            .run()
            .await
        {
            Ok(_) => {
                last.clear();
                break;
            }
            Err(e) => {
                last = e.to_string();
                if !last.contains("index.lock") || attempt == 3 {
                    return Err(last);
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }
    let staged = ctx
        .git()
        .args(["diff", "--cached", "--quiet"])
        .unchecked()
        .run()
        .await
        .map_err(|e| e.to_string())?;
    if staged.code != Some(0) {
        let inv = ctx.git().args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--no-verify",
            "--no-gpg-sign",
            "-q",
            "-m",
            message,
        ]);
        identity_env(&ctx, inv)
            .await
            .timeout(Some(Duration::from_secs(300)))
            .run()
            .await
            .map_err(|e| e.to_string())?;
    }
    git_out(&ctx, &["rev-parse", "HEAD"]).await
}

fn arena_ref(arena_id: &str, entry_id: &str) -> String {
    format!("{REF_ROOT}/{arena_id}/{entry_id}")
}

async fn diff_stats(root: &Path, from: &str, to: &str) -> Option<(i64, i64, i64)> {
    let opts = DiffOptions {
        stat_only: true,
        ..Default::default()
    };
    let d = diff::diff_trees(&gctx(root), from, to, &opts).await.ok()?;
    Some((d.files.len() as i64, d.additions as i64, d.deletions as i64))
}

fn verify_shell(command: &str) -> Invocation {
    let inv = if cfg!(windows) {
        Invocation::new("cmd")
            .args(["/D", "/S", "/C"])
            .arg(format!("({command}) 2>&1"))
    } else {
        let sh = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sh".into());
        Invocation::new(&sh)
            .arg("-c")
            .arg(format!("exec 2>&1\n{command}"))
    };
    let mut inv = inv
        .env_remove("GIT_EDITOR")
        .env_remove("PAGER")
        .env("OMNIGET_ARENA", "1");
    for k in ["LC_ALL", "LANG"] {
        inv = match std::env::var_os(k) {
            Some(v) => inv.env(k, v),
            None => inv.env_remove(k),
        };
    }
    inv
}

struct VerifyOutcome {
    status: &'static str,
    exit: Option<i64>,
    ms: i64,
    tail: String,
}

async fn run_verify(dir: &Path, command: &str, timeout_secs: u64) -> VerifyOutcome {
    let t0 = Instant::now();
    let res = verify_shell(command)
        .cwd(dir)
        .timeout(Some(Duration::from_secs(timeout_secs.max(5))))
        .max_output(8 * 1024 * 1024)
        .unchecked()
        .run()
        .await;
    let ms = t0.elapsed().as_millis() as i64;
    match res {
        Ok(o) => {
            let mut text = o.text();
            text.push_str(&o.err_text());
            let code = o.code.map(|c| c as i64);
            VerifyOutcome {
                status: if code == Some(0) { "passed" } else { "failed" },
                exit: code,
                ms,
                tail: runner::tail(&strip_ansi(&text), 40, 8000),
            }
        }
        Err(e) => VerifyOutcome {
            status: "error",
            exit: None,
            ms,
            tail: e.to_string(),
        },
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for n in it.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

struct BusyGuard(String);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .busy
            .remove(&self.0);
    }
}

fn claim(entry_id: &str) -> Option<BusyGuard> {
    let mut l = live().lock().unwrap_or_else(|e| e.into_inner());
    if l.busy.contains(entry_id) {
        return None;
    }
    l.busy.insert(entry_id.to_string());
    Some(BusyGuard(entry_id.to_string()))
}

fn millis_between(a: &str, b: &str) -> Option<i64> {
    let a = chrono::DateTime::parse_from_rfc3339(a).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(b).ok()?;
    Some((b - a).num_milliseconds().max(0))
}

async fn finalize(engine: &Arc<ThreadsEngine>, notify: &Notify, entry_id: &str, run_tests: bool) {
    let Some(_guard) = claim(entry_id) else {
        return;
    };
    if let Err(e) = finalize_inner(engine, notify, entry_id, run_tests).await {
        tracing::warn!("[arena] finalize {entry_id}: {e}");
        let _ = set_entry_fields(entry_id, &[("error", json!(e))]);
    }
    if let Ok(e) = load_entry(entry_id) {
        settle_arena(&e.arena_id);
        notify(&e.arena_id);
    }
}

async fn finalize_inner(
    engine: &Arc<ThreadsEngine>,
    notify: &Notify,
    entry_id: &str,
    run_tests: bool,
) -> Result<(), String> {
    let entry = load_entry(entry_id)?;
    let arena = load_arena(&entry.arena_id)?;
    let thread_id = entry.thread_id.clone();
    let row = engine.read(|c| store::thread_row(c, &thread_id))?;
    let Some(row) = row else {
        return Err(aerr("ERR_ARENA_THREAD", "thread is gone"));
    };
    if row.active_turn_id.is_some() {
        return Ok(());
    }

    let usage = engine
        .read(|c| store::thread_usage(c, &thread_id))
        .unwrap_or_default();
    type TurnRow = (String, Option<String>, Option<String>, Option<String>);
    let turns: Vec<TurnRow> = engine
        .read(|c| {
            let mut st = c
                .prepare(
                    "SELECT state, started_at, completed_at, error_message FROM turns
                     WHERE thread_id = ?1 ORDER BY ordinal ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = st
                .query_map([&thread_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .unwrap_or_default();
    let usage_ms: i64 = usage.iter().filter_map(|u| u.duration_ms).sum();
    let wall_ms: i64 = turns
        .iter()
        .filter_map(|(_, s, c, _)| millis_between(s.as_deref()?, c.as_deref()?))
        .sum();
    let duration = if wall_ms > 0 { wall_ms } else { usage_ms };
    let cost: Option<f64> = {
        let v: Vec<f64> = usage.iter().filter_map(|u| u.cost_usd).collect();
        (!v.is_empty()).then(|| v.iter().sum())
    };
    let last = turns.last().cloned();
    let (last_state, last_error) = last
        .map(|(s, _, _, e)| (s, e))
        .unwrap_or_else(|| ("failed".into(), None));
    let status = match last_state.as_str() {
        "completed" => "done",
        "interrupted" | "cancelled" => "interrupted",
        "pending" | "running" => return Ok(()),
        _ => "failed",
    };
    let mut sets: Vec<(&str, Value)> = vec![
        ("turns", json!(turns.len() as i64)),
        (
            "input_tokens",
            json!(usage.iter().map(|u| u.input_tokens).sum::<i64>()),
        ),
        (
            "output_tokens",
            json!(usage.iter().map(|u| u.output_tokens).sum::<i64>()),
        ),
        (
            "cached_tokens",
            json!(usage.iter().map(|u| u.cached_input_tokens).sum::<i64>()),
        ),
        (
            "reasoning_tokens",
            json!(usage.iter().map(|u| u.reasoning_output_tokens).sum::<i64>()),
        ),
        ("cost_usd", json!(cost)),
        ("duration_ms", json!(duration)),
        ("error", json!(last_error)),
    ];

    let wt = row
        .worktree_path
        .clone()
        .or(entry.worktree_path.clone())
        .map(PathBuf::from)
        .filter(|p| p.is_dir() && worktree::is_omniget_worktree(p));
    let root = PathBuf::from(&arena.repo_root);
    if let Some(dir) = &wt {
        sets.push(("worktree_path", json!(dir.to_string_lossy())));
        if let Ok(st) = repo::status(dir).await {
            if let Some(b) = st.branch {
                sets.push(("branch", json!(b)));
            }
        }
        let msg = format!(
            "arena: {}\n\n{} ({}{}), arena {}",
            one_line(&arena.title, 72),
            entry.label,
            entry.driver,
            entry
                .model
                .as_deref()
                .map(|m| format!("/{m}"))
                .unwrap_or_default(),
            arena.arena_id
        );
        match commit_worktree(dir, &msg).await {
            Ok(sha) => {
                let _ = gctx(dir)
                    .git()
                    .args(["update-ref", &arena_ref(&arena.arena_id, entry_id), &sha])
                    .run()
                    .await;
                if let Some((f, a, d)) = diff_stats(&root, &arena.base_commit, &sha).await {
                    sets.push(("files_changed", json!(f)));
                    sets.push(("additions", json!(a)));
                    sets.push(("deletions", json!(d)));
                }
                sets.push(("commit_sha", json!(sha)));
            }
            Err(e) => sets.push(("error", json!(format!("commit: {e}")))),
        }
    }

    let verify = arena
        .verify_command
        .clone()
        .filter(|c| !c.trim().is_empty());
    let will_test = run_tests && verify.is_some() && wt.is_some() && status != "failed";
    sets.push(("status", json!(if will_test { "testing" } else { status })));
    if !will_test {
        sets.push(("finished_at", json!(now_iso())));
    } else {
        sets.push(("test_status", json!("running")));
    }
    set_entry_fields(entry_id, &sets)?;
    notify(&arena.arena_id);

    if let (true, Some(cmd), Some(dir)) = (will_test, verify, wt) {
        let out = run_verify(&dir, &cmd, arena.verify_timeout_secs.max(5) as u64).await;
        set_entry_fields(
            entry_id,
            &[
                ("test_status", json!(out.status)),
                ("test_exit", json!(out.exit)),
                ("test_ms", json!(out.ms)),
                ("test_tail", json!(out.tail)),
                ("status", json!(status)),
                ("finished_at", json!(now_iso())),
            ],
        )?;
    }
    Ok(())
}

fn one_line(s: &str, max: usize) -> String {
    let l = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let l = l.trim_start_matches('#').trim();
    if l.chars().count() > max {
        let mut t: String = l.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        l.to_string()
    }
}

fn reconcile(engine: &Arc<ThreadsEngine>, notify: &Notify, arena: &ArenaView) {
    for e in &arena.entries {
        if !matches!(e.status.as_str(), "preparing" | "running" | "testing") || is_busy(&e.entry_id)
        {
            continue;
        }
        let row = engine
            .read(|c| store::thread_row(c, &e.thread_id))
            .ok()
            .flatten();
        let settled = match &row {
            Some(r) => r.active_turn_id.is_none() && r.turn_count > 0,
            None => true,
        };
        if !settled {
            continue;
        }
        if row.is_none() {
            let _ = set_entry_fields(
                &e.entry_id,
                &[
                    ("status", json!("failed")),
                    ("error", json!("thread deleted")),
                    ("finished_at", json!(now_iso())),
                ],
            );
            continue;
        }
        let notify = notify.clone();
        let engine = engine.clone();
        let id = e.entry_id.clone();
        tauri::async_runtime::spawn(async move { finalize(&engine, &notify, &id, true).await });
    }
    settle_arena(&arena.arena_id);
}

async fn dispatch(engine: &ThreadsEngine, cmd: Value) -> Result<(), String> {
    let env: CommandEnvelope =
        serde_json::from_value(cmd).map_err(|e| aerr("ERR_ARENA_INVALID", e))?;
    engine.dispatch(env).await.map(|_| ())
}

fn norm_mode(m: Option<&str>) -> &'static str {
    match m.unwrap_or("") {
        "approval-required" => "approval-required",
        "auto" => "auto",
        "full-access" => "full-access",
        _ => "auto-accept-edits",
    }
}

async fn resolve_project(
    engine: &Arc<ThreadsEngine>,
    req: &ArenaCreate,
) -> Result<(String, PathBuf), String> {
    if let Some(pid) = req.project_id.as_deref().filter(|p| !p.is_empty()) {
        let p = engine
            .read(|c| store::project_row(c, pid))?
            .ok_or_else(|| aerr("ERR_ARENA_INVALID", format!("no project {pid}")))?;
        if p.workspace_root.trim().is_empty() {
            return Err(aerr("ERR_ARENA_INVALID", "the project has no folder"));
        }
        return Ok((p.project_id, PathBuf::from(p.workspace_root)));
    }
    let root = req
        .workspace_root
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| aerr("ERR_ARENA_INVALID", "pick a project or a folder"))?;
    let root = PathBuf::from(root);
    if !root.is_absolute() || !root.is_dir() {
        return Err(aerr(
            "ERR_ARENA_INVALID",
            "the folder must be an absolute path",
        ));
    }
    let snap = engine.snapshot().await?;
    if let Some(p) = snap
        .projects
        .iter()
        .find(|p| Path::new(&p.workspace_root) == root.as_path())
    {
        return Ok((p.project_id.clone(), root));
    }
    let pid = short_id("prj_", 16);
    let title = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Arena".into());
    dispatch(
        engine,
        json!({ "type": "project.create", "projectId": pid, "title": title,
                "workspaceRoot": root.to_string_lossy() }),
    )
    .await?;
    Ok((pid, root))
}

pub fn unique_labels(raw: &[String]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let totals: HashMap<&String, usize> = raw.iter().fold(HashMap::new(), |mut m, l| {
        *m.entry(l).or_default() += 1;
        m
    });
    raw.iter()
        .map(|l| {
            let n = seen.entry(l.clone()).or_default();
            *n += 1;
            if totals.get(l).copied().unwrap_or(0) > 1 {
                format!("{l} · {n}")
            } else {
                l.clone()
            }
        })
        .collect()
}

#[tauri::command]
pub async fn arena_create(app: AppHandle, request: ArenaCreate) -> Result<ArenaView, String> {
    let host = threads_host::get(&app)?;
    let labels: Vec<(String, String)> = host
        .instances()
        .into_iter()
        .map(|i| (i.instance.id.clone(), i.instance.label.clone()))
        .collect();
    create_core(&host.engine, &notifier(&app), request, &labels).await
}

pub async fn create_core(
    engine: &Arc<ThreadsEngine>,
    notify: &Notify,
    request: ArenaCreate,
    instance_labels: &[(String, String)],
) -> Result<ArenaView, String> {
    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(aerr("ERR_ARENA_INVALID", "empty prompt"));
    }
    if request.entries.len() < MIN_ENTRIES || request.entries.len() > MAX_ENTRIES {
        return Err(aerr(
            "ERR_ARENA_INVALID",
            format!("pick between {MIN_ENTRIES} and {MAX_ENTRIES} tools"),
        ));
    }
    ensure_watcher(engine, notify);
    let (project_id, root) = resolve_project(engine, &request).await?;
    let st = repo::status(&root).await.map_err(|e| e.to_string())?;
    if !st.is_repo {
        return Err(aerr(
            "ERR_ARENA_NOT_REPO",
            "the arena needs a git repository (each tool gets its own worktree)",
        ));
    }
    let top = st.root.clone().unwrap_or(root.clone());
    let ctx = gctx(&top);
    let base_branch = request
        .base_branch
        .clone()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .or(st.branch.clone());
    let base_rev = base_branch.clone().unwrap_or_else(|| "HEAD".into());
    let base_commit = rev_parse(&ctx, &base_rev).await.ok_or_else(|| {
        aerr(
            "ERR_ARENA_INVALID",
            format!("{base_rev} is not a commit (the repository needs one)"),
        )
    })?;
    let base_branch = match base_branch {
        Some(b) if rev_parse(&ctx, &format!("refs/heads/{b}")).await.is_some() => Some(b),
        _ => None,
    };

    let arena_id = short_id("arn_", 12);
    let title = request
        .title
        .clone()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| one_line(&prompt, 80));
    let mode = norm_mode(request.runtime_mode.as_deref());
    let now = now_iso();
    let raw_labels: Vec<String> = request
        .entries
        .iter()
        .map(|e| {
            e.label
                .clone()
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| {
                    let base = instance_labels
                        .iter()
                        .find(|(id, _)| *id == e.instance_id)
                        .map(|(_, l)| l.clone())
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| e.driver.clone());
                    match e.model.as_deref().filter(|m| !m.is_empty()) {
                        Some(m) => format!("{base} · {m}"),
                        None => base,
                    }
                })
        })
        .collect();
    let labels = unique_labels(&raw_labels);
    let entries: Vec<(String, String, &EntrySpec, String)> = request
        .entries
        .iter()
        .zip(labels)
        .map(|(e, l)| (short_id("ent_", 12), short_id("thr_", 16), e, l))
        .collect();

    with_db(|c| {
        c.execute(
            &format!("INSERT INTO arenas ({ARENA_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'running',NULL,NULL,NULL,NULL,NULL,?13,?13)"),
            params![
                arena_id,
                title,
                prompt,
                project_id,
                top.to_string_lossy(),
                base_branch,
                base_commit,
                request.verify_command.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                request.verify_timeout_secs.unwrap_or(DEFAULT_VERIFY_TIMEOUT_SECS) as i64,
                mode,
                request.source_thread_id,
                request.source_plan_id,
                now,
            ],
        )?;
        for (pos, (eid, tid, spec, label)) in entries.iter().enumerate() {
            c.execute(
                "INSERT INTO entries (entry_id, arena_id, position, thread_id, instance_id, driver, model, label, status, started_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'preparing',?9)",
                params![
                    eid,
                    arena_id,
                    pos as i64,
                    tid,
                    spec.instance_id,
                    spec.driver,
                    spec.model.as_deref().filter(|m| !m.is_empty()),
                    label,
                    now,
                ],
            )?;
        }
        Ok(())
    })?;
    for (eid, tid, _, _) in &entries {
        watch(tid, eid);
    }

    for (eid, tid, spec, label) in &entries {
        let create = json!({
            "type": "thread.create",
            "threadId": tid,
            "projectId": project_id,
            "title": format!("Arena · {label}"),
            "instanceId": spec.instance_id,
            "driver": spec.driver,
            "model": spec.model.as_deref().filter(|m| !m.is_empty()),
            "agentId": spec.agent_id,
            "runtimeMode": mode,
            "worktree": true,
            "baseBranch": base_branch.clone().unwrap_or_else(|| base_commit.clone()),
        });
        let res = match dispatch(engine, create).await {
            Ok(()) => {
                dispatch(
                    engine,
                    json!({
                        "type": "thread.turn.start",
                        "threadId": tid,
                        "text": prompt,
                        "model": spec.model.as_deref().filter(|m| !m.is_empty()),
                    }),
                )
                .await
            }
            Err(e) => Err(e),
        };
        if let Err(e) = res {
            let _ = set_entry_fields(
                eid,
                &[
                    ("status", json!("failed")),
                    ("error", json!(e)),
                    ("finished_at", json!(now_iso())),
                ],
            );
        }
    }
    settle_arena(&arena_id);
    notify(&arena_id);
    load_arena(&arena_id)
}

#[tauri::command]
pub async fn arena_list(
    app: AppHandle,
    limit: Option<u32>,
    project_id: Option<String>,
) -> Result<Vec<ArenaView>, String> {
    let host = threads_host::get(&app)?;
    ensure_watcher(&host.engine, &notifier(&app));
    let limit = limit.unwrap_or(50).clamp(1, 500) as i64;
    let mut list = with_db(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {ARENA_COLS} FROM arenas WHERE (?1 IS NULL OR project_id = ?1)
             ORDER BY created_at DESC LIMIT ?2"
        ))?;
        let rows = st.query_map(params![project_id, limit], row_arena)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })?;
    for a in list.iter_mut() {
        a.entries = with_db(|c| load_entries(c, &a.arena_id))?;
    }
    Ok(list)
}

#[tauri::command]
pub async fn arena_get(app: AppHandle, arena_id: String) -> Result<ArenaView, String> {
    let host = threads_host::get(&app)?;
    ensure_watcher(&host.engine, &notifier(&app));
    let a = load_arena(&arena_id)?;
    if a.status == "running" || a.status == "review" {
        reconcile(&host.engine, &notifier(&app), &a);
    }
    load_arena(&arena_id)
}

#[tauri::command]
pub async fn arena_diff(
    app: AppHandle,
    arena_id: String,
    entry_id: String,
    options: Option<DiffOptions>,
) -> Result<DiffResult, String> {
    let _ = threads_host::get(&app)?;
    let a = load_arena(&arena_id)?;
    let e = entry_of(&a, &entry_id)?;
    let opts = options.unwrap_or_default();
    let root = PathBuf::from(&a.repo_root);
    let live_dir = e
        .worktree_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let running = matches!(e.status.as_str(), "preparing" | "running");
    if let (true, Some(dir)) = (running || e.commit_sha.is_none(), live_dir) {
        let ctx = omniget_core::core::vcs::checkpoint::context_for(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let tree = omniget_core::core::vcs::checkpoint::live_tree(&ctx)
            .await
            .map_err(|e| e.to_string())?;
        return diff::diff_trees(&ctx, &a.base_commit, &tree, &opts)
            .await
            .map_err(|e| e.to_string());
    }
    let to = match &e.commit_sha {
        Some(s) => s.clone(),
        None => return Ok(DiffResult::default()),
    };
    diff::diff_trees(&gctx(&root), &a.base_commit, &to, &opts)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn arena_vote(
    app: AppHandle,
    arena_id: String,
    entry_id: String,
    delta: Option<i64>,
) -> Result<ArenaView, String> {
    let d = delta.unwrap_or(1).clamp(-1, 1);
    with_db(|c| {
        c.execute(
            "UPDATE entries SET votes = MAX(0, votes + ?3) WHERE arena_id = ?1 AND entry_id = ?2",
            params![arena_id, entry_id, d],
        )?;
        touch_arena(c, &arena_id)
    })?;
    emit_arena(&app, &arena_id);
    load_arena(&arena_id)
}

#[tauri::command]
pub async fn arena_pick(
    app: AppHandle,
    arena_id: String,
    entry_id: Option<String>,
) -> Result<ArenaView, String> {
    let a = load_arena(&arena_id)?;
    if a.status == "applied" {
        return Err(aerr("ERR_ARENA_APPLIED", "the winner was already applied"));
    }
    if let Some(e) = &entry_id {
        entry_of(&a, e)?;
    }
    with_db(|c| {
        c.execute(
            "UPDATE entries SET winner = CASE WHEN entry_id = ?2 THEN 1 ELSE 0 END WHERE arena_id = ?1",
            params![arena_id, entry_id],
        )?;
        c.execute(
            "UPDATE arenas SET winner_entry_id = ?2, updated_at = ?3 WHERE arena_id = ?1",
            params![arena_id, entry_id, now_iso()],
        )?;
        Ok(())
    })?;
    emit_arena(&app, &arena_id);
    load_arena(&arena_id)
}

#[tauri::command]
pub async fn arena_verify(
    app: AppHandle,
    arena_id: String,
    entry_id: Option<String>,
    command: Option<String>,
) -> Result<ArenaView, String> {
    let host = threads_host::get(&app)?;
    ensure_watcher(&host.engine, &notifier(&app));
    if let Some(cmd) = command {
        let cmd = cmd.trim().to_string();
        with_db(|c| {
            c.execute(
                "UPDATE arenas SET verify_command = ?2, updated_at = ?3 WHERE arena_id = ?1",
                params![arena_id, (!cmd.is_empty()).then_some(cmd), now_iso()],
            )
        })?;
    }
    let a = load_arena(&arena_id)?;
    let cmd = a
        .verify_command
        .clone()
        .ok_or_else(|| aerr("ERR_ARENA_INVALID", "no verification command"))?;
    let targets: Vec<ArenaEntry> = a
        .entries
        .iter()
        .filter(|e| {
            entry_id
                .as_deref()
                .map(|id| id == e.entry_id)
                .unwrap_or(true)
        })
        .filter(|e| !matches!(e.status.as_str(), "preparing" | "running"))
        .cloned()
        .collect();
    for e in targets {
        let Some(dir) = e
            .worktree_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
        else {
            continue;
        };
        let Some(guard) = claim(&e.entry_id) else {
            continue;
        };
        set_entry_fields(
            &e.entry_id,
            &[
                ("test_status", json!("running")),
                ("status", json!("testing")),
            ],
        )?;
        emit_arena(&app, &arena_id);
        let app2 = app.clone();
        let cmd = cmd.clone();
        let timeout = a.verify_timeout_secs.max(5) as u64;
        let prev = e.status.clone();
        let arena_id2 = arena_id.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = guard;
            let out = run_verify(&dir, &cmd, timeout).await;
            let _ = set_entry_fields(
                &e.entry_id,
                &[
                    ("test_status", json!(out.status)),
                    ("test_exit", json!(out.exit)),
                    ("test_ms", json!(out.ms)),
                    ("test_tail", json!(out.tail)),
                    (
                        "status",
                        json!(if prev == "testing" {
                            "done"
                        } else {
                            prev.as_str()
                        }),
                    ),
                ],
            );
            settle_arena(&arena_id2);
            emit_arena(&app2, &arena_id2);
        });
    }
    load_arena(&arena_id)
}

#[tauri::command]
pub async fn arena_stop(app: AppHandle, arena_id: String) -> Result<ArenaView, String> {
    let host = threads_host::get(&app)?;
    let a = load_arena(&arena_id)?;
    for e in a
        .entries
        .iter()
        .filter(|e| e.status == "running" || e.status == "preparing")
    {
        let _ = dispatch(
            &host.engine,
            json!({ "type": "thread.turn.interrupt", "threadId": e.thread_id }),
        )
        .await;
    }
    load_arena(&arena_id)
}

async fn winner_commit(a: &ArenaView, e: &ArenaEntry) -> Result<String, String> {
    if let Some(dir) = e
        .worktree_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir() && worktree::is_omniget_worktree(p))
    {
        if repo::status(&dir).await.map(|s| s.dirty).unwrap_or(false) {
            let sha = commit_worktree(
                &dir,
                &format!("arena: {} ({})", one_line(&a.title, 72), e.label),
            )
            .await?;
            let _ = gctx(&dir)
                .git()
                .args(["update-ref", &arena_ref(&a.arena_id, &e.entry_id), &sha])
                .run()
                .await;
            set_entry_fields(&e.entry_id, &[("commit_sha", json!(sha))])?;
            return Ok(sha);
        }
    }
    let ctx = gctx(Path::new(&a.repo_root));
    if let Some(s) = rev_parse(&ctx, &arena_ref(&a.arena_id, &e.entry_id)).await {
        return Ok(s);
    }
    e.commit_sha
        .clone()
        .ok_or_else(|| aerr("ERR_ARENA_NOTHING", "this entry has no commit yet"))
}

pub fn parse_merge_tree(stdout: &str, code: Option<i32>) -> Result<(String, Vec<String>), String> {
    let mut lines = stdout.lines();
    let tree = lines.next().unwrap_or("").trim().to_string();
    match code {
        Some(0) if !tree.is_empty() => Ok((tree, Vec::new())),
        Some(1) => {
            let files: Vec<String> = lines
                .map(str::trim)
                .take_while(|l| !l.is_empty())
                .map(String::from)
                .collect();
            Ok((tree, files))
        }
        _ => Err(aerr(
            "ERR_ARENA_GIT",
            "git merge-tree failed (git 2.38 or newer is needed)",
        )),
    }
}

async fn merge_tree(
    ctx: &GitCtx,
    ours: &str,
    theirs: &str,
) -> Result<(String, Vec<String>), String> {
    let o = ctx
        .git()
        .args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            ours,
            theirs,
        ])
        .unchecked()
        .timeout(Some(Duration::from_secs(120)))
        .run()
        .await
        .map_err(|e| e.to_string())?;
    parse_merge_tree(&o.text(), o.code).map_err(|e| {
        let why = o.err_text();
        if why.trim().is_empty() {
            e
        } else {
            format!("{e}: {}", why.trim())
        }
    })
}

async fn is_ancestor(ctx: &GitCtx, a: &str, b: &str) -> bool {
    ctx.git()
        .args(["merge-base", "--is-ancestor", a, b])
        .unchecked()
        .run()
        .await
        .map(|o| o.ok())
        .unwrap_or(false)
}

async fn checkout_of(root: &Path, branch: &str) -> Option<PathBuf> {
    worktree::list(root)
        .await
        .ok()?
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch))
        .map(|w| w.path)
}

fn target_of(a: &ArenaView, target: Option<String>) -> Result<String, String> {
    let t = target
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or(a.base_branch.clone())
        .ok_or_else(|| {
            aerr(
                "ERR_ARENA_INVALID",
                "the arena started from a detached HEAD: pick a target branch",
            )
        })?;
    if t.starts_with('-') || t.contains("..") {
        return Err(aerr("ERR_ARENA_INVALID", format!("bad branch {t}")));
    }
    Ok(t)
}

async fn winner_branch(root: &Path, e: &ArenaEntry, win: &str) -> Option<String> {
    let b = e.branch.as_deref().filter(|b| !b.is_empty())?;
    (rev_parse(&gctx(root), &format!("refs/heads/{b}"))
        .await
        .as_deref()
        == Some(win))
    .then(|| b.to_string())
}

#[tauri::command]
pub async fn arena_apply_preview(
    app: AppHandle,
    arena_id: String,
    entry_id: String,
    target_branch: Option<String>,
) -> Result<ApplyPreview, String> {
    let _ = threads_host::get(&app)?;
    preview_core(&arena_id, &entry_id, target_branch).await
}

pub async fn preview_core(
    arena_id: &str,
    entry_id: &str,
    target_branch: Option<String>,
) -> Result<ApplyPreview, String> {
    let a = load_arena(arena_id)?;
    let e = entry_of(&a, entry_id)?;
    let target = target_of(&a, target_branch)?;
    let root = PathBuf::from(&a.repo_root);
    let ctx = gctx(&root);
    let head = rev_parse(&ctx, &format!("refs/heads/{target}"))
        .await
        .ok_or_else(|| aerr("ERR_ARENA_INVALID", format!("no branch {target}")))?;
    let win = winner_commit(&a, &e).await?;
    let already = is_ancestor(&ctx, &win, &head).await;
    let (_, conflicts) = if already {
        (String::new(), Vec::new())
    } else {
        merge_tree(&ctx, &head, &win).await?
    };
    let checked = checkout_of(&root, &target).await;
    let (dirty, blocker) = match &checked {
        Some(p) => match winner_branch(&root, &e, &win).await {
            Some(b) => match integrate::preview(p, &b).await {
                Ok(pre) => (!pre.dirty.is_empty(), pre.blocker()),
                Err(err) => (false, Some(err.to_string())),
            },
            None => (repo::status(p).await.map(|s| s.dirty).unwrap_or(true), None),
        },
        None => (false, None),
    };
    let (f, ad, de) = diff_stats(&root, &a.base_commit, &win)
        .await
        .unwrap_or((0, 0, 0));
    Ok(ApplyPreview {
        arena_id: arena_id.to_string(),
        entry_id: entry_id.to_string(),
        target_branch: target,
        target_head: head,
        winner_commit: win,
        checked_out_at: checked.map(|p| p.to_string_lossy().into_owned()),
        checkout_dirty: dirty,
        blocker,
        already_applied: already,
        conflicts,
        files_changed: f,
        additions: ad,
        deletions: de,
    })
}

pub async fn apply_core(
    root: &Path,
    target: &str,
    win: &str,
    strategy: &str,
    subject: &str,
    body: &str,
) -> Result<String, String> {
    let ctx = gctx(root);
    let old = rev_parse(&ctx, &format!("refs/heads/{target}"))
        .await
        .ok_or_else(|| aerr("ERR_ARENA_INVALID", format!("no branch {target}")))?;
    if is_ancestor(&ctx, win, &old).await {
        return Err(aerr(
            "ERR_ARENA_NOTHING",
            format!("{target} already has this work"),
        ));
    }
    if let Some(p) = checkout_of(root, target).await {
        return Err(aerr(
            "ERR_ARENA_CHECKOUT",
            format!(
                "{target} is checked out at {}; it is merged there",
                p.display()
            ),
        ));
    }
    let (tree, conflicts) = merge_tree(&ctx, &old, win).await?;
    if !conflicts.is_empty() {
        return Err(aerr(
            "ERR_ARENA_CONFLICT",
            format!("conflicts in {}", conflicts.join(", ")),
        ));
    }
    let mut inv = ctx
        .git()
        .args(["commit-tree", "--no-gpg-sign", &tree, "-p", &old]);
    if strategy == "merge" {
        inv = inv.args(["-p", win]);
    }
    let inv = inv.args(["-m", subject, "-m", body]);
    let new = identity_env(&ctx, inv)
        .await
        .run()
        .await
        .map_err(|e| e.to_string())?
        .trimmed();
    ctx.git()
        .args([
            "update-ref",
            "-m",
            "omniget arena: apply winner",
            &format!("refs/heads/{target}"),
            &new,
            &old,
        ])
        .run()
        .await
        .map_err(|e| e.to_string())?;
    Ok(new)
}

pub async fn apply_in_checkout(
    root: &Path,
    checkout: &Path,
    e: &ArenaEntry,
    win: &str,
    strategy: &str,
    subject: &str,
) -> Result<String, String> {
    let (branch, temp) = match winner_branch(root, e, win).await {
        Some(b) => (b, false),
        None => {
            let b = format!("omniget/arena-{}", e.entry_id);
            gctx(root)
                .git()
                .args(["branch", "-f", "--", &b, win])
                .run()
                .await
                .map_err(|x| x.to_string())?;
            (b, true)
        }
    };
    let strat = if strategy == "squash" {
        integrate::Strategy::Squash
    } else {
        integrate::Strategy::Merge
    };
    let res = integrate::integrate(checkout, &branch, strat, subject).await;
    if temp {
        let _ = integrate::delete_branch(root, &branch).await;
    }
    let out = res.map_err(|x| {
        let why = x.to_string();
        if why.contains("uncommitted") {
            aerr("ERR_ARENA_DIRTY", why)
        } else {
            aerr("ERR_ARENA_BLOCKED", why)
        }
    })?;
    match out.status.as_str() {
        "merged" => out
            .commit
            .ok_or_else(|| aerr("ERR_ARENA_GIT", "merged without a commit")),
        "up_to_date" => Err(aerr(
            "ERR_ARENA_NOTHING",
            format!("{} already has this work", out.target_branch),
        )),
        "conflict" => Err(aerr(
            "ERR_ARENA_CONFLICT",
            format!(
                "conflicts in {}{}",
                out.conflicts.join(", "),
                if out.clean {
                    "; your checkout is back as it was"
                } else {
                    "; check your checkout (git status)"
                }
            ),
        )),
        _ => Err(aerr(
            "ERR_ARENA_GIT",
            format!("{} {}", out.status, out.output.trim()),
        )),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn arena_apply(
    app: AppHandle,
    arena_id: String,
    entry_id: String,
    strategy: Option<String>,
    target_branch: Option<String>,
    confirm: Option<bool>,
    touch_checkout: Option<bool>,
    message: Option<String>,
) -> Result<ApplyResult, String> {
    if !confirm.unwrap_or(false) {
        return Err(aerr("ERR_ARENA_CONFIRM", "confirm the apply first"));
    }
    let _ = threads_host::get(&app)?;
    let r = apply_entry(
        &arena_id,
        &entry_id,
        strategy.as_deref(),
        target_branch,
        touch_checkout.unwrap_or(false),
        message,
    )
    .await?;
    crate::llm_audit::arena_action(
        "arena.applied",
        &arena_id,
        json!({
            "entry_id": entry_id,
            "strategy": r.strategy,
            "target_branch": r.target_branch,
            "updated": r.updated,
        }),
    );
    emit_arena(&app, &arena_id);
    Ok(r)
}

pub async fn apply_entry(
    arena_id: &str,
    entry_id: &str,
    strategy: Option<&str>,
    target_branch: Option<String>,
    touch_checkout: bool,
    message: Option<String>,
) -> Result<ApplyResult, String> {
    let a = load_arena(arena_id)?;
    let e = entry_of(&a, entry_id)?;
    let strategy = match strategy {
        Some("squash") | Some("cherry-pick") => "squash",
        _ => "merge",
    };
    let target = target_of(&a, target_branch)?;
    let root = PathBuf::from(&a.repo_root);
    let win = winner_commit(&a, &e).await?;
    let subject = message
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("Arena: {}", one_line(&a.title, 64)));
    let body = format!(
        "Winner: {} ({}{})\nArena: {}",
        e.label,
        e.driver,
        e.model
            .as_deref()
            .map(|m| format!("/{m}"))
            .unwrap_or_default(),
        a.arena_id
    );
    let t0 = Instant::now();
    let (new, updated) = match checkout_of(&root, &target).await {
        Some(p) => {
            if !touch_checkout {
                return Err(aerr(
                    "ERR_ARENA_CHECKOUT",
                    format!(
                        "{target} is checked out at {}; confirm to merge there",
                        p.display()
                    ),
                ));
            }
            let c = apply_in_checkout(&root, &p, &e, &win, strategy, &subject).await?;
            (c, "checkout")
        }
        None => (
            apply_core(&root, &target, &win, strategy, &subject, &body).await?,
            "ref",
        ),
    };
    let took_ms = t0.elapsed().as_millis() as u64;
    let now = now_iso();
    with_db(|c| {
        c.execute(
            "UPDATE entries SET winner = CASE WHEN entry_id = ?2 THEN 1 ELSE 0 END WHERE arena_id = ?1",
            params![arena_id, entry_id],
        )?;
        c.execute(
            "UPDATE arenas SET status = 'applied', winner_entry_id = ?2, applied_strategy = ?3,
             applied_commit = ?4, applied_branch = ?5, applied_at = ?6, updated_at = ?6 WHERE arena_id = ?1",
            params![arena_id, entry_id, strategy, new, target, now],
        )?;
        Ok(())
    })?;
    Ok(ApplyResult {
        arena: load_arena(arena_id)?,
        commit: new,
        target_branch: target,
        strategy: strategy.into(),
        updated: updated.into(),
        took_ms,
    })
}

#[tauri::command]
pub async fn arena_cleanup(
    app: AppHandle,
    arena_id: String,
    scope: Option<String>,
    delete_branches: Option<bool>,
) -> Result<CleanupResult, String> {
    let host = threads_host::get(&app)?;
    let r = cleanup_core(
        &host.engine,
        &arena_id,
        scope.as_deref() == Some("all"),
        delete_branches.unwrap_or(false),
    )
    .await?;
    crate::llm_audit::arena_action(
        "arena.cleaned",
        &arena_id,
        json!({
            "scope": if scope.as_deref() == Some("all") { "all" } else { "losers" },
            "archived": r.archived.len(),
            "worktrees_removed": r.worktrees_removed.len(),
            "branches_deleted": r.branches_deleted.len(),
            "errors": r.errors.len(),
        }),
    );
    emit_arena(&app, &arena_id);
    Ok(r)
}

pub async fn cleanup_core(
    engine: &Arc<ThreadsEngine>,
    arena_id: &str,
    all: bool,
    delete_branches: bool,
) -> Result<CleanupResult, String> {
    let a = load_arena(arena_id)?;
    let winner = a.winner_entry_id.clone();
    let targets: Vec<ArenaEntry> = a
        .entries
        .iter()
        .filter(|e| e.cleaned_at.is_none())
        .filter(|e| all || winner.as_deref() != Some(e.entry_id.as_str()))
        .cloned()
        .collect();
    let root = PathBuf::from(&a.repo_root);
    let ctx = gctx(&root);
    let mut out = CleanupResult {
        arena: a.clone(),
        archived: Vec::new(),
        worktrees_removed: Vec::new(),
        saved: Vec::new(),
        branches_deleted: Vec::new(),
        branches_kept: Vec::new(),
        errors: Vec::new(),
    };
    for e in &targets {
        if matches!(e.status.as_str(), "running" | "preparing" | "testing") {
            let _ = dispatch(
                engine,
                json!({ "type": "thread.turn.interrupt", "threadId": e.thread_id }),
            )
            .await;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while is_busy(&e.entry_id) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let row = engine
            .read(|c| store::thread_row(c, &e.thread_id))
            .ok()
            .flatten();
        let dir = e
            .worktree_path
            .clone()
            .or_else(|| row.as_ref().and_then(|r| r.worktree_path.clone()))
            .map(PathBuf::from)
            .filter(|p| p.is_dir() && worktree::is_omniget_worktree(p));
        let mut branch = e.branch.clone().filter(|b| !b.is_empty());
        let mut kept_worktree = false;
        if let Some(p) = &dir {
            if branch.is_none() {
                branch = job_worktree::branch_at(p).await;
            }
            let b = branch.clone().unwrap_or_default();
            match job_worktree::remove(&root, p, &b, Some(&a.base_commit)).await {
                Ok(r) => {
                    out.worktrees_removed.push(e.label.clone());
                    if let Some(s) = r.saved {
                        out.saved
                            .push(format!("{}: {}", e.label, &s[..s.len().min(10)]));
                    }
                }
                Err(err) => {
                    kept_worktree = true;
                    out.errors.push(format!("{}: {err}", e.label));
                }
            }
        }
        if let Some(b) = branch.as_deref().filter(|_| !kept_worktree) {
            if let Some(tip) = rev_parse(&ctx, &format!("refs/heads/{b}")).await {
                let _ = ctx
                    .git()
                    .args(["update-ref", &arena_ref(arena_id, &e.entry_id), &tip])
                    .run()
                    .await;
                let owner = repo::config_get(&ctx, &format!("branch.{b}.omniget-thread")).await;
                let ours = owner.as_deref() == Some(e.thread_id.as_str())
                    || worktree::is_temp_branch(b)
                    || (b.starts_with("omniget/") && e.branch.as_deref() == Some(b));
                let used = checkout_of(&root, b).await.is_some();
                if delete_branches && ours && !used {
                    match integrate::delete_branch(&root, b).await {
                        Ok(_) => out.branches_deleted.push(b.to_string()),
                        Err(err) => out.errors.push(format!("{}: {err}", e.label)),
                    }
                } else {
                    if delete_branches && !ours {
                        out.errors.push(format!(
                            "{}: branch {b} is not an OmniGet branch, kept",
                            e.label
                        ));
                    }
                    out.branches_kept.push(b.to_string());
                }
            }
        }
        if kept_worktree {
            continue;
        }
        if let Some(r) = &row {
            if r.archived_at.is_none() {
                match dispatch(
                    engine,
                    json!({ "type": "thread.archive", "threadId": e.thread_id }),
                )
                .await
                {
                    Ok(()) => out.archived.push(e.thread_id.clone()),
                    Err(err) => out.errors.push(format!("{}: {err}", e.label)),
                }
            }
        }
        let _ = set_entry_fields(&e.entry_id, &[("cleaned_at", json!(now_iso()))]);
        live()
            .lock()
            .unwrap_or_else(|x| x.into_inner())
            .threads
            .remove(&e.thread_id);
    }
    if all && a.status != "applied" {
        let _ = with_db(|c| {
            c.execute(
                "UPDATE arenas SET status = 'discarded', updated_at = ?2 WHERE arena_id = ?1",
                params![arena_id, now_iso()],
            )
        });
    }
    out.arena = load_arena(arena_id)?;
    Ok(out)
}

#[tauri::command]
pub async fn arena_estimate(entries: Vec<EntrySpec>) -> Result<ArenaEstimate, String> {
    let board = with_db(|c| scoreboard(c, None))?;
    let mut prices = Vec::with_capacity(entries.len());
    for e in &entries {
        let p = match e.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => {
                let m = m.split_once(':').map(|(_, x)| x).unwrap_or(m).to_string();
                tokio::time::timeout(
                    Duration::from_secs(8),
                    omniget_core::core::tools::pricing::price_for(&m),
                )
                .await
                .ok()
                .flatten()
            }
            None => None,
        };
        prices.push(p);
    }
    Ok(compare::estimate_from(&entries, &board, &prices))
}

#[tauri::command]
pub async fn arena_compare(app: AppHandle, arena_id: String) -> Result<ArenaCompare, String> {
    let _ = threads_host::get(&app)?;
    compare_core(&arena_id).await
}

pub async fn compare_core(arena_id: &str) -> Result<ArenaCompare, String> {
    let a = load_arena(arena_id)?;
    let root = PathBuf::from(&a.repo_root);
    let ctx = gctx(&root);
    let mut per = Vec::new();
    let mut pending = Vec::new();
    for e in &a.entries {
        let commit = match rev_parse(&ctx, &arena_ref(arena_id, &e.entry_id)).await {
            Some(c) => Some(c),
            None => e.commit_sha.clone(),
        };
        match commit {
            Some(c) if !matches!(e.status.as_str(), "preparing" | "running") => {
                per.push((
                    e.entry_id.clone(),
                    compare::touches(&root, &a.base_commit, &c).await?,
                ));
            }
            _ => pending.push(e.entry_id.clone()),
        }
    }
    let files = compare::divergence(&per);
    Ok(ArenaCompare {
        arena_id: arena_id.to_string(),
        entries: per.iter().map(|(id, _)| id.clone()).collect(),
        pending,
        divergent: files.iter().filter(|f| f.divergent).count(),
        agreed: files.iter().filter(|f| !f.divergent).count(),
        files,
    })
}

#[tauri::command]
pub async fn arena_delete(app: AppHandle, arena_id: String) -> Result<(), String> {
    let _ = threads_host::get(&app)?;
    let a = load_arena(&arena_id)?;
    let ctx = gctx(Path::new(&a.repo_root));
    if let Ok(list) = git_out(
        &ctx,
        &[
            "for-each-ref",
            "--format=%(refname)",
            &format!("{REF_ROOT}/{arena_id}/"),
        ],
    )
    .await
    {
        for r in list.lines().filter(|l| !l.trim().is_empty()) {
            let _ = ctx
                .git()
                .args(["update-ref", "-d", r])
                .unchecked()
                .run()
                .await;
        }
    }
    {
        let mut l = live().lock().unwrap_or_else(|e| e.into_inner());
        for e in &a.entries {
            l.threads.remove(&e.thread_id);
        }
    }
    with_db(|c| {
        c.execute("DELETE FROM entries WHERE arena_id = ?1", [&arena_id])?;
        c.execute("DELETE FROM arenas WHERE arena_id = ?1", [&arena_id])?;
        Ok(())
    })?;
    let _ = app.emit(ARENA_EVENT, json!({ "arenaId": arena_id, "deleted": true }));
    Ok(())
}

#[tauri::command]
pub async fn arena_scoreboard(project_id: Option<String>) -> Result<Vec<ScoreRow>, String> {
    with_db(|c| scoreboard(c, project_id.as_deref()))
}

pub fn scoreboard(c: &Connection, project_id: Option<&str>) -> rusqlite::Result<Vec<ScoreRow>> {
    let mut st = c.prepare(
        "SELECT e.driver, e.model, COUNT(*),
                SUM(e.winner),
                SUM(e.votes),
                SUM(CASE WHEN e.test_status = 'passed' THEN 1 ELSE 0 END),
                SUM(CASE WHEN e.test_status IN ('passed','failed','error') THEN 1 ELSE 0 END),
                SUM(CASE WHEN e.status IN ('failed','interrupted') THEN 1 ELSE 0 END),
                AVG(e.duration_ms),
                AVG(e.cost_usd),
                AVG(e.input_tokens + e.output_tokens),
                AVG(e.additions + e.deletions),
                MAX(a.created_at)
         FROM entries e JOIN arenas a ON a.arena_id = e.arena_id
         WHERE e.status IN ('done','failed','interrupted')
           AND (?1 IS NULL OR a.project_id = ?1)
         GROUP BY e.driver, COALESCE(e.model, '')
         ORDER BY SUM(e.winner) DESC, SUM(e.votes) DESC, COUNT(*) DESC",
    )?;
    let rows = st.query_map([project_id], |r| {
        Ok(ScoreRow {
            driver: r.get(0)?,
            model: r.get(1)?,
            runs: r.get(2)?,
            wins: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            votes: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            tests_passed: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
            tests_run: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
            failures: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
            avg_duration_ms: r.get(8)?,
            avg_cost_usd: r.get(9)?,
            avg_tokens: r.get(10)?,
            avg_lines: r.get(11)?,
            last_at: r.get(12)?,
        })
    })?;
    rows.collect()
}
