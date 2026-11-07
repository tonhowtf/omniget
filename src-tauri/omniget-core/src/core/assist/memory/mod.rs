pub mod consolidate;
pub mod graph;
mod records;
pub mod reflect;
pub mod retain;
pub mod retrieve;
mod search;
pub mod temporal;
mod toolset;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ctx::Scope;
use super::db::Migration;

pub use graph::{Entity, EntityRef};
pub use records::{
    correct, expire_due, forget, get, history, list, remember, retract, Correction, Written,
};
pub use reflect::Reflection;
pub use retain::{RetainInput, RetainReport};
pub use retrieve::{Hit, Probe, Why};
pub use search::{augment_text, profile, recall, source_line, Profile};
pub use toolset::{augment, call_tool, has_recall_grant, toolset};

pub const ERR_MEMORY_NOT_FOUND: &str = "ERR_MEMORY_NOT_FOUND";
pub const ERR_MEMORY_INVALID: &str = "ERR_MEMORY_INVALID";
pub const ERR_MEMORY_FORGOTTEN: &str = "ERR_MEMORY_FORGOTTEN";
pub const ERR_MEMORY_INACTIVE: &str = "ERR_MEMORY_INACTIVE";

pub const MAX_CONTENT_CHARS: usize = 4000;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        module: "memory",
        version: 1,
        sql: "
CREATE TABLE memory_records(
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL,
    scope TEXT NOT NULL,
    category TEXT NOT NULL CHECK(category IN ('declared','observation','inference','temporary','procedural')),
    subject TEXT,
    content TEXT NOT NULL,
    data TEXT,
    source_kind TEXT NOT NULL,
    source_id TEXT,
    source_conversation TEXT,
    author TEXT NOT NULL,
    source_ms INTEGER NOT NULL,
    confidence REAL NOT NULL,
    evidence TEXT,
    valid_until INTEGER,
    version INTEGER NOT NULL,
    chain TEXT NOT NULL,
    supersedes TEXT,
    status TEXT NOT NULL CHECK(status IN ('active','superseded','retracted','expired')),
    origin_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX memory_records_scope ON memory_records(principal, scope, status);
CREATE INDEX memory_records_chain ON memory_records(chain, version);
CREATE INDEX memory_records_origin ON memory_records(origin_hash);
CREATE INDEX memory_records_content ON memory_records(content_hash);
CREATE INDEX memory_records_subject ON memory_records(principal, scope, subject) WHERE subject IS NOT NULL;
CREATE TABLE memory_tombstones(
    hash TEXT PRIMARY KEY,
    principal TEXT NOT NULL,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('content','origin','record')),
    forgotten_ms INTEGER NOT NULL
);
CREATE TABLE memory_profile_cache(
    key TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    ids TEXT NOT NULL,
    built_ms INTEGER NOT NULL,
    expires_ms INTEGER
);
CREATE VIRTUAL TABLE memory_fts USING fts5(
    content, subject, record_id UNINDEXED, scope UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
",
    },
    Migration {
        module: "memory",
        version: 2,
        sql: "
CREATE TABLE memory_meta(
    record_id TEXT PRIMARY KEY,
    occurred_start INTEGER,
    occurred_end INTEGER,
    proofs INTEGER NOT NULL DEFAULT 1,
    hits INTEGER NOT NULL DEFAULT 0,
    last_hit_ms INTEGER
);
CREATE INDEX memory_meta_when ON memory_meta(occurred_start);
CREATE TABLE memory_vectors(
    record_id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    dim INTEGER NOT NULL,
    vec BLOB NOT NULL
);
CREATE TABLE memory_entities(
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL,
    name TEXT NOT NULL,
    norm TEXT NOT NULL,
    kind TEXT NOT NULL,
    mentions INTEGER NOT NULL DEFAULT 0,
    first_ms INTEGER NOT NULL,
    last_ms INTEGER NOT NULL
);
CREATE UNIQUE INDEX memory_entities_norm ON memory_entities(principal, norm);
CREATE TABLE memory_entity_refs(
    record_id TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    PRIMARY KEY(record_id, entity_id)
);
CREATE INDEX memory_entity_refs_entity ON memory_entity_refs(entity_id);
CREATE TABLE memory_links(
    src TEXT NOT NULL,
    dst TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('semantic','causal','temporal')),
    weight REAL NOT NULL,
    PRIMARY KEY(src, dst, kind)
);
CREATE INDEX memory_links_dst ON memory_links(dst);
CREATE TABLE memory_entity_notes(
    entity_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    text TEXT NOT NULL,
    ids TEXT NOT NULL,
    built_ms INTEGER NOT NULL,
    PRIMARY KEY(entity_id, scope)
);
CREATE TABLE memory_dirty_entities(
    entity_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    PRIMARY KEY(entity_id, scope)
);
CREATE TABLE memory_retain_log(
    key TEXT PRIMARY KEY,
    conversation TEXT,
    bot TEXT,
    at_ms INTEGER NOT NULL,
    facts INTEGER NOT NULL,
    reinforced INTEGER NOT NULL,
    status TEXT NOT NULL,
    detail TEXT
);
CREATE INDEX memory_retain_log_at ON memory_retain_log(at_ms);
CREATE TABLE memory_settings(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
",
    },
];

pub const TOOL_NAMES: &[&str] = &[
    "memory_recall",
    "memory_remember",
    "memory_correct",
    "memory_forget",
    "memory_profile",
    "memory_reflect",
    "memory_retain",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Declared,
    Observation,
    Inference,
    Temporary,
    Procedural,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Declared => "declared",
            Category::Observation => "observation",
            Category::Inference => "inference",
            Category::Temporary => "temporary",
            Category::Procedural => "procedural",
        }
    }

    pub fn parse(s: &str) -> Option<Category> {
        Some(match s.trim() {
            "declared" => Category::Declared,
            "observation" => Category::Observation,
            "inference" => Category::Inference,
            "temporary" => Category::Temporary,
            "procedural" => Category::Procedural,
            _ => return None,
        })
    }

    fn conflict_group(self) -> Option<&'static str> {
        match self {
            Category::Declared | Category::Inference => Some("preference"),
            Category::Temporary => Some("temporary"),
            Category::Procedural => Some("procedural"),
            Category::Observation => None,
        }
    }

    fn default_confidence(self) -> f64 {
        match self {
            Category::Inference => 0.6,
            Category::Procedural => 0.9,
            _ => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Superseded,
    Retracted,
    Expired,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Superseded => "superseded",
            Status::Retracted => "retracted",
            Status::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "active" => Status::Active,
            "superseded" => Status::Superseded,
            "retracted" => Status::Retracted,
            "expired" => Status::Expired,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub conversation: Option<String>,
    pub author: String,
    pub at_ms: i64,
}

impl Source {
    pub fn user(now: i64) -> Self {
        Self {
            kind: "user".into(),
            id: None,
            conversation: None,
            author: "user".into(),
            at_ms: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub scope: Scope,
    pub category: Category,
    #[serde(default)]
    pub subject: Option<String>,
    pub content: String,
    #[serde(default)]
    pub data: Option<Value>,
    pub source: Source,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub valid_until: Option<i64>,
    pub version: i64,
    pub chain: String,
    #[serde(default)]
    pub supersedes: Option<String>,
    pub status: Status,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone)]
pub struct NewMemory {
    pub scope: Scope,
    pub category: Category,
    pub content: String,
    pub subject: Option<String>,
    pub data: Option<Value>,
    pub source: Source,
    pub confidence: Option<f64>,
    pub evidence: Option<String>,
    pub valid_until: Option<i64>,
}

impl NewMemory {
    pub fn new(scope: Scope, category: Category, content: &str, source: Source) -> Self {
        Self {
            scope,
            category,
            content: content.to_string(),
            subject: None,
            data: None,
            source,
            confidence: None,
            evidence: None,
            valid_until: None,
        }
    }

    pub fn subject(mut self, s: &str) -> Self {
        self.subject = Some(s.to_string());
        self
    }

    pub fn valid_until(mut self, ms: i64) -> Self {
        self.valid_until = Some(ms);
        self
    }

    pub fn evidence(mut self, e: &str, confidence: f64) -> Self {
        self.evidence = Some(e.to_string());
        self.confidence = Some(confidence);
        self
    }

    pub fn data(mut self, v: Value) -> Self {
        self.data = Some(v);
        self
    }
}

pub(crate) fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_string()
}

pub(crate) fn sha(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update([0x1f]);
        }
        h.update(p.as_bytes());
    }
    format!("{:x}", h.finalize())
}

pub(crate) fn content_hash(principal: &str, scope: &str, content: &str) -> String {
    sha(&["content", principal, scope, &normalize(content)])
}

pub(crate) fn origin_hash(
    principal: &str,
    scope: &str,
    source_kind: &str,
    source_id: Option<&str>,
    content: &str,
) -> String {
    sha(&[
        "origin",
        principal,
        scope,
        source_kind,
        source_id.unwrap_or(""),
        &normalize(content),
    ])
}

pub(crate) fn record_hash(id: &str) -> String {
    sha(&["record", id])
}

pub(crate) fn not_found() -> String {
    format!("{ERR_MEMORY_NOT_FOUND}: no such memory")
}

use super::ctx::AssistCtx;
use super::db::AssistDb;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Stats {
    pub active: i64,
    pub with_vector: i64,
    pub entities: i64,
    pub notes: i64,
    pub links: i64,
    pub retains_today: i64,
    pub facts_today: i64,
    #[serde(default)]
    pub last_retain_ms: Option<i64>,
    #[serde(default)]
    pub last_retain_error: Option<String>,
    pub semantic_ready: bool,
    pub auto_retain: bool,
    pub consolidate: bool,
}

pub fn stats(db: &AssistDb, now: i64) -> Result<Stats, String> {
    let mut s = db.with(|c| {
        let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
        let day = now - 86_400_000;
        let (retains, facts): (i64, i64) = c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(facts), 0) FROM memory_retain_log WHERE at_ms >= ?1",
            [day],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let last: Option<(i64, String, Option<String>)> = {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT at_ms, status, detail FROM memory_retain_log ORDER BY at_ms DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
        };
        Ok(Stats {
            active: n("SELECT COUNT(*) FROM memory_records WHERE status = 'active'")?,
            with_vector: c.query_row(
                "SELECT COUNT(*) FROM memory_vectors v JOIN memory_records r ON r.id = v.record_id WHERE r.status = 'active' AND v.source = ?1",
                [retrieve::vector_source().unwrap_or("")],
                |r| r.get(0),
            )?,
            entities: n("SELECT COUNT(*) FROM memory_entities")?,
            notes: n("SELECT COUNT(*) FROM memory_entity_notes")?,
            links: n("SELECT COUNT(*) FROM memory_links")?,
            retains_today: retains,
            facts_today: facts,
            last_retain_ms: last.as_ref().map(|l| l.0),
            last_retain_error: last.filter(|l| l.1 == "failed").and_then(|l| l.2),
            ..Default::default()
        })
    })?;
    s.semantic_ready = retrieve::vector_source() == Some("e5");
    s.auto_retain = retain::setting(db, "auto_retain");
    s.consolidate = retain::setting(db, "consolidate");
    Ok(s)
}

pub fn entities(db: &AssistDb, ctx: &AssistCtx, limit: usize) -> Result<Vec<Entity>, String> {
    let keys = (!ctx.is_user_ui()).then(|| ctx.readable_keys());
    db.with(|c| graph::list_entities(c, &ctx.principal, keys.as_deref(), limit.clamp(1, 500)))
}

pub fn search_explained(
    db: &AssistDb,
    ctx: &AssistCtx,
    query: &str,
    limit: usize,
    now: i64,
) -> Result<Probe, String> {
    let qvec = retrieve::embed_query(query);
    retrieve::hybrid(db, ctx, query, qvec.as_deref(), limit, now)
}

pub fn upkeep(db: std::sync::Arc<AssistDb>) {
    toolset::spawn_upkeep(db);
}
