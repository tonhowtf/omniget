//! The bot's profile: what it is for, what it may do, how it treats memory.
//! Keyed by the roster's `AgentDef::id`, which never changes when the model or
//! the connection does.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::ERR_BOT;
use crate::core::assist::db::AssistDb;
use crate::core::llm::agent::{AgentDef, GrantMode, ToolGrant, ToolSource};

/// What a person picks when creating a bot, in words they understand. Each
/// maps to the tools that module exports (`TOOL_NAMES`), never to strings
/// typed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Remember what the user tells it (personal profile + the bot's notes).
    Memory,
    /// Search and read the web.
    Web,
    /// Reading journeys, progress, watched films.
    Reading,
    /// A project folder: read files, and change them / run commands on ask.
    /// Only in a conversation opened on a project.
    ProjectCode,
    /// Hand a task to another bot of a room.
    Delegate,
}

impl Capability {
    pub const ALL: [Capability; 5] = [
        Capability::Memory,
        Capability::Web,
        Capability::Reading,
        Capability::ProjectCode,
        Capability::Delegate,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Capability::Memory => "memory",
            Capability::Web => "web",
            Capability::Reading => "reading",
            Capability::ProjectCode => "project_code",
            Capability::Delegate => "delegate",
        }
    }

    /// The tools this capability stands for, from the owning modules.
    pub fn tool_names(self) -> Vec<&'static str> {
        use crate::core::assist::{groups, memory, reading, web};
        use crate::core::llm::code_tools::{READ_TOOLS, WRITE_TOOLS};
        match self {
            Capability::Memory => memory::TOOL_NAMES.to_vec(),
            Capability::Web => web::TOOL_NAMES.to_vec(),
            Capability::Reading => reading::TOOL_NAMES.to_vec(),
            Capability::Delegate => groups::TOOL_NAMES.to_vec(),
            Capability::ProjectCode => READ_TOOLS.iter().chain(WRITE_TOOLS).copied().collect(),
        }
    }
}

/// How the bot treats its memory tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPolicy {
    /// Saves and recalls on its own.
    #[default]
    Remember,
    /// Recalls on its own, asks before saving or changing anything.
    Ask,
    /// Only recalls what is already there.
    ReadOnly,
}

impl MemoryPolicy {
    fn key(self) -> &'static str {
        match self {
            MemoryPolicy::Remember => "remember",
            MemoryPolicy::Ask => "ask",
            MemoryPolicy::ReadOnly => "read_only",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "ask" => MemoryPolicy::Ask,
            "read_only" => MemoryPolicy::ReadOnly,
            _ => MemoryPolicy::Remember,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotProfile {
    pub bot_id: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub memory_policy: MemoryPolicy,
    /// The connection the user picked when creating the bot (a roster agent
    /// id or an account id), for the UI. The model and runtime themselves live
    /// on the roster's `AgentDef`.
    #[serde(default)]
    pub default_connection: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl BotProfile {
    pub fn empty(bot_id: &str) -> Self {
        Self {
            bot_id: bot_id.to_string(),
            purpose: String::new(),
            instructions: String::new(),
            capabilities: Vec::new(),
            memory_policy: MemoryPolicy::default(),
            default_connection: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }
}

/// A memory tool that only reads (recall, search, list…).
pub fn is_memory_read_tool(name: &str) -> bool {
    [
        "recall", "search", "list", "get", "show", "read", "export", "profile",
    ]
    .iter()
    .any(|w| name.contains(w))
}

/// The grant mode a capability gives one of its tools.
pub fn mode_for(cap: Capability, tool: &str, policy: MemoryPolicy) -> Option<GrantMode> {
    use crate::core::llm::code_tools::WRITE_TOOLS;
    match cap {
        Capability::Memory => match policy {
            MemoryPolicy::Remember => Some(GrantMode::Auto),
            MemoryPolicy::Ask if is_memory_read_tool(tool) => Some(GrantMode::Auto),
            MemoryPolicy::Ask => Some(GrantMode::Ask),
            MemoryPolicy::ReadOnly if is_memory_read_tool(tool) => Some(GrantMode::Auto),
            MemoryPolicy::ReadOnly => None,
        },
        Capability::ProjectCode if WRITE_TOOLS.contains(&tool) => Some(GrantMode::Ask),
        _ => Some(GrantMode::Auto),
    }
}

/// Adds the grants of every chosen capability to `agent`. A grant the agent
/// already carries for the same tool wins (a `Deny` the user set stays a
/// `Deny`): capabilities only ever add what is missing.
pub fn apply_capability_grants(agent: &mut AgentDef, profile: &BotProfile) {
    for cap in &profile.capabilities {
        for tool in cap.tool_names() {
            let Some(mode) = mode_for(*cap, tool, profile.memory_policy) else {
                continue;
            };
            add_grant(agent, tool, mode);
        }
    }
}

/// Adds an internal grant unless one for that name is already there.
pub fn add_grant(agent: &mut AgentDef, tool: &str, mode: GrantMode) {
    let present = agent
        .tools
        .iter()
        .any(|g| crate::core::llm::broker::grant_key(&g.source) == tool);
    if !present {
        agent.tools.push(ToolGrant {
            source: ToolSource::Internal {
                name: tool.to_string(),
            },
            mode,
        });
    }
}

fn caps_to_json(caps: &[Capability]) -> String {
    let mut caps = caps.to_vec();
    caps.sort();
    caps.dedup();
    serde_json::to_string(&caps).unwrap_or_else(|_| "[]".into())
}

fn row_to_profile(r: &rusqlite::Row<'_>) -> rusqlite::Result<BotProfile> {
    let caps: String = r.get(3)?;
    let policy: String = r.get(4)?;
    Ok(BotProfile {
        bot_id: r.get(0)?,
        purpose: r.get(1)?,
        instructions: r.get(2)?,
        capabilities: serde_json::from_str(&caps).unwrap_or_default(),
        memory_policy: MemoryPolicy::parse(&policy),
        default_connection: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

pub fn get(db: &AssistDb, bot: &str) -> Result<Option<BotProfile>, String> {
    db.with(|c| {
        c.query_row(
            "SELECT bot_id, purpose, instructions, capabilities, memory_policy, default_connection, created_at, updated_at \
             FROM bots_profiles WHERE bot_id = ?1",
            params![bot],
            row_to_profile,
        )
        .optional()
    })
}

pub fn list(db: &AssistDb) -> Result<Vec<BotProfile>, String> {
    db.with(|c| {
        let mut st = c.prepare(
            "SELECT bot_id, purpose, instructions, capabilities, memory_policy, default_connection, created_at, updated_at \
             FROM bots_profiles ORDER BY bot_id",
        )?;
        let rows = st.query_map([], row_to_profile)?;
        rows.collect()
    })
}

/// Creates or replaces the profile. `created_at` survives a replace.
pub fn put(db: &AssistDb, profile: &BotProfile) -> Result<BotProfile, String> {
    if profile.bot_id.trim().is_empty() {
        return Err(format!("{ERR_BOT}: a bot id is required"));
    }
    if profile.purpose.len() > 2_000 || profile.instructions.len() > 20_000 {
        return Err(format!("{ERR_BOT}: purpose or instructions too long"));
    }
    let now = crate::core::assist::now_ms();
    db.with(|c| {
        c.execute(
            "INSERT INTO bots_profiles(bot_id, purpose, instructions, capabilities, memory_policy, default_connection, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
             ON CONFLICT(bot_id) DO UPDATE SET purpose = excluded.purpose, instructions = excluded.instructions, \
             capabilities = excluded.capabilities, memory_policy = excluded.memory_policy, \
             default_connection = excluded.default_connection, updated_at = excluded.updated_at",
            params![
                profile.bot_id,
                profile.purpose.trim(),
                profile.instructions.trim(),
                caps_to_json(&profile.capabilities),
                profile.memory_policy.key(),
                profile.default_connection,
                now
            ],
        )
    })?;
    get(db, &profile.bot_id)?.ok_or_else(|| format!("{ERR_BOT}: profile not saved"))
}

/// Drops the profile and the bot's skill bindings (the bot left the roster).
/// Reads already traced stay: they are the record of past runs.
pub fn delete(db: &AssistDb, bot: &str) -> Result<(), String> {
    db.tx(|tx| {
        tx.execute("DELETE FROM bots_profiles WHERE bot_id = ?1", params![bot])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM bots_skill_bindings WHERE bot_id = ?1",
            params![bot],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// New scoped profile only. A replay never overwrites a local profile edit.
pub(crate) fn create_scoped_tx(tx: &rusqlite::Transaction, p: &BotProfile) -> Result<(), String> {
    if p.bot_id.is_empty() || p.purpose.len() > 2000 || p.instructions.len() > 20000 {
        return Err("INVALID_EXTERNAL_PROFILE".into());
    }
    tx.execute("INSERT OR IGNORE INTO bots_profiles(bot_id,purpose,instructions,capabilities,memory_policy,default_connection,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",params![p.bot_id,p.purpose.trim(),p.instructions.trim(),caps_to_json(&p.capabilities),p.memory_policy.key(),p.default_connection,crate::core::assist::now_ms()]).map_err(|_|"EXTERNAL_PROFILE_STORAGE")?;
    let actual=tx.query_row("SELECT bot_id,purpose,instructions,capabilities,memory_policy,default_connection,created_at,updated_at FROM bots_profiles WHERE bot_id=?1",[&p.bot_id],row_to_profile).map_err(|_|"EXTERNAL_PROFILE_STORAGE")?;
    if actual.purpose != p.purpose.trim()
        || actual.instructions != p.instructions.trim()
        || actual.capabilities != p.capabilities
        || actual.memory_policy != p.memory_policy
        || actual.default_connection != p.default_connection
    {
        return Err("EXTERNAL_PROFILE_CHANGED".into());
    }
    Ok(())
}
