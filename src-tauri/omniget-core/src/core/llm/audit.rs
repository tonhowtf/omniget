//! The one audit log of the `/llm` (2.8, S14, `80#P5`): who did what to which
//! job, question, grant, token or agent, through which channel, and when.
//! One SQLite file, `<app_data>/llm/audit.db`, kept 90 days by default (the
//! `llm.audit_retention_days` setting).
//!
//! What a line never carries: a prompt, a file's content, a tool's input or
//! output, a secret. [`redact`] drops the keys that hold such things
//! (`prompt`, `content`, `input`, `preview`, …) and scrubs every string with
//! the missions' redactor (`sk-…`, `Bearer …`, JWTs, `key=value` secrets,
//! home paths) plus the bridge's own token shapes (`ogt_…`, `ojt_…`).
//!
//! [`log`] is synchronous on the caller's thread: one small insert in WAL
//! mode, measured under 1 ms per event (`an_event_costs_under_a_millisecond`).
//! With no log opened (tests, a CLI) it does nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use regex::Regex;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

/// Who did it, and through what (`http`, `cli`, `window`, `mcp`, `remote`,
/// `turn`, `timeout`, `boot`, `system`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    /// `window`, `system`, `token:<id>`, `job:<id>`, `extension`,
    /// `mcp:<client>`, `device:<id>`, `agent:<id>`, `trigger:<id>`.
    pub id: String,
    pub channel: String,
}

impl Actor {
    pub fn via(channel: &str, id: &str) -> Self {
        Self {
            id: id.to_string(),
            channel: channel.to_string(),
        }
    }

    /// A person at the desktop window.
    pub fn window() -> Self {
        Self::via("window", "window")
    }

    /// The app itself (a job ending, a question expiring).
    pub fn system(channel: &str) -> Self {
        Self::via(channel, "system")
    }

    /// An agent's turn (it raised a question).
    pub fn agent(agent_id: &str) -> Self {
        Self::via("turn", &format!("agent:{agent_id}"))
    }
}

impl From<&crate::core::assist::runs::Actor> for Actor {
    fn from(a: &crate::core::assist::runs::Actor) -> Self {
        Self::via(&a.channel, &a.decided_by)
    }
}

/// What it was done to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub kind: &'static str,
    pub id: String,
}

impl Entity {
    fn of(kind: &'static str, id: &str) -> Self {
        Self {
            kind,
            id: id.to_string(),
        }
    }
    pub fn job(id: &str) -> Self {
        Self::of("job", id)
    }
    pub fn ask(id: &str) -> Self {
        Self::of("ask", id)
    }
    pub fn agent(id: &str) -> Self {
        Self::of("agent", id)
    }
    pub fn token(id: &str) -> Self {
        Self::of("token", id)
    }
    /// An agent route of the bridge (`POST /v1/agent/run`).
    pub fn route(id: &str) -> Self {
        Self::of("route", id)
    }
    /// A stored permission rule (`<agent>:<tool>`).
    pub fn rule(id: &str) -> Self {
        Self::of("rule", id)
    }
    pub fn loop_(id: &str) -> Self {
        Self::of("loop", id)
    }
    pub fn setting(id: &str) -> Self {
        Self::of("setting", id)
    }
}

/// One line as it is read back.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub id: i64,
    pub ts_ms: i64,
    pub actor: String,
    pub channel: String,
    pub action: String,
    pub entity_kind: String,
    pub entity_id: String,
    pub details: Value,
}

/// What to read: since when (epoch ms), optionally one action prefix
/// (`job.`, `ask.answered`) or one entity id, newest first.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub since_ms: Option<i64>,
    pub action: Option<String>,
    pub entity_id: Option<String>,
    pub limit: Option<usize>,
}

pub const DEFAULT_RETENTION_DAYS: u32 = 90;
/// A details object bigger than this is cut (an audit line is a fact, not a
/// payload).
const DETAILS_MAX: usize = 4096;
const LIST_MAX: usize = 5000;
const PRUNE_EVERY_MS: u64 = 3_600_000;

pub struct Audit {
    db: Mutex<Connection>,
    retention_ms: AtomicU64,
    pruned_ms: AtomicU64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Audit {
    /// Opens (or creates) the log at `path` and drops what is older than
    /// `retention_days`.
    pub fn open(path: &std::path::Path, retention_days: u32) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_ms INTEGER NOT NULL,
                actor TEXT NOT NULL,
                channel TEXT NOT NULL,
                action TEXT NOT NULL,
                entity_kind TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                details TEXT NOT NULL DEFAULT '{}');
             CREATE INDEX IF NOT EXISTS audit_ts ON audit(ts_ms);
             CREATE INDEX IF NOT EXISTS audit_entity ON audit(entity_id);",
        )
        .map_err(|e| e.to_string())?;
        let audit = Self {
            db: Mutex::new(db),
            retention_ms: AtomicU64::new(days_ms(retention_days)),
            pruned_ms: AtomicU64::new(0),
        };
        audit.prune(now_ms());
        Ok(audit)
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_retention_days(&self, days: u32) {
        self.retention_ms.store(days_ms(days), Ordering::Relaxed);
        self.prune(now_ms());
    }

    pub fn retention_days(&self) -> u32 {
        (self.retention_ms.load(Ordering::Relaxed) / 86_400_000) as u32
    }

    /// Drops the lines older than the retention. Returns how many.
    pub fn prune(&self, now: u64) -> usize {
        self.pruned_ms.store(now, Ordering::Relaxed);
        let cutoff = now.saturating_sub(self.retention_ms.load(Ordering::Relaxed)) as i64;
        self.db()
            .execute("DELETE FROM audit WHERE ts_ms < ?1", params![cutoff])
            .unwrap_or(0)
    }

    /// Writes one line at `ts_ms`. `details` goes through [`redact`].
    pub fn log_at(
        &self,
        ts_ms: u64,
        actor: &Actor,
        action: &str,
        entity: &Entity,
        details: Value,
    ) -> Result<i64, String> {
        let mut text = redact(details).to_string();
        if text.len() > DETAILS_MAX {
            text = json!({ "clipped": true, "bytes": text.len() }).to_string();
        }
        let db = self.db();
        db.execute(
            "INSERT INTO audit (ts_ms, actor, channel, action, entity_kind, entity_id, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ts_ms as i64,
                scrub_id(&actor.id),
                scrub_id(&actor.channel),
                action,
                entity.kind,
                scrub_id(&entity.id),
                text
            ],
        )
        .map_err(|e| e.to_string())?;
        let id = db.last_insert_rowid();
        drop(db);
        if ts_ms.saturating_sub(self.pruned_ms.load(Ordering::Relaxed)) > PRUNE_EVERY_MS {
            self.prune(ts_ms);
        }
        Ok(id)
    }

    pub fn log(
        &self,
        actor: &Actor,
        action: &str,
        entity: &Entity,
        details: Value,
    ) -> Result<i64, String> {
        self.log_at(now_ms(), actor, action, entity, details)
    }

    /// Newest first.
    pub fn list(&self, q: &Query) -> Vec<Row> {
        let limit = q.limit.unwrap_or(200).clamp(1, LIST_MAX) as i64;
        let action = q
            .action
            .as_deref()
            .filter(|a| !a.is_empty())
            .map(|a| format!("{}%", a.replace('%', "")));
        let db = self.db();
        let Ok(mut stmt) = db.prepare(
            "SELECT id, ts_ms, actor, channel, action, entity_kind, entity_id, details FROM audit
             WHERE ts_ms >= ?1 AND (?2 IS NULL OR action LIKE ?2) AND (?3 IS NULL OR entity_id = ?3)
             ORDER BY ts_ms DESC, id DESC LIMIT ?4",
        ) else {
            return Vec::new();
        };
        stmt.query_map(
            params![q.since_ms.unwrap_or(0), action, q.entity_id, limit],
            |r| {
                Ok(Row {
                    id: r.get(0)?,
                    ts_ms: r.get(1)?,
                    actor: r.get(2)?,
                    channel: r.get(3)?,
                    action: r.get(4)?,
                    entity_kind: r.get(5)?,
                    entity_id: r.get(6)?,
                    details: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(Value::Null),
                })
            },
        )
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }
}

fn days_ms(days: u32) -> u64 {
    u64::from(days.clamp(1, 3650)) * 86_400_000
}

// ── the one log of the app ──────────────────────────────────────────────

static GLOBAL: RwLock<Option<Arc<Audit>>> = RwLock::new(None);

/// Opens `<dir>/audit.db` as the app's log (the app calls this once, with
/// the `llm` directory and the retention setting).
pub fn init(path: &std::path::Path, retention_days: u32) -> Result<(), String> {
    let audit = Arc::new(Audit::open(path, retention_days)?);
    *GLOBAL.write().unwrap_or_else(|e| e.into_inner()) = Some(audit);
    Ok(())
}

pub fn global() -> Option<Arc<Audit>> {
    GLOBAL.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// One line in the app's log. Never fails the caller: a log that cannot be
/// written says so in the tracing log and the action goes on.
pub fn log(actor: Actor, action: &str, entity: Entity, details: Value) {
    if let Some(audit) = global() {
        if let Err(e) = audit.log(&actor, action, &entity, details) {
            tracing::warn!("[audit] {action} {}: {e}", entity.id);
        }
    }
}

pub fn list(q: &Query) -> Vec<Row> {
    global().map(|a| a.list(q)).unwrap_or_default()
}

/// `1d`, `12h`, `30m`, `90s` or a plain number of ms → a span in ms.
pub fn parse_since(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    let (num, unit) = raw.split_at(raw.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = num.parse().ok()?;
    let ms = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => return None,
    };
    Some(n.saturating_mul(ms))
}

// ── redaction ───────────────────────────────────────────────────────────

/// Keys whose value is content, never a fact about it: dropped whole.
const CONTENT_KEYS: &[&str] = &[
    "prompt",
    "system_prompt",
    "content",
    "input",
    "tool_input",
    "arguments",
    "args",
    "text",
    "body",
    "message",
    "messages",
    "preview",
    "patch",
    "diff",
    "result",
    "output",
    "stdin",
    "stdout",
    "stderr",
    "log",
    "raw",
];

/// What a line keeps of `details`: content keys become `"[omitted]"`, every
/// string is scrubbed of secrets.
pub fn redact(details: Value) -> Value {
    fn walk(v: Value) -> Value {
        match v {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| {
                        if CONTENT_KEYS.contains(&k.to_ascii_lowercase().as_str()) {
                            (k, Value::String("[omitted]".into()))
                        } else {
                            (k, walk(v))
                        }
                    })
                    .collect(),
            ),
            Value::Array(list) => Value::Array(list.into_iter().map(walk).collect()),
            Value::String(s) => Value::String(scrub(&s)),
            other => other,
        }
    }
    crate::core::assist::missions::diag::redact_json(walk(details))
}

/// The missions' redactor, then what it leaves behind on purpose (the
/// `Bearer` word, a short `sk-`) and the bridge's own token shapes.
fn scrub(text: &str) -> String {
    scrub_id(&crate::core::assist::missions::diag::redact(text))
}

/// Ids (`token:ab12`, `job:j1`) are not secrets and the missions' redactor
/// would read `token:…` as one: only the secret shapes themselves go.
fn scrub_id(text: &str) -> String {
    static EXTRA: OnceLock<Regex> = OnceLock::new();
    let extra = EXTRA.get_or_init(|| {
        Regex::new(r"(?i)\bbearer\s+\S+|\bbearer\b|\bsk-[A-Za-z0-9_-]*|\bo[gj]t_[A-Za-z0-9_]+")
            .unwrap()
    });
    extra.replace_all(text, "[redacted]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("omniget-audit-{}", uuid::Uuid::new_v4().simple()))
            .join("audit.db")
    }

    #[test]
    fn a_line_says_who_what_which_and_how() {
        let a = Audit::open(&temp(), 90).unwrap();
        a.log(
            &Actor::via("http", "token:ab12"),
            "job.created",
            &Entity::job("j1"),
            json!({"agent_id": "worker", "run_mode": "approval_required"}),
        )
        .unwrap();
        let rows = a.list(&Query::default());
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(
            (r.actor.as_str(), r.channel.as_str(), r.action.as_str()),
            ("token:ab12", "http", "job.created")
        );
        assert_eq!(
            (r.entity_kind.as_str(), r.entity_id.as_str()),
            ("job", "j1")
        );
        assert_eq!(r.details["run_mode"], "approval_required");
        assert_eq!(
            a.list(&Query {
                action: Some("ask.".into()),
                ..Query::default()
            })
            .len(),
            0
        );
    }

    #[test]
    fn prompts_content_and_secrets_never_reach_the_file() {
        let path = temp();
        let a = Audit::open(&path, 90).unwrap();
        a.log(
            &Actor::via("http", "token:x"),
            "job.created",
            &Entity::job("j1"),
            json!({
                "prompt": "deploy with key sk-ant-api03-SECRETSECRETSECRET",
                "tool": "shell_exec",
                "preview": "cat ~/.ssh/id_rsa",
                "note": "called with Authorization: Bearer ogt_abcdef123456_0123456789abcdef and sk-abc",
                "nested": {"content": "file body", "header": "Bearer xyz.abc"},
                "api_key": "hunter2hunter2",
            }),
        )
        .unwrap();
        drop(a);
        // Every byte of the database, WAL included.
        let mut bytes = Vec::new();
        for f in ["audit.db", "audit.db-wal"] {
            if let Ok(b) = std::fs::read(path.with_file_name(f)) {
                bytes.extend(b);
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        for needle in [
            "sk-",
            "Bearer ",
            "ogt_",
            "SECRETSECRET",
            "id_rsa",
            "file body",
            "hunter2",
        ] {
            assert!(!text.contains(needle), "{needle} in audit.db");
        }
        let a = Audit::open(&path, 90).unwrap();
        let row = &a.list(&Query::default())[0];
        assert_eq!(row.details["tool"], "shell_exec");
        assert_eq!(row.details["prompt"], "[omitted]");
    }

    #[test]
    fn old_lines_go_after_the_retention() {
        let a = Audit::open(&temp(), 90).unwrap();
        let now = now_ms();
        let day = 86_400_000;
        for (age, id) in [(91 * day, "old"), (89 * day, "kept"), (0, "new")] {
            a.log_at(
                now - age,
                &Actor::system("system"),
                "job.ended",
                &Entity::job(id),
                json!({}),
            )
            .unwrap();
        }
        assert_eq!(a.prune(now), 1);
        let ids: Vec<String> = a
            .list(&Query::default())
            .into_iter()
            .map(|r| r.entity_id)
            .collect();
        assert_eq!(ids, ["new", "kept"]);
        a.set_retention_days(1);
        assert_eq!(a.list(&Query::default()).len(), 1);
        assert_eq!(a.retention_days(), 1);
        let since = a.list(&Query {
            since_ms: Some((now - 1_000) as i64),
            ..Query::default()
        });
        assert_eq!(since.len(), 1);
    }

    #[test]
    fn since_reads_days_hours_and_minutes() {
        assert_eq!(parse_since("1d"), Some(86_400_000));
        assert_eq!(parse_since("12h"), Some(12 * 3_600_000));
        assert_eq!(parse_since("30m"), Some(1_800_000));
        assert_eq!(parse_since("250ms"), Some(250));
        assert_eq!(parse_since("1y"), None);
        assert_eq!(parse_since("d"), None);
    }

    /// The overhead goal of `80#P5`: under 1 ms per event, redaction and
    /// the insert included, measured over 1000 events.
    #[test]
    fn an_event_costs_under_a_millisecond() {
        let a = Audit::open(&temp(), 90).unwrap();
        let n = 1000;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            a.log(
                &Actor::via("cli", "token:ab12"),
                "ask.answered",
                &Entity::ask(&format!("apr_{i}")),
                json!({"decision": "accept", "tool": "shell_exec", "job_id": "j1"}),
            )
            .unwrap();
        }
        let per_event_ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "{}",
                    json!({"bench": "audit", "metric": "ms_per_event", "value": per_event_ms, "events": n})
                );
            }
        }
        eprintln!("audit: {per_event_ms:.4} ms per event over {n}");
        assert!(per_event_ms < 1.0, "{per_event_ms} ms per event");
    }
}
