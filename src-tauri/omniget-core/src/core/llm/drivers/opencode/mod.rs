//! `opencode` driver: OpenCode through its own HTTP server (`opencode serve`)
//! instead of ACP. Our own HTTP + SSE client (reqwest + [`translate::SseParser`]),
//! no SDK.
//!
//! One server per thread: `opencode serve --hostname=127.0.0.1 --port=<free>`
//! with a random `OPENCODE_SERVER_PASSWORD` (HTTP Basic `opencode:<pw>`; a
//! request without it gets 401 — checked against 1.18.32). Readiness is the
//! stdout line `opencode server listening on http://…`.
//!
//! - session: `POST /session {title, permission: ruleset(access)}`; resume =
//!   `GET /session/{id}` (404 ⇒ new; other directory ⇒ `POST …/fork` into the
//!   new one) + `PATCH /session/{id} {permission}`.
//! - turn: `POST /session/{id}/prompt_async {parts, model?, agent}`; events
//!   from `GET /event` (SSE) until `session.idle`.
//! - interrupt: `POST /session/{id}/abort`.
//! - approvals: `permission.asked` → `request.opened`;
//!   `POST /permission/{id}/reply {reply: once|always|reject}`.
//! - questions: `question.asked` → `user-input.requested`;
//!   `POST /question/{id}/reply {answers}` or `…/reject`.
//! - rollback/fork: `POST /session/{id}/fork {messageID}` keeps the messages
//!   *before* `messageID` (verified live), so "keep N turns" forks at the
//!   (N+1)-th user message. OpenCode's own `revert` is not used: it rewrites
//!   files, and files are the checkpoints' job.
//! - MCP: `POST /mcp {name, config}` for the servers of
//!   [`super::acp::mcp_servers_for`].

pub mod translate;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;
use tokio::sync::Mutex;

use super::acp::proc::{self, Spawn, Tail};
use super::acp::McpServerSpec;
use super::{
    AccessMode, ApprovalDecision, Driver, DriverCapabilities, DriverError, DriverInstance,
    DriverRegistration, ErrorClass, InteractionMode, ProviderRefs, RequestResolvedPayload,
    RuntimeErrorPayload, RuntimeEvent, RuntimeEventKind, RuntimeSink, RuntimeWarningPayload,
    SessionExitedPayload, SessionStart, SessionStartedPayload, SessionState, SessionStatePayload,
    ThreadStartedPayload, TurnAbortedPayload, TurnCompletedPayload, TurnEndState, TurnStart,
    TurnStartResult, TurnStartedPayload, UserInputResolvedPayload, ValuePayload, ERR_DRIVER_FAILED,
    ERR_DRIVER_NO_SESSION, ERR_DRIVER_UNAVAILABLE,
};
use translate::{Out, Translator};

pub const KIND: &str = "opencode";
pub const ERR_OPENCODE: &str = "ERR_OPENCODE";
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const ABORT_GRACE: Duration = Duration::from_secs(10);

pub fn capabilities() -> DriverCapabilities {
    DriverCapabilities {
        rollback: true,
        fork: true,
        interrupt: true,
        approvals: true,
        user_input: true,
        model_switch: true,
        plan_mode: true,
        ..Default::default()
    }
}

/// Register the `opencode` driver kind. Call once (the threads host setup).
pub fn register() {
    super::register_driver(DriverRegistration {
        kind: KIND.into(),
        label: "OpenCode (server)".into(),
        capabilities: capabilities(),
        factory: Arc::new(|instance, sink| {
            Ok(Arc::new(OpenCodeDriver::new(instance, sink)?) as Arc<dyn Driver>)
        }),
    });
}

/// The default instance, when OpenCode can start here.
pub fn instances() -> Vec<DriverInstance> {
    let inst = DriverInstance::new(KIND, KIND, "OpenCode");
    if base_command(&inst).is_some() {
        vec![inst]
    } else {
        Vec::new()
    }
}

/// Program + leading args that run the OpenCode CLI (before `serve …`):
/// the instance's command, the `opencode` binary, or the ACP registry
/// install (its `… acp` launch minus `acp`).
pub fn base_command(instance: &DriverInstance) -> Option<(String, Vec<String>)> {
    if let Some(cmd) = instance.command.clone().filter(|c| !c.trim().is_empty()) {
        return Some((cmd, instance.args.clone()));
    }
    if let Some(bin) = proc::which("opencode") {
        return Some((bin.to_string_lossy().to_string(), Vec::new()));
    }
    let reg = crate::core::clitools::acp_registry::installed().remove("opencode")?;
    let mut args = reg.args;
    if args.last().map(String::as_str) == Some("acp") {
        args.pop();
    }
    Some((reg.command, args))
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

/// Parse the readiness line.
pub fn listening_url(line: &str) -> Option<String> {
    let rest = line.split("listening on").nth(1)?.trim();
    let url: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    url.starts_with("http")
        .then(|| url.trim_end_matches('/').to_string())
}

// ── Server ──────────────────────────────────────────────────────────────

struct Server {
    base: String,
    password: String,
    cwd: PathBuf,
    http: reqwest::Client,
    pid: Option<u32>,
    child: StdMutex<Option<tokio::process::Child>>,
    stderr: Tail,
    dead: Arc<AtomicBool>,
}

fn http_err(what: &str, e: impl std::fmt::Display) -> DriverError {
    DriverError::new(ERR_DRIVER_FAILED, format!("{ERR_OPENCODE}: {what}: {e}"))
}

impl Server {
    async fn start(instance: &DriverInstance, cwd: &PathBuf) -> Result<Arc<Self>, DriverError> {
        let (cmd, mut args) = base_command(instance).ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_UNAVAILABLE,
                "OpenCode is not installed (Tools → OpenCode)",
            )
        })?;
        let port = free_port().ok_or_else(|| http_err("no free port", "bind failed"))?;
        args.extend([
            "serve".to_string(),
            "--hostname=127.0.0.1".to_string(),
            format!("--port={port}"),
        ]);
        let password = uuid::Uuid::new_v4().simple().to_string();
        let mut env: Vec<(String, String)> = instance
            .env
            .iter()
            .map(|e| (e.name.clone(), e.value.clone()))
            .collect();
        env.push(("OPENCODE_SERVER_PASSWORD".into(), password.clone()));
        let spec = Spawn {
            command: cmd,
            args,
            env,
            env_remove: vec!["OPENCODE_SERVER_USERNAME".into()],
            cwd: Some(cwd.clone()),
        };
        let mut child = proc::spawn(&spec).map_err(|e| {
            DriverError::new(
                ERR_DRIVER_UNAVAILABLE,
                format!("could not start OpenCode: {e}"),
            )
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let tail = Tail::default();
        if let Some(e) = stderr {
            proc::drain_into(e, tail.clone());
        }
        let pid = child.id();
        let mut lines =
            tokio::io::BufReader::new(stdout.ok_or_else(|| http_err("spawn", "no stdout"))?)
                .lines();
        let ready = tokio::time::timeout(READY_TIMEOUT, async {
            while let Ok(Some(line)) = lines.next_line().await {
                tail.push(&line);
                tail.push("\n");
                if let Some(url) = listening_url(&line) {
                    return Some(url);
                }
            }
            None
        })
        .await;
        let base = match ready {
            Ok(Some(url)) => url,
            other => {
                proc::kill_tree(pid);
                let why = if other.is_err() {
                    "no readiness line in 60 s"
                } else {
                    "exited"
                };
                return Err(http_err(&format!("opencode serve {why}"), tail.text()));
            }
        };
        // Keep draining stdout: a full pipe blocks OpenCode.
        let rest = lines.into_inner();
        proc::drain_into(rest, tail.clone());
        let dead = Arc::new(AtomicBool::new(false));
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|e| http_err("http client", e))?;
        let server = Arc::new(Self {
            base,
            password,
            cwd: cwd.clone(),
            http,
            pid,
            child: StdMutex::new(Some(child)),
            stderr: tail,
            dead,
        });
        Ok(server)
    }

    fn url(&self, path: &str) -> String {
        let dir = self.cwd.to_string_lossy();
        let sep = if path.contains('?') { '&' } else { '?' };
        let enc: String = url_encode(&dir);
        format!("{}{}{}directory={}", self.base, path, sep, enc)
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value), DriverError> {
        let mut req = self
            .http
            .request(method, self.url(path))
            .basic_auth("opencode", Some(&self.password))
            .timeout(HTTP_TIMEOUT);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| http_err(path, e))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let v = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((status, v))
    }

    async fn ok(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, DriverError> {
        let (status, v) = self.call(method, path, body).await?;
        if (200..300).contains(&status) {
            Ok(v)
        } else {
            Err(http_err(
                &format!("{path} → HTTP {status}"),
                proc::sanitize(&v.to_string()),
            ))
        }
    }

    fn shutdown(&self) {
        self.dead.store(true, Ordering::SeqCst);
        proc::kill_tree(self.pid);
        if let Some(mut c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.start_kill();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if !self.dead.load(Ordering::SeqCst) {
            proc::kill_tree(self.pid);
        }
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── Session ─────────────────────────────────────────────────────────────

struct Turn {
    id: String,
    interrupted: bool,
}

struct Session {
    thread_id: String,
    instance_id: String,
    server: Arc<Server>,
    session_id: StdMutex<String>,
    translator: StdMutex<Translator>,
    turn: StdMutex<Option<Turn>>,
    last_turn: StdMutex<Option<String>>,
    access: StdMutex<AccessMode>,
    interaction: StdMutex<InteractionMode>,
    model: StdMutex<Option<String>>,
    sink: RuntimeSink,
    stopping: AtomicBool,
    pump: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Session {
    fn sid(&self) -> String {
        self.session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn event(&self, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(KIND, &self.instance_id, &self.thread_id, turn, kind)
    }

    fn emit(&self, turn: Option<&str>, kind: RuntimeEventKind) {
        let _ = self.sink.send(self.event(turn, kind));
    }

    fn state(&self, state: SessionState, reason: Option<&str>) {
        self.emit(
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state,
                reason: reason.map(str::to_string),
                detail: None,
            }),
        );
    }

    fn current_turn(&self) -> Option<String> {
        self.turn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|t| t.id.clone())
            .or_else(|| {
                self.last_turn
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            })
    }

    fn cursor(&self) -> Value {
        json!({ "sessionId": self.sid(), "cwd": self.server.cwd })
    }

    fn announce(&self, message: &str) {
        self.emit(
            None,
            RuntimeEventKind::SessionStarted(SessionStartedPayload {
                message: Some(message.to_string()),
                resume: Some(self.cursor()),
            }),
        );
        self.emit(
            None,
            RuntimeEventKind::ThreadStarted(ThreadStartedPayload {
                provider_thread_id: Some(self.sid()),
            }),
        );
    }

    fn deliver(self: &Arc<Self>, outs: Vec<Out>) {
        let turn = self.current_turn();
        for o in outs {
            match o {
                Out::Event {
                    kind,
                    item_id,
                    request_id,
                    provider_item_id,
                } => {
                    let waiting = matches!(
                        kind,
                        RuntimeEventKind::RequestOpened(_)
                            | RuntimeEventKind::UserInputRequested(_)
                    );
                    let t = match kind {
                        RuntimeEventKind::ThreadMetadataUpdated(_) => None,
                        _ => turn.clone(),
                    };
                    let mut ev = self.event(t.as_deref(), kind);
                    ev.item_id = item_id;
                    ev.request_id = request_id;
                    if let Some(p) = provider_item_id {
                        ev.provider_refs = Some(ProviderRefs {
                            provider_item_id: Some(p),
                            ..Default::default()
                        });
                    }
                    let _ = self.sink.send(ev);
                    if waiting {
                        self.state(SessionState::Waiting, Some("approval"));
                    }
                }
                Out::Permission { request_id, .. } => {
                    // Full access still gets asks that bypass the ruleset
                    // (doom-loop, subagents): answer `once`, never `always`.
                    if *self.access.lock().unwrap_or_else(|e| e.into_inner())
                        == AccessMode::FullAccess
                    {
                        let me = self.clone();
                        tokio::spawn(async move {
                            let _ = me
                                .reply_permission(&request_id, ApprovalDecision::Accept)
                                .await;
                        });
                    }
                }
                Out::TurnIdle => self.finish_turn(None),
                Out::TurnFailed(msg) => self.finish_turn(Some(msg)),
            }
        }
    }

    fn finish_turn(&self, error: Option<String>) {
        let Some(turn) = self.turn.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        *self.last_turn.lock().unwrap_or_else(|e| e.into_inner()) = Some(turn.id.clone());
        let (outs, usage, cost) = self
            .translator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .end_turn();
        for o in outs {
            if let Out::Event {
                kind,
                item_id,
                provider_item_id,
                ..
            } = o
            {
                let mut ev = self.event(Some(&turn.id), kind);
                ev.item_id = item_id;
                if let Some(p) = provider_item_id {
                    ev.provider_refs = Some(ProviderRefs {
                        provider_item_id: Some(p),
                        ..Default::default()
                    });
                }
                let _ = self.sink.send(ev);
            }
        }
        let state = if turn.interrupted {
            TurnEndState::Interrupted
        } else if error.is_some() {
            TurnEndState::Failed
        } else {
            TurnEndState::Completed
        };
        let mut usage = usage;
        usage.cost_usd = Some(cost);
        usage.model = self.model.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.emit(
            Some(&turn.id),
            RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                state,
                stop_reason: None,
                usage: Some(usage),
                total_cost_usd: Some(cost),
                error_message: error,
            }),
        );
        self.state(SessionState::Ready, None);
    }

    async fn reply_permission(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), DriverError> {
        let reply = translate::reply_for(decision);
        self.translator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mark_resolved(request_id);
        self.server
            .ok(
                reqwest::Method::POST,
                &format!("/permission/{request_id}/reply"),
                Some(json!({ "reply": reply })),
            )
            .await?;
        let rt = self
            .translator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission_type_of(request_id);
        let mut ev = self.event(
            self.current_turn().as_deref(),
            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                request_type: rt,
                decision: Some(decision),
                resolution: None,
            }),
        );
        ev.request_id = Some(request_id.to_string());
        let _ = self.sink.send(ev);
        Ok(())
    }

    /// The SSE pump: `GET /event` until the server goes away.
    fn start_pump(self: &Arc<Self>) {
        let me = self.clone();
        let handle = tokio::spawn(async move {
            let resp = me
                .server
                .http
                .get(me.server.url("/event"))
                .basic_auth("opencode", Some(&me.server.password))
                .header("Accept", "text/event-stream")
                .send()
                .await;
            let mut stream = match resp {
                Ok(r) if r.status().is_success() => r.bytes_stream(),
                Ok(r) => {
                    me.emit(
                        None,
                        RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                            message: format!("OpenCode event stream: HTTP {}", r.status()),
                            class: ErrorClass::TransportError,
                            code: Some(ERR_OPENCODE.into()),
                            detail: None,
                        }),
                    );
                    return;
                }
                Err(e) => {
                    me.emit(
                        None,
                        RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                            message: "OpenCode event stream failed".into(),
                            class: ErrorClass::TransportError,
                            code: Some(ERR_OPENCODE.into()),
                            detail: Some(e.to_string()),
                        }),
                    );
                    return;
                }
            };
            let mut parser = translate::SseParser::new();
            while let Some(chunk) = stream.next().await {
                let Ok(bytes) = chunk else { break };
                for data in parser.push(&bytes) {
                    let Ok(ev) = serde_json::from_str::<Value>(&data) else {
                        continue;
                    };
                    let outs = me
                        .translator
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .on_event(&ev);
                    if !outs.is_empty() {
                        me.deliver(outs);
                    }
                }
            }
            if me.stopping.load(Ordering::SeqCst) {
                return;
            }
            let tail = me.server.stderr.text();
            let msg = "OpenCode server stopped".to_string();
            if me.turn.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                me.emit(
                    me.current_turn().as_deref(),
                    RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                        message: msg.clone(),
                        class: ErrorClass::TransportError,
                        code: Some(ERR_OPENCODE.into()),
                        detail: (!tail.is_empty()).then(|| tail.clone()),
                    }),
                );
                me.finish_turn(Some(msg.clone()));
            }
            me.server.shutdown();
            me.emit(
                None,
                RuntimeEventKind::SessionExited(SessionExitedPayload {
                    reason: Some(if tail.is_empty() {
                        msg
                    } else {
                        format!("{msg}: {tail}")
                    }),
                    recoverable: Some(true),
                    exit_kind: Some("error".into()),
                }),
            );
        });
        *self.pump.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    async fn user_message_ids(&self) -> Result<Vec<String>, DriverError> {
        let msgs = self
            .server
            .ok(
                reqwest::Method::GET,
                &format!("/session/{}/message", self.sid()),
                None,
            )
            .await?;
        Ok(msgs
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter(|m| m.pointer("/info/role").and_then(Value::as_str) == Some("user"))
            .filter_map(|m| {
                m.pointer("/info/id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    /// Fork keeping the first `turn_count` turns; `None` keeps everything.
    async fn fork_at(&self, turn_count: Option<u32>) -> Result<String, DriverError> {
        let users = self.user_message_ids().await?;
        let body = match turn_count {
            Some(n) if (n as usize) < users.len() => json!({ "messageID": users[n as usize] }),
            _ => json!({}),
        };
        let forked = self
            .server
            .ok(
                reqwest::Method::POST,
                &format!("/session/{}/fork", self.sid()),
                Some(body),
            )
            .await?;
        forked
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| http_err("fork", "no session id"))
    }
}

// ── Driver ──────────────────────────────────────────────────────────────

pub struct OpenCodeDriver {
    instance: DriverInstance,
    sink: RuntimeSink,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Forked sessions waiting for their thread's `start_session`.
    pending_forks: StdMutex<HashMap<String, String>>,
}

fn parse_model(model: &str) -> Option<Value> {
    let (provider, id) = model.split_once('/')?;
    (!provider.is_empty() && !id.is_empty())
        .then(|| json!({ "providerID": provider, "modelID": id }))
}

impl OpenCodeDriver {
    pub fn new(instance: DriverInstance, sink: RuntimeSink) -> Result<Self, DriverError> {
        if base_command(&instance).is_none() {
            return Err(DriverError::new(
                ERR_DRIVER_UNAVAILABLE,
                "OpenCode is not installed (Tools → OpenCode)",
            ));
        }
        Ok(Self {
            instance,
            sink,
            sessions: Mutex::new(HashMap::new()),
            pending_forks: StdMutex::new(HashMap::new()),
        })
    }

    async fn live(&self, thread_id: &str) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .await
            .get(thread_id)
            .filter(|s| !s.server.dead.load(Ordering::SeqCst))
            .cloned()
    }

    fn no_session(thread_id: &str) -> DriverError {
        DriverError::new(
            ERR_DRIVER_NO_SESSION,
            format!("no live OpenCode session for thread {thread_id}"),
        )
    }

    async fn open(&self, input: &SessionStart) -> Result<Arc<Session>, DriverError> {
        let cwd = input
            .cwd
            .clone()
            .or_else(dirs::home_dir)
            .ok_or_else(|| http_err("session", "no workspace folder"))?;
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        self.emit_state(&input.thread_id, SessionState::Starting);
        let server = Server::start(&self.instance, &cwd).await?;
        let rules = translate::ruleset(input.access_mode);
        let resume = self
            .pending_forks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&input.thread_id)
            .or_else(|| {
                input
                    .resume_cursor
                    .as_ref()
                    .and_then(|c| {
                        c.get("sessionId")
                            .and_then(Value::as_str)
                            .or_else(|| c.as_str())
                    })
                    .map(str::to_string)
            });
        let mut warning = None;
        let sid = match resume {
            Some(id) => {
                let (status, info) = server
                    .call(reqwest::Method::GET, &format!("/session/{id}"), None)
                    .await?;
                if status == 404 {
                    warning = Some("the previous OpenCode session is gone; starting a new one");
                    None
                } else if (200..300).contains(&status) {
                    let dir = info
                        .get("directory")
                        .and_then(Value::as_str)
                        .map(PathBuf::from);
                    let same = dir
                        .map(|d| std::fs::canonicalize(&d).unwrap_or(d) == cwd)
                        .unwrap_or(true);
                    if same {
                        Some(id)
                    } else {
                        // The thread moved (worktree): keep the history there.
                        let forked = server
                            .ok(
                                reqwest::Method::POST,
                                &format!("/session/{id}/fork"),
                                Some(json!({})),
                            )
                            .await?;
                        forked.get("id").and_then(Value::as_str).map(str::to_string)
                    }
                } else {
                    server.shutdown();
                    return Err(http_err(
                        &format!("session {id} → HTTP {status}"),
                        proc::sanitize(&info.to_string()),
                    ));
                }
            }
            None => None,
        };
        let sid = match sid {
            Some(id) => {
                server
                    .ok(
                        reqwest::Method::PATCH,
                        &format!("/session/{id}"),
                        Some(json!({ "permission": rules })),
                    )
                    .await?;
                id
            }
            None => {
                let created = server
                    .ok(
                        reqwest::Method::POST,
                        "/session",
                        Some(json!({ "title": "OmniGet", "permission": rules })),
                    )
                    .await
                    .map_err(|e| {
                        server.shutdown();
                        e
                    })?;
                created
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| http_err("session.create", "no id"))?
            }
        };
        for spec in super::acp::mcp_servers_for(&input.thread_id, &self.instance.id) {
            let (name, config) = match spec {
                McpServerSpec::Http { name, url, headers }
                | McpServerSpec::Sse { name, url, headers } => (
                    name,
                    json!({ "type": "remote", "url": url,
                        "headers": headers.into_iter().collect::<HashMap<_, _>>() }),
                ),
                McpServerSpec::Stdio {
                    name,
                    command,
                    args,
                    env,
                } => {
                    let mut cmd = vec![command];
                    cmd.extend(args);
                    (
                        name,
                        json!({ "type": "local", "command": cmd,
                            "environment": env.into_iter().collect::<HashMap<_, _>>() }),
                    )
                }
            };
            if let Err(e) = server
                .ok(
                    reqwest::Method::POST,
                    "/mcp",
                    Some(json!({ "name": name, "config": config })),
                )
                .await
            {
                let _ = self.sink.send(RuntimeEvent::new(
                    KIND,
                    &self.instance.id,
                    &input.thread_id,
                    None,
                    RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                        message: format!("OpenCode did not take the MCP server `{name}`"),
                        detail: Some(e.to_string()),
                    }),
                ));
            }
        }
        let session = Arc::new(Session {
            thread_id: input.thread_id.clone(),
            instance_id: self.instance.id.clone(),
            server: server.clone(),
            session_id: StdMutex::new(sid.clone()),
            translator: StdMutex::new(Translator::new(&sid)),
            turn: StdMutex::new(None),
            last_turn: StdMutex::new(None),
            access: StdMutex::new(input.access_mode),
            interaction: StdMutex::new(input.interaction_mode),
            model: StdMutex::new(input.model.clone()),
            sink: self.sink.clone(),
            stopping: AtomicBool::new(false),
            pump: StdMutex::new(None),
        });
        session.start_pump();
        if let Some(w) = warning {
            session.emit(
                None,
                RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                    message: w.into(),
                    detail: None,
                }),
            );
        }
        session.announce("OpenCode ready");
        let health = server
            .ok(reqwest::Method::GET, "/global/health", None)
            .await
            .unwrap_or(Value::Null);
        session.emit(
            None,
            RuntimeEventKind::SessionConfigured(ValuePayload {
                value: json!({ "server": { "version": health.get("version") }, "permission": rules }),
            }),
        );
        session.state(SessionState::Ready, None);
        Ok(session)
    }

    fn emit_state(&self, thread_id: &str, state: SessionState) {
        let _ = self.sink.send(RuntimeEvent::new(
            KIND,
            &self.instance.id,
            thread_id,
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state,
                reason: None,
                detail: None,
            }),
        ));
    }

    async fn close(&self, s: &Arc<Session>, reason: &str) {
        s.stopping.store(true, Ordering::SeqCst);
        if s.turn.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            let _ = s
                .server
                .call(
                    reqwest::Method::POST,
                    &format!("/session/{}/abort", s.sid()),
                    None,
                )
                .await;
            if let Some(t) = s.turn.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                t.interrupted = true;
            }
            s.finish_turn(None);
        }
        if let Some(h) = s.pump.lock().unwrap_or_else(|e| e.into_inner()).take() {
            h.abort();
        }
        s.server.shutdown();
        s.emit(
            None,
            RuntimeEventKind::SessionExited(SessionExitedPayload {
                reason: Some(reason.into()),
                recoverable: Some(true),
                exit_kind: Some("graceful".into()),
            }),
        );
    }
}

#[async_trait]
impl Driver for OpenCodeDriver {
    fn kind(&self) -> &str {
        KIND
    }

    fn capabilities(&self) -> DriverCapabilities {
        capabilities()
    }

    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError> {
        if let Some(s) = self.live(&input.thread_id).await {
            let same = input
                .cwd
                .as_ref()
                .map(|c| std::fs::canonicalize(c).unwrap_or_else(|_| c.clone()) == s.server.cwd)
                .unwrap_or(true);
            if same {
                self.set_access_mode(&input.thread_id, input.access_mode)
                    .await?;
                *s.interaction.lock().unwrap_or_else(|e| e.into_inner()) = input.interaction_mode;
                if input.model.is_some() {
                    *s.model.lock().unwrap_or_else(|e| e.into_inner()) = input.model.clone();
                }
                return Ok(());
            }
            let old = self.sessions.lock().await.remove(&input.thread_id);
            if let Some(old) = old {
                self.close(&old, "workspace moved").await;
            }
        }
        let s = self.open(&input).await?;
        self.sessions
            .lock()
            .await
            .insert(input.thread_id.clone(), s);
        Ok(())
    }

    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError> {
        let s = self
            .live(&input.thread_id)
            .await
            .ok_or_else(|| Self::no_session(&input.thread_id))?;
        if s.turn.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return Err(DriverError::new(
                ERR_DRIVER_FAILED,
                "OpenCode is still working on the previous turn",
            ));
        }
        if *s.access.lock().unwrap_or_else(|e| e.into_inner()) != input.access_mode {
            self.set_access_mode(&input.thread_id, input.access_mode)
                .await?;
        }
        *s.interaction.lock().unwrap_or_else(|e| e.into_inner()) = input.interaction_mode;
        if input.model.is_some() {
            *s.model.lock().unwrap_or_else(|e| e.into_inner()) = input.model.clone();
        }
        let model = s.model.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut parts = vec![json!({ "type": "text", "text": input.text })];
        for a in &input.attachments {
            let mime = a
                .get("mimeType")
                .or_else(|| a.get("mime"))
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream");
            let url = match (
                a.get("data").and_then(Value::as_str),
                a.get("path").and_then(Value::as_str),
            ) {
                (Some(d), _) => format!("data:{mime};base64,{d}"),
                (None, Some(p)) => format!("file://{p}"),
                _ => continue,
            };
            let mut part = json!({ "type": "file", "mime": mime, "url": url });
            if let Some(name) = a.get("name").and_then(Value::as_str) {
                part["filename"] = json!(name);
            }
            parts.push(part);
        }
        let mut body = json!({
            "parts": parts,
            "agent": if input.interaction_mode == InteractionMode::Plan { "plan" } else { "build" },
        });
        if let Some(m) = model.as_deref().and_then(parse_model) {
            body["model"] = m;
        }
        s.translator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .begin_turn();
        *s.turn.lock().unwrap_or_else(|e| e.into_inner()) = Some(Turn {
            id: input.turn_id.clone(),
            interrupted: false,
        });
        s.emit(
            Some(&input.turn_id),
            RuntimeEventKind::TurnStarted(TurnStartedPayload {
                model: model.clone(),
                effort: None,
            }),
        );
        s.state(SessionState::Running, None);
        if let Err(e) = s
            .server
            .ok(
                reqwest::Method::POST,
                &format!("/session/{}/prompt_async", s.sid()),
                Some(body),
            )
            .await
        {
            s.turn.lock().unwrap_or_else(|e| e.into_inner()).take();
            s.translator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .end_turn();
            return Err(e);
        }
        Ok(TurnStartResult {
            resume_cursor: Some(s.cursor()),
        })
    }

    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError> {
        let s = self.live(thread_id).await;
        let active = s.as_ref().and_then(|s| {
            s.turn
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|t| t.id.clone())
        });
        let (Some(s), Some(active)) = (s, active) else {
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
        if let Some(t) = s.turn.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            t.interrupted = true;
        }
        s.server
            .ok(
                reqwest::Method::POST,
                &format!("/session/{}/abort", s.sid()),
                None,
            )
            .await?;
        let watch = s.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ABORT_GRACE).await;
            let still = watch
                .turn
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|t| t.id.clone());
            if still.as_deref() == Some(active.as_str()) {
                watch.finish_turn(None);
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
        let s = self
            .live(thread_id)
            .await
            .ok_or_else(|| Self::no_session(thread_id))?;
        s.reply_permission(request_id, decision).await?;
        if s.turn.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            s.state(SessionState::Running, None);
        }
        Ok(())
    }

    async fn respond_user_input(
        &self,
        thread_id: &str,
        request_id: &str,
        answers: Value,
    ) -> Result<(), DriverError> {
        let s = self
            .live(thread_id)
            .await
            .ok_or_else(|| Self::no_session(thread_id))?;
        let q = {
            let mut t = s.translator.lock().unwrap_or_else(|e| e.into_inner());
            t.mark_resolved(request_id);
            t.questions.get(request_id).cloned()
        };
        let empty = matches!(&answers, Value::Null)
            || answers.as_object().map(|o| o.is_empty()).unwrap_or(false);
        match (q, empty) {
            (Some(q), false) => {
                s.server
                    .ok(
                        reqwest::Method::POST,
                        &format!("/question/{request_id}/reply"),
                        Some(translate::question_answers(&q, &answers)),
                    )
                    .await?;
            }
            _ => {
                s.server
                    .ok(
                        reqwest::Method::POST,
                        &format!("/question/{request_id}/reject"),
                        None,
                    )
                    .await?;
            }
        }
        let mut ev = s.event(
            s.current_turn().as_deref(),
            RuntimeEventKind::UserInputResolved(UserInputResolvedPayload { answers }),
        );
        ev.request_id = Some(request_id.to_string());
        let _ = self.sink.send(ev);
        Ok(())
    }

    async fn rollback(&self, thread_id: &str, turn_count: u32) -> Result<(), DriverError> {
        let s = self
            .live(thread_id)
            .await
            .ok_or_else(|| Self::no_session(thread_id))?;
        let users = s.user_message_ids().await?;
        if (turn_count as usize) >= users.len() {
            return Ok(());
        }
        let new_id = s.fork_at(Some(turn_count)).await?;
        *s.session_id.lock().unwrap_or_else(|e| e.into_inner()) = new_id.clone();
        s.translator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_session(&new_id);
        let rules = translate::ruleset(*s.access.lock().unwrap_or_else(|e| e.into_inner()));
        let _ = s
            .server
            .ok(
                reqwest::Method::PATCH,
                &format!("/session/{new_id}"),
                Some(json!({ "permission": rules })),
            )
            .await;
        s.announce("OpenCode session rolled back");
        Ok(())
    }

    async fn fork(
        &self,
        source_thread_id: &str,
        target_thread_id: &str,
        turn_count: u32,
    ) -> Result<(), DriverError> {
        let s = self
            .live(source_thread_id)
            .await
            .ok_or_else(|| Self::no_session(source_thread_id))?;
        let new_id = s.fork_at(Some(turn_count)).await?;
        self.pending_forks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(target_thread_id.to_string(), new_id.clone());
        let cursor = json!({ "sessionId": new_id, "cwd": s.server.cwd });
        for kind in [
            RuntimeEventKind::SessionStarted(SessionStartedPayload {
                message: Some("OpenCode session forked".into()),
                resume: Some(cursor),
            }),
            RuntimeEventKind::ThreadStarted(ThreadStartedPayload {
                provider_thread_id: Some(new_id.clone()),
            }),
        ] {
            let _ = self.sink.send(RuntimeEvent::new(
                KIND,
                &self.instance.id,
                target_thread_id,
                None,
                kind,
            ));
        }
        Ok(())
    }

    async fn set_access_mode(&self, thread_id: &str, mode: AccessMode) -> Result<(), DriverError> {
        let Some(s) = self.live(thread_id).await else {
            return Ok(());
        };
        {
            let mut a = s.access.lock().unwrap_or_else(|e| e.into_inner());
            if *a == mode {
                return Ok(());
            }
            *a = mode;
        }
        s.server
            .ok(
                reqwest::Method::PATCH,
                &format!("/session/{}", s.sid()),
                Some(json!({ "permission": translate::ruleset(mode) })),
            )
            .await?;
        Ok(())
    }

    async fn stop(&self, thread_id: &str) -> Result<(), DriverError> {
        let s = self.sessions.lock().await.remove(thread_id);
        if let Some(s) = s {
            self.close(&s, "stopped").await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_line_and_models_parse() {
        assert_eq!(
            listening_url("opencode server listening on http://127.0.0.1:47611").as_deref(),
            Some("http://127.0.0.1:47611")
        );
        assert_eq!(listening_url("booting"), None);
        assert_eq!(
            parse_model("anthropic/claude-sonnet-4-5"),
            Some(json!({ "providerID": "anthropic", "modelID": "claude-sonnet-4-5" }))
        );
        assert_eq!(parse_model("big-pickle"), None);
        assert_eq!(url_encode("/a b/c"), "/a%20b/c");
    }
}

#[cfg(test)]
mod live_tests;
