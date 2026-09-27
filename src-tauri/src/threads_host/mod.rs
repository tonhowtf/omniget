//! Host side of the threads engine (plan §3.1–3.3): the engine as a lazily
//! opened singleton, the reactors that turn "requested" events into driver
//! calls, the runtime ingestion (50 ms delta coalescing → persisted events),
//! the `threads://event` fan-out, the boot reconcile, the C-5 migration and
//! the `native` driver over the `LlmManager`.
//!
//! Nothing runs at rest: the first `threads_*` command opens the engine; the
//! tasks it spawns only wake on events.
//!
//! Round 2 glue (T2/T7/T8): [`ThreadGit`] (worktree per thread, checkpoint per
//! turn, diff, revert, git actions) runs next to the reactor and is awaited
//! before every `start_turn`; turns without a price get one from the price
//! table; `account.rate-limits.updated` feeds [`LimitsBook`].

pub mod cheap;
pub mod limits;
pub mod native;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use omniget_core::core::llm::cli_runtime::accounts::{AccountStore, CliKind};
use omniget_core::core::llm::drivers::coalesce::DeltaCoalescer;
use omniget_core::core::llm::drivers::*;
use omniget_core::core::omni::bus::BusEvent;
use omniget_core::core::threads::git::{GitHooks, ThreadGit};
use omniget_core::core::threads::model::{Command, CommandEnvelope, DomainEvent, StoredEvent};
use omniget_core::core::threads::usage::{self, LimitsBook};
use omniget_core::core::threads::{store, ThreadsEngine};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

pub const EVENT_THREADS: &str = "threads://event";
/// Request ids of the owner asks the embedded MCP raises in a thread
/// (`catalog_install`); their answers go to the broker, not to the driver.
pub const MCP_ASK_PREFIX: &str = "omniget-mcp-ask-";
pub const ERR_THREADS_HOST: &str = "ERR_THREADS_HOST";

static HOST: OnceLock<Arc<ThreadsHost>> = OnceLock::new();

/// One instance as the UI lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    pub instance: DriverInstance,
    pub driver_label: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub capabilities: DriverCapabilities,
}

pub struct ThreadsHost {
    pub engine: Arc<ThreadsEngine>,
    /// Worktree, checkpoints, diff, revert and git actions of the threads.
    pub git: Arc<ThreadGit>,
    /// Live rate limits per instance (`account.rate-limits.updated`).
    pub limits: LimitsBook,
    app: AppHandle,
    sink: RuntimeSink,
    drivers: Mutex<HashMap<String, Arc<dyn Driver>>>,
    /// Threads whose driver session was started in this process, with the
    /// cwd it was started in (a re-created worktree restarts the session).
    sessions: Mutex<HashMap<String, Option<PathBuf>>>,
}

/// The CLI drivers of round 2 (each `register()` is idempotent and only
/// stores a factory: no process starts until a thread opens a session).
/// Called once, from [`get`].
pub fn register_all_drivers(_app: &AppHandle) {
    // `native` is registered in `get` (it needs the LlmManager).
    //
    // c1-claude (`claude -p --input-format stream-json`, one process per
    // thread; handoff c1-claude.json). Idempotent.
    omniget_core::core::llm::drivers::claude::register();
    // c2-codex (`codex app-server` JSON-RPC; handoff c2-codex.json).
    // Instances come from accounts.json. Idempotent.
    omniget_core::core::llm::drivers::codex::register();
    // c3-acp (handoff c3-acp.json): generic ACP v0.11 (gemini, qwen,
    // cursor, grok, kimi, droid, copilot, goose, junie, auggie, vibe, kiro,
    // cline, kilo) and `opencode serve` (HTTP + SSE). Their machine
    // instances are listed in `ThreadsHost::instances`.
    omniget_core::core::llm::drivers::acp::register();
    omniget_core::core::llm::drivers::opencode::register();
}

/// The embedded OmniGet MCP for every CLI driver session (plan §3.2): ACP
/// and OpenCode read it through `acp::mcp_servers_for`, Claude writes it to
/// its `--mcp-config`, Codex to `-c mcp_servers.omniget.*`. Each call mints
/// a session token for that thread; stopping the session revokes it.
fn install_mcp_provider(app: &AppHandle) {
    let app = app.clone();
    omniget_core::core::llm::drivers::acp::set_mcp_provider(Some(Arc::new(
        move |thread_id: &str, instance_id: &str| {
            crate::mcp::driver_servers(&app, thread_id, instance_id)
        },
    )));
}

/// The owner ask of an MCP tool as an approval card in the thread.
#[allow(clippy::too_many_arguments)]
pub fn mcp_ask_opened(
    app: &AppHandle,
    thread_id: &str,
    request_id: &str,
    tool: &str,
    preview: &str,
    diff: &str,
    paths: Vec<String>,
    commands: Vec<String>,
) {
    let Some(host) = HOST.get().cloned().or_else(|| get(app).ok()) else {
        return;
    };
    let turn = host
        .engine
        .read(|c| store::thread_row(c, thread_id))
        .ok()
        .flatten()
        .and_then(|r| r.active_turn_id);
    let _ = host.sink.send(
        RuntimeEvent::new(
            "host",
            "host",
            thread_id,
            turn.as_deref(),
            RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                request_type: RequestType::DynamicToolCall,
                detail: Some(RequestDetail {
                    tool_name: Some(tool.to_string()),
                    command: (!commands.is_empty()).then(|| commands.join("\n")),
                    reason: Some(format!("OmniGet MCP: the agent wants to run `{tool}`")),
                    diff: (!diff.is_empty()).then(|| diff.to_string()),
                    paths,
                    preview: Some(preview.to_string()),
                    ..Default::default()
                }),
                app_name: Some("OmniGet".into()),
                options: vec![
                    ApprovalOption {
                        id: "accept".into(),
                        label: "Allow".into(),
                        decision: Some(ApprovalDecision::Accept),
                    },
                    ApprovalOption {
                        id: "decline".into(),
                        label: "Deny".into(),
                        decision: Some(ApprovalDecision::Decline),
                    },
                ],
                args: Some(json!({ "tool": tool, "source": "omniget-mcp" })),
            }),
        )
        .with_request(request_id),
    );
}

/// Closes the card, whoever answered (thread, pet, chat) or on timeout.
pub fn mcp_ask_resolved(_app: &AppHandle, thread_id: &str, request_id: &str, allowed: bool) {
    let Some(host) = HOST.get() else { return };
    let _ = host.sink.send(
        RuntimeEvent::new(
            "host",
            "host",
            thread_id,
            None,
            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                request_type: RequestType::DynamicToolCall,
                decision: Some(if allowed {
                    ApprovalDecision::Accept
                } else {
                    ApprovalDecision::Decline
                }),
                resolution: None,
            }),
        )
        .with_request(request_id),
    );
}

/// The one host. The first caller opens `<app_data>/llm/threads.db`,
/// registers the `native` driver, starts the tasks, reconciles turns a crash
/// left running and imports the old conversations.
pub fn get(app: &AppHandle) -> Result<Arc<ThreadsHost>, String> {
    if let Some(h) = HOST.get() {
        return Ok(h.clone());
    }
    let path = omniget_core::core::threads::default_db_path()
        .ok_or_else(|| format!("{ERR_THREADS_HOST}: no app data dir"))?;
    let engine = ThreadsEngine::open(&path)?;
    let llm = app.state::<crate::AppState>().llm.clone();
    {
        let llm = llm.clone();
        let app = app.clone();
        register_driver(DriverRegistration {
            kind: native::NATIVE.into(),
            label: "OmniGet".into(),
            capabilities: native::capabilities(),
            factory: Arc::new(move |instance, sink| {
                Ok(Arc::new(native::NativeDriver::new(
                    instance,
                    sink,
                    llm.clone(),
                    app.clone(),
                )) as Arc<dyn Driver>)
            }),
        });
    }
    register_all_drivers(app);
    install_mcp_provider(app);
    let (sink, rx) = mpsc::unbounded_channel::<RuntimeEvent>();
    let hooks = GitHooks {
        text: Some(cheap::text_gen(llm.clone())),
        on_setup: {
            let app = app.clone();
            Some(Arc::new(
                move |snap: &omniget_core::core::vcs::worktree::SetupSnapshot| {
                    let _ = app.emit(crate::commands::central::vcs::WORKTREE_SETUP_EVENT, snap);
                },
            ))
        },
    };
    let host = Arc::new(ThreadsHost {
        engine: engine.clone(),
        git: ThreadGit::new(engine.clone(), hooks),
        limits: LimitsBook::default(),
        app: app.clone(),
        sink,
        drivers: Mutex::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
    });
    if HOST.set(host.clone()).is_err() {
        // Lost a race with another first caller: use theirs.
        return HOST
            .get()
            .cloned()
            .ok_or_else(|| format!("{ERR_THREADS_HOST}: init race"));
    }
    // Subscribed before anything can dispatch.
    spawn_fanout(app.clone(), engine.subscribe());
    spawn_reactor(host.clone(), engine.subscribe());
    tauri::async_runtime::spawn(host.git.clone().run(engine.subscribe()));
    spawn_ingestion(host.clone(), rx);
    spawn_boot(host.clone(), llm);
    Ok(host)
}

/// App exit: stops every driver session this process started (the CLI
/// supervisors kill their process trees on stop) and revokes their MCP
/// tokens. Does nothing when no thread command ever opened the host.
pub async fn shutdown() {
    let Some(host) = HOST.get().cloned() else {
        return;
    };
    let threads: Vec<String> = host
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect();
    for thread_id in threads {
        if let Ok((d, _)) = host.thread_driver(&thread_id) {
            let _ = d.stop(&thread_id).await;
        }
        host.forget_session(&thread_id);
        crate::mcp::revoke_thread(&thread_id);
    }
}

// ── Fan-out ─────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct Envelope<'a> {
    sequence: i64,
    event: &'a StoredEvent,
}

fn spawn_fanout(app: AppHandle, mut rx: tokio::sync::broadcast::Receiver<StoredEvent>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let _ = app.emit(
                        EVENT_THREADS,
                        Envelope {
                            sequence: ev.sequence,
                            event: &ev,
                        },
                    );
                }
                // The front notices the gap in `sequence` and replays.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("[threads] fan-out lagged by {n}");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

// ── Ingestion ───────────────────────────────────────────────────────────

fn spawn_ingestion(host: Arc<ThreadsHost>, mut rx: mpsc::UnboundedReceiver<RuntimeEvent>) {
    tauri::async_runtime::spawn(async move {
        let mut coalescer = DeltaCoalescer::new();
        loop {
            let deadline = coalescer.deadline();
            let next = match deadline {
                Some(d) => {
                    let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(d));
                    tokio::select! {
                        ev = rx.recv() => ev.map(Some),
                        _ = sleep => Some(None),
                    }
                }
                None => rx.recv().await.map(Some),
            };
            let out = match next {
                None => {
                    // Every sender is gone: flush and stop.
                    let rest = coalescer.take();
                    for ev in rest {
                        host.persist(ev).await;
                    }
                    break;
                }
                Some(None) => coalescer.take(),
                Some(Some(ev)) => coalescer.push(ev, std::time::Instant::now()),
            };
            for ev in out {
                host.persist(ev).await;
            }
        }
    });
}

impl ThreadsHost {
    /// One runtime event → one idempotent command (`provider:<eventId>`).
    async fn persist(&self, ev: RuntimeEvent) {
        self.mirror_to_bus(&ev);
        self.limits.note(&ev);
        let env = CommandEnvelope {
            command_id: Some(format!("provider:{}", ev.event_id)),
            command: Command::RuntimeAppend { event: ev },
        };
        if let Err(e) = self.engine.dispatch(env).await {
            tracing::debug!("[threads] runtime event dropped: {e}");
        }
    }

    /// CLI drivers do not go through the broker, so the mascot, the
    /// observatory and the world hear about their work from here. The native
    /// driver already emits on the bus itself.
    fn mirror_to_bus(&self, ev: &RuntimeEvent) {
        if ev.driver == native::NATIVE {
            return;
        }
        let bus = self.app.state::<crate::AppState>().llm.bus();
        match &ev.kind {
            RuntimeEventKind::TurnStarted(_) => bus.emit(BusEvent::TurnStarted {
                agent: ev.instance_id.clone(),
                conversation: ev.thread_id.clone(),
            }),
            RuntimeEventKind::ItemCompleted(item)
                if !matches!(
                    item.item_type,
                    ItemType::AssistantMessage | ItemType::UserMessage | ItemType::Reasoning
                ) =>
            {
                let tool = item
                    .tool_name
                    .clone()
                    .or_else(|| item.title.clone())
                    .unwrap_or_default();
                let ok = item.status != Some(ItemStatus::Failed);
                bus.emit(BusEvent::ToolCalled {
                    agent: ev.instance_id.clone(),
                    tool: tool.clone(),
                    ok,
                    ms: 0,
                });
                bus.emit(BusEvent::ToolFinished { tool, ok });
            }
            RuntimeEventKind::TurnCompleted(p) => bus.emit(BusEvent::TurnEnded {
                agent: ev.instance_id.clone(),
                usage: omniget_core::core::llm::types::Usage {
                    input_tokens: p.usage.as_ref().map(|u| u.input_tokens as u32).unwrap_or(0),
                    output_tokens: p
                        .usage
                        .as_ref()
                        .map(|u| u.output_tokens as u32)
                        .unwrap_or(0),
                    cost_usd: p.total_cost_usd,
                    ..Default::default()
                },
            }),
            _ => {}
        }
    }
}

// ── Drivers and instances ───────────────────────────────────────────────

impl ThreadsHost {
    /// Native + every CLI account of `accounts.json` as an instance of its
    /// driver. A driver nobody registered yet is listed as unavailable.
    pub fn instances(&self) -> Vec<InstanceView> {
        let mut out = Vec::new();
        let native = DriverInstance {
            color: Some("#6e56cf".into()),
            ..DriverInstance::new(native::NATIVE, native::NATIVE, "OmniGet")
        };
        out.push(view(native));
        if let Some(store) = AccountStore::default_store() {
            for acct in store.list().iter() {
                let mut inst = DriverInstance::new(
                    &format!("{}-{}", acct.cli.as_str(), acct.id),
                    acct.cli.as_str(),
                    &acct.label,
                );
                inst.account_id = Some(acct.id.clone());
                inst.config_dir = Some(acct.config_dir.clone());
                inst.enabled = !acct.disabled;
                inst.color = Some(
                    match acct.cli {
                        CliKind::Claude => "#d97757",
                        CliKind::Codex => "#10a37f",
                    }
                    .into(),
                );
                out.push(view(inst));
            }
        }
        // One instance per ACP agent this machine can start (`acp-<agent>`),
        // and `opencode` when its binary exists.
        for inst in omniget_core::core::llm::drivers::acp::instances() {
            out.push(view(inst));
        }
        for inst in omniget_core::core::llm::drivers::opencode::instances() {
            if !out.iter().any(|v| v.instance.id == inst.id) {
                out.push(view(inst));
            }
        }
        // Default instances of registered drivers without an account (an
        // `acp` without a command fails in its factory with a clear reason).
        for kind in registered_drivers() {
            if !out.iter().any(|v| v.instance.driver == kind) {
                let label = driver_registration(&kind)
                    .map(|r| r.label)
                    .unwrap_or_else(|| kind.clone());
                out.push(view(DriverInstance::new(&kind, &kind, &label)));
            }
        }
        out
    }

    fn instance(&self, instance_id: &str, driver: &str) -> DriverInstance {
        self.instances()
            .into_iter()
            .map(|v| v.instance)
            .find(|i| i.id == instance_id)
            .unwrap_or_else(|| DriverInstance::new(instance_id, driver, driver))
    }

    fn driver_for(&self, instance_id: &str, driver: &str) -> Result<Arc<dyn Driver>, DriverError> {
        if let Some(d) = self
            .drivers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(instance_id)
            .cloned()
        {
            return Ok(d);
        }
        let instance = self.instance(instance_id, driver);
        if !instance.enabled {
            return Err(DriverError::new(
                ERR_DRIVER_UNAVAILABLE,
                format!("instance {instance_id} is disabled"),
            ));
        }
        let built = build_driver(instance, self.sink.clone())?;
        self.drivers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(instance_id.to_string(), built.clone());
        Ok(built)
    }

    /// The driver of a thread as the projection has it now.
    fn thread_driver(
        &self,
        thread_id: &str,
    ) -> Result<(Arc<dyn Driver>, store::ThreadRow), DriverError> {
        let row = self
            .engine
            .read(|c| store::thread_row(c, thread_id))
            .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, e))?
            .ok_or_else(|| {
                DriverError::new(ERR_DRIVER_NO_SESSION, format!("no thread {thread_id}"))
            })?;
        let d = self.driver_for(&row.instance_id, &row.driver)?;
        Ok((d, row))
    }

    async fn ensure_session(
        &self,
        driver: &Arc<dyn Driver>,
        row: &store::ThreadRow,
        cwd: Option<PathBuf>,
    ) -> Result<(), DriverError> {
        let known = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&row.thread_id)
            .cloned();
        match known {
            Some(started_in) if started_in == cwd => return Ok(()),
            // The folder moved (worktree re-created): a new session there.
            Some(_) => {
                let _ = driver.stop(&row.thread_id).await;
                self.forget_session(&row.thread_id);
                crate::mcp::revoke_thread(&row.thread_id);
            }
            None => {}
        }
        let resume = self
            .engine
            .read(|c| store::provider_session(c, &row.thread_id))
            .ok()
            .flatten()
            .and_then(|s| s.resume_cursor);
        driver
            .start_session(SessionStart {
                thread_id: row.thread_id.clone(),
                instance_id: row.instance_id.clone(),
                model: row.model.clone(),
                agent_id: row.agent_id.clone(),
                access_mode: AccessMode::parse(&row.runtime_mode),
                interaction_mode: if row.interaction_mode == "plan" {
                    InteractionMode::Plan
                } else {
                    InteractionMode::Default
                },
                resume_cursor: resume,
                cwd: cwd.clone(),
            })
            .await?;
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(row.thread_id.clone(), cwd);
        Ok(())
    }

    fn forget_session(&self, thread_id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(thread_id);
    }

    /// A driver call failed: say so in the thread and close what would
    /// otherwise hang (the turn, the request).
    fn fail(&self, thread_id: &str, turn_id: Option<&str>, err: &DriverError, close_turn: bool) {
        let class = if err.code == ERR_DRIVER_UNAVAILABLE {
            ErrorClass::ValidationError
        } else {
            ErrorClass::ProviderError
        };
        let _ = self.sink.send(RuntimeEvent::new(
            "host",
            "host",
            thread_id,
            turn_id,
            RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                message: err.message.clone(),
                class,
                code: Some(err.code.clone()),
                detail: None,
            }),
        ));
        if close_turn && turn_id.is_some() {
            let _ = self.sink.send(RuntimeEvent::new(
                "host",
                "host",
                thread_id,
                turn_id,
                RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                    state: TurnEndState::Failed,
                    stop_reason: None,
                    usage: None,
                    total_cost_usd: None,
                    error_message: Some(err.to_string()),
                }),
            ));
        }
    }
}

fn view(instance: DriverInstance) -> InstanceView {
    let reg = driver_registration(&instance.driver);
    let (available, reason) = match (&reg, instance.enabled) {
        (None, _) => (
            false,
            Some(format!("driver `{}` is not available yet", instance.driver)),
        ),
        (Some(_), false) => (false, Some("disabled".to_string())),
        (Some(_), true) => (true, None),
    };
    InstanceView {
        driver_label: reg
            .as_ref()
            .map(|r| r.label.clone())
            .unwrap_or_else(|| instance.driver.clone()),
        capabilities: reg.map(|r| r.capabilities).unwrap_or_default(),
        available,
        unavailable_reason: reason,
        instance: instance.redacted(),
    }
}

// ── Reactors ────────────────────────────────────────────────────────────

fn spawn_reactor(host: Arc<ThreadsHost>, mut rx: tokio::sync::broadcast::Receiver<StoredEvent>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let host = host.clone();
                    // Each reaction on its own task: a slow `start_turn`
                    // never holds an approval answer back.
                    tauri::async_runtime::spawn(async move { host.react(ev.event).await });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[threads] reactor lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

impl ThreadsHost {
    async fn react(&self, event: DomainEvent) {
        use DomainEvent as E;
        match event {
            E::TurnStartRequested {
                thread_id,
                turn_id,
                message_id,
                ordinal,
                text,
                attachments,
                model,
                instance_id,
                driver,
                runtime_mode,
                interaction_mode,
                ..
            } => {
                let d = match self.driver_for(&instance_id, &driver) {
                    Ok(d) => d,
                    Err(e) => return self.fail(&thread_id, Some(&turn_id), &e, true),
                };
                // Worktree ready and checkpoint N-1 taken before the driver
                // touches a file. A worktree that cannot be made fails the
                // turn: it must not run in the user's own checkout instead.
                let cwd = match self.git.before_turn(&thread_id, ordinal).await {
                    Ok(c) => c,
                    Err(e) => {
                        let err = DriverError::new(ERR_DRIVER_FAILED, format!("worktree: {e}"));
                        return self.fail(&thread_id, Some(&turn_id), &err, true);
                    }
                };
                let row = match self.engine.read(|c| store::thread_row(c, &thread_id)) {
                    Ok(Some(r)) => r,
                    _ => return,
                };
                if let Err(e) = self.ensure_session(&d, &row, cwd).await {
                    return self.fail(&thread_id, Some(&turn_id), &e, true);
                }
                let input = TurnStart {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    message_id,
                    text,
                    attachments,
                    model,
                    access_mode: runtime_mode,
                    interaction_mode,
                };
                if let Err(e) = d.start_turn(input).await {
                    self.fail(&thread_id, Some(&turn_id), &e, true);
                }
            }
            E::TurnInterruptRequested { thread_id, turn_id } => {
                if let Ok((d, _)) = self.thread_driver(&thread_id) {
                    if let Err(e) = d.interrupt(&thread_id, turn_id.as_deref()).await {
                        self.fail(&thread_id, turn_id.as_deref(), &e, true);
                    }
                }
            }
            E::ApprovalResponseRequested {
                thread_id: _,
                request_id,
                decision,
            } if request_id.starts_with(MCP_ASK_PREFIX) => {
                use omniget_core::core::llm::broker::Answer;
                let answer = match decision {
                    ApprovalDecision::AcceptAlways => Answer::Always,
                    d if d.allows() => Answer::Once,
                    _ => Answer::Deny,
                };
                let broker = self.app.state::<crate::AppState>().llm.broker();
                broker.answer_with(&request_id, answer);
            }
            E::ApprovalResponseRequested {
                thread_id,
                request_id,
                decision,
            } => {
                let result = match self.thread_driver(&thread_id) {
                    Ok((d, _)) => d.respond_request(&thread_id, &request_id, decision).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    // The harness that asked is gone (restart, crash): close
                    // the request instead of leaving it answered-but-open.
                    let _ = self.sink.send(
                        RuntimeEvent::new(
                            "host",
                            "host",
                            &thread_id,
                            None,
                            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                                request_type: RequestType::Unknown,
                                decision: Some(decision),
                                resolution: Some(format!("stale: {e}")),
                            }),
                        )
                        .with_request(request_id),
                    );
                }
            }
            E::UserInputResponseRequested {
                thread_id,
                request_id,
                answers,
            } => {
                let result = match self.thread_driver(&thread_id) {
                    Ok((d, _)) => {
                        d.respond_user_input(&thread_id, &request_id, answers.clone())
                            .await
                    }
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    let _ = self.sink.send(
                        RuntimeEvent::new(
                            "host",
                            "host",
                            &thread_id,
                            None,
                            RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                                answers: json!({ "error": e.to_string() }),
                            }),
                        )
                        .with_request(request_id),
                    );
                }
            }
            E::Reverted {
                thread_id,
                turn_count,
            } => {
                if let Ok((d, row)) = self.thread_driver(&thread_id) {
                    // Right after a restart the driver has no session for
                    // the thread yet; open it (with its resume cursor) first.
                    let cwd = self.git.cwd_of_row(&row);
                    let _ = self.ensure_session(&d, &row, cwd).await;
                    if let Err(e) = d.rollback(&thread_id, turn_count).await {
                        self.fail(&thread_id, None, &e, false);
                    }
                }
            }
            E::ThreadCreated {
                thread_id,
                forked_from: Some(from),
                instance_id,
                driver,
                ..
            } => match self.driver_for(&instance_id, &driver) {
                Ok(d) => {
                    // The source's session must exist in the driver to fork.
                    if let Ok(Some(src)) =
                        self.engine.read(|c| store::thread_row(c, &from.thread_id))
                    {
                        if src.instance_id == instance_id {
                            let cwd = self.git.cwd_of_row(&src);
                            let _ = self.ensure_session(&d, &src, cwd).await;
                        }
                    }
                    if let Err(e) = d.fork(&from.thread_id, &thread_id, from.turn_count).await {
                        if e.code != ERR_DRIVER_UNSUPPORTED {
                            self.fail(&thread_id, None, &e, false);
                        }
                    }
                }
                Err(e) => self.fail(&thread_id, None, &e, false),
            },
            E::RuntimeModeSet {
                thread_id,
                runtime_mode,
            } => {
                if let Ok((d, _)) = self.thread_driver(&thread_id) {
                    let _ = d.set_access_mode(&thread_id, runtime_mode).await;
                }
            }
            E::InstanceSet { thread_id, .. } => {
                // The old harness session belongs to the old instance; the
                // next turn opens a session on the new one.
                let old: Vec<Arc<dyn Driver>> = self
                    .drivers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .values()
                    .cloned()
                    .collect();
                if self
                    .sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&thread_id)
                {
                    for d in old {
                        let _ = d.stop(&thread_id).await;
                    }
                }
                self.forget_session(&thread_id);
                crate::mcp::revoke_thread(&thread_id);
            }
            E::SessionStopRequested { thread_id } | E::ThreadDeleted { thread_id, .. } => {
                if let Ok((d, _)) = self.thread_driver(&thread_id) {
                    let _ = d.stop(&thread_id).await;
                }
                self.forget_session(&thread_id);
                crate::mcp::revoke_thread(&thread_id);
            }
            // A driver that reports tokens but no money: price the turn.
            E::TurnUsage {
                thread_id,
                turn_id,
                usage,
            } if usage.cost_usd.is_none() => {
                let fallback = self
                    .engine
                    .read(|c| store::turn_row(c, &turn_id))
                    .ok()
                    .flatten()
                    .and_then(|t| t.model)
                    .or_else(|| {
                        self.engine
                            .read(|c| store::thread_row(c, &thread_id))
                            .ok()
                            .flatten()
                            .and_then(|r| r.model)
                    });
                if let Some(cost) = usage::price_usage(&usage, fallback.as_deref()).await {
                    let mut usage = usage;
                    usage.cost_usd = Some(cost);
                    if usage.model.is_none() {
                        usage.model = fallback;
                    }
                    let _ = self
                        .git
                        .record(DomainEvent::TurnUsage {
                            thread_id,
                            turn_id,
                            usage,
                        })
                        .await;
                }
            }
            _ => {}
        }
    }
}

// ── Boot: reconcile + C-5 ───────────────────────────────────────────────

fn spawn_boot(host: Arc<ThreadsHost>, llm: Arc<crate::llm_manager::LlmManager>) {
    tauri::async_runtime::spawn(async move {
        // Turns a crash left pending/running: nothing drives them any more.
        let dangling = host.engine.read(store::dangling_turns).unwrap_or_default();
        for (thread_id, turn_id) in dangling {
            let mut ev = RuntimeEvent::new(
                "host",
                "host",
                &thread_id,
                Some(&turn_id),
                RuntimeEventKind::TurnAborted(TurnAbortedPayload {
                    reason: "the app closed during this turn".into(),
                    usage: None,
                }),
            );
            ev.event_id = format!("reconcile-{turn_id}");
            let env = CommandEnvelope {
                command_id: Some(format!("server:reconcile:{turn_id}")),
                command: Command::RuntimeAppend { event: ev },
            };
            if let Err(e) = host.engine.dispatch(env).await {
                tracing::debug!("[threads] reconcile {turn_id}: {e}");
            }
        }
        // C-5: the old chat conversations, once (idempotent per thread).
        let dir = llm.coordinator().dir().map(|d| d.to_path_buf());
        if let Some(dir) = dir {
            let engine = host.engine.clone();
            let report = tokio::task::spawn_blocking(move || {
                omniget_core::core::threads::migrate::migrate_conversations(&engine, &dir, |id| {
                    omniget_core::core::llm::code_tools::workspace_of(id)
                })
            })
            .await;
            match report {
                Ok(r) if r.imported > 0 || !r.failed.is_empty() => {
                    tracing::info!(
                        "[threads] C-5 migration: {} imported, {} skipped, {} failed",
                        r.imported,
                        r.skipped,
                        r.failed.len()
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("[threads] C-5 migration task: {e}"),
            }
        }
    });
}
