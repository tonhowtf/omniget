//! Modelo neutro de sessão de agente de código (contrato do orquestrador).
//!
//! Todo parser de ferramenta (`parsers/<tool>.rs`) devolve estes tipos, e toda
//! tela de analytics (uso, sessões, retrospectiva, time) é uma visão sobre eles.
//! Mudar um campo aqui é mudar o contrato de todos os parsers: fale com o
//! orquestrador antes.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Id estável da ferramenta, o mesmo usado nas tabelas do agentkit
/// (`claude`, `codex`, `gemini`, `qwen`, `opencode`, `kilo`, `crush`, `cursor`,
/// `copilot`, `cline`, `goose`, `zed`, `droid`, `kimi`, `pi`, `amp`, `aider`,
/// `junie`, `auggie`, `grok`, `kiro`, `omniget`).
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

/// Nome canônico de tool, na nomenclatura do Claude (`Bash`, `Read`, `Edit`,
/// `Write`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, `Agent`, `Todo`, `Skill`,
/// `AskUser`, `Mcp`, `Other`). O nome cru da ferramenta fica em `name_raw`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name_canonical: String,
    pub name_raw: String,
    pub input: serde_json::Value,
    pub result: Option<String>,
    pub status: ToolStatus,
    pub ms: Option<u64>,
    /// Para `Agent`: o tipo de subagente (`subagent_type` do Claude, nome do
    /// agente no OpenCode/Codex etc.).
    pub subagent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    /// RFC 3339.
    pub ts: String,
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: TokenUsage,
    /// Custo registrado pela própria ferramenta, se ela grava (OpenCode, Goose,
    /// Pi, Aider, Crush, Cline). Quando `None`, o preço vem da tabela LiteLLM.
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    /// Id da mensagem na ferramenta (usado para dedupe).
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub tool: ToolId,
    /// Id da conta/perfil (config dir) quando a ferramenta tem mais de um.
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
    /// Arquivo ou banco de onde saiu (para reler e para "abrir pasta").
    pub source: PathBuf,
    /// mtime do arquivo em ms desde a época, para "ativa agora".
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub meta: SessionMeta,
    pub turns: Vec<Turn>,
}

/// O que cada parser implementa. Um parser nunca lê credencial: só arquivos de
/// sessão/log. Arquivo que não parseia vira `None`/vazio, nunca erro fatal.
pub trait SessionSource: Send + Sync {
    fn tool(&self) -> &'static str;
    /// Raízes onde a ferramenta guarda sessões nesta máquina (já resolvidas por
    /// SO e por variáveis de ambiente como `CLAUDE_CONFIG_DIR`, `CODEX_HOME`).
    fn roots(&self) -> Vec<PathBuf>;
    /// Lista barata: só metadados (pode ler cabeçalho/rodapé do arquivo).
    fn list(&self) -> Vec<SessionMeta>;
    /// Sessão completa por id.
    fn load(&self, id: &str) -> Option<Session>;
    /// Comando para retomar a sessão na própria ferramenta, quando existe.
    fn resume_command(&self, meta: &SessionMeta) -> Option<String> {
        let _ = meta;
        None
    }
}
