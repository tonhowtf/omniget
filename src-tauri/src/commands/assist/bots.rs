//! Commands for `assist::bots`: create a bot from the UI, its profile and
//! capabilities, its skill bindings, the effective capability manifest and
//! the per-run trace of skill reads. Owner: worker W3.
//!
//! The bot's id is the roster's `AgentDef::id`. Model and runtime stay on the
//! roster agent; switching them (`assist_bot_set_connection`) never changes
//! the id, the profile, the bindings or the memory (B07).

use omniget_core::core::assist::bots::{
    self, profile, skills as bot_skills, BotEnv, BotProfile, Capability, MemoryPolicy,
};
use omniget_core::core::assist::db;
use omniget_core::core::llm::agent::{AgentDef, AgentRole};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::AppState;

fn env() -> BotEnv {
    BotEnv::global()
}

fn to_value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn roster_agent(state: &AppState, bot_id: &str) -> Result<AgentDef, String> {
    state
        .llm
        .agent(bot_id)
        .ok_or_else(|| format!("{}: no bot `{bot_id}` in the roster", bots::ERR_BOT))
}

/// Keeps `AgentDef::skills` (what the roster editor shows) equal to the
/// bindings, which are the truth.
fn mirror_skills(state: &AppState, bot_id: &str) -> Result<(), String> {
    let db = db::global()?;
    let mut agent = roster_agent(state, bot_id)?;
    let names: Vec<String> = bot_skills::bindings(&db, bot_id)?
        .into_iter()
        .map(|b| b.skill)
        .collect();
    if agent.skills != names {
        agent.skills = names;
        state.llm.roster_update(agent)?;
    }
    Ok(())
}

/// A roster id from a display name: lowercase ascii, dashes, unique.
fn new_bot_id(state: &AppState, name: &str) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        "bot".to_string()
    } else {
        format!("bot-{}", &base[..base.len().min(40)])
    };
    let taken: Vec<String> = state.llm.roster().into_iter().map(|a| a.id).collect();
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewBot {
    pub name: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub memory_policy: MemoryPolicy,
    /// Installed skills to bind (read on, scripts off).
    #[serde(default)]
    pub skills: Vec<String>,
    /// A roster agent whose model and runtime the bot starts with (a
    /// connection made in the Accounts tab).
    pub connection_agent_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BotView {
    pub agent: AgentDef,
    pub profile: BotProfile,
    pub bindings: Vec<bot_skills::Binding>,
}

fn view(state: &AppState, bot_id: &str) -> Result<BotView, String> {
    let agent = roster_agent(state, bot_id)?;
    let db = db::global()?;
    let profile = bots::augment::ensure_profile(&env(), &agent)?;
    Ok(BotView {
        bindings: bot_skills::bindings(&db, bot_id)?,
        agent,
        profile,
    })
}

/// Creates a bot: a roster agent on the chosen connection, with no tools of
/// its own (no code, no shell), its profile, and its skill bindings.
#[tauri::command]
pub async fn assist_bot_create(state: State<'_, AppState>, bot: NewBot) -> Result<Value, String> {
    let name = bot.name.trim();
    if name.is_empty() || name.chars().count() > 48 {
        return Err(format!(
            "{}: a name of 1 to 48 characters is required",
            bots::ERR_BOT
        ));
    }
    let source = roster_agent(&state, &bot.connection_agent_id)?;
    let id = new_bot_id(&state, name);
    let agent = AgentDef {
        id: id.clone(),
        name: name.to_string(),
        role: AgentRole::Worker,
        system_prompt: String::new(),
        model: source.model.clone(),
        tools: Vec::new(),
        skills: Vec::new(),
        budget: Default::default(),
        runtime: source.runtime.clone(),
        skin: source.skin.clone(),
    };
    state.llm.roster_create(agent)?;
    let db = db::global()?;
    let result = (|| -> Result<(), String> {
        profile::put(
            &db,
            &BotProfile {
                bot_id: id.clone(),
                purpose: bot.purpose.clone(),
                instructions: bot.instructions.clone(),
                capabilities: bot.capabilities.clone(),
                memory_policy: bot.memory_policy,
                default_connection: Some(bot.connection_agent_id.clone()),
                created_at: 0,
                updated_at: 0,
            },
        )?;
        for skill in &bot.skills {
            bot_skills::bind(&env(), &id, skill, true, false)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        // No half-made bot: the roster entry goes with the failure.
        let _ = state.llm.roster_delete(&id);
        return Err(e);
    }
    mirror_skills(&state, &id)?;
    to_value(view(&state, &id)?)
}

#[tauri::command]
pub async fn assist_bot_get(state: State<'_, AppState>, bot_id: String) -> Result<Value, String> {
    to_value(view(&state, &bot_id)?)
}

#[tauri::command]
pub async fn assist_bots_list() -> Result<Value, String> {
    let db = db::global()?;
    to_value(profile::list(&db)?)
}

/// Saves purpose, instructions, capabilities and memory policy. The id comes
/// from the argument, never from the payload's body alone.
#[tauri::command]
pub async fn assist_bot_save_profile(
    state: State<'_, AppState>,
    bot_id: String,
    profile: BotProfile,
) -> Result<Value, String> {
    roster_agent(&state, &bot_id)?;
    let mut p = profile;
    p.bot_id = bot_id.clone();
    let db = db::global()?;
    to_value(profile::put(&db, &p)?)
}

/// Moves the bot to another connection (model + runtime of another roster
/// agent). Id, profile, bindings and memory stay. A conversation already
/// running keeps the session it pinned (runs layer).
#[tauri::command]
pub async fn assist_bot_set_connection(
    state: State<'_, AppState>,
    bot_id: String,
    connection_agent_id: String,
) -> Result<Value, String> {
    let mut agent = roster_agent(&state, &bot_id)?;
    let source = roster_agent(&state, &connection_agent_id)?;
    agent.model = source.model;
    agent.runtime = source.runtime;
    state.llm.roster_update(agent)?;
    let db = db::global()?;
    let mut p = bots::augment::ensure_profile(&env(), &roster_agent(&state, &bot_id)?)?;
    p.default_connection = Some(connection_agent_id);
    profile::put(&db, &p)?;
    to_value(view(&state, &bot_id)?)
}

#[tauri::command]
pub async fn assist_bot_bind_skill(
    state: State<'_, AppState>,
    bot_id: String,
    skill: String,
    allow_read: Option<bool>,
    allow_scripts: Option<bool>,
) -> Result<Value, String> {
    roster_agent(&state, &bot_id)?;
    bot_skills::bind(
        &env(),
        &bot_id,
        &skill,
        allow_read.unwrap_or(true),
        allow_scripts.unwrap_or(false),
    )?;
    mirror_skills(&state, &bot_id)?;
    to_value(view(&state, &bot_id)?)
}

#[tauri::command]
pub async fn assist_bot_unbind_skill(
    state: State<'_, AppState>,
    bot_id: String,
    skill: String,
) -> Result<Value, String> {
    let db = db::global()?;
    bot_skills::unbind(&db, &bot_id, &skill)?;
    mirror_skills(&state, &bot_id)?;
    to_value(view(&state, &bot_id)?)
}

/// Read and run-scripts are two separate switches.
#[tauri::command]
pub async fn assist_bot_skill_grants(
    state: State<'_, AppState>,
    bot_id: String,
    skill: String,
    allow_read: bool,
    allow_scripts: bool,
) -> Result<Value, String> {
    let db = db::global()?;
    bot_skills::set_grants(&db, &bot_id, &skill, allow_read, allow_scripts)?;
    to_value(view(&state, &bot_id)?)
}

/// The effective capability manifest: for each capability and skill, asked
/// for / available / missing, with the reason and the fix. Computed from the
/// agent after every per-turn hook, the broker's real tools and the runtime.
#[tauri::command]
pub async fn assist_bot_capabilities(
    state: State<'_, AppState>,
    bot_id: String,
    conversation_id: Option<String>,
) -> Result<Value, String> {
    let stored = roster_agent(&state, &bot_id)?;
    let conv = conversation_id.unwrap_or_default();
    let (effective, _) = state.llm.coordinator().effective_agent(&stored, &conv, "");
    let conv_opt = (!conv.is_empty()).then_some(conv.as_str());
    let report = bots::report(
        &env(),
        &bot_skills::projection(),
        &stored,
        &effective,
        conv_opt,
    )?;
    to_value(report)
}

/// Which skill, version and file each run really read.
#[tauri::command]
pub async fn assist_bot_skill_reads(
    bot_id: Option<String>,
    run_id: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let db = db::global()?;
    to_value(bot_skills::skill_reads(
        &db,
        run_id.as_deref(),
        bot_id.as_deref(),
        limit.unwrap_or(50),
    )?)
}

/// The capability list with its tools, for the "Create bot" form.
#[tauri::command]
pub async fn assist_bot_capability_catalog() -> Result<Value, String> {
    Ok(Value::Array(
        Capability::ALL
            .iter()
            .map(|c| json!({ "id": c.id(), "tools": c.tool_names() }))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_bot_payload_parses_with_defaults() {
        let bot: NewBot = serde_json::from_value(json!({
            "name": "Companheiro de leitura",
            "capabilities": ["memory", "web", "reading"],
            "connection_agent_id": "connection-x"
        }))
        .unwrap();
        assert_eq!(bot.memory_policy, MemoryPolicy::Remember);
        assert!(bot.skills.is_empty());
        assert_eq!(
            bot.capabilities,
            vec![Capability::Memory, Capability::Web, Capability::Reading]
        );
        // Project code is never switched on by default.
        assert!(!bot.capabilities.contains(&Capability::ProjectCode));
    }
}
