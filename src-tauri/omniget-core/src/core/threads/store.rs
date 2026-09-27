//! SQLite side: schema, append, SQL projectors, receipts, cursors and the
//! read queries (snapshot, replay, turn pages). Every function takes a
//! `&Connection`; the engine decides which connection writes.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::decider::{ForkSource, ForkTurn};
use super::model::{
    ActorKind, AggregateKind, DomainEvent, OpenRequest, ProjectState, ReadModel, StoredEvent,
    ThreadState,
};
use crate::core::llm::drivers::{AccessMode, InteractionMode, SessionState};

pub const ERR_THREADS_DB: &str = "ERR_THREADS_DB";

/// Replay budget of `events_after` (T3: 1000 events or 8 MiB).
pub const REPLAY_MAX_EVENTS: i64 = 1000;
pub const REPLAY_MAX_BYTES: i64 = 8 * 1024 * 1024;

/// Every SQL projector, in the order they run inside one transaction. The
/// row-deleting ones run before `turns` (they find their rows through it).
pub const PROJECTORS: &[&str] = &[
    "projects",
    "messages",
    "activities",
    "approvals",
    "user_inputs",
    "plans",
    "turn_usage",
    "provider_sessions",
    "checkpoints",
    "turns",
    "threads",
    "thread_counters",
];

pub fn db_err(e: impl std::fmt::Display) -> String {
    format!("{ERR_THREADS_DB}: {e}")
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    aggregate_kind TEXT NOT NULL,
    stream_id TEXT NOT NULL,
    stream_version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    command_id TEXT,
    causation_id TEXT,
    correlation_id TEXT,
    actor_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_stream_version ON events(aggregate_kind, stream_id, stream_version);
CREATE INDEX IF NOT EXISTS idx_events_stream_sequence ON events(aggregate_kind, stream_id, sequence);
CREATE INDEX IF NOT EXISTS idx_events_command ON events(command_id);
CREATE TABLE IF NOT EXISTS command_receipts (
    command_id TEXT PRIMARY KEY,
    aggregate_kind TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    accepted_at TEXT NOT NULL,
    result_sequence INTEGER NOT NULL,
    status TEXT NOT NULL,
    error TEXT
);
CREATE INDEX IF NOT EXISTS idx_receipts_aggregate ON command_receipts(aggregate_kind, aggregate_id);
CREATE TABLE IF NOT EXISTS projector_cursors (
    projector TEXT PRIMARY KEY,
    last_applied_sequence INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    workspace_root TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_projects_root ON projects(workspace_root, deleted_at);
CREATE TABLE IF NOT EXISTS threads (
    thread_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    title TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    driver TEXT NOT NULL,
    model TEXT,
    agent_id TEXT,
    runtime_mode TEXT NOT NULL DEFAULT 'approval-required',
    interaction_mode TEXT NOT NULL DEFAULT 'default',
    branch TEXT,
    worktree_path TEXT,
    forked_from_json TEXT,
    session_status TEXT,
    last_error TEXT,
    active_turn_id TEXT,
    latest_turn_id TEXT,
    turn_count INTEGER NOT NULL DEFAULT 0,
    pending_approval_count INTEGER NOT NULL DEFAULT 0,
    pending_user_input_count INTEGER NOT NULL DEFAULT 0,
    pinned_at TEXT,
    snoozed_until TEXT,
    archived_at TEXT,
    deleted_at TEXT,
    last_visited_at TEXT,
    latest_user_message_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    base_branch TEXT,
    worktree_state TEXT,
    worktree_setup_json TEXT,
    pr_json TEXT,
    terminals_json TEXT NOT NULL DEFAULT '[]',
    external_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_threads_project ON threads(project_id, deleted_at, archived_at);
CREATE TABLE IF NOT EXISTS turns (
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    message_id TEXT,
    state TEXT NOT NULL,
    model TEXT,
    requested_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    error_message TEXT,
    UNIQUE(thread_id, ordinal)
);
CREATE TABLE IF NOT EXISTS messages (
    message_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    turn_id TEXT,
    role TEXT NOT NULL,
    text TEXT NOT NULL,
    attachments_json TEXT NOT NULL DEFAULT '[]',
    is_streaming INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id, turn_id, created_at);
CREATE TABLE IF NOT EXISTS activities (
    activity_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    turn_id TEXT,
    kind TEXT NOT NULL,
    tone TEXT NOT NULL,
    summary TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    sequence INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_activities_thread ON activities(thread_id, turn_id, sequence);
CREATE TABLE IF NOT EXISTS approvals (
    thread_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    turn_id TEXT,
    request_type TEXT NOT NULL,
    detail_json TEXT,
    options_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    decision TEXT,
    resolution TEXT,
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    PRIMARY KEY(thread_id, request_id)
);
CREATE INDEX IF NOT EXISTS idx_approvals_status ON approvals(status, thread_id);
CREATE TABLE IF NOT EXISTS user_inputs (
    thread_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    turn_id TEXT,
    questions_json TEXT NOT NULL,
    response_mode TEXT,
    status TEXT NOT NULL,
    answers_json TEXT,
    resolution TEXT,
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    PRIMARY KEY(thread_id, request_id)
);
CREATE TABLE IF NOT EXISTS plans (
    plan_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    turn_id TEXT,
    explanation TEXT,
    steps_json TEXT,
    markdown TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS checkpoints (
    thread_id TEXT NOT NULL,
    turn_count INTEGER NOT NULL,
    turn_id TEXT,
    ref TEXT,
    status TEXT NOT NULL,
    files_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    commit_sha TEXT,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(thread_id, turn_count)
);
CREATE TABLE IF NOT EXISTS turn_usage (
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cached_input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    duration_ms INTEGER,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_usage_thread ON turn_usage(thread_id);
CREATE TABLE IF NOT EXISTS provider_sessions (
    thread_id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL,
    driver TEXT NOT NULL,
    status TEXT,
    resume_cursor_json TEXT,
    provider_thread_id TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL
);
";

/// Open (and migrate) the database. WAL, `busy_timeout` 5 s.
pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(db_err)?;
    }
    let conn = Connection::open(path).map_err(db_err)?;
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
    .map_err(db_err)?;
    conn.execute_batch(SCHEMA).map_err(db_err)?;
    ensure_columns(&conn)?;
    let now = crate::core::llm::drivers::now_iso();
    for p in PROJECTORS {
        conn.execute(
            "INSERT OR IGNORE INTO projector_cursors(projector, last_applied_sequence, updated_at) VALUES (?1, 0, ?2)",
            params![p, now],
        )
        .map_err(db_err)?;
    }
    Ok(conn)
}

/// Columns added after the first schema (T2/T7/T8): a database made by an
/// older build gets them here, empty.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("threads", "base_branch", "TEXT"),
    ("threads", "worktree_state", "TEXT"),
    ("threads", "worktree_setup_json", "TEXT"),
    ("threads", "pr_json", "TEXT"),
    ("threads", "terminals_json", "TEXT NOT NULL DEFAULT '[]'"),
    ("threads", "external_json", "TEXT"),
    ("checkpoints", "commit_sha", "TEXT"),
    ("checkpoints", "additions", "INTEGER NOT NULL DEFAULT 0"),
    ("checkpoints", "deletions", "INTEGER NOT NULL DEFAULT 0"),
];

fn ensure_columns(conn: &Connection) -> Result<(), String> {
    for (table, column, decl) in ADDED_COLUMNS {
        let has: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                [column],
                |r| r.get::<_, i64>(0),
            )
            .map_err(db_err)?
            > 0;
        if !has {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))
                .map_err(db_err)?;
        }
    }
    Ok(())
}

/// A read-only connection for queries that run next to the writer.
pub fn open_reader(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(db_err)?;
    conn.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA query_only = 1;")
        .map_err(db_err)?;
    Ok(conn)
}

pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map(|_| ())
    .map_err(db_err)
}

// ── Append ──────────────────────────────────────────────────────────────

/// An event the decider planned, before it has a sequence.
#[derive(Debug, Clone)]
pub struct Planned {
    pub event: DomainEvent,
    pub event_id: String,
    pub occurred_at: String,
    pub command_id: Option<String>,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub actor_kind: ActorKind,
    pub metadata: Value,
}

pub fn append(conn: &Connection, planned: &Planned) -> Result<StoredEvent, String> {
    let (kind, stream) = planned.event.aggregate();
    let (event_type, payload) = planned.event.to_parts();
    let payload_json = serde_json::to_string(&payload).map_err(db_err)?;
    let metadata_json = serde_json::to_string(&planned.metadata).map_err(db_err)?;
    let (sequence, version): (i64, i64) = conn
        .query_row(
            "INSERT INTO events(event_id, aggregate_kind, stream_id, stream_version, event_type,
                occurred_at, command_id, causation_id, correlation_id, actor_kind, payload_json, metadata_json)
             VALUES (?1, ?2, ?3,
                COALESCE((SELECT stream_version + 1 FROM events WHERE aggregate_kind = ?2 AND stream_id = ?3
                          ORDER BY stream_version DESC LIMIT 1), 0),
                ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             RETURNING sequence, stream_version",
            params![
                planned.event_id,
                kind.as_str(),
                stream,
                event_type,
                planned.occurred_at,
                planned.command_id,
                planned.causation_id,
                planned.correlation_id,
                planned.actor_kind.as_str(),
                payload_json,
                metadata_json,
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_err)?;
    Ok(StoredEvent {
        sequence,
        event_id: planned.event_id.clone(),
        aggregate_kind: kind,
        stream_id: stream.to_string(),
        stream_version: version,
        occurred_at: planned.occurred_at.clone(),
        command_id: planned.command_id.clone(),
        causation_id: planned.causation_id.clone(),
        correlation_id: planned.correlation_id.clone(),
        actor_kind: planned.actor_kind,
        event: planned.event.clone(),
        metadata: planned.metadata.clone(),
    })
}

fn row_to_event(r: &Row) -> rusqlite::Result<StoredEvent> {
    let event_type: String = r.get("event_type")?;
    let payload: String = r.get("payload_json")?;
    let metadata: String = r.get("metadata_json")?;
    let payload: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    let event = DomainEvent::from_parts(&event_type, payload).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(StoredEvent {
        sequence: r.get("sequence")?,
        event_id: r.get("event_id")?,
        aggregate_kind: AggregateKind::parse(&r.get::<_, String>("aggregate_kind")?),
        stream_id: r.get("stream_id")?,
        stream_version: r.get("stream_version")?,
        occurred_at: r.get("occurred_at")?,
        command_id: r.get("command_id")?,
        causation_id: r.get("causation_id")?,
        correlation_id: r.get("correlation_id")?,
        actor_kind: ActorKind::parse(&r.get::<_, String>("actor_kind")?),
        event,
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
    })
}

pub fn head(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COALESCE(MAX(sequence), 0) FROM events", [], |r| {
        r.get(0)
    })
    .map_err(db_err)
}

/// Events with `sequence > after`, ascending, at most `limit`. Rows written
/// by a newer build (unknown type) are skipped, never fatal.
pub fn read_events(conn: &Connection, after: i64, limit: i64) -> Result<Vec<StoredEvent>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM events WHERE sequence > ?1 ORDER BY sequence ASC LIMIT ?2")
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![after, limit], row_to_event)
        .map_err(db_err)?;
    Ok(rows.filter_map(Result::ok).collect())
}

// ── Receipts and cursors ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub command_id: String,
    pub aggregate_kind: String,
    pub aggregate_id: String,
    pub accepted_at: String,
    pub result_sequence: i64,
    /// `accepted|rejected`.
    pub status: String,
    pub error: Option<String>,
}

pub fn receipt(conn: &Connection, command_id: &str) -> Result<Option<Receipt>, String> {
    conn.query_row(
        "SELECT command_id, aggregate_kind, aggregate_id, accepted_at, result_sequence, status, error
         FROM command_receipts WHERE command_id = ?1",
        [command_id],
        |r| {
            Ok(Receipt {
                command_id: r.get(0)?,
                aggregate_kind: r.get(1)?,
                aggregate_id: r.get(2)?,
                accepted_at: r.get(3)?,
                result_sequence: r.get(4)?,
                status: r.get(5)?,
                error: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

pub fn put_receipt(conn: &Connection, r: &Receipt) -> Result<(), String> {
    conn.execute(
        "INSERT INTO command_receipts(command_id, aggregate_kind, aggregate_id, accepted_at, result_sequence, status, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(command_id) DO UPDATE SET result_sequence = excluded.result_sequence,
            status = excluded.status, error = excluded.error",
        params![
            r.command_id,
            r.aggregate_kind,
            r.aggregate_id,
            r.accepted_at,
            r.result_sequence,
            r.status,
            r.error
        ],
    )
    .map(|_| ())
    .map_err(db_err)
}

pub fn cursors(conn: &Connection) -> Result<HashMap<String, i64>, String> {
    let mut stmt = conn
        .prepare("SELECT projector, last_applied_sequence FROM projector_cursors")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(db_err)?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn set_cursors(conn: &Connection, names: &[&str], sequence: i64) -> Result<(), String> {
    let now = crate::core::llm::drivers::now_iso();
    for name in names {
        conn.execute(
            "INSERT INTO projector_cursors(projector, last_applied_sequence, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(projector) DO UPDATE SET last_applied_sequence = MAX(last_applied_sequence, excluded.last_applied_sequence),
                updated_at = excluded.updated_at",
            params![name, sequence, now],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

/// Boot catch-up: every projector behind the head replays from its own
/// cursor, one transaction per event (resumable, and a new projector
/// backfills itself).
pub fn bootstrap_projectors(conn: &mut Connection) -> Result<usize, String> {
    let cursors = cursors(conn)?;
    let min = PROJECTORS
        .iter()
        .map(|p| cursors.get(*p).copied().unwrap_or(0))
        .min()
        .unwrap_or(0);
    let head = head(conn)?;
    if min >= head {
        return Ok(0);
    }
    let mut replayed = 0usize;
    let mut after = min;
    loop {
        let page = read_events(conn, after, 500)?;
        if page.is_empty() {
            break;
        }
        for ev in &page {
            let behind: Vec<&str> = PROJECTORS
                .iter()
                .copied()
                .filter(|p| cursors.get(*p).copied().unwrap_or(0) < ev.sequence)
                .collect();
            let tx = conn.transaction().map_err(db_err)?;
            for p in &behind {
                project(&tx, p, ev)?;
            }
            set_cursors(&tx, &behind, ev.sequence)?;
            tx.commit().map_err(db_err)?;
            replayed += 1;
            after = ev.sequence;
        }
    }
    Ok(replayed)
}

// ── SQL projectors ──────────────────────────────────────────────────────

fn j<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Apply one event to one projector.
pub fn project(conn: &Connection, projector: &str, stored: &StoredEvent) -> Result<(), String> {
    use DomainEvent as E;
    let seq = stored.sequence;
    let at = &stored.occurred_at;
    let r: rusqlite::Result<usize> = match (projector, &stored.event) {
        // projects
        (
            "projects",
            E::ProjectCreated {
                project_id,
                title,
                workspace_root,
                created_at,
            },
        ) => conn.execute(
            "INSERT OR REPLACE INTO projects(project_id, title, workspace_root, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?4, NULL)",
            params![project_id, title, workspace_root, created_at],
        ),
        (
            "projects",
            E::ProjectMetaUpdated {
                project_id,
                title,
                workspace_root,
                updated_at,
            },
        ) => conn.execute(
            "UPDATE projects SET title = COALESCE(?2, title), workspace_root = COALESCE(?3, workspace_root),
                updated_at = ?4 WHERE project_id = ?1",
            params![project_id, title, workspace_root, updated_at],
        ),
        (
            "projects",
            E::ProjectDeleted {
                project_id,
                deleted_at,
            },
        ) => conn.execute(
            "UPDATE projects SET deleted_at = ?2, updated_at = ?2 WHERE project_id = ?1",
            params![project_id, deleted_at],
        ),

        // messages
        (
            "messages",
            E::MessageSent {
                thread_id,
                message_id,
                role,
                text,
                turn_id,
                streaming,
                attachments,
                created_at,
            },
        ) => {
            if *streaming {
                conn.execute(
                    "INSERT INTO messages(message_id, thread_id, turn_id, role, text, attachments_json, is_streaming, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)
                     ON CONFLICT(message_id) DO UPDATE SET text = messages.text || excluded.text,
                        is_streaming = 1, updated_at = excluded.updated_at",
                    params![message_id, thread_id, turn_id, role, text, j(attachments), created_at],
                )
            } else {
                conn.execute(
                    "INSERT INTO messages(message_id, thread_id, turn_id, role, text, attachments_json, is_streaming, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)
                     ON CONFLICT(message_id) DO UPDATE SET
                        text = CASE WHEN excluded.text = '' THEN messages.text ELSE excluded.text END,
                        is_streaming = 0, updated_at = excluded.updated_at",
                    params![message_id, thread_id, turn_id, role, text, j(attachments), created_at],
                )
            }
        }
        ("messages", E::TurnCompleted { turn_id, .. }) => conn.execute(
            "UPDATE messages SET is_streaming = 0 WHERE turn_id = ?1 AND is_streaming = 1",
            [turn_id],
        ),
        (
            "messages",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM messages WHERE thread_id = ?1 AND turn_id IN
                (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal > ?2)",
            params![thread_id, turn_count],
        ),

        // activities
        (
            "activities",
            E::ActivityAppended {
                thread_id,
                activity,
            },
        ) => conn.execute(
            "INSERT INTO activities(activity_id, thread_id, turn_id, kind, tone, summary, payload_json, sequence, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(activity_id) DO UPDATE SET kind = excluded.kind, tone = excluded.tone,
                summary = CASE WHEN excluded.summary = '' THEN activities.summary ELSE excluded.summary END,
                payload_json = CASE WHEN json_valid(activities.payload_json) AND json_valid(excluded.payload_json)
                    THEN json_patch(activities.payload_json, excluded.payload_json) ELSE excluded.payload_json END,
                sequence = excluded.sequence, updated_at = excluded.updated_at",
            params![
                activity.activity_id,
                thread_id,
                activity.turn_id,
                activity.kind,
                activity.tone,
                activity.summary,
                j(&activity.payload),
                seq,
                activity.created_at
            ],
        ),
        (
            "activities",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM activities WHERE thread_id = ?1 AND turn_id IN
                (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal > ?2)",
            params![thread_id, turn_count],
        ),

        // approvals
        (
            "approvals",
            E::ApprovalRequested {
                thread_id,
                turn_id,
                request_id,
                request_type,
                detail,
                options,
                created_at,
            },
        ) => conn.execute(
            "INSERT OR IGNORE INTO approvals(thread_id, request_id, turn_id, request_type, detail_json, options_json, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                thread_id,
                request_id,
                turn_id,
                enum_str(request_type),
                detail.as_ref().map(j),
                j(options),
                created_at
            ],
        ),
        (
            "approvals",
            E::ApprovalResponseRequested {
                thread_id,
                request_id,
                decision,
            },
        ) => conn.execute(
            "UPDATE approvals SET status = 'answered', decision = ?3
             WHERE thread_id = ?1 AND request_id = ?2 AND status = 'pending'",
            params![thread_id, request_id, enum_str(decision)],
        ),
        (
            "approvals",
            E::ApprovalResolved {
                thread_id,
                request_id,
                decision,
                resolution,
                resolved_at,
            },
        ) => conn.execute(
            "UPDATE approvals SET status = 'resolved', decision = COALESCE(?3, decision),
                resolution = ?4, resolved_at = ?5 WHERE thread_id = ?1 AND request_id = ?2",
            params![
                thread_id,
                request_id,
                decision.as_ref().map(enum_str),
                resolution,
                resolved_at
            ],
        ),
        (
            "approvals",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM approvals WHERE thread_id = ?1 AND turn_id IN
                (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal > ?2)",
            params![thread_id, turn_count],
        ),

        // user inputs
        (
            "user_inputs",
            E::UserInputRequested {
                thread_id,
                turn_id,
                request_id,
                questions,
                response_mode,
                created_at,
            },
        ) => conn.execute(
            "INSERT OR IGNORE INTO user_inputs(thread_id, request_id, turn_id, questions_json, response_mode, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
            params![
                thread_id,
                request_id,
                turn_id,
                j(questions),
                response_mode,
                created_at
            ],
        ),
        (
            "user_inputs",
            E::UserInputResponseRequested {
                thread_id,
                request_id,
                answers,
            },
        ) => conn.execute(
            "UPDATE user_inputs SET status = 'answered', answers_json = ?3
             WHERE thread_id = ?1 AND request_id = ?2 AND status = 'pending'",
            params![thread_id, request_id, j(answers)],
        ),
        (
            "user_inputs",
            E::UserInputResolved {
                thread_id,
                request_id,
                answers,
                resolution,
                resolved_at,
            },
        ) => conn.execute(
            "UPDATE user_inputs SET status = 'resolved', answers_json = COALESCE(?3, answers_json),
                resolution = ?4, resolved_at = ?5 WHERE thread_id = ?1 AND request_id = ?2",
            params![
                thread_id,
                request_id,
                answers.as_ref().map(j),
                resolution,
                resolved_at
            ],
        ),
        (
            "user_inputs",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM user_inputs WHERE thread_id = ?1 AND turn_id IN
                (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal > ?2)",
            params![thread_id, turn_count],
        ),

        // plans
        (
            "plans",
            E::PlanUpdated {
                thread_id,
                plan_id,
                turn_id,
                explanation,
                steps,
                markdown,
                updated_at,
            },
        ) => conn.execute(
            "INSERT INTO plans(plan_id, thread_id, turn_id, explanation, steps_json, markdown, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(plan_id) DO UPDATE SET explanation = COALESCE(excluded.explanation, plans.explanation),
                steps_json = COALESCE(excluded.steps_json, plans.steps_json),
                markdown = COALESCE(excluded.markdown, plans.markdown), updated_at = excluded.updated_at",
            params![
                plan_id,
                thread_id,
                turn_id,
                explanation,
                steps.as_ref().map(j),
                markdown,
                updated_at
            ],
        ),
        (
            "plans",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM plans WHERE thread_id = ?1 AND turn_id IN
                (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal > ?2)",
            params![thread_id, turn_count],
        ),

        // turn usage
        (
            "turn_usage",
            E::TurnUsage {
                thread_id,
                turn_id,
                usage,
            },
        ) => conn.execute(
            "INSERT INTO turn_usage(turn_id, thread_id, model, input_tokens, cached_input_tokens, cache_write_tokens,
                output_tokens, reasoning_output_tokens, cost_usd, duration_ms, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(turn_id) DO UPDATE SET model = COALESCE(excluded.model, turn_usage.model),
                input_tokens = excluded.input_tokens, cached_input_tokens = excluded.cached_input_tokens,
                cache_write_tokens = excluded.cache_write_tokens, output_tokens = excluded.output_tokens,
                reasoning_output_tokens = excluded.reasoning_output_tokens,
                cost_usd = COALESCE(excluded.cost_usd, turn_usage.cost_usd),
                duration_ms = COALESCE(excluded.duration_ms, turn_usage.duration_ms),
                updated_at = excluded.updated_at",
            params![
                turn_id,
                thread_id,
                usage.model,
                usage.input_tokens as i64,
                usage.cached_input_tokens as i64,
                usage.cache_write_tokens as i64,
                usage.output_tokens as i64,
                usage.reasoning_output_tokens as i64,
                usage.cost_usd,
                usage.duration_ms.map(|d| d as i64),
                at
            ],
        ),
        // Every finished turn gets a row, whatever the driver reported:
        // model from the turn, wall time from started → completed. A later
        // `turn-usage` of the same turn fills the tokens.
        (
            "turn_usage",
            E::TurnCompleted {
                thread_id,
                turn_id,
                completed_at,
                ..
            },
        ) => conn
            .execute(
                "INSERT OR IGNORE INTO turn_usage(turn_id, thread_id, model, updated_at)
             VALUES (?1, ?2, (SELECT model FROM turns WHERE turn_id = ?1), ?3)",
                params![turn_id, thread_id, completed_at],
            )
            .and_then(|_| {
                conn.execute(
                    "UPDATE turn_usage SET
                    model = COALESCE(model, (SELECT model FROM turns WHERE turn_id = ?1)),
                    duration_ms = COALESCE(duration_ms, (SELECT CAST(ROUND(
                        (julianday(?2) - julianday(COALESCE(started_at, requested_at))) * 86400000.0) AS INTEGER)
                        FROM turns WHERE turn_id = ?1))
                 WHERE turn_id = ?1",
                    params![turn_id, completed_at],
                )
            }),
        (
            "turn_usage",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM turn_usage WHERE thread_id = ?1 AND turn_id IN
                (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal > ?2)",
            params![thread_id, turn_count],
        ),

        // provider sessions
        (
            "provider_sessions",
            E::ThreadCreated {
                thread_id,
                instance_id,
                driver,
                created_at,
                ..
            },
        ) => conn.execute(
            "INSERT OR REPLACE INTO provider_sessions(thread_id, instance_id, driver, status, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4)",
            params![thread_id, instance_id, driver, created_at],
        ),
        (
            "provider_sessions",
            E::InstanceSet {
                thread_id,
                instance_id,
                driver,
                ..
            },
        ) => conn.execute(
            "UPDATE provider_sessions SET instance_id = ?2, driver = ?3, status = NULL,
                resume_cursor_json = NULL, provider_thread_id = NULL, updated_at = ?4 WHERE thread_id = ?1",
            params![thread_id, instance_id, driver, at],
        ),
        (
            "provider_sessions",
            E::SessionSet {
                thread_id,
                status,
                last_error,
                resume_cursor,
                provider_thread_id,
                updated_at,
            },
        ) => conn.execute(
            "UPDATE provider_sessions SET status = COALESCE(?2, status),
                last_error = CASE WHEN ?2 IS NULL THEN last_error ELSE ?3 END,
                resume_cursor_json = COALESCE(?4, resume_cursor_json),
                provider_thread_id = COALESCE(?5, provider_thread_id), updated_at = ?6
             WHERE thread_id = ?1",
            params![
                thread_id,
                status.as_ref().map(enum_str),
                last_error,
                resume_cursor.as_ref().map(j),
                provider_thread_id,
                updated_at
            ],
        ),

        // checkpoints
        (
            "checkpoints",
            E::CheckpointCaptured {
                thread_id,
                turn_count,
                turn_id,
                ref_name,
                commit,
                status,
                files,
                additions,
                deletions,
                captured_at,
            },
        ) => conn.execute(
            "INSERT OR REPLACE INTO checkpoints(thread_id, turn_count, turn_id, ref, status, files_json,
                created_at, commit_sha, additions, deletions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                thread_id,
                turn_count,
                turn_id,
                ref_name,
                status,
                j(files),
                captured_at,
                commit,
                additions,
                deletions
            ],
        ),
        (
            "checkpoints",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM checkpoints WHERE thread_id = ?1 AND turn_count > ?2",
            params![thread_id, turn_count],
        ),
        ("checkpoints", E::ThreadDeleted { thread_id, .. }) => {
            conn.execute("DELETE FROM checkpoints WHERE thread_id = ?1", [thread_id])
        }

        // turns
        (
            "turns",
            E::TurnStartRequested {
                thread_id,
                turn_id,
                message_id,
                ordinal,
                model,
                requested_at,
                ..
            },
        ) => conn.execute(
            "INSERT OR REPLACE INTO turns(turn_id, thread_id, ordinal, message_id, state, model, requested_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
            params![turn_id, thread_id, ordinal, message_id, model, requested_at],
        ),
        (
            "turns",
            E::TurnImported {
                thread_id,
                turn_id,
                ordinal,
                message_id,
                state,
                requested_at,
                completed_at,
            },
        ) => conn.execute(
            "INSERT OR REPLACE INTO turns(turn_id, thread_id, ordinal, message_id, state, requested_at, started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
            params![
                turn_id,
                thread_id,
                ordinal,
                message_id,
                state,
                requested_at,
                completed_at
            ],
        ),
        (
            "turns",
            E::TurnStarted {
                turn_id,
                model,
                started_at,
                ..
            },
        ) => conn.execute(
            "UPDATE turns SET state = 'running', started_at = ?2, model = COALESCE(?3, model)
             WHERE turn_id = ?1 AND state IN ('pending', 'running')",
            params![turn_id, started_at, model],
        ),
        (
            "turns",
            E::TurnCompleted {
                turn_id,
                state,
                error_message,
                completed_at,
                ..
            },
        ) => conn.execute(
            "UPDATE turns SET state = ?2, error_message = COALESCE(?3, error_message), completed_at = ?4
             WHERE turn_id = ?1 AND state IN ('pending', 'running')",
            params![turn_id, state, error_message, completed_at],
        ),
        (
            "turns",
            E::Reverted {
                thread_id,
                turn_count,
            },
        ) => conn.execute(
            "DELETE FROM turns WHERE thread_id = ?1 AND ordinal > ?2",
            params![thread_id, turn_count],
        ),

        // threads (the shell row)
        ("threads", ev) => project_thread_row(conn, ev, at),

        // counters on the shell row
        ("thread_counters", ev) => project_counters(conn, ev),

        _ => Ok(0),
    };
    r.map(|_| ()).map_err(|e| {
        format!(
            "{ERR_THREADS_DB}: projector {projector} on {} #{}: {e}",
            stored.event.type_name(),
            seq
        )
    })
}

fn project_thread_row(conn: &Connection, ev: &DomainEvent, at: &str) -> rusqlite::Result<usize> {
    use DomainEvent as E;
    match ev {
        E::ThreadCreated {
            thread_id,
            project_id,
            title,
            instance_id,
            driver,
            model,
            agent_id,
            runtime_mode,
            interaction_mode,
            branch,
            worktree_path,
            forked_from,
            worktree,
            created_at,
        } => conn.execute(
            "INSERT OR REPLACE INTO threads(thread_id, project_id, title, instance_id, driver, model, agent_id,
                runtime_mode, interaction_mode, branch, worktree_path, forked_from_json, turn_count, created_at, updated_at,
                base_branch, worktree_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?13, ?14, ?15)",
            params![
                thread_id,
                project_id,
                title,
                instance_id,
                driver,
                model,
                agent_id,
                runtime_mode.as_str(),
                enum_str(interaction_mode),
                branch,
                worktree_path,
                forked_from.as_ref().map(j),
                created_at,
                worktree.as_ref().and_then(|w| w.base_branch.clone()),
                worktree.as_ref().map(|_| "pending")
            ],
        ),
        E::ThreadDeleted {
            thread_id,
            deleted_at,
        } => conn.execute(
            "UPDATE threads SET deleted_at = ?2, active_turn_id = NULL, updated_at = ?2 WHERE thread_id = ?1",
            params![thread_id, deleted_at],
        ),
        E::ThreadArchived {
            thread_id,
            archived_at,
        } => conn.execute(
            "UPDATE threads SET archived_at = ?2 WHERE thread_id = ?1",
            params![thread_id, archived_at],
        ),
        E::ThreadUnarchived { thread_id } => conn.execute(
            "UPDATE threads SET archived_at = NULL WHERE thread_id = ?1",
            [thread_id],
        ),
        E::ThreadPinned {
            thread_id,
            pinned_at,
        } => conn.execute(
            "UPDATE threads SET pinned_at = ?2 WHERE thread_id = ?1",
            params![thread_id, pinned_at],
        ),
        E::ThreadUnpinned { thread_id } => conn.execute(
            "UPDATE threads SET pinned_at = NULL WHERE thread_id = ?1",
            [thread_id],
        ),
        E::ThreadSnoozed {
            thread_id,
            snoozed_until,
        } => conn.execute(
            "UPDATE threads SET snoozed_until = ?2 WHERE thread_id = ?1",
            params![thread_id, snoozed_until],
        ),
        E::ThreadUnsnoozed { thread_id } => conn.execute(
            "UPDATE threads SET snoozed_until = NULL WHERE thread_id = ?1",
            [thread_id],
        ),
        E::ThreadMetaUpdated {
            thread_id,
            title,
            updated_at,
        } => conn.execute(
            "UPDATE threads SET title = COALESCE(?2, title), updated_at = ?3 WHERE thread_id = ?1",
            params![thread_id, title, updated_at],
        ),
        E::ThreadVisited {
            thread_id,
            visited_at,
        } => conn.execute(
            "UPDATE threads SET last_visited_at = ?2 WHERE thread_id = ?1",
            params![thread_id, visited_at],
        ),
        E::RuntimeModeSet {
            thread_id,
            runtime_mode,
        } => conn.execute(
            "UPDATE threads SET runtime_mode = ?2 WHERE thread_id = ?1",
            params![thread_id, runtime_mode.as_str()],
        ),
        E::InteractionModeSet {
            thread_id,
            interaction_mode,
        } => conn.execute(
            "UPDATE threads SET interaction_mode = ?2 WHERE thread_id = ?1",
            params![thread_id, enum_str(interaction_mode)],
        ),
        E::InstanceSet {
            thread_id,
            instance_id,
            driver,
            model,
            agent_id,
        } => conn.execute(
            "UPDATE threads SET instance_id = ?2, driver = ?3, model = ?4, agent_id = ?5,
                session_status = NULL, last_error = NULL, updated_at = ?6 WHERE thread_id = ?1",
            params![thread_id, instance_id, driver, model, agent_id, at],
        ),
        E::TurnStartRequested {
            thread_id,
            turn_id,
            ordinal,
            model,
            requested_at,
            ..
        } => conn.execute(
            "UPDATE threads SET turn_count = MAX(turn_count, ?3), active_turn_id = ?2, latest_turn_id = ?2,
                model = COALESCE(?4, model), updated_at = ?5 WHERE thread_id = ?1",
            params![thread_id, turn_id, ordinal, model, requested_at],
        ),
        E::TurnImported {
            thread_id,
            turn_id,
            ordinal,
            ..
        } => conn.execute(
            "UPDATE threads SET turn_count = MAX(turn_count, ?3),
                latest_turn_id = CASE WHEN ?3 >= turn_count THEN ?2 ELSE latest_turn_id END
             WHERE thread_id = ?1",
            params![thread_id, turn_id, ordinal],
        ),
        E::TurnStarted { thread_id, .. } | E::MessageSent { thread_id, .. } => conn.execute(
            "UPDATE threads SET updated_at = ?2 WHERE thread_id = ?1",
            params![thread_id, at],
        ),
        E::TurnCompleted {
            thread_id,
            turn_id,
            completed_at,
            ..
        } => conn.execute(
            "UPDATE threads SET active_turn_id = CASE WHEN active_turn_id = ?2 THEN NULL ELSE active_turn_id END,
                updated_at = ?3 WHERE thread_id = ?1",
            params![thread_id, turn_id, completed_at],
        ),
        E::SessionSet {
            thread_id,
            status,
            last_error,
            updated_at,
            ..
        } => conn.execute(
            "UPDATE threads SET session_status = COALESCE(?2, session_status),
                last_error = CASE WHEN ?2 IS NULL THEN last_error ELSE ?3 END, updated_at = ?4
             WHERE thread_id = ?1",
            params![
                thread_id,
                status.as_ref().map(enum_str),
                last_error,
                updated_at
            ],
        ),
        E::WorktreeUpdated {
            thread_id,
            state,
            path,
            branch,
            base,
            setup,
            error,
            updated_at,
        } => conn.execute(
            "UPDATE threads SET worktree_state = ?2, worktree_path = COALESCE(?3, worktree_path),
                branch = COALESCE(?4, branch), base_branch = COALESCE(?5, base_branch),
                worktree_setup_json = COALESCE(?6, worktree_setup_json),
                last_error = CASE WHEN ?7 IS NULL THEN last_error ELSE ?7 END, updated_at = ?8
             WHERE thread_id = ?1",
            params![
                thread_id,
                state,
                path,
                branch,
                base,
                setup.as_ref().map(j),
                error,
                updated_at
            ],
        ),
        E::PrUpdated {
            thread_id,
            pr,
            updated_at,
        } => conn.execute(
            "UPDATE threads SET pr_json = ?2, updated_at = ?3 WHERE thread_id = ?1",
            params![thread_id, pr.as_ref().map(j), updated_at],
        ),
        E::TerminalAttached {
            thread_id,
            terminal_id,
            ..
        } => {
            let mut ids = thread_terminals(conn, thread_id)?;
            if ids.iter().any(|t| t == terminal_id) {
                Ok(0)
            } else {
                ids.push(terminal_id.clone());
                conn.execute(
                    "UPDATE threads SET terminals_json = ?2 WHERE thread_id = ?1",
                    params![thread_id, j(&ids)],
                )
            }
        }
        E::TerminalClosed {
            thread_id,
            terminal_id,
        } => {
            let mut ids = thread_terminals(conn, thread_id)?;
            ids.retain(|t| t != terminal_id);
            conn.execute(
                "UPDATE threads SET terminals_json = ?2 WHERE thread_id = ?1",
                params![thread_id, j(&ids)],
            )
        }
        E::ExternalLinked {
            thread_id,
            tool,
            session_id,
            source,
            resume_command,
            ..
        } => conn.execute(
            "UPDATE threads SET external_json = ?2 WHERE thread_id = ?1",
            params![
                thread_id,
                j(&serde_json::json!({
                    "tool": tool,
                    "sessionId": session_id,
                    "source": source,
                    "resumeCommand": resume_command,
                }))
            ],
        ),
        E::Reverted {
            thread_id,
            turn_count,
        } => conn.execute(
            "UPDATE threads SET turn_count = ?2, active_turn_id = NULL,
                latest_turn_id = (SELECT turn_id FROM turns WHERE thread_id = ?1 AND ordinal = ?2),
                updated_at = ?3 WHERE thread_id = ?1",
            params![thread_id, turn_count, at],
        ),
        _ => Ok(0),
    }
}

fn thread_terminals(conn: &Connection, thread_id: &str) -> rusqlite::Result<Vec<String>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT terminals_json FROM threads WHERE thread_id = ?1",
            [thread_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

fn project_counters(conn: &Connection, ev: &DomainEvent) -> rusqlite::Result<usize> {
    use DomainEvent as E;
    match ev {
        E::MessageSent {
            thread_id,
            role,
            created_at,
            ..
        } if role == "user" => conn.execute(
            "UPDATE threads SET latest_user_message_at = CASE
                WHEN latest_user_message_at IS NULL OR latest_user_message_at < ?2 THEN ?2
                ELSE latest_user_message_at END WHERE thread_id = ?1",
            params![thread_id, created_at],
        ),
        E::ApprovalRequested { thread_id, .. }
        | E::ApprovalResponseRequested { thread_id, .. }
        | E::ApprovalResolved { thread_id, .. }
        | E::UserInputRequested { thread_id, .. }
        | E::UserInputResponseRequested { thread_id, .. }
        | E::UserInputResolved { thread_id, .. }
        | E::Reverted { thread_id, .. } => conn.execute(
            "UPDATE threads SET
                pending_approval_count = (SELECT COUNT(*) FROM approvals WHERE thread_id = ?1 AND status = 'pending'),
                pending_user_input_count = (SELECT COUNT(*) FROM user_inputs WHERE thread_id = ?1 AND status = 'pending')
             WHERE thread_id = ?1",
            [thread_id],
        ),
        _ => Ok(0),
    }
}

// ── Read model bootstrap ────────────────────────────────────────────────

pub fn load_read_model(conn: &Connection) -> Result<ReadModel, String> {
    let mut model = ReadModel {
        sequence: head(conn)?,
        ..Default::default()
    };
    {
        let mut stmt = conn
            .prepare("SELECT project_id, title, workspace_root, deleted_at FROM projects")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ProjectState {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    workspace_root: r.get(2)?,
                    deleted: r.get::<_, Option<String>>(3)?.is_some(),
                })
            })
            .map_err(db_err)?;
        for p in rows.flatten() {
            model.projects.insert(p.id.clone(), p);
        }
    }
    {
        let mut stmt = conn
            .prepare(
                "SELECT thread_id, project_id, title, instance_id, driver, model, agent_id, runtime_mode,
                    interaction_mode, archived_at, deleted_at, pinned_at, snoozed_until, turn_count,
                    active_turn_id, session_status, latest_user_message_at, worktree_state,
                    external_json FROM threads",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ThreadState {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    title: r.get(2)?,
                    instance_id: r.get(3)?,
                    driver: r.get(4)?,
                    model: r.get(5)?,
                    agent_id: r.get(6)?,
                    runtime_mode: AccessMode::parse(&r.get::<_, String>(7)?),
                    interaction_mode: if r.get::<_, String>(8)? == "plan" {
                        InteractionMode::Plan
                    } else {
                        InteractionMode::Default
                    },
                    archived: r.get::<_, Option<String>>(9)?.is_some(),
                    deleted: r.get::<_, Option<String>>(10)?.is_some(),
                    pinned: r.get::<_, Option<String>>(11)?.is_some(),
                    snoozed_until: r.get(12)?,
                    turn_count: r.get::<_, i64>(13)? as u32,
                    active_turn: r.get(14)?,
                    session_status: r.get::<_, Option<String>>(15)?.and_then(|s| {
                        serde_json::from_value::<SessionState>(Value::String(s)).ok()
                    }),
                    open_approvals: HashMap::new(),
                    open_inputs: HashMap::new(),
                    has_user_message: r.get::<_, Option<String>>(16)?.is_some(),
                    worktree_state: r.get(17)?,
                    external: r.get::<_, Option<String>>(18)?.is_some(),
                })
            })
            .map_err(db_err)?;
        for t in rows.flatten() {
            model.threads.insert(t.id.clone(), t);
        }
    }
    for (table, is_approval) in [("approvals", true), ("user_inputs", false)] {
        let sql = if is_approval {
            "SELECT thread_id, request_id, turn_id, status, request_type FROM approvals WHERE status IN ('pending','answered')"
        } else {
            "SELECT thread_id, request_id, turn_id, status, NULL FROM user_inputs WHERE status IN ('pending','answered')"
        };
        let mut stmt = conn.prepare(sql).map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(db_err)?;
        for (thread_id, request_id, turn_id, status, rtype) in rows.flatten() {
            let Some(t) = model.threads.get_mut(&thread_id) else {
                continue;
            };
            let open = OpenRequest {
                turn_id,
                request_type: rtype.and_then(|s| serde_json::from_value(Value::String(s)).ok()),
                answered: status == "answered",
            };
            if table == "approvals" {
                t.open_approvals.insert(request_id, open);
            } else {
                t.open_inputs.insert(request_id, open);
            }
        }
    }
    Ok(model)
}

// ── Read side ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    pub project_id: String,
    pub title: String,
    pub workspace_root: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    pub thread_id: String,
    pub project_id: String,
    pub title: String,
    pub instance_id: String,
    pub driver: String,
    pub model: Option<String>,
    pub agent_id: Option<String>,
    pub runtime_mode: String,
    pub interaction_mode: String,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub forked_from: Option<Value>,
    pub session_status: Option<String>,
    pub last_error: Option<String>,
    pub active_turn_id: Option<String>,
    pub latest_turn_id: Option<String>,
    pub turn_count: u32,
    pub pending_approval_count: u32,
    pub pending_user_input_count: u32,
    pub pinned_at: Option<String>,
    pub snoozed_until: Option<String>,
    pub archived_at: Option<String>,
    pub last_visited_at: Option<String>,
    pub latest_user_message_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub base_branch: Option<String>,
    /// `pending|preparing|ready|failed|removed`; absent = no worktree.
    pub worktree_state: Option<String>,
    /// Last setup snapshot (stages with state/percent/tail).
    pub worktree_setup: Option<Value>,
    /// The PR/MR badge (`vcs::actions::PrInfo`).
    pub pr: Option<Value>,
    /// PTY session ids opened for this thread.
    pub terminals: Vec<String>,
    /// `{tool, sessionId, source, resumeCommand}` of a mirrored session.
    pub external: Option<Value>,
    /// Sidebar status, in priority order:
    /// `approval|input|running|error|idle`.
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRow {
    pub turn_id: String,
    pub thread_id: String,
    pub ordinal: u32,
    pub message_id: Option<String>,
    pub state: String,
    pub model: Option<String>,
    pub requested_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub message_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub role: String,
    pub text: String,
    pub attachments: Value,
    pub streaming: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRow {
    pub activity_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub kind: String,
    pub tone: String,
    pub summary: String,
    pub payload: Value,
    pub sequence: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRow {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub request_type: String,
    pub detail: Option<Value>,
    pub options: Value,
    /// `pending|answered|resolved`.
    pub status: String,
    pub decision: Option<String>,
    pub resolution: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputRow {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub questions: Value,
    pub response_mode: Option<String>,
    pub status: String,
    pub answers: Option<Value>,
    pub resolution: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRow {
    pub plan_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub explanation: Option<String>,
    pub steps: Option<Value>,
    pub markdown: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub turn_id: String,
    pub thread_id: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSessionRow {
    pub thread_id: String,
    pub instance_id: String,
    pub driver: String,
    pub status: Option<String>,
    pub resume_cursor: Option<Value>,
    pub provider_thread_id: Option<String>,
    pub last_error: Option<String>,
}

/// The shell: everything the sidebar and the inbox need, no bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sequence: i64,
    pub projects: Vec<ProjectRow>,
    pub threads: Vec<ThreadRow>,
    pub pending_approvals: Vec<ApprovalRow>,
    pub pending_user_inputs: Vec<UserInputRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnDetail {
    pub turn: TurnRow,
    pub messages: Vec<MessageRow>,
    pub activities: Vec<ActivityRow>,
    pub approvals: Vec<ApprovalRow>,
    pub user_inputs: Vec<UserInputRow>,
    pub plans: Vec<PlanRow>,
    pub usage: Option<UsageRow>,
    /// Checkpoint taken at the end of this turn, with its file stats.
    #[serde(default)]
    pub checkpoint: Option<CheckpointRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRow {
    pub thread_id: String,
    pub turn_count: u32,
    pub turn_id: Option<String>,
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    pub commit: Option<String>,
    pub status: String,
    pub files: Vec<super::model::CheckpointFile>,
    pub additions: u32,
    pub deletions: u32,
    pub created_at: String,
}

const CHECKPOINT_COLS: &str = "thread_id, turn_count, turn_id, ref, commit_sha, status, files_json,
    additions, deletions, created_at";

fn row_checkpoint(r: &Row) -> rusqlite::Result<CheckpointRow> {
    Ok(CheckpointRow {
        thread_id: r.get(0)?,
        turn_count: r.get::<_, i64>(1)? as u32,
        turn_id: r.get(2)?,
        ref_name: r.get(3)?,
        commit: r.get(4)?,
        status: r.get(5)?,
        files: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
        additions: r.get::<_, i64>(7)? as u32,
        deletions: r.get::<_, i64>(8)? as u32,
        created_at: r.get(9)?,
    })
}

/// Every checkpoint of a thread, baseline (0) first.
pub fn checkpoints(conn: &Connection, thread_id: &str) -> Result<Vec<CheckpointRow>, String> {
    collect(
        conn,
        &format!(
            "SELECT {CHECKPOINT_COLS} FROM checkpoints WHERE thread_id = ?1 ORDER BY turn_count ASC"
        ),
        [thread_id],
        row_checkpoint,
    )
}

pub fn checkpoint(
    conn: &Connection,
    thread_id: &str,
    turn_count: u32,
) -> Result<Option<CheckpointRow>, String> {
    conn.query_row(
        &format!(
            "SELECT {CHECKPOINT_COLS} FROM checkpoints WHERE thread_id = ?1 AND turn_count = ?2"
        ),
        params![thread_id, turn_count],
        row_checkpoint,
    )
    .optional()
    .map_err(db_err)
}

/// A turn of a thread by id.
pub fn turn_row(conn: &Connection, turn_id: &str) -> Result<Option<TurnRow>, String> {
    conn.query_row(
        &format!("SELECT {TURN_COLS} FROM turns WHERE turn_id = ?1"),
        [turn_id],
        row_turn,
    )
    .optional()
    .map_err(db_err)
}

/// A turn of a thread by ordinal.
pub fn turn_at(
    conn: &Connection,
    thread_id: &str,
    ordinal: u32,
) -> Result<Option<TurnRow>, String> {
    conn.query_row(
        &format!("SELECT {TURN_COLS} FROM turns WHERE thread_id = ?1 AND ordinal = ?2"),
        params![thread_id, ordinal],
        row_turn,
    )
    .optional()
    .map_err(db_err)
}

/// The user message that opened a turn: `(text, attachments)`.
pub fn turn_prompt(conn: &Connection, turn: &TurnRow) -> Result<Option<(String, Value)>, String> {
    let Some(mid) = &turn.message_id else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT text, attachments_json FROM messages WHERE message_id = ?1",
        [mid],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                json_or(r.get(1)?, Value::Array(vec![])),
            ))
        },
    )
    .optional()
    .map_err(db_err)
}

/// Usage rows of a thread, oldest turn first (cost per turn in the timeline).
pub fn thread_usage(conn: &Connection, thread_id: &str) -> Result<Vec<UsageRow>, String> {
    collect(
        conn,
        "SELECT u.turn_id, u.thread_id, u.model, u.input_tokens, u.cached_input_tokens, u.cache_write_tokens,
            u.output_tokens, u.reasoning_output_tokens, u.cost_usd, u.duration_ms
         FROM turn_usage u LEFT JOIN turns t ON t.turn_id = u.turn_id
         WHERE u.thread_id = ?1 ORDER BY t.ordinal ASC",
        [thread_id],
        row_usage,
    )
}

fn row_usage(r: &Row) -> rusqlite::Result<UsageRow> {
    Ok(UsageRow {
        turn_id: r.get(0)?,
        thread_id: r.get(1)?,
        model: r.get(2)?,
        input_tokens: r.get(3)?,
        cached_input_tokens: r.get(4)?,
        cache_write_tokens: r.get(5)?,
        output_tokens: r.get(6)?,
        reasoning_output_tokens: r.get(7)?,
        cost_usd: r.get(8)?,
        duration_ms: r.get(9)?,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnsPage {
    pub thread_id: String,
    /// Oldest first.
    pub turns: Vec<TurnDetail>,
    /// Messages/activities not tied to a turn (system notes); only on the
    /// last page (`hasMore == false`).
    pub loose_messages: Vec<MessageRow>,
    pub loose_activities: Vec<ActivityRow>,
    pub has_more: bool,
    /// Pass as `beforeTurn` to get the previous page.
    pub before_turn: Option<u32>,
    /// Global head when the page was read; apply live events up to here
    /// before merging it.
    pub sequence: i64,
    /// Last event of this thread.
    pub thread_sequence: i64,
    pub session: Option<ProviderSessionRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsPage {
    pub events: Vec<StoredEvent>,
    pub head: i64,
    /// The gap is over budget (or the cursor is ahead of the head): take a
    /// fresh snapshot instead.
    pub reset: bool,
    pub has_more: bool,
}

fn opt_json(s: Option<String>) -> Option<Value> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

fn json_or(s: String, fallback: Value) -> Value {
    serde_json::from_str(&s).unwrap_or(fallback)
}

fn thread_status(r: &ThreadRow) -> String {
    if r.pending_approval_count > 0 {
        "approval"
    } else if r.pending_user_input_count > 0 {
        "input"
    } else if r.active_turn_id.is_some() {
        "running"
    } else if r.session_status.as_deref() == Some("error") {
        "error"
    } else {
        "idle"
    }
    .to_string()
}

const THREAD_COLS: &str = "thread_id, project_id, title, instance_id, driver, model, agent_id, runtime_mode,
    interaction_mode, branch, worktree_path, forked_from_json, session_status, last_error, active_turn_id,
    latest_turn_id, turn_count, pending_approval_count, pending_user_input_count, pinned_at, snoozed_until,
    archived_at, last_visited_at, latest_user_message_at, created_at, updated_at, base_branch,
    worktree_state, worktree_setup_json, pr_json, terminals_json, external_json";

fn row_thread(r: &Row) -> rusqlite::Result<ThreadRow> {
    let mut t = ThreadRow {
        thread_id: r.get(0)?,
        project_id: r.get(1)?,
        title: r.get(2)?,
        instance_id: r.get(3)?,
        driver: r.get(4)?,
        model: r.get(5)?,
        agent_id: r.get(6)?,
        runtime_mode: r.get(7)?,
        interaction_mode: r.get(8)?,
        branch: r.get(9)?,
        worktree_path: r.get(10)?,
        forked_from: opt_json(r.get(11)?),
        session_status: r.get(12)?,
        last_error: r.get(13)?,
        active_turn_id: r.get(14)?,
        latest_turn_id: r.get(15)?,
        turn_count: r.get::<_, i64>(16)? as u32,
        pending_approval_count: r.get::<_, i64>(17)? as u32,
        pending_user_input_count: r.get::<_, i64>(18)? as u32,
        pinned_at: r.get(19)?,
        snoozed_until: r.get(20)?,
        archived_at: r.get(21)?,
        last_visited_at: r.get(22)?,
        latest_user_message_at: r.get(23)?,
        created_at: r.get(24)?,
        updated_at: r.get(25)?,
        base_branch: r.get(26)?,
        worktree_state: r.get(27)?,
        worktree_setup: opt_json(r.get(28)?),
        pr: opt_json(r.get(29)?),
        terminals: r
            .get::<_, Option<String>>(30)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        external: opt_json(r.get(31)?),
        status: String::new(),
    };
    t.status = thread_status(&t);
    Ok(t)
}

const APPROVAL_COLS: &str =
    "request_id, thread_id, turn_id, request_type, detail_json, options_json, status,
    decision, resolution, created_at, resolved_at";

fn row_approval(r: &Row) -> rusqlite::Result<ApprovalRow> {
    Ok(ApprovalRow {
        request_id: r.get(0)?,
        thread_id: r.get(1)?,
        turn_id: r.get(2)?,
        request_type: r.get(3)?,
        detail: opt_json(r.get(4)?),
        options: json_or(r.get(5)?, Value::Array(vec![])),
        status: r.get(6)?,
        decision: r.get(7)?,
        resolution: r.get(8)?,
        created_at: r.get(9)?,
        resolved_at: r.get(10)?,
    })
}

const INPUT_COLS: &str =
    "request_id, thread_id, turn_id, questions_json, response_mode, status, answers_json,
    resolution, created_at, resolved_at";

fn row_input(r: &Row) -> rusqlite::Result<UserInputRow> {
    Ok(UserInputRow {
        request_id: r.get(0)?,
        thread_id: r.get(1)?,
        turn_id: r.get(2)?,
        questions: json_or(r.get(3)?, Value::Array(vec![])),
        response_mode: r.get(4)?,
        status: r.get(5)?,
        answers: opt_json(r.get(6)?),
        resolution: r.get(7)?,
        created_at: r.get(8)?,
        resolved_at: r.get(9)?,
    })
}

const MESSAGE_COLS: &str =
    "message_id, thread_id, turn_id, role, text, attachments_json, is_streaming, created_at, updated_at";

fn row_message(r: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        message_id: r.get(0)?,
        thread_id: r.get(1)?,
        turn_id: r.get(2)?,
        role: r.get(3)?,
        text: r.get(4)?,
        attachments: json_or(r.get(5)?, Value::Array(vec![])),
        streaming: r.get::<_, i64>(6)? != 0,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

const ACTIVITY_COLS: &str =
    "activity_id, thread_id, turn_id, kind, tone, summary, payload_json, sequence, created_at, updated_at";

fn row_activity(r: &Row) -> rusqlite::Result<ActivityRow> {
    Ok(ActivityRow {
        activity_id: r.get(0)?,
        thread_id: r.get(1)?,
        turn_id: r.get(2)?,
        kind: r.get(3)?,
        tone: r.get(4)?,
        summary: r.get(5)?,
        payload: json_or(r.get(6)?, Value::Null),
        sequence: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

const TURN_COLS: &str =
    "turn_id, thread_id, ordinal, message_id, state, model, requested_at, started_at,
    completed_at, error_message";

fn row_turn(r: &Row) -> rusqlite::Result<TurnRow> {
    Ok(TurnRow {
        turn_id: r.get(0)?,
        thread_id: r.get(1)?,
        ordinal: r.get::<_, i64>(2)? as u32,
        message_id: r.get(3)?,
        state: r.get(4)?,
        model: r.get(5)?,
        requested_at: r.get(6)?,
        started_at: r.get(7)?,
        completed_at: r.get(8)?,
        error_message: r.get(9)?,
    })
}

fn collect<T>(
    conn: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
    f: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> Result<Vec<T>, String> {
    let mut stmt = conn.prepare(sql).map_err(db_err)?;
    let rows = stmt.query_map(p, f).map_err(db_err)?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn snapshot(conn: &Connection) -> Result<Snapshot, String> {
    let sequence = head(conn)?;
    let projects = collect(
        conn,
        "SELECT project_id, title, workspace_root, created_at, updated_at FROM projects
         WHERE deleted_at IS NULL ORDER BY created_at ASC",
        [],
        |r| {
            Ok(ProjectRow {
                project_id: r.get(0)?,
                title: r.get(1)?,
                workspace_root: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )?;
    let threads = collect(
        conn,
        &format!(
            "SELECT {THREAD_COLS} FROM threads WHERE deleted_at IS NULL ORDER BY updated_at DESC"
        ),
        [],
        row_thread,
    )?;
    let pending_approvals = collect(
        conn,
        &format!(
            "SELECT {APPROVAL_COLS} FROM approvals WHERE status = 'pending'
             AND thread_id IN (SELECT thread_id FROM threads WHERE deleted_at IS NULL) ORDER BY created_at ASC"
        ),
        [],
        row_approval,
    )?;
    let pending_user_inputs = collect(
        conn,
        &format!(
            "SELECT {INPUT_COLS} FROM user_inputs WHERE status = 'pending'
             AND thread_id IN (SELECT thread_id FROM threads WHERE deleted_at IS NULL) ORDER BY created_at ASC"
        ),
        [],
        row_input,
    )?;
    Ok(Snapshot {
        sequence,
        projects,
        threads,
        pending_approvals,
        pending_user_inputs,
    })
}

pub fn thread_row(conn: &Connection, thread_id: &str) -> Result<Option<ThreadRow>, String> {
    conn.query_row(
        &format!("SELECT {THREAD_COLS} FROM threads WHERE thread_id = ?1"),
        [thread_id],
        row_thread,
    )
    .optional()
    .map_err(db_err)
}

pub fn project_row(conn: &Connection, project_id: &str) -> Result<Option<ProjectRow>, String> {
    conn.query_row(
        "SELECT project_id, title, workspace_root, created_at, updated_at FROM projects WHERE project_id = ?1",
        [project_id],
        |r| {
            Ok(ProjectRow {
                project_id: r.get(0)?,
                title: r.get(1)?,
                workspace_root: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

pub fn provider_session(
    conn: &Connection,
    thread_id: &str,
) -> Result<Option<ProviderSessionRow>, String> {
    conn.query_row(
        "SELECT thread_id, instance_id, driver, status, resume_cursor_json, provider_thread_id, last_error
         FROM provider_sessions WHERE thread_id = ?1",
        [thread_id],
        |r| {
            Ok(ProviderSessionRow {
                thread_id: r.get(0)?,
                instance_id: r.get(1)?,
                driver: r.get(2)?,
                status: r.get(3)?,
                resume_cursor: opt_json(r.get(4)?),
                provider_thread_id: r.get(5)?,
                last_error: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

/// Replay `sequence > after`, bounded by 1000 events / 8 MiB of payload
/// (summed in SQL, nothing decoded). Over budget → `reset`.
pub fn events_after(conn: &Connection, after: i64, limit: i64) -> Result<EventsPage, String> {
    let head = head(conn)?;
    if after > head {
        return Ok(EventsPage {
            events: Vec::new(),
            head,
            reset: true,
            has_more: false,
        });
    }
    let (count, bytes): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(payload_json)), 0) FROM
                (SELECT payload_json FROM events WHERE sequence > ?1 AND sequence <= ?2
                 ORDER BY sequence LIMIT ?3)",
            params![after, head, REPLAY_MAX_EVENTS + 1],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_err)?;
    if count > REPLAY_MAX_EVENTS || bytes > REPLAY_MAX_BYTES {
        return Ok(EventsPage {
            events: Vec::new(),
            head,
            reset: true,
            has_more: false,
        });
    }
    let limit = limit.clamp(1, REPLAY_MAX_EVENTS);
    let mut stmt = conn
        .prepare(
            "SELECT * FROM events WHERE sequence > ?1 AND sequence <= ?2 ORDER BY sequence ASC LIMIT ?3",
        )
        .map_err(db_err)?;
    let events: Vec<StoredEvent> = stmt
        .query_map(params![after, head, limit], row_to_event)
        .map_err(db_err)?
        .filter_map(Result::ok)
        .collect();
    Ok(EventsPage {
        has_more: count > events.len() as i64,
        events,
        head,
        reset: false,
    })
}

/// Page of whole turns, newest `limit` before `before_turn` (ordinal,
/// exclusive), returned oldest first. The UI asks 10 first, then 20.
pub fn turns_page(
    conn: &Connection,
    thread_id: &str,
    before_turn: Option<u32>,
    limit: u32,
) -> Result<TurnsPage, String> {
    let limit = limit.clamp(1, 200) as i64;
    let before = before_turn.map(|b| b as i64).unwrap_or(i64::MAX);
    let mut turns = collect(
        conn,
        &format!(
            "SELECT {TURN_COLS} FROM turns WHERE thread_id = ?1 AND ordinal < ?2 ORDER BY ordinal DESC LIMIT ?3"
        ),
        params![thread_id, before, limit + 1],
        row_turn,
    )?;
    let has_more = turns.len() as i64 > limit;
    turns.truncate(limit as usize);
    turns.reverse();
    let mut details = Vec::with_capacity(turns.len());
    for turn in turns {
        let tid = turn.turn_id.clone();
        let messages = collect(
            conn,
            &format!("SELECT {MESSAGE_COLS} FROM messages WHERE turn_id = ?1 ORDER BY created_at ASC, rowid ASC"),
            [&tid],
            row_message,
        )?;
        let activities = collect(
            conn,
            &format!("SELECT {ACTIVITY_COLS} FROM activities WHERE turn_id = ?1 ORDER BY created_at ASC, sequence ASC"),
            [&tid],
            row_activity,
        )?;
        let approvals = collect(
            conn,
            &format!("SELECT {APPROVAL_COLS} FROM approvals WHERE thread_id = ?1 AND turn_id = ?2 ORDER BY created_at ASC"),
            params![thread_id, &tid],
            row_approval,
        )?;
        let user_inputs = collect(
            conn,
            &format!("SELECT {INPUT_COLS} FROM user_inputs WHERE thread_id = ?1 AND turn_id = ?2 ORDER BY created_at ASC"),
            params![thread_id, &tid],
            row_input,
        )?;
        let plans = collect(
            conn,
            "SELECT plan_id, thread_id, turn_id, explanation, steps_json, markdown, updated_at FROM plans WHERE turn_id = ?1",
            [&tid],
            |r| {
                Ok(PlanRow {
                    plan_id: r.get(0)?,
                    thread_id: r.get(1)?,
                    turn_id: r.get(2)?,
                    explanation: r.get(3)?,
                    steps: opt_json(r.get(4)?),
                    markdown: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            },
        )?;
        let usage = conn
            .query_row(
                "SELECT turn_id, thread_id, model, input_tokens, cached_input_tokens, cache_write_tokens,
                    output_tokens, reasoning_output_tokens, cost_usd, duration_ms FROM turn_usage WHERE turn_id = ?1",
                [&tid],
                row_usage,
            )
            .optional()
            .map_err(db_err)?;
        let checkpoint = checkpoint(conn, thread_id, turn.ordinal)?;
        details.push(TurnDetail {
            turn,
            messages,
            activities,
            approvals,
            user_inputs,
            plans,
            usage,
            checkpoint,
        });
    }
    let (loose_messages, loose_activities) = if has_more {
        (Vec::new(), Vec::new())
    } else {
        (
            collect(
                conn,
                &format!("SELECT {MESSAGE_COLS} FROM messages WHERE thread_id = ?1 AND turn_id IS NULL ORDER BY created_at ASC"),
                [thread_id],
                row_message,
            )?,
            collect(
                conn,
                &format!("SELECT {ACTIVITY_COLS} FROM activities WHERE thread_id = ?1 AND turn_id IS NULL ORDER BY created_at ASC"),
                [thread_id],
                row_activity,
            )?,
        )
    };
    let before_turn = details.first().map(|d| d.turn.ordinal).filter(|_| has_more);
    let sequence = head(conn)?;
    let thread_sequence = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE aggregate_kind = 'thread' AND stream_id = ?1",
            [thread_id],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    Ok(TurnsPage {
        thread_id: thread_id.to_string(),
        turns: details,
        loose_messages,
        loose_activities,
        has_more,
        before_turn,
        sequence,
        thread_sequence,
        session: provider_session(conn, thread_id)?,
    })
}

/// The first `keep` turns of a thread, with bodies, for `thread.fork`.
pub fn fork_source(conn: &Connection, thread_id: &str, keep: u32) -> Result<ForkSource, String> {
    let turns = collect(
        conn,
        &format!("SELECT {TURN_COLS} FROM turns WHERE thread_id = ?1 AND ordinal <= ?2 ORDER BY ordinal ASC"),
        params![thread_id, keep],
        row_turn,
    )?;
    let mut out = ForkSource::default();
    for t in turns {
        let messages = collect(
            conn,
            "SELECT message_id, role, text, created_at FROM messages WHERE turn_id = ?1 ORDER BY created_at ASC, rowid ASC",
            [&t.turn_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let activities = collect(
            conn,
            &format!(
                "SELECT {ACTIVITY_COLS} FROM activities WHERE turn_id = ?1 ORDER BY created_at ASC"
            ),
            [&t.turn_id],
            row_activity,
        )?
        .into_iter()
        .map(|a| super::model::Activity {
            activity_id: a.activity_id,
            turn_id: a.turn_id,
            kind: a.kind,
            tone: a.tone,
            summary: a.summary,
            payload: a.payload,
            created_at: a.created_at,
        })
        .collect();
        let state = match t.state.as_str() {
            "pending" | "running" => "interrupted".to_string(),
            s => s.to_string(),
        };
        out.turns.push(ForkTurn {
            turn_id: t.turn_id,
            ordinal: t.ordinal,
            state,
            requested_at: t.requested_at,
            completed_at: t.completed_at,
            messages,
            activities,
        });
    }
    Ok(out)
}

/// Turns still `pending|running` (a crash left them), for the boot reconcile.
pub fn dangling_turns(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    collect(
        conn,
        "SELECT t.thread_id, t.turn_id FROM turns t JOIN threads h ON h.thread_id = t.thread_id
         WHERE t.state IN ('pending', 'running') AND h.deleted_at IS NULL",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}
