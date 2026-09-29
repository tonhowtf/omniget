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

    #[cfg(test)]
    fn debt(&self) -> usize {
        self.lock().debt
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    fn limits(max: usize) -> Limits {
        Limits {
            max_parallel: max,
            max_per_agent: None,
            stagger: Duration::ZERO,
        }
    }

    fn ticket(job: &str, agent: &str, folder: Option<&str>) -> Ticket {
        Ticket {
            job_id: job.into(),
            agent_id: agent.into(),
            class: Class::Manual,
            folder: folder.map(|f| FolderKey::normalize(f, false, false)),
        }
    }

    fn mission(job: &str, folder: Option<&str>) -> Ticket {
        Ticket {
            class: Class::Mission,
            ..ticket(job, "m", folder)
        }
    }

    /// Admitted at once (within 100 ms of real time)?
    async fn now_or_never<F: Future<Output = JobLease>>(f: F) -> Option<JobLease> {
        tokio::time::timeout(Duration::from_millis(100), f)
            .await
            .ok()
    }

    /// Starts an acquire in the background; `Some(lease)` once admitted.
    fn waiting(gate: &Arc<Gate>, t: Ticket) -> tokio::task::JoinHandle<JobLease> {
        let gate = gate.clone();
        tokio::spawn(async move { gate.acquire(t).await })
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    fn queued_ids(g: &Gate) -> Vec<String> {
        g.snapshot().queued.into_iter().map(|q| q.job_id).collect()
    }

    /// Invariant of 1.6: free + in use − owed = the cap, at every step.
    fn assert_books(g: &Gate) {
        let v = g.snapshot();
        assert_eq!(
            v.available + v.held.len() - g.debt(),
            v.limits.max_parallel,
            "free {} + held {} - debt {} != cap {}",
            v.available,
            v.held.len(),
            g.debt(),
            v.limits.max_parallel
        );
    }

    /// 2.3 (`isolation = auto`): the first job takes the folder; one that
    /// arrives while it is held (or waited for) is placed elsewhere and
    /// admitted at once, without the folder; a job with the folder free
    /// keeps it.
    #[tokio::test]
    async fn an_auto_job_leaves_a_busy_folder_instead_of_waiting() {
        let gate = Gate::new(limits(4));
        let (a, a_iso) = gate
            .acquire_placed(ticket("a", "x", Some("/w/repo")), true)
            .await;
        assert!(!a_iso, "the folder was free: a keeps it");
        let (b, b_iso) = tokio::time::timeout(
            Duration::from_millis(100),
            gate.acquire_placed(ticket("b", "x", Some("/w/repo/src")), true),
        )
        .await
        .expect("b does not wait for the folder");
        assert!(b_iso);
        let held: Vec<Option<FolderKey>> =
            gate.snapshot().held.into_iter().map(|h| h.folder).collect();
        assert_eq!(held.iter().filter(|f| f.is_some()).count(), 1);
        // A `shared` job still waits; an `auto` one behind it is placed too.
        let c = waiting(&gate, ticket("c", "x", Some("/w/repo")));
        settle().await;
        assert!(!c.is_finished());
        drop(a);
        let c = tokio::time::timeout(Duration::from_secs(1), c)
            .await
            .expect("c admitted")
            .unwrap();
        let (_d, d_iso) = gate
            .acquire_placed(ticket("d", "x", Some("/w/repo")), true)
            .await;
        assert!(d_iso, "c holds the folder now");
        drop((b, c));
        let (_e, e_iso) = gate
            .acquire_placed(ticket("e", "x", Some("/w/other")), true)
            .await;
        assert!(!e_iso);
        assert_books(&gate);
    }

    #[tokio::test]
    async fn two_writers_on_one_folder_never_overlap() {
        let gate = Gate::new(limits(4));
        let a = now_or_never(gate.acquire(ticket("a", "x", Some("/w/shared"))))
            .await
            .expect("a runs");
        let b = waiting(&gate, ticket("b", "y", Some("/w/shared")));
        settle().await;
        assert!(!b.is_finished(), "b must wait for the folder");
        let v = gate.snapshot();
        assert_eq!(v.queued.len(), 1);
        assert_eq!(
            v.queued[0].reason,
            WaitReason::Workspace {
                holder: "a".into(),
                folder: FolderKey::normalize("/w/shared", false, false)
            }
        );
        drop(a);
        let b = tokio::time::timeout(Duration::from_secs(1), b)
            .await
            .expect("b admitted")
            .unwrap();
        assert_eq!(gate.snapshot().held.len(), 1);
        drop(b);
        assert_books(&gate);
    }

    #[tokio::test]
    async fn a_job_in_a_folder_blocks_its_subfolder_and_back() {
        let gate = Gate::new(limits(4));
        let repo = now_or_never(gate.acquire(ticket("repo", "x", Some("/w/repo"))))
            .await
            .unwrap();
        let sub = waiting(&gate, ticket("sub", "x", Some("/w/repo/src")));
        let sibling = now_or_never(gate.acquire(ticket("sib", "x", Some("/w/repo2")))).await;
        assert!(sibling.is_some(), "/w/repo2 is another folder");
        settle().await;
        assert!(!sub.is_finished());
        drop(repo);
        let sub = tokio::time::timeout(Duration::from_secs(1), sub)
            .await
            .unwrap()
            .unwrap();
        // And back: the parent waits for the child.
        let parent = waiting(&gate, ticket("parent", "x", Some("/w")));
        settle().await;
        assert!(!parent.is_finished());
        assert_eq!(gate.snapshot().queued[0].job_id, "parent");
        drop(sub);
        drop(sibling);
        tokio::time::timeout(Duration::from_secs(1), parent)
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlinked_spelling_waits_for_the_same_folder() {
        let root = std::env::temp_dir().join(format!("omniget-gate-{}", uuid::Uuid::new_v4()));
        let real = root.join("shared");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, root.join("alias")).unwrap();
        let gate = Gate::new(limits(4));
        let t = |job: &str, p: std::path::PathBuf| Ticket {
            job_id: job.into(),
            agent_id: job.into(),
            class: Class::Manual,
            folder: Some(FolderKey::of(&p)),
        };
        let a = now_or_never(gate.acquire(t("a", real.clone())))
            .await
            .unwrap();
        let b = waiting(&gate, t("b", root.join("alias")));
        let c = waiting(&gate, t("c", root.join("alias").join("..").join("SHARED")));
        settle().await;
        assert!(!b.is_finished(), "a symlink is the same folder");
        if cfg!(target_os = "macos") {
            assert!(!c.is_finished(), "APFS ignores case");
        }
        drop(a);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn the_cap_holds_and_a_freed_slot_goes_to_the_next() {
        let gate = Gate::new(limits(2));
        let a = now_or_never(gate.acquire(ticket("a", "x", None)))
            .await
            .unwrap();
        let _b = now_or_never(gate.acquire(ticket("b", "y", None)))
            .await
            .unwrap();
        let c = waiting(&gate, ticket("c", "z", None));
        settle().await;
        assert!(!c.is_finished());
        let v = gate.snapshot();
        assert_eq!(v.queued[0].reason, WaitReason::Slot);
        assert_eq!(v.queued[0].position, 1);
        drop(a);
        tokio::time::timeout(Duration::from_secs(1), c)
            .await
            .unwrap()
            .unwrap();
        assert_books(&gate);
    }

    #[tokio::test]
    async fn changing_the_cap_neither_loses_nor_creates_permits() {
        let gate = Gate::new(limits(4));
        let mut held = Vec::new();
        for i in 0..4 {
            held.push(
                now_or_never(gate.acquire(ticket(&format!("j{i}"), "x", None)))
                    .await
                    .unwrap(),
            );
        }
        assert_books(&gate);
        // Shrink with every slot taken: nothing to forget yet, so it is owed.
        gate.set_limits(2, None);
        assert_eq!(gate.snapshot().available, 0);
        assert_books(&gate);
        let fifth = waiting(&gate, ticket("j4", "x", None));
        settle().await;
        held.pop();
        held.pop();
        settle().await;
        assert!(!fifth.is_finished(), "two still run and the cap is 2");
        assert_books(&gate);
        held.pop();
        let fifth = tokio::time::timeout(Duration::from_secs(1), fifth)
            .await
            .unwrap()
            .unwrap();
        assert_books(&gate);
        held.clear();
        drop(fifth);
        assert_eq!(
            gate.snapshot().available,
            2,
            "after the shrink: exactly the new cap"
        );
        // Grow: exactly the new cap, all usable at once.
        gate.set_limits(6, None);
        assert_eq!(gate.snapshot().available, 6);
        for i in 0..6 {
            held.push(
                now_or_never(gate.acquire(ticket(&format!("k{i}"), "x", None)))
                    .await
                    .unwrap(),
            );
        }
        assert!(now_or_never(gate.acquire(ticket("k6", "x", None)))
            .await
            .is_none());
        assert_books(&gate);
        // Shrink with some free, then grow before the debt is paid.
        held.truncate(3);
        gate.set_limits(1, None);
        assert_books(&gate);
        gate.set_limits(3, None);
        assert_books(&gate);
        held.clear();
        assert_eq!(gate.snapshot().available, 3);
        // Out-of-range values are clamped to 1..=16.
        gate.set_limits(0, None);
        assert_eq!(gate.snapshot().limits.max_parallel, 1);
        gate.set_limits(99, None);
        assert_eq!(gate.snapshot().limits.max_parallel, 16);
        assert_eq!(gate.snapshot().available, 16);
    }

    #[tokio::test]
    async fn the_agent_cap_skips_to_another_agent() {
        let gate = Gate::new(Limits {
            max_per_agent: Some(1),
            ..limits(4)
        });
        let _a1 = now_or_never(gate.acquire(ticket("a1", "A", None)))
            .await
            .unwrap();
        let a2 = waiting(&gate, ticket("a2", "A", None));
        settle().await;
        let b1 = now_or_never(gate.acquire(ticket("b1", "B", None))).await;
        assert!(b1.is_some(), "B is not at its cap and a slot is free");
        assert!(!a2.is_finished());
        let v = gate.snapshot();
        assert_eq!(v.queued.len(), 1);
        assert_eq!(v.queued[0].reason, WaitReason::AgentCap { cap: 1 });
    }

    /// W2 / 83#P5: tasks a mission queued never stand in front of a job the
    /// user started, and a mission never runs past the cap.
    #[tokio::test]
    async fn a_mission_never_waits_ahead_of_a_manual_job() {
        let gate = Gate::new(limits(2));
        let m1 = now_or_never(gate.acquire(mission("m1", None)))
            .await
            .unwrap();
        let _m2 = now_or_never(gate.acquire(mission("m2", None)))
            .await
            .unwrap();
        let m3 = waiting(&gate, mission("m3", None));
        let m4 = waiting(&gate, mission("m4", None));
        settle().await;
        let manual = waiting(&gate, ticket("run", "x", None));
        settle().await;
        assert_eq!(queued_ids(&gate), vec!["run", "m3", "m4"]);
        assert_eq!(gate.snapshot().queued[0].position, 1);
        drop(m1);
        let _manual = tokio::time::timeout(Duration::from_secs(1), manual)
            .await
            .unwrap()
            .unwrap();
        settle().await;
        assert!(
            !m3.is_finished() && !m4.is_finished(),
            "the cap still holds for the mission"
        );
        assert_eq!(gate.snapshot().held.len(), 2);
    }

    #[tokio::test]
    async fn every_queued_job_has_a_position_and_a_reason() {
        let gate = Gate::new(Limits {
            max_per_agent: Some(1),
            ..limits(2)
        });
        let _w = now_or_never(gate.acquire(ticket("writer", "A", Some("/w/repo"))))
            .await
            .unwrap();
        let _o = now_or_never(gate.acquire(ticket("other", "B", None)))
            .await
            .unwrap();
        let _q1 = waiting(&gate, ticket("same-folder", "C", Some("/w/repo/docs")));
        let _q2 = waiting(&gate, ticket("same-agent", "A", None));
        let _q3 = waiting(&gate, ticket("plain", "D", None));
        let _q4 = waiting(&gate, mission("mission", Some("/elsewhere")));
        settle().await;
        let v = gate.snapshot();
        let positions: Vec<usize> = v.queued.iter().map(|q| q.position).collect();
        assert_eq!(positions, vec![1, 2, 3, 4]);
        let reasons: Vec<(&str, &WaitReason)> = v
            .queued
            .iter()
            .map(|q| (q.job_id.as_str(), &q.reason))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (
                    "same-folder",
                    &WaitReason::Workspace {
                        holder: "writer".into(),
                        folder: FolderKey::normalize("/w/repo", false, false)
                    }
                ),
                ("same-agent", &WaitReason::AgentCap { cap: 1 }),
                ("plain", &WaitReason::Slot),
                ("mission", &WaitReason::Slot),
            ]
        );
        assert_eq!(WaitReason::Slot.code(), "slot");
        assert_eq!(WaitReason::AgentCap { cap: 1 }.code(), "agent_cap");
    }

    #[tokio::test]
    async fn spawns_are_staggered() {
        let gate = Gate::new(Limits {
            stagger: STAGGER,
            ..limits(4)
        });
        let a = now_or_never(gate.acquire(ticket("a", "x", None)))
            .await
            .unwrap();
        let b = now_or_never(gate.acquire(ticket("b", "x", None)))
            .await
            .unwrap();
        let c = now_or_never(gate.acquire(ticket("c", "x", None)))
            .await
            .unwrap();
        assert_eq!(a.delay, Duration::ZERO);
        let near = |d: Duration, want: u64| d.as_millis().abs_diff(u128::from(want)) <= 30;
        assert!(near(b.delay, 500), "{:?}", b.delay);
        assert!(near(c.delay, 1000), "{:?}", c.delay);
    }

    #[tokio::test]
    async fn a_cancelled_wait_leaves_the_queue() {
        let gate = Gate::new(limits(1));
        let a = now_or_never(gate.acquire(ticket("a", "x", None)))
            .await
            .unwrap();
        let ghost = waiting(&gate, ticket("ghost", "x", None));
        settle().await;
        assert_eq!(queued_ids(&gate), vec!["ghost"]);
        ghost.abort();
        settle().await;
        assert!(
            queued_ids(&gate).is_empty(),
            "a cancelled wait is not a queued job"
        );
        drop(a);
        settle().await;
        assert_eq!(
            gate.snapshot().available,
            1,
            "the slot went back, not to the ghost"
        );
        assert!(gate.snapshot().held.is_empty());
    }

    #[tokio::test]
    async fn every_change_is_announced() {
        let gate = Gate::new(limits(1));
        let mut rx = gate.subscribe();
        let a = now_or_never(gate.acquire(ticket("a", "x", None)))
            .await
            .unwrap();
        assert!(rx.has_changed().unwrap());
        rx.borrow_and_update();
        drop(a);
        assert!(rx.has_changed().unwrap());
    }
}

// ── BN2 (scripts/llm/jobs_parallel_eval.py), scenarios A and C ──────────
//
// Level: the real gate, the real `FolderKey`, `scripts/llm/fake_claude.py`
// spawned through the real `spawn_turn_with` and read by the real
// `ClaudeParser`, and the real shadow-git snapshot and undo, called the way
// the Coordinator brackets a CLI turn (`before_write` before, `after_turn`
// after). What is not in it: `Jobs::run_job`, `LlmManager` and the Tauri app
// (no window here); `run_job` takes the same `Gate::acquire` first thing.
// Sizes come from the environment so the script can run the full benchmark
// (`BN2_REPS=20`, `BN2_READ_S=10.4`) while `cargo test` stays short.
#[cfg(test)]
mod bn2 {
    use super::*;
    use crate::core::llm::cli_runtime::claude::ClaudeParser;
    use crate::core::llm::cli_runtime::session::{spawn_turn_with, SpawnSpec, TurnHooks};
    use crate::core::llm::snapshot;
    use crate::core::llm::types::TurnEvent;
    use futures::StreamExt;
    use std::collections::{BTreeMap as Map, HashMap};
    use std::path::{Path, PathBuf};
    use tokio_util::sync::CancellationToken;

    fn bench(metric: &str, value: serde_json::Value) {
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "{}",
                    serde_json::json!({ "bench": "bn2", "metric": metric, "value": value, "level": "gate+turn+snapshot" })
                );
            }
        }
    }

    fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    fn fake() -> Option<PathBuf> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/llm/fake_claude.py");
        let python = std::process::Command::new("python3")
            .arg("--version")
            .output()
            .ok()?;
        (python.status.success() && script.is_file()).then_some(script)
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-bn2-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn spec(fake: &Path, cwd: &Path, env: &[(&str, String)]) -> SpawnSpec {
        SpawnSpec {
            program: PathBuf::from("python3"),
            args: vec![
                fake.display().to_string(),
                "-p".into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            cwd: Some(cwd.to_path_buf()),
            stdin: Some("User: do the task".into()),
            ..SpawnSpec::default()
        }
    }

    /// When a job got in and when its turn ended (seconds since `t0`).
    #[derive(Debug, Clone)]
    struct Ran {
        tag: String,
        started: f64,
        ended: f64,
        finished_ok: bool,
    }

    /// One job as `run_job` plus the Coordinator run it for a CLI agent:
    /// the gate, the stagger, then with a folder the snapshot before the
    /// turn, the turn, the snapshot after. `snap` = (canonical folder,
    /// conversation), as `set_conversation_workspace` stores it.
    async fn job(
        gate: Option<Arc<Gate>>,
        ticket: Ticket,
        spec: SpawnSpec,
        snap: Option<(PathBuf, String)>,
        t0: Instant,
    ) -> Ran {
        let lease = match &gate {
            Some(g) => {
                let l = g.acquire(ticket.clone()).await;
                tokio::time::sleep(l.delay).await;
                Some(l)
            }
            None => None,
        };
        let started = t0.elapsed().as_secs_f64();
        if let Some((ws, conv)) = &snap {
            snapshot::before_write(ws, conv, &format!("{conv}-req"))
                .await
                .unwrap();
        }
        let mut stream = spawn_turn_with(
            spec,
            Arc::new(ClaudeParser::new(true)),
            CancellationToken::new(),
            TurnHooks::default(),
        )
        .unwrap();
        let mut finished_ok = false;
        while let Some(ev) = stream.next().await {
            if matches!(
                ev,
                TurnEvent::Finished {
                    reason: crate::core::llm::types::FinishReason::Stop
                }
            ) {
                finished_ok = true;
            }
        }
        if let Some((ws, conv)) = &snap {
            snapshot::after_turn(ws, conv, &format!("{conv}-req"))
                .await
                .unwrap();
        }
        let ended = t0.elapsed().as_secs_f64();
        drop(lease);
        Ran {
            tag: ticket.job_id,
            started,
            ended,
            finished_ok,
        }
    }

    /// Pairs of runs whose [start, end] intervals intersect.
    fn overlapping_pairs(runs: &[Ran]) -> usize {
        let mut n = 0;
        for i in 0..runs.len() {
            for j in i + 1..runs.len() {
                if runs[i].started < runs[j].ended && runs[j].started < runs[i].ended {
                    n += 1;
                }
            }
        }
        n
    }

    fn max_concurrent(runs: &[Ran]) -> usize {
        runs.iter()
            .map(|r| {
                runs.iter()
                    .filter(|o| o.started <= r.started && r.started < o.ended)
                    .count()
            })
            .max()
            .unwrap_or(0)
    }

    fn files(dir: &Path) -> Map<String, Vec<u8>> {
        let mut out = Map::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_file() {
                out.insert(
                    e.file_name().to_string_lossy().to_string(),
                    std::fs::read(e.path()).unwrap(),
                );
            }
        }
        out
    }

    fn bytes_changed(a: &[u8], b: &[u8]) -> usize {
        let common = a.iter().zip(b).filter(|(x, y)| x != y).count();
        common + a.len().abs_diff(b.len())
    }

    /// The four spellings of one folder the jobs use: itself, a symlink to
    /// it, another letter case (case-insensitive disks) or `.` segments,
    /// and its parent (a job there writes into the same tree).
    fn spellings(root: &Path, shared: &Path) -> Vec<PathBuf> {
        let alias = root.join("alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(shared, &alias).unwrap();
        #[cfg(not(unix))]
        let alias = shared.join(".");
        let case = if cfg!(any(windows, target_os = "macos")) {
            root.join("SHARED")
        } else {
            shared.join(".")
        };
        vec![shared.to_path_buf(), alias, case, root.to_path_buf()]
    }

    fn write_job(
        fake: &Path,
        cwd: &Path,
        tag: &str,
        ledger: &Path,
        own: &Path,
        hold: f64,
    ) -> SpawnSpec {
        spec(
            fake,
            cwd,
            &[
                ("FAKE_CLAUDE_MODE", "write".into()),
                ("FAKE_CLAUDE_HOLD_S", hold.to_string()),
                ("FAKE_CLAUDE_TAG", tag.into()),
                ("FAKE_CLAUDE_LEDGER", ledger.display().to_string()),
                ("FAKE_CLAUDE_FILE", own.display().to_string()),
            ],
        )
    }

    fn ledger_tags(ledger: &Path) -> Vec<String> {
        std::fs::read_to_string(ledger)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The control: the same four writers with no gate. Proves the ledger
    /// check can see an overwrite (what happens today with two `claude -p`
    /// in one folder, W4).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bn2_a_control_without_the_gate_loses_lines() {
        let Some(fake) = fake() else {
            return eprintln!("python3 missing: skipped");
        };
        let root = temp("control");
        let shared = root.join("Shared");
        std::fs::create_dir_all(&shared).unwrap();
        let ledger = shared.join("ledger.txt");
        let t0 = Instant::now();
        let handles: Vec<_> = spellings(&root, &shared)
            .into_iter()
            .enumerate()
            .map(|(i, cwd)| {
                let tag = format!("j{i}");
                let s = write_job(
                    &fake,
                    &cwd,
                    &tag,
                    &ledger,
                    &shared.join(format!("{tag}.txt")),
                    0.3,
                );
                let t = Ticket {
                    job_id: tag,
                    agent_id: "a".into(),
                    class: Class::Manual,
                    folder: None,
                };
                tokio::spawn(job(None, t, s, None, t0))
            })
            .collect();
        let runs: Vec<Ran> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();
        let lost = 4 - ledger_tags(&ledger).len();
        bench("a_control_lost_lines", serde_json::json!(lost));
        bench(
            "a_control_overlapping_pairs",
            serde_json::json!(overlapping_pairs(&runs)),
        );
        assert!(
            lost >= 1,
            "without the gate the read-modify-write must lose lines"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Scenario A: 4 writers on one folder, spelled 4 ways. 0 lines lost,
    /// 0 overlapping runs, and the undo never touches another job's bytes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bn2_a_one_writer_per_folder_and_a_safe_undo() {
        let Some(fake) = fake() else {
            return eprintln!("python3 missing: skipped");
        };
        if !snapshot::git_available() {
            return eprintln!("git missing: skipped");
        }
        snapshot::undo_tests::test_root();
        // `cargo test` runs one short rep; the benchmark script sets
        // `OMNIGET_BENCH_OUT` and gets the production stagger.
        let full = std::env::var_os("OMNIGET_BENCH_OUT").is_some();
        let reps: usize = env_num("BN2_REPS", 1);
        let hold: f64 = env_num("BN2_WRITE_S", 0.15);
        let stagger = if full {
            STAGGER
        } else {
            Duration::from_millis(50)
        };
        let (mut lost_total, mut overlap_total) = (0usize, 0usize);
        let mut live_codes: Vec<String> = Vec::new();
        let mut last_ok = 0usize;
        let mut last_other_bytes = 0usize;
        let mut first_codes: Vec<String> = Vec::new();
        let mut first_bytes = 0usize;
        let mut waited_workspace = 0usize;
        for rep in 0..reps {
            let root = temp("a");
            let shared = root.join("Shared");
            std::fs::create_dir_all(&shared).unwrap();
            let ledger = shared.join("ledger.txt");
            let gate = Gate::new(Limits {
                max_parallel: 4,
                max_per_agent: None,
                stagger,
            });
            let t0 = Instant::now();
            let mut conv_of: HashMap<String, (PathBuf, String, PathBuf)> = HashMap::new();
            let mut handles = Vec::new();
            for (i, cwd) in spellings(&root, &shared).into_iter().enumerate() {
                let tag = format!("j{i}");
                let own = shared.join(format!("{tag}.txt"));
                let s = write_job(&fake, &cwd, &tag, &ledger, &own, hold);
                let canon = cwd.canonicalize().unwrap();
                let conv = format!("bn2-{rep}-{tag}");
                conv_of.insert(tag.clone(), (canon.clone(), conv.clone(), cwd.clone()));
                let t = Ticket {
                    job_id: tag.clone(),
                    agent_id: format!("agent-{i}"),
                    class: Class::Manual,
                    folder: Some(FolderKey::of(&cwd)),
                };
                handles.push(tokio::spawn(job(
                    Some(gate.clone()),
                    t,
                    s,
                    Some((canon, conv)),
                    t0,
                )));
            }
            // While a later job runs, undo the first one that finished.
            let mut tried_live = false;
            while handles.iter().any(|h| !h.is_finished()) {
                let view = gate.snapshot();
                waited_workspace = waited_workspace.max(
                    view.queued
                        .iter()
                        .filter(|q| q.reason.code() == "workspace")
                        .count(),
                );
                let done: Vec<&String> = conv_of
                    .keys()
                    .filter(|t| std::fs::read(shared.join(format!("{t}.txt"))).is_ok())
                    .collect();
                if !tried_live && !done.is_empty() && !view.held.is_empty() {
                    let first = done[0].clone();
                    let holder = &view.held[0].job_id;
                    if holder != &first {
                        let (ws, conv, _) = &conv_of[&first];
                        let live: Vec<PathBuf> = view
                            .held
                            .iter()
                            .map(|h| conv_of[&h.job_id].2.clone())
                            .collect();
                        let code = match snapshot::undo(ws, conv, &live).await {
                            Ok(_) => "restored".to_string(),
                            Err(e) => e.split(':').next().unwrap_or("").to_string(),
                        };
                        live_codes.push(code);
                        tried_live = true;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let mut runs = Vec::new();
            for h in handles {
                runs.push(h.await.unwrap());
            }
            assert!(runs.iter().all(|r| r.finished_ok), "{runs:?}");
            let tags = ledger_tags(&ledger);
            lost_total += 4 - tags.len();
            overlap_total += overlapping_pairs(&runs);
            // Undo the last job: only its own paths go back.
            runs.sort_by(|a, b| a.ended.total_cmp(&b.ended));
            let last = runs.last().unwrap().tag.clone();
            let before = files(&shared);
            let (ws, conv, _) = &conv_of[&last];
            if snapshot::undo(ws, conv, &[]).await.is_ok() {
                last_ok += 1;
            }
            let after = files(&shared);
            for (name, bytes) in &before {
                if name == &format!("{last}.txt") {
                    continue;
                }
                if name == "ledger.txt" {
                    let want: String = tags
                        .iter()
                        .filter(|t| **t != last)
                        .map(|t| format!("{t}\n"))
                        .collect();
                    last_other_bytes += bytes_changed(
                        want.as_bytes(),
                        after.get(name).map(Vec::as_slice).unwrap_or(&[]),
                    );
                } else {
                    last_other_bytes +=
                        bytes_changed(bytes, after.get(name).map(Vec::as_slice).unwrap_or(&[]));
                }
            }
            // Undo the first job: others wrote the ledger after it, refused.
            let first = runs.first().unwrap().tag.clone();
            let before = files(&shared);
            let (ws, conv, _) = &conv_of[&first];
            first_codes.push(match snapshot::undo(ws, conv, &[]).await {
                Ok(_) => "restored".into(),
                Err(e) => e.split(':').next().unwrap_or("").to_string(),
            });
            let after = files(&shared);
            first_bytes += before
                .iter()
                .map(|(n, b)| bytes_changed(b, after.get(n).map(Vec::as_slice).unwrap_or(&[])))
                .sum::<usize>();
            let _ = std::fs::remove_dir_all(root);
        }
        bench("a_reps", serde_json::json!(reps));
        bench("a_lost_lines", serde_json::json!(lost_total));
        bench(
            "a_overlapping_pairs_same_folder",
            serde_json::json!(overlap_total),
        );
        bench(
            "a_max_waiting_workspace",
            serde_json::json!(waited_workspace),
        );
        bench("a_undo_while_live_codes", serde_json::json!(live_codes));
        bench("a_undo_last_restored", serde_json::json!(last_ok));
        bench(
            "a_undo_last_bytes_of_others_changed",
            serde_json::json!(last_other_bytes),
        );
        bench("a_undo_first_codes", serde_json::json!(first_codes));
        bench("a_undo_first_bytes_changed", serde_json::json!(first_bytes));
        assert_eq!(lost_total, 0, "an overwrite happened");
        assert_eq!(overlap_total, 0, "two writers ran together in one folder");
        assert_eq!(
            live_codes.len(),
            reps,
            "one undo attempt per rep while another job ran"
        );
        assert!(
            live_codes.iter().all(|c| c == snapshot::ERR_UNDO_SHARED),
            "{live_codes:?}"
        );
        assert!(
            waited_workspace >= 1,
            "the queue said why: waiting for the folder"
        );
        assert_eq!(last_ok, reps);
        assert_eq!(last_other_bytes, 0);
        assert!(
            first_codes.iter().all(|c| c == snapshot::ERR_UNDO_CONFLICT),
            "{first_codes:?}"
        );
        assert_eq!(first_bytes, 0);
    }

    /// The wall time the gate should give `n` jobs of `t` seconds: each
    /// starts when a slot frees and at least `stagger` after the previous
    /// start. For `t` much longer than the stagger this is
    /// `ceil(n/cap)·t + (cap−1)·stagger`.
    fn model_wall(n: usize, cap: usize, t: f64, stagger: f64) -> f64 {
        let mut free = vec![0.0f64; cap];
        let mut prev: Option<f64> = None;
        let mut wall = 0.0f64;
        for _ in 0..n {
            let (slot, at) = free
                .iter()
                .copied()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            let start = prev.map_or(at, |p| at.max(p + stagger));
            free[slot] = start + t;
            wall = wall.max(start + t);
            prev = Some(start);
        }
        wall
    }

    #[test]
    fn the_model_matches_the_closed_form_for_long_jobs() {
        assert!((model_wall(8, 4, 10.0, 0.5) - (2.0 * 10.0 + 1.5)).abs() < 1e-9);
        assert!((model_wall(8, 2, 10.0, 0.5) - (4.0 * 10.0 + 0.5)).abs() < 1e-9);
        assert!((model_wall(8, 8, 10.0, 0.5) - (10.0 + 3.5)).abs() < 1e-9);
        // Short jobs: the stagger chains across the batches.
        assert!((model_wall(8, 4, 1.0, 0.5) - 4.5).abs() < 1e-9);
    }

    /// Scenario C on the gate alone, in virtual time: 8 jobs of 10 s under
    /// caps 2, 4 and 8 end exactly when [`model_wall`] says, and never more
    /// than the cap run at once.
    #[tokio::test(start_paused = true)]
    async fn the_cap_scales_the_wall_time_on_a_paused_clock() {
        let job_len = Duration::from_secs(10);
        for cap in [2usize, 4, 8] {
            let gate = Gate::new(Limits {
                max_parallel: cap,
                max_per_agent: None,
                stagger: STAGGER,
            });
            let t0 = Instant::now();
            let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let (gate, running, peak) = (gate.clone(), running.clone(), peak.clone());
                    tokio::spawn(async move {
                        use std::sync::atomic::Ordering::SeqCst;
                        let lease = gate
                            .acquire(Ticket {
                                job_id: format!("r{i}"),
                                agent_id: format!("agent-{}", i % 3),
                                class: Class::Manual,
                                folder: None,
                            })
                            .await;
                        tokio::time::sleep(lease.delay).await;
                        let now = running.fetch_add(1, SeqCst) + 1;
                        peak.fetch_max(now, SeqCst);
                        tokio::time::sleep(job_len).await;
                        running.fetch_sub(1, SeqCst);
                        drop(lease);
                    })
                })
                .collect();
            for h in handles {
                h.await.unwrap();
            }
            let wall = t0.elapsed().as_secs_f64();
            let model = model_wall(8, cap, job_len.as_secs_f64(), STAGGER.as_secs_f64());
            assert!(
                (wall - model).abs() < 0.01,
                "cap {cap}: wall {wall} s vs model {model} s"
            );
            assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), cap);
        }
    }

    /// Scenario C: 8 read jobs (no folder) through real processes. The wall
    /// time follows [`model_wall`], the gate never runs more than the cap,
    /// and every queued job has a place and a reason that holds when it is
    /// sampled. `cargo test` runs a scaled-down copy (cap 4, 0.2 s jobs,
    /// 0.1 s stagger; the exact scaling is checked on a paused clock above);
    /// the benchmark script sets `OMNIGET_BENCH_OUT` and gets caps 2, 4 and
    /// 8 with the production stagger.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn bn2_c_the_cap_scales_the_wall_time() {
        let Some(fake) = fake() else {
            return eprintln!("python3 missing: skipped");
        };
        let full = std::env::var_os("OMNIGET_BENCH_OUT").is_some();
        let hold: f64 = env_num("BN2_READ_S", 0.2);
        let stagger = if full {
            STAGGER
        } else {
            Duration::from_millis(100)
        };
        let caps: Vec<usize> = std::env::var("BN2_CAPS")
            .ok()
            .map(|v| v.split(',').filter_map(|c| c.trim().parse().ok()).collect())
            .unwrap_or_else(|| if full { vec![2, 4, 8] } else { vec![4] });
        let trace = temp("c-trace");
        let mut unexplained = 0usize;
        let mut samples = 0usize;
        let mut queued_seen = 0usize;
        for cap in caps {
            let gate = Gate::new(Limits {
                max_parallel: cap,
                max_per_agent: None,
                stagger,
            });
            let t0 = Instant::now();
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let s = spec(
                        &fake,
                        &trace,
                        &[
                            ("FAKE_CLAUDE_MODE", "sleep".into()),
                            ("FAKE_CLAUDE_HOLD_S", hold.to_string()),
                            ("FAKE_CLAUDE_TAG", format!("r{i}")),
                        ],
                    );
                    let t = Ticket {
                        job_id: format!("r{i}"),
                        agent_id: format!("agent-{}", i % 3),
                        class: Class::Manual,
                        folder: None,
                    };
                    tokio::spawn(job(Some(gate.clone()), t, s, None, t0))
                })
                .collect();
            let mut max_held = 0usize;
            while handles.iter().any(|h| !h.is_finished()) {
                let v = gate.snapshot();
                samples += 1;
                queued_seen += v.queued.len();
                max_held = max_held.max(v.held.len());
                for (i, q) in v.queued.iter().enumerate() {
                    let holds = match &q.reason {
                        WaitReason::Slot => v.available == 0,
                        WaitReason::AgentCap { cap } => {
                            v.held.iter().filter(|h| h.agent_id == q.agent_id).count() >= *cap
                        }
                        WaitReason::Workspace { holder, .. } => {
                            v.held.iter().any(|h| &h.job_id == holder)
                        }
                    };
                    if !holds || q.position != i + 1 {
                        unexplained += 1;
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let mut runs = Vec::new();
            for h in handles {
                runs.push(h.await.unwrap());
            }
            let wall = runs.iter().map(|r| r.ended).fold(0.0, f64::max);
            let mut d: Vec<f64> = runs.iter().map(|r| r.ended - r.started).collect();
            d.sort_by(f64::total_cmp);
            let median = (d[3] + d[4]) / 2.0;
            let model = model_wall(8, cap, median, stagger.as_secs_f64());
            let concurrent = max_concurrent(&runs);
            bench(&format!("c_wall_s_cap{cap}"), serde_json::json!(wall));
            bench(
                &format!("c_median_job_s_cap{cap}"),
                serde_json::json!(median),
            );
            bench(
                &format!("c_ratio_cap{cap}"),
                serde_json::json!(wall / median),
            );
            bench(&format!("c_model_s_cap{cap}"), serde_json::json!(model));
            bench(
                &format!("c_max_concurrent_cap{cap}"),
                serde_json::json!(concurrent.max(max_held)),
            );
            assert!(runs.iter().all(|r| r.finished_ok));
            assert!(
                concurrent <= cap && max_held <= cap,
                "cap {cap}: {concurrent} ran at once"
            );
            assert!(
                wall <= model + 0.6,
                "cap {cap}: wall {wall:.2} s vs model {model:.2} s"
            );
            assert!(
                wall >= model - 0.3,
                "cap {cap}: wall {wall:.2} s is under the model {model:.2} s"
            );
        }
        bench("c_read_job_s", serde_json::json!(hold));
        bench("c_samples", serde_json::json!(samples));
        bench("c_queued_rows_sampled", serde_json::json!(queued_seen));
        bench("c_queued_without_reason", serde_json::json!(unexplained));
        assert_eq!(unexplained, 0);
        let _ = std::fs::remove_dir_all(trace);
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Scenario B (2.3, Foco de revisão 3): 4 `auto` jobs start together in
    /// one repository whose checkout has uncommitted work of the user (an
    /// edited tracked file, an untracked file), all editing the same file.
    /// The first keeps the folder; the others are placed in worktrees by the
    /// gate, carrying the user's work in through a private index. Goals: 4
    /// independent results (each folder holds only its own job's edit on top
    /// of the user's), 0 user edits lost, the user's index and HEAD
    /// untouched, creation < 1 s with no hook, and after the discard no
    /// worktree left in `git worktree list` while each job's work survives
    /// on its branch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bn2_b_auto_jobs_in_one_repo_get_worktrees_and_lose_nothing() {
        use crate::core::vcs::job_worktree;
        let Some(fake) = fake() else {
            return eprintln!("python3 missing: skipped");
        };
        // Held for the whole test: the env var is process-wide (`test_env`).
        let _data = crate::core::test_env::shared_data_dir();
        // Long enough that the job keeping the folder has not written yet
        // when the last worktree is cut (3 staggers + creation): a worktree
        // carries the folder as it is at that instant, the shared job's
        // edit included once it lands.
        let hold: f64 = env_num("BN2_B_HOLD_S", 4.0);
        let repo = temp("b-repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("notes.txt"), "base\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "node_modules\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "seed"]);
        std::fs::create_dir_all(repo.join("node_modules/dep")).unwrap();
        std::fs::write(repo.join("node_modules/dep/index.js"), "1\n").unwrap();
        // The user's own work in progress, never committed.
        std::fs::write(repo.join("notes.txt"), "base\nuser-wip\n").unwrap();
        std::fs::write(repo.join("draft.txt"), "user draft\n").unwrap();
        let index_before = std::fs::read(repo.join(".git/index")).unwrap();
        let head_before = git(&repo, &["rev-parse", "HEAD"]);

        let gate = Gate::new(Limits {
            max_parallel: 4,
            max_per_agent: None,
            stagger: STAGGER,
        });
        let folder = FolderKey::of(&repo);
        let t0 = Instant::now();
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let (gate, fake, repo, folder) =
                    (gate.clone(), fake.clone(), repo.clone(), folder.clone());
                tokio::spawn(async move {
                    let id = format!("j{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
                    let tag = format!("job{i}");
                    let ticket = Ticket {
                        job_id: id.clone(),
                        agent_id: "a".into(),
                        class: Class::Manual,
                        folder: Some(folder),
                    };
                    let (lease, isolated) = gate.acquire_placed(ticket, true).await;
                    tokio::time::sleep(lease.delay).await;
                    let (cwd, wt, create_ms) = if isolated {
                        let t = Instant::now();
                        let wt = job_worktree::create(&repo, &id, None).await.unwrap();
                        (
                            wt.path.clone(),
                            Some(wt),
                            t.elapsed().as_secs_f64() * 1000.0,
                        )
                    } else {
                        (repo.clone(), None, 0.0)
                    };
                    let started = t0.elapsed().as_secs_f64();
                    let mut stream = spawn_turn_with(
                        spec(
                            &fake,
                            &cwd,
                            &[
                                ("FAKE_CLAUDE_MODE", "write".into()),
                                ("FAKE_CLAUDE_HOLD_S", hold.to_string()),
                                ("FAKE_CLAUDE_TAG", tag.clone()),
                                (
                                    "FAKE_CLAUDE_LEDGER",
                                    cwd.join("notes.txt").display().to_string(),
                                ),
                                (
                                    "FAKE_CLAUDE_FILE",
                                    cwd.join(format!("{tag}.txt")).display().to_string(),
                                ),
                            ],
                        ),
                        Arc::new(ClaudeParser::new(true)),
                        CancellationToken::new(),
                        TurnHooks::default(),
                    )
                    .unwrap();
                    while stream.next().await.is_some() {}
                    let ended = t0.elapsed().as_secs_f64();
                    drop(lease);
                    (tag, cwd, wt, create_ms, started, ended)
                })
            })
            .collect();
        let done: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        let tags: Vec<String> = done.iter().map(|d| d.0.clone()).collect();
        let isolated = done.iter().filter(|d| d.2.is_some()).count();
        let overlapping = {
            let mut n = 0;
            for a in &done {
                for b in &done {
                    if a.0 < b.0 && a.4 < b.5 && b.4 < a.5 {
                        n += 1;
                    }
                }
            }
            n
        };
        // Independent results: every folder holds the user's work plus only
        // its own job's line and file.
        let mut independent = 0;
        let mut carried = 0;
        for (tag, cwd, wt, _, _, _) in &done {
            let notes = std::fs::read_to_string(cwd.join("notes.txt")).unwrap();
            let others = tags
                .iter()
                .filter(|t| *t != tag)
                .any(|t| notes.lines().any(|l| l == t) || cwd.join(format!("{t}.txt")).exists());
            if notes == format!("base\nuser-wip\n{tag}\n")
                && cwd.join(format!("{tag}.txt")).is_file()
                && !others
            {
                independent += 1;
            }
            if let Some(wt) = wt {
                // The job's own diff (against the commit that carried the
                // user's work) is only the job's two files.
                let mut dirty = job_worktree::dirty_files(&wt.path).await.unwrap();
                dirty.sort();
                let mut own = vec!["notes.txt".to_string(), format!("{tag}.txt")];
                own.sort();
                if dirty == own
                    && wt.carried.contains(&"notes.txt".to_string())
                    && wt.carried.contains(&"draft.txt".to_string())
                    && std::fs::read_to_string(wt.path.join("draft.txt")).unwrap() == "user draft\n"
                {
                    carried += 1;
                }
            }
        }
        let create_ms: Vec<f64> = done.iter().filter(|d| d.2.is_some()).map(|d| d.3).collect();
        let max_create = create_ms.iter().cloned().fold(0.0, f64::max);
        let linked = done
            .iter()
            .filter_map(|d| d.2.as_ref())
            .filter(|w| w.shared == vec!["node_modules".to_string()])
            .count();

        // Discard every worktree: the work goes to the branch, nothing of the
        // user's moves, no worktree is left behind.
        let mut preserved = 0;
        let mut saved = 0;
        for (tag, _, wt, _, _, _) in &done {
            let Some(wt) = wt else { continue };
            let out = job_worktree::remove(&repo, &wt.path, &wt.branch, Some(&wt.base_oid))
                .await
                .unwrap();
            if out.preserved_branch.as_deref() == Some(wt.branch.as_str()) {
                preserved += 1;
            }
            if out.saved.is_some()
                && git(&repo, &["show", &format!("{}:{tag}.txt", wt.branch)])
                    == format!("written by {tag}")
            {
                saved += 1;
            }
        }
        let worktrees_left = git(&repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|l| l.starts_with("worktree "))
            .count()
            - 1;
        let folders_left = done
            .iter()
            .filter_map(|d| d.2.as_ref())
            .filter(|w| w.path.exists())
            .count();
        let user_notes = std::fs::read_to_string(repo.join("notes.txt")).unwrap();
        let user_lost = usize::from(!user_notes.lines().any(|l| l == "user-wip"))
            + usize::from(
                std::fs::read_to_string(repo.join("draft.txt"))
                    .ok()
                    .as_deref()
                    != Some("user draft\n"),
            );
        let index_same = std::fs::read(repo.join(".git/index")).unwrap() == index_before;
        let head_same = git(&repo, &["rev-parse", "HEAD"]) == head_before;

        bench("b_jobs", serde_json::json!(done.len()));
        bench("b_isolated", serde_json::json!(isolated));
        bench("b_overlapping_pairs", serde_json::json!(overlapping));
        bench("b_independent_results", serde_json::json!(independent));
        bench("b_user_work_carried", serde_json::json!(carried));
        bench("b_user_edits_lost", serde_json::json!(user_lost));
        bench("b_user_index_untouched", serde_json::json!(index_same));
        bench("b_user_head_untouched", serde_json::json!(head_same));
        bench("b_create_ms", serde_json::json!(create_ms));
        bench("b_create_ms_max", serde_json::json!(max_create));
        bench("b_node_modules_linked", serde_json::json!(linked));
        bench(
            "b_worktrees_left_after_discard",
            serde_json::json!(worktrees_left),
        );
        bench(
            "b_folders_left_after_discard",
            serde_json::json!(folders_left),
        );
        bench("b_branches_preserved", serde_json::json!(preserved));
        bench("b_work_saved_on_branch", serde_json::json!(saved));
        assert_eq!(isolated, 3, "one job keeps the folder, three get worktrees");
        assert!(
            overlapping >= 3,
            "the four ran together ({overlapping} pairs)"
        );
        assert_eq!(independent, 4);
        assert_eq!(carried, 3);
        assert_eq!(user_lost, 0);
        assert!(index_same && head_same);
        assert_eq!((worktrees_left, folders_left), (0, 0));
        assert_eq!((preserved, saved), (3, 3));
        let _ = std::fs::remove_dir_all(&repo);
    }
}
