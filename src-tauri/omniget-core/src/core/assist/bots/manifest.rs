//! The effective capability manifest of one bot: for every capability and
//! skill, whether it was asked for, whether it is really available this turn,
//! and if not, why and how to fix it. Built from the effective agent (after
//! every per-turn hook), the tools the broker really has and what the runtime
//! can reach — never from the checkboxes alone.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::profile::{self, BotProfile, Capability};
use super::skills::{self, Availability, BindingStatus, Fix, SkillProjection};
use super::BotEnv;
use crate::core::llm::agent::{AgentDef, GrantMode, RuntimeKind};
use crate::core::llm::broker::grant_key;
use crate::core::llm::caps::RuntimeCaps;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapState {
    Available,
    /// Some of its tools are offered, some are not.
    Partial,
    Missing,
    NotRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityStatus {
    pub id: Capability,
    pub requested: bool,
    pub state: CapState,
    /// `tools` (OmniGet tools offered) or `builtin` (the runtime's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Tools of this capability offered to the model this turn.
    pub tools: Vec<String>,
    /// Tools of this capability the bot would need but does not get.
    pub missing_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnavailableGrant {
    pub tool: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub bot_id: String,
    pub runtime_kind: String,
    pub runtime: RuntimeCaps,
    pub projectless: bool,
    pub profile: BotProfile,
    pub capabilities: Vec<CapabilityStatus>,
    pub skills: Vec<BindingStatus>,
    /// Every tool name the model is really offered this turn.
    pub offered_tools: Vec<String>,
    /// Grants that point at a tool nobody provides right now (an MCP server
    /// that is down, a legacy name).
    pub unavailable_grants: Vec<UnavailableGrant>,
}

fn runtime_kind(agent: &AgentDef) -> &'static str {
    match agent.runtime {
        RuntimeKind::Native => "native",
        RuntimeKind::Cli { .. } => "cli",
        RuntimeKind::Acp { .. } => "acp",
    }
}

/// `effective` is the agent after every per-turn hook
/// (`Coordinator::effective_agent`); `stored` is what the roster holds.
pub fn report(
    env: &BotEnv,
    projection: &SkillProjection,
    stored: &AgentDef,
    effective: &AgentDef,
    conversation: Option<&str>,
) -> Result<CapabilityReport, String> {
    let db = env.db()?;
    let profile = super::augment::ensure_profile(env, stored)?;
    let avail = Availability::for_agent(env, effective, conversation);
    let broker = env.broker();
    let offered: Vec<String> = match (&broker, avail.reaches_tools) {
        (Some(b), true) => b
            .specs_for(&effective.tools)
            .into_iter()
            .map(|s| s.name)
            .collect(),
        _ => Vec::new(),
    };
    let offered_set: HashSet<&str> = offered.iter().map(String::as_str).collect();

    let mut capabilities = Vec::new();
    for cap in Capability::ALL {
        let requested = profile.has(cap);
        let names: Vec<&str> = cap
            .tool_names()
            .into_iter()
            .filter(|t| profile::mode_for(cap, t, profile.memory_policy).is_some())
            .collect();
        let tools: Vec<String> = names
            .iter()
            .filter(|t| offered_set.contains(**t))
            .map(|t| t.to_string())
            .collect();
        let missing_tools: Vec<String> = names
            .iter()
            .filter(|t| !offered_set.contains(**t))
            .map(|t| t.to_string())
            .collect();
        let mut st = CapabilityStatus {
            id: cap,
            requested,
            state: CapState::Missing,
            via: None,
            tools: tools.clone(),
            missing_tools: if requested {
                missing_tools.clone()
            } else {
                Vec::new()
            },
            reason: None,
            fix: None,
        };
        let denied = names
            .iter()
            .any(|t| matches!(avail.grant(t), Some(GrantMode::Deny)));
        if cap == Capability::Web && avail.runtime.builtin_web {
            st.state = CapState::Available;
            st.via = Some("builtin".into());
            st.reason = Some("runtime_builtin".into());
        } else if !requested {
            st.state = CapState::NotRequested;
            st.fix = Some(Fix::cap("enable_capability", cap));
        } else if names.is_empty() {
            st.reason = Some("module_has_no_tools".into());
        } else if !avail.reaches_tools {
            st.reason = Some("runtime_cannot_reach_tools".into());
            st.fix = Some(Fix::new("switch_connection"));
        } else if cap == Capability::ProjectCode && avail.projectless {
            st.reason = Some("projectless".into());
            st.fix = Some(Fix::new("open_project"));
        } else if tools.is_empty() {
            st.reason = Some(
                if denied {
                    "denied_by_user"
                } else {
                    "tool_not_registered"
                }
                .into(),
            );
            st.fix = Some(Fix::target("grant_tool", names[0]));
        } else if !missing_tools.is_empty() {
            st.state = CapState::Partial;
            st.via = Some("tools".into());
            st.reason = Some(
                if denied {
                    "denied_by_user"
                } else {
                    "tool_not_registered"
                }
                .into(),
            );
        } else {
            st.state = CapState::Available;
            st.via = Some("tools".into());
        }
        capabilities.push(st);
    }
    // Reading confirms availability only from pages fetched with `web_fetch`
    // (reading::evidence); without working web it cannot do its job fully.
    let web_ok = capabilities
        .iter()
        .any(|c| c.id == Capability::Web && c.state == CapState::Available);
    if let Some(r) = capabilities
        .iter_mut()
        .find(|c| c.id == Capability::Reading && c.requested && c.state == CapState::Available)
    {
        if !web_ok {
            r.state = CapState::Partial;
            r.reason = Some("reading_needs_web".into());
            r.fix = Some(Fix::cap("enable_capability", Capability::Web));
        }
    }

    let skills = skills::evaluate(env, projection, &db, &profile, &avail)?
        .into_iter()
        .map(|(s, _)| s)
        .collect();

    let unavailable_grants = effective
        .tools
        .iter()
        .filter(|g| !matches!(g.mode, GrantMode::Deny))
        .map(|g| grant_key(&g.source))
        .filter(|k| !avail.is_registered(k))
        .map(|tool| UnavailableGrant {
            reason: if tool.starts_with("mcp:") {
                "mcp_not_connected".into()
            } else {
                "tool_not_registered".into()
            },
            tool,
        })
        .collect();

    Ok(CapabilityReport {
        bot_id: stored.id.clone(),
        runtime_kind: runtime_kind(effective).into(),
        runtime: avail.runtime.clone(),
        projectless: avail.projectless,
        profile,
        capabilities,
        skills,
        offered_tools: offered,
        unavailable_grants,
    })
}
