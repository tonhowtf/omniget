//! C02: limited configuration by an external controller.
//!
//! An external principal (selected from a local execution grant, never from
//! request JSON) may prepare:
//! - a derived bot: a real roster `AgentDef` + a real `bots_profiles` row,
//!   Native, model/connection inherited from a granted executor, tools =
//!   requested subset of the grant, no memory/local capabilities, no skills;
//! - a room: a real `groups_rooms` room whose members are only its own
//!   derived bots, with conservative limits bounded by the grant;
//! - delegated tasks in that room, bound to principal + key + fingerprint.
//!
//! Derived bots share the parent grant id and its debit ledger: there are no
//! child grants and so no multiplication of the ceiling. Revoking the parent
//! grant, or any change of the pinned source executor, invalidates them.
//!
//! The roster lives in a JSON file and the rest in SQLite, so bot preparation
//! is an explicit saga: a durable intent is committed before the roster is
//! touched, the roster write is a compare-and-swap (`apply_planned`), and the
//! profile/ownership rows are finalized in one later transaction. Recovery
//! compares exact hashes and never overwrites a local edit.
//!
//! Nothing here runs a turn. The legacy room dispatcher (`groups::send_user_
//! message`) is LOCAL_USER-only and must not be used for external work; the
//! mission driver runs claimed tasks through [`task_run`] / [`claim_task`] /
//! [`finish_task`] under the external authority. The local entries enforce
//! that in the domain with [`deny_local_dispatch`] / [`local_turn_allowed`]:
//! an external room or a derived bot is never dispatched as LOCAL_USER.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::authority::{self, Ceiling};
use super::bots::profile::{self, BotProfile, MemoryPolicy};
use super::db::{AssistDb, Migration};
use super::groups::store::{self as rooms, Member, MentionPolicy, RoomDraft};
use super::groups::tasks::{self, DelegateRequest, RoomLimits, RunEnd, Task};
use super::now_ms;
use crate::core::llm::agent::{AgentDef, AgentRole, GrantMode, RuntimeKind, ToolGrant, ToolSource};

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "external_config",
    version: 1,
    sql: "
CREATE TABLE external_config_intents(principal TEXT NOT NULL,kind TEXT NOT NULL,key TEXT NOT NULL,fingerprint TEXT NOT NULL,grant_id TEXT NOT NULL,target TEXT NOT NULL,plan TEXT NOT NULL,state TEXT NOT NULL,created_ms INTEGER NOT NULL,updated_ms INTEGER NOT NULL,PRIMARY KEY(principal,kind,key));
CREATE TABLE external_derived_bots(bot TEXT PRIMARY KEY,grant_id TEXT NOT NULL REFERENCES external_grants(id),principal TEXT NOT NULL,source_bot TEXT NOT NULL,source_hash TEXT NOT NULL,hash TEXT NOT NULL,tools TEXT NOT NULL,created_ms INTEGER NOT NULL);
CREATE INDEX external_derived_bots_grant ON external_derived_bots(grant_id);
CREATE TABLE external_rooms(room TEXT PRIMARY KEY REFERENCES groups_rooms(id),grant_id TEXT NOT NULL REFERENCES external_grants(id),principal TEXT NOT NULL,created_ms INTEGER NOT NULL);
CREATE TABLE external_room_tasks(task TEXT PRIMARY KEY REFERENCES groups_tasks(id),room TEXT NOT NULL,principal TEXT NOT NULL,key TEXT NOT NULL,fingerprint TEXT NOT NULL,UNIQUE(principal,key));
",
}, Migration {
    module: "external_config",
    version: 2,
    // Set when the parent grant (or the whole client) is revoked: the room is
    // archived and the derived bot disabled; both stay visible, read-only.
    sql: "
ALTER TABLE external_rooms ADD COLUMN retired_ms INTEGER;
ALTER TABLE external_derived_bots ADD COLUMN retired_ms INTEGER;
",
}];

/// Local (LOCAL_USER) entries refuse external rooms and derived bots.
pub const ERR_LOCAL_DISPATCH: &str = "EXTERNAL_ROOM_LOCAL_DISPATCH_DENIED";
/// A derived bot the person revoked from the desktop.
pub const ERR_DERIVED_REVOKED: &str = "DERIVED_BOT_REVOKED";
/// External execution needs the macOS worker sandbox.
pub const ERR_ISOLATION: &str = "EXECUTION_ISOLATION_REQUIRED";

/// Per grant: derived bots (ready or in flight) and rooms.
pub const MAX_DERIVED_PER_GRANT: i64 = 8;
pub const MAX_ROOMS_PER_GRANT: i64 = 4;
pub const MAX_ROOM_MEMBERS: usize = 6;
/// External room ceiling. Rooms never get more than this whatever is asked.
pub const EXT_MAX_DELEGATIONS: u32 = 4;
pub const EXT_MAX_TURNS: u32 = 8;
pub const EXT_MAX_TASK_MS: u64 = 900_000;
pub const EXT_UNKNOWN_RUN_TOKENS: u64 = 4_000;

// ── roster access ───────────────────────────────────────────────────────

/// The real roster, as the external layer needs it. The app installs an
/// adapter over `LlmManager`; tests use a `RosterStore`. Without one, every
/// derived operation fails closed.
pub trait BotRoster: Send + Sync {
    fn get(&self, id: &str) -> Option<AgentDef>;
    fn apply_planned(
        &self,
        agent: AgentDef,
        before: Option<AgentDef>,
        source: AgentDef,
    ) -> Result<AgentDef, String>;
}

impl BotRoster for crate::core::llm::roster_store::RosterStore {
    fn get(&self, id: &str) -> Option<AgentDef> {
        crate::core::llm::roster_store::RosterStore::get(self, id)
    }
    fn apply_planned(
        &self,
        agent: AgentDef,
        before: Option<AgentDef>,
        source: AgentDef,
    ) -> Result<AgentDef, String> {
        crate::core::llm::roster_store::RosterStore::apply_planned(self, agent, before, source)
            .map_err(|e| e.to_string())
    }
}

static ROSTER: RwLock<Option<Arc<dyn BotRoster>>> = RwLock::new(None);

pub fn install_roster(roster: Arc<dyn BotRoster>) {
    *ROSTER.write().unwrap_or_else(|e| e.into_inner()) = Some(roster);
}

pub fn installed_roster() -> Option<Arc<dyn BotRoster>> {
    ROSTER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn roster() -> Result<Arc<dyn BotRoster>, String> {
    installed_roster().ok_or_else(|| "EXTERNAL_ROSTER_UNAVAILABLE".into())
}

// ── requests (closed DTOs) ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BotRequest {
    pub grant_id: String,
    /// A granted executor; model/connection/runtime are inherited from it.
    pub source_executor_id: String,
    pub name: String,
    #[serde(default)]
    pub purpose: String,
    pub instructions: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RoomLimitsRequest {
    #[serde(default)]
    pub max_delegations: Option<u32>,
    #[serde(default)]
    pub max_task_seconds: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RoomRequest {
    pub grant_id: String,
    pub title: String,
    pub member_bot_ids: Vec<String>,
    /// Creator of every external task in the room; one of the members.
    pub coordinator_id: String,
    #[serde(default)]
    pub limits: Option<RoomLimitsRequest>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TaskRequest {
    pub room_id: String,
    /// Exact bot id of a member (not the coordinator).
    pub to: String,
    pub question: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub deliverable: String,
    #[serde(default)]
    pub limit_seconds: Option<u64>,
    pub idempotency_key: String,
}

// ── public views (never the private AgentDef) ───────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedBotView {
    pub bot_id: String,
    pub grant_id: String,
    pub source_executor_id: String,
    pub tools: Vec<String>,
    /// `ready` or `invalid` (revoked grant, changed source or local edit).
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    pub room_id: String,
    pub grant_id: String,
    pub title: String,
    pub member_bot_ids: Vec<String>,
    pub coordinator_id: Option<String>,
    pub limits: RoomLimits,
    pub grant_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub task_id: String,
    pub room_id: String,
    pub recipient: String,
    pub state: String,
    pub limit_ms: i64,
    pub created_ms: i64,
    pub finished_ms: Option<i64>,
    pub result: Option<String>,
    pub error: Option<String>,
}

impl From<&Task> for TaskView {
    fn from(t: &Task) -> Self {
        Self {
            task_id: t.id.clone(),
            room_id: t.room.clone(),
            recipient: t.recipient.clone(),
            state: t.state.clone(),
            limit_ms: t.limit_ms,
            created_ms: t.created_ms,
            finished_ms: t.finished_ms,
            result: t.result.clone(),
            error: t.error.clone(),
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────────

fn sha(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p.as_bytes());
    }
    format!("{:x}", h.finalize())
}

fn fingerprint<T: Serialize>(kind: &str, value: &T) -> Result<String, String> {
    let body = serde_json::to_string(value).map_err(|_| "INVALID_ARGUMENTS")?;
    Ok(sha(&[kind, &body]))
}

fn check_id(v: &str) -> Result<(), String> {
    if v.is_empty()
        || v.len() > 128
        || !v
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("INVALID_ARGUMENTS".into());
    }
    Ok(())
}

fn check_key(v: &str) -> Result<(), String> {
    if v.len() > 100 {
        return Err("INVALID_IDEMPOTENCY_KEY".into());
    }
    check_id(v).map_err(|_| "INVALID_IDEMPOTENCY_KEY".into())
}

fn check_text(v: &str, min: usize, max: usize) -> Result<(), String> {
    let n = v.trim().chars().count();
    if n < min || v.len() > max || v.contains('\0') {
        return Err("INVALID_ARGUMENTS".into());
    }
    Ok(())
}

fn storage<E: std::fmt::Display>(_: E) -> String {
    "EXTERNAL_CONFIG_STORAGE".into()
}

/// Live, unrevoked grant body of this principal, read inside `c`.
fn live_grant(c: &Connection, grant_id: &str, principal: &str) -> Result<Ceiling, String> {
    let body: Option<String> = c
        .query_row(
            "SELECT body FROM external_grants WHERE id=?1 AND principal=?2 AND revoked=0",
            params![grant_id, principal],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    serde_json::from_str(&body.ok_or("EXECUTION_NOT_GRANTED")?)
        .map_err(|_| "INVALID_EXECUTION_GRANT".into())
}

struct DerivedRow {
    grant_id: String,
    principal: String,
    source_bot: String,
    source_hash: String,
    hash: String,
    tools: Vec<String>,
    /// Revoked by the person (or with its grant): never usable again.
    retired: bool,
}

fn derived_row(c: &Connection, bot: &str) -> Result<Option<DerivedRow>, String> {
    c.query_row(
        "SELECT grant_id,principal,source_bot,source_hash,hash,tools,retired_ms IS NOT NULL FROM external_derived_bots WHERE bot=?1",
        [bot],
        |r| {
            let tools: String = r.get(5)?;
            Ok(DerivedRow {
                grant_id: r.get(0)?,
                principal: r.get(1)?,
                source_bot: r.get(2)?,
                source_hash: r.get(3)?,
                hash: r.get(4)?,
                tools: serde_json::from_str(&tools).unwrap_or_default(),
                retired: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(storage)
}

/// The effective authority of a derived bot under `grant_id`: the live parent
/// grant restricted to this bot, its pinned revision and its tool subset.
/// Fails when the grant is revoked, the principal differs, the source executor
/// left the grant or changed (grant pin or roster), the derived agent was
/// edited locally, or no roster is available.
pub fn derived_ceiling(
    c: &Connection,
    grant_id: &str,
    principal: &str,
    bot: &str,
    roster: Option<&dyn BotRoster>,
) -> Result<Ceiling, String> {
    let row = derived_row(c, bot)?.ok_or("EXECUTOR_NOT_GRANTED")?;
    if row.grant_id != grant_id || row.principal != principal {
        return Err("EXECUTOR_NOT_GRANTED".into());
    }
    let grant = live_grant(c, &row.grant_id, &row.principal)?;
    if row.retired {
        return Err(ERR_DERIVED_REVOKED.into());
    }
    if !grant.bots.iter().any(|b| b == &row.source_bot)
        || grant.bot_revisions.get(&row.source_bot) != Some(&row.source_hash)
    {
        return Err("DERIVED_SOURCE_CHANGED".into());
    }
    let roster = roster.ok_or("EXTERNAL_ROSTER_UNAVAILABLE")?;
    let source = roster
        .get(&row.source_bot)
        .ok_or("DERIVED_SOURCE_CHANGED")?;
    if authority::agent_revision(&source)? != row.source_hash {
        return Err("DERIVED_SOURCE_CHANGED".into());
    }
    let agent = roster.get(bot).ok_or("EXECUTOR_UNAVAILABLE")?;
    if authority::agent_revision(&agent)? != row.hash {
        return Err("EXECUTOR_REVISION_CHANGED".into());
    }
    if !matches!(agent.runtime, RuntimeKind::Native) {
        return Err("EXTERNAL_RUNTIME_ISOLATION_REQUIRED".into());
    }
    let tools = row
        .tools
        .into_iter()
        .filter(|t| grant.tools.contains(t) && authority::supported_tool(t))
        .collect();
    Ok(Ceiling {
        bots: vec![bot.to_string()],
        bot_revisions: BTreeMap::from([(bot.to_string(), row.hash)]),
        tools,
        ..grant
    })
}

/// A member bot must be a live derived bot of this principal and grant.
fn owned_bot(
    c: &Connection,
    grant: &Ceiling,
    bot: &str,
    roster: &dyn BotRoster,
) -> Result<Ceiling, String> {
    derived_ceiling(c, &grant.id, &grant.principal, bot, Some(roster)).map_err(|e| {
        if e == "EXECUTOR_NOT_GRANTED" {
            "ROOM_MEMBER_NOT_OWNED".into()
        } else {
            e
        }
    })
}

// ── derived bots ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BotPlan {
    agent: AgentDef,
    profile: BotProfile,
    source: String,
    source_hash: String,
    hash: String,
    tools: Vec<String>,
}

enum Stage {
    Ready(String),
    Apply(BotPlan),
}

/// Validates against the live grant and the current source; returns the plan.
fn plan_bot(
    grant: &Ceiling,
    roster: &dyn BotRoster,
    principal: &str,
    req: &BotRequest,
) -> Result<BotPlan, String> {
    if !grant.bots.iter().any(|b| b == &req.source_executor_id) {
        return Err("EXECUTOR_NOT_GRANTED".into());
    }
    let pinned = grant
        .bot_revisions
        .get(&req.source_executor_id)
        .ok_or("EXECUTOR_REVISION_REQUIRED")?;
    let source = roster
        .get(&req.source_executor_id)
        .ok_or("EXECUTOR_UNAVAILABLE")?;
    if &authority::agent_revision(&source)? != pinned {
        return Err("EXECUTOR_REVISION_CHANGED".into());
    }
    if !matches!(source.runtime, RuntimeKind::Native) {
        return Err("EXTERNAL_RUNTIME_ISOLATION_REQUIRED".into());
    }
    let mut tools = BTreeSet::new();
    for t in &req.tools {
        if !grant.tools.contains(t) {
            return Err("EXTERNAL_TOOL_NOT_GRANTED".into());
        }
        if !authority::supported_tool(t) {
            return Err("EXTERNAL_EXECUTOR_ISOLATION_REQUIRED".into());
        }
        tools.insert(t.clone());
    }
    let tools: Vec<String> = tools.into_iter().collect();
    let id = format!(
        "ext-{}",
        &sha(&[principal, &grant.id, &req.idempotency_key, "bot"])[..24]
    );
    let mut agent = source.clone();
    agent.id = id.clone();
    agent.name = req.name.trim().to_string();
    agent.role = AgentRole::Worker;
    agent.system_prompt = req.instructions.trim().to_string();
    agent.skills = Vec::new();
    agent.skin = None;
    agent.tools = tools
        .iter()
        .map(|name| ToolGrant {
            mode: source
                .tools
                .iter()
                .find(|g| matches!(&g.source, ToolSource::Internal { name: n } if n == name))
                .map(|g| g.mode)
                // Absent → Ask; an owner Deny stays Deny (never widened).
                .unwrap_or(GrantMode::Ask),
            source: ToolSource::Internal { name: name.clone() },
        })
        .collect();
    let hash = authority::agent_revision(&agent)?;
    let profile = BotProfile {
        bot_id: id,
        purpose: req.purpose.trim().to_string(),
        instructions: req.instructions.trim().to_string(),
        capabilities: Vec::new(),
        memory_policy: MemoryPolicy::ReadOnly,
        default_connection: Some(source.id.clone()),
        created_at: 0,
        updated_at: 0,
    };
    Ok(BotPlan {
        agent,
        profile,
        source: source.id,
        source_hash: pinned.clone(),
        hash,
        tools,
    })
}

fn validate_bot_request(req: &BotRequest) -> Result<(), String> {
    check_id(&req.grant_id)?;
    check_id(&req.source_executor_id)?;
    check_key(&req.idempotency_key)?;
    check_text(&req.name, 1, 80)?;
    check_text(&req.purpose, 0, 2_000)?;
    check_text(&req.instructions, 1, 8_192)?;
    if req.tools.len() > 32 || req.tools.iter().any(|t| check_id(t).is_err()) {
        return Err("INVALID_ARGUMENTS".into());
    }
    Ok(())
}

fn bot_view(
    db: &AssistDb,
    principal: &str,
    bot: &str,
    roster: Option<&dyn BotRoster>,
) -> Result<DerivedBotView, String> {
    let (row, validity) = db.with(|c| {
        let row = derived_row(c, bot);
        let validity = row
            .as_ref()
            .ok()
            .and_then(|r| r.as_ref())
            .map(|r| derived_ceiling(c, &r.grant_id, principal, bot, roster).map(|_| ()));
        Ok((row, validity))
    })?;
    let row = row?.ok_or("EXECUTOR_NOT_GRANTED")?;
    if row.principal != principal {
        return Err("EXECUTOR_NOT_GRANTED".into());
    }
    let err = validity.and_then(|v| v.err());
    Ok(DerivedBotView {
        bot_id: bot.to_string(),
        grant_id: row.grant_id,
        source_executor_id: row.source_bot,
        tools: row.tools,
        state: if err.is_none() { "ready" } else { "invalid" }.into(),
        reason: err,
    })
}

/// Prepares (or replays) a derived bot with the installed roster.
pub fn prepare_bot(
    db: &AssistDb,
    principal: &str,
    req: &BotRequest,
) -> Result<DerivedBotView, String> {
    prepare_bot_with(db, roster()?.as_ref(), principal, req)
}

pub fn prepare_bot_with(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    req: &BotRequest,
) -> Result<DerivedBotView, String> {
    if principal.is_empty() {
        return Err("PRINCIPAL_REQUIRED".into());
    }
    validate_bot_request(req)?;
    let fp = fingerprint("bot", req)?;
    // Phase 1: durable intent before any roster write.
    let stage = db.tx(|tx| {
        let old: Option<(String, String, String, String)> = tx
            .query_row(
                "SELECT fingerprint,state,target,plan FROM external_config_intents WHERE principal=?1 AND kind='bot' AND key=?2",
                params![principal, req.idempotency_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(storage)?;
        if let Some((old_fp, state, target, plan)) = old {
            if old_fp != fp {
                return Err("IDEMPOTENCY_CONFLICT".into());
            }
            return match state.as_str() {
                "ready" => Ok(Stage::Ready(target)),
                "pending" => Ok(Stage::Apply(
                    serde_json::from_str(&plan).map_err(|_| "EXTERNAL_CONFIG_STORAGE")?,
                )),
                _ => Err("EXTERNAL_BOT_CHANGED_LOCALLY".into()),
            };
        }
        let grant = live_grant(tx, &req.grant_id, principal)?;
        let plan = plan_bot(&grant, roster, principal, req)?;
        let used: i64 = tx
            .query_row(
                "SELECT (SELECT count(*) FROM external_derived_bots WHERE grant_id=?1)+(SELECT count(*) FROM external_config_intents WHERE grant_id=?1 AND kind='bot' AND state='pending')",
                [&grant.id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if used >= MAX_DERIVED_PER_GRANT {
            return Err("EXTERNAL_DERIVED_LIMIT".into());
        }
        let now = now_ms();
        tx.execute(
            "INSERT INTO external_config_intents(principal,kind,key,fingerprint,grant_id,target,plan,state,created_ms,updated_ms) VALUES(?1,'bot',?2,?3,?4,?5,?6,'pending',?7,?7)",
            params![
                principal,
                req.idempotency_key,
                fp,
                grant.id,
                plan.agent.id,
                serde_json::to_string(&plan).map_err(|_| "EXTERNAL_CONFIG_STORAGE")?,
                now
            ],
        )
        .map_err(storage)?;
        Ok(Stage::Apply(plan))
    })?;
    let plan = match stage {
        Stage::Ready(bot) => return ready_view(db, principal, &bot, roster),
        Stage::Apply(plan) => plan,
    };
    // Phase 2: roster compare-and-swap. The source must still be the pinned
    // revision; an existing agent under this id must be exactly the plan.
    let existing = roster.get(&plan.agent.id);
    match existing {
        Some(a) if authority::agent_revision(&a)? == plan.hash => {}
        Some(_) => {
            mark_conflict(db, principal, &req.idempotency_key)?;
            return Err("EXTERNAL_BOT_CHANGED_LOCALLY".into());
        }
        None => {
            let source = roster.get(&plan.source).ok_or("EXECUTOR_UNAVAILABLE")?;
            if authority::agent_revision(&source)? != plan.source_hash {
                return Err("EXECUTOR_REVISION_CHANGED".into());
            }
            if let Err(e) = roster.apply_planned(plan.agent.clone(), None, source) {
                if roster.get(&plan.agent.id).is_some_and(|a| {
                    authority::agent_revision(&a).ok().as_deref() != Some(plan.hash.as_str())
                }) {
                    mark_conflict(db, principal, &req.idempotency_key)?;
                    return Err("EXTERNAL_BOT_CHANGED_LOCALLY".into());
                }
                tracing::warn!("[external_config] roster write: {e}");
                return Err("EXTERNAL_ROSTER_WRITE_FAILED".into());
            }
        }
    }
    // Phase 3: profile + ownership + intent in one transaction.
    db.tx(|tx| {
        let state: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint,state FROM external_config_intents WHERE principal=?1 AND kind='bot' AND key=?2",
                params![principal, req.idempotency_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        match state {
            Some((f, s)) if f == fp && s == "ready" => return Ok(()),
            Some((f, s)) if f == fp && s == "pending" => {}
            _ => return Err("EXTERNAL_CONFIG_STATE_CHANGED".into()),
        }
        let grant = live_grant(tx, &req.grant_id, principal)?;
        if grant.bot_revisions.get(&plan.source) != Some(&plan.source_hash) {
            return Err("DERIVED_SOURCE_CHANGED".into());
        }
        profile::create_scoped_tx(tx, &plan.profile)?;
        tx.execute(
            "INSERT OR IGNORE INTO external_derived_bots(bot,grant_id,principal,source_bot,source_hash,hash,tools,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                plan.agent.id,
                grant.id,
                principal,
                plan.source,
                plan.source_hash,
                plan.hash,
                serde_json::to_string(&plan.tools).map_err(|_| "EXTERNAL_CONFIG_STORAGE")?,
                now_ms()
            ],
        )
        .map_err(storage)?;
        let row = derived_row(tx, &plan.agent.id)?.ok_or("EXTERNAL_CONFIG_STORAGE")?;
        if row.grant_id != grant.id || row.principal != principal || row.hash != plan.hash {
            return Err("EXTERNAL_BOT_CHANGED_LOCALLY".into());
        }
        tx.execute(
            "UPDATE external_config_intents SET state='ready',updated_ms=?3 WHERE principal=?1 AND kind='bot' AND key=?2",
            params![principal, req.idempotency_key, now_ms()],
        )
        .map_err(storage)?;
        Ok(())
    })?;
    ready_view(db, principal, &plan.agent.id, roster)
}

fn ready_view(
    db: &AssistDb,
    principal: &str,
    bot: &str,
    roster: &dyn BotRoster,
) -> Result<DerivedBotView, String> {
    let v = bot_view(db, principal, bot, Some(roster))?;
    match v.reason {
        Some(reason) => Err(reason),
        None => Ok(v),
    }
}

fn mark_conflict(db: &AssistDb, principal: &str, key: &str) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            "UPDATE external_config_intents SET state='conflict',updated_ms=?3 WHERE principal=?1 AND kind='bot' AND key=?2 AND state='pending'",
            params![principal, key, now_ms()],
        )
    })
    .map(|_| ())
}

/// Derived bots of this principal, with their current validity.
pub fn list_bots(db: &AssistDb, principal: &str) -> Result<Vec<DerivedBotView>, String> {
    list_bots_with(db, installed_roster().as_deref(), principal)
}

pub fn list_bots_with(
    db: &AssistDb,
    roster: Option<&dyn BotRoster>,
    principal: &str,
) -> Result<Vec<DerivedBotView>, String> {
    let ids: Vec<String> = db.with(|c| {
        let mut s = c.prepare(
            "SELECT bot FROM external_derived_bots WHERE principal=?1 ORDER BY bot LIMIT 200",
        )?;
        let rows = s.query_map([principal], |r| r.get(0))?;
        rows.collect()
    })?;
    ids.iter()
        .map(|b| bot_view(db, principal, b, roster))
        .collect()
}

// ── rooms ───────────────────────────────────────────────────────────────

/// Conservative limits for an external room, bounded by the grant.
pub fn external_limits(
    req: Option<&RoomLimitsRequest>,
    grant: &Ceiling,
) -> Result<RoomLimits, String> {
    let r = req.cloned().unwrap_or_default();
    let delegations = r.max_delegations.unwrap_or(EXT_MAX_DELEGATIONS);
    let task_ms = r
        .max_task_seconds
        .map(|s| s.saturating_mul(1000))
        .unwrap_or(EXT_MAX_TASK_MS);
    let tokens = r.max_tokens.unwrap_or(grant.max_tokens);
    if delegations == 0 || delegations > EXT_MAX_DELEGATIONS {
        return Err("EXTERNAL_LIMIT_EXCEEDED".into());
    }
    if !(5_000..=EXT_MAX_TASK_MS).contains(&task_ms) {
        return Err("EXTERNAL_LIMIT_EXCEEDED".into());
    }
    if tokens == 0 || tokens > grant.max_tokens {
        return Err("EXTERNAL_BUDGET_EXCEEDED".into());
    }
    let limits = RoomLimits {
        max_depth: 1,
        max_delegations_per_round: delegations,
        max_turns_per_round: EXT_MAX_TURNS,
        max_concurrency: 1,
        max_task_ms: task_ms,
        max_tokens_per_round: Some(tokens),
        unknown_run_tokens: EXT_UNKNOWN_RUN_TOKENS.min(tokens),
    };
    limits.validate()?;
    Ok(limits)
}

/// A room edited locally past the external ceiling is not usable externally.
fn limits_within(l: &RoomLimits, grant: &Ceiling) -> bool {
    l.max_depth <= 1
        && l.max_delegations_per_round <= EXT_MAX_DELEGATIONS
        && l.max_turns_per_round <= EXT_MAX_TURNS
        && l.max_concurrency <= 1
        && l.max_task_ms <= EXT_MAX_TASK_MS
        && l.max_tokens_per_round
            .is_some_and(|t| t <= grant.max_tokens)
}

fn validate_room_request(req: &RoomRequest) -> Result<(), String> {
    check_id(&req.grant_id)?;
    check_key(&req.idempotency_key)?;
    check_text(&req.title, 1, 120)?;
    check_id(&req.coordinator_id)?;
    if req.member_bot_ids.is_empty() || req.member_bot_ids.len() > MAX_ROOM_MEMBERS {
        return Err("INVALID_ARGUMENTS".into());
    }
    let mut seen = BTreeSet::new();
    for b in &req.member_bot_ids {
        check_id(b)?;
        if !seen.insert(b) {
            return Err("INVALID_ARGUMENTS".into());
        }
    }
    if !seen.contains(&req.coordinator_id) {
        return Err("INVALID_ARGUMENTS".into());
    }
    Ok(())
}

pub fn prepare_room(db: &AssistDb, principal: &str, req: &RoomRequest) -> Result<RoomView, String> {
    prepare_room_with(db, roster()?.as_ref(), principal, req)
}

pub fn prepare_room_with(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    req: &RoomRequest,
) -> Result<RoomView, String> {
    if principal.is_empty() {
        return Err("PRINCIPAL_REQUIRED".into());
    }
    validate_room_request(req)?;
    let fp = fingerprint("room", req)?;
    let room_id = format!(
        "{}{}",
        super::groups::ROOM_PREFIX,
        &sha(&[principal, &req.grant_id, &req.idempotency_key, "room"])[..12]
    );
    let target = db.tx(|tx| {
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint,target FROM external_config_intents WHERE principal=?1 AND kind='room' AND key=?2",
                params![principal, req.idempotency_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        if let Some((old_fp, target)) = old {
            if old_fp != fp {
                return Err("IDEMPOTENCY_CONFLICT".into());
            }
            return Ok(target);
        }
        let grant = live_grant(tx, &req.grant_id, principal)?;
        for b in &req.member_bot_ids {
            owned_bot(tx, &grant, b, roster)?;
        }
        let limits = external_limits(req.limits.as_ref(), &grant)?;
        let rooms_used: i64 = tx
            .query_row("SELECT count(*) FROM external_rooms WHERE grant_id=?1", [&grant.id], |r| r.get(0))
            .map_err(storage)?;
        if rooms_used >= MAX_ROOMS_PER_GRANT {
            return Err("EXTERNAL_ROOM_LIMIT".into());
        }
        let draft = RoomDraft {
            title: req.title.trim().to_string(),
            members: req
                .member_bot_ids
                .iter()
                .map(|b| Member { bot: b.clone(), role: String::new() })
                .collect(),
            coordinator: Some(req.coordinator_id.clone()),
            default_bot: Some(req.coordinator_id.clone()),
            mention_policy: MentionPolicy::MentionsOnly,
            limits: Some(limits),
        };
        rooms::create_room_tx(tx, &room_id, &draft)?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO external_rooms(room,grant_id,principal,created_ms) VALUES(?1,?2,?3,?4)",
            params![room_id, grant.id, principal, now],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO external_config_intents(principal,kind,key,fingerprint,grant_id,target,plan,state,created_ms,updated_ms) VALUES(?1,'room',?2,?3,?4,?5,?6,'ready',?7,?7)",
            params![
                principal,
                req.idempotency_key,
                fp,
                grant.id,
                room_id,
                serde_json::to_string(&draft).map_err(|_| "EXTERNAL_CONFIG_STORAGE")?,
                now
            ],
        )
        .map_err(storage)?;
        Ok(room_id.clone())
    })?;
    room_view(db, principal, &target)
}

fn room_owner(db: &AssistDb, principal: &str, room: &str) -> Result<String, String> {
    check_id(room)?;
    let grant: Option<String> = db.with(|c| {
        c.query_row(
            "SELECT grant_id FROM external_rooms WHERE room=?1 AND principal=?2",
            params![room, principal],
            |r| r.get(0),
        )
        .optional()
    })?;
    grant.ok_or_else(|| "ROOM_NOT_AUTHORIZED".into())
}

/// Owned room metadata. History stays readable after revocation.
pub fn room_view(db: &AssistDb, principal: &str, room_id: &str) -> Result<RoomView, String> {
    let grant_id = room_owner(db, principal, room_id)?;
    let room = rooms::get_room(db, room_id).map_err(|_| "ROOM_REMOVED_LOCALLY")?;
    let grant_active = db.with(|c| Ok(live_grant(c, &grant_id, principal).is_ok()))?;
    Ok(RoomView {
        room_id: room.id,
        grant_id,
        title: room.title,
        member_bot_ids: room.members.into_iter().map(|m| m.bot).collect(),
        coordinator_id: room.coordinator,
        limits: room.limits,
        grant_active,
    })
}

/// External tasks of an owned room (never local tasks of the same room).
pub fn room_tasks(db: &AssistDb, principal: &str, room_id: &str) -> Result<Vec<TaskView>, String> {
    room_owner(db, principal, room_id)?;
    let ids: Vec<String> = db.with(|c| {
        let mut s = c.prepare(
            "SELECT e.task FROM external_room_tasks e JOIN groups_tasks t ON t.id=e.task WHERE e.room=?1 AND e.principal=?2 ORDER BY t.created_ms LIMIT 50",
        )?;
        let rows = s.query_map(params![room_id, principal], |r| r.get(0))?;
        rows.collect()
    })?;
    ids.iter()
        .map(|id| tasks::task(db, id).map(|t| TaskView::from(&t)))
        .collect()
}

// ── delegated tasks ─────────────────────────────────────────────────────

/// Everything a task needs to be admitted or run now: the live grant, the
/// room within its external ceiling, and live owned coordinator/recipient.
struct RoomAuthority {
    grant: Ceiling,
    room: rooms::Room,
    coordinator: String,
}

fn room_authority(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    room_id: &str,
) -> Result<RoomAuthority, String> {
    let grant_id = room_owner(db, principal, room_id)?;
    let room = rooms::get_room(db, room_id).map_err(|_| "ROOM_REMOVED_LOCALLY")?;
    let grant = db.with(|c| Ok(live_grant(c, &grant_id, principal)))??;
    if !limits_within(&room.limits, &grant) {
        return Err("EXTERNAL_ROOM_LIMITS_CHANGED".into());
    }
    let coordinator = room
        .coordinator
        .clone()
        .ok_or("ROOM_COORDINATOR_REQUIRED")?;
    for m in &room.members {
        db.with(|c| Ok(owned_bot(c, &grant, &m.bot, roster)))??;
    }
    Ok(RoomAuthority {
        grant,
        room,
        coordinator,
    })
}

fn validate_task_request(req: &TaskRequest) -> Result<(), String> {
    check_id(&req.room_id)?;
    check_id(&req.to)?;
    check_key(&req.idempotency_key)?;
    check_text(&req.question, 1, 4_096)?;
    check_text(&req.context, 0, 8_192)?;
    check_text(&req.deliverable, 0, 2_000)?;
    if req
        .limit_seconds
        .is_some_and(|s| s == 0 || s > EXT_MAX_TASK_MS / 1000)
    {
        return Err("EXTERNAL_LIMIT_EXCEEDED".into());
    }
    Ok(())
}

pub fn create_task(db: &AssistDb, principal: &str, req: &TaskRequest) -> Result<TaskView, String> {
    create_task_with(db, roster()?.as_ref(), principal, req)
}

pub fn create_task_with(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    req: &TaskRequest,
) -> Result<TaskView, String> {
    if principal.is_empty() {
        return Err("PRINCIPAL_REQUIRED".into());
    }
    validate_task_request(req)?;
    let fp = fingerprint("task", req)?;
    let auth = room_authority(db, roster, principal, &req.room_id)?;
    if req.to == auth.coordinator {
        return Err("EXTERNAL_TASK_RECIPIENT_INVALID".into());
    }
    if !auth.room.members.iter().any(|m| m.bot == req.to) {
        return Err("ROOM_MEMBER_NOT_OWNED".into());
    }
    let delegate = DelegateRequest {
        to: req.to.clone(),
        question: req.question.clone(),
        context: req.context.clone(),
        scopes: Vec::new(),
        deliverable: req.deliverable.clone(),
        limit_s: req.limit_seconds,
    };
    let origin = format!("external-mcp:{principal}:{}", req.idempotency_key);
    let room_id = req.room_id.clone();
    let recipient = req.to.clone();
    let grant_id = auth.grant.id.clone();
    let guard = |tx: &rusqlite::Transaction, t: &Task| -> Result<(), String> {
        let row: Option<(String, String, String)> = tx
            .query_row(
                "SELECT principal,key,fingerprint FROM external_room_tasks WHERE task=?1",
                [&t.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage)?;
        match row {
            Some((p, k, f)) => {
                if p != principal || k != req.idempotency_key || f != fp {
                    return Err("IDEMPOTENCY_CONFLICT".into());
                }
            }
            None => {
                if t.room != room_id
                    || t.recipient != recipient
                    || t.creator_run.is_some()
                    || t.state != "pending"
                {
                    return Err("IDEMPOTENCY_CONFLICT".into());
                }
                // The grant must still be live in the admitting transaction.
                live_grant(tx, &grant_id, principal)?;
                tx.execute(
                    "INSERT INTO external_room_tasks(task,room,principal,key,fingerprint) VALUES(?1,?2,?3,?4,?5)",
                    params![t.id, t.room, principal, req.idempotency_key, fp],
                )
                .map_err(|_| "IDEMPOTENCY_CONFLICT")?;
            }
        }
        Ok(())
    };
    let task = tasks::admit_delegation_guarded(
        db,
        &req.room_id,
        &auth.coordinator,
        None,
        Some(&origin),
        &delegate,
        Some(&guard),
    )?;
    Ok(TaskView::from(&task))
}

// ── driver bridge ───────────────────────────────────────────────────────

/// What the mission driver needs to run one external room task: the task,
/// the recipient's effective (derived) authority under the parent grant and
/// the input text. Revalidated on every call.
#[derive(Debug, Clone)]
pub struct ExternalTaskRun {
    pub task: Task,
    pub principal: String,
    pub grant_id: String,
    pub bot: String,
    pub authority: Ceiling,
    pub input: String,
}

fn task_owner(db: &AssistDb, principal: &str, task_id: &str) -> Result<Task, String> {
    check_id(task_id)?;
    let yes: bool = db.with(|c| {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM external_room_tasks e JOIN external_rooms r ON r.room=e.room WHERE e.task=?1 AND e.principal=?2 AND r.principal=?2)",
            params![task_id, principal],
            |r| r.get(0),
        )
    })?;
    if !yes {
        return Err("TASK_NOT_AUTHORIZED".into());
    }
    tasks::task(db, task_id)
}

pub fn task_run(db: &AssistDb, principal: &str, task_id: &str) -> Result<ExternalTaskRun, String> {
    task_run_with(db, roster()?.as_ref(), principal, task_id)
}

pub fn task_run_with(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    task_id: &str,
) -> Result<ExternalTaskRun, String> {
    let task = task_owner(db, principal, task_id)?;
    let auth = room_authority(db, roster, principal, &task.room)?;
    if !auth.room.members.iter().any(|m| m.bot == task.recipient)
        || task.creator_bot != auth.coordinator
    {
        return Err("ROOM_MEMBER_NOT_OWNED".into());
    }
    let authority = db.with(|c| {
        Ok(derived_ceiling(
            c,
            &auth.grant.id,
            principal,
            &task.recipient,
            Some(roster),
        ))
    })??;
    Ok(ExternalTaskRun {
        input: tasks::task_input(&task),
        principal: principal.to_string(),
        grant_id: auth.grant.id,
        bot: task.recipient.clone(),
        authority,
        task,
    })
}

/// Revalidates, then claims the task for `run_id` (single transactional claim).
pub fn claim_task(
    db: &AssistDb,
    principal: &str,
    task_id: &str,
    run_id: &str,
) -> Result<ExternalTaskRun, String> {
    claim_task_with(db, roster()?.as_ref(), principal, task_id, run_id)
}

pub fn claim_task_with(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    task_id: &str,
    run_id: &str,
) -> Result<ExternalTaskRun, String> {
    claim_task_gated(
        db,
        roster,
        principal,
        task_id,
        run_id,
        execution_isolation_available(),
    )
}

/// The platform predicate of external execution (the worker sandbox exists
/// on macOS only). Same predicate as the MCP `missions_create` gate.
pub fn execution_isolation_available() -> bool {
    cfg!(target_os = "macos")
}

/// [`claim_task_with`] with the platform predicate injected (tests). Without
/// isolation the owned pending task fails closed, with a stable error, so it
/// neither runs nor stays pending forever in front of the bridge.
pub(crate) fn claim_task_gated(
    db: &AssistDb,
    roster: &dyn BotRoster,
    principal: &str,
    task_id: &str,
    run_id: &str,
    isolation: bool,
) -> Result<ExternalTaskRun, String> {
    check_id(run_id)?;
    if !isolation {
        task_owner(db, principal, task_id)?;
        db.with(|c| {
            c.execute(
                "UPDATE groups_tasks SET state='failed',error=?2,finished_ms=?3 WHERE id=?1 AND state='pending'",
                params![task_id, ERR_ISOLATION, now_ms()],
            )
        })?;
        return Err(ERR_ISOLATION.into());
    }
    let mut run = task_run_with(db, roster, principal, task_id)?;
    run.task = tasks::claim(db, task_id, run_id)?;
    Ok(run)
}

/// Records the end of an owned, claimed task run.
pub fn finish_task(
    db: &AssistDb,
    principal: &str,
    task_id: &str,
    run_id: &str,
    end: RunEnd,
    text: &str,
    error: Option<&str>,
    tokens: Option<i64>,
) -> Result<TaskView, String> {
    let task = task_owner(db, principal, task_id)?;
    if task.claim_run.as_deref() != Some(run_id) {
        return Err("TASK_NOT_CLAIMED_BY_RUN".into());
    }
    tasks::finish_run(db, run_id, end, text, error, tokens)?;
    tasks::task(db, task_id).map(|t| TaskView::from(&t))
}

/// Pending external tasks for the driver (principal, task), fair by
/// principal: at most ONE task per principal (its oldest), principals ordered
/// by the age of that head task. A driver sweep therefore runs one task of
/// each principal before a second one of any, so a principal with many tasks
/// delays another by at most one task per sweep. Tasks of revoked grants are
/// never returned.
pub fn pending_tasks(db: &AssistDb, limit: usize) -> Result<Vec<(String, String)>, String> {
    Ok(pending_heads(db, limit)?
        .into_iter()
        .map(|(p, t, _)| (p, t))
        .collect())
}

fn pending_heads(db: &AssistDb, limit: usize) -> Result<Vec<(String, String, i64)>, String> {
    db.with(|c| {
        let mut s = c.prepare(
            "SELECT principal,task,created FROM (
               SELECT e.principal AS principal,e.task AS task,t.created_ms AS created,
                      ROW_NUMBER() OVER (PARTITION BY e.principal ORDER BY t.created_ms,e.task) AS n
               FROM external_room_tasks e
               JOIN groups_tasks t ON t.id=e.task
               JOIN external_rooms r ON r.room=e.room AND r.principal=e.principal
               JOIN external_grants g ON g.id=r.grant_id
               WHERE t.state='pending' AND g.revoked=0 AND r.retired_ms IS NULL)
             WHERE n=1 ORDER BY created,task LIMIT ?1",
        )?;
        let rows = s.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    })
}

/// Round-robin by principal: the head task of the first principal after
/// `after` (by id, wrapping). A driver that keeps the last principal it
/// served never serves the same principal twice while another one waits.
pub fn next_pending_task(
    db: &AssistDb,
    after: Option<&str>,
) -> Result<Option<(String, String)>, String> {
    let mut heads = pending_heads(db, 1_000)?;
    heads.sort_by(|a, b| a.0.cmp(&b.0));
    let pick = after
        .and_then(|a| heads.iter().find(|h| h.0.as_str() > a))
        .or_else(|| heads.first());
    Ok(pick.map(|(p, t, _)| (p.clone(), t.clone())))
}

// ── local isolation (H1) ────────────────────────────────────────────────

fn tolerant<T: Default>(r: rusqlite::Result<T>) -> Result<T, String> {
    match r {
        Ok(v) => Ok(v),
        // A database opened without the external tables has no external
        // rooms or derived bots (module tests open partial schemas).
        Err(e) if e.to_string().contains("no such table") => Ok(T::default()),
        Err(e) => Err(storage(e)),
    }
}

/// Whether `room` is an external room (live, revoked or retired).
pub fn is_external_room(c: &Connection, room: &str) -> Result<bool, String> {
    tolerant(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM external_rooms WHERE room=?1)",
        [room],
        |r| r.get(0),
    ))
}

/// Whether `bot` is a derived bot of some external principal.
pub fn is_derived_bot(c: &Connection, bot: &str) -> Result<bool, String> {
    tolerant(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM external_derived_bots WHERE bot=?1)",
        [bot],
        |r| r.get(0),
    ))
}

/// Domain guard of every LOCAL_USER entry (room message, local delegation,
/// room dispatcher): refuses an external room and a derived bot, whatever
/// the grant state. External work only runs through the mission driver in a
/// bound `external-mcp-*` conversation.
pub fn deny_local_dispatch(c: &Connection, room: Option<&str>, bot: &str) -> Result<(), String> {
    if let Some(room) = room {
        if is_external_room(c, room)? {
            return Err(ERR_LOCAL_DISPATCH.into());
        }
    }
    if !bot.is_empty() && is_derived_bot(c, bot)? {
        return Err(ERR_LOCAL_DISPATCH.into());
    }
    Ok(())
}

/// Guard of a turn outside an `external-mcp-*` conversation (direct chat,
/// room participant `<room>~<bot>`, local mission, bridge): a derived bot or
/// an external room never runs there. External conversations are governed by
/// `authority`. Without the assistant database it fails closed for room
/// participants and derived-looking ids (`ext-…`).
pub fn local_turn_allowed(conversation: &str, bot: &str) -> Result<(), String> {
    if authority::external(conversation) {
        return Ok(());
    }
    let room = rooms::participant(conversation).map(|(r, _)| r);
    match super::db::global() {
        Ok(db) => db.with(|c| Ok(deny_local_dispatch(c, room, bot)))?,
        Err(_) if room.is_some() || bot.starts_with("ext-") => Err(ERR_LOCAL_DISPATCH.into()),
        Err(_) => Ok(()),
    }
}

/// Archives the external rooms and disables the derived bots of every revoked
/// grant, and fails their still-pending tasks. Runs inside the revoking
/// transaction (`authority::revoke_principal` / `authority::revoke`).
pub fn retire_revoked(c: &Connection) -> Result<(), String> {
    let now = now_ms();
    let revoked = "SELECT id FROM external_grants WHERE revoked=1";
    tolerant(c.execute(
        &format!("UPDATE external_rooms SET retired_ms=?1 WHERE retired_ms IS NULL AND grant_id IN ({revoked})"),
        [now],
    ))?;
    tolerant(c.execute(
        &format!("UPDATE external_derived_bots SET retired_ms=?1 WHERE retired_ms IS NULL AND grant_id IN ({revoked})"),
        [now],
    ))?;
    tolerant(c.execute(
        &format!("UPDATE groups_tasks SET state='failed',error='EXECUTION_NOT_GRANTED',finished_ms=?1 WHERE state='pending' AND id IN (SELECT e.task FROM external_room_tasks e JOIN external_rooms r ON r.room=e.room WHERE r.grant_id IN ({revoked}))"),
        [now],
    ))?;
    Ok(())
}

/// How the local UI shows an external object: read-only, labelled by its
/// client (principal) rather than by the controller-chosen name or title.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMark {
    pub principal: String,
    pub grant_id: String,
    pub grant_active: bool,
    pub retired: bool,
    pub read_only: bool,
}

fn marks(db: &AssistDb, table: &str, key: &str) -> Result<BTreeMap<String, ExternalMark>, String> {
    let sql = format!(
        "SELECT x.{key},x.principal,x.grant_id,COALESCE(g.revoked,1),x.retired_ms IS NOT NULL FROM {table} x LEFT JOIN external_grants g ON g.id=x.grant_id"
    );
    let rows: Vec<(String, ExternalMark)> = db.with(|c| {
        Ok(tolerant((|| {
            let mut s = c.prepare(&sql)?;
            let rows = s.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    ExternalMark {
                        principal: r.get(1)?,
                        grant_id: r.get(2)?,
                        grant_active: r.get::<_, i64>(3)? == 0,
                        retired: r.get(4)?,
                        read_only: true,
                    },
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })()))
    })??;
    Ok(rows.into_iter().collect())
}

/// External rooms by room id, for the local room list.
pub fn room_marks(db: &AssistDb) -> Result<BTreeMap<String, ExternalMark>, String> {
    marks(db, "external_rooms", "room")
}

/// A derived bot as the desktop lists it under its client's grants.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DerivedLocal {
    pub bot: String,
    pub grant_id: String,
    pub principal: String,
    pub source_bot: String,
    pub created_ms: i64,
    pub retired: bool,
    pub grant_active: bool,
}

/// Every derived bot, newest first (trusted desktop view, read-only).
pub fn derived_local(db: &AssistDb) -> Result<Vec<DerivedLocal>, String> {
    db.with(|c| {
        Ok(tolerant((|| {
            let mut s = c.prepare(
                "SELECT x.bot,x.grant_id,x.principal,x.source_bot,x.created_ms,x.retired_ms IS NOT NULL,COALESCE(g.revoked,1) FROM external_derived_bots x LEFT JOIN external_grants g ON g.id=x.grant_id ORDER BY x.created_ms DESC LIMIT 500",
            )?;
            let rows = s.query_map([], |r| {
                Ok(DerivedLocal {
                    bot: r.get(0)?,
                    grant_id: r.get(1)?,
                    principal: r.get(2)?,
                    source_bot: r.get(3)?,
                    created_ms: r.get(4)?,
                    retired: r.get(5)?,
                    grant_active: r.get::<_, i64>(6)? == 0,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })()))
    })?
}

/// Trusted desktop revocation of ONE derived bot: it is retired (kept,
/// read-only) and its next authority check fails with
/// [`ERR_DERIVED_REVOKED`]; the grant and its other bots stay live.
pub fn retire_derived(db: &AssistDb, bot: &str) -> Result<(), String> {
    let changed = db.with(|c| {
        Ok(tolerant(c.execute(
            "UPDATE external_derived_bots SET retired_ms=?2 WHERE bot=?1 AND retired_ms IS NULL",
            params![bot, now_ms()],
        )))
    })??;
    if changed == 0 && !db.with(|c| Ok(is_derived_bot(c, bot)))?? {
        return Err("EXECUTOR_NOT_GRANTED".into());
    }
    Ok(())
}

/// Derived bots by bot id, for local rosters and pickers.
pub fn derived_bot_marks(db: &AssistDb) -> Result<BTreeMap<String, ExternalMark>, String> {
    marks(db, "external_derived_bots", "bot")
}

#[cfg(test)]
mod tests;
