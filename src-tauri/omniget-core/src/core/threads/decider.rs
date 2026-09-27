//! `decide(model, command, ctx) -> events`: pure, no I/O, no clock (the
//! engine passes `now`), no id minting (the engine normalizes the command
//! first). Everything that changes a thread goes through here, including the
//! runtime events of a driver, which are folded into domain events.

use serde_json::{json, Value};

use super::model::{Activity, Command, DomainEvent, ReadModel, ThreadState, WorktreeSpec};
use crate::core::llm::drivers::{
    ItemPayload, ItemStatus, ItemType, RequestType, RuntimeEvent, RuntimeEventKind, SessionState,
    StreamKind, TurnEndState,
};

pub const ERR_THREADS_NOT_FOUND: &str = "ERR_THREADS_NOT_FOUND";
pub const ERR_THREADS_EXISTS: &str = "ERR_THREADS_EXISTS";
pub const ERR_THREADS_BUSY: &str = "ERR_THREADS_BUSY";
pub const ERR_THREADS_IDLE: &str = "ERR_THREADS_IDLE";
pub const ERR_THREADS_INVALID: &str = "ERR_THREADS_INVALID";
pub const ERR_THREADS_NO_REQUEST: &str = "ERR_THREADS_NO_REQUEST";
pub const ERR_THREADS_ANSWERED: &str = "ERR_THREADS_ANSWERED";
pub const ERR_THREADS_DELETED: &str = "ERR_THREADS_DELETED";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecideError {
    pub code: &'static str,
    pub message: String,
}

impl DecideError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DecideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DecideError {}

/// One finished turn of a source thread, loaded by the engine for `fork`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkTurn {
    pub turn_id: String,
    pub ordinal: u32,
    pub state: String,
    pub requested_at: String,
    pub completed_at: Option<String>,
    /// `(message_id, role, text, created_at)`.
    pub messages: Vec<(String, String, String, String)>,
    pub activities: Vec<Activity>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForkSource {
    pub turns: Vec<ForkTurn>,
}

/// What the decider may know besides the model.
#[derive(Debug, Clone, Copy)]
pub struct DecideCtx<'a> {
    pub now: &'a str,
    pub fork: Option<&'a ForkSource>,
}

type R = Result<Vec<DomainEvent>, DecideError>;

fn thread<'a>(model: &'a ReadModel, id: &str) -> Result<&'a ThreadState, DecideError> {
    match model.threads.get(id) {
        Some(t) if !t.deleted => Ok(t),
        Some(_) => Err(DecideError::new(
            ERR_THREADS_DELETED,
            format!("thread {id} was deleted"),
        )),
        None => Err(DecideError::new(
            ERR_THREADS_NOT_FOUND,
            format!("no thread {id}"),
        )),
    }
}

fn required(v: &Option<String>, what: &str) -> Result<String, DecideError> {
    v.clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| DecideError::new(ERR_THREADS_INVALID, format!("missing {what}")))
}

/// First line of a message, ≤ 60 chars: the title of a fresh thread.
pub fn title_from(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut s: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        s.push('…');
    }
    s
}

pub const DEFAULT_THREAD_TITLE: &str = "New thread";

/// Last component of a folder path (`Project` when there is none).
pub fn folder_title(root: &str) -> String {
    root.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or("Project")
        .to_string()
}

/// Host-observed facts `thread.host.record` accepts. Everything else a
/// client or a driver asks for goes through its own command.
pub fn host_recordable(e: &DomainEvent) -> bool {
    matches!(
        e,
        DomainEvent::WorktreeUpdated { .. }
            | DomainEvent::CheckpointCaptured { .. }
            | DomainEvent::FilesRestored { .. }
            | DomainEvent::PrUpdated { .. }
            | DomainEvent::TerminalAttached { .. }
            | DomainEvent::TerminalClosed { .. }
            | DomainEvent::ExternalLinked { .. }
            | DomainEvent::TurnUsage { .. }
            | DomainEvent::ActivityAppended { .. }
            | DomainEvent::SessionSet { .. }
            | DomainEvent::ThreadMetaUpdated { .. }
    )
}

pub fn decide(model: &ReadModel, cmd: &Command, ctx: &DecideCtx) -> R {
    use Command::*;
    let now = ctx.now.to_string();
    match cmd {
        ProjectCreate {
            project_id,
            title,
            workspace_root,
        } => {
            let id = required(project_id, "projectId")?;
            if model.projects.get(&id).map(|p| !p.deleted).unwrap_or(false) {
                return Err(DecideError::new(
                    ERR_THREADS_EXISTS,
                    format!("project {id} exists"),
                ));
            }
            let root = workspace_root.trim().to_string();
            if !root.is_empty()
                && model
                    .projects
                    .values()
                    .any(|p| !p.deleted && p.workspace_root == root)
            {
                return Err(DecideError::new(
                    ERR_THREADS_EXISTS,
                    format!("a project already uses {root}"),
                ));
            }
            let title = match title.trim() {
                "" => folder_title(&root),
                t => t.to_string(),
            };
            Ok(vec![DomainEvent::ProjectCreated {
                project_id: id,
                title,
                workspace_root: root,
                created_at: now,
            }])
        }
        ProjectUpdate {
            project_id,
            title,
            workspace_root,
        } => {
            match model.projects.get(project_id) {
                Some(p) if !p.deleted => {}
                _ => {
                    return Err(DecideError::new(
                        ERR_THREADS_NOT_FOUND,
                        format!("no project {project_id}"),
                    ))
                }
            }
            Ok(vec![DomainEvent::ProjectMetaUpdated {
                project_id: project_id.clone(),
                title: title.clone().filter(|t| !t.trim().is_empty()),
                workspace_root: workspace_root.clone(),
                updated_at: now,
            }])
        }
        ProjectDelete { project_id } => {
            match model.projects.get(project_id) {
                Some(p) if !p.deleted => {}
                _ => {
                    return Err(DecideError::new(
                        ERR_THREADS_NOT_FOUND,
                        format!("no project {project_id}"),
                    ))
                }
            }
            let mut out: Vec<DomainEvent> = model
                .threads
                .values()
                .filter(|t| t.project_id == *project_id && !t.deleted)
                .map(|t| DomainEvent::ThreadDeleted {
                    thread_id: t.id.clone(),
                    deleted_at: now.clone(),
                })
                .collect();
            out.sort_by(|a, b| a.aggregate().1.cmp(b.aggregate().1));
            out.push(DomainEvent::ProjectDeleted {
                project_id: project_id.clone(),
                deleted_at: now,
            });
            Ok(out)
        }
        ThreadCreate {
            thread_id,
            project_id,
            project_path,
            worktree,
            base_branch,
            title,
            instance_id,
            driver,
            model: m,
            agent_id,
            runtime_mode,
            interaction_mode,
            branch,
            worktree_path,
        } => {
            let id = required(thread_id, "threadId")?;
            if model.threads.contains_key(&id) {
                return Err(DecideError::new(
                    ERR_THREADS_EXISTS,
                    format!("thread {id} exists"),
                ));
            }
            if instance_id.trim().is_empty() || driver.trim().is_empty() {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    "instanceId and driver are required",
                ));
            }
            let mut out = Vec::new();
            let path = project_path
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty());
            // An explicit project wins; a path finds (or makes) its project.
            let (project, root) = match model.projects.get(project_id) {
                Some(p) if !p.deleted => (p.id.clone(), p.workspace_root.clone()),
                _ => match path {
                    Some(path) => match model
                        .projects
                        .values()
                        .find(|p| !p.deleted && p.workspace_root == path)
                    {
                        Some(p) => (p.id.clone(), p.workspace_root.clone()),
                        None => {
                            let pid = project_id.trim();
                            if pid.is_empty() || model.projects.contains_key(pid) {
                                return Err(DecideError::new(
                                    ERR_THREADS_INVALID,
                                    "missing projectId for the new project",
                                ));
                            }
                            out.push(DomainEvent::ProjectCreated {
                                project_id: pid.to_string(),
                                title: folder_title(path),
                                workspace_root: path.to_string(),
                                created_at: now.clone(),
                            });
                            (pid.to_string(), path.to_string())
                        }
                    },
                    None => {
                        return Err(DecideError::new(
                            ERR_THREADS_NOT_FOUND,
                            format!("no project {project_id}"),
                        ))
                    }
                },
            };
            let wants_worktree = *worktree && worktree_path.is_none();
            if wants_worktree && root.trim().is_empty() {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    "a worktree needs a project folder",
                ));
            }
            out.push(DomainEvent::ThreadCreated {
                thread_id: id,
                project_id: project,
                title: title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_THREAD_TITLE.to_string()),
                instance_id: instance_id.clone(),
                driver: driver.clone(),
                model: m.clone(),
                agent_id: agent_id.clone(),
                runtime_mode: *runtime_mode,
                interaction_mode: *interaction_mode,
                branch: branch.clone(),
                worktree_path: worktree_path.clone(),
                forked_from: None,
                worktree: wants_worktree.then(|| WorktreeSpec {
                    base_branch: base_branch
                        .clone()
                        .filter(|b| !b.trim().is_empty())
                        .or_else(|| branch.clone()),
                }),
                created_at: now,
            });
            Ok(out)
        }
        TurnStart {
            thread_id,
            turn_id,
            message_id,
            text,
            attachments,
            model: m,
        } => {
            let t = thread(model, thread_id)?;
            let turn_id = required(turn_id, "turnId")?;
            let message_id = required(message_id, "messageId")?;
            if text.trim().is_empty() && attachments.is_empty() {
                return Err(DecideError::new(ERR_THREADS_INVALID, "empty message"));
            }
            if t.external {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    "this thread mirrors an outside session and is read-only; resume it instead",
                ));
            }
            if let Some(active) = &t.active_turn {
                return Err(DecideError::new(
                    ERR_THREADS_BUSY,
                    format!("turn {active} is still running"),
                ));
            }
            let mut out = Vec::new();
            if t.archived {
                out.push(DomainEvent::ThreadUnarchived {
                    thread_id: t.id.clone(),
                });
            }
            if t.snoozed_until.is_some() {
                out.push(DomainEvent::ThreadUnsnoozed {
                    thread_id: t.id.clone(),
                });
            }
            if !t.has_user_message && (t.title.is_empty() || t.title == DEFAULT_THREAD_TITLE) {
                let title = title_from(text);
                if !title.is_empty() {
                    out.push(DomainEvent::ThreadMetaUpdated {
                        thread_id: t.id.clone(),
                        title: Some(title),
                        updated_at: now.clone(),
                    });
                }
            }
            out.push(DomainEvent::MessageSent {
                thread_id: t.id.clone(),
                message_id: message_id.clone(),
                role: "user".into(),
                text: text.clone(),
                turn_id: Some(turn_id.clone()),
                streaming: false,
                attachments: attachments.clone(),
                created_at: now.clone(),
            });
            out.push(DomainEvent::TurnStartRequested {
                thread_id: t.id.clone(),
                turn_id,
                message_id,
                ordinal: t.turn_count + 1,
                text: text.clone(),
                attachments: attachments.clone(),
                model: m.clone().or_else(|| t.model.clone()),
                instance_id: t.instance_id.clone(),
                driver: t.driver.clone(),
                runtime_mode: t.runtime_mode,
                interaction_mode: t.interaction_mode,
                requested_at: now,
            });
            Ok(out)
        }
        TurnInterrupt { thread_id, turn_id } => {
            let t = thread(model, thread_id)?;
            let turn = turn_id.clone().or_else(|| t.active_turn.clone());
            if turn.is_none() {
                return Err(DecideError::new(ERR_THREADS_IDLE, "no turn is running"));
            }
            Ok(vec![DomainEvent::TurnInterruptRequested {
                thread_id: t.id.clone(),
                turn_id: turn,
            }])
        }
        ApprovalRespond {
            thread_id,
            request_id,
            decision,
        } => {
            let t = thread(model, thread_id)?;
            let open = t.open_approvals.get(request_id).ok_or_else(|| {
                DecideError::new(
                    ERR_THREADS_NO_REQUEST,
                    format!("no open approval {request_id}"),
                )
            })?;
            if open.answered {
                return Err(DecideError::new(
                    ERR_THREADS_ANSWERED,
                    format!("approval {request_id} was already answered"),
                ));
            }
            Ok(vec![DomainEvent::ApprovalResponseRequested {
                thread_id: t.id.clone(),
                request_id: request_id.clone(),
                decision: *decision,
            }])
        }
        UserInputRespond {
            thread_id,
            request_id,
            answers,
        } => {
            let t = thread(model, thread_id)?;
            let open = t.open_inputs.get(request_id).ok_or_else(|| {
                DecideError::new(
                    ERR_THREADS_NO_REQUEST,
                    format!("no open question {request_id}"),
                )
            })?;
            if open.answered {
                return Err(DecideError::new(
                    ERR_THREADS_ANSWERED,
                    format!("question {request_id} was already answered"),
                ));
            }
            Ok(vec![DomainEvent::UserInputResponseRequested {
                thread_id: t.id.clone(),
                request_id: request_id.clone(),
                answers: answers.clone(),
            }])
        }
        Archive { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::ThreadArchived {
                thread_id: t.id.clone(),
                archived_at: now,
            }])
        }
        Unarchive { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::ThreadUnarchived {
                thread_id: t.id.clone(),
            }])
        }
        Pin { thread_id } => {
            let t = thread(model, thread_id)?;
            let mut out = Vec::new();
            if t.snoozed_until.is_some() {
                out.push(DomainEvent::ThreadUnsnoozed {
                    thread_id: t.id.clone(),
                });
            }
            out.push(DomainEvent::ThreadPinned {
                thread_id: t.id.clone(),
                pinned_at: now,
            });
            Ok(out)
        }
        Unpin { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::ThreadUnpinned {
                thread_id: t.id.clone(),
            }])
        }
        Snooze { thread_id, until } => {
            let t = thread(model, thread_id)?;
            let wake = chrono::DateTime::parse_from_rfc3339(until)
                .map_err(|e| DecideError::new(ERR_THREADS_INVALID, format!("until: {e}")))?;
            let now_t = chrono::DateTime::parse_from_rfc3339(ctx.now)
                .map_err(|e| DecideError::new(ERR_THREADS_INVALID, format!("now: {e}")))?;
            if wake <= now_t {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    "snooze time is in the past",
                ));
            }
            if !t.open_approvals.is_empty() || !t.open_inputs.is_empty() {
                return Err(DecideError::new(
                    ERR_THREADS_BUSY,
                    "the thread is waiting for an answer",
                ));
            }
            Ok(vec![DomainEvent::ThreadSnoozed {
                thread_id: t.id.clone(),
                snoozed_until: until.clone(),
            }])
        }
        Unsnooze { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::ThreadUnsnoozed {
                thread_id: t.id.clone(),
            }])
        }
        Rename { thread_id, title } => {
            let t = thread(model, thread_id)?;
            let title = title.trim();
            if title.is_empty() {
                return Err(DecideError::new(ERR_THREADS_INVALID, "empty title"));
            }
            Ok(vec![DomainEvent::ThreadMetaUpdated {
                thread_id: t.id.clone(),
                title: Some(title.to_string()),
                updated_at: now,
            }])
        }
        Visit { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::ThreadVisited {
                thread_id: t.id.clone(),
                visited_at: now,
            }])
        }
        ThreadDelete { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::ThreadDeleted {
                thread_id: t.id.clone(),
                deleted_at: now,
            }])
        }
        SetRuntimeMode {
            thread_id,
            runtime_mode,
        } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::RuntimeModeSet {
                thread_id: t.id.clone(),
                runtime_mode: *runtime_mode,
            }])
        }
        SetInteractionMode {
            thread_id,
            interaction_mode,
        } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::InteractionModeSet {
                thread_id: t.id.clone(),
                interaction_mode: *interaction_mode,
            }])
        }
        SetInstance {
            thread_id,
            instance_id,
            driver,
            model: m,
            agent_id,
        } => {
            let t = thread(model, thread_id)?;
            if t.active_turn.is_some() {
                return Err(DecideError::new(
                    ERR_THREADS_BUSY,
                    "wait for the turn to end before switching",
                ));
            }
            Ok(vec![DomainEvent::InstanceSet {
                thread_id: t.id.clone(),
                instance_id: instance_id.clone(),
                driver: driver.clone(),
                model: m.clone(),
                agent_id: agent_id.clone(),
            }])
        }
        Revert {
            thread_id,
            turn_count,
        } => {
            let t = thread(model, thread_id)?;
            if t.active_turn.is_some() {
                return Err(DecideError::new(
                    ERR_THREADS_BUSY,
                    "interrupt the turn before reverting",
                ));
            }
            if *turn_count > t.turn_count {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    format!("the thread has {} turns", t.turn_count),
                ));
            }
            Ok(vec![DomainEvent::Reverted {
                thread_id: t.id.clone(),
                turn_count: *turn_count,
            }])
        }
        Fork {
            thread_id,
            new_thread_id,
            turn_count,
            title,
        } => {
            let src = thread(model, thread_id)?;
            let id = required(new_thread_id, "newThreadId")?;
            if model.threads.contains_key(&id) {
                return Err(DecideError::new(
                    ERR_THREADS_EXISTS,
                    format!("thread {id} exists"),
                ));
            }
            let keep = turn_count.unwrap_or(src.turn_count).min(src.turn_count);
            let source = ctx.fork.cloned().unwrap_or_default();
            let mut out = vec![DomainEvent::ThreadCreated {
                thread_id: id.clone(),
                project_id: src.project_id.clone(),
                title: title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| format!("{} (fork)", src.title)),
                instance_id: src.instance_id.clone(),
                driver: src.driver.clone(),
                model: src.model.clone(),
                agent_id: src.agent_id.clone(),
                runtime_mode: src.runtime_mode,
                interaction_mode: src.interaction_mode,
                branch: None,
                worktree_path: None,
                forked_from: Some(super::model::ForkedFrom {
                    thread_id: src.id.clone(),
                    turn_count: keep,
                }),
                worktree: None,
                created_at: now.clone(),
            }];
            for turn in source.turns.iter().filter(|t| t.ordinal <= keep) {
                let new_turn = format!("{id}:{}", turn.turn_id);
                let first_user = turn
                    .messages
                    .iter()
                    .find(|m| m.1 == "user")
                    .map(|m| format!("{id}:{}", m.0))
                    .unwrap_or_default();
                out.push(DomainEvent::TurnImported {
                    thread_id: id.clone(),
                    turn_id: new_turn.clone(),
                    ordinal: turn.ordinal,
                    message_id: first_user,
                    state: turn.state.clone(),
                    requested_at: turn.requested_at.clone(),
                    completed_at: turn.completed_at.clone(),
                });
                for (mid, role, text, at) in &turn.messages {
                    out.push(DomainEvent::MessageSent {
                        thread_id: id.clone(),
                        message_id: format!("{id}:{mid}"),
                        role: role.clone(),
                        text: text.clone(),
                        turn_id: Some(new_turn.clone()),
                        streaming: false,
                        attachments: Vec::new(),
                        created_at: at.clone(),
                    });
                }
                for a in &turn.activities {
                    let mut a = a.clone();
                    a.activity_id = format!("{id}:{}", a.activity_id);
                    a.turn_id = Some(new_turn.clone());
                    out.push(DomainEvent::ActivityAppended {
                        thread_id: id.clone(),
                        activity: a,
                    });
                }
            }
            Ok(out)
        }
        SessionStop { thread_id } => {
            let t = thread(model, thread_id)?;
            Ok(vec![DomainEvent::SessionStopRequested {
                thread_id: t.id.clone(),
            }])
        }
        RevertToTurn {
            thread_id,
            turn,
            restore_files,
        } => {
            let t = thread(model, thread_id)?;
            if t.active_turn.is_some() {
                return Err(DecideError::new(
                    ERR_THREADS_BUSY,
                    "interrupt the turn before reverting",
                ));
            }
            if *turn > t.turn_count {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    format!("the thread has {} turns", t.turn_count),
                ));
            }
            if *restore_files && t.worktree_state.as_deref() == Some("preparing") {
                return Err(DecideError::new(
                    ERR_THREADS_BUSY,
                    "the worktree is still being set up",
                ));
            }
            Ok(vec![DomainEvent::RevertRequested {
                thread_id: t.id.clone(),
                turn_count: *turn,
                restore_files: *restore_files,
                requested_at: now,
            }])
        }
        HostRecord { event } => {
            if !host_recordable(event) {
                return Err(DecideError::new(
                    ERR_THREADS_INVALID,
                    format!("{} is not a host record", event.type_name()),
                ));
            }
            let (_, id) = event.aggregate();
            thread(model, id)?;
            Ok(vec![event.clone()])
        }
        RuntimeAppend { event } => {
            let t = thread(model, &event.thread_id)?;
            Ok(fold_runtime(t, event))
        }
        HistoryImport {
            thread_id,
            project_id,
            title,
            instance_id,
            driver,
            agent_id,
            created_at,
            messages,
        } => {
            if model.threads.contains_key(thread_id) {
                return Err(DecideError::new(
                    ERR_THREADS_EXISTS,
                    format!("thread {thread_id} exists"),
                ));
            }
            match model.projects.get(project_id) {
                Some(p) if !p.deleted => {}
                _ => {
                    return Err(DecideError::new(
                        ERR_THREADS_NOT_FOUND,
                        format!("no project {project_id}"),
                    ))
                }
            }
            let created = created_at.clone().unwrap_or_else(|| now.clone());
            let mut out = vec![DomainEvent::ThreadCreated {
                thread_id: thread_id.clone(),
                project_id: project_id.clone(),
                title: if title.trim().is_empty() {
                    DEFAULT_THREAD_TITLE.to_string()
                } else {
                    title.clone()
                },
                instance_id: instance_id.clone(),
                driver: driver.clone(),
                model: None,
                agent_id: agent_id.clone(),
                runtime_mode: Default::default(),
                interaction_mode: Default::default(),
                branch: None,
                worktree_path: None,
                forked_from: None,
                worktree: None,
                created_at: created.clone(),
            }];
            let mut ordinal = 0u32;
            let mut turn: Option<String> = None;
            for (i, m) in messages.iter().enumerate() {
                let at = m.created_at.clone().unwrap_or_else(|| created.clone());
                let mid = m
                    .message_id
                    .clone()
                    .unwrap_or_else(|| format!("{thread_id}:m{i}"));
                if m.role == "user" {
                    ordinal += 1;
                    let tid = format!("{thread_id}:t{ordinal}");
                    out.push(DomainEvent::TurnImported {
                        thread_id: thread_id.clone(),
                        turn_id: tid.clone(),
                        ordinal,
                        message_id: mid.clone(),
                        state: "completed".into(),
                        requested_at: at.clone(),
                        completed_at: Some(at.clone()),
                    });
                    turn = Some(tid);
                }
                if !m.text.is_empty() {
                    out.push(DomainEvent::MessageSent {
                        thread_id: thread_id.clone(),
                        message_id: mid.clone(),
                        role: m.role.clone(),
                        text: m.text.clone(),
                        turn_id: turn.clone(),
                        streaming: false,
                        attachments: Vec::new(),
                        created_at: at.clone(),
                    });
                }
                for (k, tool) in m.tools.iter().enumerate() {
                    let name = tool["name"].as_str().unwrap_or("tool").to_string();
                    let failed = tool["isError"].as_bool().unwrap_or(false);
                    let mut item = ItemPayload::new(item_type_of_tool(&name));
                    item.status = Some(if failed {
                        ItemStatus::Failed
                    } else {
                        ItemStatus::Completed
                    });
                    item.title = Some(name.clone());
                    item.tool_name = Some(name.clone());
                    item.data = Some(json!({
                        "input": tool.get("input").cloned().unwrap_or(Value::Null),
                        "output": tool.get("output").cloned().unwrap_or(Value::Null),
                    }));
                    let id = tool["id"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{mid}:tool{k}"));
                    out.push(DomainEvent::ActivityAppended {
                        thread_id: thread_id.clone(),
                        activity: Activity {
                            activity_id: format!("{thread_id}:item:{id}"),
                            turn_id: turn.clone(),
                            kind: "item.completed".into(),
                            tone: "tool".into(),
                            summary: name,
                            payload: serde_json::to_value(&item).unwrap_or(Value::Null),
                            created_at: at.clone(),
                        },
                    });
                }
            }
            Ok(out)
        }
    }
}

/// Which canonical item a tool name is, for tools whose names we know
/// (the native harness and the common CLI tool names).
pub fn item_type_of_tool(name: &str) -> ItemType {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("mcp:") || lower.starts_with("mcp__") {
        ItemType::McpToolCall
    } else if matches!(
        lower.as_str(),
        "shell_exec" | "bash" | "shell" | "exec_command" | "local_shell"
    ) {
        ItemType::CommandExecution
    } else if lower.starts_with("fs_write")
        || lower.starts_with("fs_edit")
        || lower.starts_with("fs_patch")
        || matches!(
            lower.as_str(),
            "edit" | "write" | "multiedit" | "apply_patch" | "notebookedit"
        )
    {
        ItemType::FileChange
    } else if lower.contains("web_search") || lower == "websearch" {
        ItemType::WebSearch
    } else if lower == "task" || lower == "agent" {
        ItemType::CollabAgentToolCall
    } else {
        ItemType::DynamicToolCall
    }
}

fn tone_of_item(item: &ItemPayload) -> &'static str {
    match item.item_type {
        ItemType::Error => "error",
        ItemType::AssistantMessage
        | ItemType::UserMessage
        | ItemType::Reasoning
        | ItemType::Plan
        | ItemType::ContextCompaction
        | ItemType::ReviewEntered
        | ItemType::ReviewExited => "info",
        _ => "tool",
    }
}

fn scope_of(ev: &RuntimeEvent) -> String {
    ev.turn_id.clone().unwrap_or_else(|| ev.thread_id.clone())
}

fn end_state(state: TurnEndState) -> &'static str {
    match state {
        TurnEndState::Completed => "completed",
        TurnEndState::Failed => "failed",
        TurnEndState::Interrupted => "interrupted",
        TurnEndState::Cancelled => "cancelled",
    }
}

/// Every approval and question still open for this turn becomes `stale`:
/// the harness that asked is gone.
fn close_stale(t: &ThreadState, turn_id: Option<&str>, now: &str) -> Vec<DomainEvent> {
    let mut out = Vec::new();
    let mut approvals: Vec<&String> = t
        .open_approvals
        .iter()
        .filter(|(_, r)| turn_id.is_none() || r.turn_id.as_deref() == turn_id)
        .map(|(k, _)| k)
        .collect();
    approvals.sort();
    for id in approvals {
        out.push(DomainEvent::ApprovalResolved {
            thread_id: t.id.clone(),
            request_id: id.clone(),
            decision: None,
            resolution: Some("stale".into()),
            resolved_at: now.to_string(),
        });
    }
    let mut inputs: Vec<&String> = t
        .open_inputs
        .iter()
        .filter(|(_, r)| turn_id.is_none() || r.turn_id.as_deref() == turn_id)
        .map(|(k, _)| k)
        .collect();
    inputs.sort();
    for id in inputs {
        out.push(DomainEvent::UserInputResolved {
            thread_id: t.id.clone(),
            request_id: id.clone(),
            answers: None,
            resolution: Some("stale".into()),
            resolved_at: now.to_string(),
        });
    }
    out
}

fn activity(ev: &RuntimeEvent, id: String, tone: &str, summary: String) -> DomainEvent {
    let payload = serde_json::to_value(&ev.kind)
        .ok()
        .and_then(|v| v.get("payload").cloned())
        .unwrap_or(Value::Null);
    let mut payload = match payload {
        Value::Object(m) => Value::Object(m),
        other => json!({ "value": other }),
    };
    if let Some(item) = &ev.item_id {
        payload["itemId"] = json!(item);
    }
    if let Some(req) = &ev.request_id {
        payload["requestId"] = json!(req);
    }
    DomainEvent::ActivityAppended {
        thread_id: ev.thread_id.clone(),
        activity: Activity {
            activity_id: id,
            turn_id: ev.turn_id.clone(),
            kind: ev.type_name().to_string(),
            tone: tone.to_string(),
            summary,
            payload,
            created_at: ev.created_at.clone(),
        },
    }
}

/// Runtime event → domain events (T3 `ProviderRuntimeIngestion`, folded).
pub fn fold_runtime(t: &ThreadState, ev: &RuntimeEvent) -> Vec<DomainEvent> {
    use RuntimeEventKind as K;
    let now = ev.created_at.clone();
    let tid = t.id.clone();
    let scope = scope_of(ev);
    match &ev.kind {
        K::ContentDelta(p) => {
            if p.delta.is_empty() {
                return Vec::new();
            }
            let (role, suffix) = match p.stream_kind {
                StreamKind::AssistantText | StreamKind::Unknown => ("assistant", "assistant"),
                StreamKind::ReasoningText | StreamKind::ReasoningSummaryText => {
                    ("reasoning", "reasoning")
                }
                StreamKind::PlanText => ("plan", "plan"),
                StreamKind::CommandOutput | StreamKind::FileChangeOutput => ("output", "output"),
            };
            let message_id = match &ev.item_id {
                Some(item) => format!("{scope}:{item}"),
                None => format!("{scope}:{suffix}"),
            };
            vec![DomainEvent::MessageSent {
                thread_id: tid,
                message_id,
                role: role.into(),
                text: p.delta.clone(),
                turn_id: ev.turn_id.clone(),
                streaming: true,
                attachments: Vec::new(),
                created_at: now,
            }]
        }
        K::ProposedDelta(p) => vec![DomainEvent::MessageSent {
            thread_id: tid,
            message_id: format!("{scope}:proposal"),
            role: "plan".into(),
            text: p.delta.clone(),
            turn_id: ev.turn_id.clone(),
            streaming: true,
            attachments: Vec::new(),
            created_at: now,
        }],
        K::ItemStarted(item) | K::ItemUpdated(item) | K::ItemCompleted(item) => {
            let completed = matches!(ev.kind, K::ItemCompleted(_));
            match item.item_type {
                ItemType::UserMessage => Vec::new(),
                ItemType::AssistantMessage | ItemType::Reasoning => {
                    // Text arrives as `content.delta`; `completed` may carry
                    // the final text in `detail`, which then wins.
                    if !completed {
                        return Vec::new();
                    }
                    let role = if item.item_type == ItemType::Reasoning {
                        "reasoning"
                    } else {
                        "assistant"
                    };
                    let message_id = match &ev.item_id {
                        Some(i) => format!("{scope}:{i}"),
                        None => format!("{scope}:{role}"),
                    };
                    vec![DomainEvent::MessageSent {
                        thread_id: tid,
                        message_id,
                        role: role.into(),
                        text: item.detail.clone().unwrap_or_default(),
                        turn_id: ev.turn_id.clone(),
                        streaming: false,
                        attachments: Vec::new(),
                        created_at: now,
                    }]
                }
                _ => {
                    let id = match &ev.item_id {
                        Some(i) => format!("{scope}:item:{i}"),
                        None => ev.event_id.clone(),
                    };
                    let summary = item
                        .title
                        .clone()
                        .or_else(|| item.tool_name.clone())
                        .unwrap_or_else(|| {
                            serde_json::to_value(item.item_type)
                                .ok()
                                .and_then(|v| v.as_str().map(str::to_string))
                                .unwrap_or_default()
                        });
                    vec![activity(ev, id, tone_of_item(item), summary)]
                }
            }
        }
        K::TurnStarted(p) => match &ev.turn_id {
            Some(turn) => vec![DomainEvent::TurnStarted {
                thread_id: tid,
                turn_id: turn.clone(),
                model: p.model.clone(),
                started_at: now,
            }],
            None => Vec::new(),
        },
        K::TurnCompleted(p) => {
            let Some(turn) = ev.turn_id.clone() else {
                return Vec::new();
            };
            let mut out = vec![DomainEvent::TurnCompleted {
                thread_id: tid.clone(),
                turn_id: turn.clone(),
                state: end_state(p.state).into(),
                error_message: p.error_message.clone(),
                completed_at: now.clone(),
            }];
            if p.usage.is_some() || p.total_cost_usd.is_some() {
                let mut usage = p.usage.clone().unwrap_or_default();
                if usage.cost_usd.is_none() {
                    usage.cost_usd = p.total_cost_usd;
                }
                out.push(DomainEvent::TurnUsage {
                    thread_id: tid,
                    turn_id: turn.clone(),
                    usage,
                });
            }
            out.extend(close_stale(t, Some(&turn), &now));
            out
        }
        K::TurnAborted(p) => {
            let Some(turn) = ev.turn_id.clone() else {
                return Vec::new();
            };
            let mut out = vec![DomainEvent::TurnCompleted {
                thread_id: tid.clone(),
                turn_id: turn.clone(),
                state: "interrupted".into(),
                error_message: Some(p.reason.clone()),
                completed_at: now.clone(),
            }];
            if let Some(usage) = &p.usage {
                out.push(DomainEvent::TurnUsage {
                    thread_id: tid,
                    turn_id: turn.clone(),
                    usage: usage.clone(),
                });
            }
            out.extend(close_stale(t, Some(&turn), &now));
            out
        }
        K::UsageUpdated(p) => match &ev.turn_id {
            Some(turn) => vec![DomainEvent::TurnUsage {
                thread_id: tid,
                turn_id: turn.clone(),
                usage: p.usage.clone(),
            }],
            None => vec![activity(ev, format!("{tid}:usage"), "info", "usage".into())],
        },
        K::SessionStarted(p) => vec![DomainEvent::SessionSet {
            thread_id: tid,
            status: Some(SessionState::Ready),
            last_error: None,
            resume_cursor: p.resume.clone(),
            provider_thread_id: None,
            updated_at: now,
        }],
        K::SessionStateChanged(p) => vec![DomainEvent::SessionSet {
            thread_id: tid,
            status: Some(p.state),
            last_error: if p.state == SessionState::Error {
                p.detail.clone().or_else(|| p.reason.clone())
            } else {
                None
            },
            resume_cursor: None,
            provider_thread_id: None,
            updated_at: now,
        }],
        K::SessionExited(p) => {
            let failed = p.exit_kind.as_deref() == Some("error");
            let mut out = vec![DomainEvent::SessionSet {
                thread_id: tid.clone(),
                status: Some(if failed {
                    SessionState::Error
                } else {
                    SessionState::Stopped
                }),
                last_error: if failed { p.reason.clone() } else { None },
                resume_cursor: None,
                provider_thread_id: None,
                updated_at: now.clone(),
            }];
            if let Some(active) = &t.active_turn {
                out.push(DomainEvent::TurnCompleted {
                    thread_id: tid,
                    turn_id: active.clone(),
                    state: "interrupted".into(),
                    error_message: Some(
                        p.reason.clone().unwrap_or_else(|| "session exited".into()),
                    ),
                    completed_at: now.clone(),
                });
                out.extend(close_stale(t, Some(active), &now));
            }
            out
        }
        K::ThreadStarted(p) => vec![DomainEvent::SessionSet {
            thread_id: tid,
            status: None,
            last_error: None,
            resume_cursor: p.provider_thread_id.as_ref().map(|s| json!(s)),
            provider_thread_id: p.provider_thread_id.clone(),
            updated_at: now,
        }],
        K::RequestOpened(p) => {
            let request_id = ev.request_id.clone().unwrap_or_else(|| ev.event_id.clone());
            if t.open_approvals.contains_key(&request_id) {
                return Vec::new();
            }
            vec![DomainEvent::ApprovalRequested {
                thread_id: tid,
                turn_id: ev.turn_id.clone(),
                request_id,
                request_type: p.request_type,
                detail: p.detail.clone(),
                options: p.options.clone(),
                created_at: now,
            }]
        }
        K::RequestResolved(p) => {
            let Some(request_id) = ev.request_id.clone() else {
                return Vec::new();
            };
            if !t.open_approvals.contains_key(&request_id) {
                return Vec::new();
            }
            vec![DomainEvent::ApprovalResolved {
                thread_id: tid,
                request_id,
                decision: p.decision,
                resolution: p.resolution.clone(),
                resolved_at: now,
            }]
        }
        K::UserInputRequested(p) => {
            let request_id = ev.request_id.clone().unwrap_or_else(|| ev.event_id.clone());
            if t.open_inputs.contains_key(&request_id) {
                return Vec::new();
            }
            vec![DomainEvent::UserInputRequested {
                thread_id: tid,
                turn_id: ev.turn_id.clone(),
                request_id,
                questions: p.questions.clone(),
                response_mode: p.response_mode.clone(),
                created_at: now,
            }]
        }
        K::UserInputResolved(p) => {
            let Some(request_id) = ev.request_id.clone() else {
                return Vec::new();
            };
            if !t.open_inputs.contains_key(&request_id) {
                return Vec::new();
            }
            vec![DomainEvent::UserInputResolved {
                thread_id: tid,
                request_id,
                answers: Some(p.answers.clone()),
                resolution: None,
                resolved_at: now,
            }]
        }
        K::PlanUpdated(p) => vec![DomainEvent::PlanUpdated {
            thread_id: tid,
            plan_id: format!("{scope}:plan"),
            turn_id: ev.turn_id.clone(),
            explanation: p.explanation.clone(),
            steps: Some(p.plan.clone()),
            markdown: None,
            updated_at: now,
        }],
        K::ProposedCompleted(p) => vec![DomainEvent::PlanUpdated {
            thread_id: tid,
            plan_id: format!("{scope}:plan"),
            turn_id: ev.turn_id.clone(),
            explanation: None,
            steps: None,
            markdown: Some(p.plan_markdown.clone()),
            updated_at: now,
        }],
        // Not worth a row: the bytes are large and transient.
        K::Raw(_) | K::RealtimeAudioDelta(_) => Vec::new(),
        K::DiffUpdated(_) => vec![activity(ev, format!("{scope}:diff"), "info", "diff".into())],
        K::RuntimeError(p) => vec![activity(
            ev,
            ev.event_id.clone(),
            "error",
            p.message.clone(),
        )],
        K::RuntimeWarning(p) => vec![activity(
            ev,
            ev.event_id.clone(),
            "warning",
            p.message.clone(),
        )],
        K::ConfigWarning(p) | K::DeprecationNotice(p) => vec![activity(
            ev,
            ev.event_id.clone(),
            "warning",
            p.summary.clone(),
        )],
        K::ModelRerouted(p) => vec![activity(
            ev,
            ev.event_id.clone(),
            "info",
            format!("{} → {}", p.from_model, p.to_model),
        )],
        K::TaskStarted(p) | K::TaskProgress(p) | K::TaskUpdated(p) | K::TaskCompleted(p) => {
            let summary = p
                .title
                .clone()
                .or_else(|| p.description.clone())
                .or_else(|| p.summary.clone())
                .unwrap_or_else(|| p.task_id.clone());
            vec![activity(
                ev,
                format!("{scope}:task:{}", p.task_id),
                "info",
                summary,
            )]
        }
        K::HookStarted(p) | K::HookProgress(p) | K::HookCompleted(p) => {
            let summary = p.hook_name.clone().unwrap_or_else(|| p.hook_id.clone());
            vec![activity(
                ev,
                format!("{scope}:hook:{}", p.hook_id),
                "info",
                summary,
            )]
        }
        K::ToolProgress(p) => {
            let id = match &p.tool_use_id {
                Some(u) => format!("{scope}:progress:{u}"),
                None => ev.event_id.clone(),
            };
            vec![activity(
                ev,
                id,
                "tool",
                p.summary
                    .clone()
                    .or_else(|| p.tool_name.clone())
                    .unwrap_or_default(),
            )]
        }
        K::ToolSummary(p) => vec![activity(ev, ev.event_id.clone(), "tool", p.summary.clone())],
        K::ToolDenied(p) => vec![activity(
            ev,
            ev.event_id.clone(),
            "warning",
            p.tool_name.clone(),
        )],
        K::RateLimitsUpdated(_) => vec![activity(
            ev,
            format!("{tid}:rate-limits"),
            "info",
            "rate limits".into(),
        )],
        K::AuthStatus(_) => vec![activity(ev, format!("{tid}:auth"), "info", "auth".into())],
        K::McpStatusUpdated(_) => vec![activity(
            ev,
            format!("{tid}:mcp-status"),
            "info",
            "mcp".into(),
        )],
        K::ThreadMetadataUpdated(p) => vec![activity(
            ev,
            format!("{tid}:metadata"),
            "info",
            p.name.clone().unwrap_or_default(),
        )],
        K::SessionConfigured(_)
        | K::ThreadStateChanged(_)
        | K::RealtimeStarted(_)
        | K::RealtimeItemAdded(_)
        | K::RealtimeError(_)
        | K::RealtimeClosed(_)
        | K::AccountUpdated(_)
        | K::McpOauthCompleted(_)
        | K::FilesPersisted(_) => vec![activity(
            ev,
            ev.event_id.clone(),
            "info",
            ev.type_name().to_string(),
        )],
    }
}

/// Canonical request type of a native tool approval.
pub fn request_type_of_tool(name: &str) -> RequestType {
    match item_type_of_tool(name) {
        ItemType::CommandExecution => RequestType::CommandExecutionApproval,
        ItemType::FileChange => RequestType::FileChangeApproval,
        _ if name.starts_with("fs_") => RequestType::FileReadApproval,
        _ => RequestType::DynamicToolCall,
    }
}
