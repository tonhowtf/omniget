//! Conversations with an explicit context, and group rooms.
//!
//! **Context (A04, A05).** Every conversation is `Projectless` (default) or
//! `Project` with one folder the user picked for it. The binding lives in
//! `groups_conversations` (assist.db) and is what `code_tools::workspace_of`
//! answers through [`DbBindings`]; there is no fallback to the last folder
//! opened elsewhere. The turn hook removes project grants (fs, shell, plan,
//! KB) from a personal conversation, so the effective agent never even sees
//! them. The process-wide folder (`code_tools::set_workspace`) survives only
//! for callers outside a turn: the embedded MCP server and the `omniget` CLI.
//!
//! **Rooms (B01–B06).** A room has members (existing bots), roles, an
//! optional coordinator, a mention policy and limits. Each member works in
//! its own session `"<room>~<bot>"`; its turn receives the room's recent
//! visible messages (with authors) and the task — never another member's
//! session. Routing, delegation, limits and cancellation: [`tasks`]. Memory:
//! the resolver maps a room (or a member session in it) to
//! `Scope::Room{room}` only; private bot memory and the user profile reach a
//! room only through a share the user created ([`store::add_share`]).
//! Parallel code work: serialised writes by default (`code_tools`), a
//! worktree only on request ([`worktree`]).

pub mod store;
pub mod tasks;
pub mod worktree;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::ctx::{self, AssistCtx, ContextKind, Scope};
use super::db::{AssistDb, Migration};
use super::tools::{need_str, opt_str, spec, AssistToolset, ERR_ASSIST_TOOL};
use crate::core::llm::agent::AgentDef;
use crate::core::llm::code_tools;
use crate::core::llm::types::ToolSpec;

pub use store::{
    participant, participant_id, room_of, Member, MentionPolicy, Room, RoomDraft, RoomMessage,
};
pub use tasks::{RoomDispatcher, RoomLimits};

pub const ERR_GROUP: &str = "ERR_GROUP";
pub const ERR_GROUP_NOT_FOUND: &str = "ERR_GROUP_NOT_FOUND";
pub const ERR_GROUP_LIMIT: &str = "ERR_GROUP_LIMIT";
pub const ERR_GROUP_BUSY: &str = "ERR_GROUP_BUSY";
pub const ERR_GROUP_CLAIMED: &str = "ERR_GROUP_CLAIMED";
pub const ERR_GROUP_NOT_OWNED: &str = "ERR_GROUP_NOT_OWNED";

/// Room ids are `grp-` + 12 hex digits: short enough that
/// `"<room>~<bot>"` stays inside the 64-char conversation-id limit.
pub const ROOM_PREFIX: &str = "grp-";

/// Tools that only make sense with a project folder; a personal
/// conversation loses them in [`augment`].
pub const KB_TOOLS: &[&str] = &["kb_search", "kb_write"];

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "groups",
    version: 1,
    sql: "
CREATE TABLE groups_conversations (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('projectless','project')),
    workspace TEXT,
    imported INTEGER NOT NULL DEFAULT 0,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE groups_rooms (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    coordinator TEXT,
    default_bot TEXT,
    mention_policy TEXT NOT NULL,
    limits TEXT NOT NULL,
    round INTEGER NOT NULL DEFAULT 0,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE groups_members (
    room TEXT NOT NULL REFERENCES groups_rooms(id) ON DELETE CASCADE,
    bot TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL,
    added_ms INTEGER NOT NULL,
    PRIMARY KEY (room, bot)
);
CREATE TABLE groups_messages (
    id TEXT PRIMARY KEY,
    room TEXT NOT NULL REFERENCES groups_rooms(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    author_kind TEXT NOT NULL CHECK (author_kind IN ('user','bot','system')),
    bot_id TEXT,
    reply_to TEXT,
    run_id TEXT,
    task_id TEXT,
    round INTEGER NOT NULL,
    text TEXT NOT NULL,
    status TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    UNIQUE (room, seq)
);
CREATE TABLE groups_tasks (
    id TEXT PRIMARY KEY,
    room TEXT NOT NULL REFERENCES groups_rooms(id) ON DELETE CASCADE,
    parent_task TEXT,
    creator_bot TEXT NOT NULL,
    creator_run TEXT,
    recipient TEXT NOT NULL,
    question TEXT NOT NULL,
    context TEXT NOT NULL DEFAULT '',
    scopes TEXT NOT NULL DEFAULT '[]',
    deliverable TEXT NOT NULL DEFAULT '',
    limit_ms INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    round INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','claimed','completed','failed','cancelled','interrupted')),
    claim_run TEXT,
    claimed_ms INTEGER,
    result TEXT,
    error TEXT,
    message_id TEXT,
    origin TEXT UNIQUE,
    created_ms INTEGER NOT NULL,
    finished_ms INTEGER
);
CREATE INDEX groups_tasks_room ON groups_tasks(room, state);
CREATE TABLE groups_runs (
    run_id TEXT PRIMARY KEY,
    room TEXT NOT NULL REFERENCES groups_rooms(id) ON DELETE CASCADE,
    bot TEXT NOT NULL,
    task_id TEXT,
    reply_to TEXT,
    round INTEGER NOT NULL,
    state TEXT NOT NULL,
    tokens INTEGER,
    started_ms INTEGER NOT NULL,
    finished_ms INTEGER
);
CREATE INDEX groups_runs_room ON groups_runs(room, state);
CREATE UNIQUE INDEX groups_runs_one_claim ON groups_runs(task_id) WHERE task_id IS NOT NULL;
CREATE TABLE groups_shares (
    id TEXT PRIMARY KEY,
    room TEXT NOT NULL REFERENCES groups_rooms(id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    record_id TEXT,
    shared_by TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_ms INTEGER NOT NULL,
    revoked_ms INTEGER
);
CREATE TABLE groups_worktrees (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    branch TEXT NOT NULL,
    conversation TEXT,
    created_ms INTEGER NOT NULL,
    removed_ms INTEGER
);
",
}];

/// Every tool this module offers; `bots` maps the `delegate` capability to these.
pub const TOOL_NAMES: &[&str] = &["delegate_task"];

// ── the installed database ───────────────────────────────────────────────

static INSTALLED: RwLock<Option<Arc<AssistDb>>> = RwLock::new(None);

fn db() -> Result<Arc<AssistDb>, String> {
    if let Some(db) = INSTALLED.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(db);
    }
    super::db::global()
}

/// Boot: makes `db` the canonical store of conversation contexts (importing
/// the legacy `workspaces.json` bindings), installs the room-aware memory
/// resolver and marks what a crash left running as interrupted. Call after
/// `code_tools::set_store_file` and `db::set_global`.
pub fn install(db: Arc<AssistDb>) {
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = Some(db.clone());
    code_tools::set_bindings(Some(Arc::new(DbBindings { db: db.clone() })));
    ctx::set_resolver(Arc::new(resolve));
    if let Err(e) = tasks::recover(&db) {
        tracing::warn!("[groups] recover: {e}");
    }
}

// ── context ──────────────────────────────────────────────────────────────

/// `code_tools`' binding store, backed by `groups_conversations`. A member
/// session (`"<room>~<bot>"`) shares its room's folder.
pub struct DbBindings {
    pub db: Arc<AssistDb>,
}

impl code_tools::WorkspaceBindings for DbBindings {
    fn get(&self, conversation: &str) -> Option<PathBuf> {
        let key = room_of(conversation).unwrap_or(conversation);
        match store::context_in(&self.db, key) {
            Ok((ContextKind::Project, p)) => p,
            _ => None,
        }
    }

    fn set(&self, conversation: &str, path: Option<&Path>) -> Result<(), String> {
        let key = room_of(conversation).unwrap_or(conversation);
        store::set_context_in(&self.db, key, path)
    }

    fn import_legacy(&self, map: &std::collections::HashMap<String, PathBuf>) {
        if let Err(e) = store::import_legacy_in(&self.db, map) {
            tracing::warn!("[groups] importing workspaces.json: {e}");
        }
    }
}

/// The context of one conversation: personal, or a project and its folder.
/// Reads the same binding every coding tool uses, so the two never disagree.
pub fn context_of(conversation_id: &str) -> (ContextKind, Option<PathBuf>) {
    match code_tools::workspace_of(conversation_id) {
        Some(p) => (ContextKind::Project, Some(p)),
        None => (ContextKind::Projectless, None),
    }
}

/// Picks the context of one conversation (the chip in the chat header).
/// Never changes another conversation nor the process-wide folder.
pub fn set_context(
    conversation_id: &str,
    path: Option<PathBuf>,
) -> Result<(ContextKind, Option<PathBuf>), String> {
    let set = code_tools::set_conversation_workspace(conversation_id, path)?;
    Ok(match set {
        Some(p) => (ContextKind::Project, Some(p)),
        None => (ContextKind::Projectless, None),
    })
}

// ── memory scopes in rooms ───────────────────────────────────────────────

/// Room-aware resolver: a room, or a member session in it, reads and writes
/// the room scope only; whole scopes the user shared into the room are
/// added read-only. A bot that is not (or no longer) a member gets nothing.
pub fn resolve(bot: &str, conversation: Option<&str>) -> AssistCtx {
    let Some(room) = conversation.and_then(room_of) else {
        return ctx::direct(bot, conversation);
    };
    let db = db().ok();
    resolve_in(db.as_deref(), bot, conversation, room)
}

pub fn resolve_in(
    db: Option<&AssistDb>,
    bot: &str,
    conversation: Option<&str>,
    room: &str,
) -> AssistCtx {
    let mut readable = Vec::new();
    let mut writable = Vec::new();
    let member = db
        .and_then(|d| store::get_room(d, room).ok())
        .map(|r| r.members.iter().any(|m| m.bot == bot))
        .unwrap_or(false);
    if member {
        let own = Scope::Room {
            conversation: room.to_string(),
        };
        readable.push(own.clone());
        writable.push(own);
        if let Some(d) = db {
            for s in store::shares(d, room).unwrap_or_default() {
                if s.record_id.is_some() {
                    continue; // single records: the memory module asks `shared_records`
                }
                if let Some(scope) = Scope::parse(&s.scope) {
                    if !readable.contains(&scope) {
                        readable.push(scope);
                    }
                }
            }
        }
    }
    AssistCtx {
        principal: ctx::LOCAL_USER.into(),
        bot_id: Some(bot.to_string()),
        conversation_id: conversation.map(str::to_string),
        run_id: None,
        readable,
        writable,
    }
}

/// Single memory records the user shared into a room: `(scope key, record id)`.
/// The memory module may return these to room members, and only these.
pub fn shared_records(room: &str) -> Vec<(String, String)> {
    db().ok()
        .and_then(|d| store::shares(&d, room).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.record_id.map(|r| (s.scope, r)))
        .collect()
}

// ── per-turn hook ────────────────────────────────────────────────────────

fn is_project_tool(name: &str) -> bool {
    code_tools::READ_TOOLS.contains(&name)
        || code_tools::WRITE_TOOLS.contains(&name)
        || KB_TOOLS.contains(&name)
}

/// Removes the project grants from `agent` (personal conversation).
pub fn strip_project_grants(agent: &mut AgentDef) -> usize {
    use crate::core::llm::agent::ToolSource;
    let before = agent.tools.len();
    agent
        .tools
        .retain(|g| !matches!(&g.source, ToolSource::Internal { name } if is_project_tool(name)));
    before - agent.tools.len()
}

const ROOM_CONTEXT_MESSAGES: usize = 12;
const ROOM_CONTEXT_CHARS: usize = 600;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// The room context one member's turn receives: who is in the room and the
/// last visible messages with their authors. Nothing else from the room.
pub fn room_context(db: &AssistDb, room_id: &str, bot: &str, user_input: &str) -> Option<String> {
    let room = store::get_room(db, room_id).ok()?;
    let me = room.members.iter().find(|m| m.bot == bot)?;
    let others: Vec<String> = room
        .members
        .iter()
        .filter(|m| m.bot != bot)
        .map(|m| {
            if m.role.is_empty() {
                format!("@{}", m.bot)
            } else {
                format!("@{} ({})", m.bot, m.role)
            }
        })
        .collect();
    let mut recent = store::recent_visible(db, room_id, ROOM_CONTEXT_MESSAGES + 1).ok()?;
    if recent
        .last()
        .map(|m| m.author == store::AuthorKind::User && m.text.trim() == user_input.trim())
        .unwrap_or(false)
    {
        recent.pop();
    }
    if recent.len() > ROOM_CONTEXT_MESSAGES {
        recent.drain(..recent.len() - ROOM_CONTEXT_MESSAGES);
    }
    let mut s = format!(
        "## Group room \"{}\"\nYou are @{}{} in a room with {}.",
        room.title,
        bot,
        if me.role.is_empty() {
            String::new()
        } else {
            format!(" ({})", me.role)
        },
        if others.is_empty() {
            "nobody else".to_string()
        } else {
            others.join(", ")
        }
    );
    if let Some(c) = &room.coordinator {
        s.push_str(&format!(" Coordinator: @{c}."));
    }
    s.push_str(
        "\nOnly the messages below are shared with you. Other members' private notes and the user's personal memory are not available here unless the user shared them; do not ask for them.",
    );
    if !recent.is_empty() {
        s.push_str("\nRecent room messages (oldest first):");
        for m in &recent {
            let who = match m.author {
                store::AuthorKind::User => "user".to_string(),
                store::AuthorKind::Bot => format!("@{}", m.bot_id.clone().unwrap_or_default()),
                store::AuthorKind::System => "app".to_string(),
            };
            s.push_str(&format!(
                "\n- {who}: {}",
                clip(m.text.trim(), ROOM_CONTEXT_CHARS)
            ));
        }
    }
    s.push_str("\nAnswer as yourself and briefly; never speak for another member.");
    if room.coordinator.as_deref() == Some(bot) && !others.is_empty() {
        s.push_str(
            " You coordinate: call a member with `delegate_task` (to = member id) only when its contribution is needed, then write one cohesive answer; members' own answers stay visible.",
        );
    }
    Some(s)
}

struct GroupsAugment;

impl crate::core::llm::coordinator::TurnAugment for GroupsAugment {
    fn augment(
        &self,
        agent: &mut AgentDef,
        conversation_id: &str,
        user_input: &str,
    ) -> Option<String> {
        let mut out = Vec::new();
        if code_tools::workspace_of(conversation_id).is_none() && strip_project_grants(agent) > 0 {
            out.push(
                "This conversation is personal (no project folder): file, shell and project-notes tools are not available here."
                    .to_string(),
            );
        }
        if let Some((room, bot)) = participant(conversation_id) {
            if let Ok(db) = db() {
                if let Some(text) = room_context(&db, room, bot, user_input) {
                    out.push(text);
                }
                // The coordinator can delegate; the backend limits apply anyway.
                if let Ok(r) = store::get_room(&db, room) {
                    if r.coordinator.as_deref() == Some(bot) && r.members.len() > 1 {
                        grant_delegate(agent);
                    }
                }
            }
        }
        (!out.is_empty()).then(|| out.join("\n\n"))
    }
}

fn grant_delegate(agent: &mut AgentDef) {
    use crate::core::llm::agent::{GrantMode, ToolGrant, ToolSource};
    for name in TOOL_NAMES {
        if !agent
            .tools
            .iter()
            .any(|g| matches!(&g.source, ToolSource::Internal { name: n } if n == name))
        {
            agent.tools.push(ToolGrant {
                source: ToolSource::Internal {
                    name: (*name).to_string(),
                },
                mode: GrantMode::Auto,
            });
        }
    }
}

/// Per-turn hook (see `llm::coordinator::TurnAugment`).
pub fn augment() -> Arc<dyn crate::core::llm::coordinator::TurnAugment> {
    Arc::new(GroupsAugment)
}

// ── tools ────────────────────────────────────────────────────────────────

struct GroupsToolset;

#[async_trait]
impl AssistToolset for GroupsToolset {
    fn name(&self) -> &'static str {
        "groups"
    }

    fn specs(&self) -> Vec<ToolSpec> {
        vec![spec(
            "delegate_task",
            "Hand one task to another member of this group room and wait for its answer. Give only the context it needs. The member works in its own session; its answer stays in the room with its name. Limits on depth, rounds, concurrency and time are enforced by the app.",
            json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Member id (as in @id)" },
                    "question": { "type": "string" },
                    "context": { "type": "string", "description": "The minimum context the member needs" },
                    "deliverable": { "type": "string", "description": "What to hand back" },
                    "scopes": { "type": "array", "items": { "type": "string" }, "description": "What the member may use (e.g. web, room memory)" },
                    "limit_s": { "type": "integer", "minimum": 5 }
                },
                "required": ["to", "question"]
            }),
        )]
    }

    async fn call(&self, ctx: &AssistCtx, tool: &str, input: Value) -> Result<Value, String> {
        if tool != "delegate_task" {
            return Err(format!("{ERR_ASSIST_TOOL}: unknown tool `{tool}`"));
        }
        let conv = ctx.conversation_id.as_deref().unwrap_or("");
        let Some(room) = room_of(conv) else {
            return Err(format!(
                "{ERR_GROUP}: delegate_task works only inside a group room"
            ));
        };
        let Some(bot) = ctx.bot_id.as_deref() else {
            return Err(format!("{ERR_GROUP}: only a member can delegate"));
        };
        let req = tasks::DelegateRequest {
            to: need_str(&input, "to")?.to_string(),
            question: need_str(&input, "question")?.to_string(),
            context: opt_str(&input, "context").unwrap_or("").to_string(),
            deliverable: opt_str(&input, "deliverable").unwrap_or("").to_string(),
            scopes: input
                .get("scopes")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            limit_s: input.get("limit_s").and_then(Value::as_u64),
        };
        let d = tasks::dispatcher()
            .ok_or_else(|| format!("{ERR_GROUP}: the room is not running in this app session"))?;
        let db = db()?;
        let origin = match (ctx.run_id.as_deref(), code_tools::current_tool_call()) {
            (Some(run), Some(call)) => Some(format!("{run}:{call}")),
            _ => None,
        };
        let t = tasks::delegate(
            &db,
            d.as_ref(),
            room,
            bot,
            ctx.run_id.as_deref(),
            origin.as_deref(),
            &req,
        )
        .await?;
        Ok(json!({
            "task_id": t.id,
            "from": t.creator_bot,
            "to": t.recipient,
            "state": t.state,
            "result": t.result,
            "error": t.error,
        }))
    }
}

pub fn toolset() -> Arc<dyn AssistToolset> {
    Arc::new(GroupsToolset)
}
