//! The bot's per-turn hook: capability grants, skill grants and the compact
//! skill index. First in the chain (bots → groups → memory → reading), so the
//! hooks after it see the grants it added.

use std::collections::HashSet;
use std::sync::Arc;

use super::profile::{self, BotProfile};
use super::skills::{self, Availability, BindingState, SkillProjection};
use super::BotEnv;
use crate::core::llm::agent::{AgentDef, GrantMode, ToolSource};
use crate::core::llm::coordinator::TurnAugment;
use crate::core::skills::inject;

pub struct BotsAugment {
    env: BotEnv,
    projection: Arc<SkillProjection>,
}

impl BotsAugment {
    pub fn new(env: BotEnv, projection: Arc<SkillProjection>) -> Self {
        Self { env, projection }
    }
}

/// The profile of `agent`, importing a roster agent that predates profiles:
/// its `skills` list becomes bindings once, then the bindings are the truth.
pub fn ensure_profile(env: &BotEnv, agent: &AgentDef) -> Result<BotProfile, String> {
    let db = env.db()?;
    if let Some(p) = profile::get(&db, &agent.id)? {
        return Ok(p);
    }
    if agent.skills.is_empty() {
        return Ok(BotProfile::empty(&agent.id));
    }
    let p = profile::put(&db, &BotProfile::empty(&agent.id))?;
    skills::sync_bindings(env, &agent.id, &agent.skills)?;
    Ok(p)
}

/// Everything the hook does except building the text, for the capability
/// manifest and the tests: returns the ready skills as (tool, manifest).
pub fn apply(
    env: &BotEnv,
    projection: &SkillProjection,
    agent: &mut AgentDef,
    conversation: Option<&str>,
) -> Result<
    (
        BotProfile,
        Vec<(String, crate::core::skills::SkillManifest)>,
    ),
    String,
> {
    let db = env.db()?;
    let profile = ensure_profile(env, agent)?;
    profile::apply_capability_grants(agent, &profile);

    // Legacy `ToolSource::Skill` grants never resolve (their `skill:` name is
    // not provider-safe): drop them, keeping a `Deny` as a refusal.
    let denied: HashSet<String> = agent
        .tools
        .iter()
        .filter_map(|g| match (&g.source, g.mode) {
            (ToolSource::Skill { name }, GrantMode::Deny) => Some(name.clone()),
            _ => None,
        })
        .collect();
    agent
        .tools
        .retain(|g| !matches!(g.source, ToolSource::Skill { .. }));

    let avail = Availability::for_agent(env, agent, conversation);
    let mut ready = Vec::new();
    for (status, manifest) in skills::evaluate(env, projection, &db, &profile, &avail)? {
        if status.state != BindingState::Ready || denied.contains(&status.skill) {
            continue;
        }
        let (Some(tool), Some(manifest)) = (status.tool, manifest) else {
            continue;
        };
        profile::add_grant(agent, &tool, GrantMode::Auto);
        ready.push((tool, manifest));
    }
    agent.skills = ready.iter().map(|(_, m)| m.name.clone()).collect();
    Ok((profile, ready))
}

/// The identity block: who the bot is and what it is for. Fresh every turn,
/// so an edit applies to conversations already open.
pub fn identity_text(agent: &AgentDef, profile: &BotProfile) -> String {
    if profile.purpose.trim().is_empty() && profile.instructions.trim().is_empty() {
        return String::new();
    }
    let mut out = String::from("# About you\n\n");
    out.push_str(&format!("You are {}.", agent.name.trim()));
    if !profile.purpose.trim().is_empty() {
        out.push_str(&format!(" Your purpose: {}", profile.purpose.trim()));
    }
    if !profile.instructions.trim().is_empty() {
        out.push_str("\n\n");
        out.push_str(profile.instructions.trim());
    }
    out
}

impl TurnAugment for BotsAugment {
    fn augment(
        &self,
        agent: &mut AgentDef,
        conversation_id: &str,
        _user_input: &str,
    ) -> Option<String> {
        let (profile, ready) =
            match apply(&self.env, &self.projection, agent, Some(conversation_id)) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("[bots] {}: {e}", agent.id);
                    return None;
                }
            };
        let named: Vec<(String, &crate::core::skills::SkillManifest)> =
            ready.iter().map(|(t, m)| (t.clone(), m)).collect();
        let parts: Vec<String> = [
            identity_text(agent, &profile),
            inject::index_prompt_named(&named),
        ]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }
}
