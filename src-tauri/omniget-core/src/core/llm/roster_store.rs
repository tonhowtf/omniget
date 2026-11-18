use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use super::agent::{
    AgentDef, AgentRole, Budget, GrantMode, ModelPolicy, RuntimeKind, ToolGrant, ToolSource,
};
use super::types::ProviderId;

pub const ERR_ROSTER_DUP: &str = "ERR_LLM_ROSTER_DUP";
pub const ERR_ROSTER_MISSING: &str = "ERR_LLM_ROSTER_MISSING";
pub const ERR_ROSTER_IO: &str = "ERR_LLM_ROSTER_IO";
pub const ERR_ROSTER_ID: &str = "ERR_LLM_ROSTER_ID";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterError {
    pub code: &'static str,
    pub message: String,
}

impl RosterError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RosterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RosterError {}

pub type Result<T> = std::result::Result<T, RosterError>;

pub fn llm_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("llm"))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn code_grants() -> Vec<ToolGrant> {
    use super::code_tools::{READ_TOOLS, WRITE_TOOLS};
    let grant = |name: &str, mode| ToolGrant {
        source: ToolSource::Internal {
            name: name.to_string(),
        },
        mode,
    };
    READ_TOOLS
        .iter()
        .map(|n| grant(n, GrantMode::Auto))
        .chain(WRITE_TOOLS.iter().map(|n| grant(n, GrantMode::Ask)))
        .chain(std::iter::once(grant("agent_delegate", GrantMode::Ask)))
        .chain(std::iter::once(grant("kb_search", GrantMode::Auto)))
        .chain(std::iter::once(grant("kb_write", GrantMode::Ask)))
        .collect()
}

fn with_code_grants(mut agents: Vec<AgentDef>) -> Vec<AgentDef> {
    for a in agents.iter_mut().filter(|a| a.id == "omni") {
        let has = a.tools.iter().any(|g| {
            matches!(&g.source, ToolSource::Internal { name } if name.starts_with("fs_") || name == "shell_exec")
        });
        if !has {
            a.tools.extend(code_grants());
        }
        for (tool, mode) in [("kb_search", GrantMode::Auto), ("kb_write", GrantMode::Ask)] {
            let has = a
                .tools
                .iter()
                .any(|g| matches!(&g.source, ToolSource::Internal { name } if name == tool));
            if !has {
                a.tools.push(ToolGrant {
                    source: ToolSource::Internal {
                        name: tool.to_string(),
                    },
                    mode,
                });
            }
        }
        let delegates = a.tools.iter().any(
            |g| matches!(&g.source, ToolSource::Internal { name } if name == "agent_delegate"),
        );
        if !delegates {
            a.tools.push(ToolGrant {
                source: ToolSource::Internal {
                    name: "agent_delegate".to_string(),
                },
                mode: GrantMode::Ask,
            });
        }
    }
    agents
}

fn candidate(provider: &str, model: &str) -> super::agent::Candidate {
    super::agent::Candidate {
        runtime: super::agent::CandidateRuntime::Native {
            provider: ProviderId::new(provider),
        },
        model: model.to_string(),
        max_cost_per_1k: None,
        min_context: 0,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptDefaults {
    pub omni: String,
    pub builder: String,
    pub scout: String,
    pub template_chief: String,
    pub template_researcher: String,
    pub template_writer: String,
    pub template_em: String,
    pub template_reviewer: String,
}

impl Default for PromptDefaults {
    fn default() -> Self {
        Self {
            omni: "You are Omni, the assistant inside OmniGet. Answer briefly and in the language the user writes in. Use a tool only when it is clearly needed.".into(),
            builder: "You are Builder, the coding agent inside OmniGet. Read the code before you change it, make the smallest edit that solves the task, and say what you changed in one short paragraph.".into(),
            scout: "You are Scout, the reading and research agent inside OmniGet. Find the relevant files and facts, quote where each one came from, and never edit anything.".into(),
            template_chief: "You run the user's day: you split a request into tasks, hand each one to a worker, and report back in one paragraph.".into(),
            template_researcher: "You gather facts and always say where each fact came from.".into(),
            template_writer: "You turn notes into short, plain prose. No filler.".into(),
            template_em: "You plan the work, decide the order and keep the scope honest.".into(),
            template_reviewer: "You review a change for correctness first and style last.".into(),
        }
    }
}

static PROMPT_DEFAULTS: OnceLock<RwLock<PromptDefaults>> = OnceLock::new();

fn prompt_slot() -> &'static RwLock<PromptDefaults> {
    PROMPT_DEFAULTS.get_or_init(|| RwLock::new(PromptDefaults::default()))
}

pub fn prompt_defaults() -> PromptDefaults {
    prompt_slot()
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

pub fn set_prompt_defaults(next: PromptDefaults) {
    if let Ok(mut guard) = prompt_slot().write() {
        *guard = next;
    }
}

pub fn default_roster() -> Vec<AgentDef> {
    roster_with_prompts(&prompt_defaults())
}

fn roster_with_prompts(prompts: &PromptDefaults) -> Vec<AgentDef> {
    let local_first = || ModelPolicy::Route {
        chain: vec![
            candidate("ollama", "qwen3:8b"),
            candidate("anthropic", "claude-sonnet-5"),
            candidate("openai", "gpt-4o-mini"),
        ],
    };
    let agent =
        |id: &str, name: &str, role: AgentRole, prompt: &str, tools: Vec<ToolGrant>| AgentDef {
            id: id.into(),
            name: name.into(),
            role,
            system_prompt: prompt.into(),
            model: local_first(),
            tools,
            skills: Vec::new(),
            budget: Budget::default(),
            runtime: RuntimeKind::Native,
            skin: None,
        };
    let read_only: Vec<ToolGrant> = code_grants()
        .into_iter()
        .filter(|g| g.mode == GrantMode::Auto)
        .collect();
    vec![
        agent(
            "omni",
            "Omni",
            AgentRole::Coordinator,
            &prompts.omni,
            code_grants(),
        ),
        agent(
            "builder",
            "Builder",
            AgentRole::Worker,
            &prompts.builder,
            code_grants(),
        ),
        agent(
            "scout",
            "Scout",
            AgentRole::Worker,
            &prompts.scout,
            read_only,
        ),
    ]
}

pub struct RosterStore {
    path: PathBuf,
    mutation: Mutex<()>,
    cache: RwLock<Option<Arc<Vec<AgentDef>>>>,
}

impl RosterStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mutation: Mutex::new(()),
            cache: RwLock::new(None),
        }
    }

    pub fn list(&self) -> Arc<Vec<AgentDef>> {
        if let Some(cached) = self.cache.read().ok().and_then(|g| g.clone()) {
            return cached;
        }
        let loaded = Arc::new(self.read_from_disk());
        if let Ok(mut g) = self.cache.write() {
            if let Some(current) = g.as_ref() {
                return current.clone();
            }
            *g = Some(loaded.clone());
        }
        loaded
    }

    pub fn get(&self, id: &str) -> Option<AgentDef> {
        self.list().iter().find(|a| a.id == id).cloned()
    }

    pub fn create(&self, agent: AgentDef) -> Result<AgentDef> {
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
        if !valid_id(&agent.id) {
            return Err(RosterError::new(
                ERR_ROSTER_ID,
                format!("invalid agent id: {:?}", agent.id),
            ));
        }
        let mut all = (*self.list()).clone();
        if all.iter().any(|a| a.id == agent.id) {
            return Err(RosterError::new(
                ERR_ROSTER_DUP,
                format!("agent {} already exists", agent.id),
            ));
        }
        all.push(agent.clone());
        self.save(all)?;
        Ok(agent)
    }

    pub fn update(&self, agent: AgentDef) -> Result<AgentDef> {
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = (*self.list()).clone();
        let slot = all.iter_mut().find(|a| a.id == agent.id).ok_or_else(|| {
            RosterError::new(ERR_ROSTER_MISSING, format!("no agent {}", agent.id))
        })?;
        *slot = agent.clone();
        self.save(all)?;
        Ok(agent)
    }

    pub fn apply_planned(
        &self,
        agent: AgentDef,
        before: Option<AgentDef>,
        source: AgentDef,
    ) -> Result<AgentDef> {
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
        if !valid_id(&agent.id) {
            return Err(RosterError::new(ERR_ROSTER_ID, "invalid planned id"));
        }
        let mut all = (*self.list()).clone();
        let equal = |a: &AgentDef, b: &AgentDef| {
            serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
        };
        if let Some(existing) = all.iter().find(|a| a.id == agent.id) {
            if equal(existing, &agent) {
                return Ok(existing.clone());
            }
        }
        if !all.iter().any(|a| a.id == source.id && equal(a, &source)) {
            return Err(RosterError::new(
                "ERR_HELP_REVISION",
                "connection changed; prepare a new plan",
            ));
        }
        match before {
            Some(previous) => {
                let slot = all
                    .iter_mut()
                    .find(|a| a.id == agent.id)
                    .ok_or_else(|| RosterError::new("ERR_HELP_REVISION", "agent was removed"))?;
                if !equal(slot, &previous) {
                    return Err(RosterError::new(
                        "ERR_HELP_REVISION",
                        "agent changed; prepare a new plan",
                    ));
                }
                *slot = agent.clone();
            }
            None => {
                if all.iter().any(|a| a.id == agent.id) {
                    return Err(RosterError::new(
                        ERR_ROSTER_DUP,
                        "planned id already exists",
                    ));
                }
                all.push(agent.clone());
            }
        }
        self.save(all)?;
        Ok(agent)
    }

    fn read_from_disk(&self) -> Vec<AgentDef> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return default_roster();
        };
        match serde_json::from_str::<Vec<AgentDef>>(&text) {
            Ok(list) => with_code_grants(list),
            Err(error) => {
                tracing::warn!("[llm] roster.json is not readable ({error}); using the default");
                default_roster()
            }
        }
    }

    fn save(&self, agents: Vec<AgentDef>) -> Result<()> {
        let text = serde_json::to_string_pretty(&agents)
            .map_err(|e| RosterError::new(ERR_ROSTER_IO, e.to_string()))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RosterError::new(ERR_ROSTER_IO, e.to_string()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())
            .map_err(|e| RosterError::new(ERR_ROSTER_IO, e.to_string()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| RosterError::new(ERR_ROSTER_IO, e.to_string()))?;
        if let Ok(mut g) = self.cache.write() {
            *g = Some(Arc::new(agents));
        }
        Ok(())
    }
}
