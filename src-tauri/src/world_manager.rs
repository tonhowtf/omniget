//! `WorldManager`: the one tick thread, the sleep states and the binary
//! channel. Owned by f7-world-bridge.
//!
//! Lazy on purpose. `AppState.world` is a `OnceLock` that stays empty until
//! somebody opens `/world`, and even a `WorldManager` that exists has no thread
//! until [`WorldManager::open`] is called: the budget line "app that never
//! opens `/world` = 0 ms CPU, 0 thread" (plan §1.2) is enforced by there being
//! nothing to run, not by a flag that something checks.
//!
//! One thread, named `omniget-world-tick`, drives everything:
//!
//! ```text
//!   lock ─ drain the bus ─ step()/catch_up() ─ diff_since() ─ encode ─ outbox
//!        └ unlock ─ send on the Channel ─ lock ─ wait(interval or forever)
//! ```
//!
//! The send happens with the lock released, and the outbox is bounded, so a
//! webview that stops draining the channel can never slow the simulation down;
//! it loses the oldest frames and gets a fresh snapshot instead of a diff with
//! a hole in it (see [`crate::commands::world::tick::Outbox`]).
//!
//! Nothing in here reads a clock the simulation can see: `World` is a pure
//! function of seed, map and inputs, and the wall clock only ever decides *when*
//! a tick happens and how much time `catch_up` has to skip.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use omniget_core::core::llm::brain::memory::{Embedder, InMemoryStore, MemoryStore};
use omniget_core::core::omni::bus::{Bus, BusEvent};
use omniget_world::{EntId, Input, Interest, MapDef, SleepState, World};

use crate::commands::world::brain::{default_embedder, Brains};
use crate::commands::world::brain_store::SqliteMemoryStore;
use crate::commands::world::input::{bus_input, energy_inputs, work_spots, BridgeInput, WorkSpots};
use crate::commands::world::save;
use crate::commands::world::tick::{decide_sleep, plan_for, sleep_tag, Outbox, TickPlan};

/// Live `omniget-world-tick` threads in this process. The "Never = 0 threads"
/// budget is a test, not a promise: see `commands::world::session::tests`.
static TICK_THREADS: AtomicUsize = AtomicUsize::new(0);

/// How many tick threads are running right now.
pub fn tick_threads() -> usize {
    TICK_THREADS.load(Ordering::Relaxed)
}

/// Bus events swallowed per wake-up, so a burst cannot turn one tick into a
/// long one.
const BUS_DRAIN_PER_TICK: usize = 64;

/// How often the quota windows are re-read into agent energy. Energy is an
/// input (plan §9.1) and a quota window moves in minutes, not in ticks.
const ENERGY_REFRESH_MS: u64 = 60_000;

/// How often the roster is compared with the cast of the house. A roster
/// changes when the user edits it, which is minutes apart at the very least.
const ROSTER_REFRESH_MS: u64 = 5_000;

/// How long an unanswered permission prompt keeps its agent waving.
const ASK_WAVE_MS: u64 = 120_000;

/// Entities from here up are visitors of an open house (`commands::world::house`),
/// never residents: the roster sync leaves them alone.
const VISITOR_ENT_BASE: u32 = 1000;

/// Emitted whenever what an agent is doing changes, with one [`AgentWork`] as
/// the payload. The activity panel of the route listens to it.
pub const EVENT_ACTIVITY: &str = "world://activity";

/// What one resident is doing on behalf of the app, as the bus tells it. The
/// simulation knows the pose; this is the reason for the pose.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct AgentWork {
    pub agent: String,
    pub ent: u32,
    /// Between `TurnStarted` and `TurnEnded`.
    pub working: bool,
    /// Waiting for the user to answer a permission prompt.
    pub asking: bool,
    /// Last tool called in this turn, with its caption (`fs_edit cart.js`).
    pub tool: String,
    pub caption: String,
    pub conversation: String,
    pub tools_called: u32,
    pub budget_hit: bool,
    /// Where the body is and what tick the house is at, filled in by
    /// [`WorldManager::work`] only.
    pub tile: Option<(i32, i32)>,
    pub tick: u64,
}

/// Where diffs go. A trait rather than `tauri::ipc::Channel` so the whole
/// manager is testable without a webview.
pub trait DiffSink: Send + Sync {
    fn send(&self, bytes: Vec<u8>) -> Result<(), String>;
}

/// What the quota side of the app can tell us: `(account label, fraction of the
/// window already used)`.
pub type QuotaSource = Arc<dyn Fn() -> Vec<(String, f32)> + Send + Sync>;

/// The ids of the roster, in roster order. Every one of them lives in the
/// house, whatever its runtime (native, CLI, ACP).
pub type RosterSource = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Counters the tests and the bench read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub ticks: u64,
    pub steps: u64,
    pub catch_ups: u64,
    pub caught_up_ticks: u64,
    pub sent: u64,
    pub dropped: u64,
    pub resyncs: u64,
    pub snapshots: u64,
    pub diffs: u64,
    pub diff_bytes: u64,
    pub saves: u64,
    pub rejected_inputs: u64,
}

impl WorldStats {
    /// Mean size of the diffs actually sent.
    pub fn mean_diff_bytes(&self) -> f64 {
        if self.diffs == 0 {
            0.0
        } else {
            self.diff_bytes as f64 / self.diffs as f64
        }
    }

    /// Mean bytes per tick, counting the ticks that sent nothing. This is the
    /// number the budget names (≤ 2 KB at 10 Hz with 8 agents), because an
    /// empty tick really does cost nothing on the wire.
    pub fn mean_bytes_per_tick(&self) -> f64 {
        if self.steps == 0 {
            0.0
        } else {
            self.diff_bytes as f64 / self.steps as f64
        }
    }
}

struct Inner {
    world: Option<World>,
    map: Option<MapDef>,
    house: String,
    sleep: SleepState,
    route_visible: bool,
    /// When the app went to the background, for the ten-minute hibernation.
    background_since: Option<Instant>,
    /// When hibernation started, so waking knows how much to skip.
    hibernating_since: Option<Instant>,
    sink: Option<Arc<dyn DiffSink>>,
    /// Second listener: the open house (the room server). Gets the very same
    /// blobs as the route, so a visitor decodes what the host decodes.
    net_sink: Option<Arc<dyn DiffSink>>,
    outbox: Outbox,
    interest: Interest,
    /// Tick the client is known to have; `None` means it needs a snapshot.
    last_sent: Option<u64>,
    pending: Vec<Input>,
    /// What the brains decided, waiting for the next step. Separate from
    /// [`Inner::pending`] so that "what the outside sent" stays readable.
    from_minds: Vec<Input>,
    names: BTreeMap<String, EntId>,
    spots: WorkSpots,
    /// What the bus said each resident is doing, by roster name.
    work: BTreeMap<String, AgentWork>,
    /// Changes of `work` not yet emitted; drained with the lock released.
    work_changed: Vec<AgentWork>,
    /// What the last permission prompt of an agent showed, for the caption of
    /// the tool call that follows it.
    ask_preview: BTreeMap<String, String>,
    /// Who is waving at the camera, and since when.
    asking: BTreeMap<EntId, Instant>,
    /// Residents that are not in the roster on purpose (the demo's extras).
    guests: BTreeSet<String>,
    last_roster: Instant,
    /// One brain per agent. Empty until a world is installed and empty again
    /// the moment it is deleted: "no world = nothing allocated" holds for the
    /// minds as much as for the thread.
    brains: Brains,
    /// Shared by every brain of the house. `None` until a world exists, so a
    /// process that never opens `/world` never opens a database either.
    store: Option<Arc<dyn MemoryStore>>,
    embedder: Option<Arc<dyn Embedder>>,
    dirty: bool,
    last_save: Instant,
    last_energy: Instant,
    next_due: Instant,
    stop: bool,
    stats: WorldStats,
    sleep_changed: Option<SleepState>,
}

impl Inner {
    fn new() -> Inner {
        let now = Instant::now();
        Inner {
            world: None,
            map: None,
            house: String::new(),
            sleep: SleepState::Dozing,
            route_visible: false,
            background_since: None,
            hibernating_since: None,
            sink: None,
            net_sink: None,
            outbox: Outbox::default(),
            interest: Interest::ALL,
            last_sent: None,
            pending: Vec::new(),
            from_minds: Vec::new(),
            names: BTreeMap::new(),
            spots: WorkSpots::default(),
            work: BTreeMap::new(),
            work_changed: Vec::new(),
            ask_preview: BTreeMap::new(),
            asking: BTreeMap::new(),
            guests: BTreeSet::new(),
            last_roster: now,
            brains: Brains::new(),
            store: None,
            embedder: None,
            dirty: false,
            last_save: now,
            last_energy: now,
            next_due: now,
            stop: false,
            stats: WorldStats::default(),
            sleep_changed: None,
        }
    }

    fn background_ms(&self, now: Instant) -> Option<u64> {
        self.background_since
            .map(|t| now.saturating_duration_since(t).as_millis() as u64)
    }

    /// The memory store and the embedder the brains share, built on first use.
    /// A process that never installs a world never opens the database.
    fn ensure_minds(&mut self, root: &Path) {
        if self.store.is_none() {
            self.store = Some(
                SqliteMemoryStore::shared(root)
                    .unwrap_or_else(|| Arc::new(InMemoryStore::new()) as Arc<dyn MemoryStore>),
            );
        }
        if self.embedder.is_none() {
            self.embedder = Some(default_embedder());
        }
    }

    /// Re-read the work spots, the host agent and the cast of the house from
    /// the world. Cheap enough for the handful of moments somebody arrives,
    /// leaves, or an object is moved — never per tick.
    fn refresh_cast(&mut self) {
        let Some(w) = self.world.as_ref() else {
            self.spots = WorkSpots::default();
            self.brains.clear();
            return;
        };
        let snap = w.snapshot();
        let placed: Vec<_> = snap
            .objects
            .iter()
            .map(|o| (o.id, o.kind.clone(), o.tile))
            .collect();
        let objects: Vec<_> = snap.objects.into_iter().map(|o| (o.id, o.kind)).collect();
        let agents: Vec<_> = snap.agents.into_iter().map(|a| (a.id, a.name)).collect();
        // Visitors walk around; only residents have a post.
        let residents: Vec<_> = agents
            .iter()
            .filter(|(id, _)| id.0 < VISITOR_ENT_BASE)
            .cloned()
            .collect();
        self.spots = work_spots(&placed, &residents);
        if let (Some(store), Some(embedder)) = (self.store.clone(), self.embedder.clone()) {
            self.brains.rebuild(&agents, &objects, &store, &embedder);
        }
    }

    fn note_input(&mut self, input: &Input) {
        match input {
            Input::Spawn { ent, name, .. } => {
                self.names.insert(name.clone(), *ent);
            }
            Input::Despawn { ent } => {
                self.names.retain(|_, v| v != ent);
                self.asking.remove(ent);
                let gone: Vec<String> = self
                    .work
                    .iter()
                    .filter(|(_, w)| w.ent == ent.0)
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in gone {
                    self.work.remove(&k);
                }
            }
            _ => {}
        }
    }

    /// Move to the state the world should be in, catching up whatever
    /// hibernation skipped.
    fn settle_sleep(&mut self, now: Instant) {
        // An open house has an audience even when the host's own window is
        // covered or in the tray: visitors must keep getting diffs.
        let watched = self.route_visible || self.net_sink.is_some();
        // ...and the house does not hibernate under the visitors' feet.
        let background = if self.net_sink.is_some() {
            None
        } else {
            self.background_ms(now)
        };
        let want = decide_sleep(watched, background);
        if want == self.sleep {
            return;
        }
        let was_hibernating = self.sleep == SleepState::Hibernating;
        self.sleep = want;
        if let Some(w) = self.world.as_mut() {
            w.set_sleep(want);
        }
        if want == SleepState::Hibernating {
            self.hibernating_since = Some(now);
        } else if was_hibernating {
            let slept = self
                .hibernating_since
                .take()
                .map(|t| now.saturating_duration_since(t).as_millis() as u64)
                .unwrap_or(0);
            if let Some(w) = self.world.as_mut() {
                if slept >= omniget_world::TICK_MS {
                    let report = w.catch_up(slept);
                    self.stats.catch_ups += 1;
                    self.stats.caught_up_ticks += report.caught_up;
                    self.dirty = true;
                }
            }
            self.next_due = now;
        }
        self.sleep_changed = Some(want);
    }

    /// One wake-up of the thread: maybe step, maybe encode, maybe save.
    fn pump(&mut self, now: Instant, root: &Path) -> Vec<Vec<u8>> {
        self.settle_sleep(now);
        if self.world.is_none() {
            self.next_due = now + Duration::from_secs(3600);
            return Vec::new();
        }
        let plan = plan_for(self.sleep);
        if plan == TickPlan::Idle {
            // Hibernating: nothing runs and nothing is scheduled. Only a
            // notify (the route coming back, the app being focused) restarts it.
            return self.outbox.take();
        }
        if now < self.next_due {
            return self.outbox.take();
        }
        let interval = self.sleep.interval_ms().unwrap_or(omniget_world::TICK_MS);
        self.next_due = now + Duration::from_millis(interval);

        let mut inputs = std::mem::take(&mut self.pending);
        inputs.extend(self.keep_waving(now));
        // What the minds decided on the previous wake-up rides in with what the
        // outside submitted. They are kept in two queues so that `pending`
        // stays "what somebody sent us", which is what the input tests read.
        // An agent in the middle of a turn is at its post because the app put
        // it there; its own agenda waits until the turn is over.
        let busy: BTreeSet<u32> = self
            .work
            .values()
            .filter(|w| w.working || w.asking)
            .map(|w| w.ent)
            .collect();
        self.from_minds.retain(|i| match i {
            Input::Decision { ent, .. } => !busy.contains(&ent.0),
            _ => true,
        });
        inputs.append(&mut self.from_minds);
        let touches_cast = inputs.iter().any(|i| {
            matches!(
                i,
                Input::PlaceObject { .. }
                    | Input::RemoveObject { .. }
                    | Input::Spawn { .. }
                    | Input::Despawn { .. }
            )
        });
        // Energy is an input to the world *and* news for the mind that lives in
        // that body: it is what makes an agent yawn and what turns a renewed
        // quota window into `WorldEvent::Dawn`. Fed in before the step so the
        // yawn rides on the very next tick.
        let mut from_minds = Vec::new();
        for input in &inputs {
            if let Input::SetEnergy { ent, energy } = input {
                from_minds.extend(self.brains.set_energy(*ent, *energy));
            }
        }
        let world = self.world.as_mut().expect("checked above");
        let report = world.step(&inputs);
        self.stats.steps += 1;
        self.stats.ticks = report.tick;
        self.stats.rejected_inputs += report.rejected as u64;
        // What just happened, for the memories. Taken here and not after the
        // catch-up: `catch_up` clears the event list before it writes its own
        // `CaughtUp`, and a dozing house catches up after every single step.
        let mut events: Vec<_> = world.last_events().to_vec();
        if let TickPlan::StepThenCatchUp { ms } = plan {
            let r = world.catch_up(ms);
            self.stats.catch_ups += 1;
            self.stats.caught_up_ticks += r.caught_up;
            self.stats.ticks = r.tick;
            events.extend(world.last_events().iter().cloned());
        }
        let tick = world.tick();
        if !inputs.is_empty() || report.moved > 0 || report.events > 0 {
            self.dirty = true;
        }
        if touches_cast {
            self.refresh_cast();
        }
        // The brain half of the tick, and the whole of it: observe, then let the
        // agenda answer. Both are pure — no model is called, so a running house
        // costs no tokens and touches no network (see `commands::world::brain`).
        if !self.brains.is_empty() {
            self.brains.observe(tick, &events);
            self.from_minds.extend(self.brains.tick_hints(tick));
            self.from_minds.extend(from_minds);
        }

        self.encode_outgoing();
        self.maybe_save(now, root);
        self.outbox.take()
    }

    /// Queue whatever the client still has to hear about.
    fn encode_outgoing(&mut self) {
        if self.sink.is_none() && self.net_sink.is_none() {
            // Nobody is listening: keep no backlog at all.
            self.last_sent = None;
            self.outbox.reset();
            return;
        }
        let world = self.world.as_ref().expect("caller checked");
        let tick = world.tick();
        let resync = self.outbox.needs_resync() || self.last_sent.is_none();
        if !resync {
            let from = self.last_sent.expect("not a resync");
            match world.diff_since(from, &self.interest) {
                Ok(diff) => {
                    if diff.is_empty() {
                        // Nothing this client can see changed, so nothing is
                        // sent — but the client *is* up to date at this tick,
                        // and saying so keeps it inside the 64-tick history. A
                        // house that idles for seven seconds must not cost a
                        // whole snapshot when somebody finally moves.
                        self.last_sent = Some(tick);
                        return;
                    }
                    let blob = diff.encode();
                    self.stats.diffs += 1;
                    self.stats.diff_bytes += blob.len() as u64;
                    self.outbox.push(blob);
                    self.last_sent = Some(tick);
                    return;
                }
                // The client fell out of the 64-tick history: a snapshot is the
                // only honest answer.
                Err(_) => self.outbox.arm_resync(),
            }
        }
        let blob = world.snapshot().encode();
        self.stats.snapshots += 1;
        self.outbox.push(blob);
        self.outbox.clear_resync();
        self.last_sent = Some(tick);
    }

    fn maybe_save(&mut self, now: Instant, root: &Path) {
        let elapsed = now.saturating_duration_since(self.last_save).as_millis() as u64;
        if !save::save_due(self.dirty, elapsed) {
            return;
        }
        self.save_now(root);
        self.last_save = now;
    }

    fn save_now(&mut self, root: &Path) {
        let Some(w) = self.world.as_ref() else {
            return;
        };
        let bytes = w.snapshot().encode();
        match save::write_save(root, &bytes) {
            Ok(()) => {
                self.dirty = false;
                self.stats.saves += 1;
            }
            Err(e) => tracing::warn!("[world] save failed: {e}"),
        }
    }

    /// A wave lasts a second; a permission prompt lasts until somebody answers
    /// it. Whoever is still waiting waves again as soon as the last one ends.
    fn keep_waving(&mut self, now: Instant) -> Vec<Input> {
        if self.asking.is_empty() {
            return Vec::new();
        }
        self.asking.retain(|_, since| {
            now.saturating_duration_since(*since).as_millis() < ASK_WAVE_MS as u128
        });
        let Some(w) = self.world.as_ref() else {
            return Vec::new();
        };
        self.asking
            .keys()
            .filter(|ent| {
                !matches!(
                    w.activity_of(**ent),
                    Some(omniget_world::ents::agent::Activity::Waving(_)) | None
                )
            })
            .map(|ent| Input::Decision {
                ent: *ent,
                decision: omniget_world::Decision::Wave(*ent),
            })
            .collect()
    }

    /// Fold a bus event into "what is this resident doing", for the panel.
    fn note_work(&mut self, ev: &BusEvent) {
        let agent = match ev {
            BusEvent::TurnStarted { agent, .. }
            | BusEvent::ToolAsk { agent, .. }
            | BusEvent::ToolCalled { agent, .. }
            | BusEvent::TurnEnded { agent, .. }
            | BusEvent::BudgetHit { agent } => agent,
            _ => return,
        };
        let Some(ent) = self.names.get(agent).copied() else {
            return;
        };
        let preview = self.ask_preview.get(agent).cloned().unwrap_or_default();
        let w = self.work.entry(agent.clone()).or_default();
        w.agent = agent.clone();
        w.ent = ent.0;
        match ev {
            BusEvent::TurnStarted { conversation, .. } => {
                w.working = true;
                w.asking = false;
                w.budget_hit = false;
                w.tool.clear();
                w.caption.clear();
                w.tools_called = 0;
                w.conversation = conversation.clone();
            }
            BusEvent::ToolAsk { tool, preview, .. } => {
                w.asking = true;
                w.tool = tool.clone();
                w.caption = crate::commands::world::input::tool_caption(tool, preview);
                self.asking.insert(ent, Instant::now());
                self.ask_preview.insert(agent.clone(), preview.clone());
            }
            BusEvent::ToolCalled { tool, .. } => {
                w.asking = false;
                w.tool = tool.clone();
                w.caption = crate::commands::world::input::tool_caption(tool, &preview);
                w.tools_called += 1;
                self.asking.remove(&ent);
                self.ask_preview.remove(agent);
            }
            BusEvent::TurnEnded { .. } => {
                w.working = false;
                w.asking = false;
                self.asking.remove(&ent);
                self.ask_preview.remove(agent);
            }
            BusEvent::BudgetHit { .. } => w.budget_hit = true,
            _ => {}
        }
        let w = w.clone();
        self.work_changed.push(w);
    }

    fn ingest_bus(&mut self, events: &[BusEvent]) {
        if self.world.is_none() || events.is_empty() {
            return;
        }
        let tick = self.world.as_ref().map(|w| w.tick()).unwrap_or(0);
        for ev in events {
            let arg = match ev {
                BusEvent::ToolCalled { agent, .. } => self.ask_preview.get(agent).cloned(),
                _ => None,
            };
            self.pending
                .extend(bus_input(ev, &self.names, &self.spots, arg.as_deref()));
            self.note_work(ev);
            // The body is moved by `bus_input` above; the mind only remembers.
            // Doing both from the brain would put the same decision in the
            // mailbox twice (see `commands::world::brain`).
            self.brains.observe_bus(tick, ev);
        }
    }

    fn wait(&self, now: Instant) -> Option<Duration> {
        if self.sleep == SleepState::Hibernating {
            return None;
        }
        Some(self.next_due.saturating_duration_since(now))
    }
}

struct Shared {
    inner: Mutex<Inner>,
    cv: Condvar,
    bus: Mutex<Option<Arc<Bus>>>,
    quota: Mutex<Option<QuotaSource>>,
    roster: Mutex<Option<RosterSource>>,
    app: Mutex<Option<tauri::AppHandle>>,
    running: Mutex<bool>,
    /// True between the spawn and the last line of [`run`]. Watched by
    /// `stop_thread` so a delete followed by a create cannot end up with two
    /// threads on the same world.
    alive: std::sync::atomic::AtomicBool,
    /// Builds the LLM-backed thinker for an agent name; `None` = thinking is off
    /// (the default) or that agent has no roster entry. Asked every time, so the
    /// Settings toggle is live.
    thinker: Mutex<Option<ThinkerFactory>>,
    think_busy: std::sync::atomic::AtomicBool,
    last_think: Mutex<Option<Instant>>,
}

/// `(agent name) -> (thinker, minimum seconds between thoughts)`.
pub type ThinkerFactory = Arc<
    dyn Fn(&str) -> Option<(Arc<dyn omniget_core::core::llm::brain::Thinker>, u32)> + Send + Sync,
>;

/// The world's whole presence in the app process.
pub struct WorldManager {
    root: PathBuf,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for WorldManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.lock();
        f.debug_struct("WorldManager")
            .field("root", &self.root)
            .field("house", &g.house)
            .field("has_world", &g.world.is_some())
            .field("sleep", &g.sleep)
            .field("brains", &g.brains.len())
            .field("thread", &self.shared.alive.load(Ordering::SeqCst))
            .finish()
    }
}

impl WorldManager {
    /// A manager with no world and no thread. Building one costs an
    /// allocation; it does not start anything.
    pub fn new(root: PathBuf) -> WorldManager {
        WorldManager {
            root,
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner::new()),
                cv: Condvar::new(),
                bus: Mutex::new(None),
                quota: Mutex::new(None),
                roster: Mutex::new(None),
                app: Mutex::new(None),
                running: Mutex::new(false),
                alive: std::sync::atomic::AtomicBool::new(false),
                thinker: Mutex::new(None),
                think_busy: std::sync::atomic::AtomicBool::new(false),
                last_think: Mutex::new(None),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Hand over the bus (F2) so the tick thread can turn turns, tool calls and
    /// downloads into work in the house.
    pub fn attach_bus(&self, bus: Arc<Bus>) {
        *self.shared.bus.lock().expect("bus lock") = Some(bus);
    }

    /// Hand over the app so `world://sleep-state` can be emitted.
    pub fn attach_app(&self, app: tauri::AppHandle) {
        *self.shared.app.lock().expect("app lock") = Some(app);
    }

    /// Where agent energy comes from (plan §9.1).
    /// Attaches the LLM side of the minds. Without it the house runs on the
    /// schedule alone and makes zero model calls.
    pub fn set_thinker(&self, factory: ThinkerFactory) {
        *self.shared.thinker.lock().expect("thinker lock") = Some(factory);
    }

    pub fn set_quota_source(&self, source: QuotaSource) {
        *self.shared.quota.lock().expect("quota lock") = Some(source);
    }

    /// Who lives in the house: the roster. Without it nobody moves in or out
    /// on their own, which is what the tests want.
    pub fn set_roster_source(&self, source: RosterSource) {
        *self.shared.roster.lock().expect("roster lock") = Some(source);
    }

    /// Make the cast of the house match the roster: `Spawn` whoever is missing
    /// (name = roster id, which is how bus events find their agent), `Despawn`
    /// whoever left. Returns `(moved in, moved out)`.
    pub fn sync_roster(&self) -> (Vec<String>, Vec<String>) {
        let changed = sync_roster(&self.shared);
        if !changed.0.is_empty() || !changed.1.is_empty() {
            self.shared.cv.notify_all();
        }
        changed
    }

    /// A house saved before every agent had a post of its own has one
    /// workbench. Where the map now defaults a slot to a workbench and the save
    /// still holds the old default, the workbench takes its place — as long as
    /// the house has fewer than two, so a player who arranged their own posts
    /// is left alone.
    pub fn upgrade_posts(&self) {
        {
            let mut g = self.lock();
            let (Some(w), Some(map)) = (g.world.as_ref(), g.map.as_ref()) else {
                return;
            };
            let objects = w.snapshot().objects;
            let benches = objects
                .iter()
                .filter(|o| o.kind == "object/workbench")
                .count();
            if benches >= 2 {
                return;
            }
            let mut inputs = Vec::new();
            for slot in map
                .slots
                .iter()
                .filter(|s| s.default.as_deref() == Some("object/workbench"))
            {
                let Some(old) = objects
                    .iter()
                    .find(|o| o.slot.as_deref() == Some(slot.id.as_str()))
                else {
                    continue;
                };
                if old.kind == "object/workbench" {
                    continue;
                }
                inputs.push(Input::PlaceObject {
                    object: old.id,
                    kind: "object/workbench".into(),
                    tile: old.tile,
                    dir: old.dir,
                    slot: Some(slot.id.clone()),
                });
            }
            if inputs.is_empty() {
                return;
            }
            g.pending.extend(inputs);
        }
        self.shared.cv.notify_all();
    }

    /// A body the roster does not know, for the length of a demo. Spawned at
    /// the door like everybody else and exempt from the roster sync.
    pub fn add_guest(&self, name: &str) {
        {
            let mut g = self.lock();
            if g.world.is_none() || g.names.contains_key(name) {
                return;
            }
            let spawn = g
                .map
                .as_ref()
                .and_then(|m| m.markers.iter().find(|m| m.name == "spawn"))
                .map(|m| m.tile)
                .unwrap_or(omniget_world::Tile::new(1, 1));
            let used: BTreeSet<u32> = g.names.values().map(|e| e.0).collect();
            let Some(id) = (1..VISITOR_ENT_BASE).find(|i| !used.contains(i)) else {
                return;
            };
            g.guests.insert(name.to_string());
            let input = Input::Spawn {
                ent: EntId(id),
                name: name.to_string(),
                at: omniget_world::Tile::new(spawn.x + (id as i32 - 1) % 6, spawn.y),
            };
            g.note_input(&input);
            g.pending.push(input);
        }
        self.shared.cv.notify_all();
    }

    /// The demo is over: the guest leaves.
    pub fn remove_guest(&self, name: &str) {
        {
            let mut g = self.lock();
            g.guests.remove(name);
            let Some(ent) = g.names.get(name).copied() else {
                return;
            };
            let input = Input::Despawn { ent };
            g.note_input(&input);
            g.pending.push(input);
        }
        self.shared.cv.notify_all();
    }

    /// Residents by roster name, visitors left out.
    pub fn residents(&self) -> Vec<(String, EntId)> {
        let g = self.lock();
        let mut v: Vec<_> = g
            .names
            .iter()
            .filter(|(_, e)| e.0 < VISITOR_ENT_BASE)
            .map(|(n, e)| (n.clone(), *e))
            .collect();
        v.sort_by_key(|(_, e)| *e);
        v
    }

    /// What every resident is doing, for a panel that just mounted.
    pub fn work(&self) -> Vec<AgentWork> {
        let g = self.lock();
        let mut v: Vec<_> = g
            .names
            .iter()
            .filter(|(_, e)| e.0 < VISITOR_ENT_BASE)
            .map(|(name, ent)| {
                let mut w = g.work.get(name).cloned().unwrap_or_else(|| AgentWork {
                    agent: name.clone(),
                    ent: ent.0,
                    ..AgentWork::default()
                });
                w.tick = g.world.as_ref().map(|world| world.tick()).unwrap_or(0);
                w.tile = g
                    .world
                    .as_ref()
                    .and_then(|world| world.tile_of(*ent))
                    .map(|t| (t.x, t.y));
                w
            })
            .collect();
        v.sort_by_key(|w| w.ent);
        v
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn has_world(&self) -> bool {
        self.lock().world.is_some()
    }

    pub fn house(&self) -> String {
        self.lock().house.clone()
    }

    /// The map this world was built from. Phase 9's room server needs it to
    /// stand up a replica; here it is what `restore` validated against.
    pub fn map_def(&self) -> Option<MapDef> {
        self.lock().map.clone()
    }

    pub fn stats(&self) -> WorldStats {
        let g = self.lock();
        let mut s = g.stats;
        s.sent = g.outbox.sent();
        s.dropped = g.outbox.dropped();
        s.resyncs = g.outbox.resyncs();
        s
    }

    /// Is *this* manager's tick thread alive. The process-wide count is
    /// [`tick_threads`]; this is the per-manager answer, which is what a test
    /// running beside other tests can assert on.
    pub fn thread_running(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// How many agents in this house have a mind. Zero when there is no world.
    pub fn brain_count(&self) -> usize {
        self.lock().brains.len()
    }

    /// Model calls every brain in this house has made. The whole of phase 7's
    /// token budget is "this is zero": no `Thinker` is attached, so the world
    /// runs on its agenda alone.
    pub fn brain_model_calls(&self) -> u32 {
        self.lock().brains.model_calls()
    }

    /// Memories the house holds, across agents.
    pub fn memory_count(&self) -> usize {
        self.lock().brains.memory_count()
    }

    pub fn sleep_state(&self) -> SleepState {
        self.lock().sleep
    }

    pub fn tick(&self) -> u64 {
        self.lock().world.as_ref().map(|w| w.tick()).unwrap_or(0)
    }

    /// Build a brand new world and write it down straight away, so a crash
    /// before the first autosave does not lose the house the user just made.
    pub fn create(&self, seed: u64, map: MapDef, house: &str) -> Result<(), String> {
        let world = World::new(seed, map.clone()).map_err(|e| format!("{}: {e}", e.code()))?;
        self.install(world, map, house);
        let mut g = self.lock();
        g.save_now(&self.root);
        Ok(())
    }

    /// Restore the world from `<app_data>/world/save.bin`.
    pub fn restore(&self, bytes: &[u8], map: MapDef, house: &str) -> Result<(), String> {
        let world = save::restore_bytes(bytes, map.clone())?;
        self.install(world, map, house);
        Ok(())
    }

    fn install(&self, mut world: World, map: MapDef, house: &str) {
        let mut g = self.lock();
        g.ensure_minds(&self.root);
        for a in world.snapshot().agents {
            g.names.insert(a.name, a.id);
        }
        // The world's own idea of its sleep state travels in every snapshot, so
        // it starts out agreeing with the bridge instead of with its default.
        world.set_sleep(g.sleep);
        g.world = Some(world);
        g.map = Some(map);
        g.house = house.to_string();
        g.last_sent = None;
        g.outbox.reset();
        g.dirty = false;
        g.next_due = Instant::now();
        g.refresh_cast();
        drop(g);
        self.shared.cv.notify_all();
    }

    /// Attach the channel, mark the world Active and start the thread if it is
    /// not running yet. Returns the snapshot the route starts from — the client
    /// is at that tick, so the channel carries diffs from then on.
    pub fn open(&self, sink: Arc<dyn DiffSink>) -> Result<Vec<u8>, String> {
        let (bytes, changed) = {
            let mut g = self.lock();
            if g.world.is_none() {
                return Err(crate::commands::world::ERR_NO_WORLD.to_string());
            }
            g.route_visible = true;
            g.background_since = None;
            // Active *before* the snapshot is taken: the blob the route decodes
            // must already say the world is awake.
            g.settle_sleep(Instant::now());
            let world = g.world.as_ref().expect("checked above");
            let bytes = world.snapshot().encode();
            let tick = world.tick();
            g.sink = Some(sink);
            g.outbox.reset();
            g.last_sent = Some(tick);
            g.next_due = Instant::now();
            (bytes, g.sleep_changed.take())
        };
        self.start_thread();
        self.shared.cv.notify_all();
        if let Some(state) = changed {
            self.emit_sleep(state);
        }
        Ok(bytes)
    }

    /// The route went away: drop the channel and fall back to 0.2 Hz.
    pub fn close(&self) {
        {
            let mut g = self.lock();
            g.route_visible = false;
            g.sink = None;
            g.outbox.reset();
            g.last_sent = None;
            g.save_now(&self.root);
        }
        self.settle();
    }

    /// The route is still mounted but hidden, or visible again.
    pub fn set_visible(&self, visible: bool) {
        {
            let mut g = self.lock();
            g.route_visible = visible;
            if visible {
                g.background_since = None;
                g.next_due = Instant::now();
            }
        }
        self.settle();
    }

    /// The app window lost or regained the foreground. Ten minutes in the
    /// background is what hibernation counts.
    pub fn note_background(&self, backgrounded: bool) {
        {
            let mut g = self.lock();
            if backgrounded {
                if g.background_since.is_none() {
                    g.background_since = Some(Instant::now());
                }
            } else {
                g.background_since = None;
                g.next_due = Instant::now();
            }
        }
        self.settle();
    }

    /// Queue an input for the next tick, or change what this client is
    /// interested in.
    pub fn submit(&self, input: BridgeInput) {
        {
            let mut g = self.lock();
            match input {
                BridgeInput::Interest(i) => {
                    if g.interest != i {
                        g.interest = i;
                        // A different circle means the client's picture is no
                        // longer a function of the diffs it has: re-sync.
                        g.outbox.arm_resync();
                    }
                }
                BridgeInput::World(input) => {
                    g.note_input(&input);
                    g.pending.push(*input);
                }
            }
        }
        self.shared.cv.notify_all();
    }

    /// The open house listens to the same stream as the route. `None` closes it.
    pub fn set_net_sink(&self, sink: Option<Arc<dyn DiffSink>>) {
        let mut g = self.lock();
        g.net_sink = sink;
        g.outbox.arm_resync();
        drop(g);
        self.shared.cv.notify_all();
    }

    /// Someone new is watching: the next blob is a whole snapshot.
    pub fn resync(&self) {
        let mut g = self.lock();
        g.outbox.arm_resync();
        drop(g);
        self.shared.cv.notify_all();
    }

    /// Write the world down now (close, quit).
    pub fn save_now(&self) {
        let mut g = self.lock();
        g.save_now(&self.root);
    }

    /// Stop the thread, forget the world and erase it from disk.
    pub fn delete(&self) -> Result<(), String> {
        self.stop_thread();
        {
            let mut g = self.lock();
            g.world = None;
            g.map = None;
            g.sink = None;
            g.names.clear();
            g.spots = WorkSpots::default();
            g.work.clear();
            g.work_changed.clear();
            g.ask_preview.clear();
            g.asking.clear();
            g.guests.clear();
            // The minds go with the house, and the database file goes with the
            // directory `delete_world` removes below.
            g.brains.clear();
            g.store = None;
            g.embedder = None;
            g.last_sent = None;
            g.outbox.reset();
            g.dirty = false;
        }
        save::delete_world(&self.root)
    }

    /// Shut the thread down without touching the save. Called on app exit.
    pub fn shutdown(&self) {
        self.save_now();
        self.stop_thread();
    }

    fn settle(&self) {
        let changed = {
            let mut g = self.lock();
            let now = Instant::now();
            g.settle_sleep(now);
            g.sleep_changed.take()
        };
        self.shared.cv.notify_all();
        if let Some(state) = changed {
            self.emit_sleep(state);
        }
    }

    fn emit_sleep(&self, state: SleepState) {
        let app = self.shared.app.lock().expect("app lock").clone();
        if let Some(app) = app {
            use tauri::Emitter;
            let _ = app.emit(
                crate::commands::world::EVENT_SLEEP_STATE,
                serde_json::json!({ "state": sleep_tag(state) }),
            );
        }
    }

    fn start_thread(&self) {
        let mut running = self.shared.running.lock().expect("running lock");
        if *running {
            return;
        }
        *running = true;
        self.lock().stop = false;
        let shared = self.shared.clone();
        let root = self.root.clone();
        TICK_THREADS.fetch_add(1, Ordering::Relaxed);
        self.shared.alive.store(true, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("omniget-world-tick".into())
            .spawn(move || run(shared, root));
        if spawned.is_err() {
            TICK_THREADS.fetch_sub(1, Ordering::Relaxed);
            self.shared.alive.store(false, Ordering::SeqCst);
            *running = false;
            tracing::error!("[world] could not spawn the tick thread");
        }
    }

    fn stop_thread(&self) {
        let was_running = {
            let mut running = self.shared.running.lock().expect("running lock");
            let was = *running;
            *running = false;
            was
        };
        if !was_running {
            return;
        }
        self.lock().stop = true;
        self.shared.cv.notify_all();
        // The thread checks `stop` on every wake-up; give it a moment to leave
        // so a delete followed by a create does not run two ticks at once.
        for _ in 0..400 {
            if !self.shared.alive.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for WorldManager {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

fn run(shared: Arc<Shared>, root: PathBuf) {
    let mut bus_rx = shared
        .bus
        .lock()
        .expect("bus lock")
        .as_ref()
        .map(|b| b.subscribe());
    loop {
        // The bus is drained with the state lock released: a lagging broadcast
        // receiver must never be something the tick waits on.
        let mut events = Vec::new();
        if let Some(rx) = bus_rx.as_mut() {
            for _ in 0..BUS_DRAIN_PER_TICK {
                match rx.try_recv() {
                    Ok(e) => events.push(e),
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }
        let quota = refresh_quota(&shared);
        refresh_roster(&shared, &events);

        let net_sink;
        let work_changed;
        let (outgoing, sink, wait, changed) = {
            let mut g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.stop {
                break;
            }
            let now = Instant::now();
            g.ingest_bus(&events);
            if let Some(quotas) = quota {
                let names = g.names.clone();
                g.pending.extend(energy_inputs(&quotas, &names));
            }
            let outgoing = g.pump(now, &root);
            let wait = g.wait(Instant::now());
            net_sink = g.net_sink.clone();
            work_changed = std::mem::take(&mut g.work_changed);
            (outgoing, g.sink.clone(), wait, g.sleep_changed.take())
        };
        if let Some(net) = &net_sink {
            for blob in &outgoing {
                if net.send(blob.clone()).is_err() {
                    let mut g = shared.inner.lock().unwrap_or_else(|x| x.into_inner());
                    g.net_sink = None;
                    break;
                }
            }
        }

        maybe_think(&shared);

        if !work_changed.is_empty() {
            let app = shared.app.lock().expect("app lock").clone();
            if let Some(app) = app {
                use tauri::Emitter;
                for w in &work_changed {
                    let _ = app.emit(EVENT_ACTIVITY, w);
                }
            }
        }

        if let Some(state) = changed {
            let app = shared.app.lock().expect("app lock").clone();
            if let Some(app) = app {
                use tauri::Emitter;
                let _ = app.emit(
                    crate::commands::world::EVENT_SLEEP_STATE,
                    serde_json::json!({ "state": sleep_tag(state) }),
                );
            }
        }

        if let Some(sink) = sink {
            for blob in outgoing {
                if let Err(e) = sink.send(blob) {
                    tracing::debug!("[world] channel gone: {e}");
                    let mut g = shared.inner.lock().unwrap_or_else(|x| x.into_inner());
                    g.sink = None;
                    g.outbox.reset();
                    g.last_sent = None;
                    break;
                }
            }
        }

        let g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.stop {
            break;
        }
        match wait {
            Some(d) if d.is_zero() => drop(g),
            Some(d) => {
                // The guard has to be named: `let _ = ` on a lock drops it
                // straight away, which rustc denies by default and which would
                // turn the wait into a spin.
                let (guard, _) = shared
                    .cv
                    .wait_timeout(g, d)
                    .unwrap_or_else(|e| e.into_inner());
                drop(guard);
            }
            // Hibernating: park until somebody notifies. This is the 0 Hz.
            None => {
                let guard = shared.cv.wait(g).unwrap_or_else(|e| e.into_inner());
                drop(guard);
            }
        }
    }
    shared.alive.store(false, Ordering::SeqCst);
    TICK_THREADS.fetch_sub(1, Ordering::Relaxed);
}

/// At most one thought in flight in the whole house, at most one every
/// `think_interval_s`, only while somebody is watching (a sink is attached), and
/// only for a brain that says it is due. The model call runs on the async
/// runtime; its decisions come back through `from_minds` like any other.
fn maybe_think(shared: &Arc<Shared>) {
    use std::sync::atomic::Ordering as O;
    let Some(factory) = shared.thinker.lock().expect("thinker lock").clone() else {
        return;
    };
    if shared.think_busy.load(O::SeqCst) {
        return;
    }
    let picked = {
        let g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.sink.is_none() {
            return;
        }
        let Some(world) = g.world.as_ref() else {
            return;
        };
        let tick = world.tick();
        let mut found = None;
        for (ent, brain) in g.brains.iter() {
            if !brain.should_think(tick) {
                continue;
            }
            if let Some((name, _)) = g.names.iter().find(|(_, e)| **e == ent) {
                found = Some((ent, brain, name.clone()));
                break;
            }
        }
        found
    };
    let Some((ent, brain, name)) = picked else {
        return;
    };
    let Some((thinker, interval_s)) = factory(&name) else {
        return;
    };
    {
        let mut last = shared.last_think.lock().expect("last_think lock");
        if let Some(at) = *last {
            if at.elapsed() < Duration::from_secs(interval_s.max(10) as u64) {
                return;
            }
        }
        *last = Some(Instant::now());
    }
    if shared.think_busy.swap(true, O::SeqCst) {
        return;
    }
    let shared = shared.clone();
    tauri::async_runtime::spawn(async move {
        match brain.think(thinker.as_ref()).await {
            Ok(report) => {
                tracing::info!(
                    "[world] {name} thought: {} decisions, {} model call(s)",
                    report.decisions.len(),
                    report.model_calls
                );
                let mut g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
                g.from_minds
                    .extend(report.decisions.iter().map(|d| Input::Decision {
                        ent,
                        decision: crate::commands::world::brain::to_world_decision(d),
                    }));
                drop(g);
                shared.cv.notify_all();
            }
            Err(e) => tracing::debug!("[world] {name} could not think: {e:?}"),
        }
        shared.think_busy.store(false, O::SeqCst);
    });
}

/// Compare the roster with the cast. Runs every few seconds, and at once when
/// the bus names an agent the house has never seen: somebody the user just
/// added is already working.
fn refresh_roster(shared: &Arc<Shared>, events: &[BusEvent]) {
    {
        let mut g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.world.is_none() {
            return;
        }
        let stranger = events.iter().any(|ev| match ev {
            BusEvent::TurnStarted { agent, .. } => !g.names.contains_key(agent),
            _ => false,
        });
        let now = Instant::now();
        let due =
            now.saturating_duration_since(g.last_roster).as_millis() >= ROSTER_REFRESH_MS as u128;
        if !stranger && !due {
            return;
        }
        g.last_roster = now;
    }
    sync_roster(shared);
}

fn sync_roster(shared: &Arc<Shared>) -> (Vec<String>, Vec<String>) {
    let Some(source) = shared.roster.lock().expect("roster lock").clone() else {
        return (Vec::new(), Vec::new());
    };
    // Read with the state lock released: the roster has a lock of its own.
    let roster = source();
    let mut g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
    if g.world.is_none() {
        return (Vec::new(), Vec::new());
    }
    let spawn = g
        .map
        .as_ref()
        .and_then(|m| m.markers.iter().find(|m| m.name == "spawn"))
        .map(|m| m.tile)
        .unwrap_or(omniget_world::Tile::new(1, 1));
    let mut moved_in = Vec::new();
    let mut moved_out = Vec::new();
    let gone: Vec<(String, EntId)> = g
        .names
        .iter()
        .filter(|(name, ent)| {
            ent.0 < VISITOR_ENT_BASE && !roster.contains(name) && !g.guests.contains(*name)
        })
        .map(|(n, e)| (n.clone(), *e))
        .collect();
    for (name, ent) in gone {
        let input = Input::Despawn { ent };
        g.note_input(&input);
        g.pending.push(input);
        moved_out.push(name);
    }
    for name in roster {
        if g.names.contains_key(&name) {
            continue;
        }
        let used: BTreeSet<u32> = g.names.values().map(|e| e.0).collect();
        let Some(id) = (1..VISITOR_ENT_BASE).find(|i| !used.contains(i)) else {
            break;
        };
        // Side by side at the door, in the order they moved in.
        let input = Input::Spawn {
            ent: EntId(id),
            name: name.clone(),
            at: omniget_world::Tile::new(spawn.x + (id as i32 - 1) % 6, spawn.y),
        };
        g.note_input(&input);
        g.pending.push(input);
        moved_in.push(name);
    }
    if !moved_in.is_empty() || !moved_out.is_empty() {
        tracing::info!("[world] roster sync: in {moved_in:?}, out {moved_out:?}");
        g.next_due = Instant::now();
    }
    (moved_in, moved_out)
}

/// Read the quota windows at most once a minute.
fn refresh_quota(shared: &Arc<Shared>) -> Option<Vec<(String, f32)>> {
    let source = shared.quota.lock().expect("quota lock").clone()?;
    {
        let mut g = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if now.saturating_duration_since(g.last_energy).as_millis() < ENERGY_REFRESH_MS as u128 {
            return None;
        }
        g.last_energy = now;
    }
    Some(source())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::world::session::test_map;
    use omniget_world::{Snapshot, Tile};

    #[derive(Default)]
    struct Collector {
        blobs: Mutex<Vec<Vec<u8>>>,
        fail: std::sync::atomic::AtomicBool,
    }

    impl DiffSink for Collector {
        fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
            if self.fail.load(Ordering::Relaxed) {
                return Err("gone".into());
            }
            self.blobs.lock().unwrap().push(bytes);
            Ok(())
        }
    }

    fn tmp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-world-mgr-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manager(name: &str) -> (WorldManager, PathBuf) {
        let root = tmp_root(name);
        let m = WorldManager::new(root.clone());
        m.create(11, test_map(), "casa-v1").unwrap();
        (m, root)
    }

    /// Run one wake-up by hand, without the thread, so the tests are
    /// deterministic instead of timing-dependent.
    fn pump(m: &WorldManager, now: Instant) -> Vec<Vec<u8>> {
        let mut g = m.lock();
        g.pump(now, &m.root)
    }

    #[test]
    fn a_manager_with_no_world_has_no_thread_and_no_state() {
        let root = tmp_root("empty");
        let m = WorldManager::new(root.clone());
        assert!(!m.has_world());
        assert!(!m.thread_running(), "building a manager starts nothing");
        // Creating the world is still no thread: only opening the route is.
        m.create(1, test_map(), "casa-v1").unwrap();
        assert!(m.has_world());
        assert!(!m.thread_running(), "creating a world starts nothing");
        assert!(save::save_exists(&root), "a new world is written at once");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn opening_starts_exactly_one_thread_and_deleting_stops_it() {
        let (m, root) = manager("thread");
        // Counted per manager, not process-wide: the test binary runs its
        // tests in parallel and other worlds come and go beside this one.
        assert!(!m.thread_running());
        let sink: Arc<dyn DiffSink> = Arc::new(Collector::default());
        m.open(sink.clone()).unwrap();
        assert!(m.thread_running());
        assert!(
            tick_threads() >= 1,
            "the process-wide count sees it too (it is what `ps -M` counts)"
        );
        // Opening twice does not start a second one.
        m.open(sink).unwrap();
        assert!(m.thread_running());
        m.delete().unwrap();
        assert!(!m.thread_running(), "delete stops the thread");
        assert!(!save::save_exists(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn opening_returns_a_decodable_snapshot_and_the_channel_only_carries_diffs() {
        let (m, root) = manager("open");
        let sink = Arc::new(Collector::default());
        let bytes = m.open(sink.clone()).unwrap();
        // Stepped by hand from here on; see `dozing_advances_…`.
        m.stop_thread();
        let snap = Snapshot::decode(&bytes).expect("the route can decode what open returns");
        assert_eq!(snap.tick, 0);
        assert_eq!(snap.sleep, SleepState::Active);

        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(1),
            name: "Omni".into(),
            at: Tile::new(8, 12),
        })));
        let mut now = Instant::now();
        let mut blobs = Vec::new();
        for _ in 0..6 {
            blobs.extend(pump(&m, now));
            now += Duration::from_millis(100);
        }
        assert!(!blobs.is_empty(), "a spawn produces a diff");
        for b in &blobs {
            assert_eq!(b[4], 2, "kind 2 = diff; the snapshot went out of band");
            omniget_world::Diff::decode(b).expect("every blob decodes");
        }
        drop(sink);
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_idle_world_sends_nothing_at_all() {
        let (m, root) = manager("idle");
        m.open(Arc::new(Collector::default())).unwrap();
        // Stepped by hand from here on; see `dozing_advances_…`.
        m.stop_thread();
        let mut now = Instant::now();
        let mut blobs = Vec::new();
        for _ in 0..20 {
            blobs.extend(pump(&m, now));
            now += Duration::from_millis(100);
        }
        assert!(
            blobs.is_empty(),
            "an empty house changes nothing, so the channel stays quiet: {} blobs",
            blobs.len()
        );
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_closed_route_dozes_and_a_hidden_app_hibernates() {
        let (m, root) = manager("sleep");
        m.open(Arc::new(Collector::default())).unwrap();
        assert_eq!(m.sleep_state(), SleepState::Active);
        m.close();
        assert_eq!(m.sleep_state(), SleepState::Dozing);
        // From here the clock is moved by hand; the live tick thread would
        // settle the state back with the real one.
        m.stop_thread();

        // Time is moved forward, never backward: `Instant - Duration` panics on
        // a machine whose uptime is shorter than the span.
        let t0 = Instant::now();
        m.lock().background_since = Some(t0);
        pump(&m, t0 + Duration::from_millis(11 * 60 * 1000));
        assert_eq!(m.sleep_state(), SleepState::Hibernating);
        assert_eq!(
            m.lock().wait(Instant::now()),
            None,
            "hibernating parks instead of polling"
        );
        let before = m.tick();

        // Waking up skips the time instead of replaying it: eight hours is
        // 288 000 ticks, and nobody waits for those to be simulated.
        let stats_before = m.stats();
        {
            let mut g = m.lock();
            g.route_visible = true;
            g.background_since = None;
            g.settle_sleep(t0 + Duration::from_secs(9 * 3600));
            assert_eq!(g.sleep, SleepState::Active);
        }
        let stats = m.stats();
        assert!(
            stats.caught_up_ticks - stats_before.caught_up_ticks >= 288_000,
            "8 h skipped in one call: {stats:?}"
        );
        assert!(m.tick() >= before + 288_000);
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dozing_advances_the_world_by_the_whole_interval() {
        let (m, root) = manager("doze-rate");
        m.open(Arc::new(Collector::default())).unwrap();
        m.close();
        // Stepped by hand below: on some machines the live tick thread takes
        // the due step first, pushes `next_due` ahead and leaves this pump with
        // nothing to do (0 ticks instead of 50).
        m.stop_thread();
        let before = m.tick();
        // Past any `next_due` the thread may have left behind before it stopped.
        let now = Instant::now() + Duration::from_secs(30);
        pump(&m, now);
        // One step plus the catch-up of the other 4.9 s = 50 ticks of world.
        assert_eq!(m.tick() - before, SleepState::Dozing.stride());
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stalled_client_loses_frames_and_gets_a_snapshot_back() {
        let (m, root) = manager("resync");
        let sink = Arc::new(Collector::default());
        m.open(sink.clone()).unwrap();
        // This test steps the world by hand; the live tick thread would drain
        // the outbox behind its back on a fast (or a slow) machine.
        m.stop_thread();
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(1),
            name: "Omni".into(),
            at: Tile::new(8, 12),
        })));
        m.submit(BridgeInput::World(Box::new(Input::Move {
            ent: EntId(1),
            to: Tile::new(13, 5),
        })));

        // Nothing is drained: pump() hands the blobs back but the test throws
        // them away, exactly like a webview that stopped listening.
        let mut now = Instant::now();
        for _ in 0..(crate::commands::world::tick::OUTBOX_CAP + 8) {
            let mut g = m.lock();
            let inputs = std::mem::take(&mut g.pending);
            let world = g.world.as_mut().unwrap();
            world.step(&inputs);
            g.encode_outgoing();
            drop(g);
            now += Duration::from_millis(100);
        }
        let stats = m.stats();
        assert!(stats.dropped > 0, "the outbox dropped frames: {stats:?}");
        assert!(stats.resyncs > 0, "and asked for a resync: {stats:?}");
        assert!(
            m.lock().outbox.len() <= crate::commands::world::tick::OUTBOX_CAP,
            "the queue never grows past its cap"
        );
        let blobs = { m.lock().outbox.take() };
        let snapshots: Vec<_> = blobs.iter().filter(|b| b[4] == 1).collect();
        assert!(
            !snapshots.is_empty(),
            "after a hole the client is sent a whole snapshot, not a diff it cannot apply"
        );
        for s in snapshots {
            Snapshot::decode(s).unwrap();
        }
        let _ = now;
        drop(sink);
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_world_survives_a_restart_through_the_save_file() {
        let (m, root) = manager("persist");
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(4),
            name: "Ana".into(),
            at: Tile::new(8, 12),
        })));
        let mut now = Instant::now();
        for _ in 0..5 {
            pump(&m, now);
            now += Duration::from_millis(100);
        }
        m.save_now();
        let snapshot_before = m.lock().world.as_ref().unwrap().snapshot();
        drop(m);

        let m2 = WorldManager::new(root.clone());
        let bytes = save::read_save(&root).expect("a save is on disk");
        m2.restore(&bytes, test_map(), "casa-v1").unwrap();
        assert_eq!(
            m2.lock().world.as_ref().unwrap().snapshot(),
            snapshot_before
        );
        assert_eq!(
            m2.lock().names.get("Ana"),
            Some(&EntId(4)),
            "restoring rebuilds the name → entity map the bus needs"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_bus_puts_an_agent_to_work_without_the_route_being_open() {
        let (m, root) = manager("bus");
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(1),
            name: "worker".into(),
            at: Tile::new(8, 12),
        })));
        pump(&m, Instant::now());
        {
            let mut g = m.lock();
            g.ingest_bus(&[BusEvent::ToolCalled {
                agent: "worker".into(),
                tool: "dl_add".into(),
                ok: true,
                ms: 4,
            }]);
            assert_eq!(
                g.pending
                    .iter()
                    .filter(|i| matches!(i, Input::Decision { .. }))
                    .count(),
                1,
                "the tool call became a decision"
            );
        }
        let mut now = Instant::now();
        for _ in 0..30 {
            pump(&m, now);
            now += Duration::from_millis(100);
        }
        let act = m.lock().world.as_ref().unwrap().activity_of(EntId(1));
        assert!(
            act.is_some(),
            "the agent still exists and took the decision path"
        );
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dead_channel_is_dropped_instead_of_backing_up() {
        let (m, root) = manager("dead-channel");
        let sink = Arc::new(Collector::default());
        m.open(sink.clone()).unwrap();
        sink.fail.store(true, Ordering::Relaxed);
        {
            let mut g = m.lock();
            g.sink = None;
            g.encode_outgoing();
            assert!(g.outbox.is_empty(), "no sink, no backlog");
            assert_eq!(g.last_sent, None);
        }
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn narrowing_the_interest_forces_a_resync() {
        let (m, root) = manager("interest");
        m.open(Arc::new(Collector::default())).unwrap();
        m.submit(BridgeInput::Interest(Interest::new(Tile::new(8, 8), 4)));
        assert!(m.lock().outbox.needs_resync());
        assert_eq!(m.lock().interest.radius, 4);
        // The same circle again changes nothing.
        m.lock().outbox.clear_resync();
        m.submit(BridgeInput::Interest(Interest::new(Tile::new(8, 8), 4)));
        assert!(!m.lock().outbox.needs_resync());
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Mean encoded diff at 10 Hz with 8 agents walking. The budget is 2 KB
    /// (plan, Fase 7). Not `#[ignore]`d: it costs a few milliseconds and it is
    /// the number the whole channel design rests on.
    #[test]
    fn eight_agents_at_ten_hertz_fit_in_the_diff_budget() {
        let (m, root) = manager("diff-budget");
        m.open(Arc::new(Collector::default())).unwrap();
        // Stepped by hand from here on; see `dozing_advances_…`.
        m.stop_thread();
        let spots = [
            (2, 2),
            (4, 2),
            (8, 10),
            (8, 13),
            (13, 5),
            (9, 5),
            (12, 5),
            (11, 14),
        ];
        for (i, (x, y)) in spots.iter().enumerate() {
            m.submit(BridgeInput::World(Box::new(Input::Spawn {
                ent: EntId(i as u32 + 1),
                name: format!("a{i}"),
                at: Tile::new(8, 12),
            })));
            m.submit(BridgeInput::World(Box::new(Input::Move {
                ent: EntId(i as u32 + 1),
                to: Tile::new(*x, *y),
            })));
        }
        let mut now = Instant::now();
        for _ in 0..200 {
            pump(&m, now);
            now += Duration::from_millis(100);
        }
        let s = m.stats();
        assert_eq!(s.snapshots, 0, "no re-syncs happened: {s:?}");
        assert!(s.diffs > 50, "the run produced diffs: {s:?}");
        let mean = s.mean_diff_bytes();
        println!(
            "[measure] 8 agents @ 10 Hz over {} ticks: {} diffs, {:.1} B mean diff, \
             {:.1} B mean per tick, {} B total; budget 2048 B",
            s.steps,
            s.diffs,
            mean,
            s.mean_bytes_per_tick(),
            s.diff_bytes
        );
        assert!(
            mean <= 2048.0,
            "mean diff {mean:.1} B is over the 2 KB budget"
        );
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// CPU of a world nobody is looking at. Budget: 0.5 ms per second.
    /// `#[ignore]`d because it spends twenty seconds of wall clock.
    #[test]
    #[ignore = "measurement: 20 s of wall clock"]
    fn a_dozing_world_costs_almost_no_cpu() {
        let (m, root) = manager("dozing-cpu");
        for i in 0..8u32 {
            m.submit(BridgeInput::World(Box::new(Input::Spawn {
                ent: EntId(i + 1),
                name: format!("a{i}"),
                at: Tile::new(8, 12),
            })));
        }
        m.open(Arc::new(Collector::default())).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        m.close();
        assert_eq!(m.sleep_state(), SleepState::Dozing);

        let before = crate::world_bench::process_cpu_seconds().expect("process CPU is readable");
        let wall = Instant::now();
        std::thread::sleep(Duration::from_secs(20));
        let elapsed = wall.elapsed().as_secs_f64();
        let after = crate::world_bench::process_cpu_seconds().expect("process CPU is readable");
        let ms_per_s = (after - before) * 1000.0 / elapsed;
        println!(
            "[measure] dozing: {:.4} ms of CPU per second over {:.1} s, budget 0.5",
            ms_per_s, elapsed
        );
        assert!(
            ms_per_s <= 0.5,
            "a dozing world costs {ms_per_s:.4} ms/s, over the 0.5 ms/s budget"
        );
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The brain side of the bridge: an empty house has no minds, a spawn
    /// gives one, the agenda drives the body without a single model call, and
    /// what happens in the house is remembered.
    #[test]
    fn the_house_thinks_without_calling_a_model() {
        let (m, root) = manager("brains");
        assert_eq!(m.brain_count(), 0, "an empty house holds no minds");
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(1),
            name: "Omni".into(),
            at: Tile::new(8, 12),
        })));
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(2),
            name: "worker".into(),
            at: Tile::new(8, 12),
        })));
        let mut now = Instant::now();
        for _ in 0..4 {
            pump(&m, now);
            now += Duration::from_millis(100);
        }
        assert_eq!(m.brain_count(), 2, "one brain per agent in the house");

        // Somebody says something: both agents remember it. The memory only
        // leaves the queue inside `think`, which nothing calls, so what we can
        // assert here is that it was queued and that nothing was spent.
        m.submit(BridgeInput::World(Box::new(Input::Decision {
            ent: EntId(2),
            decision: omniget_world::Decision::Say("the radio is broken".into()),
        })));
        for _ in 0..40 {
            pump(&m, now);
            now += Duration::from_millis(100);
        }
        let queued: usize = {
            let g = m.lock();
            (1..=2)
                .filter_map(|id| g.brains.get(EntId(id)))
                .map(|b| b.queue_len())
                .sum()
        };
        assert!(queued >= 2, "both agents heard it: {queued} observations");
        assert_eq!(
            m.brain_model_calls(),
            0,
            "not one token was spent to run the house"
        );
        m.delete().unwrap();
        assert_eq!(m.brain_count(), 0, "deleting the world deletes the minds");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A bus event is remembered by the agent it is about, and moves the body
    /// exactly once (through `bus_input`, never twice through the brain too).
    #[test]
    fn a_tool_call_is_remembered_by_the_agent_that_made_it() {
        let (m, root) = manager("brain-bus");
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(1),
            name: "worker".into(),
            at: Tile::new(8, 12),
        })));
        pump(&m, Instant::now());
        {
            let mut g = m.lock();
            g.ingest_bus(&[BusEvent::ToolCalled {
                agent: "worker".into(),
                tool: "dl_add".into(),
                ok: true,
                ms: 4,
            }]);
            assert_eq!(
                g.pending
                    .iter()
                    .filter(|i| matches!(i, Input::Decision { .. }))
                    .count(),
                1,
                "one decision in the mailbox, not two: the brain observes, `bus_input` moves"
            );
            assert_eq!(g.brains.get(EntId(1)).unwrap().queue_len(), 1);
        }
        assert_eq!(m.brain_model_calls(), 0);
        m.delete().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The brain's memories live in `<app_data>/world/memory.sqlite`, which is
    /// inside the directory `world_delete` removes.
    #[test]
    fn the_memories_are_a_file_inside_the_world_directory() {
        let (m, root) = manager("brain-db");
        let db = crate::commands::world::brain_store::memory_path(&root);
        assert!(db.exists(), "installing a world opens the database: {db:?}");
        m.delete().unwrap();
        assert!(!db.exists(), "deleting the world deletes the memories");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An open house ticks at 10 Hz. A thread that wakes up late, or that
    /// schedules the next tick from the end of the work instead of from the
    /// tick that was due, runs the whole house in slow motion.
    #[test]
    fn an_open_house_keeps_its_ten_ticks_a_second() {
        let (m, root) = manager("rate");
        let sink = Arc::new(Collector::default());
        m.open(sink.clone()).unwrap();
        for (i, name) in ["omni", "builder", "scout"].iter().enumerate() {
            m.submit(BridgeInput::World(Box::new(Input::Spawn {
                ent: EntId(i as u32 + 1),
                name: (*name).into(),
                at: Tile::new(8 + i as i32, 12),
            })));
            m.submit(BridgeInput::World(Box::new(Input::Move {
                ent: EntId(i as u32 + 1),
                to: Tile::new(13, 5 + i as i32),
            })));
        }
        let before = m.tick();
        std::thread::sleep(Duration::from_millis(700));
        let ticks = m.tick() - before;
        // A dozing house would manage one tick at most in this window. The
        // bar is well under the nominal 7 so a loaded CI runner passes too.
        assert!(
            ticks >= 3,
            "0.7 s of an active house is about 7 ticks, got {ticks}"
        );
        m.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_tick_thread_really_runs_and_really_stops() {
        let (m, root) = manager("live-thread");
        let sink = Arc::new(Collector::default());
        m.open(sink.clone()).unwrap();
        m.submit(BridgeInput::World(Box::new(Input::Spawn {
            ent: EntId(1),
            name: "Omni".into(),
            at: Tile::new(8, 12),
        })));
        m.submit(BridgeInput::World(Box::new(Input::Move {
            ent: EntId(1),
            to: Tile::new(13, 5),
        })));
        let since = std::time::Instant::now();
        while m.tick() < 3 && since.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ticks = m.tick();
        assert!(ticks >= 3, "the thread advanced the world: {ticks}");
        assert!(
            !sink.blobs.lock().unwrap().is_empty(),
            "and pushed diffs down the channel"
        );
        m.shutdown();
        let after = m.tick();
        // Two tick periods: a thread still running would have ticked.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(m.tick(), after, "a stopped thread stops the world");
        let _ = std::fs::remove_dir_all(&root);
    }
}
