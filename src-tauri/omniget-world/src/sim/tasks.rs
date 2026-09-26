//! Residents with work to do: tasks, reservations and needs, all inside the
//! tick and all deterministic (session 06 of the V2 plan).
//!
//! An agent with autonomy switched on picks a task from what the region
//! offers (utility over its needs, integers only, ties by object id), reserves
//! the object (capacity one), walks to an approach tile, executes in small
//! observable steps and releases the object. Everything that can go wrong has
//! a reason code instead of a loop: a seat taken (`busy`, retried with
//! backoff, then given up on with a cooldown), a path cut by an edit
//! (`no_path` after trying the other approach tiles), a target removed
//! (`target_gone`), a delegation revoked (`revoked`).
//!
//! Effects with consequences outside the simulation (a harvest credits an
//! inventory) are not decided here. The task stops in `awaiting_effect` and
//! emits an [`EffectRequest`]; the server runs it through the same
//! transactional domain a person's click goes through, keyed by
//! `(task, revision)` so a retry is idempotent, and answers with
//! `Input::TaskResult`. A result for an older revision is ignored: that is
//! the fence against a stale worker or a replayed answer.
//!
//! No model is consulted. A model's decision (a `Decision` in the mailbox)
//! becomes a task like any other and is validated the same way.
//!
//! The task book is part of the checkpoint, so a restart resumes exactly;
//! the trace is bounded and is for the inspector, it never goes on the wire.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::ents::agent::{Activity, ENERGY_FULL};
use crate::ents::id::{EntId, ObjectId};
use crate::ents::object::Object;
use crate::map::Tile;

#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct TaskId(pub u64);

/// What a task does at its target.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAction {
    /// Sleep in a bed until rested.
    Sleep,
    /// Sit on a seat for a while (comfort; recovers a little energy).
    Sit,
    /// Work at a workstation (costs energy).
    Work,
    /// Farm a delegated bed. The effect is the server's.
    Water,
    Plant,
    Harvest,
    Clear,
    /// Stand still for a while: the filler when nothing is worth doing.
    Loiter,
}

impl TaskAction {
    pub const fn is_farm(self) -> bool {
        matches!(
            self,
            TaskAction::Water | TaskAction::Plant | TaskAction::Harvest | TaskAction::Clear
        )
    }
    /// Index in [`TaskStats::completed_by_action`].
    pub const fn index(self) -> usize {
        match self {
            TaskAction::Sleep => 0,
            TaskAction::Sit => 1,
            TaskAction::Work => 2,
            TaskAction::Water => 3,
            TaskAction::Plant => 4,
            TaskAction::Harvest => 5,
            TaskAction::Clear => 6,
            TaskAction::Loiter => 7,
        }
    }
    pub const fn code(self) -> &'static str {
        match self {
            TaskAction::Sleep => "sleep",
            TaskAction::Sit => "sit",
            TaskAction::Work => "work",
            TaskAction::Water => "water",
            TaskAction::Plant => "plant",
            TaskAction::Harvest => "harvest",
            TaskAction::Clear => "clear",
            TaskAction::Loiter => "loiter",
        }
    }
}

/// Who asked for the task.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    /// The local selection (needs and demand).
    Local,
    /// A decision in the mailbox: a routine entry or a model's intent.
    Intent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting for a reservation and a route (both are budgeted per tick).
    Queued,
    Travelling,
    Executing,
    /// Done here; the server is applying the effect.
    AwaitingEffect,
    /// Could not proceed; retried at `retry_at`.
    Blocked,
    Completed,
    Cancelled,
    Failed,
}

impl TaskState {
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Cancelled | TaskState::Failed
        )
    }
    /// States that hold the target's reservation.
    pub const fn holds(self) -> bool {
        matches!(
            self,
            TaskState::Travelling | TaskState::Executing | TaskState::AwaitingEffect
        )
    }
}

/// Why a task is blocked, cancelled or failed. Stable codes: the inspector
/// and the tests read them.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    #[default]
    None,
    /// The target is reserved by someone else.
    Busy,
    /// No approach tile can be reached.
    NoPath,
    /// The target was removed.
    TargetGone,
    /// The delegation that allowed the task was revoked.
    Revoked,
    /// Autonomy was switched off or a stronger need took over.
    Preempted,
    /// The server refused the effect, with its code.
    EffectRefused(String),
    /// No answer from the server in time, after the retries.
    EffectTimeout,
    /// The target cannot do what was asked (not a seat, not delegated…).
    NotAllowed,
    /// Cancelled from outside (`Input::CancelTask`).
    Cancelled,
}

impl Reason {
    pub fn code(&self) -> String {
        match self {
            Reason::None => String::new(),
            Reason::Busy => "busy".into(),
            Reason::NoPath => "no_path".into(),
            Reason::TargetGone => "target_gone".into(),
            Reason::Revoked => "revoked".into(),
            Reason::Preempted => "preempted".into(),
            Reason::EffectRefused(c) => format!("effect_refused:{c}"),
            Reason::EffectTimeout => "effect_timeout".into(),
            Reason::NotAllowed => "not_allowed".into(),
            Reason::Cancelled => "cancelled".into(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub agent: EntId,
    pub action: TaskAction,
    /// `ObjectId(0)` for a task without a target (loiter).
    pub target: ObjectId,
    pub source: TaskSource,
    pub state: TaskState,
    /// Bumped whenever the task is (re)reserved or re-issued; an effect
    /// result must carry the current one.
    pub revision: u32,
    pub attempts: u8,
    pub created: u64,
    /// Tick the current state was entered.
    pub since: u64,
    pub retry_at: u64,
    /// Ticks spent executing.
    pub progress: u32,
    /// Ticks the execution lasts (0 = until a condition, for sleep).
    pub duration: u32,
    pub reason: Reason,
    /// Where the agent stands to use the target.
    pub approach: Option<Tile>,
    /// Effect requests sent for the current revision.
    pub effect_sends: u8,
}

/// One object held by one task.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Reservation {
    pub task: TaskId,
    pub agent: EntId,
    pub revision: u32,
    /// Renewed every tick the task is alive; an orphan expires by itself.
    pub expires: u64,
}

/// What a farm task asks the server to do.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EffectRequest {
    pub task: TaskId,
    pub revision: u32,
    pub agent: EntId,
    pub action: TaskAction,
    pub object: ObjectId,
    pub tile: Tile,
}

/// One line of the inspector's trace.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TraceEntry {
    pub tick: u64,
    pub task: TaskId,
    pub agent: EntId,
    pub action: TaskAction,
    pub target: ObjectId,
    pub state: TaskState,
    pub revision: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// Per agent: whether it chooses its own tasks, and what it may touch.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Autonomy {
    pub enabled: bool,
    /// The task it is on.
    pub current: Option<TaskId>,
    /// Earliest tick of the next selection.
    pub next_eval: u64,
    /// Objects (farm beds) someone with the right delegated to this agent.
    pub delegated: BTreeSet<ObjectId>,
    /// Targets not to pick again before the tick.
    pub cooldown: BTreeMap<ObjectId, u64>,
    /// Ticks since the last need update.
    pub need_ticks: u32,
}

/// Every knob, integers only; `Input::SetTaskParams` replaces them (tests,
/// operators). Durations are in ticks (10 per second).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskParams {
    /// Selections per tick, round robin by id: the evaluation budget.
    pub eval_per_tick: u16,
    /// Reservations + routes per tick: the path budget; the rest waits queued.
    pub route_per_tick: u16,
    pub reservation_ttl: u32,
    /// `busy` / `no_path` retries before giving up.
    pub max_attempts: u8,
    /// Backoff: base × 2^attempt ticks.
    pub retry_base: u32,
    /// Pause after a task ends before choosing again.
    pub settle_ticks: u32,
    /// A target that failed is left alone this long.
    pub target_cooldown: u32,
    /// A watered bed is not watered again this soon.
    pub water_cooldown: u32,
    pub effect_timeout: u32,
    pub effect_max_sends: u8,
    /// Need update period and per-period deltas.
    pub need_period: u32,
    pub cost_work: u8,
    pub cost_awake: u8,
    pub gain_sit: u8,
    pub gain_sleep: u8,
    /// Below: sleep wins. Above after sleeping: wake.
    pub energy_sleep: u8,
    pub energy_rested: u8,
    /// Below: sitting down gets attractive.
    pub energy_sit: u8,
    pub sit_ticks: u32,
    pub work_ticks: u32,
    pub farm_ticks: u32,
    pub loiter_ticks: u32,
    pub sleep_max_ticks: u32,
    /// Utility base scores.
    pub score_sleep: i32,
    pub score_sit: i32,
    pub score_work: i32,
    pub score_harvest: i32,
    pub score_plant: i32,
    pub score_clear: i32,
    pub score_water: i32,
    pub score_loiter: i32,
    /// Utility lost per tile of distance.
    pub distance_cost: i32,
    /// Objects farther than this (Chebyshev tiles) are not considered.
    pub reach: i32,
}

impl Default for TaskParams {
    fn default() -> Self {
        TaskParams {
            eval_per_tick: 8,
            route_per_tick: 8,
            reservation_ttl: 50,
            max_attempts: 3,
            retry_base: 20,
            settle_ticks: 10,
            target_cooldown: 600,
            water_cooldown: 3000,
            effect_timeout: 100,
            effect_max_sends: 3,
            need_period: 50,
            cost_work: 3,
            cost_awake: 1,
            gain_sit: 4,
            gain_sleep: 12,
            energy_sleep: 48,
            energy_rested: 240,
            energy_sit: 140,
            sit_ticks: 300,
            work_ticks: 600,
            farm_ticks: 40,
            loiter_ticks: 80,
            sleep_max_ticks: 6000,
            score_sleep: 900,
            score_sit: 250,
            score_work: 400,
            score_harvest: 700,
            score_plant: 450,
            score_clear: 500,
            score_water: 300,
            score_loiter: 20,
            distance_cost: 6,
            reach: 24,
        }
    }
}

/// What an object offers, by exact kind. No suffix matching: a decorative
/// `prop/chair-sculpture` stays decorative. `c01/*` are the V2 fixture ids.
pub fn affordance(kind: &str) -> Option<TaskAction> {
    Some(match kind {
        "object/bed" | "c01/bed" => TaskAction::Sleep,
        // The street bench is where a resident rests outside (capacity one,
        // like every seat here).
        "object/chair" | "object/sofa" | "object/stool" | "c01/chair" | "prop/bench" => {
            TaskAction::Sit
        }
        "object/workbench" | "object/desk" | "object/table" | "object/bookshelf"
        | "c01/workbench" => TaskAction::Work,
        "prop/soil-empty" => TaskAction::Plant,
        k if k.starts_with("crop/") => {
            // crop/<crop>/<stage>
            match k.rsplit('/').next().unwrap_or("") {
                "ripe" => TaskAction::Harvest,
                "withered" => TaskAction::Clear,
                _ => TaskAction::Water,
            }
        }
        _ => return None,
    })
}

/// Farm actions need a delegation; the rest is open to any resident.
pub fn needs_delegation(action: TaskAction) -> bool {
    action.is_farm()
}

/// Where an agent may stand to use an object: south first (the side the
/// camera sees, what the art is drawn for), then east, west and north, then
/// the corners. Only tiles outside the footprint.
pub fn approach_tiles(o: &Object) -> Vec<Tile> {
    let w = o.footprint[0].max(1) as i32;
    let d = o.footprint[1].max(1) as i32;
    let (x0, y0) = (o.tile.x, o.tile.y);
    let mut out = Vec::with_capacity(2 * (w + d) as usize + 4);
    for x in x0..x0 + w {
        out.push(Tile::new(x, y0 + d));
    }
    for y in y0..y0 + d {
        out.push(Tile::new(x0 + w, y));
    }
    for y in y0..y0 + d {
        out.push(Tile::new(x0 - 1, y));
    }
    for x in x0..x0 + w {
        out.push(Tile::new(x, y0 - 1));
    }
    out.extend([
        Tile::new(x0 + w, y0 + d),
        Tile::new(x0 - 1, y0 + d),
        Tile::new(x0 + w, y0 - 1),
        Tile::new(x0 - 1, y0 - 1),
    ]);
    out
}

pub const TRACE_LEN: usize = 512;

/// Counters for budgets and backpressure (not checkpointed).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize)]
pub struct TaskStats {
    /// Completions per action: sleep, sit, work, water, plant, harvest,
    /// clear, loiter (so waiting around is never counted as work).
    pub completed_by_action: [u64; 8],
    pub evaluations: u64,
    pub routes: u64,
    /// Queued tasks left waiting by the route budget, summed over ticks.
    pub deferred: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub effects_sent: u64,
}

/// All task state of a world.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskBook {
    pub next_id: u64,
    pub tasks: BTreeMap<TaskId, Task>,
    pub reservations: BTreeMap<ObjectId, Reservation>,
    pub autonomy: BTreeMap<EntId, Autonomy>,
    pub params: TaskParams,
    /// Round-robin position of the evaluation budget (last id evaluated).
    pub cursor: u32,
    /// Watered beds: not watered again before the tick.
    pub watered: BTreeMap<ObjectId, u64>,
    pub trace: VecDeque<TraceEntry>,
    #[serde(skip)]
    pub effects: Vec<EffectRequest>,
    #[serde(skip)]
    pub stats: TaskStats,
}

impl TaskBook {
    pub fn is_autonomous(&self, ent: EntId) -> bool {
        self.autonomy.get(&ent).map(|a| a.enabled).unwrap_or(false)
    }

    pub fn current(&self, ent: EntId) -> Option<&Task> {
        let id = self.autonomy.get(&ent)?.current?;
        self.tasks.get(&id)
    }

    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Who holds an object, if anyone.
    pub fn holder(&self, object: ObjectId) -> Option<&Reservation> {
        self.reservations.get(&object)
    }

    /// Farm tasks waiting on the server: after a restart the server sends
    /// these again (same revision, so the domain's idempotency holds).
    pub fn awaiting_effects(&self) -> Vec<EffectRequest> {
        self.tasks
            .values()
            .filter(|t| t.state == TaskState::AwaitingEffect)
            .map(|t| EffectRequest {
                task: t.id,
                revision: t.revision,
                agent: t.agent,
                action: t.action,
                object: t.target,
                tile: t.approach.unwrap_or_default(),
            })
            .collect()
    }

    /// Effect requests emitted since the last call.
    pub fn take_effects(&mut self) -> Vec<EffectRequest> {
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn trace(&mut self, tick: u64, t: &Task) {
        if self.trace.len() >= TRACE_LEN {
            self.trace.pop_front();
        }
        self.trace.push_back(TraceEntry {
            tick,
            task: t.id,
            agent: t.agent,
            action: t.action,
            target: t.target,
            state: t.state,
            revision: t.revision,
            reason: if t.reason == Reason::None {
                String::new()
            } else {
                t.reason.code()
            },
        });
    }

    /// Recent trace lines of one agent, newest last.
    pub fn trace_of(&self, ent: EntId, n: usize) -> Vec<TraceEntry> {
        let mut v: Vec<TraceEntry> = self
            .trace
            .iter()
            .filter(|e| e.agent == ent)
            .cloned()
            .collect();
        if v.len() > n {
            v.drain(..v.len() - n);
        }
        v
    }

    /// Counts per state, for metrics.
    pub fn census(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        for t in self.tasks.values() {
            let k = match t.state {
                TaskState::Queued => "queued",
                TaskState::Travelling => "travelling",
                TaskState::Executing => "executing",
                TaskState::AwaitingEffect => "awaiting_effect",
                TaskState::Blocked => "blocked",
                TaskState::Completed => "completed",
                TaskState::Cancelled => "cancelled",
                TaskState::Failed => "failed",
            };
            *m.entry(k).or_insert(0) += 1;
        }
        m
    }
}

/// Energy after one need period, by what the agent is doing.
pub fn need_delta(p: &TaskParams, activity: Activity, energy: u8) -> u8 {
    let e = energy as i32;
    let next = match activity {
        Activity::Sleeping => e + p.gain_sleep as i32,
        Activity::Sitting(_) => e + p.gain_sit as i32,
        Activity::Working(_) => e - p.cost_work as i32,
        _ => e - p.cost_awake as i32,
    };
    next.clamp(0, ENERGY_FULL as i32) as u8
}
