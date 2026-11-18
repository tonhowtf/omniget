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
use super::router::{self, AlwaysAvailable, Router};
use super::runtime::AgentRuntime;
use super::types::{
    ContentPart, FinishReason, GenParams, Message, ModelRef, Role, TurnEvent, TurnRequest, Usage,
};
use crate::core::omni::bus::{Bus, BusEvent};

pub const ERR_LLM_TOOL_LIMIT: &str = "ERR_LLM_TOOL_LIMIT";
pub const ERR_LLM_CONVERSATION: &str = "ERR_LLM_CONVERSATION";

pub const DEFAULT_MAX_TOOL_CALLS: u8 = 48;
const CHANNEL_CAP: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub message: Message,
}

pub fn default_conversations_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("llm").join("conversations"))
}

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

pub trait TurnAugment: Send + Sync {
    fn augment(
        &self,
        agent: &mut AgentDef,
        conversation_id: &str,
        user_input: &str,
    ) -> Option<String>;

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
    runs: std::sync::RwLock<Option<crate::core::assist::runs::Registry>>,
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

    async fn policy_gate(
        &self,
        engine: &super::budget::policy::Engine,
        ctx: &super::budget::policy::TurnCtx,
        cancel: &CancellationToken,
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
                    let ask = self.broker.ask_user_ended(
                        &ctx.agent_id,
                        &ctx.request_id,
                        &call,
                        need.action(),
                        &need.message,
                    );
                    tokio::pin!(ask);
                    let end = tokio::select! {
                        biased;
                        end = &mut ask => end,
                        _ = cancel.cancelled() => {
                            self.broker.cancel_request(&ctx.request_id);
                            ask.await;
                            return Err(LlmError::new(
                                super::error::ERR_LLM_CANCELLED,
                                "the turn was cancelled while waiting for a budget answer",
                            ));
                        }
                    };
                    let allow = !matches!(end.answer(), super::broker::Answer::Deny);
                    if let Some(block) = engine.answer(&need, allow, Some(&ctx.agent_id)) {
                        return Err(block.error());
                    }
                }
                Ok(())
            }
        }
    }

    pub fn with_router(mut self, router: Arc<Router>) -> Self {
        self.router = router;
        self
    }

    pub fn with_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.dir = dir;
        self
    }

    pub fn add_augment(&self, augment: Arc<dyn TurnAugment>) {
        self.augments
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(augment);
    }

    pub fn effective_agent(
        &self,
        agent: &AgentDef,
        conversation_id: &str,
        user_input: &str,
    ) -> (AgentDef, Vec<String>) {
        let mut agent = agent.clone();
        if crate::core::assist::authority::external(conversation_id) {
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
        if history.is_empty() && !agent.system_prompt.is_empty() {
            let system = Message::text(Role::System, agent.system_prompt.clone());
            self.persist(&conversation_id, &agent.id, &system);
            history.push(system);
        }
        let user_msg = Message::text(Role::User, user_input.clone());
        self.persist(&conversation_id, &agent.id, &user_msg);
        history.push(user_msg);

        let estimate = estimate_tokens(&history);
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
        let policy_turn = self.policies().map(|engine| {
            let model = self.resolve(&agent, &[]).map(|(m, _)| m).ok();
            let ctx = super::budget::policy::TurnCtx {
                request_id: request_id.clone(),
                agent_id: agent.id.clone(),
                account: account_of(&agent, model.as_ref()),
                local: model
                    .as_ref()
                    .is_some_and(|m| super::budget::policy::provider_is_local(m.provider.as_str())),
                model: model.map(|m| m.model).unwrap_or_default(),
                conversation_id: conversation_id.clone(),
                estimate_tokens: estimate as u64,
            };
            (engine, ctx)
        });
        let reservation = match reservation {
            Ok(id) => match &policy_turn {
                Some((engine, ctx)) if engine.has_policies() => {
                    match self.policy_gate(engine, ctx, &cancel).await {
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

        if let Some(reg) = &runs {
            let _ = reg.transition(
                &request_id,
                crate::core::assist::runs::RunState::Running,
                None,
            );
        }
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
            let sent = history.clone();
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
                    TurnEvent::Usage { usage } => {
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

            if let (Some(debit), Some(_)) = (external_debit.take(), request_input) {
                let used = request_billable_input + request_output as u64;
                let _ = if matches!(agent.runtime, super::agent::RuntimeKind::Native) {
                    crate::core::assist::authority::settle_model(&debit, used)
                } else {
                    crate::core::assist::authority::settle_model_actual(&debit, used)
                };
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

            if !matches!(agent.runtime, super::agent::RuntimeKind::Native) {
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
        };

        total.total_ms = started.elapsed().as_millis() as u32;
        if !matches!(finish, FinishReason::Cancelled) {
            let _ = tx
                .send(TurnEvent::Usage {
                    usage: total.clone(),
                })
                .await;
        }
        let used_anything =
            total.input_tokens + total.output_tokens > 0 || total.cost_usd.is_some();
        if matches!(finish, FinishReason::Cancelled) && !used_anything {
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
                    ctx.local = super::budget::policy::provider_is_local(m.provider.as_str());
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
    }

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

#[cfg(test)]
mod policy_gate_tests {
    use super::*;
    use crate::core::llm::audit::Actor;
    use crate::core::llm::budget::policy::{BudgetPolicy, Engine, Metric, Scope, TurnCtx, Window};

    struct Nop;

    #[async_trait::async_trait]
    impl super::super::broker::ToolExecutor for Nop {
        async fn execute(&self, _name: &str, _input: Value) -> Result<String, LlmError> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn a_turn_cancelled_before_the_question_does_not_wait() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(&dir.path().join("budget.sqlite")).unwrap();
        engine.upsert(usd_policy(), Actor::system("test")).unwrap();
        let bus = Arc::new(Bus::new());
        let broker = Arc::new(ToolBroker::new(Vec::new(), Arc::new(Nop), bus.clone()));
        let coordinator = Coordinator::new(
            Arc::new(super::super::runtime::NativeRuntime::new()),
            broker.clone(),
            Arc::new(BudgetStore::at(dir.path().join("pool.json"))),
            bus,
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            coordinator.policy_gate(&engine, &ctx(), &cancel),
        )
        .await
        .expect("an already cancelled turn must not wait for an answer");
        assert_eq!(
            out.unwrap_err().code,
            super::super::error::ERR_LLM_CANCELLED
        );
        assert!(!broker.has_pending("r-gate"));
    }

    fn usd_policy() -> BudgetPolicy {
        BudgetPolicy {
            id: String::new(),
            scope: Scope::Global,
            metric: Metric::Usd,
            window: Window::Day,
            amount: 5.0,
            warn_pct: None,
            hard_stop: true,
            enabled: true,
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn ctx() -> TurnCtx {
        TurnCtx {
            request_id: "r-gate".into(),
            agent_id: "omni".into(),
            account: None,
            model: "a-model-with-no-price".into(),
            conversation_id: "job-test".into(),
            estimate_tokens: 10,
            local: false,
        }
    }

    #[tokio::test]
    async fn a_cancelled_turn_stops_waiting_for_a_budget_answer() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(&dir.path().join("budget.sqlite")).unwrap();
        engine.upsert(usd_policy(), Actor::system("test")).unwrap();
        let bus = Arc::new(Bus::new());
        let broker = Arc::new(ToolBroker::new(Vec::new(), Arc::new(Nop), bus.clone()));
        let mut events = bus.subscribe();
        let coordinator = Coordinator::new(
            Arc::new(super::super::runtime::NativeRuntime::new()),
            broker.clone(),
            Arc::new(BudgetStore::at(dir.path().join("pool.json"))),
            bus,
        );
        let ctx = ctx();
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            stop.cancel();
        });
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            coordinator.policy_gate(&engine, &ctx, &cancel),
        )
        .await
        .expect("the budget question must end when the turn is cancelled");
        assert_eq!(
            out.unwrap_err().code,
            super::super::error::ERR_LLM_CANCELLED
        );
        assert!(!broker.has_pending(&ctx.request_id));
        let mut resolved = false;
        while let Ok(e) = events.try_recv() {
            resolved |= matches!(e, BusEvent::ToolAskResolved { ref request_id, .. } if request_id == "r-gate");
        }
        assert!(resolved, "the window must be told the question is gone");
    }
}
