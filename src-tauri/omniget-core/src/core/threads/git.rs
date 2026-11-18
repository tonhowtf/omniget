use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::broadcast;

use super::engine::ThreadsEngine;
use super::model::{Activity, CheckpointFile, Command, CommandEnvelope, DomainEvent, StoredEvent};
use super::store::{self, ThreadRow};
use crate::core::llm::drivers::now_iso;
use crate::core::vcs::checkpoint;
use crate::core::vcs::diff::{DiffOptions, DiffResult};
use crate::core::vcs::repo;
use crate::core::vcs::worktree::{self, SetupSnapshot, WorktreeRequest};

pub const ERR_THREADS_GIT: &str = "ERR_THREADS_GIT";

pub type TextFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;
pub type TextGen = Arc<dyn Fn(String, String) -> TextFuture + Send + Sync>;
pub type SetupSink = Arc<dyn Fn(&SetupSnapshot) + Send + Sync>;

#[derive(Clone, Default)]
pub struct GitHooks {
    pub text: Option<TextGen>,
    pub on_setup: Option<SetupSink>,
}

pub struct ThreadGit {
    engine: Arc<ThreadsEngine>,
    hooks: GitHooks,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
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
}

const BRANCH_SYSTEM: &str = "You name git branches. Reply with only a short kebab-case branch \
name of 2 to 5 lowercase English words that says what the task is about. No prefix, no quotes, \
no explanation.";

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

pub fn heuristic_branch_name(text: &str) -> Option<String> {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    join_branch(&slug_words(first, 5))
}

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
