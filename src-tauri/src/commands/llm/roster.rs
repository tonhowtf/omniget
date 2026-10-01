use omniget_core::core::llm::agent::AgentDef;
use omniget_core::core::llm::roster_store;
use serde_json::Value;
use tauri::State;

use crate::AppState;

fn to_agent(value: Value) -> Result<AgentDef, String> {
    serde_json::from_value(value).map_err(|e| format!("{}: {e}", roster_store::ERR_ROSTER_ID))
}

fn ok(list: Vec<AgentDef>) -> Result<Value, String> {
    serde_json::to_value(list).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_roster_list(state: State<'_, AppState>) -> Result<Value, String> {
    ok(state.llm.roster())
}

#[tauri::command]
pub async fn llm_roster_create(state: State<'_, AppState>, agent: Value) -> Result<Value, String> {
    let agent = to_agent(agent)?;
    ok(state.llm.roster_create(agent)?)
}

#[tauri::command]
pub async fn llm_roster_update(state: State<'_, AppState>, agent: Value) -> Result<Value, String> {
    let agent = to_agent(agent)?;
    ok(state.llm.roster_update(agent)?)
}

#[tauri::command]
pub async fn llm_roster_delete(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Value, String> {
    ok(state.llm.roster_delete(&agent_id)?)
}

#[tauri::command]
pub async fn llm_roster_apply_template(
    state: State<'_, AppState>,
    template: String,
) -> Result<Value, String> {
    ok(state.llm.roster_apply_template(&template)?)
}

#[tauri::command]
pub async fn llm_acp_detect() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!(
        omniget_core::core::llm::acp::detect().await
    ))
}

#[tauri::command]
pub async fn llm_acp_agent_create(
    state: tauri::State<'_, crate::AppState>,
    name: String,
    command: String,
    args: Vec<String>,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::agent::{AgentDef, AgentRole, ModelPolicy, RuntimeKind};
    use omniget_core::core::llm::types::{ModelRef, ProviderId};
    if command.trim().is_empty() {
        return Err("ERR_LLM_ACP: empty command".into());
    }
    let roster = state.llm.roster();
    if roster.iter().any(|a| {
        a.name == name
            && matches!(&a.runtime,
        RuntimeKind::Acp { command: c, args: argv } if c == &command && argv == &args)
    }) {
        return serde_json::to_value(roster).map_err(|e| e.to_string());
    }
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() {
        "acp".to_string()
    } else {
        base
    };
    let taken: Vec<String> = state.llm.roster().into_iter().map(|a| a.id).collect();
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let agent = AgentDef {
        id,
        name: if name.trim().is_empty() {
            command.clone()
        } else {
            name
        },
        role: AgentRole::Worker,
        system_prompt: String::new(),
        model: ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("acp"),
                model: command.clone(),
            },
        },
        tools: Vec::new(),
        skills: Vec::new(),
        budget: Default::default(),
        runtime: RuntimeKind::Acp { command, args },
        skin: None,
    };
    serde_json::to_value(state.llm.roster_create(agent)?).map_err(|e| e.to_string())
}
