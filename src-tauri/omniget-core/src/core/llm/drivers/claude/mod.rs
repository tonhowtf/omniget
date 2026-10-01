pub mod cursor;
pub mod diff;
pub mod protocol;
pub mod supervisor;
pub mod translate;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::{
    register_driver, AccessMode, ApprovalDecision, Driver, DriverCapabilities, DriverError,
    DriverInstance, DriverRegistration, ErrorClass, InteractionMode, RuntimeErrorPayload,
    RuntimeEvent, RuntimeEventKind, RuntimeSink, RuntimeWarningPayload, SessionExitedPayload,
    SessionStart, SessionStartedPayload, TurnAbortedPayload, TurnEndState, TurnStart,
    TurnStartResult, ERR_DRIVER_FAILED, ERR_DRIVER_NO_SESSION, ERR_DRIVER_UNAVAILABLE,
};
use cursor::{ClaudeCursor, CursorStore};
use protocol::Launch;
use supervisor::{Exit, ProcHandle, SpawnSpec};
use translate::{Output, Translator, DRIVER};

const INTERRUPT_WATCHDOG: Duration = Duration::from_secs(10);
const MAX_CRASHES: u32 = 3;

pub fn capabilities() -> DriverCapabilities {
    DriverCapabilities {
        rollback: true,
        fork: true,
        interrupt: true,
        approvals: true,
        user_input: true,
        model_switch: true,
        plan_mode: true,
        compaction: true,
        continuation: false,
        steer: true,
        rate_limits: true,
    }
}

pub fn registration() -> DriverRegistration {
    DriverRegistration {
        kind: DRIVER.into(),
        label: "Claude Code".into(),
        capabilities: capabilities(),
        factory: Arc::new(|instance, sink| {
            Ok(Arc::new(ClaudeDriver::new(instance, sink)) as Arc<dyn Driver>)
        }),
    }
}

pub fn register() {
    register_driver(registration());
}

fn shared_store() -> Arc<CursorStore> {
    static STORE: OnceLock<Arc<CursorStore>> = OnceLock::new();
    STORE
        .get_or_init(|| Arc::new(CursorStore::at(CursorStore::default_path())))
        .clone()
}

#[derive(Debug, Clone, Default)]
struct Config {
    cwd: Option<PathBuf>,
    model: Option<String>,
    access: AccessMode,
    interaction: InteractionMode,
    mcp_config: Option<String>,
}

struct Inner {
    tr: Translator,
    config: Config,
    proc: Option<ProcHandle>,
    generation: u64,
    launched_full: bool,
    mode: String,
    control_seq: u64,
    waiters: HashMap<String, oneshot::Sender<(bool, Value)>>,
    crashes: u32,
    inits_at_spawn: u64,
    hard_interrupt: bool,
    persisted: ClaudeCursor,
}

struct Session {
    thread_id: String,
    inner: Mutex<Inner>,
}

fn embedded_mcp_path(thread_id: &str) -> Option<PathBuf> {
    use sha2::Digest;
    let h = hex::encode(sha2::Sha256::digest(thread_id.as_bytes()));
    crate::core::llm::roster_store::llm_dir()
        .map(|d| d.join("mcp").join(format!("claude-{}.json", &h[..16])))
}

fn embedded_mcp_file(thread_id: &str, instance_id: &str) -> Option<String> {
    let servers = super::acp::mcp_servers_for(thread_id, instance_id);
    if servers.is_empty() {
        return None;
    }
    let path = embedded_mcp_path(thread_id)?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    let body = serde_json::to_vec_pretty(&protocol::mcp_config_json(&servers)).ok()?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut f = opts.open(&path).ok()?;
    f.write_all(&body).ok()?;
    Some(path.display().to_string())
}

fn remove_embedded_mcp_file(thread_id: &str) {
    if let Some(p) = embedded_mcp_path(thread_id) {
        let _ = std::fs::remove_file(p);
    }
}

struct Shared {
    instance: DriverInstance,
    sink: RuntimeSink,
    store: Arc<CursorStore>,
}

pub struct ClaudeDriver {
    shared: Arc<Shared>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    binary: Mutex<Option<PathBuf>>,
}

impl Shared {
    fn emit(&self, ev: RuntimeEvent) {
        let _ = self.sink.send(ev);
    }

    fn event(&self, thread_id: &str, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(DRIVER, &self.instance.id, thread_id, turn, kind)
    }

    fn persist(&self, session: &Session, inner: &mut Inner) {
        if inner.tr.cursor != inner.persisted {
            inner.persisted = inner.tr.cursor.clone();
            self.store.put(&session.thread_id, &inner.persisted);
        }
    }

    fn on_line(&self, session: &Arc<Session>, generation: u64, line: &str) {
        let mut inner = session.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != generation {
            return;
        }
        let outputs = inner.tr.on_line(line, &|p| diff::read_for_preview(p));
        for out in outputs {
            match out {
                Output::Event(ev) => {
                    if let RuntimeEventKind::TurnCompleted(p) = &ev.kind {
                        if p.state == TurnEndState::Completed {
                            inner.crashes = 0;
                        }
                    }
                    self.emit(ev);
                }
                Output::Send(v) => {
                    if let Some(p) = &inner.proc {
                        let _ = p.stdin.send(v.to_string());
                    }
                }
                Output::ControlResult {
                    request_id,
                    ok,
                    payload,
                } => {
                    if !ok {
                        tracing::debug!("[claude] control {request_id} failed: {payload}");
                    }
                    if let Some(w) = inner.waiters.remove(&request_id) {
                        let _ = w.send((ok, payload));
                    }
                }
            }
        }
        self.persist(session, &mut inner);
    }

    fn on_exit(&self, session: &Arc<Session>, generation: u64, exit: Exit) {
        let mut inner = session.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != generation || inner.proc.is_none() {
            return;
        }
        inner.proc = None;
        inner.waiters.clear();
        let hard = std::mem::take(&mut inner.hard_interrupt);
        let tail: String = exit
            .stderr_tail
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" / ");
        let reason = if hard {
            "Interrupted (Claude did not stop in time; the process was stopped)".to_string()
        } else {
            format!(
                "Claude Code exited ({}){}",
                exit.code
                    .map(|c| format!("code {c}"))
                    .unwrap_or_else(|| "signal".into()),
                if tail.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            )
        };
        let thread = session.thread_id.clone();
        let live = inner.tr.has_live_turn();
        if !hard {
            inner.crashes += 1;
            if inner.tr.inits() == inner.inits_at_spawn && !inner.tr.cursor.session_id.is_empty() {
                inner.tr.cursor = ClaudeCursor::default();
                self.emit(self.event(
                    &thread,
                    None,
                    RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                        message: "Claude could not resume the saved session; the next message starts a new one".into(),
                        detail: Some(reason.clone()),
                    }),
                ));
            }
            let code = crate::core::llm::cli_runtime::parse::classify_error_text(&exit.stderr_tail);
            self.emit(self.event(
                &thread,
                inner.tr.active_turn().as_deref(),
                RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                    message: reason.clone(),
                    class: if code.is_some() {
                        ErrorClass::ProviderError
                    } else {
                        ErrorClass::TransportError
                    },
                    code: code.map(str::to_string),
                    detail: exit.code.map(|c| c.to_string()),
                }),
            ));
        }
        let state = if hard {
            TurnEndState::Interrupted
        } else {
            TurnEndState::Failed
        };
        for ev in inner.tr.abort_all(state, &reason) {
            self.emit(ev);
        }
        inner.tr.reset_process();
        self.persist(session, &mut inner);
        self.emit(self.event(
            &thread,
            None,
            RuntimeEventKind::SessionExited(SessionExitedPayload {
                reason: Some(reason),
                recoverable: Some(true),
                exit_kind: Some(if hard || !live { "graceful" } else { "error" }.into()),
            }),
        ));
    }
}

impl ClaudeDriver {
    pub fn new(instance: DriverInstance, sink: RuntimeSink) -> Self {
        Self::with_store(instance, sink, shared_store())
    }

    pub fn with_store(
        instance: DriverInstance,
        sink: RuntimeSink,
        store: Arc<CursorStore>,
    ) -> Self {
        let binary = instance
            .command
            .as_ref()
            .filter(|c| !c.trim().is_empty())
            .map(PathBuf::from);
        Self {
            shared: Arc::new(Shared {
                instance,
                sink,
                store,
            }),
            sessions: Mutex::new(HashMap::new()),
            binary: Mutex::new(binary),
        }
    }

    fn emit(&self, ev: RuntimeEvent) {
        self.shared.emit(ev);
    }

    fn event(&self, thread_id: &str, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        self.shared.event(thread_id, turn, kind)
    }

    fn persist(&self, session: &Session, inner: &mut Inner) {
        self.shared.persist(session, inner);
    }

    fn session(&self, thread_id: &str) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(thread_id)
            .cloned()
    }

    fn stored_cursor(&self, thread_id: &str, fallback: Option<&Value>) -> ClaudeCursor {
        self.shared
            .store
            .get(thread_id)
            .or_else(|| fallback.and_then(ClaudeCursor::from_value))
            .unwrap_or_default()
    }

    fn session_or_create(&self, thread_id: &str, cursor: Option<&Value>) -> Arc<Session> {
        let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = map.get(thread_id) {
            return s.clone();
        }
        let cursor = self.stored_cursor(thread_id, cursor);
        let s = Arc::new(Session {
            thread_id: thread_id.to_string(),
            inner: Mutex::new(Inner {
                tr: Translator::new(&self.shared.instance.id, thread_id, None, cursor.clone()),
                config: Config::default(),
                proc: None,
                generation: 0,
                launched_full: false,
                mode: String::new(),
                control_seq: 0,
                waiters: HashMap::new(),
                crashes: 0,
                inits_at_spawn: 0,
                hard_interrupt: false,
                persisted: cursor,
            }),
        });
        map.insert(thread_id.to_string(), s.clone());
        s
    }

    async fn binary(&self) -> Result<PathBuf, DriverError> {
        if let Some(p) = self
            .binary
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(p);
        }
        let found = crate::core::dependencies::find_tool("claude")
            .await
            .ok_or_else(|| {
                DriverError::new(
                    ERR_DRIVER_UNAVAILABLE,
                    "Claude Code (`claude`) is not installed or not on PATH",
                )
            })?;
        *self.binary.lock().unwrap_or_else(|e| e.into_inner()) = Some(found.clone());
        Ok(found)
    }

    fn control(inner: &mut Inner, request: Value) -> Option<String> {
        let proc = inner.proc.as_ref()?;
        inner.control_seq += 1;
        let id = format!("omniget-{}", inner.control_seq);
        let line = protocol::control_request(&id, request).to_string();
        proc.stdin.send(line).ok()?;
        Some(id)
    }

    async fn spawn(&self, session: &Arc<Session>) -> Result<(), DriverError> {
        let program = self.binary().await?;
        let mut inner = session.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.proc.is_some() {
            return Ok(());
        }
        if inner.crashes >= MAX_CRASHES {
            return Err(DriverError::new(
                ERR_DRIVER_FAILED,
                format!(
                    "Claude Code exited {} times in a row; check the account login and try again",
                    inner.crashes
                ),
            ));
        }
        let cursor = inner.tr.cursor.clone();
        let launch = Launch {
            model: inner.config.model.clone(),
            access: inner.config.access,
            interaction: inner.config.interaction,
            resume: (!cursor.session_id.is_empty()).then(|| cursor.session_id.clone()),
            resume_at: cursor.resume_at.clone(),
            fork: cursor.fork,
            session_id: None,
            mcp_config: inner
                .config
                .mcp_config
                .clone()
                .or_else(|| embedded_mcp_file(&session.thread_id, &self.shared.instance.id)),
            extra_args: self.shared.instance.args.clone(),
        };
        let spec = SpawnSpec {
            program,
            args: launch.argv(),
            env: protocol::child_env(
                self.shared.instance.config_dir.as_ref(),
                &self.shared.instance.env,
            ),
            scrub: protocol::scrub_env(),
            cwd: inner.config.cwd.clone().filter(|c| c.is_dir()),
        };
        inner.generation += 1;
        let generation = inner.generation;
        let (me, s1, s2) = (self.shared.clone(), session.clone(), session.clone());
        let me2 = self.shared.clone();
        let handle = supervisor::spawn(
            spec,
            move |line| me.on_line(&s1, generation, &line),
            move |exit| me2.on_exit(&s2, generation, exit),
        )
        .map_err(|e| {
            DriverError::new(ERR_DRIVER_UNAVAILABLE, format!("cannot start claude: {e}"))
        })?;
        inner.tr.reset_process();
        inner.tr.cwd = inner.config.cwd.clone();
        inner.inits_at_spawn = inner.tr.inits();
        inner.launched_full = inner.config.access == AccessMode::FullAccess;
        inner.mode =
            protocol::permission_mode(inner.config.access, inner.config.interaction).to_string();
        inner.hard_interrupt = false;
        inner.proc = Some(handle);
        Self::control(&mut inner, json!({"subtype": "initialize", "hooks": null}));
        Ok(())
    }

    fn relaunch_if_idle(inner: &mut Inner) -> Option<ProcHandle> {
        if inner.tr.has_live_turn() {
            return None;
        }
        inner.generation += 1;
        inner.proc.take()
    }
}

#[async_trait]
impl Driver for ClaudeDriver {
    fn kind(&self) -> &str {
        DRIVER
    }

    fn capabilities(&self) -> DriverCapabilities {
        capabilities()
    }

    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError> {
        let s = self.session_or_create(&input.thread_id, input.resume_cursor.as_ref());
        let old = {
            let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            let cwd_changed = inner.config.cwd.is_some() && inner.config.cwd != input.cwd;
            inner.config.cwd = input.cwd.clone();
            if input.model.is_some() {
                inner.config.model = input.model.clone();
            }
            inner.config.access = input.access_mode;
            inner.config.interaction = input.interaction_mode;
            inner.tr.cwd = input.cwd.clone();
            if cwd_changed {
                Self::relaunch_if_idle(&mut inner)
            } else {
                None
            }
        };
        if let Some(p) = old {
            p.shutdown().await;
        }
        Ok(())
    }

    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError> {
        let s = self.session_or_create(&input.thread_id, None);
        let old = {
            let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.config.access = input.access_mode;
            if input.model.is_some() {
                inner.config.model = input.model.clone();
            }
            let needs_flag = input.access_mode == AccessMode::FullAccess && !inner.launched_full;
            if inner.proc.is_some() && needs_flag {
                Self::relaunch_if_idle(&mut inner)
            } else {
                None
            }
        };
        if let Some(p) = old {
            p.shutdown().await;
        }
        let spawned_now = {
            let inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.proc.is_none()
        };
        if spawned_now {
            {
                let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.config.interaction = input.interaction_mode;
            }
            self.spawn(&s).await?;
        }
        let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.proc.is_none() {
            return Err(DriverError::new(
                ERR_DRIVER_NO_SESSION,
                "Claude is not running",
            ));
        }
        let want = protocol::permission_mode(input.access_mode, input.interaction_mode).to_string();
        if want != inner.mode {
            Self::control(
                &mut inner,
                json!({"subtype": "set_permission_mode", "mode": want}),
            );
            inner.mode = want;
        }
        inner.config.interaction = input.interaction_mode;
        if let Some(model) = input.model.as_ref().filter(|m| !m.trim().is_empty()) {
            if !spawned_now && inner.tr.model.as_deref() != Some(model.as_str()) {
                Self::control(&mut inner, json!({"subtype": "set_model", "model": model}));
            }
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let line = protocol::user_message(&uuid, &input.text, &input.attachments).to_string();
        let sent = inner
            .proc
            .as_ref()
            .map(|p| p.stdin.send(line).is_ok())
            .unwrap_or(false);
        if !sent {
            return Err(DriverError::new(
                ERR_DRIVER_NO_SESSION,
                "Claude's stdin is closed",
            ));
        }
        for ev in inner
            .tr
            .register_prompt(&input.turn_id, &uuid, input.model.as_deref())
        {
            self.emit(ev);
        }
        self.persist(&s, &mut inner);
        let cursor = &inner.tr.cursor;
        Ok(TurnStartResult {
            resume_cursor: (!cursor.session_id.is_empty()).then(|| cursor.to_value()),
        })
    }

    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError> {
        let Some(s) = self.session(thread_id) else {
            if let Some(t) = turn_id {
                self.emit(self.event(
                    thread_id,
                    Some(t),
                    RuntimeEventKind::TurnAborted(TurnAbortedPayload {
                        reason: "no live Claude session".into(),
                        usage: None,
                    }),
                ));
            }
            return Ok(());
        };
        let generation = {
            let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.proc.is_none() || !inner.tr.has_live_turn() {
                drop(inner);
                if let Some(t) = turn_id {
                    self.emit(self.event(
                        thread_id,
                        Some(t),
                        RuntimeEventKind::TurnAborted(TurnAbortedPayload {
                            reason: "no live turn".into(),
                            usage: None,
                        }),
                    ));
                }
                return Ok(());
            }
            inner.tr.mark_interrupt_requested();
            Self::control(
                &mut inner,
                json!({"subtype": "interrupt", "cancel_queued": true}),
            );
            inner.generation
        };
        let s2 = s.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INTERRUPT_WATCHDOG).await;
            let mut inner = s2.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.generation == generation && inner.tr.has_live_turn() {
                inner.hard_interrupt = true;
                if let Some(p) = inner.proc.as_mut() {
                    p.kill_now();
                }
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
            .session(thread_id)
            .ok_or_else(|| DriverError::new(ERR_DRIVER_NO_SESSION, "no Claude session"))?;
        let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.proc.is_none() {
            return Err(DriverError::new(
                ERR_DRIVER_NO_SESSION,
                "Claude is not running",
            ));
        }
        let (line, events) = inner
            .tr
            .answer_approval(request_id, decision)
            .map_err(|e| DriverError::new(ERR_DRIVER_NO_SESSION, e))?;
        if let Some(p) = &inner.proc {
            let _ = p.stdin.send(line.to_string());
        }
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
        let s = self
            .session(thread_id)
            .ok_or_else(|| DriverError::new(ERR_DRIVER_NO_SESSION, "no Claude session"))?;
        let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.proc.is_none() {
            return Err(DriverError::new(
                ERR_DRIVER_NO_SESSION,
                "Claude is not running",
            ));
        }
        let (line, events) = inner
            .tr
            .answer_questions(request_id, &answers)
            .map_err(|e| DriverError::new(ERR_DRIVER_NO_SESSION, e))?;
        if let Some(p) = &inner.proc {
            let _ = p.stdin.send(line.to_string());
        }
        for ev in events {
            self.emit(ev);
        }
        Ok(())
    }

    async fn rollback(&self, thread_id: &str, turn_count: u32) -> Result<(), DriverError> {
        let (cursor, old) = match self.session(thread_id) {
            Some(s) => {
                let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
                let mut c = inner.tr.cursor.clone();
                let changed = c
                    .truncate(turn_count as usize)
                    .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, e))?;
                if !changed {
                    return Ok(());
                }
                inner.tr.cursor = c.clone();
                inner.generation += 1;
                let old = inner.proc.take();
                for ev in inner
                    .tr
                    .abort_all(TurnEndState::Interrupted, "Thread rolled back")
                {
                    self.emit(ev);
                }
                self.persist(&s, &mut inner);
                (c, old)
            }
            None => {
                let mut c = self.stored_cursor(thread_id, None);
                let changed = c
                    .truncate(turn_count as usize)
                    .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, e))?;
                if !changed {
                    return Ok(());
                }
                self.shared.store.put(thread_id, &c);
                (c, None)
            }
        };
        if let Some(p) = old {
            p.shutdown().await;
        }
        if !cursor.session_id.is_empty() {
            self.emit(self.event(
                thread_id,
                None,
                RuntimeEventKind::SessionStarted(SessionStartedPayload {
                    message: Some(format!("rolled back to {turn_count} turn(s)")),
                    resume: Some(cursor.to_value()),
                }),
            ));
        }
        Ok(())
    }

    async fn fork(
        &self,
        source_thread_id: &str,
        target_thread_id: &str,
        turn_count: u32,
    ) -> Result<(), DriverError> {
        let mut c = match self.session(source_thread_id) {
            Some(s) => s
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tr
                .cursor
                .clone(),
            None => self.stored_cursor(source_thread_id, None),
        };
        c.resume_at = None;
        c.fork = false;
        let cut = c
            .truncate(turn_count as usize)
            .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, e))?;
        if !cut && !c.session_id.is_empty() {
            c.fork = true;
        }
        self.shared.store.put(target_thread_id, &c);
        if let Some(s) = self.session(target_thread_id) {
            let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.tr.cursor = c.clone();
            inner.persisted = c.clone();
        }
        if !c.session_id.is_empty() {
            self.emit(self.event(
                target_thread_id,
                None,
                RuntimeEventKind::SessionStarted(SessionStartedPayload {
                    message: Some(format!("forked from {source_thread_id}")),
                    resume: Some(c.to_value()),
                }),
            ));
        }
        Ok(())
    }

    async fn set_access_mode(&self, thread_id: &str, mode: AccessMode) -> Result<(), DriverError> {
        let Some(s) = self.session(thread_id) else {
            return Ok(());
        };
        let old = {
            let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.config.access = mode;
            if inner.proc.is_none() {
                None
            } else if mode == AccessMode::FullAccess && !inner.launched_full {
                Self::relaunch_if_idle(&mut inner)
            } else {
                let want = protocol::permission_mode(mode, inner.config.interaction).to_string();
                if want != inner.mode {
                    Self::control(
                        &mut inner,
                        json!({"subtype": "set_permission_mode", "mode": want}),
                    );
                    inner.mode = want;
                }
                None
            }
        };
        if let Some(p) = old {
            p.shutdown().await;
        }
        Ok(())
    }

    async fn stop(&self, thread_id: &str) -> Result<(), DriverError> {
        remove_embedded_mcp_file(thread_id);
        let Some(s) = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(thread_id)
        else {
            return Ok(());
        };
        let old = {
            let mut inner = s.inner.lock().unwrap_or_else(|e| e.into_inner());
            let (sends, events) = inner.tr.drain_requests("stopped");
            if let Some(p) = &inner.proc {
                for line in sends {
                    let _ = p.stdin.send(line.to_string());
                }
            }
            for ev in events {
                self.emit(ev);
            }
            for ev in inner
                .tr
                .abort_all(TurnEndState::Interrupted, "Session stopped.")
            {
                self.emit(ev);
            }
            self.persist(&s, &mut inner);
            inner.generation += 1;
            inner.waiters.clear();
            inner.proc.take()
        };
        if let Some(p) = old {
            p.shutdown().await;
        }
        self.emit(self.event(
            thread_id,
            None,
            RuntimeEventKind::SessionExited(SessionExitedPayload {
                reason: Some("Session stopped".into()),
                recoverable: Some(true),
                exit_kind: Some("graceful".into()),
            }),
        ));
        Ok(())
    }
}
