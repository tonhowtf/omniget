//! The engine: one writer thread owns the write connection and the light
//! read model, and processes commands strictly one at a time (T3's serialized
//! queue). Each command is normalized (ids, clock), checked against its
//! receipt, decided, then appended + projected + receipted in ONE
//! transaction; subscribers hear the events only after the commit.
//! Readers use their own connection (WAL), so a snapshot never waits for a
//! turn that is streaming.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::decider::{decide, DecideCtx, ForkSource};
use super::model::{ActorKind, Command, CommandEnvelope, ReadModel, StoredEvent};
use super::store::{self, db_err, EventsPage, Planned, Receipt, Snapshot, TurnsPage};

pub const ERR_THREADS_ENGINE: &str = "ERR_THREADS_ENGINE";
const BROADCAST_CAP: usize = 4096;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchResult {
    pub command_id: String,
    /// Sequence of the last event the command produced (or of the head, for
    /// a no-op).
    pub sequence: i64,
    pub events: Vec<StoredEvent>,
    /// The command id had an accepted receipt already: nothing was redone.
    pub deduplicated: bool,
}

enum Job {
    Dispatch {
        env: CommandEnvelope,
        reply: oneshot::Sender<Result<DispatchResult, String>>,
    },
}

pub struct ThreadsEngine {
    path: PathBuf,
    jobs: mpsc::UnboundedSender<Job>,
    events: broadcast::Sender<StoredEvent>,
    reader: Mutex<Connection>,
}

impl std::fmt::Debug for ThreadsEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadsEngine")
            .field("path", &self.path)
            .finish()
    }
}

fn short(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..16]
    )
}

/// Fill what the client may omit: ids and the command id. Pure apart from
/// the random ids.
pub fn normalize(env: CommandEnvelope) -> (String, Command) {
    use Command::*;
    let command_id = env
        .command_id
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| short("cmd_"));
    let mut cmd = env.command;
    match &mut cmd {
        ProjectCreate { project_id, .. } => {
            if project_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                *project_id = Some(short("prj_"));
            }
        }
        ThreadCreate {
            thread_id,
            project_id,
            project_path,
            ..
        } => {
            if thread_id.as_deref().map(str::trim).unwrap_or("").is_empty() {
                *thread_id = Some(short("thr_"));
            }
            // A path without a project: the id the project gets if the
            // decider has to create it.
            if project_id.trim().is_empty()
                && project_path.as_deref().map(str::trim).unwrap_or("") != ""
            {
                *project_id = short("prj_");
            }
        }
        TurnStart {
            turn_id,
            message_id,
            ..
        } => {
            if turn_id.as_deref().map(str::trim).unwrap_or("").is_empty() {
                *turn_id = Some(short("turn_"));
            }
            if message_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                *message_id = Some(short("msg_"));
            }
        }
        Fork { new_thread_id, .. } => {
            if new_thread_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                *new_thread_id = Some(short("thr_"));
            }
        }
        _ => {}
    }
    (command_id, cmd)
}

struct Writer {
    conn: Connection,
    model: ReadModel,
    events: broadcast::Sender<StoredEvent>,
}

impl Writer {
    fn process(&mut self, env: CommandEnvelope) -> Result<DispatchResult, String> {
        let explicit_id = env.command_id.is_some();
        let (command_id, cmd) = normalize(env);
        let (agg_kind, agg_id) = cmd.aggregate();
        if let Some(r) = store::receipt(&self.conn, &command_id)? {
            if r.aggregate_kind != agg_kind.as_str() || r.aggregate_id != agg_id {
                return Err(format!(
                    "ERR_THREADS_COMMAND_CONFLICT: command {command_id} was used for another aggregate"
                ));
            }
            if r.status == "accepted" {
                return Ok(DispatchResult {
                    command_id,
                    sequence: r.result_sequence,
                    events: Vec::new(),
                    deduplicated: true,
                });
            }
            return Err(r.error.unwrap_or_else(|| {
                format!("ERR_THREADS_REJECTED: command {command_id} was rejected before")
            }));
        }

        let now = crate::core::llm::drivers::now_iso();
        let fork: Option<ForkSource> = match &cmd {
            Command::Fork {
                thread_id,
                turn_count,
                ..
            } => {
                let keep = turn_count.unwrap_or(u32::MAX);
                Some(store::fork_source(&self.conn, thread_id, keep)?)
            }
            _ => None,
        };
        let ctx = DecideCtx {
            now: &now,
            fork: fork.as_ref(),
        };
        let planned = match decide(&self.model, &cmd, &ctx) {
            Ok(p) => p,
            Err(e) => {
                let msg = e.to_string();
                if explicit_id {
                    let _ = store::put_receipt(
                        &self.conn,
                        &Receipt {
                            command_id: command_id.clone(),
                            aggregate_kind: agg_kind.as_str().into(),
                            aggregate_id: agg_id.clone(),
                            accepted_at: now.clone(),
                            result_sequence: 0,
                            status: "rejected".into(),
                            error: Some(msg.clone()),
                        },
                    );
                }
                return Err(msg);
            }
        };
        if planned.is_empty() {
            let head = store::head(&self.conn)?;
            if explicit_id {
                store::put_receipt(
                    &self.conn,
                    &Receipt {
                        command_id: command_id.clone(),
                        aggregate_kind: agg_kind.as_str().into(),
                        aggregate_id: agg_id,
                        accepted_at: now,
                        result_sequence: head,
                        status: "accepted".into(),
                        error: None,
                    },
                )?;
            }
            return Ok(DispatchResult {
                command_id,
                sequence: head,
                events: Vec::new(),
                deduplicated: false,
            });
        }

        let actor = ActorKind::of_command(&command_id);
        let tx = self.conn.transaction().map_err(db_err)?;
        let mut stored: Vec<StoredEvent> = Vec::with_capacity(planned.len());
        let mut causation: Option<String> = None;
        for event in planned {
            let event_id = uuid::Uuid::new_v4().to_string();
            let p = Planned {
                event,
                event_id: event_id.clone(),
                occurred_at: now.clone(),
                command_id: Some(command_id.clone()),
                causation_id: causation.clone(),
                correlation_id: Some(command_id.clone()),
                actor_kind: actor,
                metadata: serde_json::json!({ "command": cmd.type_name() }),
            };
            let saved = store::append(&tx, &p)?;
            for projector in store::PROJECTORS {
                store::project(&tx, projector, &saved)?;
            }
            causation = Some(event_id);
            stored.push(saved);
        }
        let last = stored.last().map(|e| e.sequence).unwrap_or(0);
        store::set_cursors(&tx, store::PROJECTORS, last)?;
        store::put_receipt(
            &tx,
            &Receipt {
                command_id: command_id.clone(),
                aggregate_kind: agg_kind.as_str().into(),
                aggregate_id: agg_id,
                accepted_at: now,
                result_sequence: last,
                status: "accepted".into(),
                error: None,
            },
        )?;
        tx.commit().map_err(db_err)?;

        for e in &stored {
            self.model.apply(e);
            let _ = self.events.send(e.clone());
        }
        Ok(DispatchResult {
            command_id,
            sequence: last,
            events: stored,
            deduplicated: false,
        })
    }
}

impl ThreadsEngine {
    /// Open the database, catch the projectors up, load the read model and
    /// start the writer thread.
    pub fn open(path: &Path) -> Result<Arc<Self>, String> {
        let mut conn = store::open(path)?;
        let replayed = store::bootstrap_projectors(&mut conn)?;
        if replayed > 0 {
            tracing::info!("[threads] projectors caught up on {replayed} event(s)");
        }
        let model = store::load_read_model(&conn)?;
        let reader = store::open_reader(path)?;
        let (events, _) = broadcast::channel(BROADCAST_CAP);
        let (jobs, mut rx) = mpsc::unbounded_channel::<Job>();
        let mut writer = Writer {
            conn,
            model,
            events: events.clone(),
        };
        std::thread::Builder::new()
            .name("threads-writer".into())
            .spawn(move || {
                while let Some(job) = rx.blocking_recv() {
                    match job {
                        Job::Dispatch { env, reply } => {
                            let result = writer.process(env);
                            if result.is_err() {
                                // A failed transaction rolled back; the model
                                // was not touched, but a storage error may have
                                // left it behind the database.
                                if let Ok(m) = store::load_read_model(&writer.conn) {
                                    if m.sequence != writer.model.sequence {
                                        writer.model = m;
                                    }
                                }
                            }
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .map_err(|e| format!("{ERR_THREADS_ENGINE}: {e}"))?;
        Ok(Arc::new(Self {
            path: path.to_path_buf(),
            jobs,
            events,
            reader: Mutex::new(reader),
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every committed event, in sequence order. A lagging receiver gets
    /// `RecvError::Lagged` and should resume with `events_after`.
    pub fn subscribe(&self) -> broadcast::Receiver<StoredEvent> {
        self.events.subscribe()
    }

    fn send(
        &self,
        env: CommandEnvelope,
    ) -> Result<oneshot::Receiver<Result<DispatchResult, String>>, String> {
        let (reply, rx) = oneshot::channel();
        self.jobs
            .send(Job::Dispatch { env, reply })
            .map_err(|_| format!("{ERR_THREADS_ENGINE}: writer stopped"))?;
        Ok(rx)
    }

    pub async fn dispatch(&self, env: CommandEnvelope) -> Result<DispatchResult, String> {
        self.send(env)?
            .await
            .map_err(|_| format!("{ERR_THREADS_ENGINE}: writer dropped the reply"))?
    }

    /// Same, for a caller outside the async runtime (tests, the migration).
    pub fn dispatch_blocking(&self, env: CommandEnvelope) -> Result<DispatchResult, String> {
        self.send(env)?
            .blocking_recv()
            .map_err(|_| format!("{ERR_THREADS_ENGINE}: writer dropped the reply"))?
    }

    /// Run a read on the reader connection (blocking; wrap in
    /// `spawn_blocking` from async code, or use the async helpers below).
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let conn = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn)
    }

    pub fn snapshot_blocking(&self) -> Result<Snapshot, String> {
        self.read(store::snapshot)
    }

    pub fn events_after_blocking(&self, after: i64, limit: i64) -> Result<EventsPage, String> {
        self.read(|c| store::events_after(c, after, limit))
    }

    pub fn turns_page_blocking(
        &self,
        thread_id: &str,
        before_turn: Option<u32>,
        limit: u32,
    ) -> Result<TurnsPage, String> {
        self.read(|c| store::turns_page(c, thread_id, before_turn, limit))
    }

    pub async fn snapshot(self: &Arc<Self>) -> Result<Snapshot, String> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.snapshot_blocking())
            .await
            .map_err(|e| format!("{ERR_THREADS_ENGINE}: {e}"))?
    }

    pub async fn events_after(
        self: &Arc<Self>,
        after: i64,
        limit: i64,
    ) -> Result<EventsPage, String> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.events_after_blocking(after, limit))
            .await
            .map_err(|e| format!("{ERR_THREADS_ENGINE}: {e}"))?
    }

    pub async fn turns_page(
        self: &Arc<Self>,
        thread_id: String,
        before_turn: Option<u32>,
        limit: u32,
    ) -> Result<TurnsPage, String> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.turns_page_blocking(&thread_id, before_turn, limit)
        })
        .await
        .map_err(|e| format!("{ERR_THREADS_ENGINE}: {e}"))?
    }
}
