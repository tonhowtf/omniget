//! Tool broker: one door for internal tools, MCP servers and skills, with a
//! per-agent grant. Owned by f2-llm-coordinator.
//!
//! Phase 3 shape: the broker holds one spec list fed by three sources —
//! [`ToolBroker::register_internal`] (the shared `tool_table`),
//! [`ToolBroker::register_mcp`] (one external MCP server) and
//! [`ToolBroker::register_skills`] — each with its own [`ToolExecutor`]. The
//! name decides the route: `mcp:<server>:<tool>` goes to that server's
//! executor, `skill:<name>` to the skills executor, anything else to the
//! internal one. That is the same key [`grant_key`] produces, so a grant, a
//! spec and a route can never drift apart.
//!
//! Registration takes `&self`: a server added in the UI shows up in a broker
//! that is already inside an `Arc`, without rebuilding the coordinator.
//!
//! `GrantMode::Ask` emits `BusEvent::ToolAsk` and waits for
//! [`ToolBroker::answer`] until `ask_timeout`; a timeout denies the call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::oneshot;

use super::agent::{GrantMode, ToolGrant, ToolSource};
use super::error::LlmError;
use super::types::ToolSpec;
use crate::core::omni::bus::{Bus, BusEvent};

/// The agent is not allowed to call this tool.
pub const ERR_TOOL_DENIED: &str = "ERR_TOOL_DENIED";
/// The tool is not in the table the broker was given.
pub const ERR_TOOL_UNKNOWN: &str = "ERR_TOOL_UNKNOWN";
/// Nobody answered a `GrantMode::Ask` prompt in time.
pub const ERR_TOOL_TIMEOUT: &str = "ERR_TOOL_TIMEOUT";

/// How long an `Ask` grant waits for the user before denying.
pub const DEFAULT_ASK_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs one tool call. The app implements it over `omniget_core::core::tools::*`
/// plus the MCP client; the tests implement it in three lines.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError>;
}

/// Stable key of a grant, matched against the tool name the model emitted.
/// Internal tools keep their bare name; MCP and skills are namespaced so an
/// MCP server can never shadow an internal tool.
pub fn grant_key(source: &ToolSource) -> String {
    match source {
        ToolSource::Internal { name } => name.clone(),
        ToolSource::Mcp { server, tool } => format!("mcp:{server}:{tool}"),
        ToolSource::Skill { name } => format!("skill:{name}"),
    }
}

/// What the user said to an `Ask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Deny,
    Once,
    /// Allow, and store a rule so the same kind of call stops asking.
    Always,
}

/// Result of one brokered call, for telemetry.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub name: String,
    pub ok: bool,
    pub ms: u32,
    pub content: String,
}

/// The prefix of the internal source: internal tools keep their bare name.
const INTERNAL_PREFIX: &str = "";
/// The prefix every skill tool carries, matching [`grant_key`].
pub const SKILL_PREFIX: &str = "skill:";

/// The prefix every tool of one MCP server carries, matching [`grant_key`].
pub fn mcp_prefix(server: &str) -> String {
    format!("mcp:{server}:")
}

/// Prefix every spec name once. A spec that already carries the prefix (a
/// source that namespaced its own names) is left alone.
fn namespaced(prefix: &str, specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    specs
        .into_iter()
        .map(|mut spec| {
            if !spec.name.starts_with(prefix) {
                spec.name = format!("{prefix}{}", spec.name);
            }
            spec
        })
        .collect()
}

/// One place tools come from: the internal table, one MCP server, the skills.
struct Source {
    /// Route key. Also the id a later `register_*` replaces.
    prefix: String,
    executor: Arc<dyn ToolExecutor>,
    specs: Vec<ToolSpec>,
}

/// One question waiting for the user. `permission` is the durable
/// `PermissionRequest` of the run (when a run registry is active): the answer
/// must resolve that exact request, once and in time, before it releases the
/// waiting call.
struct PendingAsk {
    tx: oneshot::Sender<Answer>,
    request_id: String,
    permission: Option<(crate::core::assist::runs::Registry, String)>,
}

/// Why an answer did not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerRefused {
    /// No such question is waiting (already answered, never asked).
    Unknown,
    /// The question ran out of time; the late answer applies to nothing.
    Expired,
    /// The question already had its answer, or its run was cancelled.
    Resolved,
}

pub struct ToolBroker {
    sources: RwLock<Vec<Source>>,
    bus: Arc<Bus>,
    pending: Mutex<HashMap<String, PendingAsk>>,
    ask_timeout: Duration,
    /// The run registry of the last durable question, so a late answer can
    /// still be told apart (expired vs resolved) after its waiter left.
    last_registry: Mutex<Option<crate::core::assist::runs::Registry>>,
}

impl std::fmt::Debug for ToolBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("ToolBroker")
            .field("sources", &sources.len())
            .field(
                "specs",
                &sources.iter().map(|s| s.specs.len()).sum::<usize>(),
            )
            .field("ask_timeout", &self.ask_timeout)
            .finish()
    }
}

impl ToolBroker {
    /// The internal source, ready to run. `register_internal` replaces it
    /// later without touching anything else.
    pub fn new(specs: Vec<ToolSpec>, executor: Arc<dyn ToolExecutor>, bus: Arc<Bus>) -> Self {
        let broker = Self {
            sources: RwLock::new(Vec::new()),
            bus,
            pending: Mutex::new(HashMap::new()),
            ask_timeout: DEFAULT_ASK_TIMEOUT,
            last_registry: Mutex::new(None),
        };
        broker.register_internal(specs, executor);
        broker
    }

    pub fn with_ask_timeout(mut self, timeout: Duration) -> Self {
        self.ask_timeout = timeout;
        self
    }

    // ── Sources ───────────────────────────────────────────────────────

    /// Replace the internal source (the shared `tool_table`). Returns how many
    /// specs it now offers.
    pub fn register_internal(
        &self,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        self.register(INTERNAL_PREFIX.to_string(), specs, executor)
    }

    /// Replace the tools of one MCP server. `specs` carry the bare names the
    /// server reported; the broker namespaces them to `mcp:<server>:<tool>`,
    /// and the executor is called with that full name.
    pub fn register_mcp(
        &self,
        server: &str,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        let prefix = mcp_prefix(server);
        let specs = namespaced(&prefix, specs);
        self.register(prefix, specs, executor)
    }

    /// Replace the skill tools. `specs` carry the bare skill names; the broker
    /// namespaces them to `skill:<name>`.
    pub fn register_skills(&self, specs: Vec<ToolSpec>, executor: Arc<dyn ToolExecutor>) -> usize {
        let specs = namespaced(SKILL_PREFIX, specs);
        self.register(SKILL_PREFIX.to_string(), specs, executor)
    }

    /// Replace the tools of a named source whose names are already final
    /// (no namespacing): the assistant tools (`assist`), for one.
    pub fn register_source(
        &self,
        prefix: &str,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        self.register(prefix.to_string(), specs, executor)
    }

    /// Drop a source registered with [`Self::register_source`].
    pub fn unregister_source(&self, prefix: &str) -> usize {
        self.unregister(prefix)
    }

    /// Drop one MCP server (disabled, removed, or failed to connect). Returns
    /// how many specs went away.
    pub fn unregister_mcp(&self, server: &str) -> usize {
        self.unregister(&mcp_prefix(server))
    }

    /// Drop every skill tool.
    pub fn unregister_skills(&self) -> usize {
        self.unregister(SKILL_PREFIX)
    }

    /// The prefixes currently registered, in order, for diagnostics.
    pub fn sources(&self) -> Vec<String> {
        self.sources
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|s| s.prefix.clone())
            .collect()
    }

    fn register(
        &self,
        prefix: String,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        let mut sources = self.sources.write().unwrap_or_else(|e| e.into_inner());
        // A name another source already owns is dropped, not shadowed: the
        // internal table wins over a server that picked the same name.
        let taken: std::collections::HashSet<String> = sources
            .iter()
            .filter(|s| s.prefix != prefix)
            .flat_map(|s| s.specs.iter().map(|spec| spec.name.clone()))
            .collect();
        let mut seen = std::collections::HashSet::new();
        let specs: Vec<ToolSpec> = specs
            .into_iter()
            .filter(|spec| !taken.contains(&spec.name) && seen.insert(spec.name.clone()))
            .collect();
        let count = specs.len();
        let source = Source {
            prefix: prefix.clone(),
            executor,
            specs,
        };
        match sources.iter_mut().find(|s| s.prefix == prefix) {
            Some(slot) => *slot = source,
            None => sources.push(source),
        }
        count
    }

    fn unregister(&self, prefix: &str) -> usize {
        let mut sources = self.sources.write().unwrap_or_else(|e| e.into_inner());
        let mut gone = 0;
        sources.retain(|s| {
            if s.prefix == prefix {
                gone += s.specs.len();
                false
            } else {
                true
            }
        });
        gone
    }

    /// The executor that owns this tool name, if any source does.
    fn executor_for(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        sources
            .iter()
            .find(|s| s.specs.iter().any(|spec| spec.name == name))
            .map(|s| s.executor.clone())
    }

    /// Every tool on offer, sources in registration order.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.sources
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .flat_map(|s| s.specs.iter().cloned())
            .collect()
    }

    /// The tools this agent may see, in the order the table has them. A tool
    /// with no grant, or with `Deny`, is not offered to the model at all.
    pub fn specs_for(&self, grants: &[ToolGrant]) -> Vec<ToolSpec> {
        self.specs()
            .into_iter()
            .filter(|spec| !matches!(self.mode_for(grants, &spec.name), GrantMode::Deny))
            .collect()
    }

    /// `Deny` unless the agent has a grant for this exact tool. Closed by
    /// default on purpose.
    pub fn mode_for(&self, grants: &[ToolGrant], name: &str) -> GrantMode {
        grants
            .iter()
            .find(|g| grant_key(&g.source) == name)
            .map(|g| g.mode)
            .unwrap_or(GrantMode::Deny)
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.executor_for(name).is_some()
    }

    /// Answer a pending `Ask`. Returns false when the id is unknown (already
    /// answered, or timed out).
    pub fn answer(&self, tool_call_id: &str, allow: bool) -> bool {
        self.answer_with(
            tool_call_id,
            if allow { Answer::Once } else { Answer::Deny },
        )
    }

    pub fn answer_with(&self, tool_call_id: &str, answer: Answer) -> bool {
        self.deliver(tool_call_id, answer).is_ok()
    }

    /// Answers by the durable permission id (the Activity list, the
    /// projection). A late answer is refused as expired and releases nothing.
    pub fn answer_permission(
        &self,
        permission_id: &str,
        answer: Answer,
    ) -> Result<(), AnswerRefused> {
        let key = {
            let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending
                .iter()
                .find(|(_, p)| {
                    p.permission.as_ref().map(|(_, id)| id.as_str()) == Some(permission_id)
                })
                .map(|(k, _)| k.clone())
        };
        match key {
            Some(k) => self.deliver(&k, answer),
            None => {
                // Nobody waits: tell expired from resolved if the record knows.
                let reg = crate::core::assist::runs::active().or_else(|| {
                    self.last_registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                });
                let state = reg
                    .and_then(|r| r.permission(permission_id))
                    .map(|p| p.state);
                use crate::core::assist::runs::PermissionState as S;
                Err(match state {
                    Some(S::Expired) => AnswerRefused::Expired,
                    Some(S::Pending) | None => AnswerRefused::Unknown,
                    Some(_) => AnswerRefused::Resolved,
                })
            }
        }
    }

    fn deliver(&self, tool_call_id: &str, answer: Answer) -> Result<(), AnswerRefused> {
        let entry = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.remove(tool_call_id)
        };
        let Some(entry) = entry else {
            return Err(AnswerRefused::Unknown);
        };
        if let Some((reg, id)) = &entry.permission {
            let word = match answer {
                Answer::Deny => "deny",
                Answer::Once => "once",
                Answer::Always => "always",
            };
            if let Err(e) = reg.answer_permission(id, word) {
                // Dropping `entry.tx` makes the waiter read "no".
                return Err(
                    if e.starts_with(crate::core::assist::runs::ERR_PERMISSION_EXPIRED) {
                        AnswerRefused::Expired
                    } else {
                        AnswerRefused::Resolved
                    },
                );
            }
        }
        entry.tx.send(answer).map_err(|_| AnswerRefused::Expired)
    }

    /// Cancels every question of one turn: each resolves as `cancelled` in
    /// the run record and the waiting calls read "no". Returns how many.
    pub fn cancel_request(&self, request_id: &str) -> usize {
        let gone: Vec<PendingAsk> = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let keys: Vec<String> = pending
                .iter()
                .filter(|(_, p)| p.request_id == request_id)
                .map(|(k, _)| k.clone())
                .collect();
            keys.into_iter()
                .filter_map(|k| pending.remove(&k))
                .collect()
        };
        let n = gone.len();
        let mut done = std::collections::HashSet::new();
        for p in gone {
            if let Some((reg, _)) = &p.permission {
                if done.insert(p.request_id.clone()) {
                    let _ = reg.cancel_permissions(&p.request_id);
                }
            }
            drop(p.tx);
        }
        n
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Run one tool call for `agent_id`, honouring the grant. `request_id` is
    /// the id of the turn the call belongs to (the one the coordinator put in
    /// `TurnEvent::Started`), so the UI can tie an ask back to its turn. Emits
    /// `ToolCalled` and `ToolFinished` on the bus either way.
    pub async fn call(
        &self,
        agent_id: &str,
        grants: &[ToolGrant],
        request_id: &str,
        tool_call_id: &str,
        name: &str,
        input: Value,
    ) -> Result<ToolOutcome, LlmError> {
        let start = Instant::now();
        let external_context = super::code_tools::current_turn()
            .filter(|c| crate::core::assist::authority::external(&c.conversation));
        if let Some(ctx) = super::code_tools::current_turn() {
            if let Err(reason) =
                crate::core::assist::authority::check_tool(&ctx.conversation, agent_id, name)
            {
                return Err(self.fail(
                    agent_id,
                    name,
                    start,
                    LlmError::new(ERR_TOOL_DENIED, reason),
                ));
            }
        }
        let Some(executor) = self.executor_for(name) else {
            return Err(self.fail(
                agent_id,
                name,
                start,
                LlmError::new(ERR_TOOL_UNKNOWN, format!("unknown tool `{name}`")),
            ));
        };
        let mut mode = self.mode_for(grants, name);
        if !matches!(mode, GrantMode::Deny) {
            match super::perm::decide(agent_id, name, &input) {
                Some(super::perm::Action::Allow) => mode = GrantMode::Auto,
                Some(super::perm::Action::Ask) => mode = GrantMode::Ask,
                Some(super::perm::Action::Deny) => mode = GrantMode::Deny,
                None => {}
            }
            // A path outside the workspace is its own question, whatever the
            // grant says; a yes covers this one call.
            let path = input.get("path").and_then(Value::as_str).unwrap_or("");
            if external_context.is_none()
                && name.starts_with("fs_")
                && super::code_tools::is_external(path)
            {
                let ok = self
                    .ask(
                        agent_id,
                        request_id,
                        tool_call_id,
                        "external_directory",
                        &Value::String(format!("{name} {path}")),
                    )
                    .await;
                if !matches!(ok, Answer::Deny) {
                    super::code_tools::allow_external_once();
                    mode = GrantMode::Auto;
                } else {
                    mode = GrantMode::Deny;
                }
            }
        }
        match mode {
            GrantMode::Deny => {
                return Err(self.fail(
                    agent_id,
                    name,
                    start,
                    LlmError::new(ERR_TOOL_DENIED, format!("`{name}` is not granted")),
                ))
            }
            GrantMode::Ask => {
                let answer = self
                    .ask(agent_id, request_id, tool_call_id, name, &input)
                    .await;
                // An external run's "Always" covers this call only: it never
                // becomes a permanent local rule of the executor (L2).
                if matches!(answer, Answer::Always) && external_context.is_none() {
                    super::perm::add_rule(agent_id, super::perm::always_rule(name, &input));
                }
                if matches!(answer, Answer::Deny) {
                    return Err(self.fail(
                        agent_id,
                        name,
                        start,
                        LlmError::new(ERR_TOOL_DENIED, format!("`{name}` was refused")),
                    ));
                }
            }
            GrantMode::Auto => {}
        }

        // A mission attached to this conversation may take a permission
        // away (a `before_tool` policy), never add one; its round log sees
        // every result.
        let mission_conv = super::code_tools::current_turn()
            .map(|c| c.conversation.clone())
            .filter(|c| crate::core::assist::missions::hooks::mission_of(c).is_some());
        let mission_input = mission_conv.as_ref().map(|_| input.clone());
        if let (Some(conv), Some(inp)) = (&mission_conv, &mission_input) {
            if let Some(reason) = crate::core::assist::missions::hooks::before_tool(conv, name, inp)
            {
                return Err(self.fail(
                    agent_id,
                    name,
                    start,
                    LlmError::new(ERR_TOOL_DENIED, reason),
                ));
            }
        }

        if external_context.is_none() && super::code_tools::WRITE_TOOLS.contains(&name) {
            if let (Some(ctx), Some(ws)) = (
                super::code_tools::current_turn(),
                super::code_tools::workspace(),
            ) {
                if let Err(e) =
                    super::snapshot::before_write(&ws, &ctx.conversation, &ctx.request).await
                {
                    tracing::warn!("[llm] snapshot before {name} failed: {e}");
                }
            }
        }

        if let Some(ctx) = super::code_tools::current_turn() {
            if let Err(reason) =
                crate::core::assist::authority::check_tool(&ctx.conversation, agent_id, name)
            {
                return Err(self.fail(
                    agent_id,
                    name,
                    start,
                    LlmError::new(ERR_TOOL_DENIED, reason),
                ));
            }
        }
        let result = if let Some(ctx) = external_context.filter(|_| name.starts_with("fs_")) {
            let conversation = ctx.conversation.clone();
            let bot = agent_id.to_owned();
            let tool = name.to_owned();
            tokio::task::spawn_blocking(move || {
                crate::core::assist::authority::execute_files(&conversation, &bot, &tool, &input)
            })
            .await
            .map_err(|_| LlmError::new(ERR_TOOL_DENIED, "external file executor interrupted"))
            .and_then(|r| r.map_err(|reason| LlmError::new(ERR_TOOL_DENIED, reason)))
        } else {
            super::code_tools::scope_tool_call(tool_call_id, executor.execute(name, input)).await
        };
        let ms = start.elapsed().as_millis() as u32;
        if let (Some(conv), Some(inp)) = (&mission_conv, &mission_input) {
            crate::core::assist::missions::hooks::after_tool(conv, name, inp, result.is_ok(), ms);
        }
        match result {
            Ok(content) => {
                self.bus.emit(BusEvent::ToolCalled {
                    agent: agent_id.to_string(),
                    tool: name.to_string(),
                    ok: true,
                    ms,
                });
                self.bus.emit(BusEvent::ToolFinished {
                    tool: name.to_string(),
                    ok: true,
                });
                Ok(ToolOutcome {
                    name: name.to_string(),
                    ok: true,
                    ms,
                    content,
                })
            }
            Err(err) => Err(self.fail(agent_id, name, start, err)),
        }
    }

    fn fail(&self, agent_id: &str, name: &str, start: Instant, err: LlmError) -> LlmError {
        self.bus.emit(BusEvent::ToolCalled {
            agent: agent_id.to_string(),
            tool: name.to_string(),
            ok: false,
            ms: start.elapsed().as_millis() as u32,
        });
        self.bus.emit(BusEvent::ToolFinished {
            tool: name.to_string(),
            ok: false,
        });
        err
    }

    /// The same prompt for a caller that is not a brokered tool (an ACP agent
    /// asking permission for one of its own tools).
    pub async fn ask_user(
        &self,
        agent_id: &str,
        request_id: &str,
        tool_call_id: &str,
        label: &str,
        preview: &str,
    ) -> Answer {
        self.ask(
            agent_id,
            request_id,
            tool_call_id,
            label,
            &Value::String(preview.to_string()),
        )
        .await
    }

    /// Ask the user and wait. Returns false on refusal, on timeout and when
    /// nobody is listening on the bus (no UI, no consent).
    async fn ask(
        &self,
        agent_id: &str,
        request_id: &str,
        tool_call_id: &str,
        name: &str,
        input: &Value,
    ) -> Answer {
        let (tx, rx) = oneshot::channel();
        let preview_text = match input {
            Value::String(s) => s.clone(),
            _ => super::code_tools::preview(name, input),
        };
        // The durable request, when the turn has a run record: the answer
        // must resolve exactly this one, in time.
        let deadline = crate::core::assist::now_ms()
            + self.ask_timeout.as_millis().min(i64::MAX as u128) as i64;
        let permission = crate::core::assist::runs::active().and_then(|reg| {
            reg.run(request_id)?;
            let req = reg
                .open_permission(
                    request_id,
                    tool_call_id,
                    name,
                    &preview_text,
                    &["once", "always", "deny"],
                    deadline,
                )
                .ok()?;
            Some((reg, req.id))
        });
        let permission_id = permission.as_ref().map(|(_, id)| id.clone());
        if let Some((reg, _)) = &permission {
            *self.last_registry.lock().unwrap_or_else(|e| e.into_inner()) = Some(reg.clone());
        }
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.insert(
                tool_call_id.to_string(),
                PendingAsk {
                    tx,
                    request_id: request_id.to_string(),
                    permission: permission.clone(),
                },
            );
        }
        self.bus.emit(BusEvent::ToolAsk {
            agent: agent_id.to_string(),
            request_id: request_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            tool: name.to_string(),
            preview: preview_text,
        });
        let allowed = match tokio::time::timeout(self.ask_timeout, rx).await {
            Ok(Ok(allow)) => allow,
            // Sender dropped or the wait expired: both mean "no".
            _ => Answer::Deny,
        };
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            // Only our own entry: a newer question under the same id stays.
            let ours = pending
                .get(tool_call_id)
                .map(|p| p.permission.as_ref().map(|(_, id)| id.clone()) == permission_id)
                .unwrap_or(false);
            if ours {
                pending.remove(tool_call_id);
            }
        }
        // Timed out: the record says expired, so a late answer is refused.
        if let Some((reg, id)) = &permission {
            if let Some(p) = reg.permission(id) {
                if p.state == crate::core::assist::runs::PermissionState::Pending {
                    let _ = reg.expire_permission(id);
                }
            }
        }
        allowed
    }

    /// The code the coordinator reports when an `Ask` expires.
    pub fn timeout_error(name: &str) -> LlmError {
        LlmError::new(ERR_TOOL_TIMEOUT, format!("nobody answered for `{name}`"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct EchoExecutor;

    #[async_trait]
    impl ToolExecutor for EchoExecutor {
        async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError> {
            if name == "boom" {
                return Err(LlmError::new(ERR_TOOL_UNKNOWN, "exploded"));
            }
            Ok(format!("{name}:{input}"))
        }
    }

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("the {name} tool"),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn broker() -> Arc<ToolBroker> {
        Arc::new(ToolBroker::new(
            vec![spec("dl_add"), spec("mcp:files:read"), spec("boom")],
            Arc::new(EchoExecutor),
            Arc::new(Bus::new()),
        ))
    }

    fn grant(source: ToolSource, mode: GrantMode) -> ToolGrant {
        ToolGrant { source, mode }
    }

    fn auto(name: &str) -> ToolGrant {
        grant(ToolSource::Internal { name: name.into() }, GrantMode::Auto)
    }

    #[test]
    fn grant_key_namespaces_mcp_and_skills() {
        assert_eq!(
            grant_key(&ToolSource::Internal {
                name: "dl_add".into()
            }),
            "dl_add"
        );
        assert_eq!(
            grant_key(&ToolSource::Mcp {
                server: "files".into(),
                tool: "read".into()
            }),
            "mcp:files:read"
        );
        assert_eq!(
            grant_key(&ToolSource::Skill { name: "pdf".into() }),
            "skill:pdf"
        );
    }

    #[test]
    fn a_tool_without_a_grant_is_denied_and_hidden() {
        let b = broker();
        assert_eq!(b.mode_for(&[], "dl_add"), GrantMode::Deny);
        assert!(b.specs_for(&[]).is_empty());
        assert_eq!(b.specs_for(&[auto("dl_add")]).len(), 1);
    }

    #[tokio::test]
    async fn auto_grant_runs_the_tool() {
        let b = broker();
        let out = b
            .call(
                "agent",
                &[auto("dl_add")],
                "req-1",
                "call-1",
                "dl_add",
                json!({"url": "x"}),
            )
            .await
            .unwrap();
        assert!(out.ok);
        assert!(out.content.starts_with("dl_add:"));
    }

    #[tokio::test]
    async fn an_unknown_tool_fails_with_its_own_code() {
        let b = broker();
        let err = b
            .call(
                "agent",
                &[auto("nope")],
                "req-1",
                "call-1",
                "nope",
                json!({}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ERR_TOOL_UNKNOWN);
    }

    #[tokio::test]
    async fn a_denied_grant_never_reaches_the_executor() {
        let b = broker();
        let grants = vec![grant(
            ToolSource::Internal {
                name: "dl_add".into(),
            },
            GrantMode::Deny,
        )];
        let err = b
            .call("agent", &grants, "req-1", "call-1", "dl_add", json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ERR_TOOL_DENIED);
    }

    #[tokio::test]
    async fn the_bus_sees_every_call_ok_or_not() {
        let bus = Arc::new(Bus::new());
        let b = ToolBroker::new(
            vec![spec("dl_add"), spec("boom")],
            Arc::new(EchoExecutor),
            bus.clone(),
        );
        let mut rx = bus.subscribe();
        b.call(
            "agent",
            &[auto("dl_add")],
            "req-1",
            "c1",
            "dl_add",
            json!({}),
        )
        .await
        .unwrap();
        b.call("agent", &[auto("boom")], "req-1", "c2", "boom", json!({}))
            .await
            .unwrap_err();
        let mut calls = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let BusEvent::ToolCalled { tool, ok, .. } = ev {
                calls.push((tool, ok));
            }
        }
        assert_eq!(calls, vec![("dl_add".into(), true), ("boom".into(), false)]);
    }

    #[tokio::test]
    async fn ask_emits_tool_ask_and_runs_once_allowed() {
        let bus = Arc::new(Bus::new());
        let b = Arc::new(ToolBroker::new(
            vec![spec("dl_add")],
            Arc::new(EchoExecutor),
            bus.clone(),
        ));
        let mut rx = bus.subscribe();
        let grants = vec![grant(
            ToolSource::Internal {
                name: "dl_add".into(),
            },
            GrantMode::Ask,
        )];
        let b2 = b.clone();
        let task = tokio::spawn(async move {
            b2.call("agent", &grants, "req-9", "call-9", "dl_add", json!({}))
                .await
        });
        // The UI sees the ask, then answers.
        let ev = rx.recv().await.unwrap();
        match &ev {
            BusEvent::ToolAsk {
                request_id,
                tool_call_id,
                tool,
                ..
            } => {
                assert_eq!(request_id, "req-9", "the turn id, not the tool call id");
                assert_eq!(tool_call_id, "call-9");
                assert_eq!(tool, "dl_add");
            }
            other => panic!("unexpected event {other:?}"),
        }
        while !b.answer("call-9", true) {
            tokio::task::yield_now().await;
        }
        let out = task.await.unwrap().unwrap();
        assert!(out.ok);
        assert_eq!(b.pending_count(), 0);
    }

    #[tokio::test]
    async fn ask_refused_denies_the_call() {
        let b = Arc::new(ToolBroker::new(
            vec![spec("dl_add")],
            Arc::new(EchoExecutor),
            Arc::new(Bus::new()),
        ));
        let grants = vec![grant(
            ToolSource::Internal {
                name: "dl_add".into(),
            },
            GrantMode::Ask,
        )];
        let b2 = b.clone();
        let task = tokio::spawn(async move {
            b2.call("agent", &grants, "req-9", "call-9", "dl_add", json!({}))
                .await
        });
        while !b.answer("call-9", false) {
            tokio::task::yield_now().await;
        }
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.code, ERR_TOOL_DENIED);
    }

    #[tokio::test]
    async fn ask_denies_when_nobody_answers_in_time() {
        let b = ToolBroker::new(
            vec![spec("dl_add")],
            Arc::new(EchoExecutor),
            Arc::new(Bus::new()),
        )
        .with_ask_timeout(Duration::from_millis(50));
        let grants = vec![grant(
            ToolSource::Internal {
                name: "dl_add".into(),
            },
            GrantMode::Ask,
        )];
        let err = b
            .call("agent", &grants, "req-9", "call-9", "dl_add", json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ERR_TOOL_DENIED);
        assert_eq!(b.pending_count(), 0);
    }

    #[test]
    fn answering_an_unknown_id_is_false_not_a_panic() {
        assert!(!broker().answer("nope", true));
    }

    // ── Three sources (Phase 3) ───────────────────────────────────────

    /// An executor that reports which source ran the call.
    struct Tagged(&'static str);

    #[async_trait]
    impl ToolExecutor for Tagged {
        async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
            Ok(format!("{}|{name}", self.0))
        }
    }

    fn three_sources() -> Arc<ToolBroker> {
        let b = ToolBroker::new(
            vec![spec("dl_add")],
            Arc::new(Tagged("internal")),
            Arc::new(Bus::new()),
        );
        b.register_mcp("files", vec![spec("read")], Arc::new(Tagged("mcp")));
        b.register_skills(vec![spec("pdf")], Arc::new(Tagged("skills")));
        Arc::new(b)
    }

    #[test]
    fn the_three_sources_namespace_their_names_like_grant_key() {
        let b = three_sources();
        let names: Vec<String> = b.specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["dl_add", "mcp:files:read", "skill:pdf"]);
        assert_eq!(
            names[1],
            grant_key(&ToolSource::Mcp {
                server: "files".into(),
                tool: "read".into()
            })
        );
        assert_eq!(
            names[2],
            grant_key(&ToolSource::Skill { name: "pdf".into() })
        );
    }

    #[tokio::test]
    async fn each_source_runs_its_own_tools() {
        let b = three_sources();
        for (name, want) in [
            ("dl_add", "internal|dl_add"),
            ("mcp:files:read", "mcp|mcp:files:read"),
            ("skill:pdf", "skills|skill:pdf"),
        ] {
            let out = b
                .call("agent", &[auto(name)], "req", "call", name, json!({}))
                .await
                .unwrap();
            assert_eq!(out.content, want);
        }
    }

    #[test]
    fn a_server_cannot_shadow_an_internal_tool() {
        let b = ToolBroker::new(
            vec![spec("dl_add")],
            Arc::new(Tagged("internal")),
            Arc::new(Bus::new()),
        );
        // Namespacing already makes shadowing impossible; a second source
        // claiming the same full name is dropped too.
        assert_eq!(
            b.register_skills(vec![spec("skill:dup"), spec("dup")], Arc::new(Tagged("s"))),
            1
        );
        let names: Vec<String> = b.specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["dl_add", "skill:dup"]);
    }

    #[test]
    fn registering_the_same_source_twice_replaces_it() {
        let b = three_sources();
        assert_eq!(
            b.register_mcp(
                "files",
                vec![spec("read"), spec("write")],
                Arc::new(Tagged("mcp"))
            ),
            2
        );
        assert_eq!(b.specs().len(), 4);
        assert_eq!(b.sources(), vec!["", "mcp:files:", "skill:"]);
    }

    #[tokio::test]
    async fn unregistering_a_server_takes_its_tools_with_it() {
        let b = three_sources();
        assert_eq!(b.unregister_mcp("files"), 1);
        assert!(!b.has_tool("mcp:files:read"));
        let err = b
            .call(
                "agent",
                &[auto("mcp:files:read")],
                "req",
                "call",
                "mcp:files:read",
                json!({}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ERR_TOOL_UNKNOWN);
        assert_eq!(b.unregister_skills(), 1);
        assert_eq!(b.specs().len(), 1);
    }

    #[test]
    fn register_internal_replaces_the_table_in_place() {
        let b = three_sources();
        assert_eq!(
            b.register_internal(
                vec![spec("dl_add"), spec("dl_pause")],
                Arc::new(Tagged("internal"))
            ),
            2
        );
        let names: Vec<String> = b.specs().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec!["dl_add", "dl_pause", "mcp:files:read", "skill:pdf"]
        );
    }
}

/// A11 through the broker: each answer resolves the durable request it
/// names, once; a late answer after the deadline applies to nothing and never
/// releases the next tool; cancelling the turn cancels what waits.
#[cfg(test)]
mod a11_permission_tests {
    use super::*;
    use crate::core::assist::db::AssistDb;
    use crate::core::assist::runs::{NewRun, PermissionState, Registry, RunState};
    use serde_json::json;

    struct Echo;

    #[async_trait]
    impl ToolExecutor for Echo {
        async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
            Ok(format!("ran {name}"))
        }
    }

    fn setup(timeout_ms: u64) -> (Arc<ToolBroker>, Registry) {
        let spec = |n: &str| ToolSpec {
            name: n.into(),
            description: n.into(),
            input_schema: json!({"type":"object"}),
        };
        let broker = Arc::new(
            ToolBroker::new(
                vec![spec("tool_a"), spec("tool_b")],
                Arc::new(Echo),
                Arc::new(Bus::new()),
            )
            .with_ask_timeout(Duration::from_millis(timeout_ms)),
        );
        let reg =
            Registry::with_instance(Arc::new(AssistDb::open_in_memory().unwrap()), "boot-a11");
        reg.open_run(NewRun {
            id: "run-1".into(),
            conversation_id: "c".into(),
            bot_id: "a11-agent".into(),
            ..Default::default()
        })
        .unwrap();
        reg.transition("run-1", RunState::Running, None).unwrap();
        (broker, reg)
    }

    fn ask_grants() -> Vec<ToolGrant> {
        ["tool_a", "tool_b"]
            .iter()
            .map(|n| ToolGrant {
                source: ToolSource::Internal { name: (*n).into() },
                mode: GrantMode::Ask,
            })
            .collect()
    }

    fn spawn_call(
        broker: &Arc<ToolBroker>,
        reg: &Registry,
        call: &'static str,
        tool: &'static str,
    ) -> tokio::task::JoinHandle<Result<ToolOutcome, LlmError>> {
        let (b, r) = (broker.clone(), reg.clone());
        tokio::spawn(async move {
            crate::core::assist::runs::scope(
                Some(r),
                b.call("a11-agent", &ask_grants(), "run-1", call, tool, json!({})),
            )
            .await
        })
    }

    async fn pending_of(reg: &Registry, call: &str) -> String {
        for _ in 0..400 {
            if let Some(p) = reg
                .permissions(Some("run-1"), true)
                .into_iter()
                .find(|p| p.tool_call_id == call)
            {
                return p.id;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("no pending permission for {call}");
    }

    #[tokio::test]
    async fn approved_and_denied_apply_once_to_their_own_call() {
        let (broker, reg) = setup(5_000);
        let task = spawn_call(&broker, &reg, "call-1", "tool_a");
        let id = pending_of(&reg, "call-1").await;
        assert_eq!(reg.run("run-1").unwrap().state, RunState::WaitingUser);
        broker.answer_permission(&id, Answer::Once).unwrap();
        assert!(task.await.unwrap().unwrap().ok);
        assert_eq!(
            reg.permission(&id).unwrap().state,
            PermissionState::Approved
        );
        assert_eq!(reg.run("run-1").unwrap().state, RunState::Running);
        // The same answer again applies to nothing.
        assert_eq!(
            broker.answer_permission(&id, Answer::Once),
            Err(AnswerRefused::Resolved)
        );

        let task = spawn_call(&broker, &reg, "call-2", "tool_b");
        let id2 = pending_of(&reg, "call-2").await;
        broker.answer_permission(&id2, Answer::Deny).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ERR_TOOL_DENIED);
        assert_eq!(reg.permission(&id2).unwrap().state, PermissionState::Denied);
    }

    #[tokio::test]
    async fn a_late_answer_is_expired_and_never_releases_the_next_tool() {
        let (broker, reg) = setup(80);
        let task = spawn_call(&broker, &reg, "call-1", "tool_a");
        let old = pending_of(&reg, "call-1").await;
        // Nobody answers in time.
        assert_eq!(task.await.unwrap().unwrap_err().code, ERR_TOOL_DENIED);
        assert_eq!(
            reg.permission(&old).unwrap().state,
            PermissionState::Expired
        );
        assert_eq!(
            broker.answer_permission(&old, Answer::Once),
            Err(AnswerRefused::Expired)
        );

        // The next tool asks; the stale answers (by id and by old call id)
        // must not release it.
        let broker2 = Arc::new(
            ToolBroker::new(vec![], Arc::new(Echo), Arc::new(Bus::new()))
                .with_ask_timeout(Duration::from_millis(400)),
        );
        broker2.register_internal(
            vec![ToolSpec {
                name: "tool_b".into(),
                description: String::new(),
                input_schema: json!({}),
            }],
            Arc::new(Echo),
        );
        let next = spawn_call(&broker2, &reg, "call-2", "tool_b");
        let fresh = pending_of(&reg, "call-2").await;
        assert!(broker2.answer_permission(&old, Answer::Once).is_err());
        assert!(!broker2.answer("call-1", true));
        assert_eq!(
            reg.permission(&fresh).unwrap().state,
            PermissionState::Pending
        );
        // It runs out on its own: still refused, never approved by the old answer.
        assert_eq!(next.await.unwrap().unwrap_err().code, ERR_TOOL_DENIED);
        assert_eq!(
            reg.permission(&fresh).unwrap().state,
            PermissionState::Expired
        );
    }

    #[tokio::test]
    async fn cancelling_the_turn_cancels_what_waits() {
        let (broker, reg) = setup(5_000);
        let task = spawn_call(&broker, &reg, "call-1", "tool_a");
        let id = pending_of(&reg, "call-1").await;
        assert_eq!(broker.cancel_request("run-1"), 1);
        assert_eq!(task.await.unwrap().unwrap_err().code, ERR_TOOL_DENIED);
        assert_eq!(
            reg.permission(&id).unwrap().state,
            PermissionState::Cancelled
        );
        assert_eq!(
            broker.answer_permission(&id, Answer::Once),
            Err(AnswerRefused::Resolved)
        );
    }
}
