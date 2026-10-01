//! Companies of agents: the control plane Paperclip built on Node and
//! Postgres, rebuilt local-first on one SQLite file (study 92).
//!
//! A company has a mission (its root goal), an org chart of staff (each one a
//! roster agent with a role, a title, a manager and a heartbeat), goals,
//! projects and issues. Agents are woken by durable **wakeups** (timer,
//! assignment, mention, approval, routine, retry…); a wakeup becomes a
//! **run**, which the app executes as an ordinary job (`src-tauri/src/company.rs`).
//! Governance lives in the same file: approvals, budget policies with
//! incidents, a per-request cost ledger with rollups, and a hash-chained audit
//! log.
//!
//! Everything that changes state goes through [`CompanyStore`], so the UI,
//! the bridge, the MCP tools the agents call and the scheduler share one set
//! of rules (Paperclip validates issue transitions only in some routes).
//!
//! Layout:
//! - [`schema`]: the `company.db` migrations;
//! - [`org`]: companies, staff and the org chart, goals, projects;
//! - [`issues`]: issues, the status machine, checkout, comments, documents,
//!   relations, labels, work products, search, inbox;
//! - [`gov`]: approvals and decisions, budget policies, incidents, the cost
//!   ledger;
//! - [`audit`]: the hash-chained activity log;
//! - [`heartbeat`]: wakeups, coalescing, runs, liveness, retry;
//! - [`routines`]: recurring work that files issues on a cron;
//! - [`portable`]: export/import of a whole company as one JSON package and
//!   the built-in templates;
//! - [`context`]: the heartbeat prompt an agent receives;
//! - [`toolset`]: the `company_*` tools an agent calls through the MCP
//!   projection.

pub mod audit;
pub mod context;
pub mod exec;
pub mod gov;
pub mod heartbeat;
pub mod issues;
pub mod org;
pub mod portable;
pub mod routines;
pub mod rpc;
pub mod schema;
pub mod toolset;

use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::core::assist::db::AssistDb;

pub const ERR_COMPANY: &str = "ERR_COMPANY";
pub const ERR_COMPANY_NOT_FOUND: &str = "ERR_COMPANY_NOT_FOUND";
pub const ERR_COMPANY_INVALID: &str = "ERR_COMPANY_INVALID";
pub const ERR_COMPANY_CONFLICT: &str = "ERR_COMPANY_CONFLICT";
pub const ERR_COMPANY_FORBIDDEN: &str = "ERR_COMPANY_FORBIDDEN";
pub const ERR_COMPANY_BUDGET: &str = "ERR_COMPANY_BUDGET";
pub const DB_FILE: &str = "company.db";

pub(crate) fn invalid(msg: impl std::fmt::Display) -> String {
    format!("{ERR_COMPANY_INVALID}: {msg}")
}
pub(crate) fn not_found(what: &str, id: &str) -> String {
    format!("{ERR_COMPANY_NOT_FOUND}: {what} `{id}`")
}
pub(crate) fn conflict(msg: impl std::fmt::Display) -> String {
    format!("{ERR_COMPANY_CONFLICT}: {msg}")
}
pub(crate) fn forbidden(msg: impl std::fmt::Display) -> String {
    format!("{ERR_COMPANY_FORBIDDEN}: {msg}")
}
pub(crate) fn sql(e: rusqlite::Error) -> String {
    format!("{ERR_COMPANY}: {e}")
}

/// `Option<Option<T>>` fields of a patch: absent = keep, `null` = clear,
/// a value = set. Plain serde reads `null` as absent.
pub(crate) fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Who did something. Every mutation carries one; it lands in the audit log
/// and decides what an agent is allowed to do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// The person at the keyboard (the board).
    User { channel: String },
    /// A staff member acting from a run (MCP tool, heartbeat).
    Agent {
        staff_id: String,
        run_id: Option<String>,
    },
    /// The scheduler, budgets, recovery.
    System { what: String },
}

impl Actor {
    pub fn user() -> Self {
        Actor::User {
            channel: "app".into(),
        }
    }
    pub fn user_via(channel: &str) -> Self {
        Actor::User {
            channel: channel.into(),
        }
    }
    pub fn system(what: &str) -> Self {
        Actor::System { what: what.into() }
    }
    pub fn agent(staff_id: &str, run_id: Option<&str>) -> Self {
        Actor::Agent {
            staff_id: staff_id.into(),
            run_id: run_id.map(str::to_string),
        }
    }
    pub fn is_user(&self) -> bool {
        matches!(self, Actor::User { .. })
    }
    pub fn staff_id(&self) -> Option<&str> {
        match self {
            Actor::Agent { staff_id, .. } => Some(staff_id),
            _ => None,
        }
    }
    pub fn run_id(&self) -> Option<&str> {
        match self {
            Actor::Agent { run_id, .. } => run_id.as_deref(),
            _ => None,
        }
    }
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Actor::User { .. } => "user",
            Actor::Agent { .. } => "agent",
            Actor::System { .. } => "system",
        }
    }
    pub(crate) fn id(&self) -> String {
        match self {
            Actor::User { .. } => "owner".into(),
            Actor::Agent { staff_id, .. } => staff_id.clone(),
            Actor::System { what } => what.clone(),
        }
    }
    pub(crate) fn channel(&self) -> &str {
        match self {
            Actor::User { channel } => channel,
            Actor::Agent { .. } => "mcp",
            Actor::System { .. } => "system",
        }
    }
}

/// Something changed; the app turns these into `company://changed` events.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub company_id: String,
    pub entity: &'static str,
    pub id: String,
}

type Listener = Arc<dyn Fn(&Change) + Send + Sync>;

pub struct CompanyStore {
    db: AssistDb,
    listeners: RwLock<Vec<Listener>>,
}

impl std::fmt::Debug for CompanyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanyStore")
            .field("db", &self.db)
            .finish()
    }
}

impl CompanyStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = AssistDb::open_with(path, schema::MIGRATIONS)?;
        Ok(Self {
            db,
            listeners: RwLock::new(Vec::new()),
        })
    }

    /// Reads with the shared connection.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        self.db.with(f)
    }

    /// One transaction; any `Err` rolls it back. Changes are announced after
    /// the commit, never before.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction, &mut Vec<Change>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut changes = Vec::new();
        let out = self.db.tx(|tx| f(tx, &mut changes))?;
        if !changes.is_empty() {
            let ls = self.listeners.read().unwrap_or_else(|e| e.into_inner());
            for c in &changes {
                for l in ls.iter() {
                    l(c);
                }
            }
        }
        Ok(out)
    }

    pub fn on_change(&self, f: impl Fn(&Change) + Send + Sync + 'static) {
        self.listeners
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(f));
    }

    pub fn path(&self) -> Option<&Path> {
        self.db.path()
    }
}

pub(crate) fn changed(out: &mut Vec<Change>, company_id: &str, entity: &'static str, id: &str) {
    out.push(Change {
        company_id: company_id.to_string(),
        entity,
        id: id.to_string(),
    });
}

static STORE: OnceLock<Arc<CompanyStore>> = OnceLock::new();

/// The process-wide store at `<llm>/company.db`.
pub fn store() -> Result<Arc<CompanyStore>, String> {
    if let Some(s) = STORE.get() {
        return Ok(s.clone());
    }
    let dir = crate::core::llm::roster_store::llm_dir()
        .ok_or_else(|| format!("{ERR_COMPANY}: no data folder"))?;
    let s = Arc::new(CompanyStore::open(&dir.join(DB_FILE))?);
    Ok(STORE.get_or_init(|| s).clone())
}
