use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::folder_key::FolderKey;

pub const STAGGER: Duration = Duration::from_millis(500);
pub const MAX_PARALLEL_RANGE: (usize, usize) = (1, 16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Manual,
    Mission,
}

#[derive(Debug, Clone)]
pub struct Ticket {
    pub job_id: String,
    pub agent_id: String,
    pub class: Class,
    pub folder: Option<FolderKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_parallel: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    Slot,
    AgentCap { cap: usize },
    Workspace { holder: String, folder: FolderKey },
}

impl WaitReason {
    pub fn code(&self) -> &'static str {
        match self {
            WaitReason::Slot => "slot",
            WaitReason::AgentCap { .. } => "agent_cap",
            WaitReason::Workspace { .. } => "workspace",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Held {
    pub job_id: String,
    pub agent_id: String,
    pub class: Class,
    pub folder: Option<FolderKey>,
}

#[derive(Debug, Clone)]
pub struct Queued {
    pub job_id: String,
    pub agent_id: String,
    pub class: Class,
    pub folder: Option<FolderKey>,
    pub position: usize,
    pub reason: WaitReason,
}

#[derive(Debug, Clone)]
pub struct GateView {
    pub held: Vec<Held>,
    pub queued: Vec<Queued>,
    pub limits: Limits,
    pub available: usize,
}

struct Waiter {
    seq: u64,
    ticket: Ticket,
    tx: oneshot::Sender<JobLease>,
}

struct Inner {
    limits: Limits,
    debt: usize,
    next_spawn: Option<Instant>,
    seq: u64,
    waiters: Vec<Waiter>,
    held: BTreeMap<u64, Held>,
}

pub struct Gate {
    inner: Mutex<Inner>,
    slots: Arc<Semaphore>,
    changed: watch::Sender<u64>,
}

pub struct JobLease {
    gate: Arc<Gate>,
    id: u64,
    permit: Option<OwnedSemaphorePermit>,
    pub delay: Duration,
}

impl Drop for JobLease {
    fn drop(&mut self) {
        self.gate.release(self.id, self.permit.take());
    }
}

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
            Err(_) => std::future::pending().await,
        }
    }

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

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

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
