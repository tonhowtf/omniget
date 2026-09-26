//! `AcpRuntime`: any CLI that speaks the Agent Client Protocol (Gemini CLI,
//! `claude-code-acp`, `codex-acp`, goose…) as one more agent of the roster.
//! Our own client, no SDK: newline-delimited JSON-RPC 2.0 over stdio, read from
//! `agentclientprotocol/agent-client-protocol` (schema) and `compozy/acp-go-sdk`
//! (flow): `initialize` → `session/new { cwd }` → `session/prompt`, with the
//! agent streaming `session/update` notifications and calling back into us for
//! `session/request_permission`, `fs/read_text_file` and `fs/write_text_file`.
//!
//! The MCP stdio transport only routes responses, so ACP has its own reader:
//! it must also answer requests the agent makes. Line framing is shared.
//!
//! One child per conversation, kept alive between turns so the agent keeps its
//! own context; permissions go through our broker (`ToolAsk` on the bus, the
//! same prompt the native tools use, "Always" included).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::agent::{AgentDef, RuntimeKind};
use super::broker::{Answer, ToolBroker};
use super::error::LlmError;
use super::runtime::AgentRuntime;
use super::types::{ContentPart, FinishReason, Message, Role, TurnEvent, TurnRequest};
use crate::core::mcp::stdio::LineFramer;

pub const ERR_ACP: &str = "ERR_LLM_ACP";
const MAX_LINE: usize = 16 * 1024 * 1024;

type Pending = Arc<StdMutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// A request or notification the agent sent us.
struct Inbound {
    id: Option<Value>,
    method: String,
    params: Value,
}

struct Session {
    stdin: Mutex<tokio::process::ChildStdin>,
    _child: StdMutex<tokio::process::Child>,
    pending: Pending,
    next_id: AtomicI64,
    dead: Arc<AtomicBool>,
    inbound: Mutex<mpsc::UnboundedReceiver<Inbound>>,
    session_id: String,
    cwd: PathBuf,
    command: String,
    /// The agent can reopen a session by id (`loadSession` capability).
    load_session: bool,
    /// Projection token of this session (refreshed every turn).
    grant_id: Option<String>,
}

/// What `start` should do about the session: create one, or reopen the one
/// the agent gave us before (only when it announces `loadSession`).
struct StartOptions {
    mcp_server: Option<Value>,
    grant_id: Option<String>,
    load: Option<String>,
}

impl Session {
    async fn write(&self, msg: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())
    }

    fn register(&self) -> (i64, oneshot::Receiver<Result<Value, String>>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        (id, rx)
    }
}

fn spawn_reader(
    stdout: tokio::process::ChildStdout,
    pending: Pending,
    dead: Arc<AtomicBool>,
    tx: mpsc::UnboundedSender<Inbound>,
) {
    tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut framer = LineFramer::new(MAX_LINE);
        let mut buf = vec![0u8; 16 * 1024];
        let handle = |line: &[u8]| {
            let Ok(msg) = serde_json::from_slice::<Value>(line) else {
                return;
            };
            if let Some(method) = msg.get("method").and_then(Value::as_str) {
                let _ = tx.send(Inbound {
                    id: msg.get("id").cloned().filter(|v| !v.is_null()),
                    method: method.to_string(),
                    params: msg.get("params").cloned().unwrap_or(Value::Null),
                });
                return;
            }
            let Some(id) = msg.get("id").and_then(Value::as_i64) else {
                return;
            };
            let waiter = pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            if let Some(waiter) = waiter {
                let _ = waiter.send(match msg.get("error") {
                    Some(err) if !err.is_null() => Err(err
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("agent error")
                        .to_string()),
                    _ => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                });
            }
        };
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => framer.push(&buf[..n], handle),
            }
        }
        dead.store(true, Ordering::SeqCst);
        let waiting: Vec<_> = pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        for (_, w) in waiting {
            let _ = w.send(Err("the ACP agent closed its output".into()));
        }
    });
}

async fn start(
    command: &str,
    args: &[String],
    cwd: PathBuf,
    opts: StartOptions,
) -> Result<(Arc<Session>, bool), String> {
    let mut cmd = crate::core::process::command(command);
    cmd.args(args)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // claude-code-acp 0.16 hangs in `session/new` on the Claude Code bundled in
    // its SDK; the user's own `claude` (the one that is logged in) answers.
    if command.contains("claude-code-acp") && std::env::var_os("CLAUDE_CODE_EXECUTABLE").is_none() {
        let found = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|d| d.join("claude"))
                .find(|p| p.is_file())
        });
        if let Some(claude) = found {
            cmd.env("CLAUDE_CODE_EXECUTABLE", claude);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start `{command}`: {e}"))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
    let dead = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::unbounded_channel();
    spawn_reader(stdout, pending.clone(), dead.clone(), tx);

    let mut session = Session {
        stdin: Mutex::new(stdin),
        _child: StdMutex::new(child),
        pending,
        next_id: AtomicI64::new(1),
        dead,
        inbound: Mutex::new(rx),
        session_id: String::new(),
        cwd: cwd.clone(),
        command: command.to_string(),
        load_session: false,
        grant_id: None,
    };
    let call = |s: &Session, method: &str, params: Value| {
        let (id, rx) = s.register();
        (
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
            rx,
        )
    };
    let wait = |rx: oneshot::Receiver<Result<Value, String>>| async move {
        match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("the ACP agent went away".to_string()),
            Err(_) => Err("the ACP agent did not answer in 60 s".to_string()),
        }
    };
    let (msg, rx) = call(
        &session,
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": false },
            "clientInfo": { "name": "omniget", "version": env!("CARGO_PKG_VERSION") },
        }),
    );
    session.write(&msg).await?;
    let init = wait(rx).await.map_err(|e| format!("initialize: {e}"))?;
    let caps = &init["agentCapabilities"];
    session.load_session = caps["loadSession"].as_bool().unwrap_or(false);
    // Our assistant tools go in only over a transport the agent announced.
    let http = caps["mcpCapabilities"]["http"].as_bool().unwrap_or(false);
    let servers: Vec<Value> = match (&opts.mcp_server, http) {
        (Some(server), true) => {
            session.grant_id = opts.grant_id.clone();
            vec![server.clone()]
        }
        _ => Vec::new(),
    };
    // Reopen the agent's own session when it can; otherwise a new one.
    if let (Some(id), true) = (&opts.load, session.load_session) {
        let (msg, rx) = call(
            &session,
            "session/load",
            json!({ "sessionId": id, "cwd": cwd.to_string_lossy(), "mcpServers": servers }),
        );
        session.write(&msg).await?;
        if wait(rx).await.is_ok() {
            session.session_id = id.clone();
            // The agent replays the history as `session/update`s; drop them
            // so the next turn does not show the past again.
            if let Ok(mut inbound) = session.inbound.try_lock() {
                while inbound.try_recv().is_ok() {}
            }
            return Ok((Arc::new(session), true));
        }
    }
    let (msg, rx) = call(
        &session,
        "session/new",
        json!({ "cwd": cwd.to_string_lossy(), "mcpServers": servers }),
    );
    session.write(&msg).await?;
    let created = wait(rx)
        .await
        .map_err(|e| format!("session/new: {e} (is the CLI logged in?)"))?;
    session.session_id = created["sessionId"]
        .as_str()
        .ok_or("session/new returned no sessionId")?
        .to_string();
    Ok((Arc::new(session), false))
}

pub struct AcpRuntime {
    broker: Arc<ToolBroker>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl AcpRuntime {
    pub fn new(broker: Arc<ToolBroker>) -> Self {
        Self {
            broker,
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

fn text_of(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A fresh child has no memory: it gets the transcript. A live one only needs
/// the last user message.
fn prompt_for(req: &TurnRequest, fresh: bool) -> String {
    let last_user = req.messages.iter().rposition(|m| m.role == Role::User);
    let last = last_user
        .map(|i| text_of(&req.messages[i]))
        .unwrap_or_default();
    if !fresh {
        return last;
    }
    let mut out = String::new();
    for (i, m) in req.messages.iter().enumerate() {
        if Some(i) == last_user {
            continue;
        }
        let text = text_of(m);
        if text.trim().is_empty() {
            continue;
        }
        let who = match m.role {
            Role::System => "Instructions",
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => continue,
        };
        out.push_str(&format!("[{who}]\n{text}\n\n"));
    }
    if out.is_empty() {
        last
    } else {
        format!("{out}[User]\n{last}")
    }
}

fn inside(cwd: &std::path::Path, raw: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(raw);
    let p = if p.is_absolute() { p } else { cwd.join(p) };
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
        || !p.starts_with(cwd)
    {
        return Err(format!("{}: {raw}", super::code_tools::ERR_OUTSIDE));
    }
    Ok(p)
}

#[async_trait]
impl AgentRuntime for AcpRuntime {
    async fn turn(
        &self,
        agent: &AgentDef,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
        let RuntimeKind::Acp { command, args } = &agent.runtime else {
            return Err(LlmError::new(
                ERR_ACP,
                format!("agent `{}` is not an ACP agent", agent.id),
            ));
        };
        // The coordinator opens the turn scope around this call.
        let ctx = super::code_tools::current_turn();
        let conversation = ctx
            .as_ref()
            .map(|c| c.conversation.clone())
            .unwrap_or_else(|| format!("acp-{}", agent.id));
        let request_id = ctx.as_ref().map(|c| c.request.clone()).unwrap_or_default();
        // The conversation's own folder; a personal conversation runs in an
        // empty private folder of the bot, never in the last folder used.
        let workspace = ctx
            .as_ref()
            .and_then(|c| super::code_tools::workspace_of(&c.conversation));
        let projectless = ctx.is_some() && workspace.is_none();
        let cwd = match (&ctx, workspace) {
            (_, Some(ws)) => ws,
            (Some(_), None) => super::roster_store::llm_dir()
                .map(|d| d.join("sandboxes").join(&agent.id))
                .filter(|d| std::fs::create_dir_all(d).is_ok())
                .unwrap_or_else(std::env::temp_dir),
            (None, None) => super::code_tools::workspace()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(std::env::temp_dir),
        };
        let registry = crate::core::assist::runs::active();
        let tools = crate::core::assist::projection::granted_tools(&self.broker, &agent.tools);
        let has_history = req.messages.iter().any(|m| m.role == Role::Assistant);

        let key = format!("{conversation}\u{0}{}", agent.id);
        let (session, fresh, loaded) = {
            let mut sessions = self.sessions.lock().await;
            let live = sessions
                .get(&key)
                .filter(|s| !s.dead.load(Ordering::SeqCst) && s.cwd == cwd && &s.command == command)
                .cloned();
            match live {
                Some(s) => (s, false, false),
                None => {
                    // A token for this session, when the bridge is up.
                    let (mcp_server, grant_id) =
                        match (&registry, crate::core::assist::projection::endpoint()) {
                            (Some(reg), Some(url)) => match crate::core::assist::projection::issue(
                                reg.db(),
                                &agent.id,
                                &conversation,
                                Some(&request_id),
                                &tools,
                                false,
                                crate::core::assist::projection::DEFAULT_TTL_MS,
                            ) {
                                Ok(issued) => {
                                    crate::core::assist::projection::attach_broker(
                                        &issued.id,
                                        self.broker.clone(),
                                    );
                                    (
                                        Some(crate::core::assist::projection::acp_mcp_server(
                                            &url,
                                            &issued.token,
                                        )),
                                        Some(issued.id),
                                    )
                                }
                                Err(_) => (None, None),
                            },
                            _ => (None, None),
                        };
                    // The agent's handle from an earlier app session, if the
                    // folder and command are the same.
                    let load = registry.as_ref().and_then(|reg| {
                        reg.session(&conversation, &agent.id)
                            .filter(|s| {
                                s.runtime == format!("acp:{command}")
                                    && s.cwd.as_deref() == Some(&*cwd.to_string_lossy())
                            })
                            .and_then(|s| s.provider_handle)
                    });
                    let (s, loaded) = start(
                        command,
                        args,
                        cwd.clone(),
                        StartOptions {
                            mcp_server,
                            grant_id: grant_id.clone(),
                            load,
                        },
                    )
                    .await
                    .map_err(|e| LlmError::new(ERR_ACP, e))?;
                    if s.grant_id.is_none() {
                        if let (Some(reg), Some(id)) = (&registry, &grant_id) {
                            crate::core::assist::projection::revoke(reg.db(), id);
                        }
                    }
                    sessions.insert(key, s.clone());
                    (s, true, loaded)
                }
            }
        };
        // Each turn: this turn's grants and run on the session's token.
        if let (Some(reg), Some(id)) = (&registry, &session.grant_id) {
            let _ = crate::core::assist::projection::refresh(
                reg.db(),
                id,
                Some(&request_id),
                &tools,
                crate::core::assist::projection::DEFAULT_TTL_MS,
            );
        }
        let resume_kind = if !fresh || loaded {
            crate::core::assist::runs::ResumeKind::Native
        } else if has_history {
            crate::core::assist::runs::ResumeKind::Replay
        } else {
            crate::core::assist::runs::ResumeKind::New
        };
        if let Some(reg) = &registry {
            let pins = crate::core::assist::runs::SessionPins {
                runtime: format!("acp:{command}"),
                account: None,
                exe_version: None,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                context_kind: if projectless {
                    "projectless"
                } else {
                    "project"
                }
                .into(),
                native_resume: session.load_session,
            };
            if let Ok((rec, _)) = reg.open_session(&conversation, &agent.id, &pins) {
                let _ = reg.set_handle(&rec.id, &session.session_id);
                let _ = reg.bind_session(
                    &request_id,
                    &rec.id,
                    resume_kind,
                    Some(&cwd.to_string_lossy()),
                );
            }
        }
        let fresh = fresh && !loaded;

        let (tx, rx) = mpsc::channel::<TurnEvent>(256);
        let broker = self.broker.clone();
        let agent_id = agent.id.clone();
        let cancel = req.cancel.clone();
        let prompt = prompt_for(&req, fresh);
        let run_registry = registry.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(TurnEvent::Started {
                    request_id: request_id.clone(),
                })
                .await;
            let (id, mut done) = session.register();
            let msg = json!({ "jsonrpc": "2.0", "id": id, "method": "session/prompt",
                "params": { "sessionId": session.session_id, "prompt": [{ "type": "text", "text": prompt }] } });
            if let Err(e) = session.write(&msg).await {
                let _ = tx
                    .send(TurnEvent::Error {
                        error: LlmError::new(ERR_ACP, e),
                    })
                    .await;
                let _ = tx
                    .send(TurnEvent::Finished {
                        reason: FinishReason::Other,
                    })
                    .await;
                return;
            }
            let mut inbound = session.inbound.lock().await;
            let mut cancelled = false;
            let reason = loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled(), if !cancelled => {
                        cancelled = true;
                        let _ = session.write(&json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session.session_id } })).await;
                    }
                    result = &mut done => {
                        break match result {
                            Ok(Ok(v)) => match v["stopReason"].as_str() {
                                Some("cancelled") => FinishReason::Cancelled,
                                Some("max_tokens") | Some("max_turn_requests") => FinishReason::Length,
                                _ => FinishReason::Stop,
                            },
                            Ok(Err(e)) => {
                                let _ = tx.send(TurnEvent::Error { error: LlmError::new(ERR_ACP, e) }).await;
                                FinishReason::Other
                            }
                            Err(_) => FinishReason::Other,
                        };
                    }
                    Some(msg) = inbound.recv() => {
                        let reply = handle_inbound(&msg, &session, &broker, &agent_id, &conversation, &request_id, &tx, run_registry.as_ref()).await;
                        if let (Some(id), Some(reply)) = (msg.id.clone(), reply) {
                            let body = match reply {
                                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                                Err(message) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message } }),
                            };
                            let _ = session.write(&body).await;
                        }
                    }
                }
            };
            let reason = if cancelled {
                FinishReason::Cancelled
            } else {
                reason
            };
            let _ = tx.send(TurnEvent::Finished { reason }).await;
        });
        Ok(
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) })
                .boxed(),
        )
    }
}

async fn handle_inbound(
    msg: &Inbound,
    session: &Session,
    broker: &ToolBroker,
    agent_id: &str,
    conversation: &str,
    request_id: &str,
    tx: &mpsc::Sender<TurnEvent>,
    registry: Option<&crate::core::assist::runs::Registry>,
) -> Option<Result<Value, String>> {
    let p = &msg.params;
    match msg.method.as_str() {
        "session/update" => {
            let u = &p["update"];
            let chunk = u["content"]["text"].as_str().unwrap_or("").to_string();
            match u["sessionUpdate"].as_str().unwrap_or("") {
                "agent_message_chunk" if !chunk.is_empty() => {
                    let _ = tx.send(TurnEvent::TextDelta { text: chunk }).await;
                }
                "agent_thought_chunk" if !chunk.is_empty() => {
                    let _ = tx.send(TurnEvent::ThinkingDelta { text: chunk }).await;
                }
                // The agent runs its own tools: shown, never re-executed by us.
                "tool_call" | "tool_call_update"
                    if registry.is_some() && u["toolCallId"].is_string() =>
                {
                    // Recorded once per id and status (the agent's own tools).
                    if let Some(reg) = registry {
                        let id = u["toolCallId"].as_str().unwrap_or("");
                        let status = u["status"].as_str().unwrap_or("pending");
                        let _ = reg.add_event(
                            request_id,
                            "tool_call",
                            Some(&format!("{id}:{status}")),
                            json!({
                                "name": u["title"].as_str().or(u["kind"].as_str()).unwrap_or("tool"),
                                "input": u.get("rawInput").cloned().unwrap_or(Value::Null),
                                "status": status,
                                "runtime": "acp",
                            }),
                        );
                    }
                    if u["sessionUpdate"] == "tool_call" {
                        let title = u["title"].as_str().unwrap_or("tool");
                        let _ = tx
                            .send(TurnEvent::ThinkingDelta {
                                text: format!("\n→ {title}\n"),
                            })
                            .await;
                    }
                }
                "tool_call" => {
                    let title = u["title"].as_str().unwrap_or("tool");
                    let _ = tx
                        .send(TurnEvent::ThinkingDelta {
                            text: format!("\n→ {title}\n"),
                        })
                        .await;
                }
                "plan" => {
                    let steps: Vec<String> = u["entries"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|e| {
                            format!(
                                "  [{}] {}",
                                e["status"].as_str().unwrap_or("pending"),
                                e["content"].as_str().unwrap_or("")
                            )
                        })
                        .collect();
                    if !steps.is_empty() {
                        let _ = tx
                            .send(TurnEvent::ThinkingDelta {
                                text: format!("\nPlan:\n{}\n", steps.join("\n")),
                            })
                            .await;
                    }
                }
                _ => {}
            }
            None
        }
        "session/request_permission" => {
            let call = &p["toolCall"];
            let title = call["title"].as_str().unwrap_or("tool call");
            let kind = call["kind"].as_str().unwrap_or("other");
            let call_id = call["toolCallId"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let raw = call
                .get("rawInput")
                .map(|v| v.to_string())
                .unwrap_or_default();
            let preview = format!("{title}\n{raw}");
            let label = format!("acp:{kind}");
            let subject = json!({ "command": title });
            let answer = match super::perm::decide(agent_id, &label, &subject) {
                Some(super::perm::Action::Allow) => Answer::Once,
                Some(super::perm::Action::Deny) => Answer::Deny,
                _ => {
                    broker
                        .ask_user(agent_id, request_id, &call_id, &label, &preview)
                        .await
                }
            };
            if matches!(answer, Answer::Always) {
                super::perm::add_rule(
                    agent_id,
                    super::perm::Rule {
                        tool: label.clone(),
                        pattern: "*".into(),
                        action: super::perm::Action::Allow,
                    },
                );
            }
            let want: &[&str] = match answer {
                Answer::Always => &["allow_always", "allow_once"],
                Answer::Once => &["allow_once", "allow_always"],
                Answer::Deny => &["reject_once", "reject_always"],
            };
            let options = p["options"].as_array().cloned().unwrap_or_default();
            let pick = want
                .iter()
                .find_map(|k| options.iter().find(|o| o["kind"].as_str() == Some(k)));
            Some(Ok(match pick.and_then(|o| o["optionId"].as_str()) {
                Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
                None => json!({ "outcome": { "outcome": "cancelled" } }),
            }))
        }
        "fs/read_text_file" => Some(
            inside(&session.cwd, p["path"].as_str().unwrap_or("")).and_then(|path| {
                let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let from = p["line"].as_u64().unwrap_or(1).max(1) as usize - 1;
                let content = match p["limit"].as_u64() {
                    Some(n) => text
                        .lines()
                        .skip(from)
                        .take(n as usize)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None if from > 0 => text.lines().skip(from).collect::<Vec<_>>().join("\n"),
                    None => text,
                };
                Ok(json!({ "content": content }))
            }),
        ),
        "fs/write_text_file" => {
            let path = match inside(&session.cwd, p["path"].as_str().unwrap_or("")) {
                Ok(path) => path,
                Err(e) => return Some(Err(e)),
            };
            let _ = super::snapshot::before_write(&session.cwd, conversation, request_id).await;
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            Some(
                std::fs::write(&path, p["content"].as_str().unwrap_or(""))
                    .map(|_| Value::Null)
                    .map_err(|e| e.to_string()),
            )
        }
        _ if msg.id.is_some() => Some(Err(format!(
            "method `{}` is not supported by this client",
            msg.method
        ))),
        _ => None,
    }
}

/// ACP-capable CLIs we know how to launch, for the detection in the Accounts tab.
pub const KNOWN: &[(&str, &str, &str, &[&str])] = &[
    ("gemini", "Gemini CLI", "gemini", &["--experimental-acp"]),
    (
        "claude-code-acp",
        "Claude Code (ACP)",
        "claude-code-acp",
        &[],
    ),
    ("codex-acp", "Codex (ACP)", "codex-acp", &[]),
    ("goose", "Goose", "goose", &["acp"]),
    ("opencode", "opencode", "opencode", &["acp"]),
];

/// Which of [`KNOWN`] are on the PATH right now.
pub async fn detect() -> Vec<Value> {
    let mut out = Vec::new();
    for (id, name, bin, args) in KNOWN {
        let probe = if cfg!(windows) { "where" } else { "which" };
        let found = crate::core::process::command(probe)
            .arg(bin)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            })
            .filter(|p| !p.is_empty());
        out.push(json!({ "id": id, "name": name, "command": bin, "args": args, "path": found, "installed": found.is_some() }));
    }
    out
}
