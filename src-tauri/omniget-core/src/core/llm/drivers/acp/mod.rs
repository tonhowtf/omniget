//! `acp` driver: any agent that speaks the Agent Client Protocol (v1 schema,
//! `agentclientprotocol/agent-client-protocol` `schema/v1`, checked
//! 2026-09-22) behind the Central's [`Driver`] trait. One child process and
//! one root ACP session per thread.
//!
//! Flow: `initialize` (fs + terminal + auth.terminal + elicitation client
//! capabilities) → `session/load` (resume cursor, when `loadSession`) or
//! `session/resume` (when `sessionCapabilities.resume`) or `session/new
//! {cwd, mcpServers}` → modes/model applied (`session/set_mode`,
//! `session/set_config_option`, `session/set_model`) → `session/prompt` per
//! turn, `session/cancel` (a notification, never a request) to interrupt.
//! The agent's `session/update`s become [`RuntimeEvent`]s (see
//! [`translate`]); its requests are served here: `session/request_permission`
//! → durable `request.opened` with the canonical type and the agent's own
//! options, `fs/*` inside the thread's workspace, `terminal/*` with our own
//! processes, `elicitation/create` → `user-input.requested`.
//!
//! Auth: when `session/new` says "authentication required" the driver only
//! tries silent methods ([`agents::silent_methods`]); otherwise it emits
//! `auth.status` with the login command and fails the session with
//! `ERR_ACP_AUTH_REQUIRED`. No credential is ever read.

pub mod agents;
pub mod client;
pub mod diff;
pub mod proc;
pub mod rpc;
pub mod translate;

#[cfg(test)]
mod live_tests;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use super::{
    AccessMode, ApprovalDecision, AuthStatusPayload, Driver, DriverCapabilities, DriverError,
    DriverInstance, DriverRegistration, ErrorClass, InteractionMode, NoticePayload, PlanStep,
    PlanUpdatedPayload, ProposedCompletedPayload, ProviderRefs, RawPayload, RequestOpenedPayload,
    RequestResolvedPayload, RequestType, RuntimeErrorPayload, RuntimeEvent, RuntimeEventKind,
    RuntimeSink, RuntimeWarningPayload, SessionExitedPayload, SessionStart, SessionStartedPayload,
    SessionState, SessionStatePayload, ThreadStartedPayload, TurnAbortedPayload,
    TurnCompletedPayload, TurnEndState, TurnStart, TurnStartResult, TurnStartedPayload,
    UserInputRequestedPayload, UserInputResolvedPayload, ValuePayload, ERR_DRIVER_FAILED,
    ERR_DRIVER_NO_SESSION, ERR_DRIVER_UNAVAILABLE,
};
use rpc::{Connection, Inbound, RpcError};
use translate::{Emit, Translator};

pub const KIND: &str = "acp";
pub const PROTOCOL_VERSION: u64 = 1;
pub const ERR_ACP_AUTH: &str = "ERR_ACP_AUTH_REQUIRED";
pub const ERR_ACP: &str = "ERR_ACP";

/// First `initialize` of an npx/uvx agent downloads the package.
const INIT_TIMEOUT: Duration = Duration::from_secs(180);
const SESSION_TIMEOUT: Duration = Duration::from_secs(90);
const SMALL_TIMEOUT: Duration = Duration::from_secs(30);
/// After `session/cancel`, how long the prompt may take to return.
const CANCEL_GRACE: Duration = Duration::from_secs(15);

pub fn capabilities() -> DriverCapabilities {
    DriverCapabilities {
        interrupt: true,
        approvals: true,
        user_input: true,
        model_switch: true,
        plan_mode: true,
        ..Default::default()
    }
}

/// Register the `acp` driver kind. Call once (the threads host setup).
pub fn register() {
    super::register_driver(DriverRegistration {
        kind: KIND.into(),
        label: "ACP agents".into(),
        capabilities: capabilities(),
        factory: Arc::new(|instance, sink| {
            Ok(Arc::new(AcpDriver::new(instance, sink)?) as Arc<dyn Driver>)
        }),
    });
}

/// Every ACP agent that can start here, as instances of this driver.
pub fn instances() -> Vec<DriverInstance> {
    agents::instances()
}

// ── MCP injection (plan §3.2: the OmniGet MCP goes into `session/new`) ──

/// One MCP server to hand to an agent in `session/new|load|resume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpServerSpec {
    Http {
        name: String,
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    Sse {
        name: String,
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    Stdio {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
}

/// `(thread_id, instance_id) → servers`. Set by the host once the embedded
/// MCP has per-session credentials; empty until then.
pub type McpProvider = Arc<dyn Fn(&str, &str) -> Vec<McpServerSpec> + Send + Sync>;

static MCP: RwLock<Option<McpProvider>> = RwLock::new(None);

pub fn set_mcp_provider(provider: Option<McpProvider>) {
    *MCP.write().unwrap_or_else(|e| e.into_inner()) = provider;
}

pub fn mcp_servers_for(thread_id: &str, instance_id: &str) -> Vec<McpServerSpec> {
    MCP.read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|f| f(thread_id, instance_id))
        .unwrap_or_default()
}

/// ACP wire form, filtered by what the agent said it supports.
fn mcp_json(servers: &[McpServerSpec], http: bool, sse: bool) -> Value {
    let headers = |h: &[(String, String)]| -> Value {
        h.iter()
            .map(|(k, v)| json!({ "name": k, "value": v }))
            .collect()
    };
    Value::Array(
        servers
            .iter()
            .filter_map(|s| match s {
                McpServerSpec::Http { name, url, headers: h } if http => {
                    Some(json!({ "type": "http", "name": name, "url": url, "headers": headers(h) }))
                }
                McpServerSpec::Sse { name, url, headers: h } if sse => {
                    Some(json!({ "type": "sse", "name": name, "url": url, "headers": headers(h) }))
                }
                McpServerSpec::Stdio { name, command, args, env } => Some(json!({
                    "name": name, "command": command, "args": args,
                    "env": env.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>(),
                })),
                _ => None,
            })
            .collect(),
    )
}

// ── Modes ───────────────────────────────────────────────────────────────

const PLAN_MODES: &[&str] = &["plan", "architect"];
const DEFAULT_MODES: &[&str] = &["default", "build", "code", "agent", "normal"];
const EDIT_MODES: &[&str] = &[
    "acceptEdits",
    "auto_edit",
    "autoEdit",
    "auto-edit",
    "accept-edits",
];
const FULL_MODES: &[&str] = &[
    "yolo",
    "bypassPermissions",
    "bypass",
    "full-access",
    "full_access",
    "dangerously-skip-permissions",
    "auto-approve",
];
const AUTO_MODES: &[&str] = &["auto"];

/// The mode id to switch to for an access/interaction pair, among the ids the
/// agent offers. `None` = leave the agent's mode alone.
pub fn desired_mode(
    available: &[String],
    current: Option<&str>,
    access: AccessMode,
    interaction: InteractionMode,
) -> Option<String> {
    let find = |cands: &[&str]| {
        cands.iter().find_map(|c| {
            available
                .iter()
                .find(|a| a.eq_ignore_ascii_case(c))
                .cloned()
        })
    };
    let target = if interaction == InteractionMode::Plan {
        find(PLAN_MODES)
    } else {
        let by_access = match access {
            AccessMode::FullAccess => find(FULL_MODES),
            AccessMode::AutoAcceptEdits => find(EDIT_MODES),
            AccessMode::Auto => find(AUTO_MODES),
            AccessMode::ApprovalRequired => None,
        };
        by_access.or_else(|| {
            // Leaving plan (or a permissive mode) goes back to the default.
            let cur = current.unwrap_or("");
            let special = PLAN_MODES
                .iter()
                .chain(FULL_MODES)
                .chain(EDIT_MODES)
                .chain(AUTO_MODES)
                .any(|m| m.eq_ignore_ascii_case(cur));
            if special {
                find(DEFAULT_MODES)
            } else {
                None
            }
        })
    };
    target.filter(|t| Some(t.as_str()) != current)
}

/// Values of a `select` config option (flat or grouped).
fn select_values(opt: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for o in opt
        .get("options")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
    {
        if let Some(v) = o.get("value").and_then(Value::as_str) {
            out.push(v.to_string());
        } else if let Some(group) = o.get("options").and_then(Value::as_array) {
            out.extend(
                group
                    .iter()
                    .filter_map(|g| g.get("value").and_then(Value::as_str).map(str::to_string)),
            );
        }
    }
    out
}

fn config_option<'a>(opts: Option<&'a Value>, category: &str) -> Option<&'a Value> {
    opts?.as_array()?.iter().find(|o| {
        o.get("category").and_then(Value::as_str) == Some(category)
            || o.get("id").and_then(Value::as_str) == Some(category)
    })
}

/// Would an access mode let this permission through without asking?
pub fn auto_allows(access: AccessMode, kind: &str) -> bool {
    let read_only = matches!(kind, "read" | "search" | "think");
    match access {
        AccessMode::FullAccess => true,
        AccessMode::AutoAcceptEdits => read_only || matches!(kind, "edit" | "delete" | "move"),
        AccessMode::Auto => read_only,
        AccessMode::ApprovalRequired => false,
    }
}

// ── Session ─────────────────────────────────────────────────────────────

struct Turn {
    id: String,
    interrupted: bool,
}

struct PendingPerm {
    rpc_id: Value,
    options: Vec<Value>,
    request_type: RequestType,
    memory_key: String,
    turn: Option<String>,
}

struct PendingInput {
    rpc_id: Value,
    params: Value,
    turn: Option<String>,
}

struct State {
    translator: Translator,
    turn: Option<Turn>,
    last_turn: Option<String>,
    loading: bool,
    access: AccessMode,
    interaction: InteractionMode,
    modes: Option<Value>,
    models: Option<Value>,
    model: Option<String>,
    perms: HashMap<String, PendingPerm>,
    inputs: HashMap<String, PendingInput>,
    session_allow: HashSet<String>,
}

struct Session {
    thread_id: String,
    instance_id: String,
    agent: String,
    agent_name: String,
    login_cmd: Option<String>,
    conn: Arc<Connection>,
    session_id: StdMutex<String>,
    cwd: PathBuf,
    init: StdMutex<Value>,
    state: StdMutex<State>,
    terminals: Arc<client::Terminals>,
    sink: RuntimeSink,
    stopping: AtomicBool,
}

impl Session {
    fn sid(&self) -> String {
        self.session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn event(&self, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(KIND, &self.instance_id, &self.thread_id, turn, kind)
    }

    fn send(&self, ev: RuntimeEvent) {
        let _ = self.sink.send(ev);
    }

    fn emit(&self, turn: Option<&str>, kind: RuntimeEventKind) {
        self.send(self.event(turn, kind));
    }

    fn emit_translated(&self, turn: Option<&str>, e: Emit) {
        let mut ev = self.event(turn, e.kind);
        ev.item_id = e.item_id;
        if let Some(p) = e.provider_item_id {
            ev.provider_refs = Some(ProviderRefs {
                provider_item_id: Some(p),
                ..Default::default()
            });
        }
        self.send(ev);
    }

    fn state_changed(&self, state: SessionState, reason: Option<&str>) {
        self.emit(
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state,
                reason: reason.map(str::to_string),
                detail: None,
            }),
        );
    }

    fn resume_cursor(&self) -> Value {
        json!({ "sessionId": self.sid(), "agent": self.agent, "cwd": self.cwd })
    }

    fn current_turn(&self) -> Option<String> {
        let st = self.st();
        st.turn
            .as_ref()
            .map(|t| t.id.clone())
            .or_else(|| st.last_turn.clone())
    }

    /// Main loop: every agent request/notification, in order.
    async fn run(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<Inbound>) {
        while let Some(msg) = rx.recv().await {
            if msg.method == rpc::EXIT {
                self.on_exit(&msg.params);
                break;
            }
            self.clone().handle(msg).await;
        }
    }

    fn on_exit(&self, params: &Value) {
        self.terminals.release_all();
        if self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let tail = params
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let code = params.get("code").and_then(Value::as_i64);
        let message = match code {
            Some(c) => format!("{} exited with code {c}", self.agent_name),
            None => format!("{} stopped", self.agent_name),
        };
        let turn = self.st().turn.take();
        self.stale_pending("agent-exited");
        if let Some(t) = &turn {
            self.emit(
                Some(&t.id),
                RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                    message: message.clone(),
                    class: ErrorClass::TransportError,
                    code: Some(ERR_ACP.into()),
                    detail: (!tail.is_empty()).then(|| tail.clone()),
                }),
            );
            self.emit(
                Some(&t.id),
                RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                    state: if t.interrupted {
                        TurnEndState::Interrupted
                    } else {
                        TurnEndState::Failed
                    },
                    stop_reason: None,
                    usage: None,
                    total_cost_usd: None,
                    error_message: Some(message.clone()),
                }),
            );
        }
        self.emit(
            None,
            RuntimeEventKind::SessionExited(SessionExitedPayload {
                reason: Some(if tail.is_empty() {
                    message
                } else {
                    format!("{message}: {tail}")
                }),
                recoverable: Some(true),
                exit_kind: Some("error".into()),
            }),
        );
    }

    /// Answer every open permission/question as cancelled and tell the host.
    fn stale_pending(&self, resolution: &str) {
        let (perms, inputs) = {
            let mut st = self.st();
            (
                st.perms.drain().collect::<Vec<_>>(),
                st.inputs.drain().collect::<Vec<_>>(),
            )
        };
        for (rid, p) in perms {
            let conn = self.conn.clone();
            let id = p.rpc_id.clone();
            tokio::spawn(async move {
                conn.respond(id, Ok(json!({ "outcome": { "outcome": "cancelled" } })))
                    .await;
            });
            let mut ev = self.event(
                p.turn.as_deref(),
                RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                    request_type: p.request_type,
                    decision: Some(ApprovalDecision::Cancel),
                    resolution: Some(resolution.to_string()),
                }),
            );
            ev.request_id = Some(rid);
            self.send(ev);
        }
        for (rid, p) in inputs {
            let conn = self.conn.clone();
            let id = p.rpc_id.clone();
            tokio::spawn(async move {
                conn.respond(id, Ok(json!({ "action": "cancel" }))).await;
            });
            let mut ev = self.event(
                p.turn.as_deref(),
                RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                    answers: Value::Null,
                }),
            );
            ev.request_id = Some(rid);
            self.send(ev);
        }
    }

    async fn reply(&self, id: Option<Value>, result: Result<Value, RpcError>) {
        if let Some(id) = id {
            self.conn.respond(id, result).await;
        }
    }

    fn foreign(&self, params: &Value) -> bool {
        match params.get("sessionId").and_then(Value::as_str) {
            Some(s) => {
                let mine = self.sid();
                !mine.is_empty() && s != mine
            }
            None => false,
        }
    }

    async fn handle(self: Arc<Self>, msg: Inbound) {
        let p = &msg.params;
        match msg.method.as_str() {
            rpc::PROMPT_DONE => self.on_prompt_done(p),
            "session/update" => {
                if self.foreign(p) {
                    return;
                }
                let update = &p["update"];
                let kind = update
                    .get("sessionUpdate")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let config_like = matches!(
                    kind,
                    "available_commands_update"
                        | "current_mode_update"
                        | "config_option_update"
                        | "session_info_update"
                        | "usage_update"
                );
                let (emits, turn) = {
                    let mut st = self.st();
                    if st.loading && !config_like {
                        return;
                    }
                    let emits = st.translator.on_update(update);
                    let turn = st
                        .turn
                        .as_ref()
                        .map(|t| t.id.clone())
                        .or_else(|| st.last_turn.clone());
                    (emits, turn)
                };
                for e in emits {
                    let t = match &e.kind {
                        RuntimeEventKind::SessionConfigured(_)
                        | RuntimeEventKind::ThreadMetadataUpdated(_) => None,
                        _ => turn.clone(),
                    };
                    self.emit_translated(t.as_deref(), e);
                }
            }
            "session/request_permission" => self.on_permission(msg.id, p).await,
            "fs/read_text_file" => {
                let r = client::read_text_file(&self.cwd, p);
                self.reply(msg.id, r).await;
            }
            "fs/write_text_file" => {
                let r = client::write_text_file(&self.cwd, p);
                self.reply(msg.id, r).await;
            }
            "terminal/create" => {
                let r = self.terminals.create(p);
                self.reply(msg.id, r).await;
            }
            "terminal/output" => {
                let r = self.terminals.output(p);
                self.reply(msg.id, r).await;
            }
            "terminal/kill" => {
                let r = self.terminals.kill(p);
                self.reply(msg.id, r).await;
            }
            "terminal/release" => {
                if let Some(tid) = p.get("terminalId").and_then(Value::as_str) {
                    self.st().translator.forget_terminal(tid);
                }
                let r = self.terminals.release(p);
                self.reply(msg.id, r).await;
            }
            "terminal/wait_for_exit" => {
                // Long wait: off the loop.
                let me = self.clone();
                let params = p.clone();
                tokio::spawn(async move {
                    let r = me.terminals.wait_for_exit(&params).await;
                    me.reply(msg.id, r).await;
                });
            }
            "elicitation/create" => {
                let Some(id) = msg.id else { return };
                let questions = translate::elicitation_questions(p);
                let rid = format!("elicit-{}", uuid::Uuid::new_v4().simple());
                let turn = self.current_turn();
                self.st().inputs.insert(
                    rid.clone(),
                    PendingInput {
                        rpc_id: id,
                        params: p.clone(),
                        turn: turn.clone(),
                    },
                );
                let mut ev = self.event(
                    turn.as_deref(),
                    RuntimeEventKind::UserInputRequested(UserInputRequestedPayload {
                        questions,
                        response_mode: None,
                    }),
                );
                ev.request_id = Some(rid);
                self.send(ev);
                self.state_changed(SessionState::Waiting, Some("user-input"));
            }
            // Cursor extensions (T3 Code's CursorAcpExtension).
            "cursor/update_todos" => {
                let todos = p
                    .get("todos")
                    .and_then(Value::as_array)
                    .map(|a| a.as_slice())
                    .unwrap_or(&[]);
                let plan = todos
                    .iter()
                    .map(|t| PlanStep {
                        step: t
                            .get("content")
                            .or_else(|| t.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        status: match t.get("status").and_then(Value::as_str) {
                            Some("completed") | Some("done") => "completed",
                            Some("in_progress") | Some("inProgress") => "inProgress",
                            _ => "pending",
                        }
                        .to_string(),
                    })
                    .collect();
                let turn = self.current_turn();
                self.emit(
                    turn.as_deref(),
                    RuntimeEventKind::PlanUpdated(PlanUpdatedPayload {
                        explanation: None,
                        plan,
                    }),
                );
                self.reply(msg.id, Ok(Value::Null)).await;
            }
            "cursor/create_plan" => {
                let md = ["plan", "markdown", "content", "text"]
                    .iter()
                    .find_map(|k| p.get(*k).and_then(Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let turn = self.current_turn();
                self.emit(
                    turn.as_deref(),
                    RuntimeEventKind::ProposedCompleted(ProposedCompletedPayload {
                        plan_markdown: md,
                    }),
                );
                self.reply(msg.id, Ok(json!({ "accepted": true }))).await;
            }
            // Grok sometimes completes a prompt without answering the RPC.
            "_x.ai/session/prompt_complete" | "x.ai/session/prompt_complete" => {
                let active = self.st().turn.as_ref().map(|t| t.id.clone());
                if let Some(turn) = active {
                    let stop = p
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn");
                    self.on_prompt_done(
                        &json!({ "turnId": turn, "result": { "stopReason": stop } }),
                    );
                }
            }
            other => {
                if msg.id.is_some() {
                    self.reply(
                        msg.id,
                        Err(RpcError::new(
                            rpc::CODE_METHOD_NOT_FOUND,
                            format!("`{other}` is not supported by OmniGet"),
                        )),
                    )
                    .await;
                } else {
                    let turn = self.current_turn();
                    self.emit(
                        turn.as_deref(),
                        RuntimeEventKind::Raw(RawPayload {
                            source: "acp.jsonrpc".into(),
                            method: Some(other.to_string()),
                            payload: p.clone(),
                        }),
                    );
                }
            }
        }
    }

    async fn on_permission(self: &Arc<Self>, id: Option<Value>, p: &Value) {
        let Some(id) = id else { return };
        let ask = translate::permission_ask(p);
        let options: Vec<Value> = p
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (auto, turn) = {
            let st = self.st();
            let auto =
                auto_allows(st.access, &ask.kind) || st.session_allow.contains(&ask.memory_key);
            (
                auto,
                st.turn
                    .as_ref()
                    .map(|t| t.id.clone())
                    .or_else(|| st.last_turn.clone()),
            )
        };
        if auto {
            let outcome = translate::permission_outcome(&options, ApprovalDecision::Accept);
            self.conn.respond(id, Ok(outcome)).await;
            return;
        }
        let rid = format!("perm-{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
        self.st().perms.insert(
            rid.clone(),
            PendingPerm {
                rpc_id: id.clone(),
                options,
                request_type: ask.request_type,
                memory_key: ask.memory_key.clone(),
                turn: turn.clone(),
            },
        );
        let mut detail = ask.detail.clone();
        if detail.cwd.is_none() && ask.kind == "execute" {
            detail.cwd = Some(self.cwd.to_string_lossy().to_string());
        }
        let mut ev = self.event(
            turn.as_deref(),
            RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                request_type: ask.request_type,
                detail: Some(detail),
                app_name: Some(self.agent_name.clone()),
                options: ask.options.clone(),
                args: Some(json!({ "toolCallId": ask.tool_call_id, "kind": ask.kind })),
            }),
        );
        ev.request_id = Some(rid);
        if !ask.tool_call_id.is_empty() {
            ev.item_id = Some(ask.tool_call_id.clone());
        }
        ev.provider_refs = Some(ProviderRefs {
            provider_request_id: Some(id.to_string()),
            provider_item_id: (!ask.tool_call_id.is_empty()).then(|| ask.tool_call_id.clone()),
            ..Default::default()
        });
        self.send(ev);
        self.state_changed(SessionState::Waiting, Some("approval"));
    }

    fn on_prompt_done(&self, p: &Value) {
        let turn_id = p.get("turnId").and_then(Value::as_str).unwrap_or("");
        let (turn, emits, cost) = {
            let mut st = self.st();
            match &st.turn {
                Some(t) if t.id == turn_id => {}
                // Late answer of a turn already closed (watchdog, Grok's
                // prompt_complete): ignore.
                _ => return,
            }
            let turn = st.turn.take().expect("checked above");
            st.last_turn = Some(turn.id.clone());
            let emits = st.translator.end_turn();
            (turn, emits, st.translator.turn_cost())
        };
        for e in emits {
            self.emit_translated(Some(&turn.id), e);
        }
        // Questions the agent left open die with the turn.
        self.stale_pending("stale");
        let completed = match (p.get("result"), p.get("error")) {
            (Some(result), _) if !result.is_null() => {
                let (state, error) = translate::turn_end(
                    result.get("stopReason").and_then(Value::as_str),
                    turn.interrupted,
                );
                TurnCompletedPayload {
                    state,
                    stop_reason: result
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    usage: translate::prompt_usage(result).map(|mut u| {
                        u.cost_usd = cost;
                        u
                    }),
                    total_cost_usd: cost,
                    error_message: error,
                }
            }
            (_, Some(err)) => {
                let e = RpcError::from_value(err);
                if e.is_auth_required() {
                    self.emit_auth_status(Some(&turn.id), &e.message);
                }
                self.emit(
                    Some(&turn.id),
                    RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                        message: e.message.clone(),
                        class: if e.is_auth_required() {
                            ErrorClass::PermissionError
                        } else {
                            ErrorClass::ProviderError
                        },
                        code: Some(e.code.to_string()),
                        detail: e.data.as_ref().map(|d| d.to_string()),
                    }),
                );
                TurnCompletedPayload {
                    state: if turn.interrupted {
                        TurnEndState::Interrupted
                    } else {
                        TurnEndState::Failed
                    },
                    stop_reason: None,
                    usage: None,
                    total_cost_usd: cost,
                    error_message: Some(e.message),
                }
            }
            _ => TurnCompletedPayload {
                state: if turn.interrupted {
                    TurnEndState::Interrupted
                } else {
                    TurnEndState::Completed
                },
                stop_reason: None,
                usage: None,
                total_cost_usd: cost,
                error_message: None,
            },
        };
        self.emit(Some(&turn.id), RuntimeEventKind::TurnCompleted(completed));
        self.state_changed(SessionState::Ready, None);
    }

    fn emit_auth_status(&self, turn: Option<&str>, error: &str) {
        let init = self.init.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let output = auth_lines(
            &self.agent_name,
            &init,
            self.login_cmd.as_deref(),
            agents::login_hint(&self.agent).as_deref(),
        );
        self.emit(
            turn,
            RuntimeEventKind::AuthStatus(AuthStatusPayload {
                is_authenticating: Some(false),
                output,
                error: Some(error.to_string()),
            }),
        );
    }

    async fn apply_modes(&self) {
        let (available, current, access, interaction, via_config) = {
            let st = self.st();
            let modes = st.modes.clone();
            let cfg = config_option(st.translator.config_options(), "mode").cloned();
            let (available, current, via_config) = match (&modes, &cfg) {
                (Some(m), _) if m.get("availableModes").is_some() => (
                    m["availableModes"]
                        .as_array()
                        .map(|a| a.as_slice())
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|x| x.get("id").and_then(Value::as_str).map(str::to_string))
                        .collect::<Vec<_>>(),
                    st.translator
                        .current_mode()
                        .map(str::to_string)
                        .or_else(|| {
                            m.get("currentModeId")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        }),
                    None,
                ),
                (_, Some(c)) => (
                    select_values(c),
                    c.get("currentValue")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    c.get("id").and_then(Value::as_str).map(str::to_string),
                ),
                _ => return,
            };
            (available, current, st.access, st.interaction, via_config)
        };
        let Some(target) = desired_mode(&available, current.as_deref(), access, interaction) else {
            return;
        };
        let sid = self.sid();
        let result = match &via_config {
            None => self
                .conn
                .request(
                    "session/set_mode",
                    json!({ "sessionId": sid, "modeId": target }),
                    Some(SMALL_TIMEOUT),
                )
                .await
                .map(|_| None),
            Some(config_id) => self
                .conn
                .request(
                    "session/set_config_option",
                    json!({ "sessionId": sid, "configId": config_id, "value": target }),
                    Some(SMALL_TIMEOUT),
                )
                .await
                .map(|r| r.get("configOptions").cloned()),
        };
        match result {
            Ok(cfg) => {
                let mut st = self.st();
                st.translator.set_current_mode(Some(target.clone()));
                if let Some(c) = cfg {
                    st.translator.set_config_options(Some(c));
                }
                drop(st);
                self.emit(
                    None,
                    RuntimeEventKind::SessionConfigured(ValuePayload {
                        value: json!({ "currentModeId": target }),
                    }),
                );
            }
            Err(e) => self.emit(
                None,
                RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                    message: format!("could not switch {} to mode `{target}`", self.agent_name),
                    detail: Some(e.to_string()),
                }),
            ),
        }
    }

    async fn apply_model(&self, model: &str) {
        if model.trim().is_empty() {
            return;
        }
        let (cfg, models, current) = {
            let st = self.st();
            (
                config_option(st.translator.config_options(), "model").cloned(),
                st.models.clone(),
                st.model.clone(),
            )
        };
        if current.as_deref() == Some(model) {
            return;
        }
        let sid = self.sid();
        let result = if let Some(c) = cfg.filter(|c| select_values(c).iter().any(|v| v == model)) {
            let id = c
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("model")
                .to_string();
            self.conn
                .request(
                    "session/set_config_option",
                    json!({ "sessionId": sid, "configId": id, "value": model }),
                    Some(SMALL_TIMEOUT),
                )
                .await
                .map(|r| {
                    if let Some(c) = r.get("configOptions").cloned() {
                        self.st().translator.set_config_options(Some(c));
                    }
                })
        } else if models.is_some() {
            // Unstable `session/set_model` (Grok, Cursor, older Gemini).
            self.conn
                .request(
                    "session/set_model",
                    json!({ "sessionId": sid, "modelId": model }),
                    Some(SMALL_TIMEOUT),
                )
                .await
                .map(|_| ())
        } else {
            Err(RpcError::new(0, "the agent offers no model choice"))
        };
        match result {
            Ok(()) => {
                self.st().model = Some(model.to_string());
                self.emit(
                    None,
                    RuntimeEventKind::SessionConfigured(ValuePayload {
                        value: json!({ "model": model }),
                    }),
                );
            }
            Err(e) => self.emit(
                None,
                RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                    message: format!(
                        "{} kept its model; `{model}` was not accepted",
                        self.agent_name
                    ),
                    detail: Some(e.to_string()),
                }),
            ),
        }
    }
}

/// Lines of the `auth.status` shown when an agent needs a sign-in. The line
/// starting with `$ ` is the command to run (in the system or embedded
/// terminal); nothing here is a secret.
pub fn auth_lines(
    agent_name: &str,
    init: &Value,
    login_cmd: Option<&str>,
    hint: Option<&str>,
) -> Vec<String> {
    let mut out = vec![format!(
        "{agent_name} needs a sign-in before it can start a session."
    )];
    let methods = init
        .get("authMethods")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if !methods.is_empty() {
        let list: Vec<String> = methods
            .iter()
            .map(|m| {
                format!(
                    "{} ({})",
                    m.get("name").and_then(Value::as_str).unwrap_or(""),
                    m.get("id").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect();
        out.push(format!("Sign-in methods: {}", list.join(", ")));
    }
    if let Some(cmd) = login_cmd {
        out.push(format!("$ {cmd}"));
    }
    if let Some(h) = hint {
        out.push(h.to_string());
    }
    out
}

/// Prompt content blocks for a turn: the text, then attachments the agent
/// accepts (`image` with base64 when `promptCapabilities.image`, files as
/// `resource_link`).
pub fn prompt_blocks(text: &str, attachments: &[Value], prompt_caps: &Value) -> Vec<Value> {
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(json!({ "type": "text", "text": text }));
    }
    let image_ok = prompt_caps
        .get("image")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let audio_ok = prompt_caps
        .get("audio")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for a in attachments {
        let mime = a
            .get("mimeType")
            .or_else(|| a.get("mime"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let data = a.get("data").and_then(Value::as_str);
        let path = a
            .get("path")
            .or_else(|| a.get("uri"))
            .and_then(Value::as_str);
        match (data, mime) {
            (Some(d), m) if m.starts_with("image/") && image_ok => {
                blocks.push(json!({ "type": "image", "data": d, "mimeType": m }))
            }
            (Some(d), m) if m.starts_with("audio/") && audio_ok => {
                blocks.push(json!({ "type": "audio", "data": d, "mimeType": m }))
            }
            _ => {
                if let Some(p) = path {
                    let uri = if p.contains("://") {
                        p.to_string()
                    } else {
                        format!("file://{p}")
                    };
                    let name = a
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            std::path::Path::new(p)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| p.to_string())
                        });
                    let mut link = json!({ "type": "resource_link", "uri": uri, "name": name });
                    if !mime.is_empty() {
                        link["mimeType"] = Value::String(mime.to_string());
                    }
                    blocks.push(link);
                }
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "" }));
    }
    blocks
}

// ── Driver ──────────────────────────────────────────────────────────────

pub struct AcpDriver {
    instance: DriverInstance,
    launch: agents::Launch,
    sink: RuntimeSink,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

fn failed(msg: impl Into<String>) -> DriverError {
    DriverError::new(ERR_DRIVER_FAILED, msg)
}

impl AcpDriver {
    pub fn new(instance: DriverInstance, sink: RuntimeSink) -> Result<Self, DriverError> {
        let launch = agents::launch_for(&instance).ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_UNAVAILABLE,
                format!(
                    "no ACP agent for instance `{}`: install it in Tools or set a command",
                    instance.id
                ),
            )
        })?;
        Ok(Self {
            instance,
            launch,
            sink,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    async fn live(&self, thread_id: &str) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .await
            .get(thread_id)
            .filter(|s| !s.conn.is_dead())
            .cloned()
    }

    async fn open(&self, input: &SessionStart) -> Result<Arc<Session>, DriverError> {
        let cwd = input
            .cwd
            .clone()
            .or_else(dirs::home_dir)
            .ok_or_else(|| failed("no workspace folder for the ACP session"))?;
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        let (tx, rx) = mpsc::unbounded_channel();
        let spec = proc::Spawn {
            command: self.launch.command.clone(),
            args: self.launch.args.clone(),
            env: self.launch.env.clone(),
            env_remove: Vec::new(),
            cwd: Some(cwd.clone()),
        };
        let conn = Connection::spawn(&spec, tx, false)
            .map_err(|e| DriverError::new(ERR_DRIVER_UNAVAILABLE, e))?;
        let sink = self.sink.clone();
        let session_slot: Arc<StdMutex<Option<std::sync::Weak<Session>>>> =
            Arc::new(StdMutex::new(None));
        let slot = session_slot.clone();
        let terminals = client::Terminals::new(
            cwd.clone(),
            self.launch.env.clone(),
            Arc::new(move |tid: &str, chunk: &str| {
                let s = slot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .and_then(|w| w.upgrade());
                if let Some(s) = s {
                    let (emits, turn) = {
                        let mut st = s.st();
                        let t = st
                            .turn
                            .as_ref()
                            .map(|t| t.id.clone())
                            .or_else(|| st.last_turn.clone());
                        (st.translator.on_terminal_output(tid, chunk), t)
                    };
                    for e in emits {
                        s.emit_translated(turn.as_deref(), e);
                    }
                }
            }),
        );
        let login_cmd = agents::login_command(&self.launch.agent).map(|v| v.join(" "));
        let session = Arc::new(Session {
            thread_id: input.thread_id.clone(),
            instance_id: self.instance.id.clone(),
            agent: self.launch.agent.clone(),
            agent_name: self.launch.name.clone(),
            login_cmd,
            conn: conn.clone(),
            session_id: StdMutex::new(String::new()),
            cwd: cwd.clone(),
            init: StdMutex::new(Value::Null),
            state: StdMutex::new(State {
                translator: Translator::new(),
                turn: None,
                last_turn: None,
                loading: false,
                access: input.access_mode,
                interaction: input.interaction_mode,
                modes: None,
                models: None,
                model: None,
                perms: HashMap::new(),
                inputs: HashMap::new(),
                session_allow: HashSet::new(),
            }),
            terminals,
            sink,
            stopping: AtomicBool::new(false),
        });
        *session_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(&session));
        tokio::spawn(session.clone().run(rx));
        session.state_changed(SessionState::Starting, None);

        match self.handshake(&session, input).await {
            Ok(()) => Ok(session),
            Err(e) => {
                session.stopping.store(true, Ordering::SeqCst);
                conn.shutdown().await;
                Err(e)
            }
        }
    }

    async fn handshake(&self, s: &Arc<Session>, input: &SessionStart) -> Result<(), DriverError> {
        let init = s
            .conn
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        "terminal": true,
                        "auth": { "terminal": true },
                        "elicitation": { "form": {}, "url": {} },
                    },
                    "clientInfo": { "name": "omniget", "title": "OmniGet", "version": env!("CARGO_PKG_VERSION") },
                }),
                Some(INIT_TIMEOUT),
            )
            .await
            .map_err(|e| {
                let tail = s.conn.stderr_tail();
                failed(format!(
                    "{ERR_ACP}: initialize failed for {}: {e}{}",
                    s.agent_name,
                    if tail.is_empty() { String::new() } else { format!(" — {tail}") }
                ))
            })?;
        *s.init.lock().unwrap_or_else(|e| e.into_inner()) = init.clone();
        let caps = init
            .get("agentCapabilities")
            .cloned()
            .unwrap_or(Value::Null);
        let load_ok = caps
            .get("loadSession")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let resume_ok = caps
            .get("sessionCapabilities")
            .and_then(|c| c.get("resume"))
            .map(|v| !v.is_null())
            .unwrap_or(false);
        let mcp_http = caps
            .pointer("/mcpCapabilities/http")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mcp_sse = caps
            .pointer("/mcpCapabilities/sse")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mcp = mcp_json(
            &mcp_servers_for(&input.thread_id, &self.instance.id),
            mcp_http,
            mcp_sse,
        );
        let cwd = s.cwd.to_string_lossy().to_string();
        let advertised: Vec<String> = init
            .get("authMethods")
            .and_then(Value::as_array)
            .map(|a| a.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();

        let resume_id = input
            .resume_cursor
            .as_ref()
            .and_then(|c| {
                c.get("sessionId")
                    .and_then(Value::as_str)
                    .or_else(|| c.as_str())
            })
            .map(str::to_string);
        let mut tried_auth: Vec<String> = Vec::new();
        let mut resumed = false;
        let response = loop {
            let attempt = match (&resume_id, load_ok, resume_ok, resumed) {
                (Some(id), true, _, false) => {
                    s.st().loading = true;
                    let r = s
                        .conn
                        .request(
                            "session/load",
                            json!({ "sessionId": id, "cwd": cwd, "mcpServers": mcp }),
                            Some(SESSION_TIMEOUT),
                        )
                        .await
                        .map(|mut v| {
                            if v.is_null() {
                                v = json!({});
                            }
                            v["sessionId"] = json!(id);
                            v
                        });
                    s.st().loading = false;
                    r
                }
                (Some(id), false, true, false) => s
                    .conn
                    .request(
                        "session/resume",
                        json!({ "sessionId": id, "cwd": cwd, "mcpServers": mcp }),
                        Some(SESSION_TIMEOUT),
                    )
                    .await
                    .map(|mut v| {
                        if v.is_null() {
                            v = json!({});
                        }
                        v["sessionId"] = json!(id);
                        v
                    }),
                _ => {
                    s.conn
                        .request(
                            "session/new",
                            json!({ "cwd": cwd, "mcpServers": mcp }),
                            Some(SESSION_TIMEOUT),
                        )
                        .await
                }
            };
            match attempt {
                Ok(v) => break v,
                Err(e) if e.is_auth_required() => {
                    let next = agents::silent_methods(&s.agent, &advertised, &self.launch.env)
                        .into_iter()
                        .find(|m| !tried_auth.contains(m));
                    match next {
                        Some(method) => {
                            tried_auth.push(method.clone());
                            if let Err(ae) = s
                                .conn
                                .request(
                                    "authenticate",
                                    json!({ "methodId": method }),
                                    Some(SESSION_TIMEOUT),
                                )
                                .await
                            {
                                s.emit(
                                    None,
                                    RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                                        message: format!("`{method}` sign-in did not work"),
                                        detail: Some(ae.to_string()),
                                    }),
                                );
                            }
                        }
                        None => {
                            s.emit_auth_status(None, &e.message);
                            return Err(DriverError::new(
                                ERR_DRIVER_FAILED,
                                format!("{ERR_ACP_AUTH}: {} — {}", s.agent_name, e.message),
                            ));
                        }
                    }
                }
                Err(e) if resume_id.is_some() && !resumed => {
                    resumed = true;
                    s.emit(
                        None,
                        RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                            message: format!(
                                "{} could not reopen its session; starting a new one",
                                s.agent_name
                            ),
                            detail: Some(e.to_string()),
                        }),
                    );
                }
                Err(e) => {
                    let tail = s.conn.stderr_tail();
                    return Err(failed(format!(
                        "{ERR_ACP}: session/new failed for {}: {e}{}",
                        s.agent_name,
                        if tail.is_empty() {
                            String::new()
                        } else {
                            format!(" — {tail}")
                        }
                    )));
                }
            }
        };
        let sid = response
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| failed(format!("{ERR_ACP}: the agent returned no sessionId")))?
            .to_string();
        *s.session_id.lock().unwrap_or_else(|e| e.into_inner()) = sid.clone();
        {
            let mut st = s.st();
            st.modes = response.get("modes").cloned().filter(|v| !v.is_null());
            st.models = response.get("models").cloned().filter(|v| !v.is_null());
            if let Some(cur) = st
                .models
                .as_ref()
                .and_then(|m| m.get("currentModelId"))
                .and_then(Value::as_str)
            {
                st.model = Some(cur.to_string());
            }
            let cur_mode = st
                .modes
                .as_ref()
                .and_then(|m| m.get("currentModeId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            st.translator.set_current_mode(cur_mode);
            st.translator
                .set_config_options(response.get("configOptions").cloned());
            if let Some(model) = config_option(st.translator.config_options(), "model")
                .and_then(|c| c.get("currentValue"))
                .and_then(Value::as_str)
            {
                st.model = Some(model.to_string());
            }
        }
        s.apply_modes().await;
        if let Some(m) = &input.model {
            s.apply_model(m).await;
        }
        s.emit(
            None,
            RuntimeEventKind::SessionStarted(SessionStartedPayload {
                message: Some(format!("{} ready", s.agent_name)),
                resume: Some(s.resume_cursor()),
            }),
        );
        s.emit(
            None,
            RuntimeEventKind::ThreadStarted(ThreadStartedPayload {
                provider_thread_id: Some(sid),
            }),
        );
        let st = s.st();
        let configured = json!({
            "agent": s.agent,
            "agentInfo": init.get("agentInfo"),
            "protocolVersion": init.get("protocolVersion"),
            "agentCapabilities": caps,
            "authMethods": init.get("authMethods"),
            "modes": st.modes,
            "models": st.models,
            "configOptions": st.translator.config_options(),
            "model": st.model,
            "launch": self.launch.source,
        });
        drop(st);
        s.emit(
            None,
            RuntimeEventKind::SessionConfigured(ValuePayload { value: configured }),
        );
        for n in launch_notices(&self.launch) {
            s.emit(None, RuntimeEventKind::DeprecationNotice(n));
        }
        s.state_changed(SessionState::Ready, None);
        Ok(())
    }

    fn session_or_err(
        s: Option<Arc<Session>>,
        thread_id: &str,
    ) -> Result<Arc<Session>, DriverError> {
        s.ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_NO_SESSION,
                format!("no live ACP session for thread {thread_id}"),
            )
        })
    }

    /// An initialize-only probe (no session, no auth): agent info, auth
    /// methods and capabilities. For the Tools screen / diagnostics.
    pub async fn probe(instance: &DriverInstance) -> Result<Value, String> {
        let launch = agents::launch_for(instance)
            .ok_or_else(|| format!("{ERR_DRIVER_UNAVAILABLE}: no command for {}", instance.id))?;
        let (tx, _rx) = mpsc::unbounded_channel();
        let spec = proc::Spawn {
            command: launch.command.clone(),
            args: launch.args.clone(),
            env: launch.env.clone(),
            env_remove: Vec::new(),
            cwd: dirs::home_dir(),
        };
        let conn = Connection::spawn(&spec, tx, false)?;
        let r = conn
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                    "clientInfo": { "name": "omniget", "version": env!("CARGO_PKG_VERSION") },
                }),
                Some(INIT_TIMEOUT),
            )
            .await;
        conn.shutdown().await;
        r.map(|mut v| {
            v["launch"] =
                json!({ "command": launch.command, "args": launch.args, "source": launch.source });
            v
        })
        .map_err(|e| format!("{ERR_ACP}: {e}"))
    }
}

#[async_trait]
impl Driver for AcpDriver {
    fn kind(&self) -> &str {
        KIND
    }

    fn capabilities(&self) -> DriverCapabilities {
        capabilities()
    }

    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError> {
        if let Some(s) = self.live(&input.thread_id).await {
            let same_cwd = match &input.cwd {
                Some(c) => std::fs::canonicalize(c).unwrap_or_else(|_| c.clone()) == s.cwd,
                None => true,
            };
            if same_cwd {
                let changed = {
                    let mut st = s.st();
                    let changed =
                        st.access != input.access_mode || st.interaction != input.interaction_mode;
                    st.access = input.access_mode;
                    st.interaction = input.interaction_mode;
                    changed
                };
                if changed {
                    s.apply_modes().await;
                }
                if let Some(m) = &input.model {
                    s.apply_model(m).await;
                }
                return Ok(());
            }
            // Workspace moved (worktree): a new process in the new folder.
            self.stop(&input.thread_id).await?;
        }
        let s = self.open(&input).await?;
        self.sessions
            .lock()
            .await
            .insert(input.thread_id.clone(), s);
        Ok(())
    }

    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError> {
        let s = Self::session_or_err(self.live(&input.thread_id).await, &input.thread_id)?;
        if s.st().turn.is_some() {
            return Err(failed(format!(
                "{} is still working on the previous turn",
                s.agent_name
            )));
        }
        let changed = {
            let mut st = s.st();
            let changed =
                st.access != input.access_mode || st.interaction != input.interaction_mode;
            st.access = input.access_mode;
            st.interaction = input.interaction_mode;
            changed
        };
        if changed {
            s.apply_modes().await;
        }
        if let Some(m) = &input.model {
            s.apply_model(m).await;
        }
        let caps = s
            .init
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pointer("/agentCapabilities/promptCapabilities")
            .cloned()
            .unwrap_or(Value::Null);
        let blocks = prompt_blocks(&input.text, &input.attachments, &caps);
        let model = {
            let mut st = s.st();
            st.translator.begin_turn(&input.text);
            st.turn = Some(Turn {
                id: input.turn_id.clone(),
                interrupted: false,
            });
            st.model.clone()
        };
        s.emit(
            Some(&input.turn_id),
            RuntimeEventKind::TurnStarted(TurnStartedPayload {
                model,
                effort: None,
            }),
        );
        s.state_changed(SessionState::Running, None);
        let rx = match s
            .conn
            .send_request(
                "session/prompt",
                json!({ "sessionId": s.sid(), "prompt": blocks }),
            )
            .await
        {
            Ok(rx) => rx,
            Err(e) => {
                let mut st = s.st();
                st.turn = None;
                st.translator.end_turn();
                return Err(failed(format!("{ERR_ACP}: session/prompt: {e}")));
            }
        };
        let conn = s.conn.clone();
        let turn_id = input.turn_id.clone();
        tokio::spawn(async move {
            let payload = match rx.await {
                Ok(Ok(result)) => json!({ "turnId": turn_id, "result": result }),
                Ok(Err(e)) => {
                    json!({ "turnId": turn_id, "error": { "code": e.code, "message": e.message, "data": e.data } })
                }
                Err(_) => {
                    json!({ "turnId": turn_id, "error": { "code": rpc::CODE_INTERNAL, "message": "the ACP agent went away" } })
                }
            };
            conn.inject(rpc::PROMPT_DONE, payload);
        });
        Ok(TurnStartResult {
            resume_cursor: Some(s.resume_cursor()),
        })
    }

    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError> {
        let live = self.live(thread_id).await;
        let active = live
            .as_ref()
            .and_then(|s| s.st().turn.as_ref().map(|t| t.id.clone()));
        let (Some(s), Some(active)) = (live, active) else {
            if let Some(t) = turn_id {
                let _ = self.sink.send(RuntimeEvent::new(
                    KIND,
                    &self.instance.id,
                    thread_id,
                    Some(t),
                    RuntimeEventKind::TurnAborted(TurnAbortedPayload {
                        reason: "no turn was running".into(),
                        usage: None,
                    }),
                ));
            }
            return Ok(());
        };
        if let Some(t) = s.st().turn.as_mut() {
            t.interrupted = true;
        }
        let _ = s
            .conn
            .notify("session/cancel", json!({ "sessionId": s.sid() }))
            .await;
        // Spec: the client answers every pending permission as cancelled.
        s.stale_pending("cancelled");
        // Agents that ignore the cancel get stopped after a grace period.
        let watch = s.clone();
        tokio::spawn(async move {
            tokio::time::sleep(CANCEL_GRACE).await;
            let still = watch.st().turn.as_ref().map(|t| t.id.clone());
            if still.as_deref() == Some(active.as_str()) {
                watch.emit(
                    Some(&active),
                    RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                        message: format!(
                            "{} did not finish cancelling; its process was stopped",
                            watch.agent_name
                        ),
                        detail: None,
                    }),
                );
                watch.on_prompt_done(
                    &json!({ "turnId": active, "result": { "stopReason": "cancelled" } }),
                );
                watch.stopping.store(true, Ordering::SeqCst);
                watch.conn.shutdown().await;
                watch.emit(
                    None,
                    RuntimeEventKind::SessionExited(SessionExitedPayload {
                        reason: Some("stopped after an unanswered cancel".into()),
                        recoverable: Some(true),
                        exit_kind: Some("error".into()),
                    }),
                );
            }
        });
        Ok(())
    }

    async fn respond_request(
        &self,
        thread_id: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), DriverError> {
        let s = Self::session_or_err(self.live(thread_id).await, thread_id)?;
        let pending = {
            let mut st = s.st();
            let p = st.perms.remove(request_id);
            if let (Some(p), ApprovalDecision::AcceptForSession) = (&p, decision) {
                st.session_allow.insert(p.memory_key.clone());
            }
            p
        };
        let p = pending.ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_NO_SESSION,
                format!("no open ACP permission {request_id}"),
            )
        })?;
        let outcome = translate::permission_outcome(&p.options, decision);
        s.conn.respond(p.rpc_id.clone(), Ok(outcome)).await;
        let mut ev = s.event(
            p.turn.as_deref(),
            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                request_type: p.request_type,
                decision: Some(decision),
                resolution: None,
            }),
        );
        ev.request_id = Some(request_id.to_string());
        s.send(ev);
        if s.st().turn.is_some() {
            s.state_changed(SessionState::Running, None);
        }
        Ok(())
    }

    async fn respond_user_input(
        &self,
        thread_id: &str,
        request_id: &str,
        answers: Value,
    ) -> Result<(), DriverError> {
        let s = Self::session_or_err(self.live(thread_id).await, thread_id)?;
        let p = s.st().inputs.remove(request_id).ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_NO_SESSION,
                format!("no open ACP question {request_id}"),
            )
        })?;
        let response = translate::elicitation_response(&p.params, &answers);
        s.conn.respond(p.rpc_id.clone(), Ok(response)).await;
        let mut ev = s.event(
            p.turn.as_deref(),
            RuntimeEventKind::UserInputResolved(UserInputResolvedPayload { answers }),
        );
        ev.request_id = Some(request_id.to_string());
        s.send(ev);
        if s.st().turn.is_some() {
            s.state_changed(SessionState::Running, None);
        }
        Ok(())
    }

    async fn rollback(&self, _thread_id: &str, _turn_count: u32) -> Result<(), DriverError> {
        Err(DriverError::unsupported(
            "rollback (ACP sessions have no provider-side history cut)",
        ))
    }

    async fn set_access_mode(&self, thread_id: &str, mode: AccessMode) -> Result<(), DriverError> {
        if let Some(s) = self.live(thread_id).await {
            let changed = {
                let mut st = s.st();
                let c = st.access != mode;
                st.access = mode;
                c
            };
            if changed {
                s.apply_modes().await;
            }
        }
        Ok(())
    }

    async fn stop(&self, thread_id: &str) -> Result<(), DriverError> {
        let s = self.sessions.lock().await.remove(thread_id);
        let Some(s) = s else { return Ok(()) };
        s.stopping.store(true, Ordering::SeqCst);
        let turn = s.st().turn.take();
        if turn.is_some() {
            let _ = s
                .conn
                .notify("session/cancel", json!({ "sessionId": s.sid() }))
                .await;
        }
        s.stale_pending("stopped");
        if let Some(t) = turn {
            s.emit(
                Some(&t.id),
                RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                    state: TurnEndState::Interrupted,
                    stop_reason: Some("stopped".into()),
                    usage: None,
                    total_cost_usd: None,
                    error_message: None,
                }),
            );
        }
        // Agents that support it drop their session state cleanly first.
        let close_ok = s
            .init
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pointer("/agentCapabilities/sessionCapabilities/close")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        if close_ok && !s.conn.is_dead() {
            let _ = s
                .conn
                .request(
                    "session/close",
                    json!({ "sessionId": s.sid() }),
                    Some(Duration::from_secs(3)),
                )
                .await;
        }
        s.terminals.release_all();
        s.conn.shutdown().await;
        s.emit(
            None,
            RuntimeEventKind::SessionExited(SessionExitedPayload {
                reason: Some("stopped".into()),
                recoverable: Some(true),
                exit_kind: Some("graceful".into()),
            }),
        );
        Ok(())
    }
}

/// Deprecation notices of a launch (the old `--experimental-acp` flag).
pub fn launch_notices(launch: &agents::Launch) -> Vec<NoticePayload> {
    launch
        .args
        .iter()
        .filter(|a| a.as_str() == "--experimental-acp")
        .map(|_| NoticePayload {
            summary: format!(
                "{} uses the deprecated --experimental-acp flag",
                launch.name
            ),
            details: Some("Recent versions take --acp.".into()),
            path: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_follow_access_and_plan() {
        let avail: Vec<String> = ["default", "acceptEdits", "plan", "bypassPermissions"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            desired_mode(
                &avail,
                Some("default"),
                AccessMode::FullAccess,
                InteractionMode::Default
            )
            .as_deref(),
            Some("bypassPermissions")
        );
        assert_eq!(
            desired_mode(
                &avail,
                Some("default"),
                AccessMode::AutoAcceptEdits,
                InteractionMode::Default
            )
            .as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(
            desired_mode(
                &avail,
                Some("default"),
                AccessMode::ApprovalRequired,
                InteractionMode::Plan
            )
            .as_deref(),
            Some("plan")
        );
        // Leaving plan goes back to the default; staying put is a no-op.
        assert_eq!(
            desired_mode(
                &avail,
                Some("plan"),
                AccessMode::ApprovalRequired,
                InteractionMode::Default
            )
            .as_deref(),
            Some("default")
        );
        assert_eq!(
            desired_mode(
                &avail,
                Some("default"),
                AccessMode::ApprovalRequired,
                InteractionMode::Default
            ),
            None
        );
        // OpenCode exposes build/plan as a config option.
        let oc: Vec<String> = vec!["build".into(), "plan".into()];
        assert_eq!(
            desired_mode(
                &oc,
                Some("plan"),
                AccessMode::FullAccess,
                InteractionMode::Default
            )
            .as_deref(),
            Some("build")
        );
    }

    #[test]
    fn access_modes_gate_permissions() {
        assert!(auto_allows(AccessMode::FullAccess, "execute"));
        assert!(auto_allows(AccessMode::AutoAcceptEdits, "edit"));
        assert!(!auto_allows(AccessMode::AutoAcceptEdits, "execute"));
        assert!(auto_allows(AccessMode::Auto, "read"));
        assert!(!auto_allows(AccessMode::Auto, "edit"));
        assert!(!auto_allows(AccessMode::ApprovalRequired, "read"));
    }

    #[test]
    fn mcp_servers_follow_agent_capabilities() {
        let servers = vec![
            McpServerSpec::Http {
                name: "omniget".into(),
                url: "http://127.0.0.1:1/mcp".into(),
                headers: vec![("Authorization".into(), "Bearer x".into())],
            },
            McpServerSpec::Stdio {
                name: "local".into(),
                command: "node".into(),
                args: vec!["s.js".into()],
                env: vec![],
            },
        ];
        let v = mcp_json(&servers, false, false);
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["name"], "local");
        let v = mcp_json(&servers, true, false);
        assert_eq!(v[0]["type"], "http");
        assert_eq!(v[0]["headers"][0]["name"], "Authorization");
    }

    #[test]
    fn prompt_blocks_respect_capabilities() {
        let atts = vec![
            json!({ "mimeType": "image/png", "data": "AAAA", "path": "/w/a.png" }),
            json!({ "path": "/w/notes.md", "mimeType": "text/markdown" }),
        ];
        let with = prompt_blocks("hi", &atts, &json!({ "image": true }));
        assert_eq!(with[1]["type"], "image");
        assert_eq!(with[2]["type"], "resource_link");
        assert_eq!(with[2]["uri"], "file:///w/notes.md");
        let without = prompt_blocks("hi", &atts, &json!({}));
        assert_eq!(without[1]["type"], "resource_link");
    }

    #[test]
    fn auth_lines_name_the_login_command() {
        let init =
            json!({ "authMethods": [{ "id": "oauth-personal", "name": "Log in with Google" }] });
        let lines = auth_lines("Gemini CLI", &init, Some("gemini"), None);
        assert!(lines.iter().any(|l| l == "$ gemini"));
        assert!(lines.iter().any(|l| l.contains("oauth-personal")));
    }
}
