//! Coordinator: owner of the turn. Streams the model through, resolves the
//! tool loop, cuts on budget, reroutes on rate limits without losing the turn,
//! persists the conversation and hands off between agents.
//! Owned by f2-llm-coordinator.
//!
//! The returned stream is the only thing the commands layer needs: every tool
//! call, reroute and retry already happened inside it. Everything the UI shows
//! beyond the text (tool asks, telemetry, the mascot) comes from the bus.
//!
//! Conversations are appended to `<app_data>/llm/conversations/<id>.jsonl`, one
//! JSON record per line (`OMNIGET_DATA_DIR` overrides the root).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use futures::channel::mpsc;
use futures::sink::SinkExt;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::agent::{AgentDef, ModelPolicy};
use super::broker::ToolBroker;
use super::budget::BudgetStore;
use super::error::LlmError;
use super::prune::ContextPruner;
use super::router::{self, AlwaysAvailable, Router};
use super::runtime::AgentRuntime;
use super::types::{
    ContentPart, FinishReason, GenParams, Message, ModelRef, Role, TurnEvent, TurnRequest, Usage,
};
use crate::core::omni::bus::{Bus, BusEvent};

/// The agent asked for more tool calls than its budget allows in one turn.
pub const ERR_LLM_TOOL_LIMIT: &str = "ERR_LLM_TOOL_LIMIT";
/// The conversation id is not a safe file name.
pub const ERR_LLM_CONVERSATION: &str = "ERR_LLM_CONVERSATION";

/// Used when `Budget::max_tool_calls_per_turn` is 0 (unset), so a fresh agent
/// can still use tools.
pub const DEFAULT_MAX_TOOL_CALLS: u8 = 48;
/// Buffer of the event channel handed to the caller.
const CHANNEL_CAP: usize = 64;

/// One line of `<id>.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub message: Message,
}

/// `<app_data>/llm/conversations`.
pub fn default_conversations_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("llm").join("conversations"))
}

/// Conversation ids become file names, so they are restricted to
/// `[A-Za-z0-9_-]` (uuids and slugs pass, `../` does not).
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn conversation_path(dir: &Path, id: &str) -> Result<PathBuf, LlmError> {
    if !is_safe_id(id) {
        return Err(LlmError::new(
            ERR_LLM_CONVERSATION,
            format!("unsafe conversation id `{id}`"),
        ));
    }
    Ok(dir.join(format!("{id}.jsonl")))
}

/// Cut a conversation back to its first `keep` messages and return what was
/// removed. Undo uses it: the tools of the removed turn are not run again.
pub fn truncate_conversation(dir: &Path, id: &str, keep: usize) -> Result<Vec<Message>, LlmError> {
    let path = conversation_path(dir, id)?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let mut kept = String::new();
    let mut removed = Vec::new();
    let mut seen = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<ConversationRecord>(line) else {
            continue;
        };
        if seen < keep {
            kept.push_str(line);
            kept.push('\n');
        } else {
            removed.push(record.message);
        }
        seen += 1;
    }
    if !removed.is_empty() {
        std::fs::write(&path, kept)
            .map_err(|e| LlmError::new(ERR_LLM_CONVERSATION, e.to_string()))?;
    }
    Ok(removed)
}

/// Read a conversation back. Unparseable lines are skipped: a truncated file
/// must not take the chat down.
pub fn load_conversation(dir: &Path, id: &str) -> Result<Vec<Message>, LlmError> {
    let path = conversation_path(dir, id)?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<ConversationRecord>(l).ok())
        .map(|r| r.message)
        .collect())
}

/// Append one message. Best effort: a read-only disk must not fail the turn.
pub fn append_message(dir: &Path, id: &str, agent: Option<&str>, message: &Message) {
    let Ok(path) = conversation_path(dir, id) else {
        return;
    };
    let record = ConversationRecord {
        ts: chrono::Utc::now().to_rfc3339(),
        agent: agent.map(|s| s.to_string()),
        message: message.clone(),
    };
    let Ok(mut line) = serde_json::to_string(&record) else {
        return;
    };
    line.push('\n');
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Rough token estimate for the budget gate: ~4 characters per token. Cheap on
/// purpose; the real number comes back in `Usage`.
/// The project brief and the KB index, folded into the first system message of
/// the request. Read fresh each time (an agent may have just written a note)
/// and never persisted, so the transcript does not grow with it.
fn with_project_kb(conversation_id: &str, history: &[Message]) -> Vec<Message> {
    let mut messages = history.to_vec();
    let Some(ws) = super::code_tools::workspace_of(conversation_id) else {
        return messages;
    };
    let index = super::kb::index_prompt(&ws);
    if index.trim().is_empty() {
        return messages;
    }
    match messages.iter_mut().find(|m| m.role == Role::System) {
        Some(system) => system.parts.push(ContentPart::Text {
            text: format!("\n\n{index}"),
        }),
        None => messages.insert(0, Message::text(Role::System, index)),
    }
    messages
}

/// The augments' context for this request, folded into the system message
/// the same way as the KB index: fresh every request, never persisted.
fn with_turn_context(mut messages: Vec<Message>, extra: &[String]) -> Vec<Message> {
    if extra.is_empty() {
        return messages;
    }
    let text = extra.join("\n\n");
    match messages.iter_mut().find(|m| m.role == Role::System) {
        Some(system) => system.parts.push(ContentPart::Text {
            text: format!("\n\n{text}"),
        }),
        None => messages.insert(0, Message::text(Role::System, text)),
    }
    messages
}

/// Ceiling for one tool result of a CLI agent kept in the transcript.
const CLI_RESULT_MAX_CHARS: usize = 12_000;

fn clip_cli_result(content: &str) -> String {
    if content.len() <= CLI_RESULT_MAX_CHARS {
        return content.to_string();
    }
    let mut end = CLI_RESULT_MAX_CHARS;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[clipped: {} of {} chars kept]",
        &content[..end],
        end,
        content.len()
    )
}

/// The account a turn is charged to (3.3): a CLI agent's account, or
/// `provider:<id>` for a native model; an ACP agent has none.
fn account_of(agent: &AgentDef, model: Option<&ModelRef>) -> Option<String> {
    match &agent.runtime {
        super::agent::RuntimeKind::Cli { account, .. } => Some(account.clone()),
        super::agent::RuntimeKind::Native => {
            model.map(|m| format!("provider:{}", m.provider.as_str()))
        }
        super::agent::RuntimeKind::Acp { .. } => None,
    }
}

pub fn estimate_tokens(messages: &[Message]) -> u32 {
    let chars: usize = messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .map(|p| match p {
            ContentPart::Text { text } => text.len(),
            ContentPart::Image { data_b64, .. } => data_b64.len() / 3,
            ContentPart::ToolUse { input, .. } => input.to_string().len(),
            ContentPart::ToolResult { content, .. } => content.len(),
        })
        .sum();
    (chars / 4) as u32
}

/// A tool call being assembled from the stream deltas.
#[derive(Debug, Clone, Default)]
struct PendingCall {
    id: String,
    name: String,
    input_json: String,
}

impl PendingCall {
    fn input(&self) -> Value {
        serde_json::from_str(&self.input_json).unwrap_or(Value::Null)
    }
}

/// A per-turn hook: adjusts the agent the turn runs with (extra grants, a
/// skill binding) and returns context for this request only. The text goes
/// into the system message of the request and is never persisted in the
/// transcript, so memory and skill indexes are read fresh every turn.
///
/// Implementations: `assist::bots` (skills + capability grants),
/// `assist::memory` (recalled profile), `assist::reading`, `assist::groups`.
pub trait TurnAugment: Send + Sync {
    fn augment(
        &self,
        agent: &mut AgentDef,
        conversation_id: &str,
        user_input: &str,
    ) -> Option<String>;

    /// Called once a turn finished normally, off the stream (the reply was
    /// already sent). Must not block: spawn anything slow. `assist::memory`
    /// extracts facts here.
    fn after_turn(
        &self,
        _agent: &AgentDef,
        _conversation_id: &str,
        _user_input: &str,
        _reply: &str,
    ) {
    }
}

pub struct Coordinator {
    runtime: Arc<dyn AgentRuntime>,
    augments: std::sync::RwLock<Vec<Arc<dyn TurnAugment>>>,
    broker: Arc<ToolBroker>,
    budget: Arc<BudgetStore>,
    bus: Arc<Bus>,
    router: Arc<Router>,
    dir: Option<PathBuf>,
    params: GenParams,
    /// Context pruning. Disabled until someone calls `with_prune`, and even
    /// then it only ever runs between turns. Behind a lock so a settings
    /// change swaps it without rebuilding the coordinator; a turn in flight
    /// keeps the one it started with.
    pruner: std::sync::RwLock<Arc<ContextPruner>>,
    /// Durable run record. `None` falls back to `assist::runs::active()`
    /// (the app's, once enabled at boot).
    runs: std::sync::RwLock<Option<crate::core::assist::runs::Registry>>,
    /// Budget policies (3.3). `None` falls back to the app's engine
    /// (`budget::policy::active()`); without either, or without a policy,
    /// nothing below changes.
    policies: std::sync::RwLock<Option<Arc<super::budget::policy::Engine>>>,
}

impl std::fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coordinator")
            .field("dir", &self.dir)
            .finish()
    }
}

impl Coordinator {
    pub fn new(
        runtime: Arc<dyn AgentRuntime>,
        broker: Arc<ToolBroker>,
        budget: Arc<BudgetStore>,
        bus: Arc<Bus>,
    ) -> Self {
        Self {
            runtime,
            augments: std::sync::RwLock::new(Vec::new()),
            broker,
            budget,
            bus,
            router: Arc::new(Router::new(Arc::new(AlwaysAvailable))),
            dir: default_conversations_dir(),
            params: GenParams::default(),
            pruner: std::sync::RwLock::new(Arc::new(ContextPruner::disabled())),
            runs: std::sync::RwLock::new(None),
            policies: std::sync::RwLock::new(None),
        }
    }

    fn policies(&self) -> Option<Arc<super::budget::policy::Engine>> {
        self.policies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .or_else(super::budget::policy::active)
    }

    fn runs(&self) -> Option<crate::core::assist::runs::Registry> {
        self.runs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .or_else(crate::core::assist::runs::active)
    }

    /// The policy gate of one turn (3.3): refuses a paused scope or a
    /// reached ceiling, and asks the questions it needs through the durable
    /// approvals (2.2) — a checkpoint "continue?", a model with no price
    /// under a dollar ceiling — each once, remembered.
    async fn policy_gate(
        &self,
        engine: &super::budget::policy::Engine,
        ctx: &super::budget::policy::TurnCtx,
    ) -> Result<(), LlmError> {
        use super::budget::policy::Verdict;
        match engine.gate(ctx).await {
            Verdict::Allow => Ok(()),
            Verdict::Deny(block) => Err(block.error()),
            Verdict::Ask(needs) => {
                for need in needs {
                    let call = format!(
                        "budget-{}",
                        &uuid::Uuid::new_v4().simple().to_string()[..12]
                    );
                    let end = self
                        .broker
                        .ask_user_ended(
                            &ctx.agent_id,
                            &ctx.request_id,
                            &call,
                            need.action(),
                            &need.message,
                        )
                        .await;
                    let allow = !matches!(end.answer(), super::broker::Answer::Deny);
                    if let Some(block) = engine.answer(&need, allow, Some(&ctx.agent_id)) {
                        return Err(block.error());
                    }
                }
                Ok(())
            }
        }
    }

    pub fn budget(&self) -> &Arc<BudgetStore> {
        &self.budget
    }

    pub fn with_router(mut self, router: Arc<Router>) -> Self {
        self.router = router;
        self
    }

    /// Same as [`with_prune`], on a coordinator that is already shared.
    pub fn set_prune(&self, pruner: Arc<ContextPruner>) {
        match self.pruner.write() {
            Ok(mut slot) => *slot = pruner,
            Err(poisoned) => *poisoned.into_inner() = pruner,
        }
    }

    pub fn pruner(&self) -> Arc<ContextPruner> {
        match self.pruner.read() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Where conversations are written; `None` keeps the turn in memory.
    pub fn with_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.dir = dir;
        self
    }

    /// Adds a per-turn hook; hooks run in the order they were added.
    pub fn add_augment(&self, augment: Arc<dyn TurnAugment>) {
        self.augments
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(augment);
    }

    /// Runs every hook over a copy of `agent`. Public so the app can show
    /// the effective agent (grants, skills) without starting a turn.
    pub fn effective_agent(
        &self,
        agent: &AgentDef,
        conversation_id: &str,
        user_input: &str,
    ) -> (AgentDef, Vec<String>) {
        let mut agent = agent.clone();
        // Personal overlays, room context and reading data have local-only
        // legacy stores. External runs receive only the explicitly pinned
        // executor prompt until each augment has principal-aware queries.
        if crate::core::assist::authority::external(conversation_id) {
            // Grant ∩ the executor's own modes: Deny stays out, Ask stays Ask.
            let own = std::mem::take(&mut agent.tools);
            if let Ok(ceiling) = crate::core::assist::db::global().and_then(|db| {
                crate::core::assist::authority::resolve(&db, conversation_id, &agent.id)
            }) {
                agent.tools =
                    crate::core::assist::authority::external_tool_grants(&own, &ceiling.tools);
            }
            return (agent, Vec::new());
        }
        let hooks = self
            .augments
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut extra = Vec::new();
        for h in hooks {
            if let Some(text) = h.augment(&mut agent, conversation_id, user_input) {
                if !text.trim().is_empty() {
                    extra.push(text);
                }
            }
        }
        (agent, extra)
    }

    pub fn bus(&self) -> &Arc<Bus> {
        &self.bus
    }

    pub fn broker(&self) -> &Arc<ToolBroker> {
        &self.broker
    }

    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn history(&self, conversation_id: &str) -> Result<Vec<Message>, LlmError> {
        match &self.dir {
            Some(dir) => load_conversation(dir, conversation_id),
            None => Ok(Vec::new()),
        }
    }

    fn persist(&self, conversation_id: &str, agent: &str, message: &Message) {
        if let Some(dir) = &self.dir {
            append_message(dir, conversation_id, Some(agent), message);
        }
    }

    /// Which model serves the next attempt, plus its routing key. `tried` holds
    /// the keys already burned in this turn.
    fn resolve(&self, agent: &AgentDef, tried: &[String]) -> Result<(ModelRef, String), LlmError> {
        match &agent.model {
            ModelPolicy::Fixed { model } => Ok((
                model.clone(),
                format!("native:{}:{}", model.provider.as_str(), model.model),
            )),
            ModelPolicy::Route { chain } => {
                let route =
                    self.router
                        .pick_for(chain, tried, router::TaskKind::of_agent(agent))?;
                Ok((route.model, route.key))
            }
        }
    }

    /// Hand the conversation from one agent to another with a written summary.
    /// The summary lands in the transcript as a system message, so the next
    /// agent's first turn already carries it.
    pub fn handoff(
        &self,
        conversation_id: &str,
        from: &AgentDef,
        to: &AgentDef,
        summary: &str,
    ) -> Result<Message, LlmError> {
        if let Some(dir) = &self.dir {
            conversation_path(dir, conversation_id)?;
        }
        let message = Message::text(
            Role::System,
            format!(
                "Handoff from {} ({}) to {} ({}). Summary of the work so far:\n{}",
                from.name, from.id, to.name, to.id, summary
            ),
        );
        self.persist(conversation_id, &to.id, &message);
        Ok(message)
    }

    /// Run one user turn. The returned stream already has the tool loop, the
    /// reroutes and the cancellation resolved.
    pub fn run_turn(
        self: &Arc<Self>,
        conversation_id: &str,
        agent: &AgentDef,
        user_input: &str,
        cancel: CancellationToken,
    ) -> BoxStream<'static, TurnEvent> {
        self.run_turn_with_id(
            uuid::Uuid::new_v4().to_string(),
            conversation_id,
            agent,
            user_input,
            cancel,
        )
    }

    /// Same turn, with the caller's own id. The commands layer mints the id it
    /// already handed to the front end and passes it here, so
    /// `TurnEvent::Started { request_id }` and `BusEvent::ToolAsk.request_id`
    /// carry that id and the UI can tie an ask back to its turn.
    pub fn run_turn_with_id(
        self: &Arc<Self>,
        request_id: String,
        conversation_id: &str,
        agent: &AgentDef,
        user_input: &str,
        cancel: CancellationToken,
    ) -> BoxStream<'static, TurnEvent> {
        let (tx, rx) = mpsc::channel::<TurnEvent>(CHANNEL_CAP);
        let this = self.clone();
        let conversation_id = conversation_id.to_string();
        let agent = agent.clone();
        let user_input = user_input.to_string();
        tokio::spawn(async move {
            this.drive(request_id, conversation_id, agent, user_input, cancel, tx)
                .await;
        });
        rx.boxed()
    }

    async fn drive(
        self: Arc<Self>,
        request_id: String,
        conversation_id: String,
        agent: AgentDef,
        user_input: String,
        cancel: CancellationToken,
        mut tx: mpsc::Sender<TurnEvent>,
    ) {
        let started = Instant::now();
        if let Err(reason) = crate::core::assist::authority::check_agent(&conversation_id, &agent) {
            let _ = tx
                .send(TurnEvent::Error {
                    error: LlmError::new("EXTERNAL_EXECUTION_DENIED", reason),
                })
                .await;
            return;
        }
        let authority_agent = agent.clone();
        let (agent, turn_context) = self.effective_agent(&agent, &conversation_id, &user_input);
        // The run is on disk before anything reaches the provider or the UI.
        let runs = self.runs();
        if crate::core::assist::authority::external(&conversation_id) && runs.is_none() {
            let _ = tx
                .send(TurnEvent::Error {
                    error: LlmError::new(
                        "RUN_STORAGE_REQUIRED",
                        "External execution requires durable run records",
                    ),
                })
                .await;
            return;
        }
        let runtime_label = match &agent.runtime {
            super::agent::RuntimeKind::Native => "native".to_string(),
            super::agent::RuntimeKind::Cli { cli, .. } => format!("cli:{cli}"),
            super::agent::RuntimeKind::Acp { command, .. } => format!("acp:{command}"),
        };
        if let Some(reg) = &runs {
            if let Err(e) = reg.open_run(crate::core::assist::runs::NewRun {
                id: request_id.clone(),
                conversation_id: conversation_id.clone(),
                bot_id: agent.id.clone(),
                parent_run_id: None,
                runtime: Some(runtime_label.clone()),
                input_preview: user_input.clone(),
            }) {
                tracing::warn!("[runs] {e}");
            }
            let _ = reg.transition(
                &request_id,
                crate::core::assist::runs::RunState::Preparing,
                None,
            );
        }
        let mut turn_error: Option<String> = None;
        let mut history = self.history(&conversation_id).unwrap_or_default();
        super::snapshot::mark_turn(&request_id, history.len());
        // Replay the decisions a previous turn already froze. This is a map
        // lookup and a string swap, never a judge call: if the judgement of
        // the last turn is still running, this turn simply uses the history as
        // it stands and picks the omissions up next time.
        //
        // `history` itself stays whole: the omissions are applied to the copy
        // each request is built from (see the loop below), so the judge always
        // sees the real outputs and a verdict that lands in the middle of a
        // long turn takes effect on the very next request of that turn.
        if history.is_empty() && !agent.system_prompt.is_empty() {
            // Persisted too, so a resumed conversation replays identically.
            let system = Message::text(Role::System, agent.system_prompt.clone());
            self.persist(&conversation_id, &agent.id, &system);
            history.push(system);
        }
        let user_msg = Message::text(Role::User, user_input.clone());
        self.persist(&conversation_id, &agent.id, &user_msg);
        history.push(user_msg);

        // Gate before anything reaches the network.
        let estimate = estimate_tokens(&history);
        // The per-turn token ceiling, then a reservation against the day:
        // two turns of the same agent cannot both slip under the cap.
        let reservation = self
            .budget
            .check(&agent.id, &agent.budget, estimate)
            .and_then(|_| {
                self.budget.reserve(
                    &agent.id,
                    &super::budget::PoolLimits {
                        usd: agent.budget.usd_per_day,
                        tokens: None,
                        turns: None,
                        strict_unknown: false,
                    },
                    super::budget::Estimate {
                        usd: None,
                        tokens: estimate as u64,
                    },
                )
            });
        // Budget policies (3.3): a paused scope, a hard ceiling already
        // reached, or a question someone must answer first. Skipped whole
        // when no policy exists.
        let policy_turn = self.policies().map(|engine| {
            let model = self.resolve(&agent, &[]).map(|(m, _)| m).ok();
            let ctx = super::budget::policy::TurnCtx {
                request_id: request_id.clone(),
                agent_id: agent.id.clone(),
                account: account_of(&agent, model.as_ref()),
                model: model.map(|m| m.model).unwrap_or_default(),
                conversation_id: conversation_id.clone(),
                estimate_tokens: estimate as u64,
            };
            (engine, ctx)
        });
        let reservation = match reservation {
            Ok(id) => match &policy_turn {
                Some((engine, ctx)) if engine.has_policies() => {
                    match self.policy_gate(engine, ctx).await {
                        Ok(()) => Ok(id),
                        Err(err) => {
                            self.budget.release(&id);
                            Err(err)
                        }
                    }
                }
                _ => Ok(id),
            },
            Err(err) => Err(err),
        };
        let reservation = match reservation {
            Ok(id) => id,
            Err(err) => {
                if let Some(reg) = &runs {
                    let _ = reg.transition(
                        &request_id,
                        crate::core::assist::runs::RunState::Failed,
                        Some(&format!("{}: {}", err.code, err.message)),
                    );
                }
                self.bus.emit(BusEvent::BudgetHit {
                    agent: agent.id.clone(),
                });
                let _ = tx.send(TurnEvent::Error { error: err }).await;
                let _ = tx
                    .send(TurnEvent::Finished {
                        reason: FinishReason::Other,
                    })
                    .await;
                return;
            }
        };

        // Running: persisted before the frontend hears `Started`.
        if let Some(reg) = &runs {
            let _ = reg.transition(
                &request_id,
                crate::core::assist::runs::RunState::Running,
                None,
            );
        }
        // A CLI agent runs its own tool loop: it reaches the assistant tools
        // through a scoped MCP projection with this turn's grants.
        let projection = match (
            &agent.runtime,
            &runs,
            crate::core::assist::projection::endpoint(),
        ) {
            (super::agent::RuntimeKind::Cli { .. }, Some(reg), Some(url)) => {
                let tools = if crate::core::assist::authority::external(&conversation_id) {
                    crate::core::assist::projection::granted_tools_external(
                        &self.broker,
                        &agent.tools,
                    )
                } else {
                    crate::core::assist::projection::granted_tools(&self.broker, &agent.tools)
                };
                match crate::core::assist::projection::issue(
                    reg.db(),
                    &agent.id,
                    &conversation_id,
                    Some(&request_id),
                    &tools,
                    true,
                    crate::core::assist::projection::DEFAULT_TTL_MS,
                ) {
                    Ok(issued) => {
                        crate::core::assist::projection::attach_broker(
                            &issued.id,
                            self.broker.clone(),
                        );
                        Some(crate::core::assist::projection::TurnProjection {
                            grant_id: issued.id,
                            url,
                            token: issued.token,
                        })
                    }
                    Err(e) => {
                        tracing::warn!("[projection] {e}");
                        None
                    }
                }
            }
            _ => None,
        };
        let mut last_text = String::new();

        self.bus.emit(BusEvent::TurnStarted {
            agent: agent.id.clone(),
            conversation: conversation_id.clone(),
        });
        if tx
            .send(TurnEvent::Started {
                request_id: request_id.clone(),
            })
            .await
            .is_err()
        {
            // Nobody listens any more: nothing was sent, free everything.
            self.budget.release(&reservation);
            if let Some(reg) = &runs {
                let _ = reg.transition(
                    &request_id,
                    crate::core::assist::runs::RunState::Cancelled,
                    None,
                );
                if let Some(p) = &projection {
                    crate::core::assist::projection::revoke(reg.db(), &p.grant_id);
                }
            }
            return;
        }

        let mut specs = self.broker.specs_for(&agent.tools);
        if crate::core::assist::authority::external(&conversation_id) {
            specs.retain(|s| {
                crate::core::assist::authority::check_tool(&conversation_id, &agent.id, &s.name)
                    .is_ok()
            });
            for spec in &mut specs {
                if spec.name == "fs_read" {
                    spec.description="Read a bounded text file relative to the granted workspace. Directories and protected paths are denied.".into();
                }
                if spec.name == "fs_glob" {
                    spec.description="Find paths relative to the granted workspace using *, ** or ?. Braces, character classes and protected paths are unsupported.".into();
                }
                // External missions get the external file tools' own schemas
                // (revision-bound edits, create-only writes).
                if let Some((d, schema)) =
                    crate::core::assist::external_files::external_spec(&spec.name)
                {
                    spec.description = d.into();
                    spec.input_schema = schema;
                }
            }
        }
        let max_tool_calls = match agent.budget.max_tool_calls_per_turn {
            0 => DEFAULT_MAX_TOOL_CALLS,
            n => n,
        };
        let max_reroutes = match &agent.model {
            ModelPolicy::Route { chain } => chain.len(),
            ModelPolicy::Fixed { .. } => 0,
        };

        let mut tried: Vec<String> = Vec::new();
        let mut reroutes = 0usize;
        let mut tool_calls_used = 0u8;
        let mut total = Usage::default();
        let mut requests = 0u32;
        let mut last_model: Option<ModelRef> = None;
        let finish: FinishReason = 'turn: loop {
            if cancel.is_cancelled() {
                break 'turn FinishReason::Cancelled;
            }
            let (model, key) = match self.resolve(&agent, &tried) {
                Ok(v) => {
                    last_model = Some(v.0.clone());
                    v
                }
                Err(err) => {
                    turn_error = Some(format!("{}: {}", err.code, err.message));
                    let _ = tx.send(TurnEvent::Error { error: err }).await;
                    break 'turn FinishReason::Other;
                }
            };
            let pruner = self.pruner();
            let (sent, pruned) = pruner.apply_sticky(&conversation_id, &history);
            let receipt = pruner.config().enabled.then(|| {
                (
                    pruned.omitted as u32,
                    estimate_tokens(&history),
                    estimate_tokens(&sent),
                )
            });
            requests += 1;
            let mut external_debit: Option<String> = None;
            let mut req = TurnRequest {
                model,
                messages: with_turn_context(
                    if crate::core::assist::authority::external(&conversation_id) {
                        sent.clone()
                    } else {
                        with_project_kb(&conversation_id, &sent)
                    },
                    &turn_context,
                ),
                tools: specs.clone(),
                params: self.params.clone(),
                cancel: cancel.clone(),
                agent_id: Some(agent.id.clone()),
            };
            if crate::core::assist::authority::external(&conversation_id) {
                let output = req.params.max_tokens.unwrap_or(4096).min(4096);
                req.params.max_tokens = Some(output);
                req.params.extra.clear();
                let bound = serde_json::to_vec(&(&req.messages, &req.tools))
                    .map(|v| v.len() as u64)
                    .unwrap_or(u64::MAX)
                    .saturating_add(output as u64)
                    .saturating_add(4096);
                match crate::core::assist::authority::reserve_model_id(
                    &conversation_id,
                    &agent.id,
                    bound,
                ) {
                    Ok(id) => external_debit = Some(id),
                    Err(reason) => {
                        turn_error = Some(reason.clone());
                        let _ = tx
                            .send(TurnEvent::Error {
                                error: LlmError::new("EXTERNAL_EXECUTION_DENIED", reason),
                            })
                            .await;
                        break 'turn FinishReason::Other;
                    }
                }
            }
            // A CLI or ACP agent edits files with its own tools, which never
            // pass through the broker: the undo snapshot is taken up front.
            if !matches!(agent.runtime, super::agent::RuntimeKind::Native) {
                if let Some(ws) = super::code_tools::workspace_of(&conversation_id) {
                    if let Err(e) =
                        super::snapshot::before_write(&ws, &conversation_id, &request_id).await
                    {
                        tracing::warn!("[snapshot] {e}");
                    }
                }
            }
            let opening = super::code_tools::scope(
                &conversation_id,
                &agent.id,
                &request_id,
                crate::core::assist::runs::scope(
                    runs.clone(),
                    crate::core::assist::projection::scope_turn(
                        projection.clone(),
                        self.runtime.turn(&agent, req),
                    ),
                ),
            );
            tokio::pin!(opening);
            let mut opening_tick = tokio::time::interval(std::time::Duration::from_millis(250));
            let opened = loop {
                tokio::select! {
                    biased;
                    _=cancel.cancelled()=>break Err(LlmError::new("EXTERNAL_EXECUTION_CANCELLED","request cancelled before stream")),
                    _=opening_tick.tick(),if crate::core::assist::authority::external(&conversation_id)=>{
                        if let Err(reason)=crate::core::assist::authority::check_agent(&conversation_id,&authority_agent) {
                            cancel.cancel();break Err(LlmError::new("EXTERNAL_EXECUTION_DENIED",reason));
                        }
                    },
                    result=&mut opening=>break result,
                }
            };
            let stream = match opened {
                Ok(s) => Some(s),
                Err(err) => {
                    match self.maybe_reroute(
                        &agent,
                        &err,
                        &key,
                        &mut tried,
                        &mut reroutes,
                        max_reroutes,
                    ) {
                        true => continue 'turn,
                        false => {
                            turn_error = Some(format!("{}: {}", err.code, err.message));
                            let _ = tx.send(TurnEvent::Error { error: err }).await;
                            break 'turn FinishReason::Other;
                        }
                    }
                }
            };
            let mut stream = stream.expect("stream or an early continue");

            let mut text = String::new();
            let mut calls: Vec<PendingCall> = Vec::new();
            let mut failed: Option<LlmError> = None;
            let mut reason: Option<FinishReason> = None;
            let mut cancelled = false;
            let mut request_input: Option<u32> = None;
            let mut request_billable_input: u64 = 0;
            let mut request_output: u32 = 0;
            let mut cli_results: Vec<(String, String, bool)> = Vec::new();
            let mut authority_tick = tokio::time::interval(std::time::Duration::from_millis(250));

            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        cancelled = true;
                        None
                    }
                    _ = authority_tick.tick(), if crate::core::assist::authority::external(&conversation_id) => {
                        if crate::core::assist::authority::check_agent(&conversation_id,&authority_agent).is_err() {
                            cancel.cancel();cancelled=true;None
                        } else {continue;}
                    }
                    ev = stream.next() => ev,
                };
                let Some(event) = next else { break };
                match event {
                    TurnEvent::Started { .. } => {}
                    TurnEvent::TextDelta { text: delta } => {
                        self.bus.emit(BusEvent::TokenDelta {
                            agent: agent.id.clone(),
                            chars: delta.chars().count() as u32,
                        });
                        text.push_str(&delta);
                        if tx.send(TurnEvent::TextDelta { text: delta }).await.is_err() {
                            cancelled = true;
                            break;
                        }
                    }
                    TurnEvent::ThinkingDelta { text: delta } => {
                        let _ = tx.send(TurnEvent::ThinkingDelta { text: delta }).await;
                    }
                    TurnEvent::ToolCallStart { id, name } => {
                        calls.push(PendingCall {
                            id: id.clone(),
                            name: name.clone(),
                            input_json: String::new(),
                        });
                        let _ = tx.send(TurnEvent::ToolCallStart { id, name }).await;
                    }
                    TurnEvent::ToolCallDelta {
                        id,
                        input_json_delta,
                    } => {
                        if let Some(c) = calls.iter_mut().find(|c| c.id == id) {
                            c.input_json.push_str(&input_json_delta);
                        }
                        let _ = tx
                            .send(TurnEvent::ToolCallDelta {
                                id,
                                input_json_delta,
                            })
                            .await;
                    }
                    TurnEvent::ToolCallEnd { id } => {
                        let _ = tx.send(TurnEvent::ToolCallEnd { id }).await;
                    }
                    TurnEvent::ToolResult {
                        id,
                        content,
                        is_error,
                    } => {
                        cli_results.push((id, content, is_error));
                    }
                    // Only the Coordinator writes receipts.
                    TurnEvent::PruneReceipt { .. } => {}
                    TurnEvent::Usage { usage } => {
                        // `input_tokens` already holds the cache (one
                        // Usage convention); caps count the weighted input.
                        request_input = Some(
                            request_input
                                .unwrap_or(0)
                                .saturating_add(usage.input_tokens),
                        );
                        request_billable_input =
                            request_billable_input.saturating_add(usage.billable_input_tokens());
                        request_output += usage.output_tokens;
                        total.input_tokens += usage.input_tokens;
                        total.output_tokens += usage.output_tokens;
                        total.cache_read_tokens += usage.cache_read_tokens;
                        total.cache_write_tokens += usage.cache_write_tokens;
                        if total.first_token_ms.is_none() {
                            total.first_token_ms = usage.first_token_ms;
                        }
                        if let Some(c) = usage.cost_usd {
                            total.cost_usd = Some(total.cost_usd.unwrap_or(0.0) + c);
                        }
                    }
                    TurnEvent::Finished { reason: r } => {
                        reason = Some(r);
                        break;
                    }
                    TurnEvent::Error { error } => {
                        failed = Some(error);
                        break;
                    }
                }
            }

            // External budget: settle the conservative debit with the usage
            // the provider reported for this request (unknown keeps the bound).
            if let (Some(debit), Some(_)) = (external_debit.take(), request_input) {
                let used = request_billable_input + request_output as u64;
                let _ = if matches!(agent.runtime, super::agent::RuntimeKind::Native) {
                    crate::core::assist::authority::settle_model(&debit, used)
                } else {
                    crate::core::assist::authority::settle_model_actual(&debit, used)
                };
            }
            if let Some((omitted, before, after)) = receipt {
                let _ = tx
                    .send(TurnEvent::PruneReceipt {
                        request: requests,
                        omitted,
                        est_tokens_before: before,
                        est_tokens_after: after,
                        input_tokens: request_input,
                        billable_input_tokens: request_input
                            .map(|_| request_billable_input.min(u32::MAX as u64) as u32),
                    })
                    .await;
            }

            if cancelled || cancel.is_cancelled() {
                break 'turn FinishReason::Cancelled;
            }

            if let Some(err) = failed {
                if self.maybe_reroute(&agent, &err, &key, &mut tried, &mut reroutes, max_reroutes) {
                    continue 'turn;
                }
                turn_error = Some(format!("{}: {}", err.code, err.message));
                let _ = tx.send(TurnEvent::Error { error: err }).await;
                break 'turn FinishReason::Other;
            }

            // A CLI agent (Claude Code, Codex, ACP) runs its own tools inside
            // its own process: what it streamed is a record of work already
            // done, not a request for the broker. Re-running it would call
            // tools we do not have and start the CLI all over again.
            if !matches!(agent.runtime, super::agent::RuntimeKind::Native) {
                // What it did is still worth remembering: the next turn replays
                // the conversation, and without these pairs the CLI reads the
                // same files again. Only complete pairs are kept (a call with
                // no result would be an invalid request for a native model),
                // each result clipped, and from there on they are ordinary
                // candidates for the pruner.
                let pairs: Vec<(&PendingCall, &(String, String, bool))> = calls
                    .iter()
                    .filter_map(|c| cli_results.iter().find(|r| r.0 == c.id).map(|r| (c, r)))
                    .collect();
                if let Some(reg) = &runs {
                    for c in &calls {
                        let result = cli_results.iter().find(|r| r.0 == c.id);
                        let _ = reg.add_event(
                            &request_id,
                            "tool_call",
                            Some(&c.id),
                            serde_json::json!({
                                "name": c.name,
                                "input": c.input(),
                                "ok": result.map(|r| !r.2),
                                "output": result.map(|r| clip_cli_result(&r.1).chars().take(1500).collect::<String>()),
                                "runtime": "own",
                            }),
                        );
                    }
                }
                if !pairs.is_empty() {
                    let asked = Message {
                        role: Role::Assistant,
                        parts: pairs
                            .iter()
                            .map(|(c, _)| ContentPart::ToolUse {
                                id: c.id.clone(),
                                name: c.name.clone(),
                                input: c.input(),
                            })
                            .collect(),
                    };
                    let answered = Message {
                        role: Role::Tool,
                        parts: pairs
                            .iter()
                            .map(|(_, r)| ContentPart::ToolResult {
                                tool_use_id: r.0.clone(),
                                content: clip_cli_result(&r.1),
                                is_error: r.2,
                            })
                            .collect(),
                    };
                    for msg in [asked, answered] {
                        self.persist(&conversation_id, &agent.id, &msg);
                        history.push(msg);
                    }
                }
                calls.clear();
            }

            if !text.trim().is_empty() {
                last_text = text.clone();
            }
            if calls.is_empty() {
                if !text.is_empty() {
                    let msg = Message::text(Role::Assistant, text);
                    self.persist(&conversation_id, &agent.id, &msg);
                    history.push(msg);
                }
                break 'turn reason.unwrap_or(FinishReason::Stop);
            }

            if tool_calls_used as usize + calls.len() > max_tool_calls as usize {
                let _ = tx
                    .send(TurnEvent::Error {
                        error: LlmError::new(
                            ERR_LLM_TOOL_LIMIT,
                            format!(
                                "agent `{}` hit {max_tool_calls} tool calls in one turn",
                                agent.id
                            ),
                        ),
                    })
                    .await;
                break 'turn FinishReason::Other;
            }

            // The assistant message that asked for the tools, then one tool
            // message per result; both go to disk so a resumed conversation
            // replays the same loop.
            let mut parts: Vec<ContentPart> = Vec::new();
            if !text.is_empty() {
                parts.push(ContentPart::Text { text: text.clone() });
            }
            for c in &calls {
                parts.push(ContentPart::ToolUse {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    input: c.input(),
                });
            }
            let assistant = Message {
                role: Role::Assistant,
                parts,
            };
            self.persist(&conversation_id, &agent.id, &assistant);
            history.push(assistant);

            for c in &calls {
                if cancel.is_cancelled() {
                    break 'turn FinishReason::Cancelled;
                }
                let tool_future = super::code_tools::scope(
                    &conversation_id,
                    &agent.id,
                    &request_id,
                    crate::core::assist::runs::scope(
                        runs.clone(),
                        self.broker.call(
                            &agent.id,
                            &agent.tools,
                            &request_id,
                            &c.id,
                            &c.name,
                            c.input(),
                        ),
                    ),
                );
                tokio::pin!(tool_future);
                let mut tool_tick = tokio::time::interval(std::time::Duration::from_millis(250));
                let outcome = loop {
                    tokio::select! {
                        biased;
                        _=cancel.cancelled()=>{self.broker.cancel_request(&request_id);break 'turn FinishReason::Cancelled;},
                        _=tool_tick.tick(),if crate::core::assist::authority::external(&conversation_id)=>{
                            if crate::core::assist::authority::check_agent(&conversation_id,&authority_agent).is_err(){cancel.cancel();self.broker.cancel_request(&request_id);break 'turn FinishReason::Cancelled;}
                        },
                        result=&mut tool_future=>break result,
                    }
                };
                let (content, is_error) = match outcome {
                    Ok(o) => (super::compress::tool_output(&c.name, &o.content), false),
                    // No path inside the app's profile reaches the model
                    // (the turn's own workspace stays readable).
                    Err(e) => (
                        format!(
                            "{}: {}",
                            e.code,
                            crate::core::paths::redact_private_paths(
                                &e.message,
                                super::code_tools::workspace_of(&conversation_id).as_deref()
                            )
                        ),
                        true,
                    ),
                };
                if let Some(reg) = &runs {
                    let _ = reg.add_event(
                        &request_id,
                        "tool_call",
                        Some(&c.id),
                        serde_json::json!({
                            "name": c.name,
                            "input": c.input(),
                            "ok": !is_error,
                            "output": content.chars().take(1500).collect::<String>(),
                            "runtime": "broker",
                        }),
                    );
                }
                let msg = Message {
                    role: Role::Tool,
                    parts: vec![ContentPart::ToolResult {
                        tool_use_id: c.id.clone(),
                        content,
                        is_error,
                    }],
                };
                self.persist(&conversation_id, &agent.id, &msg);
                history.push(msg);
                tool_calls_used += 1;
            }

            // A long turn is many requests. The judge looks at what is already
            // old inside this same turn, in its own task; whatever it freezes
            // is applied to the next request above. Never awaited here.
            if !crate::core::assist::authority::external(&conversation_id) && pruner.is_active() {
                let (pruner, id, snapshot) =
                    (pruner.clone(), conversation_id.clone(), history.clone());
                tokio::spawn(async move {
                    pruner.judge_turn(&id, &snapshot).await;
                });
            }
        };

        total.total_ms = started.elapsed().as_millis() as u32;
        if !matches!(finish, FinishReason::Cancelled) {
            let _ = tx
                .send(TurnEvent::Usage {
                    usage: total.clone(),
                })
                .await;
        }
        // Close the record before the stream says `Finished`: whoever reads
        // the run after the turn sees its final state.
        let used_anything =
            total.input_tokens + total.output_tokens > 0 || total.cost_usd.is_some();
        if matches!(finish, FinishReason::Cancelled) && !used_anything {
            // Cancelled before the provider billed anything: free the room.
            self.budget.release(&reservation);
        } else {
            self.budget.settle(
                &reservation,
                total.cost_usd,
                total.billable_input_tokens().min(u32::MAX as u64) as u32,
                total.output_tokens,
            );
            if let Some((engine, mut ctx)) = policy_turn {
                if let Some(m) = &last_model {
                    ctx.account = account_of(&agent, Some(m)).or(ctx.account);
                    ctx.model = m.model.clone();
                }
                let crossings = engine
                    .charge(
                        &ctx,
                        super::budget::policy::TurnUse {
                            usd: total.cost_usd,
                            input_tokens: total.billable_input_tokens(),
                            output_tokens: total.output_tokens as u64,
                        },
                    )
                    .await;
                for c in crossings {
                    if c.kind == "hard" {
                        self.bus.emit(BusEvent::BudgetHit {
                            agent: agent.id.clone(),
                        });
                    }
                    self.bus.emit(BusEvent::BudgetCrossed {
                        agent: agent.id.clone(),
                        request: request_id.clone(),
                        scope: c.policy.scope.key(),
                        kind: c.kind.to_string(),
                        incident: c.incident.map(|i| i.id),
                        observed: c.observed,
                        limit: c.policy.amount,
                        metric: c.policy.metric.as_str().to_string(),
                    });
                }
            }
        }
        if let Some(p) = &projection {
            if let Some(reg) = &runs {
                crate::core::assist::projection::revoke(reg.db(), &p.grant_id);
            }
        }
        if matches!(finish, FinishReason::Cancelled) {
            // Questions of this turn resolve as cancelled, never later.
            self.broker.cancel_request(&request_id);
        }
        if let Some(ws) = super::code_tools::workspace_of(&conversation_id)
            .filter(|_| !crate::core::assist::authority::external(&conversation_id))
        {
            if let Err(e) = super::snapshot::after_turn(&ws, &conversation_id, &request_id).await {
                tracing::debug!("[snapshot] after turn: {e}");
            }
        }
        if let Some(reg) = &runs {
            use crate::core::assist::runs::RunState;
            let (state, error) = match finish {
                FinishReason::Cancelled => (RunState::Cancelled, None),
                FinishReason::Stop | FinishReason::Length if turn_error.is_none() => {
                    (RunState::Completed, None)
                }
                _ => (
                    RunState::Failed,
                    Some(
                        turn_error
                            .clone()
                            .unwrap_or_else(|| "the turn ended abnormally".into()),
                    ),
                ),
            };
            let outcome = reg.set_outcome(
                &request_id,
                (!last_text.is_empty()).then_some(last_text.as_str()),
                total.input_tokens as u64,
                total.output_tokens as u64,
                total.cost_usd,
            );
            // A run a person already moved (cancelled from elsewhere) stays.
            let terminal = reg.transition(&request_id, state, error.as_deref());
            if crate::core::assist::authority::external(&conversation_id)
                && (outcome.is_err() || terminal.is_err())
            {
                let _ = tx
                    .send(TurnEvent::Error {
                        error: LlmError::new(
                            "RUN_OUTCOME_UNKNOWN",
                            "The final run receipt could not be persisted",
                        ),
                    })
                    .await;
                return;
            }
        }
        let _ = tx.send(TurnEvent::Finished { reason: finish }).await;
        self.bus.emit(BusEvent::TurnEnded {
            agent: agent.id.clone(),
            usage: total,
        });

        if matches!(finish, FinishReason::Stop)
            && turn_error.is_none()
            && !crate::core::assist::authority::external(&conversation_id)
            && !last_text.is_empty()
        {
            let hooks = self
                .augments
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            for h in hooks {
                h.after_turn(&agent, &conversation_id, &user_input, &last_text);
            }
        }

        // Context pruning runs here and only here: the turn is over, the
        // stream is closed, and the judge — a model or a network call — gets
        // its own task so nothing in the SSE path ever awaits it. The verdicts
        // land in the sticky store and take effect on the next turn.
        let pruner = self.pruner();
        if !crate::core::assist::authority::external(&conversation_id) && pruner.is_active() {
            let id = conversation_id;
            tokio::spawn(async move {
                pruner.judge_turn(&id, &history).await;
            });
        }
    }

    /// Rate limited? Burn this candidate, put it on cooldown and tell the bus
    /// where the turn moved. The conversation is resent whole by the caller.
    fn maybe_reroute(
        &self,
        agent: &AgentDef,
        err: &LlmError,
        key: &str,
        tried: &mut Vec<String>,
        reroutes: &mut usize,
        max_reroutes: usize,
    ) -> bool {
        if !router::is_reroutable(&err.code) || *reroutes >= max_reroutes {
            return false;
        }
        let ModelPolicy::Route { chain } = &agent.model else {
            return false;
        };
        tried.push(key.to_string());
        self.router.rotate_away(key);
        let Ok(next) = self
            .router
            .pick_for(chain, tried, router::TaskKind::of_agent(agent))
        else {
            return false;
        };
        *reroutes += 1;
        self.bus.emit(BusEvent::Rerouted {
            agent: agent.id.clone(),
            from: key.to_string(),
            to: next.key,
            why: err.code.to_string(),
        });
        true
    }
}
