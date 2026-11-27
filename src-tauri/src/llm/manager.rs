use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use omniget_core::core::llm::agent::{AgentDef, Candidate, CandidateRuntime, ModelPolicy};
use omniget_core::core::llm::broker::{ToolBroker, ToolExecutor};
use omniget_core::core::llm::budget::BudgetStore;
use omniget_core::core::llm::cli_runtime::parse as cli_parse;
use omniget_core::core::llm::cli_runtime::{
    AccountStore, CliCapacity, CliCapacitySource, CliRuntime,
};
use omniget_core::core::llm::coordinator::{self, Coordinator};
use omniget_core::core::llm::error::{LlmError, ERR_LLM_NET};
use omniget_core::core::llm::providers::fake::FakeProvider;
use omniget_core::core::llm::providers::{
    anthropic::AnthropicProvider, openai_compat::OpenAiCompat,
};
use omniget_core::core::llm::providers::{Provider, WireCapture};
use omniget_core::core::llm::roster_store::{self, RosterStore};
use omniget_core::core::llm::router::{self, Capacity, CapacitySource, Router};
use omniget_core::core::llm::runtime::{AgentRuntime, CompositeRuntime, NativeRuntime};
use omniget_core::core::llm::types::{ModelRef, ProviderId, ToolSpec, TurnEvent, Usage};
use omniget_core::core::omni::bus::{Bus, BusEvent};
use omniget_core::core::tools::ai_keys;

pub const ERR_NO_AGENT: &str = "ERR_LLM_NO_AGENT";
pub const ERR_NO_TURN: &str = "ERR_LLM_NO_TURN";
pub const ERR_NO_ASK: &str = "ERR_NO_ASK";
pub const ERR_STORE: &str = "ERR_LLM_STORE";
pub const ERR_NO_PROVIDER: &str = "ERR_LLM_NO_PROVIDER";

pub const FAKE_PROVIDER: &str = "fake";

const MAX_HISTORY: usize = 240;
const MAX_EVENTS: usize = 40;

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
        let result = match crate::mcp::call(&app, name, input).await {
            Ok(value) => Ok(match value {
                Value::String(s) => s,
                other => other.to_string(),
            }),
            Err(message) => Err(LlmError::new(ERR_LLM_NET, message)),
        };
        result
    }
}

pub fn internal_specs() -> Vec<ToolSpec> {
    omniget_core::core::llm::tool_table::specs()
}

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
}

impl CapacitySource for AppCapacity {
    fn capacity(&self, candidate: &Candidate) -> Capacity {
        self.get(&router::candidate_key(candidate))
            .unwrap_or_default()
    }
}

pub struct CompositeCapacity {
    native: Arc<AppCapacity>,
    cli: CliCapacitySource,
    rotation: Mutex<Option<(std::time::Instant, bool)>>,
}

pub const PREFS_TTL: std::time::Duration = std::time::Duration::from_secs(2);

impl CompositeCapacity {
    pub fn new(native: Arc<AppCapacity>, cli: CliCapacitySource) -> Self {
        Self {
            native,
            cli,
            rotation: Mutex::new(None),
        }
    }

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

pub fn apply_rotation_switch(capacity: Capacity, rotation_enabled: bool) -> Capacity {
    if rotation_enabled {
        return capacity;
    }
    match capacity.quota_remaining {
        Some(q) if q <= 0.0 => capacity,
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

pub fn provider_available(provider: &str) -> bool {
    if provider == FAKE_PROVIDER || matches!(provider, "ollama" | "lmstudio" | "llama-server") {
        return true;
    }
    ai_keys::list()
        .into_iter()
        .any(|k| (k.id == provider || k.kind == provider) && k.has_key)
}

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
    pub context_window: u64,
    pub context_estimated: bool,
    pub last_error_code: Option<String>,
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
    pub kind: String,
    pub used: f32,
    pub resets_at_ms: Option<u64>,
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

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = coordinator::conversation_path(&self.dir, id)
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("{ERR_STORE}: {e}"))?;
        }
        Ok(())
    }
}

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
    cli: Arc<CliRuntime>,
    router: Arc<Router>,
    coordinator: Arc<Coordinator>,
    executor: Arc<McpToolExecutor>,
    turns: Mutex<HashMap<String, TurnHandle>>,
    model_overrides: Mutex<HashMap<String, ModelRef>>,
    telemetry_ticking: AtomicBool,
    bus_forwarding: AtomicBool,
    next_request: AtomicU64,
}

#[derive(Clone)]
struct TurnHandle {
    cancel: CancellationToken,
    agent_id: String,
}

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
        omniget_core::core::llm::run_mode::set_store_file(root.join("run-modes.json"));
        omniget_core::core::llm::snapshot::set_root(root.join("snapshots"));

        let bus = Arc::new(Bus::new());
        let executor = Arc::new(McpToolExecutor::new());
        let broker = Arc::new(ToolBroker::new(
            internal_specs(),
            executor.clone(),
            bus.clone(),
        ));
        let budget = Arc::new(BudgetStore::at(root.join("budget.json")));
        let capacity = Arc::new(AppCapacity::default());
        let accounts = Arc::new(AccountStore::at(root.join("accounts.json")));
        let cli_capacity = Arc::new(CliCapacity::new());
        omniget_core::core::llm::budget::policy::set_quota_source(
            omniget_core::core::llm::budget::policy::quota_from_capacity(cli_capacity.clone()),
        );
        let router = Arc::new(Router::new(Arc::new(CompositeCapacity::new(
            capacity.clone(),
            CliCapacitySource::new(cli_capacity.clone(), accounts.clone()),
        ))));
        let native = Arc::new(NativeRuntime::new());
        native.register(
            FAKE_PROVIDER,
            Arc::new(FakeProvider::text(
                "This is the fake provider answering (ProviderId \"fake\").",
                8,
            )),
        );
        let pause_broker = Arc::downgrade(&broker);
        let cli = Arc::new(
            CliRuntime::new(accounts.clone(), cli_capacity.clone())
                .with_pause_probe(Arc::new(move |request: &str| {
                    pause_broker
                        .upgrade()
                        .map(|b| b.has_pending(request))
                        .unwrap_or(false)
                }))
                .with_broker(Arc::downgrade(&broker)),
        );
        let runtime: Arc<dyn AgentRuntime> = Arc::new(
            CompositeRuntime::new(native.clone())
                .with_cli(cli.clone())
                .with_acp(Arc::new(omniget_core::core::llm::acp::AcpRuntime::new(
                    broker.clone(),
                ))),
        );
        let coordinator = Arc::new(
            Coordinator::new(runtime, broker.clone(), budget.clone(), bus.clone())
                .with_router(router.clone())
                .with_dir(Some(root.join("conversations"))),
        );

        {
            use omniget_core::core::assist;
            match assist::db::AssistDb::open(&root.join(assist::db::DB_FILE)) {
                Ok(db) => {
                    let db = Arc::new(db);
                    assist::db::set_global(db.clone());
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
            cli,
            router,
            coordinator,
            executor,
            turns: Mutex::new(HashMap::new()),
            model_overrides: Mutex::new(read_overrides(&root.join("model-overrides.json"))),
            telemetry_ticking: AtomicBool::new(false),
            bus_forwarding: AtomicBool::new(false),
            next_request: AtomicU64::new(1),
            root,
        });
        let _ = self.inner.set(inner.clone());
        let chosen = self.inner.get().cloned().unwrap_or(inner);
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

    pub fn budget(&self) -> Arc<BudgetStore> {
        self.inner().budget.clone()
    }

    pub fn coordinator(&self) -> Arc<Coordinator> {
        self.inner().coordinator.clone()
    }

    pub fn attach_app(&self, app: &tauri::AppHandle) {
        self.inner().executor.set_app(app.clone());
    }

    pub fn provider_for(&self, id: &ProviderId) -> Option<Arc<dyn Provider>> {
        let inner = self.inner();
        if let Some(found) = inner.native.get(id.as_str()) {
            return Some(found);
        }
        let built = build_provider(id)?;
        inner.native.register(id.as_str(), built.clone());
        Some(built)
    }

    pub fn roster(&self) -> Vec<AgentDef> {
        (*self.inner().roster.list()).clone()
    }

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
                    cost_per_1k: previous.cost_per_1k,
                    last_error: previous.last_error.clone(),
                    ..Capacity::default()
                },
            );
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

    pub async fn side_turn_stream(
        &self,
        conversation_id: &str,
        source_agent_id: &str,
        system: &str,
        input: &str,
        requested_model: Option<&str>,
    ) -> Result<
        (
            String,
            CancellationToken,
            BoxStream<'static, TurnEvent>,
            router::SideModel,
        ),
        String,
    > {
        if !router::is_side_conversation(conversation_id) {
            return Err(format!(
                "ERR_SIDE_SESSION: `{conversation_id}` is not a side conversation"
            ));
        }
        let mut agent = self
            .agent(source_agent_id)
            .ok_or_else(|| format!("{ERR_NO_AGENT}: no agent {source_agent_id}"))?;
        let side = self.side_agent(&mut agent, system, requested_model);
        let (request, cancel, stream) = self
            .turn_with_agent(conversation_id, agent, input, CancellationToken::new())
            .await?;
        Ok((request, cancel, stream, side))
    }

    pub fn side_agent(
        &self,
        agent: &mut AgentDef,
        system: &str,
        requested_model: Option<&str>,
    ) -> router::SideModel {
        use omniget_core::core::llm::agent::{AgentRole, RuntimeKind};
        let session = self.model_of(agent);
        let family = match &agent.runtime {
            RuntimeKind::Cli { cli, .. } => format!("{}-cli", cli.trim().to_ascii_lowercase()),
            RuntimeKind::Native => session.provider.as_str().to_ascii_lowercase(),
            RuntimeKind::Acp { .. } => "acp".to_string(),
        };
        let side = router::resolve_side(requested_model, None, &family, &session.model);
        agent.model = match &agent.model {
            ModelPolicy::Fixed { model } => ModelPolicy::Fixed {
                model: ModelRef {
                    provider: model.provider.clone(),
                    model: side.model.clone(),
                },
            },
            ModelPolicy::Route { chain } => {
                let picked = self
                    .inner()
                    .router
                    .pick_for(chain, &[], router::TaskKind::of_agent(agent))
                    .map(|r| r.candidate)
                    .ok()
                    .or_else(|| chain.first().cloned());
                ModelPolicy::Route {
                    chain: picked
                        .map(|c| Candidate {
                            model: side.model.clone(),
                            ..c
                        })
                        .into_iter()
                        .collect(),
                }
            }
        };
        agent.role = AgentRole::Custom(router::SIDE_ROLE.into());
        agent.system_prompt = system.to_string();
        agent.tools.clear();
        agent.skills.clear();
        side
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

        let first = stream.next().await;
        let request_id = match &first {
            Some(TurnEvent::Started { request_id }) => request_id.clone(),
            _ => format!("r{}", inner.next_request.fetch_add(1, Ordering::SeqCst)),
        };
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

    pub fn drop_conversation(&self, conversation_id: &str) {
        let _ = self.inner().conversations.delete(conversation_id);
    }

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

    pub fn model_of(&self, agent: &AgentDef) -> ModelRef {
        match &agent.model {
            ModelPolicy::Fixed { model } => model.clone(),
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

    fn ensure_providers(&self, agent: &AgentDef) -> Result<(), String> {
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

    pub fn note_event(&self, agent_id: &str, event: &TurnEvent) {
        let inner = self.inner();
        inner.telemetry.with(agent_id, agent_id, |t| match event {
            TurnEvent::TextDelta { text } => {
                t.state = AgentState::Streaming;
                t.context_used_tokens += (text.chars().count() as u64) / 4;
            }
            TurnEvent::ToolCallStart { .. } => t.state = AgentState::WaitingTool,
            TurnEvent::Usage { usage } => fold_usage(t, usage),
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
        inner.broker.cancel_request(request_id);
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

    pub fn pending_ask_list(&self) -> Vec<serde_json::Value> {
        use omniget_core::core::assist::runs::{self, PermissionState};
        let broker = self.inner().broker.clone();
        let Some(reg) = runs::active() else {
            return broker
                .pending_calls()
                .into_iter()
                .map(|(request_id, tool_call_id, tool)| {
                    serde_json::json!({
                        "request_id": request_id, "tool_call_id": tool_call_id,
                        "tool": tool, "preview": "", "agent": "",
                        "kind": omniget_core::core::llm::broker::ask_kind(&tool),
                        "can_always": omniget_core::core::llm::broker::ask_can_always(&tool),
                        "impact": broker.pending_impact(&tool_call_id),
                    })
                })
                .collect();
        };
        let mut rows = reg.permissions_in(PermissionState::Pending, 200);
        rows.retain(|p| {
            p.instance_id == reg.instance() && broker.has_pending_call(&p.tool_call_id)
        });
        rows.sort_by_key(|p| p.created_ms);
        rows.into_iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "agent": p.agent_id,
                    "request_id": p.run_id,
                    "tool_call_id": p.tool_call_id,
                    "tool": p.action,
                    "kind": omniget_core::core::llm::broker::ask_kind(&p.action),
                    "can_always": omniget_core::core::llm::broker::ask_can_always(&p.action),
                    "preview": p.preview,
                    "impact": broker.pending_impact(&p.tool_call_id),
                    "asked_ms": p.created_ms,
                    "deadline_ms": (p.deadline_ms < i64::MAX / 2).then_some(p.deadline_ms),
                })
            })
            .collect()
    }

    pub fn has_pending_ask(&self, request_id: &str) -> bool {
        self.inner().broker.has_pending(request_id)
    }

    pub fn has_pending_call(&self, tool_call_id: &str) -> bool {
        self.inner().broker.has_pending_call(tool_call_id)
    }

    pub fn set_watchdog(&self, config: omniget_core::core::llm::cli_runtime::WatchdogConfig) {
        self.inner().cli.set_watchdog(config);
    }

    pub fn ask_timeout_for(&self, request_id: &str) -> std::time::Duration {
        self.inner().broker.ask_timeout_for(request_id)
    }

    pub fn set_ask_timeout(&self, timeout: std::time::Duration) {
        self.inner().broker.set_ask_timeout(timeout);
    }

    pub fn set_ask_timeout_resolver(
        &self,
        resolver: Option<omniget_core::core::llm::broker::AskTimeoutResolver>,
    ) {
        self.inner().broker.set_ask_timeout_resolver(resolver);
    }

    pub fn answer_tool(
        &self,
        _request_id: &str,
        tool_call_id: &str,
        allow: bool,
        always: bool,
    ) -> Result<(), String> {
        use omniget_core::core::llm::broker::Decision;
        let decision = match (allow, always) {
            (false, _) => Decision::Decline,
            (true, true) => Decision::AcceptAlways,
            (true, false) => Decision::Accept,
        };
        self.answer_tool_with(tool_call_id, decision)
    }

    pub fn answer_tool_with(
        &self,
        tool_call_id: &str,
        decision: omniget_core::core::llm::broker::Decision,
    ) -> Result<(), String> {
        self.inner()
            .broker
            .answer_call_by(
                tool_call_id,
                decision,
                &omniget_core::core::assist::runs::Actor::window(),
            )
            .map_err(|_| format!("{ERR_NO_ASK}: {tool_call_id}"))
    }

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

    fn quotas(&self) -> Vec<QuotaStatus> {
        let inner = self.inner();
        let chain = inner.telemetry.chain();
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

    pub fn accounts(&self) -> Arc<AccountStore> {
        self.inner().accounts.clone()
    }

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

    pub fn claim_bus_forwarder(&self) -> bool {
        self.inner().bus_forwarding.swap(true, Ordering::SeqCst)
    }

    pub fn note_bus(&self, event: &BusEvent) {
        let inner = self.inner();
        match event {
            BusEvent::ToolAsk { agent, .. } => inner.telemetry.with(agent, agent, |t| {
                t.state = AgentState::WaitingTool;
            }),
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
}

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

pub fn build_provider(id: &ProviderId) -> Option<Arc<dyn Provider>> {
    build_provider_with(id, false)
}

pub fn provider_base_url(provider: &str) -> Option<String> {
    match key_view(&ai_keys::list(), provider) {
        Some(view) => Some(ai_keys::app_base_url(&view.kind, &view.base_url)),
        None => omniget_core::core::llm::local_servers::LocalKind::all()
            .into_iter()
            .find(|k| k.id() == provider)
            .map(|k| k.openai_base("")),
    }
}

fn key_view<'a>(views: &'a [ai_keys::KeyView], provider: &str) -> Option<&'a ai_keys::KeyView> {
    views
        .iter()
        .find(|k| k.id == provider && k.has_key)
        .or_else(|| views.iter().find(|k| k.kind == provider && k.has_key))
}

pub fn build_provider_with(id: &ProviderId, capture: bool) -> Option<Arc<dyn Provider>> {
    let provider = id.as_str();
    if provider == FAKE_PROVIDER {
        return Some(Arc::new(FakeProvider::text("fake provider", 4)));
    }
    let views = ai_keys::list();
    let entry =
        key_view(&views, provider).and_then(|view| ai_keys::entry_with_secret(&view.id).ok());
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
