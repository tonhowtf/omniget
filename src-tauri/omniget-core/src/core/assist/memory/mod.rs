//! Personal memory: scoped records with provenance, versions, corrections,
//! tombstones, a derived FTS index, export/import. Owner: worker W1.
//!
//! The table `memory_records` is the only source of truth. Everything else
//! (`memory_fts`, `memory_profile_cache`) is derived and rebuilt from it by
//! [`reindex`]; tombstones (`memory_tombstones`) keep a forgotten record from
//! coming back through a reindex or an import, and hold hashes only, never the
//! forgotten text.
//!
//! Rules this module enforces (spec 02 "Memória que cresce com o usuário"):
//! - every query carries the caller's [`AssistCtx`] and filters by its scopes
//!   in SQL, before ranking and `LIMIT`; a record outside those scopes is
//!   indistinguishable from one that does not exist;
//! - writes are idempotent per origin (same source + same content → the same
//!   record);
//! - an inference is a candidate until the person confirms it; it never
//!   becomes a stated preference on its own, and it never displaces one;
//! - a correction supersedes the previous version (one active record per
//!   subject and kind), keeping the chain for the history screen;
//! - forgetting removes every version, the index rows and the cached profile,
//!   and leaves hashes behind.

mod records;
mod search;
mod toolset;
mod transfer;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ctx::Scope;
use super::db::Migration;

pub use records::{
    confirm, correct, expire_due, forget, get, history, list, remember, retract, scopes,
    Correction, ScopeSummary, Written,
};
pub use search::{augment_text, profile, recall, reindex, source_line, Profile};
pub use toolset::{augment, call_tool, has_recall_grant, toolset};
pub use transfer::{
    backup_files, delete_backups, export_json, export_markdown, import_json, ExportFile,
    ImportReport, Tombstone, EXPORT_FORMAT, EXPORT_VERSION,
};

pub const ERR_MEMORY_NOT_FOUND: &str = "ERR_MEMORY_NOT_FOUND";
pub const ERR_MEMORY_INVALID: &str = "ERR_MEMORY_INVALID";
pub const ERR_MEMORY_FORGOTTEN: &str = "ERR_MEMORY_FORGOTTEN";
pub const ERR_MEMORY_INACTIVE: &str = "ERR_MEMORY_INACTIVE";
pub const ERR_MEMORY_IMPORT: &str = "ERR_MEMORY_IMPORT";

/// Longest text one record may hold.
pub const MAX_CONTENT_CHARS: usize = 4000;

pub const MIGRATIONS: &[Migration] = &[Migration {
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
}];

/// Every tool this module offers; `bots` maps the `memory` capability to these.
pub const TOOL_NAMES: &[&str] = &[
    "memory_recall",
    "memory_remember",
    "memory_correct",
    "memory_forget",
    "memory_profile",
];

/// What kind of knowledge a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A fact or preference the person stated (or confirmed).
    Declared,
    /// Something that happened: a movie marked watched, a reaction to it.
    Observation,
    /// A guess with evidence and confidence; a candidate until confirmed.
    Inference,
    /// Context that holds for a while ("today I want something light").
    Temporary,
    /// A working note on how to do something for this person.
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

    /// Records of the same group and subject cannot be active together.
    /// Observations are events: many can coexist.
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
    /// Replaced by a later version of the same chain.
    Superseded,
    /// Withdrawn without a replacement ("that was not it").
    Retracted,
    /// Temporary context whose validity ran out.
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

/// Where a record came from. `author` is `"user"` or `"bot:<id>"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// `message` | `event` | `document` | `conversation` | `user` | `import`.
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub conversation: Option<String>,
    pub author: String,
    pub at_ms: i64,
}

impl Source {
    /// The person typing on the memory screen.
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

/// A record about to be written.
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

// ── Hashing (tombstones and idempotency) ──────────────────────────────────

/// Lowercase, single spaces, no punctuation at the ends: two spellings of
/// the same sentence hash the same.
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
