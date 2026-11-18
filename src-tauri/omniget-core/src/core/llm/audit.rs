use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use regex::Regex;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
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

    pub fn window() -> Self {
        Self::via("window", "window")
    }

    pub fn system(channel: &str) -> Self {
        Self::via(channel, "system")
    }

    pub fn agent(agent_id: &str) -> Self {
        Self::via("turn", &format!("agent:{agent_id}"))
    }
}

impl From<&crate::core::assist::runs::Actor> for Actor {
    fn from(a: &crate::core::assist::runs::Actor) -> Self {
        Self::via(&a.channel, &a.decided_by)
    }
}

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
    pub fn route(id: &str) -> Self {
        Self::of("route", id)
    }
    pub fn rule(id: &str) -> Self {
        Self::of("rule", id)
    }
    pub fn loop_(id: &str) -> Self {
        Self::of("loop", id)
    }
    pub fn budget(id: &str) -> Self {
        Self::of("budget", id)
    }
    pub fn app(pid: &str) -> Self {
        Self::of("app", pid)
    }
    pub fn workflow(root: &str) -> Self {
        Self::of("workflow", root)
    }
}

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

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub since_ms: Option<i64>,
    pub action: Option<String>,
    pub entity_id: Option<String>,
    pub limit: Option<usize>,
}

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

    pub fn prune(&self, now: u64) -> usize {
        self.pruned_ms.store(now, Ordering::Relaxed);
        let cutoff = now.saturating_sub(self.retention_ms.load(Ordering::Relaxed)) as i64;
        self.db()
            .execute("DELETE FROM audit WHERE ts_ms < ?1", params![cutoff])
            .unwrap_or(0)
    }

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

static GLOBAL: RwLock<Option<Arc<Audit>>> = RwLock::new(None);

pub fn init(path: &std::path::Path, retention_days: u32) -> Result<(), String> {
    let audit = Arc::new(Audit::open(path, retention_days)?);
    *GLOBAL.write().unwrap_or_else(|e| e.into_inner()) = Some(audit);
    Ok(())
}

pub fn global() -> Option<Arc<Audit>> {
    GLOBAL.read().unwrap_or_else(|e| e.into_inner()).clone()
}

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

fn scrub(text: &str) -> String {
    scrub_id(&crate::core::assist::missions::diag::redact(text))
}

fn scrub_id(text: &str) -> String {
    static EXTRA: OnceLock<Regex> = OnceLock::new();
    let extra = EXTRA.get_or_init(|| {
        Regex::new(r"(?i)\bbearer\s+\S+|\bbearer\b|\bsk-[A-Za-z0-9_-]*|\bo[gj]t_[A-Za-z0-9_]+")
            .unwrap()
    });
    extra.replace_all(text, "[redacted]").into_owned()
}
