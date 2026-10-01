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
//! [`ToolBroker::answer`] until the ask timeout of its request (the chat's
//! default, or what [`ToolBroker::set_ask_timeout_resolver`] says for a
//! background job). Running out of time is not a refusal: the call fails with
//! `ERR_TOOL_TIMEOUT`, and every ending of an ask — answer, expiry, cancel —
//! goes out as one `BusEvent::ToolAskResolved`, so an expiry is never silent.

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

/// How long an `Ask` grant waits for the user when nothing else was set.
/// The app replaces it with the user's `ask_timeout_chat_s` (30 min by
/// default); 120 s only survives in a broker nobody configured.
pub const DEFAULT_ASK_TIMEOUT: Duration = Duration::from_secs(120);
/// No question ever expires sooner than this, whatever a resolver says (a job
/// one second from its deadline still gets a real chance, and the deadline
/// then ends the job with its own reason).
pub const MIN_ASK_TIMEOUT: Duration = Duration::from_secs(1);

/// Picks the ask timeout of one request (`None` = the default).
pub type AskTimeoutResolver = Arc<dyn Fn(&str) -> Option<Duration> + Send + Sync>;

/// How a question ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskEnd {
    /// A person (or a stored rule) answered.
    Answered(Answer),
    /// Nobody answered within the timeout: the action did not run.
    Expired { waited: Duration },
    /// The turn ended, or the answer could not apply to the durable record.
    Cancelled,
}

impl AskEnd {
    /// What the caller acts on: only an explicit yes allows.
    pub fn answer(self) -> Answer {
        match self {
            AskEnd::Answered(a) => a,
            AskEnd::Expired { .. } | AskEnd::Cancelled => Answer::Deny,
        }
    }

    /// The `outcome` of `BusEvent::ToolAskResolved`.
    pub fn outcome(self) -> &'static str {
        match self {
            AskEnd::Answered(Answer::Deny) => "deny",
            AskEnd::Answered(_) => "allow",
            AskEnd::Expired { .. } => "expired",
            AskEnd::Cancelled => "cancelled",
        }
    }
}

/// The error of a brokered call whose question expired.
pub fn timeout_error(name: &str, waited: Duration) -> LlmError {
    LlmError::new(
        ERR_TOOL_TIMEOUT,
        format!(
            "nobody answered the permission question for `{name}` in {} s; it did not run",
            waited.as_secs()
        ),
    )
}

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

/// A person's answer to a durable ask, in the words of the approvals API
/// (item 2.2): `POST /v1/approvals/{id}`, `omniget agent approve`, MCP
/// `approval_respond`, the window and a paired phone all send one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Allow this call.
    Accept,
    /// Allow this call and the same tool for the rest of this turn.
    AcceptSession,
    /// Allow, and store a rule so the same kind of call stops asking.
    AcceptAlways,
    Decline,
    /// Refuse and stop the turn that asked.
    Cancel,
}

impl Decision {
    /// Every decision, in the order the elicitation payload lists them.
    pub const ALL: [&'static str; 5] = [
        "accept",
        "accept_session",
        "accept_always",
        "decline",
        "cancel",
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "accept" => Decision::Accept,
            "accept_session" => Decision::AcceptSession,
            "accept_always" => Decision::AcceptAlways,
            "decline" => Decision::Decline,
            "cancel" => Decision::Cancel,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Accept => "accept",
            Decision::AcceptSession => "accept_session",
            Decision::AcceptAlways => "accept_always",
            Decision::Decline => "decline",
            Decision::Cancel => "cancel",
        }
    }

    /// The word stored in `runs_permissions.answer`.
    pub fn word(self) -> &'static str {
        match self {
            Decision::Accept => "once",
            Decision::AcceptSession => "session",
            Decision::AcceptAlways => "always",
            Decision::Decline => "deny",
            Decision::Cancel => "cancel",
        }
    }

    /// What the waiting call reads.
    pub fn answer(self) -> Answer {
        match self {
            Decision::Accept | Decision::AcceptSession => Answer::Once,
            Decision::AcceptAlways => Answer::Always,
            Decision::Decline | Decision::Cancel => Answer::Deny,
        }
    }

    fn of_answer(answer: Answer) -> Self {
        match answer {
            Answer::Deny => Decision::Decline,
            Answer::Once => Decision::Accept,
            Answer::Always => Decision::AcceptAlways,
        }
    }
}

/// The question as an MCP `elicitation/create` request (`85#P2`): what a
/// remote client renders and answers with `{decision}`. Stored with the row,
/// served by `GET /v1/approvals`; it carries no ids of the turn.
pub fn elicitation(agent_id: &str, action: &str, preview: &str) -> Value {
    serde_json::json!({
        "method": "elicitation/create",
        "params": {
            "message": format!("{agent_id} asks to run {action}: {preview}"),
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "decision": {
                        "type": "string",
                        "title": "Decision",
                        "enum": Decision::ALL,
                        "default": "decline"
                    }
                },
                "required": ["decision"]
            }
        }
    })
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
    /// The tool (or label) asked about: an "accept for the session" answer
    /// covers the next asks of the same turn for it.
    tool: String,
    permission: Option<(crate::core::assist::runs::Registry, String)>,
    /// The agent that asked, for the audit line of an "always" (2.8).
    agent: String,
}

/// The id an ask has in the audit log: its durable id when it has a row,
/// else the tool call it belongs to.
fn ask_entity(
    permission: &Option<(crate::core::assist::runs::Registry, String)>,
    call: &str,
) -> super::audit::Entity {
    super::audit::Entity::ask(
        permission
            .as_ref()
            .map(|(_, id)| id.as_str())
            .unwrap_or(call),
    )
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
    /// The process that asked is gone (a restart): nothing can release that
    /// call; the turn has to be started again.
    Stale,
}

pub struct ToolBroker {
    sources: RwLock<Vec<Source>>,
    bus: Arc<Bus>,
    pending: Mutex<HashMap<String, PendingAsk>>,
    ask_timeout: RwLock<Duration>,
    ask_timeout_resolver: RwLock<Option<AskTimeoutResolver>>,
    /// The run registry of the last durable question, so a late answer can
    /// still be told apart (expired vs resolved) after its waiter left.
    last_registry: Mutex<Option<crate::core::assist::runs::Registry>>,
    /// "Accept for the session" answers: request id → tools that stop asking
    /// for the rest of that turn. Bounded: cleared when it grows past
    /// [`SESSION_ALLOW_MAX`] turns (a turn then simply asks again).
    session_allow: Mutex<HashMap<String, std::collections::HashSet<String>>>,
}

/// Turns remembered by `session_allow` before it starts over.
const SESSION_ALLOW_MAX: usize = 512;

impl std::fmt::Debug for ToolBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("ToolBroker")
            .field("sources", &sources.len())
            .field(
                "specs",
                &sources.iter().map(|s| s.specs.len()).sum::<usize>(),
            )
            .field("ask_timeout", &self.default_ask_timeout())
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
            ask_timeout: RwLock::new(DEFAULT_ASK_TIMEOUT),
            ask_timeout_resolver: RwLock::new(None),
            last_registry: Mutex::new(None),
            session_allow: Mutex::new(HashMap::new()),
        };
        broker.register_internal(specs, executor);
        broker
    }

    pub fn with_ask_timeout(self, timeout: Duration) -> Self {
        self.set_ask_timeout(timeout);
        self
    }

    /// The default ask timeout (the chat's), settable on a broker that is
    /// already shared: the settings change while the app runs.
    pub fn set_ask_timeout(&self, timeout: Duration) {
        *self.ask_timeout.write().unwrap_or_else(|e| e.into_inner()) = timeout;
    }

    fn default_ask_timeout(&self) -> Duration {
        *self.ask_timeout.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Per-request timeouts: the jobs register one so a background job's
    /// question waits as long as the job may still run.
    pub fn set_ask_timeout_resolver(&self, resolver: Option<AskTimeoutResolver>) {
        *self
            .ask_timeout_resolver
            .write()
            .unwrap_or_else(|e| e.into_inner()) = resolver;
    }

    /// How long a question of `request_id` would wait. What a resolver
    /// picks is never under [`MIN_ASK_TIMEOUT`]; the default is taken as set
    /// (the settings clamp it, the tests shorten it).
    pub fn ask_timeout_for(&self, request_id: &str) -> Duration {
        let resolver = self
            .ask_timeout_resolver
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match resolver.and_then(|r| r(request_id)) {
            Some(t) => t.max(MIN_ASK_TIMEOUT),
            None => self.default_ask_timeout(),
        }
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
        self.respond(
            permission_id,
            Decision::of_answer(answer),
            &crate::core::assist::runs::Actor::window(),
        )
    }

    /// The one entry of every channel (item 2.2): answers the durable ask
    /// `permission_id` with `decision`, recording who answered and through
    /// what. Exactly one answer applies; any later one is refused
    /// (`Resolved`/`Expired`/`Unknown`). A `stale` ask (its waiter died with
    /// another boot) takes no answer; `Cancel` closes it.
    pub fn respond(
        &self,
        permission_id: &str,
        decision: Decision,
        actor: &crate::core::assist::runs::Actor,
    ) -> Result<(), AnswerRefused> {
        self.respond_in(None, permission_id, decision, actor)
    }

    /// [`Self::respond`] against a given run record when nobody waits here
    /// (a handler outside the turn's task, a test); `None` = the active one.
    pub fn respond_in(
        &self,
        registry: Option<&crate::core::assist::runs::Registry>,
        permission_id: &str,
        decision: Decision,
        actor: &crate::core::assist::runs::Actor,
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
        if let Some(k) = key {
            return self.deliver_by(&k, decision, actor);
        }
        // Nobody waits here: tell expired, stale and resolved apart if the
        // record knows.
        let reg = registry
            .cloned()
            .or_else(crate::core::assist::runs::active)
            .or_else(|| {
                self.last_registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            });
        let Some(reg) = reg else {
            return Err(AnswerRefused::Unknown);
        };
        use crate::core::assist::runs::PermissionState as S;
        match reg.permission(permission_id).map(|p| p.state) {
            Some(S::Stale) if decision == Decision::Cancel => {
                reg.dismiss_stale(permission_id, actor)
                    .map_err(|_| AnswerRefused::Resolved)?;
                super::audit::log(
                    actor.into(),
                    "ask.dismissed",
                    super::audit::Entity::ask(permission_id),
                    serde_json::json!({ "state": "stale" }),
                );
                Ok(())
            }
            Some(S::Stale) => Err(AnswerRefused::Stale),
            Some(S::Expired) => Err(AnswerRefused::Expired),
            Some(S::Pending) | None => Err(AnswerRefused::Unknown),
            Some(_) => Err(AnswerRefused::Resolved),
        }
    }

    fn deliver(&self, tool_call_id: &str, answer: Answer) -> Result<(), AnswerRefused> {
        self.deliver_by(
            tool_call_id,
            Decision::of_answer(answer),
            &crate::core::assist::runs::Actor::window(),
        )
    }

    /// [`Self::answer_with`] with the full decision and who gave it (the
    /// window's cards answer by tool call id).
    pub fn answer_call_by(
        &self,
        tool_call_id: &str,
        decision: Decision,
        actor: &crate::core::assist::runs::Actor,
    ) -> Result<(), AnswerRefused> {
        self.deliver_by(tool_call_id, decision, actor)
    }

    fn deliver_by(
        &self,
        tool_call_id: &str,
        decision: Decision,
        actor: &crate::core::assist::runs::Actor,
    ) -> Result<(), AnswerRefused> {
        let entry = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.remove(tool_call_id)
        };
        let Some(entry) = entry else {
            return Err(AnswerRefused::Unknown);
        };
        let entity = ask_entity(&entry.permission, tool_call_id);
        if let Some((reg, id)) = &entry.permission {
            if let Err(e) = reg.answer_permission_by(id, decision.word(), actor) {
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
        // Who answered, through what (2.8): the one line every channel leaves.
        super::audit::log(
            actor.into(),
            "ask.answered",
            entity,
            serde_json::json!({ "decision": decision.as_str(), "tool": entry.tool, "run": entry.request_id }),
        );
        if decision == Decision::AcceptAlways {
            super::audit::log(
                actor.into(),
                "grant.always",
                super::audit::Entity::rule(&format!("{}:{}", entry.agent, entry.tool)),
                serde_json::json!({ "agent": entry.agent, "tool": entry.tool }),
            );
        }
        if decision == Decision::AcceptSession {
            let mut allow = self.session_allow.lock().unwrap_or_else(|e| e.into_inner());
            if allow.len() >= SESSION_ALLOW_MAX && !allow.contains_key(&entry.request_id) {
                allow.clear();
            }
            allow
                .entry(entry.request_id.clone())
                .or_default()
                .insert(entry.tool.clone());
        }
        entry
            .tx
            .send(decision.answer())
            .map_err(|_| AnswerRefused::Expired)
    }

    /// Whether an "accept for the session" answer already covers `tool` in
    /// this turn.
    fn session_allows(&self, request_id: &str, tool: &str) -> bool {
        self.session_allow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(request_id)
            .is_some_and(|t| t.contains(tool))
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
        self.session_allow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(request_id);
        let n = gone.len();
        let mut done = std::collections::HashSet::new();
        for p in gone {
            super::audit::log(
                super::audit::Actor::system("cancel"),
                "ask.cancelled",
                ask_entity(&p.permission, ""),
                serde_json::json!({ "tool": p.tool, "run": p.request_id }),
            );
            if let Some((reg, _)) = &p.permission {
                if done.insert(p.request_id.clone()) {
                    let _ = reg.cancel_permissions(&p.request_id);
                }
            }
            drop(p.tx);
        }
        n
    }

    /// The questions this broker waits on, as `(request_id, tool_call_id,
    /// tool)`: the window's list when no run record is available.
    pub fn pending_calls(&self) -> Vec<(String, String, String)> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, p)| (p.request_id.clone(), k.clone(), p.tool.clone()))
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether a question of this turn waits for a person right now. The
    /// turn watchdog freezes its idle clock on it.
    pub fn has_pending(&self, request_id: &str) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|p| p.request_id == request_id)
    }

    /// Whether this exact question is still unanswered (a notification is
    /// re-checked against it at delivery).
    pub fn has_pending_call(&self, tool_call_id: &str) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(tool_call_id)
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
        // The grant, the turn's run mode inside it (2.7), the stored rules and
        // the always-ask tools: the one gate the Claude Code hook uses too
        // (3.4), so native and CLI give the same verdict.
        let run =
            super::code_tools::current_turn().and_then(|c| super::run_mode::of(&c.conversation));
        let mut mode = super::perm::gate(
            &super::perm::rules_of(agent_id),
            run,
            self.mode_for(grants, name),
            name,
            &super::perm::native_input(name, &input, super::code_tools::workspace().as_deref()),
        )
        .mode;
        if !matches!(mode, GrantMode::Deny) {
            // A path outside the workspace is its own question, whatever the
            // grant says; a yes covers this one call.
            let path = input.get("path").and_then(Value::as_str).unwrap_or("");
            if external_context.is_none()
                && name.starts_with("fs_")
                && super::code_tools::is_external(path)
            {
                let end = self
                    .ask_ended(
                        agent_id,
                        request_id,
                        tool_call_id,
                        "external_directory",
                        &Value::String(format!("{name} {path}")),
                    )
                    .await;
                if let AskEnd::Expired { waited } = end {
                    return Err(self.fail(agent_id, name, start, timeout_error(name, waited)));
                }
                if !matches!(end.answer(), Answer::Deny) {
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
                let end = self
                    .ask_ended(agent_id, request_id, tool_call_id, name, &input)
                    .await;
                if let AskEnd::Expired { waited } = end {
                    return Err(self.fail(agent_id, name, start, timeout_error(name, waited)));
                }
                let answer = end.answer();
                // An external run's "Always" covers this call only: it never
                // becomes a permanent local rule of the executor (L2).
                if matches!(answer, Answer::Always)
                    && external_context.is_none()
                    && !super::tool_table::always_asks(name)
                {
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
        self.ask_user_ended(agent_id, request_id, tool_call_id, label, preview)
            .await
            .answer()
    }

    /// [`Self::ask_user`], saying how the question ended, so the caller can
    /// tell its agent "nobody answered" instead of "the user refused".
    pub async fn ask_user_ended(
        &self,
        agent_id: &str,
        request_id: &str,
        tool_call_id: &str,
        label: &str,
        preview: &str,
    ) -> AskEnd {
        self.ask_ended(
            agent_id,
            request_id,
            tool_call_id,
            label,
            &Value::String(preview.to_string()),
        )
        .await
    }

    /// Ask the user and wait. Only an explicit yes allows; a refusal, a
    /// timeout, a cancelled turn and nobody listening on the bus (no UI, no
    /// consent) all come back as something else, each told apart.
    async fn ask_ended(
        &self,
        agent_id: &str,
        request_id: &str,
        tool_call_id: &str,
        name: &str,
        input: &Value,
    ) -> AskEnd {
        // "Accept for the session" already said yes to this tool in this
        // turn: no new question.
        if self.session_allows(request_id, name) {
            return AskEnd::Answered(Answer::Once);
        }
        let asked_at = Instant::now();
        let ask_timeout = self.ask_timeout_for(request_id);
        let (tx, rx) = oneshot::channel();
        let preview_text = match input {
            Value::String(s) => s.clone(),
            _ => super::code_tools::preview(name, input),
        };
        // A thread of the Central persists the ask as `request.opened` and
        // waits for the answer however long it takes (plan §3.3); jobs and
        // the old chat keep the configurable timeout.
        let durable = super::code_tools::current_turn()
            .map(|ctx| super::drivers::is_durable_conversation(&ctx.conversation))
            .unwrap_or(false);
        // The durable request (item 2.2: every ask is a row of
        // `runs_permissions`, with or without a run record): the answer must
        // resolve exactly this one, in time. A durable thread has no deadline.
        let deadline = if durable {
            i64::MAX
        } else {
            crate::core::assist::now_ms() + ask_timeout.as_millis().min(i64::MAX as u128) as i64
        };
        let permission = crate::core::assist::runs::active().and_then(|reg| {
            let req = reg
                .open_ask(crate::core::assist::runs::NewAsk {
                    run_id: request_id.to_string(),
                    tool_call_id: tool_call_id.to_string(),
                    action: name.to_string(),
                    preview: preview_text.clone(),
                    options: Decision::ALL.iter().map(|d| d.to_string()).collect(),
                    deadline_ms: deadline,
                    agent_id: agent_id.to_string(),
                    payload: elicitation(agent_id, name, &preview_text),
                })
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
                    tool: name.to_string(),
                    permission: permission.clone(),
                    agent: agent_id.to_string(),
                },
            );
        }
        // Never the preview (it can hold a file's content): the tool and the
        // turn are the fact (2.8).
        super::audit::log(
            super::audit::Actor::agent(agent_id),
            "ask.opened",
            ask_entity(&permission, tool_call_id),
            serde_json::json!({ "tool": name, "run": request_id }),
        );
        self.bus.emit(BusEvent::ToolAsk {
            agent: agent_id.to_string(),
            request_id: request_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            tool: name.to_string(),
            preview: preview_text,
        });
        let end = if durable {
            match rx.await {
                Ok(answer) => AskEnd::Answered(answer),
                Err(_) => AskEnd::Cancelled,
            }
        } else {
            match tokio::time::timeout(ask_timeout, rx).await {
                Ok(Ok(answer)) => AskEnd::Answered(answer),
                // The sender was dropped: the turn was cancelled, or the answer
                // could not resolve the durable record.
                Ok(Err(_)) => AskEnd::Cancelled,
                Err(_) => AskEnd::Expired {
                    waited: asked_at.elapsed(),
                },
            }
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
        if let AskEnd::Expired { waited } = end {
            super::audit::log(
                super::audit::Actor::system("timeout"),
                "ask.expired",
                ask_entity(&permission, tool_call_id),
                serde_json::json!({ "tool": name, "run": request_id, "waited_ms": waited.as_millis() as u64 }),
            );
            tracing::warn!(
                "[broker] {ERR_TOOL_TIMEOUT}: `{name}` of {request_id} unanswered after {} s",
                waited.as_secs()
            );
        }
        self.bus.emit(BusEvent::ToolAskResolved {
            agent: agent_id.to_string(),
            request_id: request_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            tool: name.to_string(),
            outcome: end.outcome().to_string(),
            waited_ms: asked_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
        });
        end
    }
}
