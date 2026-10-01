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

pub const ERR_TOOL_DENIED: &str = "ERR_TOOL_DENIED";
pub const ERR_TOOL_UNKNOWN: &str = "ERR_TOOL_UNKNOWN";
pub const ERR_TOOL_TIMEOUT: &str = "ERR_TOOL_TIMEOUT";

pub const DEFAULT_ASK_TIMEOUT: Duration = Duration::from_secs(120);
pub const MIN_ASK_TIMEOUT: Duration = Duration::from_secs(1);

pub type AskTimeoutResolver = Arc<dyn Fn(&str) -> Option<Duration> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskEnd {
    Answered(Answer),
    Expired { waited: Duration },
    Cancelled,
}

impl AskEnd {
    pub fn answer(self) -> Answer {
        match self {
            AskEnd::Answered(a) => a,
            AskEnd::Expired { .. } | AskEnd::Cancelled => Answer::Deny,
        }
    }

    pub fn outcome(self) -> &'static str {
        match self {
            AskEnd::Answered(Answer::Deny) => "deny",
            AskEnd::Answered(_) => "allow",
            AskEnd::Expired { .. } => "expired",
            AskEnd::Cancelled => "cancelled",
        }
    }
}

pub fn timeout_error(name: &str, waited: Duration) -> LlmError {
    LlmError::new(
        ERR_TOOL_TIMEOUT,
        format!(
            "nobody answered the permission question for `{name}` in {} s; it did not run",
            waited.as_secs()
        ),
    )
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError>;
}

pub fn grant_key(source: &ToolSource) -> String {
    match source {
        ToolSource::Internal { name } => name.clone(),
        ToolSource::Mcp { server, tool } => format!("mcp:{server}:{tool}"),
        ToolSource::Skill { name } => format!("skill:{name}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Deny,
    Once,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accept,
    AcceptSession,
    AcceptAlways,
    Decline,
    Cancel,
}

impl Decision {
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

    pub fn word(self) -> &'static str {
        match self {
            Decision::Accept => "once",
            Decision::AcceptSession => "session",
            Decision::AcceptAlways => "always",
            Decision::Decline => "deny",
            Decision::Cancel => "cancel",
        }
    }

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

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub name: String,
    pub ok: bool,
    pub ms: u32,
    pub content: String,
}

const INTERNAL_PREFIX: &str = "";
pub const SKILL_PREFIX: &str = "skill:";

struct Source {
    prefix: String,
    executor: Arc<dyn ToolExecutor>,
    specs: Vec<ToolSpec>,
}

struct PendingAsk {
    tx: oneshot::Sender<Answer>,
    request_id: String,
    tool: String,
    permission: Option<(crate::core::assist::runs::Registry, String)>,
    agent: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerRefused {
    Unknown,
    Expired,
    Resolved,
    Stale,
}

pub struct ToolBroker {
    sources: RwLock<Vec<Source>>,
    bus: Arc<Bus>,
    pending: Mutex<HashMap<String, PendingAsk>>,
    ask_timeout: RwLock<Duration>,
    ask_timeout_resolver: RwLock<Option<AskTimeoutResolver>>,
    last_registry: Mutex<Option<crate::core::assist::runs::Registry>>,
    session_allow: Mutex<HashMap<String, std::collections::HashSet<String>>>,
}

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

    pub fn set_ask_timeout(&self, timeout: Duration) {
        *self.ask_timeout.write().unwrap_or_else(|e| e.into_inner()) = timeout;
    }

    fn default_ask_timeout(&self) -> Duration {
        *self.ask_timeout.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_ask_timeout_resolver(&self, resolver: Option<AskTimeoutResolver>) {
        *self
            .ask_timeout_resolver
            .write()
            .unwrap_or_else(|e| e.into_inner()) = resolver;
    }

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

    pub fn register_internal(
        &self,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        self.register(INTERNAL_PREFIX.to_string(), specs, executor)
    }

    pub fn register_source(
        &self,
        prefix: &str,
        specs: Vec<ToolSpec>,
        executor: Arc<dyn ToolExecutor>,
    ) -> usize {
        self.register(prefix.to_string(), specs, executor)
    }

    pub fn unregister_skills(&self) -> usize {
        self.unregister(SKILL_PREFIX)
    }

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

    fn executor_for(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        let sources = self.sources.read().unwrap_or_else(|e| e.into_inner());
        sources
            .iter()
            .find(|s| s.specs.iter().any(|spec| spec.name == name))
            .map(|s| s.executor.clone())
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.sources
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .flat_map(|s| s.specs.iter().cloned())
            .collect()
    }

    pub fn specs_for(&self, grants: &[ToolGrant]) -> Vec<ToolSpec> {
        self.specs()
            .into_iter()
            .filter(|spec| !matches!(self.mode_for(grants, &spec.name), GrantMode::Deny))
            .collect()
    }

    pub fn mode_for(&self, grants: &[ToolGrant], name: &str) -> GrantMode {
        grants
            .iter()
            .find(|g| grant_key(&g.source) == name)
            .map(|g| g.mode)
            .unwrap_or(GrantMode::Deny)
    }

    pub fn answer(&self, tool_call_id: &str, allow: bool) -> bool {
        self.answer_with(
            tool_call_id,
            if allow { Answer::Once } else { Answer::Deny },
        )
    }

    pub fn answer_with(&self, tool_call_id: &str, answer: Answer) -> bool {
        self.deliver(tool_call_id, answer).is_ok()
    }

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

    pub fn respond(
        &self,
        permission_id: &str,
        decision: Decision,
        actor: &crate::core::assist::runs::Actor,
    ) -> Result<(), AnswerRefused> {
        self.respond_in(None, permission_id, decision, actor)
    }

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
                return Err(
                    if e.starts_with(crate::core::assist::runs::ERR_PERMISSION_EXPIRED) {
                        AnswerRefused::Expired
                    } else {
                        AnswerRefused::Resolved
                    },
                );
            }
        }
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

    fn session_allows(&self, request_id: &str, tool: &str) -> bool {
        self.session_allow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(request_id)
            .is_some_and(|t| t.contains(tool))
    }

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

    pub fn pending_calls(&self) -> Vec<(String, String, String)> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, p)| (p.request_id.clone(), k.clone(), p.tool.clone()))
            .collect()
    }

    pub fn has_pending(&self, request_id: &str) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|p| p.request_id == request_id)
    }

    pub fn has_pending_call(&self, tool_call_id: &str) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(tool_call_id)
    }

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

    async fn ask_ended(
        &self,
        agent_id: &str,
        request_id: &str,
        tool_call_id: &str,
        name: &str,
        input: &Value,
    ) -> AskEnd {
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
        let durable = super::code_tools::current_turn()
            .map(|ctx| super::drivers::is_durable_conversation(&ctx.conversation))
            .unwrap_or(false);
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
                Ok(Err(_)) => AskEnd::Cancelled,
                Err(_) => AskEnd::Expired {
                    waited: asked_at.elapsed(),
                },
            }
        };
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let ours = pending
                .get(tool_call_id)
                .map(|p| p.permission.as_ref().map(|(_, id)| id.clone()) == permission_id)
                .unwrap_or(false);
            if ours {
                pending.remove(tool_call_id);
            }
        }
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
