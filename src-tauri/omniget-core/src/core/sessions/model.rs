use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub type ToolId = String;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write + self.reasoning
    }
    pub fn add(&mut self, o: &TokenUsage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.reasoning += o.reasoning;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    Error,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name_canonical: String,
    pub name_raw: String,
    pub input: serde_json::Value,
    pub result: Option<String>,
    pub status: ToolStatus,
    pub ms: Option<u64>,
    pub subagent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub ts: String,
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: TokenUsage,
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub tool: ToolId,
    pub account: Option<String>,
    pub id: String,
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub started: Option<String>,
    pub ended: Option<String>,
    pub models: Vec<String>,
    pub parent_session: Option<String>,
    pub turn_count: u32,
    pub usage: TokenUsage,
    pub cost_usd: f64,
    pub source: PathBuf,
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub meta: SessionMeta,
    pub turns: Vec<Turn>,
}

pub trait SessionSource: Send + Sync {
    fn tool(&self) -> &'static str;
    fn roots(&self) -> Vec<PathBuf>;
    fn list(&self) -> Vec<SessionMeta>;
    fn load(&self, id: &str) -> Option<Session>;
    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        let _ = meta;
        None
    }
}
