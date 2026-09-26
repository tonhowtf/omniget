//! `LlmManager`: the app-side owner of the LLM stack, kept in `AppState.llm`.
//! Owned by f2-llm-commands.
//!
//! `new()` stays a zero-argument constructor because `AppState` is built before
//! the Tauri app exists; everything that touches the disk is built on the first
//! command, behind a `OnceLock`.
//!
//! What it wires together (all of it from f2-llm-coordinator and
//! f2-llm-providers):
//!
//! ```text
//! Coordinator ── runtime  → CompositeRuntime → NativeRuntime → Provider per ProviderId
//!             │                            └→ CliRuntime → conta de assinatura (F4)
//!             ├─ broker   → ToolBroker(specs de mcp::tools(), McpToolExecutor)
//!             ├─ budget   → BudgetStore em <app_data>/llm/budget.json
//!             ├─ router   → Router(CompositeCapacity: AppCapacity para Native,
//!             │                     CliCapacitySource para Cli)
//!             └─ bus      → Bus → llm://tool-ask, llm://rerouted, telemetria
//! ```
//!
//! The conversation on disk belongs to the Coordinator
//! (`<app_data>/llm/conversations/<id>.jsonl`); this file only reads it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use omniget_core::core::llm::agent::{
    AgentDef, Candidate, CandidateRuntime, GrantMode, ModelPolicy, ToolGrant, ToolSource,
};
use omniget_core::core::llm::broker::{ToolBroker, ToolExecutor};
use omniget_core::core::llm::budget::BudgetStore;
use omniget_core::core::llm::cli_runtime::parse as cli_parse;
use omniget_core::core::llm::cli_runtime::{
    AccountStore, CliCapacity, CliCapacitySource, CliRuntime,
};
use omniget_core::core::llm::coordinator::{self, Coordinator};
use omniget_core::core::llm::error::{LlmError, ERR_LLM_MODEL, ERR_LLM_NET};
use omniget_core::core::llm::providers::fake::FakeProvider;
use omniget_core::core::llm::providers::{
    anthropic::AnthropicProvider, openai_compat::OpenAiCompat,
};
use omniget_core::core::llm::providers::{Provider, WireCapture};
use omniget_core::core::llm::roster_store::{self, RosterStore};
use omniget_core::core::llm::router::{self, Capacity, CapacityError, CapacitySource, Router};
use omniget_core::core::llm::runtime::{AgentRuntime, CompositeRuntime, NativeRuntime};
use omniget_core::core::llm::types::{
    Message, ModelRef, ProviderId, Role, ToolSpec, TurnEvent, Usage,
};
use omniget_core::core::omni::bus::{Bus, BusEvent};
use omniget_core::core::tools::ai_keys;

pub const ERR_NO_AGENT: &str = "ERR_LLM_NO_AGENT";
pub const ERR_NO_TURN: &str = "ERR_LLM_NO_TURN";
pub const ERR_NO_ASK: &str = "ERR_LLM_NO_ASK";
pub const ERR_STORE: &str = "ERR_LLM_STORE";
pub const ERR_NO_PROVIDER: &str = "ERR_LLM_NO_PROVIDER";

/// The `ProviderId` reserved for `FakeProvider`: tests and the wire probe.
pub const FAKE_PROVIDER: &str = "fake";

/// How many telemetry points are kept per agent before the oldest is dropped.
const MAX_HISTORY: usize = 240;
/// How many timeline events are kept per agent.
const MAX_EVENTS: usize = 40;

// ── Tool executor (the app half of the broker) ─────────────────────────

/// The broker's executor over the 37 tools the embedded MCP server already
/// exposes. The `AppHandle` arrives on the first command, so the broker can be
/// built before the Tauri app exists.
pub struct McpToolExecutor {
    app: Mutex<Option<tauri::AppHandle>>,
}

impl Default for McpToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl McpToolExecutor {
    pub fn new() -> Self {
        Self {
            app: Mutex::new(None),
        }
    }

    pub fn set_app(&self, app: tauri::AppHandle) {
        let mut slot = self.app.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(app);
        }
    }

    pub fn has_app(&self) -> bool {
        self.app.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

#[async_trait]
impl ToolExecutor for McpToolExecutor {
    async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError> {
        let app = {
            let slot = self.app.lock().unwrap_or_else(|e| e.into_inner());
            slot.clone()
        };
        let Some(app) = app else {
            return Err(LlmError::new(
                ERR_LLM_NET,
                "the app is not ready to run tools yet",
            ));
        };
        let help_turn = omniget_core::core::llm::code_tools::current_turn()
            .filter(|ctx| ctx.conversation.starts_with("help-"));
        let help_context = help_turn.is_some();
        let tool_call_id = omniget_core::core::llm::code_tools::current_tool_call();
        let mut input = input;
        if name == "download_enqueue" {
            if let Some(ctx) = omniget_core::core::llm::code_tools::current_turn()
                .filter(|ctx| ctx.conversation.starts_with("help-"))
            {
                let key = crate::commands::llm::help::download_intent(
                    &ctx.request,
                    input["url"].as_str().unwrap_or(""),
                    input["mode"].as_str().unwrap_or("video"),
                )
                .map_err(|e| LlmError::new(ERR_LLM_NET, e))?;
                input["idempotencyKey"] = serde_json::json!(key);
            }
        }
        let result = match crate::mcp::call(&app, name, input).await {
            Ok(value) => {
                let value = if help_context {
                    crate::commands::llm::help_redaction::redact_download_output(value)
                } else {
                    value
                };
                Ok(match value {
                    Value::String(s) => s,
                    other => other.to_string(),
                })
            }
            Err(message) => Err(LlmError::new(
                ERR_LLM_NET,
                if help_context {
                    crate::commands::llm::help_redaction::redact_text(&message)
                } else {
                    message
                },
            )),
        };
        // Native streams report calls but not their results. Help needs the real,
        // sanitized result to render actionable download cards without guessing.
        if let (Some(ctx), Some(call_id)) = (help_turn, tool_call_id) {
            use tauri::Emitter;
            let event = help_tool_result_event(call_id, &result);
            let _ = app.emit(
                "help://turn",
                serde_json::json!({"request_id":ctx.request,"event":event}),
            );
        }
        result
    }
}

fn help_tool_result_event(id: String, result: &Result<String, LlmError>) -> TurnEvent {
    let (content, is_error) = match result {
        Ok(content) => (content.clone(), false),
        Err(error) => (format!("{}: {}", error.code, error.message), true),
    };
    TurnEvent::ToolResult {
        id,
        content,
        is_error,
    }
}

/// Every internal tool as a `ToolSpec`, straight from the shared table
/// (`core::llm::tool_table`) the embedded MCP server also serves. Pure.
pub fn internal_specs() -> Vec<ToolSpec> {
    omniget_core::core::llm::tool_table::specs()
}

/// Old name of [`internal_specs`], kept for the call sites of Phase 2.
pub fn mcp_specs() -> Vec<ToolSpec> {
    internal_specs()
}

/// Source 2 of the broker: one external MCP server, driven through the
/// **same** `McpRegistry` singleton the `/llm/mcp` tab uses
/// (`commands::llm::mcp::registry()`). Two registries would mean two child
/// processes per server, so this never builds one of its own.
///
/// The broker hands over the namespaced name (`mcp:<server>:<tool>`), which is
/// exactly what a grant carries; splitting it here keeps that knowledge in one
/// place.
pub struct McpSourceExecutor;

#[async_trait]
impl ToolExecutor for McpSourceExecutor {
    async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError> {
        let Some((server, tool)) = split_mcp_name(name) else {
            return Err(LlmError::new(
                ERR_LLM_MODEL,
                format!("`{name}` is not an MCP tool name (mcp:<server>:<tool>)"),
            ));
        };
        let Some(registry) = crate::commands::llm::mcp::registry() else {
            return Err(LlmError::new(
                ERR_LLM_NET,
                "no app data dir for the MCP server list",
            ));
        };
        let client = registry.client_for(&server).await?;
        let value = client.call(&tool, input, None).await?;
        Ok(match value {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }
}

/// `mcp:<server>:<tool>` → (server, tool). The tool part may contain colons;
/// the server part may not (the core's id rule forbids them).
pub fn split_mcp_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("mcp:")?;
    let (server, tool) = rest.split_once(':')?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server.to_string(), tool.to_string()))
}

// ── Capacity (what the Router reads) ──────────────────────────────────

/// A snapshot of what each candidate can do, refreshed off the hot path.
/// `capacity()` is a map lookup, so picking a model never blocks on I/O.
#[derive(Default)]
pub struct AppCapacity {
    map: Mutex<HashMap<String, Capacity>>,
}

impl AppCapacity {
    pub fn set(&self, key: String, capacity: Capacity) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(key, capacity);
    }

    pub fn get(&self, key: &str) -> Option<Capacity> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl CapacitySource for AppCapacity {
    fn capacity(&self, candidate: &Candidate) -> Capacity {
        // Unknown candidate: available, because the router only rejects on
        // evidence and the snapshot may simply not have run yet.
        self.get(&router::candidate_key(candidate))
            .unwrap_or_default()
    }
}

/// Capacity of the whole roster: `Native` candidates come from the vault
/// snapshot, `Cli` candidates from the CLI usage windows (f4-cli-runtime).
///
/// It is also where the rotation switch lives. With rotation **off** (the
/// default, plan §10 D-2) a CLI account reports an *unknown* window, so the
/// router keeps the chain in the order the user wrote and never swaps on a
/// threshold. What still moves the chain is evidence of refusal: a `Rejected`
/// window (`quota_remaining == 0`), a disabled/missing account, or an
/// `ERR_CLI_RATE` on the turn — those go through untouched.
pub struct CompositeCapacity {
    native: Arc<AppCapacity>,
    cli: CliCapacitySource,
    /// `accounts-ui.json` is read at most once every `PREFS_TTL`, so a pick
    /// never turns into a file read per candidate.
    rotation: Mutex<Option<(std::time::Instant, bool)>>,
}

/// How long the rotation toggle is cached before `accounts-ui.json` is read
/// again. Short enough that flipping the switch takes effect on the next turn.
pub const PREFS_TTL: std::time::Duration = std::time::Duration::from_secs(2);

impl CompositeCapacity {
    pub fn new(native: Arc<AppCapacity>, cli: CliCapacitySource) -> Self {
        Self {
            native,
            cli,
            rotation: Mutex::new(None),
        }
    }

    /// Is automatic rotation between subscriptions on? Off by default.
    pub fn rotation_enabled(&self) -> bool {
        let mut slot = self.rotation.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, value)) = *slot {
            if at.elapsed() < PREFS_TTL {
                return value;
            }
        }
        let value = crate::commands::llm::accounts::load_prefs()
            .map(|p| p.rotation.enabled)
            .unwrap_or(false);
        *slot = Some((std::time::Instant::now(), value));
        value
    }
}

/// With rotation off, an account that is merely *busy* must not push the
/// router to the next link; an account that was refused still must. Pure, so
/// both halves of the rule are testable without files.
pub fn apply_rotation_switch(capacity: Capacity, rotation_enabled: bool) -> Capacity {
    if rotation_enabled {
        return capacity;
    }
    match capacity.quota_remaining {
        // Refused: keep the zero, the router skips this candidate.
        Some(q) if q <= 0.0 => capacity,
        // Anything else: hide the window so no threshold can rotate.
        _ => Capacity {
            quota_remaining: None,
            ..capacity
        },
    }
}

impl CapacitySource for CompositeCapacity {
    fn capacity(&self, candidate: &Candidate) -> Capacity {
        match &candidate.runtime {
            CandidateRuntime::Native { .. } => self.native.capacity(candidate),
            CandidateRuntime::Cli { .. } => {
                apply_rotation_switch(self.cli.capacity(candidate), self.rotation_enabled())
            }
        }
    }
}

/// Is there a usable key (or a keyless local server) for this provider?
/// Pure over the vault: no network.
pub fn provider_available(provider: &str) -> bool {
    if provider == FAKE_PROVIDER || matches!(provider, "ollama" | "lmstudio" | "llama-server") {
        return true;
    }
    ai_keys::list()
        .into_iter()
        .any(|k| (k.id == provider || k.kind == provider) && k.has_key)
}

// ── Telemetry (the contract f2-observatory-ui asked for) ──────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Streaming,
    WaitingTool,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPoint {
    pub t_ms: u64,
    pub tokens_out: u64,
    pub tokens_in: u64,
    pub tps: f32,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub t_ms: u64,
    /// turn_started | turn_finished | rerouted | error | budget
    pub kind: String,
    pub code: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTelemetry {
    pub agent_id: String,
    pub agent_name: String,
    pub provider: String,
    pub model: String,
    pub state: AgentState,
    pub turns: u64,
    pub errors: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub first_token_ms_p50: Option<u32>,
    pub first_token_ms_last: Option<u32>,
    pub tps_last: Option<f32>,
    pub cost_usd: f64,
    pub context_used_tokens: u64,
    /// 0 means unknown: the Observatory hides the Context Guardian.
    pub context_window: u64,
    pub context_estimated: bool,
    pub last_error_code: Option<String>,
    /// Set while a turn is alive; the Observatory enables Cancel with it.
    pub request_id: Option<String>,
    pub history: Vec<TelemetryPoint>,
    pub events: Vec<AgentEvent>,
    #[serde(skip)]
    first_token_samples: Vec<u32>,
    #[serde(skip)]
    turn_started_ms: u64,
}

impl AgentTelemetry {
    fn new(agent_id: &str, agent_name: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            agent_name: agent_name.to_string(),
            provider: String::new(),
            model: String::new(),
            state: AgentState::Idle,
            turns: 0,
            errors: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            first_token_ms_p50: None,
            first_token_ms_last: None,
            tps_last: None,
            cost_usd: 0.0,
            context_used_tokens: 0,
            context_window: 0,
            context_estimated: true,
            last_error_code: None,
            request_id: None,
            history: Vec::new(),
            events: Vec::new(),
            first_token_samples: Vec::new(),
            turn_started_ms: 0,
        }
    }

    fn push_event(&mut self, kind: &str, code: Option<String>, detail: Option<String>) {
        self.events.push(AgentEvent {
            t_ms: now_ms(),
            kind: kind.to_string(),
            code,
            detail,
        });
        if self.events.len() > MAX_EVENTS {
            let cut = self.events.len() - MAX_EVENTS;
            self.events.drain(0..cut);
        }
    }

    fn push_point(&mut self) {
        self.history.push(TelemetryPoint {
            t_ms: now_ms(),
            tokens_out: self.output_tokens,
            tokens_in: self.input_tokens,
            tps: self.tps_last.unwrap_or(0.0),
            cost_usd: self.cost_usd,
        });
        if self.history.len() > MAX_HISTORY {
            let cut = self.history.len() - MAX_HISTORY;
            self.history.drain(0..cut);
        }
    }
}

/// One quota line per saved key. No reported window exists for a BYOK account,
/// so `used` stays 0 with `source: "estimated"` and the UI labels it.
pub fn quotas_of_keys(chain: &[String]) -> Vec<QuotaStatus> {
    ai_keys::list()
        .into_iter()
        .filter(|k| k.has_key || k.kind == "ollama")
        .map(|k| {
            let active = chain.iter().any(|c| c.contains(&k.kind));
            QuotaStatus {
                id: k.id.clone(),
                label: if k.name.trim().is_empty() {
                    k.kind.clone()
                } else {
                    k.name.clone()
                },
                kind: "byok".to_string(),
                used: 0.0,
                resets_at_ms: None,
                source: "estimated".to_string(),
                exhausts_at_ms: None,
                active,
            }
        })
        .collect()
}

/// Median of the samples, which is what `first_token_ms_p50` means here.
pub fn p50(samples: &[u32]) -> Option<u32> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaStatus {
    pub id: String,
    pub label: String,
    /// cli | byok
    pub kind: String,
    pub used: f32,
    pub resets_at_ms: Option<u64>,
    /// reported | estimated
    pub source: String,
    pub exhausts_at_ms: Option<u64>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBucket {
    pub day: String,
    pub cost_usd: f64,
    pub calls: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub ts_ms: u64,
    pub agents: Vec<AgentTelemetry>,
    pub quotas: Vec<QuotaStatus>,
    pub cost_by_day: Vec<CostBucket>,
    /// Candidate keys in the order the router would try them.
    pub fallback_chain: Vec<String>,
}

#[derive(Default)]
struct Telemetry {
    agents: Mutex<HashMap<String, AgentTelemetry>>,
    fallback_chain: Mutex<Vec<String>>,
}

impl Telemetry {
    fn with<F: FnOnce(&mut AgentTelemetry)>(&self, agent_id: &str, name: &str, f: F) {
        let mut map = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentTelemetry::new(agent_id, name));
        f(entry);
    }

    fn agents(&self) -> Vec<AgentTelemetry> {
        let map = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<AgentTelemetry> = map.values().cloned().collect();
        out.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        out
    }

    fn set_chain(&self, chain: Vec<String>) {
        let mut slot = self
            .fallback_chain
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = chain;
    }

    fn chain(&self) -> Vec<String> {
        self.fallback_chain
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Conversations (read side; the Coordinator owns the writes) ────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationInfo {
    pub id: String,
    pub title: String,
    pub agent_id: Option<String>,
    pub messages: u32,
    pub updated_ms: u64,
}

/// Reads the JSONL the Coordinator writes. The only write left here is
/// `seed` — the bridge, which is stateless, replays its history into a
/// throwaway conversation so the real Coordinator sees a real history.
pub struct ConversationStore {
    dir: PathBuf,
}

impl ConversationStore {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    pub fn messages(&self, id: &str) -> Vec<Message> {
        coordinator::load_conversation(&self.dir, id).unwrap_or_default()
    }

    pub fn seed(&self, id: &str, agent: &str, message: &Message) {
        coordinator::append_message(&self.dir, id, Some(agent), message);
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = coordinator::conversation_path(&self.dir, id)
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("{ERR_STORE}: {e}"))?;
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<ConversationInfo> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<ConversationInfo> = entries
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "jsonl").unwrap_or(false))
            .filter_map(|e| {
                let id = e.path().file_stem()?.to_string_lossy().to_string();
                let updated_ms = e
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let messages = self.messages(&id);
                Some(ConversationInfo {
                    id,
                    title: title_of(&messages),
                    agent_id: self.last_agent(&e.path()),
                    messages: messages.len() as u32,
                    updated_ms,
                })
            })
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.updated_ms));
        out
    }

    /// Last `agent` field written into the file, so the chat list can show who
    /// answered without loading every record.
    fn last_agent(&self, path: &std::path::Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        text.lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<coordinator::ConversationRecord>(l).ok())
            .find_map(|r| r.agent)
    }
}

/// First line of the first user message, capped at 60 chars.
pub fn title_of(messages: &[Message]) -> String {
    let text = messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(message_text)
        .unwrap_or_default();
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return "…".to_string();
    }
    let mut s: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        s.push('…');
    }
    s
}

/// Flattens the text parts of a message.
pub fn message_text(message: &Message) -> String {
    use omniget_core::core::llm::types::ContentPart;
    message
        .parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Conversation ids come from the frontend and become file names.
pub fn sanitize_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "conversation".to_string()
    } else {
        cleaned
    }
}

// ── Manager ───────────────────────────────────────────────────────────

/// Roster adapter for `assist::external_config` (C02). Weak, so it never
/// keeps a manager alive; a dropped manager reads as "no roster" (fail closed).
struct RosterReader(std::sync::Weak<Inner>);

impl omniget_core::core::assist::external_config::BotRoster for RosterReader {
    fn get(&self, id: &str) -> Option<AgentDef> {
        self.0.upgrade().and_then(|i| i.roster.get(id))
    }
    fn apply_planned(
        &self,
        agent: AgentDef,
        before: Option<AgentDef>,
        source: AgentDef,
    ) -> Result<AgentDef, String> {
        let inner = self.0.upgrade().ok_or("EXTERNAL_ROSTER_UNAVAILABLE")?;
        inner
            .roster
            .apply_planned(agent, before, source)
            .map_err(|e| e.to_string())
    }
}

struct Inner {
    root: PathBuf,
    roster: RosterStore,
    conversations: ConversationStore,
    bus: Arc<Bus>,
    telemetry: Telemetry,
    native: Arc<NativeRuntime>,
    broker: Arc<ToolBroker>,
    budget: Arc<BudgetStore>,
    capacity: Arc<AppCapacity>,
    accounts: Arc<AccountStore>,
    cli_capacity: Arc<CliCapacity>,
    router: Arc<Router>,
    coordinator: Arc<Coordinator>,
    executor: Arc<McpToolExecutor>,
    turns: Mutex<HashMap<String, TurnHandle>>,
    model_overrides: Mutex<HashMap<String, ModelRef>>,
    bridge_openai: AtomicBool,
    telemetry_ticking: AtomicBool,
    bus_forwarding: AtomicBool,
    next_request: AtomicU64,
}

#[derive(Clone)]
struct TurnHandle {
    cancel: CancellationToken,
    agent_id: String,
}

/// Tool asks still waiting for an answer, so a window opened in the middle of
/// a turn (the pet) can catch up. Cleared entry by entry on answer and
/// wholesale whenever the broker says nothing is pending any more.
static PRUNE_RECEIPTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
static PENDING_ASKS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

/// The roster editor's skill checklist (`AgentDef::skills`) is a mirror of
/// the bot's bindings in `assist.db`; saving the agent brings the bindings in
/// line. A failure is logged, not fatal: the agent itself was saved.
fn sync_skill_bindings(bot: &str, skills: &[String]) {
    use omniget_core::core::assist::bots::{skills as bot_skills, BotEnv};
    if let Err(e) = bot_skills::sync_bindings(&BotEnv::global(), bot, skills) {
        tracing::warn!("[bots] skill bindings of {bot}: {e}");
    }
}

pub struct LlmManager {
    inner: OnceLock<Arc<Inner>>,
    root_override: Mutex<Option<PathBuf>>,
}

impl Default for LlmManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmManager {
    pub fn new() -> Self {
        Self {
            inner: OnceLock::new(),
            root_override: Mutex::new(None),
        }
    }

    /// Points the manager at a directory of its own. Only has an effect before
    /// the first use; the tests rely on it.
    pub fn set_root(&self, root: PathBuf) {
        if let Ok(mut g) = self.root_override.lock() {
            *g = Some(root);
        }
    }

    fn inner(&self) -> Arc<Inner> {
        if let Some(inner) = self.inner.get() {
            return inner.clone();
        }
        let root = self
            .root_override
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .or_else(roster_store::llm_dir)
            .unwrap_or_else(|| std::env::temp_dir().join("omniget-llm"));

        let _ = std::fs::create_dir_all(&root);
        omniget_core::core::llm::code_tools::set_store_file(root.join("workspaces.json"));
        omniget_core::core::llm::perm::set_store_file(root.join("permission-rules.json"));
        omniget_core::core::llm::snapshot::set_root(root.join("snapshots"));

        let bus = Arc::new(Bus::new());
        let executor = Arc::new(McpToolExecutor::new());
        // Source 1 of 3: the internal table. MCP servers (`register_mcp_tools`)
        // register themselves later, when the user enables them; the skills
        // (`register_skill_tools`) right after the assistant database opens.
        let broker = Arc::new(ToolBroker::new(
            internal_specs(),
            executor.clone(),
            bus.clone(),
        ));
        let budget = Arc::new(BudgetStore::at(root.join("budget.json")));
        let capacity = Arc::new(AppCapacity::default());
        // CLI subscriptions (f4-cli-runtime): the account book and the usage
        // windows are shared by the runtime and by the router's capacity.
        // `root` is `<app_data>/llm` in production, which is exactly where
        // `commands/llm/accounts.rs` keeps `accounts.json`, so this is the same
        // file the Accounts tab writes — and a directory of its own under test,
        // instead of the owner's real account book.
        let accounts = Arc::new(AccountStore::at(root.join("accounts.json")));
        let cli_capacity = Arc::new(CliCapacity::new());
        let router = Arc::new(Router::new(Arc::new(CompositeCapacity::new(
            capacity.clone(),
            CliCapacitySource::new(cli_capacity.clone(), accounts.clone()),
        ))));
        let native = Arc::new(NativeRuntime::new());
        // The FakeProvider is always registered: tests and the wire probe
        // address it as ProviderId("fake").
        native.register(
            FAKE_PROVIDER,
            Arc::new(FakeProvider::text(
                "This is the fake provider answering (ProviderId \"fake\").",
                8,
            )),
        );
        // Without `with_cli` every `RuntimeKind::Cli` agent answers ERR_LLM_MODEL.
        let cli = Arc::new(CliRuntime::new(accounts.clone(), cli_capacity.clone()));
        let runtime: Arc<dyn AgentRuntime> = Arc::new(
            CompositeRuntime::new(native.clone())
                .with_cli(cli)
                .with_acp(Arc::new(omniget_core::core::llm::acp::AcpRuntime::new(
                    broker.clone(),
                ))),
        );
        let coordinator = Arc::new(
            Coordinator::new(runtime, broker.clone(), budget.clone(), bus.clone())
                .with_router(router.clone())
                .with_dir(Some(root.join("conversations"))),
        );

        // Assistant subsystem: `assist.db` (bots, memory, reading, runs),
        // its tools as a broker source, and the per-turn hooks.
        {
            use omniget_core::core::assist;
            match assist::db::AssistDb::open(&root.join(assist::db::DB_FILE)) {
                Ok(db) => {
                    let db = Arc::new(db);
                    assist::db::set_global(db.clone());
                    // Conversation context (projectless/project) and the
                    // room-aware memory resolver live in `assist::groups`.
                    assist::groups::install(db);
                }
                Err(e) => tracing::error!("[assist] {e}"),
            }
            broker.register_source(
                assist::tools::ASSIST_SOURCE,
                assist::tools::all_specs(),
                Arc::new(assist::tools::AssistExecutor),
            );
            for hook in assist::augments() {
                coordinator.add_augment(hook);
            }
        }

        // Bot layer: the broker it checks "registered" against, and the skill
        // tools of whatever is installed (needs `assist.db`, opened above).
        omniget_core::core::assist::bots::set_broker(broker.clone());
        if let Err(e) = omniget_core::core::assist::bots::skills::projection().reproject() {
            tracing::warn!("[skills] projection: {e}");
        }

        let inner = Arc::new(Inner {
            roster: RosterStore::at(root.join("roster.json")),
            conversations: ConversationStore::at(root.join("conversations")),
            bus,
            telemetry: Telemetry::default(),
            native,
            broker,
            budget,
            capacity,
            accounts,
            cli_capacity,
            router,
            coordinator,
            executor,
            turns: Mutex::new(HashMap::new()),
            model_overrides: Mutex::new(read_overrides(&root.join("model-overrides.json"))),
            bridge_openai: AtomicBool::new(read_flag(&root.join("bridge-openai.json"))),
            telemetry_ticking: AtomicBool::new(false),
            bus_forwarding: AtomicBool::new(false),
            next_request: AtomicU64::new(1),
            root,
        });
        let _ = self.inner.set(inner.clone());
        let chosen = self.inner.get().cloned().unwrap_or(inner);
        // C02: external derived bots read/write the same real roster.
        omniget_core::core::assist::external_config::install_roster(Arc::new(RosterReader(
            Arc::downgrade(&chosen),
        )));
        chosen
    }

    pub fn bus(&self) -> Arc<Bus> {
        self.inner().bus.clone()
    }

    pub fn broker(&self) -> Arc<ToolBroker> {
        self.inner().broker.clone()
    }

    /// Source 2: one external MCP server. `specs` carry the bare names the
    /// server reported (`McpClient::tools()`); the broker namespaces them to
    /// `mcp:<server>:<tool>`. Calling it again for the same server replaces
    /// its tool list (a reconnect, a `tools/list` refresh). The calls go
    /// through [`McpSourceExecutor`], i.e. the registry the MCP tab owns.
    pub fn register_mcp_tools(&self, server: &str, specs: Vec<ToolSpec>) -> usize {
        self.register_mcp_tools_with(server, specs, Arc::new(McpSourceExecutor))
    }

    /// Same, with an executor of your own (tests, a fake server).
    pub fn register_mcp_tools_with(
        &self,
        server: &str,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        self.inner().broker.register_mcp(server, specs, executor)
    }

    /// Drops one MCP server's tools (disabled or removed in the UI).
    pub fn unregister_mcp_tools(&self, server: &str) -> usize {
        self.inner().broker.unregister_mcp(server)
    }

    /// Source 3: the installed skills, one provider-safe tool per installed
    /// version (`skill__<name>_<hash8>`, see `assist::bots::skills`). Called
    /// at boot and after every install, update, removal or repair; the
    /// source is replaced whole, so no tool outlives its skill.
    pub fn register_skill_tools(
        &self,
    ) -> Result<omniget_core::core::assist::bots::skills::ProjectionSummary, String> {
        let _ = self.inner();
        omniget_core::core::assist::bots::skills::projection().reproject()
    }

    pub fn budget(&self) -> Arc<BudgetStore> {
        self.inner().budget.clone()
    }

    pub fn router(&self) -> Arc<Router> {
        self.inner().router.clone()
    }

    pub fn coordinator(&self) -> Arc<Coordinator> {
        self.inner().coordinator.clone()
    }

    /// Hands the `AppHandle` to the tool executor. Idempotent; called by the
    /// first command and by the bridge.
    pub fn attach_app(&self, app: &tauri::AppHandle) {
        self.inner().executor.set_app(app.clone());
    }

    pub fn tools_ready(&self) -> bool {
        self.inner().executor.has_app()
    }

    // Providers -------------------------------------------------------

    /// The provider behind a `ProviderId`, built from the vault on first use
    /// and cached in the `NativeRuntime`. `ProviderId("fake")` always resolves,
    /// which is what the wire probe and the tests use.
    pub fn provider_for(&self, id: &ProviderId) -> Option<Arc<dyn Provider>> {
        let inner = self.inner();
        if let Some(found) = inner.native.get(id.as_str()) {
            return Some(found);
        }
        let built = build_provider(id)?;
        inner.native.register(id.as_str(), built.clone());
        Some(built)
    }

    /// The same client, but built with a `WireCapture` attached, for the wire
    /// probe. Deliberately NOT cached in the `NativeRuntime`: the chat must not
    /// pay for keeping the last body and the last raw response (256 KB) in
    /// memory on every turn.
    pub fn probe_provider_for(&self, id: &ProviderId) -> Option<Arc<dyn Provider>> {
        build_provider_with(id, true)
    }

    /// Registers a provider by hand (tests, and the CLI runtime of Phase 4).
    pub fn register_provider(&self, id: &str, provider: Arc<dyn Provider>) {
        self.inner().native.register(id, provider);
    }

    // Roster ----------------------------------------------------------

    pub fn roster(&self) -> Vec<AgentDef> {
        (*self.inner().roster.list()).clone()
    }

    /// `provider/model` the agent would run on now, for job records.
    pub fn model_label(&self, agent_id: &str) -> String {
        match self.agent(agent_id) {
            Some(a) => {
                let m = self.model_of(&a);
                format!("{}/{}", m.provider.as_str(), m.model)
            }
            None => String::new(),
        }
    }

    pub fn agent(&self, id: &str) -> Option<AgentDef> {
        self.inner().roster.get(id)
    }

    pub fn roster_create(&self, agent: AgentDef) -> Result<Vec<AgentDef>, String> {
        let (id, skills) = (agent.id.clone(), agent.skills.clone());
        self.inner()
            .roster
            .create(agent)
            .map_err(|e| e.to_string())?;
        sync_skill_bindings(&id, &skills);
        Ok(self.roster())
    }

    pub fn roster_update(&self, agent: AgentDef) -> Result<Vec<AgentDef>, String> {
        let (id, skills) = (agent.id.clone(), agent.skills.clone());
        self.inner()
            .roster
            .update(agent)
            .map_err(|e| e.to_string())?;
        sync_skill_bindings(&id, &skills);
        Ok(self.roster())
    }

    pub fn roster_apply_planned(
        &self,
        agent: AgentDef,
        before: Option<AgentDef>,
        source: AgentDef,
    ) -> Result<AgentDef, String> {
        self.inner()
            .roster
            .apply_planned(agent, before, source)
            .map_err(|e| e.to_string())
    }

    pub fn roster_delete(&self, id: &str) -> Result<Vec<AgentDef>, String> {
        self.inner().roster.delete(id).map_err(|e| e.to_string())?;
        // The bot's profile and skill bindings go with it; its traced reads
        // stay as the record of past runs.
        if let Ok(db) = omniget_core::core::assist::db::global() {
            if let Err(e) = omniget_core::core::assist::bots::profile::delete(&db, id) {
                tracing::warn!("[bots] deleting {id}: {e}");
            }
        }
        Ok(self.roster())
    }

    pub fn roster_apply_template(&self, template: &str) -> Result<Vec<AgentDef>, String> {
        let list = self
            .inner()
            .roster
            .apply_template(template)
            .map_err(|e| e.to_string())?;
        Ok((*list).clone())
    }

    // Conversations ---------------------------------------------------

    pub fn conversations(&self) -> Vec<ConversationInfo> {
        self.inner()
            .conversations
            .list()
            .into_iter()
            .filter(|c| !c.id.starts_with("help-"))
            .collect()
    }

    pub fn conversation(&self, id: &str) -> Vec<Message> {
        self.inner().conversations.messages(id)
    }

    pub fn conversation_delete(&self, id: &str) -> Result<(), String> {
        self.inner().conversations.delete(id)
    }

    /// Model override for a conversation, kept out of the JSONL on purpose:
    /// a note inside the history would be sent to the model on every turn.
    pub fn switch_model(&self, conversation_id: &str, model: ModelRef) -> Result<(), String> {
        let inner = self.inner();
        {
            let mut map = inner
                .model_overrides
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            map.insert(sanitize_id(conversation_id), model);
            write_overrides(&inner.root.join("model-overrides.json"), &map);
        }
        Ok(())
    }

    pub fn model_override(&self, conversation_id: &str) -> Option<ModelRef> {
        let inner = self.inner();
        let map = inner
            .model_overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.get(&sanitize_id(conversation_id)).cloned()
    }

    // Turns -----------------------------------------------------------

    /// Refreshes the capacity snapshot of one agent's chain. Vault-only (no
    /// network); the price lookup is spawned so it lands in the next turn.
    /// What the chat path does before a turn, for callers that drive the
    /// `Coordinator` themselves (the world's thinker).
    pub fn prepare_agent(&self, agent: &AgentDef) -> Result<(), String> {
        self.ensure_providers(agent)?;
        self.refresh_capacity(agent);
        Ok(())
    }

    pub fn refresh_capacity(&self, agent: &AgentDef) {
        let inner = self.inner();
        let candidates = chain_of(agent);
        let mut chain_keys = Vec::new();
        for candidate in &candidates {
            let key = router::candidate_key(candidate);
            chain_keys.push(key.clone());
            let provider = match &candidate.runtime {
                CandidateRuntime::Native { provider } => provider.as_str().to_string(),
                CandidateRuntime::Cli { account_id } => account_id.clone(),
            };
            let previous = inner.capacity.get(&key).unwrap_or_default();
            let budget_remaining_usd = inner.budget.remaining_usd(&agent.id, &agent.budget);
            inner.capacity.set(
                key.clone(),
                Capacity {
                    available: provider_available(&provider),
                    budget_remaining_usd,
                    // Kept from the previous snapshot; filled by the price task.
                    cost_per_1k: previous.cost_per_1k,
                    last_error: previous.last_error.clone(),
                    ..Capacity::default()
                },
            );
            // Price out of the hot path: it may hit the network the first time.
            let model = candidate.model.clone();
            let capacity = inner.capacity.clone();
            let key_for_task = key.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(price) = omniget_core::core::tools::pricing::price_for(&model).await {
                    if let Some(cost) = omniget_core::core::tools::pricing::cost(&price, 1000, 0) {
                        if let Some(mut current) = capacity.get(&key_for_task) {
                            current.cost_per_1k = Some(cost);
                            capacity.set(key_for_task, current);
                        }
                    }
                }
            });
        }
        inner.telemetry.set_chain(chain_keys);
    }

    /// Marks a candidate as failed, so the router cools it down and the
    /// capacity snapshot carries the reason into the next pick.
    pub fn note_capacity_error(&self, agent: &AgentDef, code: &str) {
        let inner = self.inner();
        for candidate in chain_of(agent) {
            let key = router::candidate_key(&candidate);
            if let Some(mut current) = inner.capacity.get(&key) {
                current.last_error = Some(CapacityError {
                    code: code.to_string(),
                    seconds_ago: 0,
                });
                inner.capacity.set(key, current);
            }
        }
    }

    /// Runs a turn through the Coordinator: tool loop, budget, reroute and
    /// conversation persistence all happen inside the returned stream.
    pub async fn turn_stream(
        &self,
        conversation_id: &str,
        agent_id: &str,
        input: &str,
    ) -> Result<(String, CancellationToken, BoxStream<'static, TurnEvent>), String> {
        let agent = self
            .agent(agent_id)
            .ok_or_else(|| format!("{ERR_NO_AGENT}: no agent {agent_id}"))?;
        self.turn_with_agent(conversation_id, agent, input, CancellationToken::new())
            .await
    }

    /// The job owns this token before the first provider handshake begins.
    pub async fn turn_stream_with_cancel(
        &self,
        conversation_id: &str,
        agent_id: &str,
        input: &str,
        cancel: CancellationToken,
    ) -> Result<(String, CancellationToken, BoxStream<'static, TurnEvent>), String> {
        if cancel.is_cancelled() {
            return Err("CANCELLED".into());
        }
        let agent = self
            .agent(agent_id)
            .ok_or_else(|| format!("{ERR_NO_AGENT}: no agent {agent_id}"))?;
        self.turn_with_agent(conversation_id, agent, input, cancel)
            .await
    }

    pub async fn help_turn_stream(
        &self,
        conversation_id: &str,
        source_agent_id: &str,
        input: &str,
    ) -> Result<(String, CancellationToken, BoxStream<'static, TurnEvent>), String> {
        if !conversation_id.starts_with("help-") {
            return Err("ERR_HELP_SESSION".into());
        }
        let mut agent = self.agent(source_agent_id).ok_or("ERR_HELP_CONNECTION")?;
        // Keep the real runtime/account/model, but never register another roster agent.
        agent.id = format!("help-{}", agent.id);
        agent.name = "Help".into();
        agent.role = omniget_core::core::llm::agent::AgentRole::Worker;
        // Localized like the other seeded prompts: the frontend pushes the text
        // for the active locale through sync_llm_prompts (see roster_store).
        agent.system_prompt = omniget_core::core::llm::roster_store::prompt_defaults().help;
        agent.skills.clear();
        agent.tools.retain(
            |g| !matches!(&g.source, ToolSource::Internal { name } if name.starts_with("help_")),
        );
        for name in [
            "help_docs_search",
            "help_docs_read",
            "help_setup_inspect",
            "help_connection_check",
            "help_diagnostic_run",
            "help_agent_plan",
            "help_agent_apply",
            "download_enqueue",
            "download_status",
            "download_cancel",
        ] {
            if !agent
                .tools
                .iter()
                .any(|g| matches!(&g.source, ToolSource::Internal { name: n } if n == name))
            {
                agent.tools.push(ToolGrant {
                    source: ToolSource::Internal { name: name.into() },
                    mode: if name.ends_with("apply")
                        || name == "download_enqueue"
                        || name == "download_cancel"
                    {
                        GrantMode::Ask
                    } else {
                        GrantMode::Auto
                    },
                });
            }
        }
        self.turn_with_agent(conversation_id, agent, input, CancellationToken::new())
            .await
    }

    async fn turn_with_agent(
        &self,
        conversation_id: &str,
        mut agent: AgentDef,
        input: &str,
        cancel: CancellationToken,
    ) -> Result<(String, CancellationToken, BoxStream<'static, TurnEvent>), String> {
        let inner = self.inner();
        let external = omniget_core::core::assist::authority::external(conversation_id);
        if external {
            omniget_core::core::assist::authority::check_agent(conversation_id, &agent)?;
        } else {
            // A derived bot (or an external room participant) never runs as
            // LOCAL_USER: direct chat, room, local mission, bridge, help (H1).
            let bot = agent.id.strip_prefix("help-").unwrap_or(&agent.id);
            omniget_core::core::assist::external_config::local_turn_allowed(conversation_id, bot)?;
            if let Some(model) = self.model_override(conversation_id) {
                agent.model = ModelPolicy::Fixed { model };
            }
        }
        self.ensure_providers(&agent)?;
        if !external {
            self.refresh_capacity(&agent);
        }

        let mut stream = inner.coordinator.run_turn(
            &sanitize_id(conversation_id),
            &agent,
            input,
            cancel.clone(),
        );

        // ONE id end to end. The Coordinator mints the request id and puts it
        // in `TurnEvent::Started` and in every `BusEvent::ToolAsk` of the turn,
        // so the app adopts that one instead of minting a second. Waiting for
        // `Started` costs nothing: the Coordinator emits it before the model
        // call, right after the budget gate. A turn cut before that (budget,
        // no candidate) never emits it, and only then a local id is used.
        let first = stream.next().await;
        let request_id = match &first {
            Some(TurnEvent::Started { request_id }) => request_id.clone(),
            _ => format!("r{}", inner.next_request.fetch_add(1, Ordering::SeqCst)),
        };
        // The event is put back, so the caller sees the stream whole.
        let stream: BoxStream<'static, TurnEvent> =
            futures::stream::iter(first).chain(stream).boxed();

        if let Ok(mut turns) = inner.turns.lock() {
            turns.insert(
                request_id.clone(),
                TurnHandle {
                    cancel: cancel.clone(),
                    agent_id: agent.id.clone(),
                },
            );
        }
        let model = self.model_of(&agent);
        inner.telemetry.with(&agent.id, &agent.name, |t| {
            t.turns += 1;
            t.state = AgentState::Streaming;
            t.request_id = Some(request_id.clone());
            t.provider = model.provider.as_str().to_string();
            t.model = model.model.clone();
            t.turn_started_ms = now_ms();
            t.push_event("turn_started", None, None);
        });
        Ok((request_id, cancel, stream))
    }

    /// The bridge is stateless: it ships the whole history every time. The
    /// history is replayed into a throwaway conversation so the real
    /// Coordinator sees a real conversation; the file is removed at the end.
    pub async fn turn_stateless(
        &self,
        agent_id: &str,
        mut messages: Vec<Message>,
    ) -> Result<
        (
            String,
            String,
            CancellationToken,
            BoxStream<'static, TurnEvent>,
        ),
        String,
    > {
        let inner = self.inner();
        if inner.roster.get(agent_id).is_none() {
            return Err(format!("{ERR_NO_AGENT}: no agent {agent_id}"));
        }
        let last = messages
            .iter()
            .rposition(|m| m.role == Role::User)
            .map(|i| messages.remove(i))
            .ok_or_else(|| "ERR_LLM_PARSE: no user message".to_string())?;
        let conversation_id = format!("bridge-{}", uuid::Uuid::new_v4().simple());
        for message in &messages {
            inner
                .conversations
                .seed(&conversation_id, agent_id, message);
        }
        let input = message_text(&last);
        let (request_id, cancel, stream) =
            self.turn_stream(&conversation_id, agent_id, &input).await?;
        Ok((request_id, conversation_id, cancel, stream))
    }

    /// Removes a throwaway bridge conversation.
    pub fn drop_conversation(&self, conversation_id: &str) {
        let _ = self.inner().conversations.delete(conversation_id);
    }

    /// Deletes `bridge-*.jsonl` left behind by a process that died mid-turn.
    /// Cheap (one `read_dir` over a folder the user never fills) and called
    /// once, by the first command that wires the app up. Returns how many went.
    pub fn sweep_bridge_conversations(&self) -> usize {
        let inner = self.inner();
        let Ok(entries) = std::fs::read_dir(inner.conversations.dir()) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|e| {
                let path = e.path();
                path.extension().map(|x| x == "jsonl").unwrap_or(false)
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("bridge-"))
                        .unwrap_or(false)
            })
            .filter(|e| std::fs::remove_file(e.path()).is_ok())
            .count()
    }

    /// The model an agent will actually ask for, after the router's first pick.
    pub fn model_of(&self, agent: &AgentDef) -> ModelRef {
        match &agent.model {
            ModelPolicy::Fixed { model } => model.clone(),
            // The agent declares what its turns are for (a coding agent prefers
            // a Codex account), so the model shown in the UI is the one the
            // coordinator's own `pick_for` would land on.
            ModelPolicy::Route { chain } => self
                .inner()
                .router
                .pick_for(chain, &[], router::TaskKind::of_agent(agent))
                .map(|route| route.model)
                .unwrap_or_else(|_| {
                    chain
                        .first()
                        .map(router::model_ref)
                        .unwrap_or_else(|| ModelRef {
                            provider: ProviderId::new("openai"),
                            model: "gpt-4o-mini".into(),
                        })
                }),
        }
    }

    /// Builds every provider the agent's chain can reach. At least one must
    /// resolve, otherwise the turn would fail inside the stream with a code the
    /// UI cannot act on.
    fn ensure_providers(&self, agent: &AgentDef) -> Result<(), String> {
        // A CLI agent (own protocol or ACP) brings its own model and login.
        if !matches!(
            agent.runtime,
            omniget_core::core::llm::agent::RuntimeKind::Native
        ) {
            return Ok(());
        }
        let mut any = false;
        for candidate in chain_of(agent) {
            if let CandidateRuntime::Native { provider } = &candidate.runtime {
                if self.provider_for(provider).is_some() {
                    any = true;
                }
            }
        }
        if any {
            Ok(())
        } else {
            Err(format!(
                "{ERR_NO_PROVIDER}: no key or local server for agent {}",
                agent.id
            ))
        }
    }

    /// Bookkeeping at the end of a turn.
    pub fn finish_turn(&self, request_id: &str, agent_id: &str) {
        let inner = self.inner();
        if let Ok(mut turns) = inner.turns.lock() {
            turns.remove(request_id);
        }
        inner.telemetry.with(agent_id, agent_id, |t| {
            if t.request_id.as_deref() == Some(request_id) {
                t.request_id = None;
            }
            if t.state != AgentState::Error {
                t.state = AgentState::Idle;
            }
            t.push_event("turn_finished", None, None);
            t.push_point();
        });
    }

    /// Folds one turn event into the telemetry.
    pub fn note_event(&self, agent_id: &str, event: &TurnEvent) {
        let inner = self.inner();
        inner.telemetry.with(agent_id, agent_id, |t| match event {
            TurnEvent::TextDelta { text } => {
                t.state = AgentState::Streaming;
                t.context_used_tokens += (text.chars().count() as u64) / 4;
            }
            TurnEvent::ToolCallStart { .. } => t.state = AgentState::WaitingTool,
            TurnEvent::Usage { usage } => fold_usage(t, usage),
            TurnEvent::PruneReceipt { .. } => {
                if let Ok(v) = serde_json::to_value(event) {
                    let mut last = PRUNE_RECEIPTS.lock().unwrap_or_else(|e| e.into_inner());
                    last.push(serde_json::json!({ "agent": agent_id, "receipt": v }));
                    let extra = last.len().saturating_sub(12);
                    last.drain(..extra);
                }
            }
            TurnEvent::Error { error } => {
                t.errors += 1;
                t.state = AgentState::Error;
                t.last_error_code = Some(error.code.to_string());
                t.push_event("error", Some(error.code.to_string()), None);
            }
            _ => {}
        });
    }

    pub fn cancel(&self, request_id: &str) -> Result<(), String> {
        let inner = self.inner();
        let handle = inner
            .turns
            .lock()
            .ok()
            .and_then(|mut turns| turns.remove(request_id))
            .ok_or_else(|| format!("{ERR_NO_TURN}: {request_id}"))?;
        handle.cancel.cancel();
        // Pending permission requests of this turn resolve as cancelled, not
        // denied, and a late answer can no longer apply to them.
        inner.broker.cancel_request(request_id);
        // A question of a turn that no longer exists must not wait out its
        // 120 s: it would sit in the jobs list as a ghost approval.
        let orphans: Vec<String> = {
            let mut asks = PENDING_ASKS.lock().unwrap_or_else(|e| e.into_inner());
            let (gone, keep): (Vec<_>, Vec<_>) = asks
                .drain(..)
                .partition(|a| a["request_id"].as_str() == Some(request_id));
            *asks = keep;
            gone.iter()
                .filter_map(|a| a["tool_call_id"].as_str().map(str::to_string))
                .collect()
        };
        for id in orphans {
            inner.broker.answer(&id, false);
        }
        inner
            .telemetry
            .with(&handle.agent_id, &handle.agent_id, |t| {
                t.request_id = None;
                t.state = AgentState::Idle;
            });
        Ok(())
    }

    pub fn active_turns(&self) -> usize {
        self.inner()
            .turns
            .lock()
            .map(|t| t.len())
            .unwrap_or_default()
    }

    /// Answers a `GrantMode::Ask` question. The broker keys on the tool call
    /// id; the request id is carried for the UI only.
    /// Asks raised and not answered yet (timed-out ones fall out as soon as
    /// the broker holds none).
    /// The newest prune receipts of this process (at most 12), for the
    /// settings card: measured numbers per model request.
    pub fn prune_receipts(&self) -> Vec<serde_json::Value> {
        PRUNE_RECEIPTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn pending_ask_list(&self) -> Vec<serde_json::Value> {
        let live = self.inner().broker.pending_count();
        let mut asks = PENDING_ASKS.lock().unwrap_or_else(|e| e.into_inner());
        if live == 0 {
            asks.clear();
        } else if asks.len() > live {
            let drop_n = asks.len() - live;
            asks.drain(..drop_n);
        }
        asks.clone()
    }

    pub fn answer_tool(
        &self,
        _request_id: &str,
        tool_call_id: &str,
        allow: bool,
        always: bool,
    ) -> Result<(), String> {
        use omniget_core::core::llm::broker::Answer;
        if let Ok(mut asks) = PENDING_ASKS.lock() {
            asks.retain(|a| a["tool_call_id"] != tool_call_id);
        }
        let answer = match (allow, always) {
            (false, _) => Answer::Deny,
            (true, true) => Answer::Always,
            (true, false) => Answer::Once,
        };
        if self.inner().broker.answer_with(tool_call_id, answer) {
            Ok(())
        } else {
            Err(format!("{ERR_NO_ASK}: {tool_call_id}"))
        }
    }

    pub fn pending_asks(&self) -> usize {
        self.inner().broker.pending_count()
    }

    // Telemetry -------------------------------------------------------

    pub fn telemetry(&self) -> TelemetrySnapshot {
        let inner = self.inner();
        TelemetrySnapshot {
            ts_ms: now_ms(),
            agents: inner.telemetry.agents(),
            quotas: self.quotas(),
            cost_by_day: self.cost_by_day(),
            fallback_chain: inner.telemetry.chain(),
        }
    }

    /// One quota per saved account. Without a reported window the share is an
    /// estimate over the agent's daily budget, and `source` says so.
    fn quotas(&self) -> Vec<QuotaStatus> {
        let inner = self.inner();
        let chain = inner.telemetry.chain();
        // CLI subscriptions first: they are the ones with a real window.
        let mut out: Vec<QuotaStatus> = inner
            .accounts
            .list()
            .iter()
            .map(|account| {
                let snapshot = inner.cli_capacity.get(&account.id);
                let window = snapshot.as_ref().and_then(|s| s.worst());
                QuotaStatus {
                    id: account.id.clone(),
                    label: if account.label.trim().is_empty() {
                        account.cli.as_str().to_string()
                    } else {
                        account.label.clone()
                    },
                    kind: "cli".to_string(),
                    used: window.map(|w| w.used).unwrap_or(0.0),
                    resets_at_ms: window
                        .and_then(|w| w.resets_at)
                        .map(|secs| secs as u64 * 1000),
                    source: match snapshot.as_ref().map(|s| s.source) {
                        Some(cli_parse::QuotaSource::Real) => "reported".to_string(),
                        _ => "estimated".to_string(),
                    },
                    exhausts_at_ms: None,
                    active: chain.iter().any(|c| c.contains(account.id.as_str())),
                }
            })
            .collect();
        out.extend(quotas_of_keys(&chain));
        out
    }

    /// The account book and the usage windows, for the Accounts tab and for
    /// whoever records a fresh window (`CliCapacity::record`).
    pub fn accounts(&self) -> Arc<AccountStore> {
        self.inner().accounts.clone()
    }

    pub fn cli_capacity(&self) -> Arc<CliCapacity> {
        self.inner().cli_capacity.clone()
    }

    /// Today's spend from the budget store. Older days are not kept there, so
    /// the Observatory falls back to `tool_usage_report` for the history.
    fn cost_by_day(&self) -> Vec<CostBucket> {
        let state = self.inner().budget.snapshot();
        let (usd, calls) = state
            .agents
            .values()
            .fold((0.0, 0u64), |(usd, calls), spend| {
                (usd + spend.usd, calls + spend.turns as u64)
            });
        if calls == 0 && usd == 0.0 {
            return Vec::new();
        }
        vec![CostBucket {
            day: state.day.clone(),
            cost_usd: usd,
            calls,
        }]
    }

    pub fn claim_telemetry_ticker(&self) -> bool {
        self.inner().telemetry_ticking.swap(true, Ordering::SeqCst)
    }

    pub fn release_telemetry_ticker(&self) {
        self.inner()
            .telemetry_ticking
            .store(false, Ordering::SeqCst);
    }

    /// Claims the bus forwarder, so only one task re-emits bus events as Tauri
    /// events for the whole process.
    pub fn claim_bus_forwarder(&self) -> bool {
        self.inner().bus_forwarding.swap(true, Ordering::SeqCst)
    }

    /// Folds a bus event into the telemetry (the Tauri re-emission lives in
    /// `commands/llm/mod.rs`).
    pub fn note_bus(&self, event: &BusEvent) {
        let inner = self.inner();
        match event {
            BusEvent::ToolAsk {
                agent,
                request_id,
                tool_call_id,
                tool,
                preview,
            } => {
                if let Ok(mut asks) = PENDING_ASKS.lock() {
                    asks.push(serde_json::json!({
                        "agent": agent,
                        "request_id": request_id,
                        "tool_call_id": tool_call_id,
                        "tool": tool,
                        "preview": preview,
                    }));
                }
                inner.telemetry.with(agent, agent, |t| {
                    t.state = AgentState::WaitingTool;
                })
            }
            BusEvent::ToolCalled { agent, ok, .. } => inner.telemetry.with(agent, agent, |t| {
                if !*ok {
                    t.errors += 1;
                }
                t.state = AgentState::Streaming;
            }),
            BusEvent::Rerouted { agent, to, why, .. } => inner.telemetry.with(agent, agent, |t| {
                t.push_event("rerouted", Some(why.clone()), Some(to.clone()));
            }),
            BusEvent::BudgetHit { agent } => inner.telemetry.with(agent, agent, |t| {
                t.push_event("budget", Some("ERR_LLM_BUDGET".into()), None);
            }),
            BusEvent::TurnEnded { agent, usage } => inner.telemetry.with(agent, agent, |t| {
                fold_usage(t, usage);
            }),
            _ => {}
        }
    }

    // Bridge flag -----------------------------------------------------

    pub fn bridge_openai_enabled(&self) -> bool {
        self.inner().bridge_openai.load(Ordering::SeqCst)
    }

    pub fn set_bridge_openai(&self, enabled: bool) -> bool {
        let inner = self.inner();
        inner.bridge_openai.store(enabled, Ordering::SeqCst);
        write_flag(&inner.root.join("bridge-openai.json"), enabled);
        enabled
    }
}

/// Usage folded into one agent's counters, including first-token latency.
fn fold_usage(t: &mut AgentTelemetry, usage: &Usage) {
    t.input_tokens += usage.input_tokens as u64;
    t.output_tokens += usage.output_tokens as u64;
    t.cache_read_tokens += usage.cache_read_tokens as u64;
    t.cache_write_tokens += usage.cache_write_tokens as u64;
    t.cost_usd += usage.cost_usd.unwrap_or(0.0);
    t.context_used_tokens = (usage.input_tokens + usage.output_tokens) as u64;
    t.context_estimated = false;
    if let Some(ms) = usage.first_token_ms {
        t.first_token_ms_last = Some(ms);
        t.first_token_samples.push(ms);
        if t.first_token_samples.len() > 64 {
            t.first_token_samples.remove(0);
        }
        t.first_token_ms_p50 = p50(&t.first_token_samples);
    }
    if usage.total_ms > 0 && usage.output_tokens > 0 {
        t.tps_last = Some(usage.output_tokens as f32 * 1000.0 / usage.total_ms as f32);
    }
    t.push_point();
}

/// An agent's candidate chain, with `Fixed` seen as a chain of one.
pub fn chain_of(agent: &AgentDef) -> Vec<Candidate> {
    match &agent.model {
        ModelPolicy::Fixed { model } => vec![Candidate {
            runtime: CandidateRuntime::Native {
                provider: model.provider.clone(),
            },
            model: model.model.clone(),
            max_cost_per_1k: None,
            min_context: 0,
        }],
        ModelPolicy::Route { chain } => chain.clone(),
    }
}

/// Builds the provider for a `ProviderId` out of the vault. No network: the key
/// comes from the secret store and the base URL from the `ai_keys` table.
pub fn build_provider(id: &ProviderId) -> Option<Arc<dyn Provider>> {
    build_provider_with(id, false)
}

/// `capture` attaches a `WireCapture`, which is what makes `wire_capture()`
/// answer `Some` and the probe able to read the body that went out. Only the
/// probe asks for it.
pub fn build_provider_with(id: &ProviderId, capture: bool) -> Option<Arc<dyn Provider>> {
    let provider = id.as_str();
    if provider == FAKE_PROVIDER {
        // The FakeProvider always captures; there is no client to configure.
        return Some(Arc::new(FakeProvider::text("fake provider", 4)));
    }
    // An explicit vault id pins the exact account; legacy kind ids still work.
    let views = ai_keys::list();
    let entry = views
        .iter()
        .find(|k| k.id == provider && k.has_key)
        .or_else(|| views.iter().find(|k| k.kind == provider && k.has_key))
        .and_then(|view| ai_keys::entry_with_secret(&view.id).ok());
    let kind_id = entry.as_ref().map(|e| e.kind.as_str()).unwrap_or(provider);
    let kind = ai_keys::kind_of(kind_id);
    let local = omniget_core::core::llm::local_servers::LocalKind::all()
        .into_iter()
        .find(|k| k.id() == provider);
    let (base_url, key) = match entry.as_ref() {
        Some(entry) => (
            ai_keys::app_base_url(&entry.kind, &entry.base_url),
            entry.key.clone(),
        ),
        None => (local?.openai_base(""), String::new()),
    };

    if kind.wire == "anthropic" {
        return AnthropicProvider::new(base_url, key).ok().map(|p| {
            let p = match capture {
                true => p.with_capture(Arc::new(WireCapture::default())),
                false => p,
            };
            Arc::new(p) as Arc<dyn Provider>
        });
    }
    OpenAiCompat::new(id.clone(), base_url, key).ok().map(|p| {
        let p = match capture {
            true => p.with_capture(Arc::new(WireCapture::default())),
            false => p,
        };
        Arc::new(p) as Arc<dyn Provider>
    })
}

/// Error for an unknown model on the bridge.
pub fn unknown_model_error(model: &str) -> LlmError {
    LlmError::new(ERR_LLM_MODEL, format!("unknown model {model}"))
}

/// Collects a turn stream into the final answer, for callers that do not
/// stream (the bridge without `stream: true`).
pub async fn collect_answer(
    manager: &LlmManager,
    agent_id: &str,
    mut stream: BoxStream<'static, TurnEvent>,
) -> Result<String, LlmError> {
    let mut answer = String::new();
    while let Some(event) = stream.next().await {
        manager.note_event(agent_id, &event);
        match event {
            TurnEvent::TextDelta { text } => answer.push_str(&text),
            TurnEvent::Error { error } => return Err(error),
            TurnEvent::Finished { .. } => break,
            _ => {}
        }
    }
    Ok(answer)
}

fn read_flag(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
        .unwrap_or(false)
}

fn write_flag(path: &std::path::Path, enabled: bool) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::json!({ "enabled": enabled }).to_string());
}

fn read_overrides(path: &std::path::Path) -> HashMap<String, ModelRef> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_overrides(path: &std::path::Path, map: &HashMap<String, ModelRef>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(map) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniget_core::core::llm::agent::{GrantMode, ToolGrant, ToolSource};

    fn temp_manager(tag: &str) -> LlmManager {
        let root =
            std::env::temp_dir().join(format!("omniget-llm-{}-{}", tag, uuid::Uuid::new_v4()));
        let m = LlmManager::new();
        m.set_root(root);
        m
    }

    /// An agent pinned to the fake provider, which always resolves.
    fn fake_agent(manager: &LlmManager) -> AgentDef {
        let mut agent = roster_store::default_roster().remove(0);
        agent.model = ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new(FAKE_PROVIDER),
                model: "fake".into(),
            },
        };
        manager.roster_update(agent.clone()).unwrap();
        agent
    }

    #[tokio::test]
    async fn help_session_stays_out_of_roster_and_chat_history() {
        let m = temp_manager("help-isolation");
        let agent = fake_agent(&m);
        let roster_before = serde_json::to_value(m.roster()).unwrap();
        let (id, _, stream) = m
            .help_turn_stream("help-isolated", &agent.id, "hello")
            .await
            .unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        m.finish_turn(&id, &format!("help-{}", agent.id));
        assert!(events
            .iter()
            .any(|e| matches!(e, TurnEvent::TextDelta { .. })));
        assert_eq!(serde_json::to_value(m.roster()).unwrap(), roster_before);
        assert!(!m.conversation("help-isolated").is_empty());
        assert!(!m.conversations().iter().any(|c| c.id == "help-isolated"));
        assert_eq!(m.active_turns(), 0);
    }

    #[tokio::test]
    async fn help_session_cancel_reaches_the_real_runtime_token() {
        let m = temp_manager("help-cancel");
        let agent = fake_agent(&m);
        let (id, token, stream) = m
            .help_turn_stream("help-cancelled", &agent.id, "hello")
            .await
            .unwrap();
        m.cancel(&id).unwrap();
        assert!(token.is_cancelled());
        let _: Vec<TurnEvent> = stream.collect().await;
        m.finish_turn(&id, &format!("help-{}", agent.id));
        assert_eq!(m.active_turns(), 0);
        assert!(!m.conversations().iter().any(|c| c.id == "help-cancelled"));
    }

    #[tokio::test]
    async fn help_rejects_a_normal_chat_session_id() {
        let m = temp_manager("help-prefix");
        let agent = fake_agent(&m);
        assert!(m
            .help_turn_stream("chat-existing", &agent.id, "hello")
            .await
            .is_err());
        assert_eq!(m.active_turns(), 0);
        assert!(m.conversation("chat-existing").is_empty());
    }

    #[test]
    fn help_tool_result_uses_real_call_id_and_wire_schema() {
        let result = Ok(serde_json::json!({"item":{"id":42},"outcome":"queued"}).to_string());
        let event =
            serde_json::to_value(help_tool_result_event("call-real".into(), &result)).unwrap();
        assert_eq!(event["type"], "tool_result");
        assert_eq!(event["id"], "call-real");
        assert_eq!(event["is_error"], false);
        let content: Value = serde_json::from_str(event["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["item"]["id"], 42);
        let failed = Err(LlmError::new(ERR_LLM_NET, "failed"));
        assert_eq!(
            serde_json::to_value(help_tool_result_event("call-error".into(), &failed)).unwrap()
                ["is_error"],
            true
        );
    }

    #[test]
    fn sanitize_id_keeps_slugs_and_rejects_traversal() {
        assert_eq!(sanitize_id("abc-1_2"), "abc-1_2");
        assert_eq!(sanitize_id("../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitize_id(""), "conversation");
    }

    #[test]
    fn the_fake_provider_always_resolves() {
        let m = temp_manager("fake");
        assert!(m.provider_for(&ProviderId::new(FAKE_PROVIDER)).is_some());
    }

    #[test]
    fn a_provider_without_a_key_does_not_resolve() {
        let m = temp_manager("nokey");
        // "custom" has no saved account in a clean test environment.
        assert!(m
            .provider_for(&ProviderId::new("no-such-provider"))
            .is_none());
    }

    #[test]
    fn only_the_probe_client_carries_a_wire_capture() {
        let m = temp_manager("capture");
        // Ollama needs no key, so both paths build a real client here.
        let id = ProviderId::new("ollama");
        let chat = m.provider_for(&id).expect("ollama client");
        assert!(
            chat.wire_capture().is_none(),
            "the chat client must not pay for the capture"
        );
        let probe = m.probe_provider_for(&id).expect("ollama probe client");
        assert!(
            probe.wire_capture().is_some(),
            "the probe client must capture the body"
        );
        // And it is a client of its own: the cached one stays uncaptured.
        assert!(m.provider_for(&id).unwrap().wire_capture().is_none());
    }

    #[test]
    fn provider_available_is_true_for_fake_and_ollama() {
        assert!(provider_available(FAKE_PROVIDER));
        assert!(provider_available("ollama"));
    }

    #[test]
    fn model_override_round_trips_outside_the_conversation() {
        let m = temp_manager("model");
        assert!(m.model_override("c1").is_none());
        m.switch_model(
            "c1",
            ModelRef {
                provider: ProviderId::new("openrouter"),
                model: "z-ai/glm-4.6".into(),
            },
        )
        .unwrap();
        let got = m.model_override("c1").unwrap();
        assert_eq!(got.provider.as_str(), "openrouter");
        assert_eq!(got.model, "z-ai/glm-4.6");
        // The conversation itself stays clean: no note was written into it.
        assert!(m.conversation("c1").is_empty());
    }

    #[test]
    fn bridge_flag_defaults_off_and_persists() {
        let m = temp_manager("flag");
        assert!(!m.bridge_openai_enabled());
        m.set_bridge_openai(true);
        assert!(m.bridge_openai_enabled());
        assert!(read_flag(&m.inner().root.join("bridge-openai.json")));
    }

    #[test]
    fn chain_of_sees_a_fixed_model_as_a_chain_of_one() {
        let mut agent = roster_store::default_roster().remove(0);
        agent.model = ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("openai"),
                model: "gpt-4o-mini".into(),
            },
        };
        let chain = chain_of(&agent);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].model, "gpt-4o-mini");
    }

    #[test]
    fn rotation_off_hides_a_busy_window_but_never_a_refusal() {
        // Busy account (30 % left): with rotation off the router must not see
        // a window at all, so no threshold can move the chain.
        let busy = Capacity {
            quota_remaining: Some(0.3),
            ..Capacity::default()
        };
        assert_eq!(
            apply_rotation_switch(busy.clone(), false).quota_remaining,
            None
        );
        // With rotation on it goes through untouched.
        assert_eq!(apply_rotation_switch(busy, true).quota_remaining, Some(0.3));
        // Refused: the zero survives either way, so the router skips it.
        let refused = Capacity {
            quota_remaining: Some(0.0),
            ..Capacity::default()
        };
        assert_eq!(
            apply_rotation_switch(refused.clone(), false).quota_remaining,
            Some(0.0)
        );
        assert_eq!(
            apply_rotation_switch(refused, true).quota_remaining,
            Some(0.0)
        );
        // An unavailable account stays unavailable.
        assert!(!apply_rotation_switch(Capacity::unavailable(), false).available);
    }

    #[tokio::test]
    async fn a_cli_agent_reaches_the_cli_runtime_instead_of_err_llm_model() {
        use omniget_core::core::llm::agent::RuntimeKind;
        let m = temp_manager("cli");
        let mut agent = fake_agent(&m);
        agent.runtime = RuntimeKind::Cli {
            cli: "claude".into(),
            account: "no-such-account".into(),
        };
        m.roster_update(agent.clone()).unwrap();

        let (request_id, _cancel, stream) = m
            .turn_stream("c-cli", &agent.id, "oi")
            .await
            .expect("a CLI agent must start a turn now that with_cli is wired");
        let events: Vec<TurnEvent> = stream.collect().await;
        m.finish_turn(&request_id, &agent.id);

        // The account does not exist, so the turn fails — but with a CLI code,
        // which proves the CliRuntime took it. Before `with_cli` the answer was
        // ERR_LLM_MODEL "no CLI runtime for ... (Phase 4)".
        let codes: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::Error { error } => Some(error.code.to_string()),
                _ => None,
            })
            .collect();
        assert!(
            codes.iter().any(|c| c.starts_with("ERR_CLI")),
            "expected an ERR_CLI_* code from the CliRuntime, got {codes:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                TurnEvent::Error { error } if error.message.contains("Phase 4")
            )),
            "the CompositeRuntime still has no CLI half"
        );
    }

    #[test]
    fn cli_accounts_show_up_in_the_telemetry_quotas() {
        use omniget_core::core::llm::cli_runtime::accounts::{CliAccount, CliKind};
        use omniget_core::core::llm::cli_runtime::parse::{
            QuotaSource, RateSnapshot, RateStatus, RateWindow,
        };
        let m = temp_manager("quotas");
        m.accounts()
            .create(CliAccount {
                id: "acc-1".into(),
                cli: CliKind::Claude,
                config_dir: std::env::temp_dir().join("omniget-acc-1"),
                label: "Max pessoal".into(),
                disabled: false,
                sandbox: Default::default(),
            })
            .expect("account created");
        m.cli_capacity().record(
            "acc-1",
            RateSnapshot {
                status: RateStatus::Allowed,
                window_5h: Some(RateWindow {
                    used: 0.42,
                    resets_at: Some(1_700_000_000),
                }),
                window_7d: None,
                source: QuotaSource::Real,
            },
        );
        let quota = m
            .telemetry()
            .quotas
            .into_iter()
            .find(|q| q.id == "acc-1")
            .expect("the CLI account must have a quota line");
        assert_eq!(quota.kind, "cli");
        assert_eq!(quota.label, "Max pessoal");
        assert!((quota.used - 0.42).abs() < 1e-6, "used = {}", quota.used);
        assert_eq!(quota.resets_at_ms, Some(1_700_000_000_000));
        assert_eq!(quota.source, "reported", "a real window is not an estimate");
    }

    /// End to end through the real manager: a coding agent whose chain has a
    /// Claude account first and a Codex account second lands on the Codex one,
    /// and a non-coding agent with the very same chain does not.
    #[test]
    fn a_code_agent_prefers_a_codex_account_unless_it_is_rate_limited() {
        use omniget_core::core::llm::agent::AgentRole;
        use omniget_core::core::llm::cli_runtime::accounts::{CliAccount, CliKind, SandboxMode};
        use omniget_core::core::llm::cli_runtime::parse::{
            QuotaSource, RateSnapshot, RateStatus, RateWindow,
        };
        let m = temp_manager("prefer-codex");
        for (id, cli) in [("claude-1", CliKind::Claude), ("codex-1", CliKind::Codex)] {
            m.accounts()
                .create(CliAccount {
                    id: id.into(),
                    cli,
                    config_dir: std::env::temp_dir().join(format!("omniget-{id}")),
                    label: id.into(),
                    disabled: false,
                    sandbox: SandboxMode::default(),
                })
                .expect("account created");
        }
        let chain = vec![
            Candidate {
                runtime: CandidateRuntime::Cli {
                    account_id: "claude-1".into(),
                },
                model: "sonnet".into(),
                max_cost_per_1k: None,
                min_context: 0,
            },
            Candidate {
                runtime: CandidateRuntime::Cli {
                    account_id: "codex-1".into(),
                },
                model: "gpt-5-codex".into(),
                max_cost_per_1k: None,
                min_context: 0,
            },
        ];
        let mut agent = roster_store::default_roster().remove(0);
        agent.model = ModelPolicy::Route {
            chain: chain.clone(),
        };

        // Declared as a coding agent: Codex wins even from second place.
        agent.role = AgentRole::Custom("code".into());
        assert_eq!(m.model_of(&agent).model, "gpt-5-codex");

        // The same chain, an agent that never declared code: chain order.
        agent.role = AgentRole::Coordinator;
        agent.id = "chief-of-staff".into();
        assert_eq!(m.model_of(&agent).model, "sonnet");

        // Codex out of window: the preference is gone, without any switch.
        agent.role = AgentRole::Custom("code".into());
        m.cli_capacity().record(
            "codex-1",
            RateSnapshot {
                status: RateStatus::Rejected,
                window_5h: Some(RateWindow {
                    used: 1.0,
                    resets_at: None,
                }),
                window_7d: None,
                source: QuotaSource::Real,
            },
        );
        assert_eq!(m.model_of(&agent).model, "sonnet");
    }

    #[test]
    fn the_capacity_snapshot_marks_a_keyless_provider_unavailable() {
        let m = temp_manager("capacity");
        let mut agent = roster_store::default_roster().remove(0);
        agent.model = ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("no-such-provider"),
                model: "m".into(),
            },
        };
        m.refresh_capacity(&agent);
        let chain = m.inner().telemetry.chain();
        assert_eq!(chain.len(), 1);
        let capacity = m.inner().capacity.get(&chain[0]).unwrap();
        assert!(!capacity.available);
    }

    #[test]
    fn p50_is_the_median_sample() {
        assert_eq!(p50(&[]), None);
        assert_eq!(p50(&[10]), Some(10));
        assert_eq!(p50(&[30, 10, 20]), Some(20));
    }

    #[test]
    fn mcp_specs_are_not_empty_and_carry_schemas() {
        let specs = mcp_specs();
        assert!(specs.len() >= 30, "only {} specs", specs.len());
        assert!(specs.iter().all(|s| s.input_schema.is_object()));
    }

    #[tokio::test]
    async fn a_turn_streams_through_the_coordinator_and_persists() {
        let m = temp_manager("turn");
        let agent = fake_agent(&m);
        let (request_id, _cancel, stream) = match m.turn_stream("c1", &agent.id, "oi").await {
            Ok(t) => t,
            Err(e) => panic!("turn failed: {e}"),
        };
        assert_eq!(m.active_turns(), 1);
        let answer = collect_answer(&m, &agent.id, stream).await.unwrap();
        assert!(!answer.is_empty(), "the fake provider said nothing");
        m.finish_turn(&request_id, &agent.id);
        assert_eq!(m.active_turns(), 0);
        // The Coordinator is the one that wrote the conversation.
        let messages = m.conversation("c1");
        assert!(
            messages.len() >= 2,
            "coordinator did not persist the turn: {messages:?}"
        );
        assert!(m.conversations().iter().any(|c| c.id == "c1"));
    }

    #[tokio::test]
    async fn the_request_id_is_the_one_the_coordinator_minted() {
        let m = temp_manager("oneid");
        let agent = fake_agent(&m);
        let (request_id, _cancel, mut stream) = m.turn_stream("c1", &agent.id, "oi").await.unwrap();
        // The stream still starts with `Started`, and it carries that same id:
        // nothing downstream has to translate between two of them.
        match stream.next().await {
            Some(TurnEvent::Started {
                request_id: from_stream,
            }) => {
                assert_eq!(from_stream, request_id);
            }
            other => panic!("the turn must start with Started, got {other:?}"),
        }
        assert!(
            !request_id.starts_with('r') || request_id.len() > 8,
            "the local fallback id was used instead of the Coordinator's: {request_id}"
        );
        let _ = collect_answer(&m, &agent.id, stream).await;
        m.finish_turn(&request_id, &agent.id);
    }

    #[tokio::test]
    async fn an_unknown_agent_never_starts_a_turn() {
        let m = temp_manager("noagent");
        let err = match m.turn_stream("c1", "ghost", "oi").await {
            Ok(_) => panic!("an unknown agent must not start a turn"),
            Err(e) => e,
        };
        assert!(err.starts_with(ERR_NO_AGENT), "{err}");
    }

    #[tokio::test]
    async fn an_agent_without_any_provider_fails_before_streaming() {
        let m = temp_manager("noprovider");
        let mut agent = roster_store::default_roster().remove(0);
        agent.model = ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("no-such-provider"),
                model: "m".into(),
            },
        };
        m.roster_update(agent.clone()).unwrap();
        let err = match m.turn_stream("c1", &agent.id, "oi").await {
            Ok(_) => panic!("a keyless provider must not start a turn"),
            Err(e) => e,
        };
        assert!(err.starts_with(ERR_NO_PROVIDER), "{err}");
    }

    #[tokio::test]
    async fn the_bridge_turn_replays_history_and_cleans_up() {
        let m = temp_manager("stateless");
        let agent = fake_agent(&m);
        let messages = vec![
            Message::text(Role::User, "primeira"),
            Message::text(Role::Assistant, "resposta"),
            Message::text(Role::User, "segunda"),
        ];
        let (request_id, conversation_id, _cancel, stream) =
            m.turn_stateless(&agent.id, messages).await.unwrap();
        let answer = collect_answer(&m, &agent.id, stream).await.unwrap();
        assert!(!answer.is_empty());
        m.finish_turn(&request_id, &agent.id);
        // The replayed history was seen by the Coordinator…
        assert!(m.conversation(&conversation_id).len() >= 3);
        // …and the throwaway file goes away.
        m.drop_conversation(&conversation_id);
        assert!(m.conversation(&conversation_id).is_empty());
    }

    #[test]
    fn the_sweep_only_takes_orphan_bridge_files() {
        let m = temp_manager("sweep");
        let dir = m.inner().conversations.dir().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bridge-abc.jsonl"), b"").unwrap();
        std::fs::write(dir.join("bridge-def.jsonl"), b"").unwrap();
        std::fs::write(dir.join("c1.jsonl"), b"").unwrap();
        assert_eq!(m.sweep_bridge_conversations(), 2);
        assert!(dir.join("c1.jsonl").exists(), "a real chat must survive");
        assert_eq!(m.sweep_bridge_conversations(), 0);
    }

    #[tokio::test]
    async fn cancel_of_an_unknown_request_fails() {
        let m = temp_manager("cancel");
        assert!(m.cancel("nope").unwrap_err().starts_with(ERR_NO_TURN));
    }

    #[tokio::test]
    async fn tool_answer_goes_to_the_broker() {
        let m = temp_manager("ask");
        // Nothing pending: the broker refuses and the command says so.
        let err = m.answer_tool("r1", "call-1", true, false).unwrap_err();
        assert!(err.starts_with(ERR_NO_ASK), "{err}");
        assert_eq!(m.pending_asks(), 0);
    }

    #[tokio::test]
    async fn telemetry_carries_the_observatory_fields() {
        let m = temp_manager("telemetry");
        let agent = fake_agent(&m);
        let (request_id, _cancel, stream) = m.turn_stream("c1", &agent.id, "oi").await.unwrap();
        let snapshot = m.telemetry();
        assert!(snapshot.ts_ms > 0);
        let live = snapshot
            .agents
            .iter()
            .find(|a| a.agent_id == agent.id)
            .expect("agent in telemetry");
        assert_eq!(live.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(live.provider, FAKE_PROVIDER);
        assert!(matches!(live.state, AgentState::Streaming));
        assert_eq!(snapshot.fallback_chain.len(), 1);
        let _ = collect_answer(&m, &agent.id, stream).await;
        m.finish_turn(&request_id, &agent.id);
        let after = m.telemetry();
        let done = after
            .agents
            .iter()
            .find(|a| a.agent_id == agent.id)
            .unwrap();
        assert!(done.request_id.is_none());
        assert!(done.turns >= 1);
        assert!(done.output_tokens > 0, "usage was not folded in");
        assert!(done.events.iter().any(|e| e.kind == "turn_finished"));
    }

    #[test]
    fn note_bus_records_a_reroute_in_the_timeline() {
        let m = temp_manager("bus");
        m.note_bus(&BusEvent::Rerouted {
            agent: "omni".into(),
            from: "native:openai:gpt-4o-mini".into(),
            to: "native:ollama:qwen3:0.6b".into(),
            why: "ERR_LLM_RATE".into(),
        });
        let snapshot = m.telemetry();
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.agent_id == "omni")
            .unwrap();
        let event = agent.events.iter().find(|e| e.kind == "rerouted").unwrap();
        assert_eq!(event.code.as_deref(), Some("ERR_LLM_RATE"));
        assert_eq!(event.detail.as_deref(), Some("native:ollama:qwen3:0.6b"));
    }

    #[test]
    fn grants_are_the_broker_s_business_now() {
        // The broker owns grant matching; this only checks the key shape the
        // roster writes, so a UI change cannot silently widen a grant.
        let grant = ToolGrant {
            source: ToolSource::Mcp {
                server: "omniget".into(),
                tool: "download_url".into(),
            },
            mode: GrantMode::Ask,
        };
        assert_eq!(
            omniget_core::core::llm::broker::grant_key(&grant.source),
            "mcp:omniget:download_url"
        );
        // And the executor takes that exact key apart again, so the grant, the
        // spec and the server call can never mean three different things.
        assert_eq!(
            crate::llm_manager::split_mcp_name("mcp:omniget:download_url"),
            Some(("omniget".to_string(), "download_url".to_string()))
        );
        // A tool name with a colon in it still belongs to its server.
        assert_eq!(
            crate::llm_manager::split_mcp_name("mcp:files:read:raw"),
            Some(("files".to_string(), "read:raw".to_string()))
        );
        for bad in [
            "download_url",
            "skill:pdf",
            "mcp:",
            "mcp:files:",
            "mcp::read",
        ] {
            assert_eq!(crate::llm_manager::split_mcp_name(bad), None, "{bad}");
        }
    }
}
