//! [`CodexDriver`]: one `codex app-server` process per thread, driven over
//! JSON-RPC, with its events translated into [`RuntimeEvent`]s.
//!
//! Life of a thread: `start_session` spawns the app-server with the
//! account's `CODEX_HOME`, `initialize`s it and opens the Codex thread
//! (`thread/resume` with the stored cursor, falling back to `thread/start`
//! only when Codex says the thread does not exist). Turns go through
//! `turn/start`; approvals come back as server requests and wait, durably, for
//! `respond_request`. The process is closed after [`IDLE_SHUTDOWN`] without a
//! turn and reopened (resumed) on the next one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::super::super::cli_runtime::accounts::AccountStore;
use super::super::{
    AccessMode, ApprovalDecision, AuthStatusPayload, Driver, DriverCapabilities, DriverError,
    DriverInstance, InteractionMode, RuntimeEvent, RuntimeEventKind, RuntimeSink,
    RuntimeWarningPayload, SessionExitedPayload, SessionStart, SessionStartedPayload, SessionState,
    SessionStatePayload, ThreadStartedPayload, TurnStart, TurnStartResult, ERR_DRIVER_FAILED,
    ERR_DRIVER_NO_SESSION, ERR_DRIVER_UNAVAILABLE, ERR_DRIVER_UNSUPPORTED,
};
use super::launch::{self, codex_home, launch_spec, mode_config};
use super::protocol as p;
use super::rpc::{Incoming, RpcClient, RpcError};
use super::supervisor::{Connector, ProcessHandle, RestartBudget};
use super::translate::{AutoReply, Translator, DRIVER};

pub const IDLE_SHUTDOWN: Duration = Duration::from_secs(10 * 60);
const T_INIT: Duration = Duration::from_secs(20);
const T_THREAD: Duration = Duration::from_secs(30);
const T_TURN: Duration = Duration::from_secs(60);
const T_INTERRUPT: Duration = Duration::from_secs(10);
const T_CHILD_INTERRUPT: Duration = Duration::from_secs(3);
const T_PROBE: Duration = Duration::from_secs(5);

/// What the driver can do (also what `register` advertises).
pub fn capabilities() -> DriverCapabilities {
    DriverCapabilities {
        rollback: true,
        fork: true,
        interrupt: true,
        approvals: true,
        user_input: true,
        model_switch: true,
        plan_mode: true,
        compaction: false,
        continuation: false,
        steer: false,
        rate_limits: true,
    }
}

/// `{threadId}` (T3's `CodexResumeCursorSchema`), or a bare string.
pub fn cursor_thread(cursor: &Value) -> Option<String> {
    match cursor {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(o) => o
            .get("threadId")
            .or_else(|| o.get("thread_id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

pub fn cursor_of(provider_thread: &str) -> Value {
    json!({ "threadId": provider_thread })
}

/// `thread/resume` failed because Codex does not have that thread: start a
/// fresh one. Anything else is an error (never a silent fresh start).
pub fn is_missing_thread(err: &RpcError) -> bool {
    let msg = err.message().to_lowercase();
    msg.contains("thread")
        && [
            "not found",
            "missing thread",
            "no such thread",
            "unknown thread",
            "does not exist",
        ]
        .iter()
        .any(|w| msg.contains(w))
        || msg.contains("no rollout found")
}

fn rpc_err(what: &str, e: RpcError) -> DriverError {
    let code = match e {
        RpcError::Closed(_) => ERR_DRIVER_NO_SESSION,
        _ => ERR_DRIVER_FAILED,
    };
    DriverError::new(code, format!("{what}: {e}"))
}

/// `turn/start.input` from the text and the attachments. Images go by path
/// (`localImage`) or URL; other files are named in the prompt, since Codex
/// reads the workspace itself.
pub fn turn_input(text: &str, attachments: &[Value]) -> Vec<p::UserInput> {
    let mut text = text.to_string();
    let mut extra = Vec::new();
    for a in attachments {
        let path = a.get("path").and_then(Value::as_str);
        let url = a.get("url").and_then(Value::as_str);
        let mime = a
            .get("mime")
            .or_else(|| a.get("mimeType"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let kind = a
            .get("type")
            .or_else(|| a.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let looks_image = mime.starts_with("image/")
            || kind == "image"
            || path
                .map(|p| {
                    let l = p.to_lowercase();
                    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
                        .iter()
                        .any(|e| l.ends_with(e))
                })
                .unwrap_or(false);
        match (path, url, looks_image) {
            (Some(path), _, true) => extra.push(p::UserInput::LocalImage {
                detail: None,
                path: path.to_string(),
            }),
            (None, Some(url), true) => extra.push(p::UserInput::Image {
                detail: None,
                url: Some(url.to_string()),
                file_id: None,
            }),
            (Some(path), _, false) => {
                text.push_str(&format!("\n\n[attached file: {path}]"));
            }
            _ => {}
        }
    }
    let mut out = vec![p::UserInput::Text {
        text,
        text_elements: Some(Vec::new()),
    }];
    out.extend(extra);
    out
}

struct Session {
    rpc: RpcClient,
    tr: Arc<Mutex<Translator>>,
    provider_thread: String,
    process: Option<Arc<ProcessHandle>>,
    alive: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    /// Set when the process died on its own (counts against the budget).
    crashed: Arc<AtomicBool>,
    idle_gen: Arc<AtomicU64>,
    /// Last collaboration mode sent (`turn/start.collaborationMode`).
    /// `None` on a fresh process: the first turn always states the mode,
    /// since a resumed Codex thread may still carry an older one.
    interaction: Mutex<Option<InteractionMode>>,
}

impl Session {
    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.rpc.is_closed()
    }

    fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.rpc.close("stopped");
        if let Some(p) = &self.process {
            p.stop();
        }
    }
}

type Slot = Arc<tokio::sync::Mutex<Option<Arc<Session>>>>;

pub struct CodexDriver {
    instance: DriverInstance,
    sink: RuntimeSink,
    connector: Arc<dyn Connector>,
    accounts: Option<Arc<AccountStore>>,
    slots: Mutex<HashMap<String, Slot>>,
    /// Last `start_session` input per thread, to reopen after a crash/idle.
    known: Mutex<HashMap<String, SessionStart>>,
    /// Engine thread → Codex thread.
    cursors: Mutex<HashMap<String, String>>,
    budgets: Mutex<HashMap<String, RestartBudget>>,
    idle: Duration,
}

impl CodexDriver {
    pub fn new(
        instance: DriverInstance,
        sink: RuntimeSink,
        connector: Arc<dyn Connector>,
        accounts: Option<Arc<AccountStore>>,
    ) -> Self {
        Self {
            instance,
            sink,
            connector,
            accounts,
            slots: Mutex::new(HashMap::new()),
            known: Mutex::new(HashMap::new()),
            cursors: Mutex::new(HashMap::new()),
            budgets: Mutex::new(HashMap::new()),
            idle: IDLE_SHUTDOWN,
        }
    }

    pub fn with_idle(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    fn emit(&self, ev: RuntimeEvent) {
        let _ = self.sink.send(ev);
    }

    fn ev(&self, thread: &str, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(DRIVER, &self.instance.id, thread, turn, kind)
    }

    fn slot(&self, thread: &str) -> Slot {
        self.slots
            .lock()
            .map(|mut m| m.entry(thread.to_string()).or_default().clone())
            .unwrap_or_default()
    }

    async fn live(&self, thread: &str) -> Option<Arc<Session>> {
        let slot = self.slots.lock().ok()?.get(thread).cloned()?;
        let guard = slot.lock().await;
        guard.as_ref().filter(|s| s.alive()).cloned()
    }

    /// The live session of a thread, opening (or reopening, resumed) it.
    async fn ensure(&self, thread: &str) -> Result<Arc<Session>, DriverError> {
        let slot = self.slot(thread);
        let mut guard = slot.lock().await;
        if let Some(s) = guard.as_ref() {
            if s.alive() {
                return Ok(s.clone());
            }
            if s.crashed.load(Ordering::SeqCst) {
                let ok = self
                    .budgets
                    .lock()
                    .map(|mut b| {
                        b.entry(thread.to_string())
                            .or_default()
                            .allow(Instant::now())
                    })
                    .unwrap_or(true);
                if !ok {
                    return Err(DriverError::new(
                        ERR_DRIVER_FAILED,
                        format!(
                            "the Codex app-server crashed {} times in {} minutes; not restarting it",
                            super::supervisor::RESTART_LIMIT,
                            super::supervisor::RESTART_WINDOW.as_secs() / 60
                        ),
                    ));
                }
            }
        }
        let start = self
            .known
            .lock()
            .ok()
            .and_then(|k| k.get(thread).cloned())
            .ok_or_else(|| {
                DriverError::new(
                    ERR_DRIVER_NO_SESSION,
                    format!("no Codex session for {thread}"),
                )
            })?;
        let session = self.open(&start).await?;
        *guard = Some(session.clone());
        Ok(session)
    }

    async fn open(&self, start: &SessionStart) -> Result<Arc<Session>, DriverError> {
        let thread = start.thread_id.clone();
        self.emit(self.ev(
            &thread,
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state: SessionState::Starting,
                reason: None,
                detail: None,
            }),
        ));
        let home = codex_home(&self.instance, self.accounts.as_deref());
        let program = self
            .instance
            .command
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| "codex".into());
        let spec = launch::with_mcp(
            launch_spec(&self.instance, program.into(), start.cwd.clone(), home),
            &super::super::acp::mcp_servers_for(&thread, &self.instance.id),
        );
        let conn = match self.connector.connect(&spec).await {
            Ok(c) => c,
            Err(e) => {
                self.session_error(&thread, &e.message);
                return Err(e);
            }
        };
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let rpc = RpcClient::start(conn.reader, conn.writer, in_tx);
        let process = conn.process.map(Arc::new);

        let fail = |msg: String| {
            rpc.close("open failed");
            if let Some(p) = &process {
                p.stop();
            }
            msg
        };

        // Handshake.
        let init = match rpc
            .request_value("initialize", Some(launch::initialize_params()), T_INIT)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let tail = process
                    .as_ref()
                    .map(|p| p.stderr_tail())
                    .unwrap_or_default();
                let msg = fail(format!(
                    "Codex app-server did not start: {e}{}",
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!(" ({tail})")
                    }
                ));
                self.session_error(&thread, &msg);
                return Err(DriverError::new(ERR_DRIVER_UNAVAILABLE, msg));
            }
        };
        let _ = rpc.notify("initialized", None);
        let user_agent = init
            .get("userAgent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // Thread: resume, else start.
        let mode = mode_config(start.access_mode);
        let cursor = self
            .cursors
            .lock()
            .ok()
            .and_then(|c| c.get(&thread).cloned())
            .or_else(|| start.resume_cursor.as_ref().and_then(cursor_thread));
        let cwd = start.cwd.as_ref().map(|c| c.display().to_string());
        let mut opened: Option<(String, Option<String>, bool)> = None;
        if let Some(provider) = &cursor {
            let params = json!({
                "threadId": provider,
                "cwd": cwd,
                "approvalPolicy": mode.approval_policy,
                "sandbox": mode.sandbox,
                "approvalsReviewer": mode.reviewer,
                "model": start.model,
                "excludeTurns": true,
            });
            match rpc
                .request_value("thread/resume", Some(strip_nulls(params)), T_THREAD)
                .await
            {
                Ok(v) => {
                    // Decode only what is needed: history items of any
                    // version must never break a resume.
                    let id = v
                        .get("thread")
                        .and_then(|t| t.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or(provider)
                        .to_string();
                    let model = v.get("model").and_then(Value::as_str).map(str::to_string);
                    opened = Some((id, model, true));
                }
                Err(e) if is_missing_thread(&e) => {
                    tracing::warn!("[codex] thread {provider} is gone ({e}); starting a new one");
                    self.emit(
                        self.ev(
                            &thread,
                            None,
                            RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                                message:
                                    "Codex no longer has this conversation; a new one was started."
                                        .into(),
                                detail: Some(e.message()),
                            }),
                        ),
                    );
                }
                Err(e) => {
                    let msg = fail(format!("thread/resume failed: {e}"));
                    self.session_error(&thread, &msg);
                    return Err(DriverError::new(ERR_DRIVER_FAILED, msg));
                }
            }
        }
        let (provider, model, resumed) = match opened {
            Some(o) => o,
            None => {
                let params = p::ThreadStartParams {
                    approval_policy: Some(mode.approval_policy.clone()),
                    approvals_reviewer: Some(mode.reviewer.clone()),
                    cwd: cwd.clone(),
                    model: start.model.clone(),
                    sandbox: Some(mode.sandbox.clone()),
                    ..Default::default()
                };
                let v: Value = match rpc.request("thread/start", &params, T_THREAD).await {
                    Ok(v) => v,
                    Err(e) => {
                        let msg = fail(format!("thread/start failed: {e}"));
                        self.session_error(&thread, &msg);
                        return Err(DriverError::new(ERR_DRIVER_FAILED, msg));
                    }
                };
                let id = v
                    .get("thread")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        DriverError::new(
                            ERR_DRIVER_FAILED,
                            "thread/start answered without a thread id",
                        )
                    })?;
                let model = v.get("model").and_then(Value::as_str).map(str::to_string);
                (id, model, false)
            }
        };

        let mut tr = Translator::new(&self.instance.id, &thread);
        tr.set_provider_thread(&provider);
        tr.set_model(model.clone().or_else(|| start.model.clone()));
        let tr = Arc::new(Mutex::new(tr));
        if let Ok(mut c) = self.cursors.lock() {
            c.insert(thread.clone(), provider.clone());
        }

        let session = Arc::new(Session {
            rpc: rpc.clone(),
            tr: tr.clone(),
            provider_thread: provider.clone(),
            process,
            alive: Arc::new(AtomicBool::new(true)),
            stopping: Arc::new(AtomicBool::new(false)),
            crashed: Arc::new(AtomicBool::new(false)),
            idle_gen: Arc::new(AtomicU64::new(0)),
            interaction: Mutex::new(None),
        });
        self.spawn_pump(&thread, session.clone(), in_rx);

        self.emit(self.ev(
            &thread,
            None,
            RuntimeEventKind::SessionStarted(SessionStartedPayload {
                message: Some(if resumed {
                    format!("resumed ({user_agent})")
                } else {
                    user_agent.clone()
                }),
                resume: Some(cursor_of(&provider)),
            }),
        ));
        self.emit(self.ev(
            &thread,
            None,
            RuntimeEventKind::ThreadStarted(ThreadStartedPayload {
                provider_thread_id: Some(provider.clone()),
            }),
        ));
        self.emit(self.ev(
            &thread,
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state: SessionState::Ready,
                reason: None,
                detail: model,
            }),
        ));
        self.probe_account(&thread, &session);
        Ok(session)
    }

    fn session_error(&self, thread: &str, msg: &str) {
        self.emit(self.ev(
            thread,
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state: SessionState::Error,
                reason: Some(msg.to_string()),
                detail: None,
            }),
        ));
    }

    /// Login state and rate limits, once per process, in the background.
    fn probe_account(&self, thread: &str, session: &Arc<Session>) {
        let rpc = session.rpc.clone();
        let tr = session.tr.clone();
        let sink = self.sink.clone();
        let instance = self.instance.id.clone();
        let thread = thread.to_string();
        tokio::spawn(async move {
            if let Ok(acc) = rpc
                .request_value("account/read", Some(json!({})), T_PROBE)
                .await
            {
                let missing = acc.get("account").map(Value::is_null).unwrap_or(true)
                    && acc
                        .get("requiresOpenaiAuth")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                if missing {
                    let _ = sink.send(RuntimeEvent::new(
                        DRIVER,
                        &instance,
                        &thread,
                        None,
                        RuntimeEventKind::AuthStatus(AuthStatusPayload {
                            is_authenticating: Some(false),
                            output: Vec::new(),
                            error: Some(
                                "Codex is not logged in on this account. Run `codex login` in its terminal.".into(),
                            ),
                        }),
                    ));
                    return;
                }
            }
            if let Ok(v) = rpc
                .request_value("account/rateLimits/read", None, T_PROBE)
                .await
            {
                let ev = tr.lock().ok().and_then(|mut t| t.on_rate_limits_read(&v));
                if let Some(ev) = ev {
                    let _ = sink.send(ev);
                }
            }
        });
    }

    fn spawn_pump(
        &self,
        thread: &str,
        session: Arc<Session>,
        mut rx: mpsc::UnboundedReceiver<Incoming>,
    ) {
        let sink = self.sink.clone();
        let instance = self.instance.id.clone();
        let thread = thread.to_string();
        let idle = self.idle;
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    Incoming::Notification { method, params } => {
                        let events = session
                            .tr
                            .lock()
                            .map(|mut t| t.on_notification(&method, &params))
                            .unwrap_or_default();
                        for ev in events {
                            let _ = sink.send(ev);
                        }
                        if method == "turn/completed" {
                            schedule_idle(&session, &sink, &instance, &thread, idle);
                        }
                    }
                    Incoming::Request { id, method, params } => {
                        let outcome = session
                            .tr
                            .lock()
                            .map(|mut t| t.on_server_request(id.clone(), &method, &params))
                            .unwrap_or_default();
                        for ev in outcome.events {
                            let _ = sink.send(ev);
                        }
                        match outcome.reply {
                            Some(AutoReply::Result(v)) => {
                                let _ = session.rpc.respond(id, v);
                            }
                            Some(AutoReply::Error { code, message }) => {
                                session.rpc.respond_error(id, code, &message);
                            }
                            None => {}
                        }
                    }
                    Incoming::Closed { reason } => {
                        session.alive.store(false, Ordering::SeqCst);
                        let stopping = session.stopping.load(Ordering::SeqCst);
                        let code = match &session.process {
                            Some(p) => p.exit_code(Duration::from_millis(500)).await,
                            None => None,
                        };
                        let tail = session
                            .process
                            .as_ref()
                            .map(|p| p.stderr_tail())
                            .unwrap_or_default();
                        let events = if stopping {
                            Vec::new()
                        } else {
                            session.crashed.store(true, Ordering::SeqCst);
                            let mut msg = format!(
                                "The Codex app-server stopped ({}{})",
                                reason,
                                code.map(|c| format!(", exit code {c}")).unwrap_or_default()
                            );
                            if !tail.is_empty() {
                                msg.push_str(&format!(": {tail}"));
                            }
                            session
                                .tr
                                .lock()
                                .map(|mut t| t.abort_active(&msg))
                                .unwrap_or_default()
                        };
                        for ev in events {
                            let _ = sink.send(ev);
                        }
                        let _ = sink.send(RuntimeEvent::new(
                            DRIVER,
                            &instance,
                            &thread,
                            None,
                            RuntimeEventKind::SessionExited(SessionExitedPayload {
                                reason: Some(if stopping {
                                    "stopped".into()
                                } else {
                                    reason.clone()
                                }),
                                recoverable: Some(true),
                                exit_kind: Some(if stopping {
                                    "graceful".into()
                                } else {
                                    "error".into()
                                }),
                            }),
                        ));
                        if let Some(p) = &session.process {
                            p.stop();
                        }
                        break;
                    }
                }
            }
        });
    }

    fn remember(&self, input: &SessionStart) {
        if let Ok(mut k) = self.known.lock() {
            k.insert(input.thread_id.clone(), input.clone());
        }
    }

    fn update_known(&self, thread: &str, f: impl FnOnce(&mut SessionStart)) {
        if let Ok(mut k) = self.known.lock() {
            if let Some(s) = k.get_mut(thread) {
                f(s);
            }
        }
    }

    async fn list_turn_ids(&self, session: &Session) -> Result<Vec<String>, DriverError> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let params = json!({
                "threadId": session.provider_thread,
                "cursor": cursor,
                "limit": 100,
                "sortDirection": "asc",
                "itemsView": "notLoaded",
            });
            let page = session
                .rpc
                .request_value("thread/turns/list", Some(strip_nulls(params)), T_THREAD)
                .await;
            let page = match page {
                Ok(v) => v,
                Err(e) if e.is_unknown_method() => {
                    // Older CLI: full read.
                    let v = session
                        .rpc
                        .request_value(
                            "thread/read",
                            Some(json!({ "threadId": session.provider_thread, "includeTurns": true })),
                            T_THREAD,
                        )
                        .await
                        .map_err(|e| rpc_err("thread/read", e))?;
                    return Ok(v
                        .get("thread")
                        .and_then(|t| t.get("turns"))
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t.get("id").and_then(Value::as_str))
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default());
                }
                Err(e) => return Err(rpc_err("thread/turns/list", e)),
            };
            if let Some(data) = page.get("data").and_then(Value::as_array) {
                ids.extend(
                    data.iter()
                        .filter_map(|t| t.get("id").and_then(Value::as_str))
                        .map(str::to_string),
                );
            }
            match page.get("nextCursor").and_then(Value::as_str) {
                Some(next) => {
                    if !seen.insert(next.to_string()) {
                        return Err(DriverError::new(
                            ERR_DRIVER_FAILED,
                            "thread/turns/list returned a cursor cycle",
                        ));
                    }
                    cursor = Some(next.to_string());
                }
                None => break,
            }
        }
        Ok(ids)
    }
}

/// Removes `null` members so optional params are omitted, not sent as null.
fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(o.into_iter().filter(|(_, v)| !v.is_null()).collect()),
        other => other,
    }
}

fn schedule_idle(
    session: &Arc<Session>,
    sink: &RuntimeSink,
    instance: &str,
    thread: &str,
    idle: Duration,
) {
    let gen = session.idle_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let s = session.clone();
    let sink = sink.clone();
    let instance = instance.to_string();
    let thread = thread.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(idle).await;
        if s.idle_gen.load(Ordering::SeqCst) != gen || !s.alive() {
            return;
        }
        let busy =
            s.tr.lock()
                .map(|t| t.active_engine_turn().is_some())
                .unwrap_or(true);
        if busy {
            return;
        }
        let _ = sink.send(RuntimeEvent::new(
            DRIVER,
            &instance,
            &thread,
            None,
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state: SessionState::Stopped,
                reason: Some("idle".into()),
                detail: None,
            }),
        ));
        s.shutdown();
    });
}

#[async_trait]
impl Driver for CodexDriver {
    fn kind(&self) -> &str {
        DRIVER
    }

    fn capabilities(&self) -> DriverCapabilities {
        capabilities()
    }

    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError> {
        if let Some(provider) = input.resume_cursor.as_ref().and_then(cursor_thread) {
            if let Ok(mut c) = self.cursors.lock() {
                c.entry(input.thread_id.clone()).or_insert(provider);
            }
        }
        self.remember(&input);
        self.ensure(&input.thread_id).await.map(|_| ())
    }

    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError> {
        self.update_known(&input.thread_id, |s| {
            s.access_mode = input.access_mode;
            s.interaction_mode = input.interaction_mode;
            if input.model.is_some() {
                s.model = input.model.clone();
            }
        });
        let session = self.ensure(&input.thread_id).await?;
        session.idle_gen.fetch_add(1, Ordering::SeqCst);
        let mode = mode_config(input.access_mode);
        let cwd = self
            .known
            .lock()
            .ok()
            .and_then(|k| k.get(&input.thread_id).and_then(|s| s.cwd.clone()))
            .map(|c| c.display().to_string());
        let params = p::TurnStartParams {
            thread_id: session.provider_thread.clone(),
            input: turn_input(&input.text, &input.attachments),
            approval_policy: Some(mode.approval_policy),
            approvals_reviewer: Some(mode.reviewer),
            sandbox_policy: Some(mode.sandbox_policy),
            model: input.model.clone(),
            cwd,
            ..Default::default()
        };
        let mut value = serde_json::to_value(&params)
            .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, format!("turn/start params: {e}")))?;
        // Plan mode is `collaborationMode` (experimental API, enabled in
        // `initialize`). Sent when it changes; it sticks for later turns.
        let last = session.interaction.lock().ok().and_then(|m| *m);
        if last != Some(input.interaction_mode) {
            let model = input
                .model
                .clone()
                .or_else(|| {
                    session
                        .tr
                        .lock()
                        .ok()
                        .and_then(|t| t.model().map(str::to_string))
                })
                .unwrap_or_default();
            value["collaborationMode"] = json!({
                "mode": if input.interaction_mode == InteractionMode::Plan { "plan" } else { "default" },
                "settings": { "model": model, "reasoning_effort": null, "developer_instructions": null },
            });
        }
        if let Ok(mut t) = session.tr.lock() {
            t.begin_turn(&input.turn_id);
            if input.model.is_some() {
                t.set_model(input.model.clone());
            }
        }
        match session
            .rpc
            .request_value("turn/start", Some(value), T_TURN)
            .await
        {
            Ok(v) => {
                if let Ok(mut m) = session.interaction.lock() {
                    *m = Some(input.interaction_mode);
                }
                if let Some(id) = v
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                {
                    if let Ok(mut t) = session.tr.lock() {
                        t.bind_provider_turn(id);
                    }
                }
                Ok(TurnStartResult {
                    resume_cursor: Some(cursor_of(&session.provider_thread)),
                })
            }
            Err(e) => {
                if let Ok(mut t) = session.tr.lock() {
                    t.turn_rejected();
                }
                Err(rpc_err("turn/start", e))
            }
        }
    }

    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError> {
        let Some(session) = self.live(thread_id).await else {
            if let Some(turn) = turn_id {
                let mut tr = Translator::new(&self.instance.id, thread_id);
                self.emit(tr.aborted(turn, "interrupted"));
            }
            return Ok(());
        };
        // 1. Parked prompts first, or Stop deadlocks.
        let (answers, events) = session
            .tr
            .lock()
            .map(|mut t| t.cancel_all_pending())
            .unwrap_or_default();
        for (id, result) in answers {
            let _ = session.rpc.respond(id, result);
        }
        for ev in events {
            self.emit(ev);
        }
        // 2. Sub-agent turns, bounded.
        let children = session
            .tr
            .lock()
            .map(|t| t.child_turns())
            .unwrap_or_default();
        let deadline = Instant::now() + Duration::from_secs(10);
        for (child, turn) in children {
            if Instant::now() > deadline {
                break;
            }
            let _ = session
                .rpc
                .request_value(
                    "turn/interrupt",
                    Some(json!({ "threadId": child, "turnId": turn })),
                    T_CHILD_INTERRUPT,
                )
                .await;
        }
        // 3. The root's active turn (not a queued one).
        let active = session
            .tr
            .lock()
            .ok()
            .and_then(|t| t.active_provider_turn().map(str::to_string));
        let Some(active) = active else {
            if let Some(turn) = turn_id {
                let ev = session
                    .tr
                    .lock()
                    .map(|mut t| t.aborted(turn, "interrupted"));
                if let Ok(ev) = ev {
                    self.emit(ev);
                }
            }
            return Ok(());
        };
        match session
            .rpc
            .request_value(
                "turn/interrupt",
                Some(json!({ "threadId": session.provider_thread, "turnId": active })),
                T_INTERRUPT,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(RpcError::Remote { message, .. }) if message.contains("no active turn") => {
                if let Some(turn) = turn_id {
                    let ev = session
                        .tr
                        .lock()
                        .map(|mut t| t.aborted(turn, "interrupted"));
                    if let Ok(ev) = ev {
                        self.emit(ev);
                    }
                }
                Ok(())
            }
            Err(e) => Err(rpc_err("turn/interrupt", e)),
        }
    }

    async fn respond_request(
        &self,
        thread_id: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), DriverError> {
        let session = self.live(thread_id).await.ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_NO_SESSION,
                "the Codex process that asked this is gone; the request is stale",
            )
        })?;
        let ((id, result), events) = session
            .tr
            .lock()
            .map_err(|_| DriverError::new(ERR_DRIVER_FAILED, "translator lock poisoned"))?
            .answer_request(request_id, decision)?;
        session
            .rpc
            .respond(id, result)
            .map_err(|e| rpc_err("answer", e))?;
        for ev in events {
            self.emit(ev);
        }
        Ok(())
    }

    async fn respond_user_input(
        &self,
        thread_id: &str,
        request_id: &str,
        answers: Value,
    ) -> Result<(), DriverError> {
        if request_id.starts_with("codex-async:") {
            return Err(DriverError::new(
                ERR_DRIVER_UNSUPPORTED,
                "answer this Codex question with a normal message",
            ));
        }
        let session = self.live(thread_id).await.ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_NO_SESSION,
                "the Codex process that asked this is gone",
            )
        })?;
        let ((id, result), events) = session
            .tr
            .lock()
            .map_err(|_| DriverError::new(ERR_DRIVER_FAILED, "translator lock poisoned"))?
            .answer_user_input(request_id, &answers)?;
        session
            .rpc
            .respond(id, result)
            .map_err(|e| rpc_err("answer", e))?;
        for ev in events {
            self.emit(ev);
        }
        Ok(())
    }

    async fn rollback(&self, thread_id: &str, turn_count: u32) -> Result<(), DriverError> {
        let session = self.ensure(thread_id).await?;
        let ids = self.list_turn_ids(&session).await?;
        let keep = turn_count as usize;
        if ids.len() <= keep {
            return Ok(());
        }
        let revert = session
            .rpc
            .request_value(
                "thread/revert",
                Some(json!({ "threadId": session.provider_thread, "beforeTurnId": ids[keep] })),
                T_THREAD,
            )
            .await;
        match revert {
            Ok(_) => {}
            Err(e) if e.is_unknown_method() => {
                session
                    .rpc
                    .request_value(
                        "thread/rollback",
                        Some(json!({ "threadId": session.provider_thread, "numTurns": ids.len() - keep })),
                        T_THREAD,
                    )
                    .await
                    .map_err(|e| rpc_err("thread/rollback", e))?;
            }
            Err(e) => return Err(rpc_err("thread/revert", e)),
        }
        if let Ok(mut t) = session.tr.lock() {
            t.reset_usage_baseline();
        }
        Ok(())
    }

    async fn fork(&self, source: &str, target: &str, turn_count: u32) -> Result<(), DriverError> {
        if turn_count == 0 {
            // Nothing to copy: the target opens a fresh Codex thread.
            return Ok(());
        }
        let session = self.ensure(source).await?;
        let ids = self.list_turn_ids(&session).await?;
        if ids.is_empty() {
            return Ok(());
        }
        let last = ids[(turn_count as usize).min(ids.len()) - 1].clone();
        let known = self.known.lock().ok().and_then(|k| k.get(source).cloned());
        let mode = mode_config(known.as_ref().map(|k| k.access_mode).unwrap_or_default());
        let params = json!({
            "threadId": session.provider_thread,
            "lastTurnId": last,
            "excludeTurns": true,
            "cwd": known.as_ref().and_then(|k| k.cwd.as_ref()).map(|c| c.display().to_string()),
            "approvalPolicy": mode.approval_policy,
            "sandbox": mode.sandbox,
            "approvalsReviewer": mode.reviewer,
        });
        let v = session
            .rpc
            .request_value("thread/fork", Some(strip_nulls(params)), T_THREAD)
            .await
            .map_err(|e| rpc_err("thread/fork", e))?;
        let new_id = v
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                DriverError::new(
                    ERR_DRIVER_FAILED,
                    "thread/fork answered without a thread id",
                )
            })?;
        // The fork was loaded in the source's process; the target reopens it
        // in its own.
        let _ = session
            .rpc
            .request_value(
                "thread/unsubscribe",
                Some(json!({ "threadId": new_id })),
                T_PROBE,
            )
            .await;
        if let Ok(mut c) = self.cursors.lock() {
            c.insert(target.to_string(), new_id.clone());
        }
        self.emit(self.ev(
            target,
            None,
            RuntimeEventKind::SessionStarted(SessionStartedPayload {
                message: Some(format!("forked from {source}")),
                resume: Some(cursor_of(&new_id)),
            }),
        ));
        self.emit(self.ev(
            target,
            None,
            RuntimeEventKind::ThreadStarted(ThreadStartedPayload {
                provider_thread_id: Some(new_id),
            }),
        ));
        Ok(())
    }

    async fn set_access_mode(&self, thread_id: &str, mode: AccessMode) -> Result<(), DriverError> {
        // Applied by the next `turn/start` (its overrides stick).
        self.update_known(thread_id, |s| s.access_mode = mode);
        Ok(())
    }

    async fn stop(&self, thread_id: &str) -> Result<(), DriverError> {
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|m| m.get(thread_id).cloned());
        let Some(slot) = slot else { return Ok(()) };
        let session = slot.lock().await.take();
        if let Some(session) = session {
            let (answers, events) = session
                .tr
                .lock()
                .map(|mut t| t.cancel_all_pending())
                .unwrap_or_default();
            for (id, result) in answers {
                let _ = session.rpc.respond(id, result);
            }
            for ev in events {
                self.emit(ev);
            }
            // Closing stdin ends the server; the pump announces the exit.
            session.shutdown();
        }
        Ok(())
    }
}

impl Drop for CodexDriver {
    fn drop(&mut self) {
        if let Ok(slots) = self.slots.lock() {
            for slot in slots.values() {
                if let Ok(guard) = slot.try_lock() {
                    if let Some(s) = guard.as_ref() {
                        s.shutdown();
                    }
                }
            }
        }
    }
}
