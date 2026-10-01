use serde::{Deserialize, Serialize};

use super::types::{ModelRef, ProviderId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Coordinator,
    Worker,
    Advisor,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "runtime", rename_all = "snake_case")]
pub enum CandidateRuntime {
    Native { provider: ProviderId },
    Cli { account_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(flatten)]
    pub runtime: CandidateRuntime,
    pub model: String,
    pub max_cost_per_1k: Option<f64>,
    #[serde(default)]
    pub min_context: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum ModelPolicy {
    Fixed { model: ModelRef },
    Route { chain: Vec<Candidate> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ToolSource {
    Internal { name: String },
    Mcp { server: String, tool: String },
    Skill { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantMode {
    Auto,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolGrant {
    #[serde(flatten)]
    pub source: ToolSource,
    pub mode: GrantMode,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub usd_per_day: Option<f64>,
    pub tokens_per_turn: Option<u32>,
    #[serde(default)]
    pub max_tool_calls_per_turn: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeKind {
    Native,
    Cli {
        cli: String,
        account: String,
    },
    Acp {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    pub id: String,
    pub tint: [u8; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDef {
    pub id: String,
    pub name: String,
    pub role: AgentRole,
    pub system_prompt: String,
    pub model: ModelPolicy,
    #[serde(default)]
    pub tools: Vec<ToolGrant>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub budget: Budget,
    pub runtime: RuntimeKind,
    pub skin: Option<Skin>,
}
