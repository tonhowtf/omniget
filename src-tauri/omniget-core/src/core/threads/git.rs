//! Threads × git (plan §3.4, T2/T7): the worktree of a thread, a checkpoint
//! per turn, the turn diff, "edit from here", and the git actions of the
//! split button, all recorded as thread events.
//!
//! [`ThreadGit`] is a reactor over the engine's events plus a few calls the
//! host makes directly:
//! - `thread.created` with a worktree → `vcs::worktree::create` in stages;
//!   each stage change becomes `thread.worktree-updated{state: preparing}`,
//!   then `ready` (cwd of the thread) or `failed`.
//! - [`ThreadGit::before_turn`] (the host awaits it before `start_turn`):
//!   worktree ready (created or re-created after an archive), checkpoint
//!   `N-1` present (turn 0 = baseline).
//! - `thread.turn-completed` → checkpoint `N` + numstat of the turn →
//!   `thread.checkpoint-captured`.
//! - `thread.turn-start-requested` of turn 1 → the temporary branch
//!   `omniget/<hex>` is renamed after the prompt (cheap model when the host
//!   gives one, else a heuristic).
//! - `thread.revert-requested` → files back to checkpoint `N` (optional) and
//!   then `thread.revert` (the conversation cut; the driver rolls back).
//! - `thread.archived` → the worktree folder goes (its content stays in the
//!   checkpoint refs); `thread.deleted` → worktree, temp branch and refs go.
//!
//! One async lock per thread serializes all of it, so a checkpoint is never
//! taken while the worktree is still being made. Nothing polls.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use super::engine::ThreadsEngine;
use super::model::{Activity, CheckpointFile, Command, CommandEnvelope, DomainEvent, StoredEvent};
use super::store::{self, ThreadRow};
use crate::core::llm::drivers::now_iso;
use crate::core::vcs::actions::{
    self, CommitResult, PrCreateResult, PrInfo, PrRequest, PushResult,
};
use crate::core::vcs::checkpoint;
use crate::core::vcs::diff::{self, DiffOptions, DiffResult};
use crate::core::vcs::repo::{self, RepoStatus};
use crate::core::vcs::worktree::{self, SetupSnapshot, WorktreeRequest};

pub const ERR_THREADS_GIT: &str = "ERR_THREADS_GIT";

pub type TextFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;
/// `(system, prompt)` → the answer of a cheap model, or `None` (no model,
/// error, timeout). The host plugs the `LlmManager` in here.
pub type TextGen = Arc<dyn Fn(String, String) -> TextFuture + Send + Sync>;
/// Every raw setup snapshot (percent included), for a live progress bar.
pub type SetupSink = Arc<dyn Fn(&SetupSnapshot) + Send + Sync>;

#[derive(Clone, Default)]
pub struct GitHooks {
    pub text: Option<TextGen>,
    pub on_setup: Option<SetupSink>,
}

/// What "edit from here" gives back to the composer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertOutcome {
    pub thread_id: String,
    pub turn_count: u32,
    /// Files put back (empty when only the conversation was cut).
    pub restored_files: Vec<String>,
    /// The prompt of the first dropped turn, for the composer.
    pub prompt: Option<String>,
    pub attachments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitMessage {
    pub subject: String,
    pub body: String,
    /// `model|heuristic`.
    pub source: String,
    pub files: usize,
}

impl CommitMessage {
    pub fn text(&self) -> String {
        if self.body.trim().is_empty() {
            self.subject.clone()
        } else {
            format!("{}\n\n{}", self.subject, self.body.trim())
        }
    }
}

pub struct ThreadGit {
    engine: Arc<ThreadsEngine>,
    hooks: GitHooks,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Setups running now, by thread (the UI's cancel button).
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn gerr(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    if s.starts_with("ERR_") {
        s
    } else {
        format!("{ERR_THREADS_GIT}: {s}")
    }
}

impl ThreadGit {
    pub fn new(engine: Arc<ThreadsEngine>, hooks: GitHooks) -> Arc<Self> {
        Arc::new(Self {
            engine,
            hooks,
            locks: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
        })
    }

    /// The reactor loop. Spawn it once with a receiver taken before anything
    /// can dispatch.
    pub async fn run(self: Arc<Self>, mut rx: broadcast::Receiver<StoredEvent>) {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let this = self.clone();
                    tokio::spawn(async move { this.react(ev).await });
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[threads/git] lagged by {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    async fn react(&self, ev: StoredEvent) {
        use DomainEvent as E;
        match ev.event {
            E::ThreadCreated {
                thread_id,
                worktree: Some(_),
                ..
            } => {
                let _g = self.lock(&thread_id).await;
                if let Ok(row) = self.row(&thread_id) {
                    if let Err(e) = self.ensure_worktree(&row).await {
                        tracing::warn!("[threads/git] worktree of {thread_id}: {e}");
                    }
                }
            }
            E::TurnStartRequested {
                thread_id,
                ordinal: 1,
                text,
                ..
            } => self.rename_after_first(&thread_id, &text).await,
            E::TurnCompleted {
                thread_id, turn_id, ..
            } => {
                if let Err(e) = self.after_turn(&thread_id, &turn_id).await {
                    tracing::debug!("[threads/git] checkpoint of {turn_id}: {e}");
                }
            }
            E::RevertRequested {
                thread_id,
                turn_count,
                restore_files,
                ..
            } => {
                self.revert(&thread_id, turn_count, restore_files, &ev.event_id)
                    .await
            }
            E::ThreadArchived { thread_id, .. } => self.cleanup(&thread_id, false).await,
            E::ThreadDeleted { thread_id, .. } => self.cleanup(&thread_id, true).await,
            _ => {}
        }
    }

    async fn lock(&self, thread_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let m = self
            .locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(thread_id.to_string())
            .or_default()
            .clone();
        m.lock_owned().await
    }

    fn row(&self, thread_id: &str) -> Result<ThreadRow, String> {
        self.engine
            .read(|c| store::thread_row(c, thread_id))?
            .ok_or_else(|| format!("ERR_THREADS_NOT_FOUND: no thread {thread_id}"))
    }

    fn project_root(&self, row: &ThreadRow) -> Option<PathBuf> {
        self.engine
            .read(|c| store::project_row(c, &row.project_id))
            .ok()
            .flatten()
            .map(|p| p.workspace_root)
            .filter(|r| !r.trim().is_empty())
            .map(PathBuf::from)
    }

    /// Where the thread works: its ready worktree, a worktree path given at
    /// creation, or the project folder.
    pub fn cwd_of_row(&self, row: &ThreadRow) -> Option<PathBuf> {
        let wt = row
            .worktree_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        match row.worktree_state.as_deref() {
            Some("ready") | None => {
                if let Some(p) = wt.filter(|p| p.is_dir()) {
                    return Some(p);
                }
            }
            _ => {}
        }
        self.project_root(row)
    }

    pub fn cwd(&self, thread_id: &str) -> Result<PathBuf, String> {
        let row = self.row(thread_id)?;
        self.cwd_of_row(&row)
            .ok_or_else(|| format!("ERR_THREADS_INVALID: thread {thread_id} has no folder"))
    }

    /// Asks the running worktree setup of a thread to stop (it removes what
    /// it made). `false` when none is running.
    pub fn cancel_setup(&self, thread_id: &str) -> bool {
        match self
            .cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(thread_id)
        {
            Some(f) => {
                f.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Records a host-observed fact on its thread (`thread.host.record`).
    pub async fn record(&self, event: DomainEvent) -> Result<(), String> {
        let env = CommandEnvelope {
            command_id: Some(format!("server:{}", uuid::Uuid::new_v4().simple())),
            command: Command::HostRecord { event },
        };
        self.engine.dispatch(env).await.map(|_| ())
    }

    async fn note(&self, thread_id: &str, kind: &str, tone: &str, summary: String, payload: Value) {
        let _ = self
            .record(DomainEvent::ActivityAppended {
                thread_id: thread_id.to_string(),
                activity: Activity {
                    activity_id: format!("{thread_id}:{kind}:{}", uuid::Uuid::new_v4().simple()),
                    turn_id: None,
                    kind: kind.to_string(),
                    tone: tone.to_string(),
                    summary,
                    payload,
                    created_at: now_iso(),
                },
            })
            .await;
    }

    // ── Worktree ────────────────────────────────────────────────────────

    /// With the thread lock held. `Ok(None)`: the thread has no worktree.
    async fn ensure_worktree(&self, row: &ThreadRow) -> Result<Option<PathBuf>, String> {
        let state = match row.worktree_state.as_deref() {
            None => return Ok(None),
            Some(s) => s,
        };
        if state == "ready" {
            if let Some(p) = row
                .worktree_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
            {
                return Ok(Some(p));
            }
        }
        let root = self
            .project_root(row)
            .ok_or_else(|| "ERR_THREADS_INVALID: the project has no folder".to_string())?;
        // After an archive: start from the thread's branch if it survived,
        // then put back the last checkpoint (uncommitted work included).
        let recreate = state == "removed" || (state == "ready" && row.worktree_path.is_some());
        let mut base = row.base_branch.clone();
        if recreate {
            if let Some(b) = row.branch.as_deref() {
                if branch_exists(&root, b).await {
                    base = Some(b.to_string());
                }
            }
        }
        let req = WorktreeRequest {
            repo: root,
            thread: row.thread_id.clone(),
            base,
            branch: if recreate { row.branch.clone() } else { None },
            start_from_origin: false,
            submodules: None,
            run_setup: true,
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SetupSnapshot>();
        let sink = self.hooks.on_setup.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(row.thread_id.clone(), cancel.clone());
        let create = worktree::create(&req, cancel, move |snap: &SetupSnapshot| {
            if let Some(s) = &sink {
                s(snap);
            }
            let _ = tx.send(snap.clone());
        });
        let thread_id = row.thread_id.clone();
        let drain = async {
            let mut last = String::new();
            while let Some(snap) = rx.recv().await {
                if snap.phase != "running" {
                    continue;
                }
                let sig = setup_signature(&snap);
                if sig == last {
                    continue;
                }
                last = sig;
                let _ = self
                    .record(DomainEvent::WorktreeUpdated {
                        thread_id: thread_id.clone(),
                        state: "preparing".into(),
                        path: None,
                        branch: snap.branch.clone(),
                        base: snap.base.clone(),
                        setup: serde_json::to_value(&snap).ok(),
                        error: None,
                        updated_at: now_iso(),
                    })
                    .await;
            }
        };
        let (result, _) = tokio::join!(create, drain);
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&thread_id);
        match result {
            Ok(info) => {
                let path = info.path.to_string_lossy().into_owned();
                self.record(DomainEvent::WorktreeUpdated {
                    thread_id: thread_id.clone(),
                    state: "ready".into(),
                    path: Some(path),
                    branch: Some(info.branch.clone()),
                    base: Some(info.base.clone()).filter(|b| !b.is_empty()),
                    setup: None,
                    error: None,
                    updated_at: now_iso(),
                })
                .await?;
                if recreate {
                    let last = self
                        .engine
                        .read(|c| store::checkpoints(c, &thread_id))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| c.turn_count)
                        .max();
                    if let Some(n) = last {
                        match checkpoint::restore(&info.path, &thread_id, n, true, false).await {
                            Ok(r) if !r.files.is_empty() => {
                                let _ = self
                                    .record(DomainEvent::FilesRestored {
                                        thread_id: thread_id.clone(),
                                        turn_count: n,
                                        files: r.files,
                                        restored_at: now_iso(),
                                    })
                                    .await;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                self.note(
                                    &thread_id,
                                    "worktree.restore",
                                    "warning",
                                    e.to_string(),
                                    json!({ "turnCount": n }),
                                )
                                .await
                            }
                        }
                    }
                }
                Ok(Some(info.path))
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = self
                    .record(DomainEvent::WorktreeUpdated {
                        thread_id,
                        state: "failed".into(),
                        path: None,
                        branch: None,
                        base: None,
                        setup: None,
                        error: Some(msg.clone()),
                        updated_at: now_iso(),
                    })
                    .await;
                Err(msg)
            }
        }
    }

    /// Before turn `ordinal` goes to the driver: the worktree is ready and
    /// checkpoint `ordinal - 1` exists. Returns the cwd of the turn. An error
    /// means the worktree could not be made (the turn must not run in the
    /// user's own checkout instead).
    pub async fn before_turn(
        &self,
        thread_id: &str,
        ordinal: u32,
    ) -> Result<Option<PathBuf>, String> {
        let _g = self.lock(thread_id).await;
        let row = self.row(thread_id)?;
        let made = self.ensure_worktree(&row).await?;
        let row = if made.is_some() {
            self.row(thread_id)?
        } else {
            row
        };
        let cwd = self.cwd_of_row(&row);
        if let (Some(dir), true) = (&cwd, ordinal > 0) {
            if let Err(e) = self.capture_if_missing(thread_id, dir, ordinal - 1).await {
                self.note(
                    thread_id,
                    "checkpoint.failed",
                    "warning",
                    e.clone(),
                    json!({ "turnCount": ordinal - 1 }),
                )
                .await;
            }
        }
        Ok(cwd)
    }

    /// Checkpoint of a finished turn (idempotent: a checkpoint already taken
    /// for that turn is kept).
    pub async fn after_turn(&self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        let _g = self.lock(thread_id).await;
        let row = self.row(thread_id)?;
        if row.worktree_state.is_some() && row.worktree_state.as_deref() != Some("ready") {
            return Ok(());
        }
        let Some(turn) = self.engine.read(|c| store::turn_row(c, turn_id))? else {
            return Ok(());
        };
        let Some(dir) = self.cwd_of_row(&row) else {
            return Ok(());
        };
        self.capture_if_missing(thread_id, &dir, turn.ordinal).await
    }

    async fn capture_if_missing(&self, thread_id: &str, dir: &Path, n: u32) -> Result<(), String> {
        if self
            .engine
            .read(|c| store::checkpoint(c, thread_id, n))?
            .is_some()
        {
            return Ok(());
        }
        self.capture(thread_id, dir, n).await
    }

    async fn capture(&self, thread_id: &str, dir: &Path, n: u32) -> Result<(), String> {
        let cp = checkpoint::capture(dir, thread_id, n).await.map_err(gerr)?;
        let has_prev = n > 0
            && self
                .engine
                .read(|c| store::checkpoint(c, thread_id, n - 1))?
                .is_some();
        let (files, additions, deletions) = if has_prev {
            let opts = DiffOptions {
                stat_only: true,
                ..Default::default()
            };
            match checkpoint::diff_turn(dir, thread_id, n, &opts).await {
                Ok(d) => {
                    let mut files = files_of(&d);
                    // A stat-only diff cannot tell added/deleted apart from
                    // modified; `--name-status` can, cheaply.
                    let st = name_status(dir, thread_id, n).await;
                    for f in files.iter_mut() {
                        if let Some(s) = st.get(&f.path) {
                            if f.status == "modified" {
                                f.status = s.clone();
                            }
                        }
                    }
                    (files, d.additions, d.deletions)
                }
                Err(_) => (Vec::new(), 0, 0),
            }
        } else {
            (Vec::new(), 0, 0)
        };
        let turn_id = self
            .engine
            .read(|c| store::turn_at(c, thread_id, n))?
            .map(|t| t.turn_id);
        self.record(DomainEvent::CheckpointCaptured {
            thread_id: thread_id.to_string(),
            turn_count: n,
            turn_id,
            ref_name: cp.ref_name,
            commit: cp.commit,
            status: "ready".into(),
            files,
            additions,
            deletions,
            captured_at: now_iso(),
        })
        .await
    }

    // ── Branch name ─────────────────────────────────────────────────────

    async fn rename_after_first(&self, thread_id: &str, text: &str) {
        match self.row(thread_id) {
            Ok(r) if r.worktree_state.is_some() => {}
            _ => return,
        }
        let name = self.branch_name(text).await;
        let Some(name) = name else { return };
        let _g = self.lock(thread_id).await;
        let Ok(row) = self.row(thread_id) else { return };
        if row.worktree_state.as_deref() != Some("ready")
            || !row
                .branch
                .as_deref()
                .map(worktree::is_temp_branch)
                .unwrap_or(false)
        {
            return;
        }
        let Some(path) = row.worktree_path.as_deref().map(PathBuf::from) else {
            return;
        };
        match worktree::rename_branch(&path, &name).await {
            Ok(final_name) => {
                let _ = self
                    .record(DomainEvent::WorktreeUpdated {
                        thread_id: thread_id.to_string(),
                        state: "ready".into(),
                        path: None,
                        branch: Some(final_name),
                        base: None,
                        setup: None,
                        error: None,
                        updated_at: now_iso(),
                    })
                    .await;
            }
            Err(e) => tracing::debug!("[threads/git] rename branch of {thread_id}: {e}"),
        }
    }

    /// `omniget/<words>` from the cheap model, else from the prompt itself.
    pub async fn branch_name(&self, text: &str) -> Option<String> {
        if let Some(gen) = &self.hooks.text {
            let prompt: String = text.chars().take(2000).collect();
            let answer = tokio::time::timeout(
                Duration::from_secs(20),
                gen(BRANCH_SYSTEM.to_string(), prompt),
            )
            .await
            .ok()
            .flatten();
            if let Some(name) = answer.as_deref().and_then(clean_branch_suggestion) {
                return Some(name);
            }
        }
        heuristic_branch_name(text)
    }

    // ── Revert ("edit from here") ───────────────────────────────────────

    async fn revert(&self, thread_id: &str, n: u32, restore_files: bool, event_id: &str) {
        let _g = self.lock(thread_id).await;
        let fail = |msg: String| async move {
            let _ = self
                .record(DomainEvent::ActivityAppended {
                    thread_id: thread_id.to_string(),
                    activity: Activity {
                        activity_id: format!("{thread_id}:revert-failed:{event_id}"),
                        turn_id: None,
                        kind: "revert.failed".into(),
                        tone: "error".into(),
                        summary: msg.clone(),
                        payload: json!({ "turnCount": n, "error": msg, "requestEventId": event_id }),
                        created_at: now_iso(),
                    },
                })
                .await;
        };
        let row = match self.row(thread_id) {
            Ok(r) => r,
            Err(e) => return fail(e).await,
        };
        let dir = self.cwd_of_row(&row);
        if restore_files {
            let Some(dir) = dir else {
                return fail("the thread has no folder to restore".into()).await;
            };
            match checkpoint::restore(&dir, thread_id, n, true, true).await {
                Ok(r) => {
                    let _ = self
                        .record(DomainEvent::FilesRestored {
                            thread_id: thread_id.to_string(),
                            turn_count: n,
                            files: r.files,
                            restored_at: now_iso(),
                        })
                        .await;
                }
                Err(e) => return fail(e.to_string()).await,
            }
        } else if let Some(dir) = dir {
            // The files stay; the next turn's diff starts from checkpoint N.
            let _ = checkpoint::delete_after(&dir, thread_id, n).await;
        }
        let env = CommandEnvelope {
            command_id: Some(format!("server:revert:{event_id}")),
            command: Command::Revert {
                thread_id: thread_id.to_string(),
                turn_count: n,
            },
        };
        if let Err(e) = self.engine.dispatch(env).await {
            fail(e).await;
        }
    }

    /// Dispatch `thread.revert-to-turn` and wait until the conversation is
    /// cut (or the revert failed). The first dropped prompt comes back.
    pub async fn revert_to_turn(
        &self,
        thread_id: &str,
        turn: u32,
        restore_files: bool,
    ) -> Result<RevertOutcome, String> {
        let prompt = self
            .engine
            .read(|c| match store::turn_at(c, thread_id, turn + 1)? {
                Some(t) => store::turn_prompt(c, &t),
                None => Ok(None),
            })?;
        let mut rx = self.engine.subscribe();
        let res = self
            .engine
            .dispatch(CommandEnvelope {
                command_id: None,
                command: Command::RevertToTurn {
                    thread_id: thread_id.to_string(),
                    turn,
                    restore_files,
                },
            })
            .await?;
        let request_event = res
            .events
            .first()
            .map(|e| e.event_id.clone())
            .unwrap_or_default();
        let mut restored = Vec::new();
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(ev) => match ev.event {
                        DomainEvent::FilesRestored {
                            thread_id: t,
                            files,
                            ..
                        } if t == thread_id => restored = files,
                        DomainEvent::Reverted {
                            thread_id: t,
                            turn_count,
                        } if t == thread_id && turn_count == turn => return Ok(()),
                        DomainEvent::ActivityAppended {
                            thread_id: t,
                            activity,
                        } if t == thread_id
                            && activity.kind == "revert.failed"
                            && activity.activity_id.ends_with(&request_event) =>
                        {
                            return Err(format!("{ERR_THREADS_GIT}: {}", activity.summary))
                        }
                        _ => {}
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(format!("{ERR_THREADS_GIT}: engine stopped"))
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(300), wait)
            .await
            .map_err(|_| format!("{ERR_THREADS_GIT}: revert timed out"))??;
        let (prompt, attachments) = match prompt {
            Some((p, a)) => (Some(p), a),
            None => (None, Value::Array(vec![])),
        };
        Ok(RevertOutcome {
            thread_id: thread_id.to_string(),
            turn_count: turn,
            restored_files: restored,
            prompt,
            attachments,
        })
    }

    // ── Cleanup ─────────────────────────────────────────────────────────

    async fn cleanup(&self, thread_id: &str, deleting: bool) {
        let _g = self.lock(thread_id).await;
        let Ok(row) = self.row(thread_id) else { return };
        let wt = row
            .worktree_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir() && worktree::is_omniget_worktree(p));
        match wt {
            Some(path) if row.worktree_state.is_some() => {
                if !deleting {
                    // Work not in a checkpoint yet survives the folder.
                    let dirty = repo::status(&path).await.map(|s| s.dirty).unwrap_or(false);
                    if dirty {
                        let n = row.turn_count;
                        if let Err(e) = self.capture(thread_id, &path, n).await {
                            tracing::warn!("[threads/git] archive checkpoint of {thread_id}: {e}");
                            return;
                        }
                    }
                }
                let temp = row
                    .branch
                    .as_deref()
                    .map(worktree::is_temp_branch)
                    .unwrap_or(false);
                let thread = deleting.then_some(thread_id);
                match worktree::remove(&path, true, deleting && temp, thread).await {
                    Ok(_) if !deleting => {
                        let _ = self
                            .record(DomainEvent::WorktreeUpdated {
                                thread_id: thread_id.to_string(),
                                state: "removed".into(),
                                path: None,
                                branch: None,
                                base: None,
                                setup: None,
                                error: None,
                                updated_at: now_iso(),
                            })
                            .await;
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("[threads/git] remove worktree of {thread_id}: {e}"),
                }
            }
            _ if deleting => {
                if let Some(dir) = self.cwd_of_row(&row) {
                    let _ = checkpoint::delete_all(&dir, thread_id).await;
                }
            }
            _ => {}
        }
    }

    // ── Diff, status and the split button ───────────────────────────────

    /// Diff of turn `turn` (checkpoint N-1 → N), or of the whole thread
    /// (baseline → live tree) when `turn` is `None`.
    pub async fn diff(
        &self,
        thread_id: &str,
        turn: Option<u32>,
        opts: &DiffOptions,
    ) -> Result<DiffResult, String> {
        let dir = self.cwd(thread_id)?;
        match turn {
            Some(n) => checkpoint::diff_turn(&dir, thread_id, n, opts).await,
            None => checkpoint::diff_full(&dir, thread_id, None, opts).await,
        }
        .map_err(gerr)
    }

    /// Files of the thread's folder whose path contains every word of
    /// `query` (case-insensitive), basename hits first. Tracked and
    /// untracked-but-not-ignored files; a folder without git is walked.
    pub async fn files(
        &self,
        thread_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, String> {
        let dir = self.cwd(thread_id)?;
        let listed = crate::core::vcs::runner::Invocation::git()
            .cwd(&dir)
            .args(["ls-files", "-co", "--exclude-standard", "-z"])
            .unchecked()
            .run()
            .await
            .ok()
            .filter(|o| o.ok());
        let all: Vec<String> = match listed {
            Some(o) => o
                .stdout
                .split(|b| *b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect(),
            None => {
                let root = dir.clone();
                tokio::task::spawn_blocking(move || walk_files(&root, 20_000))
                    .await
                    .map_err(gerr)?
            }
        };
        Ok(rank_files(all, query, limit.clamp(1, 500)))
    }

    pub async fn status(&self, thread_id: &str) -> Result<RepoStatus, String> {
        repo::status(&self.cwd(thread_id)?).await.map_err(gerr)
    }

    /// A commit message for what the thread changed: the cheap model over
    /// the diff, else `vcs`'s own suggestion.
    pub async fn commit_message(&self, thread_id: &str) -> Result<CommitMessage, String> {
        let dir = self.cwd(thread_id)?;
        let changes = actions::working_changes(&dir).await.map_err(gerr)?;
        let fallback = actions::suggest_commit_message(&changes);
        let heuristic = CommitMessage {
            subject: fallback.subject.clone(),
            body: fallback.body.clone(),
            source: "heuristic".into(),
            files: changes.len(),
        };
        let Some(gen) = self.hooks.text.clone() else {
            return Ok(heuristic);
        };
        if changes.is_empty() {
            return Ok(heuristic);
        }
        let patch = match working_patch(&dir).await {
            Some(p) => p,
            None => return Ok(heuristic),
        };
        let answer = tokio::time::timeout(
            Duration::from_secs(30),
            gen(COMMIT_SYSTEM.to_string(), patch),
        )
        .await
        .ok()
        .flatten();
        Ok(match answer.as_deref().and_then(clean_commit_message) {
            Some((subject, body)) => CommitMessage {
                subject,
                body,
                source: "model".into(),
                files: changes.len(),
            },
            None => heuristic,
        })
    }

    pub async fn commit(
        &self,
        thread_id: &str,
        message: Option<String>,
    ) -> Result<CommitResult, String> {
        let dir = self.cwd(thread_id)?;
        let message = match message.filter(|m| !m.trim().is_empty()) {
            Some(m) => m,
            None => self.commit_message(thread_id).await?.text(),
        };
        let r = actions::commit(&dir, Some(&message), None)
            .await
            .map_err(gerr)?;
        if r.status == "created" {
            self.note(
                thread_id,
                "git.commit",
                "info",
                r.subject.clone(),
                serde_json::to_value(&r).unwrap_or(Value::Null),
            )
            .await;
        }
        Ok(r)
    }

    pub async fn push(&self, thread_id: &str) -> Result<PushResult, String> {
        let dir = self.cwd(thread_id)?;
        let r = actions::push(&dir).await.map_err(gerr)?;
        if r.status == "pushed" {
            self.note(
                thread_id,
                "git.push",
                "info",
                r.upstream.clone(),
                serde_json::to_value(&r).unwrap_or(Value::Null),
            )
            .await;
        }
        Ok(r)
    }

    /// Opens the PR/MR (draft by default) and records the badge.
    pub async fn open_pr(
        &self,
        thread_id: &str,
        title: Option<String>,
        body: Option<String>,
        base: Option<String>,
        draft: bool,
    ) -> Result<PrCreateResult, String> {
        let row = self.row(thread_id)?;
        let dir = self.cwd(thread_id)?;
        let title = match title.filter(|t| !t.trim().is_empty()) {
            Some(t) => t,
            None => {
                let t = row.title.trim();
                if t.is_empty() || t == super::decider::DEFAULT_THREAD_TITLE {
                    self.commit_message(thread_id).await?.subject
                } else {
                    t.to_string()
                }
            }
        };
        let body = body.unwrap_or_else(|| pr_body(&row));
        let req = PrRequest {
            title,
            body,
            base: base.or(row.base_branch.clone()),
            draft,
        };
        let r = actions::pr_create(&dir, &req).await.map_err(gerr)?;
        self.set_pr(thread_id, &row, Some(&r.pr)).await;
        Ok(r)
    }

    /// Reads the PR of the thread's branch again and updates the badge when
    /// it changed. Called by the UI when the thread is visible.
    pub async fn refresh_pr(&self, thread_id: &str) -> Result<Option<PrInfo>, String> {
        let row = self.row(thread_id)?;
        let dir = self.cwd(thread_id)?;
        let pr = actions::pr_view(&dir, None).await.map_err(gerr)?;
        self.set_pr(thread_id, &row, pr.as_ref()).await;
        Ok(pr)
    }

    async fn set_pr(&self, thread_id: &str, row: &ThreadRow, pr: Option<&PrInfo>) {
        let v = pr.and_then(|p| serde_json::to_value(p).ok());
        if v == row.pr {
            return;
        }
        let _ = self
            .record(DomainEvent::PrUpdated {
                thread_id: thread_id.to_string(),
                pr: v,
                updated_at: now_iso(),
            })
            .await;
    }
}

const BRANCH_SYSTEM: &str = "You name git branches. Reply with only a short kebab-case branch \
name of 2 to 5 lowercase English words that says what the task is about. No prefix, no quotes, \
no explanation.";

const COMMIT_SYSTEM: &str = "Write a git commit message for the diff the user sends. First line: \
an imperative subject of at most 72 characters. Then, only if useful, a blank line and a short \
body of plain lines. Reply with the message only: no code fences, no preamble.";

async fn branch_exists(root: &Path, name: &str) -> bool {
    crate::core::vcs::runner::Invocation::git()
        .cwd(root)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ])
        .unchecked()
        .run()
        .await
        .map(|o| o.ok())
        .unwrap_or(false)
}

/// HEAD → live tree as one patch (clipped), for the commit-message model.
async fn working_patch(dir: &Path) -> Option<String> {
    let ctx = checkpoint::context_for(dir).await.ok()?;
    let tree = checkpoint::live_tree(&ctx).await.ok()?;
    let head = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{tree}"])
        .unchecked()
        .run()
        .await
        .ok()
        .filter(|o| o.ok())
        .map(|o| o.trimmed())
        // The empty tree: a repository without a commit yet.
        .unwrap_or_else(|| "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string());
    let d = diff::diff_trees(
        &ctx,
        &head,
        &tree,
        &DiffOptions {
            max_bytes: 48 * 1024,
            max_file_bytes: 12 * 1024,
            ..Default::default()
        },
    )
    .await
    .ok()?;
    Some(patch_for_prompt(&d, 12_000))
}

/// `path → added|deleted|modified|renamed` between checkpoints N-1 and N.
async fn name_status(dir: &Path, thread_id: &str, n: u32) -> HashMap<String, String> {
    let Ok(ctx) = checkpoint::context_for(dir).await else {
        return HashMap::new();
    };
    let out = ctx
        .git()
        .args([
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            "--name-status",
            "-z",
            &checkpoint::ref_name(thread_id, n - 1),
            &checkpoint::ref_name(thread_id, n),
            "--",
        ])
        .unchecked()
        .run()
        .await;
    match out {
        Ok(o) if o.ok() => parse_name_status_z(&o.stdout),
        _ => HashMap::new(),
    }
}

/// `git diff --name-status -z` → `path → status` (the new path of a rename).
pub fn parse_name_status_z(raw: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut it = raw
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned());
    while let Some(code) = it.next() {
        let status = match code.chars().next() {
            Some('A') => "added",
            Some('D') => "deleted",
            Some('R') => "renamed",
            Some('C') => "added",
            _ => "modified",
        };
        if matches!(code.chars().next(), Some('R') | Some('C')) {
            let _old = it.next();
        }
        if let Some(path) = it.next() {
            out.insert(path, status.to_string());
        }
    }
    out
}

fn walk_files(root: &Path, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root)
        .max_depth(12)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            e.depth() == 0 || !(n.starts_with('.') || n == "node_modules" || n == "target")
        })
        .flatten()
    {
        if e.file_type().is_file() {
            if let Ok(rel) = e.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
                if out.len() >= cap {
                    break;
                }
            }
        }
    }
    out
}

/// Paths containing every word of `query`; basename hits, then shorter
/// paths first. An empty query lists the first `limit` paths.
pub fn rank_files(all: Vec<String>, query: &str, limit: usize) -> Vec<String> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut hits: Vec<(u8, usize, String)> = all
        .into_iter()
        .filter_map(|p| {
            let lower = p.to_lowercase();
            if !words.iter().all(|w| lower.contains(w.as_str())) {
                return None;
            }
            let base = lower.rsplit('/').next().unwrap_or(&lower).to_string();
            let rank = if words.is_empty() {
                1
            } else if words.iter().all(|w| base.contains(w.as_str())) {
                if words.len() == 1 && base.starts_with(words[0].as_str()) {
                    0
                } else {
                    1
                }
            } else {
                2
            };
            Some((rank, p.len(), p))
        })
        .collect();
    hits.sort();
    hits.into_iter().take(limit).map(|h| h.2).collect()
}

fn files_of(d: &DiffResult) -> Vec<CheckpointFile> {
    d.files
        .iter()
        .map(|f| CheckpointFile {
            path: f.path.clone(),
            status: f.status.clone(),
            additions: f.additions,
            deletions: f.deletions,
        })
        .collect()
}

fn setup_signature(s: &SetupSnapshot) -> String {
    let mut sig = s.phase.clone();
    for st in &s.stages {
        sig.push('|');
        sig.push_str(&st.id);
        sig.push(':');
        sig.push_str(&st.state);
    }
    sig
}

fn pr_body(row: &ThreadRow) -> String {
    format!(
        "Opened from the OmniGet thread \"{}\" ({} turn(s)).",
        row.title, row.turn_count
    )
}

// ── Pure helpers ────────────────────────────────────────────────────────

const BRANCH_PREFIX: &str = "omniget/";
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "to", "of", "and", "or", "in", "on", "for", "with", "please", "can", "you",
    "could", "would", "i", "me", "my", "we", "our", "it", "this", "that", "is", "are", "be", "do",
    "de", "da", "do", "das", "dos", "e", "o", "os", "as", "um", "uma", "para", "pra", "no", "na",
    "nos", "nas", "com", "por", "que", "se", "voce", "você", "favor", "em",
];

fn slug_words(text: &str, max_words: usize) -> Vec<String> {
    let mut words = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let w: String = raw
            .chars()
            .flat_map(|c| c.to_lowercase())
            .map(fold_accent)
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if w.is_empty() || STOPWORDS.contains(&w.as_str()) {
            continue;
        }
        words.push(w);
        if words.len() == max_words {
            break;
        }
    }
    words
}

fn fold_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

fn join_branch(words: &[String]) -> Option<String> {
    let mut name = String::new();
    for w in words {
        if name.len() + w.len() + 1 > 40 {
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(w);
    }
    (!name.is_empty()).then(|| format!("{BRANCH_PREFIX}{name}"))
}

/// `omniget/<up to 5 meaningful words of the first line>`.
pub fn heuristic_branch_name(text: &str) -> Option<String> {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    join_branch(&slug_words(first, 5))
}

/// The model's answer as a branch name (first line, words only).
pub fn clean_branch_suggestion(answer: &str) -> Option<String> {
    let line = answer
        .lines()
        .map(|l| l.trim().trim_matches(|c| c == '`' || c == '"' || c == '\''))
        .find(|l| !l.is_empty())?;
    let line = line
        .strip_prefix("branch:")
        .or_else(|| line.strip_prefix("Branch:"))
        .unwrap_or(line)
        .trim();
    let line = line.strip_prefix(BRANCH_PREFIX).unwrap_or(line);
    let words: Vec<String> = slug_words(line, 6);
    if words.is_empty() || line.len() > 80 {
        return None;
    }
    join_branch(&words)
}

/// `(subject ≤ 72 chars, body)` from the model's answer.
pub fn clean_commit_message(answer: &str) -> Option<(String, String)> {
    let text: Vec<&str> = answer
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect();
    let mut lines = text.iter().map(|l| l.trim_end());
    let subject = lines.by_ref().map(str::trim).find(|l| !l.is_empty())?;
    let subject = subject
        .trim_start_matches("Subject:")
        .trim()
        .trim_matches('"')
        .trim();
    if subject.is_empty() {
        return None;
    }
    let mut s: String = subject.chars().take(72).collect();
    if subject.chars().count() > 72 {
        if let Some(i) = s.rfind(' ') {
            s.truncate(i);
        }
    }
    let body: Vec<&str> = lines.collect();
    let body = body.join("\n").trim().to_string();
    Some((s, body))
}

/// The patch of a diff, file by file, clipped to `max` bytes.
pub fn patch_for_prompt(d: &DiffResult, max: usize) -> String {
    let mut out = String::new();
    for f in &d.files {
        let chunk = if f.binary {
            format!("Binary file {} ({})\n", f.path, f.status)
        } else if f.patch.is_empty() {
            format!(
                "{} {} (+{} -{})\n",
                f.status, f.path, f.additions, f.deletions
            )
        } else {
            format!("{}\n", f.patch.trim_end())
        };
        if out.len() + chunk.len() > max {
            let room = max.saturating_sub(out.len());
            let mut end = room.min(chunk.len());
            while end > 0 && !chunk.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&chunk[..end]);
            out.push_str("\n[diff clipped]\n");
            break;
        }
        out.push_str(&chunk);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_from_prompt_skips_filler_and_accents() {
        assert_eq!(
            heuristic_branch_name("Please add a hello file to the repo").as_deref(),
            Some("omniget/add-hello-file-repo")
        );
        assert_eq!(
            heuristic_branch_name("\n  Corrigir a validação do login com erro\nmais").as_deref(),
            Some("omniget/corrigir-validacao-login-erro")
        );
        assert_eq!(heuristic_branch_name("   \n ?!"), None);
    }

    #[test]
    fn model_branch_answer_is_cleaned() {
        assert_eq!(
            clean_branch_suggestion("`fix-login-timeout`\n").as_deref(),
            Some("omniget/fix-login-timeout")
        );
        assert_eq!(
            clean_branch_suggestion("branch: omniget/Add Dark Mode").as_deref(),
            Some("omniget/add-dark-mode")
        );
        assert_eq!(clean_branch_suggestion("  \n"), None);
    }

    #[test]
    fn files_rank_basename_first() {
        let all = vec![
            "src/lib/app.ts".to_string(),
            "docs/app-notes/readme.md".to_string(),
            "src/components/AppShell.svelte".to_string(),
            "tests/other.rs".to_string(),
        ];
        assert_eq!(
            rank_files(all.clone(), "app", 10),
            [
                "src/lib/app.ts",
                "src/components/AppShell.svelte",
                "docs/app-notes/readme.md"
            ]
        );
        assert_eq!(
            rank_files(all.clone(), "src shell", 10),
            ["src/components/AppShell.svelte"]
        );
        assert_eq!(rank_files(all, "", 2).len(), 2);
    }

    #[test]
    fn name_status_z_maps_codes() {
        let m = parse_name_status_z(b"A\0new.txt\0M\0a.txt\0R100\0old.rs\0new.rs\0D\0gone\0");
        assert_eq!(m["new.txt"], "added");
        assert_eq!(m["a.txt"], "modified");
        assert_eq!(m["new.rs"], "renamed");
        assert_eq!(m["gone"], "deleted");
        assert!(!m.contains_key("old.rs"));
    }

    #[test]
    fn model_commit_answer_is_cleaned() {
        let (s, b) =
            clean_commit_message("```\nAdd hello file\n\nCreates hello.txt with a greeting.\n```")
                .unwrap();
        assert_eq!(s, "Add hello file");
        assert_eq!(b, "Creates hello.txt with a greeting.");
        let long = format!("Subject: {}", "word ".repeat(30));
        let (s, _) = clean_commit_message(&long).unwrap();
        assert!(s.chars().count() <= 72 && !s.ends_with(' '));
        assert!(clean_commit_message("```\n```").is_none());
    }
}
