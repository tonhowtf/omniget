//! The one gate every background job passes before it starts (roadmap 1.5
//! and 1.6): a slot of the machine, the agent's own cap, and its folder.
//!
//! Why one gate and not three locks: a job that took a slot and then waited
//! for its folder would sit on capacity doing nothing (W1 gets worse), and
//! one that took the folder and then waited for a slot would block the
//! folder for nobody. So admission is one step, under one mutex: a waiter is
//! let in only when a slot is free, its agent is under its cap and no running
//! job holds an overlapping folder ([`FolderKey::overlaps`]: `/repo` blocks
//! `/repo/sub` and back). The [`JobLease`] it gets back gives all three back
//! on `Drop`. This is the roadmap's `acquire_workspace -> WorkspaceLease`,
//! merged with the slot.
//!
//! The lock is per job, not per tool, so it holds for `claude -p`, `codex
//! exec` and ACP agents, which edit files with their own tools and never pass
//! the broker's write lock (W4). Chat turns do not come here: a conversation
//! is not a second writer, and a chat waiting on a background job of the
//! same conversation would deadlock (multica, idea only).
//!
//! Order: jobs a person started (run, Loop round, trigger) before mission
//! tasks, first come first served inside each class (symphony P5: a mission
//! must not block a manual job when a slot frees; it still never passes the
//! cap). Every job left waiting has a real reason and a position, which the
//! jobs page, `GET /v1/state` and `omniget agent status` show (W7).
//!
//! The cap can change while jobs run. The slots are a `tokio` semaphore:
//! growing adds permits (after paying any debt), shrinking forgets the free
//! ones and records the rest as owed, paid by forgetting permits as jobs
//! end. Free + in use − owed is always the cap; no permit is lost or made up.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};
// The tokio clock is the system clock outside tests; a test with a paused
// runtime drives the stagger in virtual time.
use tokio::time::Instant;

use super::folder_key::FolderKey;

/// Time between two job spawns (orca: "at most one terminal per tick"): N
/// `claude` processes starting in the same instant fight for CPU, disk and
/// the account's rate limit for nothing.
pub const STAGGER: Duration = Duration::from_millis(500);
/// Bounds of the parallel cap (`jobs_max_parallel`).
pub const MAX_PARALLEL_RANGE: (usize, usize) = (1, 16);

/// Who waits first. A job a person started is served before a mission task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Manual,
    Mission,
}

/// What a job asks the gate for.
#[derive(Debug, Clone)]
pub struct Ticket {
    pub job_id: String,
    pub agent_id: String,
    pub class: Class,
    /// The folder the job may write in; `None` = no project folder (a
    /// personal job runs in the agent's private sandbox), no folder lock.
    pub folder: Option<FolderKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_parallel: usize,
    /// Jobs of one agent at the same time; `None` = only the global cap.
    pub max_per_agent: Option<usize>,
    pub stagger: Duration,
}

impl Limits {
    fn clamped(mut self) -> Self {
        let (lo, hi) = MAX_PARALLEL_RANGE;
        self.max_parallel = self.max_parallel.clamp(lo, hi);
        self.max_per_agent = self.max_per_agent.map(|n| n.clamp(lo, hi));
        self
    }
}

/// Why a job is still waiting, the first barrier that holds in this order:
/// its folder (the most specific, and the one that outlasts a free slot),
/// its agent's cap, a slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    Slot,
    AgentCap {
        cap: usize,
    },
    /// `holder` is the running job whose folder overlaps this one's.
    Workspace {
        holder: String,
        folder: FolderKey,
    },
}

impl WaitReason {
    /// `slot` | `agent_cap` | `workspace`: what the UI translates.
    pub fn code(&self) -> &'static str {
        match self {
            WaitReason::Slot => "slot",
            WaitReason::AgentCap { .. } => "agent_cap",
            WaitReason::Workspace { .. } => "workspace",
        }
    }
}

/// A job admitted and not finished yet.
#[derive(Debug, Clone)]
pub struct Held {
    pub job_id: String,
    pub agent_id: String,
    pub class: Class,
    pub folder: Option<FolderKey>,
}

/// A job still waiting, where it stands and why.
#[derive(Debug, Clone)]
pub struct Queued {
    pub job_id: String,
    pub agent_id: String,
    pub class: Class,
    pub folder: Option<FolderKey>,
    /// 1-based, in the order the gate would let them in.
    pub position: usize,
    pub reason: WaitReason,
}

/// The gate at one instant.
#[derive(Debug, Clone)]
pub struct GateView {
    pub held: Vec<Held>,
    pub queued: Vec<Queued>,
    pub limits: Limits,
    /// Free slots right now.
    pub available: usize,
}

struct Waiter {
    seq: u64,
    ticket: Ticket,
    tx: oneshot::Sender<JobLease>,
}

struct Inner {
    limits: Limits,
    /// Permits to forget as jobs end (the cap shrank while they ran).
    debt: usize,
    next_spawn: Option<Instant>,
    seq: u64,
    waiters: Vec<Waiter>,
    /// By lease id: the earliest holder is named first in a reason.
    held: BTreeMap<u64, Held>,
}

pub struct Gate {
    inner: Mutex<Inner>,
    slots: Arc<Semaphore>,
    changed: watch::Sender<u64>,
}

/// A job's right to run: a slot, its agent's share and its folder. Dropping
/// it gives all three back and lets the next waiter in.
pub struct JobLease {
    gate: Arc<Gate>,
    id: u64,
    permit: Option<OwnedSemaphorePermit>,
    /// How long to wait before spawning (the stagger); the slot is already
    /// this job's.
    pub delay: Duration,
}

impl Drop for JobLease {
    fn drop(&mut self) {
        self.gate.release(self.id, self.permit.take());
    }
}

/// Removes a waiter whose `acquire` was dropped (the job was cancelled while
/// it waited), so it never shows as queued nor receives a slot.
struct Leave {
    gate: Arc<Gate>,
    seq: u64,
}

impl Drop for Leave {
    fn drop(&mut self) {
        let sends = {
            let mut g = self.gate.lock();
            let before = g.waiters.len();
            g.waiters.retain(|w| w.seq != self.seq);
            if g.waiters.len() == before {
                return;
            }
            self.gate.admit(&mut g)
        };
        self.gate.deliver(sends);
    }
}

type Sends = Vec<(oneshot::Sender<JobLease>, JobLease)>;

impl Gate {
    pub fn new(limits: Limits) -> Arc<Gate> {
        let limits = limits.clamped();
        Arc::new(Gate {
            slots: Arc::new(Semaphore::new(limits.max_parallel)),
            inner: Mutex::new(Inner {
                limits,
                debt: 0,
                next_spawn: None,
                seq: 0,
                waiters: Vec::new(),
                held: BTreeMap::new(),
            }),
            changed: watch::channel(0).0,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Waits for a slot, the agent's share and the folder, all at once.
    /// Dropping the future (a cancel while queued) leaves the queue.
    pub async fn acquire(self: &Arc<Self>, ticket: Ticket) -> JobLease {
        self.acquire_placed(ticket, false).await.0
    }

    /// [`Self::acquire`] for a job with `isolation = auto` (2.3, D3): when
    /// `isolate_if_busy` and another job already holds or waits for an
    /// overlapping folder, the ticket drops its folder (the job will run in
    /// a worktree of its own instead of waiting) and the second value is
    /// `true`. Decided under the gate's lock, so two jobs arriving together
    /// never both take the folder nor both leave it.
    pub async fn acquire_placed(
        self: &Arc<Self>,
        mut ticket: Ticket,
        isolate_if_busy: bool,
    ) -> (JobLease, bool) {
        let (tx, rx) = oneshot::channel();
        let mut isolated = false;
        let (seq, sends) = {
            let mut g = self.lock();
            if isolate_if_busy {
                if let Some(folder) = &ticket.folder {
                    let busy = g
                        .held
                        .values()
                        .filter_map(|h| h.folder.as_ref())
                        .chain(g.waiters.iter().filter_map(|w| w.ticket.folder.as_ref()))
                        .any(|f| f.overlaps(folder));
                    if busy {
                        ticket.folder = None;
                        isolated = true;
                    }
                }
            }
            g.seq += 1;
            let seq = g.seq;
            g.waiters.push(Waiter { seq, ticket, tx });
            (seq, self.admit(&mut g))
        };
        let _leave = Leave {
            gate: self.clone(),
            seq,
        };
        self.deliver(sends);
        match rx.await {
            Ok(lease) => (lease, isolated),
            // Only if the waiter was removed without an answer, which only
            // `Leave` does, i.e. never while this future is alive.
            Err(_) => std::future::pending().await,
        }
    }

    /// New caps, applied at once: a larger cap lets waiters in now, a
    /// smaller one lets running jobs finish and admits nobody until the
    /// count is under it.
    pub fn set_limits(self: &Arc<Self>, max_parallel: usize, max_per_agent: Option<usize>) {
        let sends = {
            let mut g = self.lock();
            let next = Limits {
                max_parallel,
                max_per_agent,
                ..g.limits
            }
            .clamped();
            let old = g.limits.max_parallel;
            let new = next.max_parallel;
            if new > old {
                let mut add = new - old;
                let paid = add.min(g.debt);
                g.debt -= paid;
                add -= paid;
                self.slots.add_permits(add);
            } else if new < old {
                let cut = old - new;
                let forgotten = self.slots.forget_permits(cut);
                g.debt += cut - forgotten;
            }
            g.limits = next;
            self.admit(&mut g)
        };
        self.deliver(sends);
    }

    pub fn limits(&self) -> Limits {
        self.lock().limits
    }

    pub fn snapshot(&self) -> GateView {
        let g = self.lock();
        let queued = Self::order(&g)
            .into_iter()
            .enumerate()
            .map(|(i, idx)| {
                let t = &g.waiters[idx].ticket;
                Queued {
                    job_id: t.job_id.clone(),
                    agent_id: t.agent_id.clone(),
                    class: t.class,
                    folder: t.folder.clone(),
                    position: i + 1,
                    // Right after every admission round each waiter has a
                    // barrier; `Slot` is only the fallback of the type.
                    reason: self.blocker(&g, t).unwrap_or(WaitReason::Slot),
                }
            })
            .collect();
        GateView {
            held: g.held.values().cloned().collect(),
            queued,
            limits: g.limits,
            available: self.slots.available_permits(),
        }
    }

    /// Bumped after every admission, release and cap change.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Indices of the waiters in the order they are let in.
    fn order(g: &Inner) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..g.waiters.len()).collect();
        idx.sort_by_key(|&i| (g.waiters[i].ticket.class, g.waiters[i].seq));
        idx
    }

    fn blocker(&self, g: &Inner, t: &Ticket) -> Option<WaitReason> {
        if let Some(folder) = &t.folder {
            if let Some(h) = g
                .held
                .values()
                .find(|h| h.folder.as_ref().is_some_and(|f| f.overlaps(folder)))
            {
                return Some(WaitReason::Workspace {
                    holder: h.job_id.clone(),
                    folder: h.folder.clone().unwrap_or_else(|| folder.clone()),
                });
            }
        }
        if let Some(cap) = g.limits.max_per_agent {
            if g.held.values().filter(|h| h.agent_id == t.agent_id).count() >= cap {
                return Some(WaitReason::AgentCap { cap });
            }
        }
        if self.slots.available_permits() == 0 {
            return Some(WaitReason::Slot);
        }
        None
    }

    /// One admission round, under the lock: every waiter with no barrier is
    /// let in, in order. The answers are sent after the lock is released.
    fn admit(self: &Arc<Self>, g: &mut Inner) -> Sends {
        g.waiters.retain(|w| !w.tx.is_closed());
        let now = Instant::now();
        let mut chosen: Vec<(usize, u64, OwnedSemaphorePermit, Duration)> = Vec::new();
        for idx in Self::order(g) {
            let ticket = g.waiters[idx].ticket.clone();
            if self.blocker(g, &ticket).is_some() {
                continue;
            }
            let Ok(permit) = self.slots.clone().try_acquire_owned() else {
                break;
            };
            g.seq += 1;
            let id = g.seq;
            let start = g.next_spawn.map_or(now, |n| n.max(now));
            g.next_spawn = Some(start + g.limits.stagger);
            g.held.insert(
                id,
                Held {
                    job_id: ticket.job_id,
                    agent_id: ticket.agent_id,
                    class: ticket.class,
                    folder: ticket.folder,
                },
            );
            chosen.push((idx, id, permit, start - now));
        }
        chosen.sort_by_key(|c| std::cmp::Reverse(c.0));
        chosen
            .into_iter()
            .map(|(idx, id, permit, delay)| {
                let w = g.waiters.remove(idx);
                (
                    w.tx,
                    JobLease {
                        gate: self.clone(),
                        id,
                        permit: Some(permit),
                        delay,
                    },
                )
            })
            .collect()
    }

    /// Hands the leases out, outside the lock. One whose waiter went away in
    /// the meantime is dropped here, which releases it again.
    fn deliver(&self, sends: Sends) {
        for (tx, lease) in sends {
            if let Err(lease) = tx.send(lease) {
                drop(lease);
            }
        }
        self.changed.send_modify(|v| *v += 1);
    }

    fn release(self: &Arc<Self>, id: u64, permit: Option<OwnedSemaphorePermit>) {
        let sends = {
            let mut g = self.lock();
            g.held.remove(&id);
            if let Some(permit) = permit {
                if g.debt > 0 {
                    permit.forget();
                    g.debt -= 1;
                } else {
                    drop(permit);
                }
            }
            self.admit(&mut g)
        };
        self.deliver(sends);
    }
}
