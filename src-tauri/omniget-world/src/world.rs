//! `World`: the public face of the simulation.
//!
//! One fixed tick of 100 ms, one seed, one map. Feed it inputs, get a
//! `StepReport`; ask it for a `Snapshot` or a `Diff`. It owns no thread, no
//! socket, no GPU and no clock: whoever drives it decides when a tick happens,
//! which is what lets the same struct run at 10 Hz behind a visible route, at
//! 0.2 Hz behind a closed one, and at whatever rate a room server wants.

pub mod checkpoint;

use std::collections::{BTreeMap, VecDeque};

use crate::ents::agent::Activity;
use crate::ents::id::{EntId, ObjectId};
use crate::ents::object::Object;
use crate::ents::{dir_from_delta, Agents, Objects};
use crate::error::{Result, WorldError};
use crate::fixed::Fixed;
use crate::map::{Map, MapDef, Tile};
use crate::path::{AStar, Grid};
use crate::rng::Rng;
use crate::sim::mailbox::{Decision, Mailbox};
use crate::sim::routine::Routine;
use crate::sim::sleep::{ticks_from_ms, SleepState, MAX_CATCH_UP_TICKS};
use crate::snapshot::diff::{
    Diff, EntDelta, ObjDelta, WorldEvent, FIELD_ACT, FIELD_ANIM, FIELD_DIR, FIELD_ENERGY, FIELD_POS,
};
use crate::snapshot::{AgentState, Interest, ObjectState, Snapshot};

/// Ticks of observable state kept for `diff_since`. Six seconds at 10 Hz is
/// far more than a client behind one frame needs, and at 24 agents it costs
/// about 40 KB.
pub const HISTORY_TICKS: usize = 64;
/// Events kept for `diff_since`.
pub const HISTORY_EVENTS: usize = 512;
/// Object changes kept for `diff_since`.
pub const HISTORY_OBJ: usize = 256;

/// Something the outside asks the world to do. Everything that enters the
/// simulation enters here: there is no other setter.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Input {
    /// Put an agent in the world.
    Spawn {
        ent: EntId,
        name: String,
        at: Tile,
    },
    /// Take one out.
    Despawn {
        ent: EntId,
    },
    /// Walk somewhere. The player's own click, not a model's idea.
    Move {
        ent: EntId,
        to: Tile,
    },
    /// A decision from the brain. Goes through the mailbox, never straight
    /// into the tick.
    Decision {
        ent: EntId,
        decision: Decision,
    },
    /// Put an object down, or move one that already exists.
    PlaceObject {
        object: ObjectId,
        kind: String,
        tile: Tile,
        dir: u8,
        slot: Option<String>,
    },
    RemoveObject {
        object: ObjectId,
    },
    Interact {
        ent: EntId,
        object: ObjectId,
    },
    /// Energy is the account's remaining quota (plan §9.1). It is pushed in
    /// from `cli_usage` and `budget`; the world never invents it and never
    /// asks a model for it.
    SetEnergy {
        ent: EntId,
        energy: u8,
    },
    /// Give an agent a daily routine.
    SetRoutine {
        ent: EntId,
        routine: Routine,
    },
    /// A line over the agent's head that changes nothing else: what it is
    /// doing right now (`fs_edit cart.js`). Unlike `Decision::Say` it does not
    /// take the agent out of whatever it is busy with.
    Caption {
        ent: EntId,
        text: String,
    },
    /// Let an agent choose its own tasks (residents), or stop (the task in
    /// hand is cancelled with `preempted`).
    SetAutonomy {
        ent: EntId,
        enabled: bool,
    },
    /// The objects (farm beds) someone entitled delegated to this agent. The
    /// full set: an object missing from it is revoked.
    SetDelegation {
        ent: EntId,
        objects: Vec<ObjectId>,
    },
    /// Cancel the task in hand.
    CancelTask {
        ent: EntId,
    },
    /// The server's answer to an `EffectRequest`.
    TaskResult {
        task: crate::sim::tasks::TaskId,
        revision: u32,
        ok: bool,
        #[serde(default)]
        code: String,
    },
    /// Replace the task and need parameters.
    SetTaskParams {
        params: crate::sim::tasks::TaskParams,
    },
    /// Nothing; useful for a caller that has no input this tick.
    Tick,
}

/// What one `step()` did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StepReport {
    pub tick: u64,
    /// Inputs accepted.
    pub accepted: u32,
    /// Inputs refused, each with a `WorldEvent::Rejected` in the events.
    pub rejected: u32,
    /// Decisions taken out of the mailbox this tick.
    pub decisions: u32,
    /// Agents that moved.
    pub moved: u32,
    /// Events produced.
    pub events: u32,
    /// Ticks skipped by a catch-up rather than simulated.
    pub caught_up: u64,
}

/// Observable agent fields, without the name: a `Copy` record so the history
/// ring costs no allocation per tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ObsAgent {
    pub id: EntId,
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
    pub dir: u8,
    pub anim: u8,
    pub energy: u8,
    pub activity: Activity,
}

impl ObsAgent {
    fn tile(&self) -> Tile {
        Tile::new(self.x.floor_tile(), self.y.floor_tile())
    }

    fn delta_to(&self, now: &ObsAgent) -> Option<EntDelta> {
        let mut mask = 0u8;
        if self.x != now.x || self.y != now.y || self.z != now.z {
            mask |= FIELD_POS;
        }
        if self.dir != now.dir {
            mask |= FIELD_DIR;
        }
        if self.anim != now.anim {
            mask |= FIELD_ANIM;
        }
        if self.energy != now.energy {
            mask |= FIELD_ENERGY;
        }
        if self.activity != now.activity {
            mask |= FIELD_ACT;
        }
        if mask == 0 {
            return None;
        }
        // Same rule as `EntDelta::between`: fields the mask does not claim
        // stay at their default, so encode/decode is an identity.
        let mut d = EntDelta {
            id: now.id,
            mask,
            ..EntDelta::default()
        };
        if mask & FIELD_POS != 0 {
            d.x = now.x;
            d.y = now.y;
            d.z = now.z;
        }
        if mask & FIELD_DIR != 0 {
            d.dir = now.dir;
        }
        if mask & FIELD_ANIM != 0 {
            d.anim = now.anim;
        }
        if mask & FIELD_ENERGY != 0 {
            d.energy = now.energy;
        }
        if mask & FIELD_ACT != 0 {
            d.activity = now.activity;
        }
        Some(d)
    }
}

#[derive(Clone, Debug, Default)]
struct Frame {
    tick: u64,
    agents: Vec<ObsAgent>,
}

/// The world.
#[derive(Clone, Debug)]
pub struct World {
    seed: u64,
    rng: Rng,
    tick: u64,
    sleep: SleepState,
    map: Map,
    pub(crate) grid: Grid,
    pub(crate) agents: Agents,
    pub(crate) objects: Objects,
    mailbox: Mailbox,
    /// Tasks, reservations and autonomy (session 06).
    pub(crate) tasks: crate::sim::tasks::TaskBook,
    pub(crate) astar: AStar,
    // --- scratch, reused so a tick in steady state allocates nothing ---
    pub(crate) path_buf: Vec<Tile>,
    pub(crate) order_buf: Vec<usize>,
    pub(crate) events: Vec<WorldEvent>,
    // --- history for diff_since ---
    frames: VecDeque<Frame>,
    spare_frames: Vec<Frame>,
    event_log: VecDeque<(u64, WorldEvent)>,
    obj_log: VecDeque<(u64, ObjDelta)>,
}

impl World {
    /// Build a world from a seed and a map definition.
    pub fn new(seed: u64, map: MapDef) -> Result<World> {
        let map = Map::load(&map)?;
        let grid = Grid::from_map(&map);
        let mut w = World {
            seed,
            rng: Rng::new(seed),
            tick: 0,
            sleep: SleepState::Active,
            map,
            grid,
            agents: Agents::default(),
            objects: Objects::default(),
            mailbox: Mailbox::new(),
            tasks: Default::default(),
            astar: AStar::new(),
            path_buf: Vec::with_capacity(64),
            order_buf: Vec::with_capacity(32),
            events: Vec::with_capacity(32),
            frames: VecDeque::with_capacity(HISTORY_TICKS),
            spare_frames: Vec::new(),
            event_log: VecDeque::with_capacity(HISTORY_EVENTS),
            obj_log: VecDeque::with_capacity(HISTORY_OBJ),
        };
        w.place_map_objects();
        w.record_frame();
        Ok(w)
    }

    fn place_map_objects(&mut self) {
        let defs = self.map.objects.clone();
        for d in defs {
            let (walkable, height, footprint) = self.palette_of(&d.kind);
            let o = Object {
                id: ObjectId(d.id),
                kind: d.kind,
                tile: d.tile,
                dir: d.dir,
                slot: d.slot,
                footprint,
                walkable,
                height,
            };
            if !o.walkable {
                self.grid.set_footprint(o.tile, o.footprint, true);
            }
            self.objects.put(o);
        }
    }

    /// Palette facts for an object kind, defaulting to a blocking 1x1 when the
    /// map's palette says nothing: an unknown object must not become a hole in
    /// the wall.
    fn palette_of(&self, kind: &str) -> (bool, u16, [u8; 2]) {
        match self.map.palette.iter().find(|t| t.key == kind) {
            Some(t) => (t.walkable, t.height, t.footprint),
            None => (false, 0, [1, 1]),
        }
    }

    // --- accessors ---

    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Tasks, reservations, autonomy and the inspector's trace.
    pub fn tasks(&self) -> &crate::sim::tasks::TaskBook {
        &self.tasks
    }

    /// Effect requests emitted by the last steps (farm tasks), for the server.
    pub fn take_effects(&mut self) -> Vec<crate::sim::tasks::EffectRequest> {
        self.tasks.take_effects()
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub const fn sleep(&self) -> SleepState {
        self.sleep
    }

    pub fn set_sleep(&mut self, s: SleepState) {
        self.sleep = s;
    }

    pub fn map(&self) -> &Map {
        &self.map
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn mailbox_pending(&self) -> usize {
        self.mailbox.total_pending()
    }

    /// The tile an agent stands on, for tests and for the bridge.
    pub fn tile_of(&self, ent: EntId) -> Option<Tile> {
        self.agents.slot(ent).map(|s| self.agents.tile_of(s))
    }

    pub fn activity_of(&self, ent: EntId) -> Option<Activity> {
        self.agents.slot(ent).map(|s| self.agents.activity[s])
    }

    pub fn energy_of(&self, ent: EntId) -> Option<u8> {
        self.agents.slot(ent).map(|s| self.agents.energy[s])
    }

    // --- the tick ---

    /// Apply the inputs and advance one tick. Never fails: a bad input is
    /// refused with a `WorldEvent::Rejected` and the world carries on, because
    /// the thing producing inputs is a language model.
    pub fn step(&mut self, inputs: &[Input]) -> StepReport {
        self.events.clear();
        let mut report = StepReport::default();
        for input in inputs {
            match self.apply_input(input) {
                Ok(true) => report.accepted += 1,
                Ok(false) => {}
                Err(e) => {
                    report.rejected += 1;
                    let ent = input_ent(input);
                    self.events.push(WorldEvent::Rejected {
                        ent,
                        code: e.code().to_string(),
                    });
                }
            }
        }
        self.tick += 1;
        self.run_tick(&mut report);
        report.tick = self.tick;
        report.events = self.events.len() as u32;
        self.commit_events();
        self.record_frame();
        report
    }

    /// Skip time instead of simulating it. Used when the app comes back from
    /// hibernation: eight hours is 288 000 ticks, and replaying them would
    /// take longer than the user is willing to wait and produce a history
    /// nobody will ever look at. Agents are placed where their routine says
    /// they should be, and one `CaughtUp` event says how much was skipped.
    pub fn catch_up(&mut self, elapsed_ms: u64) -> StepReport {
        self.events.clear();
        let mut report = StepReport::default();
        let ticks = ticks_from_ms(elapsed_ms).min(MAX_CATCH_UP_TICKS);
        if ticks == 0 {
            report.tick = self.tick;
            return report;
        }
        self.tick += ticks;
        report.caught_up = ticks;
        self.settle_to_routine();
        self.settle_tasks(ticks);
        self.events.push(WorldEvent::CaughtUp { ticks });
        report.tick = self.tick;
        report.events = self.events.len() as u32;
        self.commit_events();
        self.record_frame();
        report
    }

    fn apply_input(&mut self, input: &Input) -> Result<bool> {
        match input {
            Input::Tick => Ok(false),
            Input::Spawn { ent, name, at } => {
                // The grid, not the map: a bed is not a wall, but an agent
                // still may not spawn inside one.
                let tile = self
                    .grid
                    .nearest_passable(*at, 8)
                    .ok_or(WorldError::NotWalkable { x: at.x, y: at.y })?;
                let z = self.map.z_at(tile);
                let rng = self.rng;
                self.agents.spawn(*ent, name, tile, z, rng)?;
                self.events.push(WorldEvent::Spawned {
                    ent: *ent,
                    at: tile,
                });
                Ok(true)
            }
            Input::Despawn { ent } => {
                self.agents.despawn(*ent)?;
                self.mailbox.forget(*ent);
                self.events.push(WorldEvent::Despawned { ent: *ent });
                Ok(true)
            }
            Input::Move { ent, to } => {
                let slot = self
                    .agents
                    .slot(*ent)
                    .ok_or(WorldError::UnknownEnt(ent.0))?;
                self.route(slot, *to)?;
                Ok(true)
            }
            Input::Decision { ent, decision } => {
                if !self.agents.contains(*ent) {
                    return Err(WorldError::UnknownEnt(ent.0));
                }
                self.mailbox.post(*ent, decision.clone());
                Ok(true)
            }
            Input::SetEnergy { ent, energy } => {
                let slot = self
                    .agents
                    .slot(*ent)
                    .ok_or(WorldError::UnknownEnt(ent.0))?;
                self.agents.energy[slot] = *energy;
                Ok(true)
            }
            Input::Caption { ent, text } => {
                if !self.agents.contains(*ent) {
                    return Err(WorldError::UnknownEnt(ent.0));
                }
                let mut end = text.len().min(crate::sim::MAX_SAY_BYTES);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                self.events.push(WorldEvent::Said {
                    ent: *ent,
                    text: text[..end].to_string(),
                });
                Ok(true)
            }
            Input::SetRoutine { ent, routine } => {
                let slot = self
                    .agents
                    .slot(*ent)
                    .ok_or(WorldError::UnknownEnt(ent.0))?;
                self.agents.routine[slot] = routine.clone();
                self.agents.last_routine_min[slot] = u16::MAX;
                Ok(true)
            }
            Input::Interact { ent, object } => {
                let slot = self
                    .agents
                    .slot(*ent)
                    .ok_or(WorldError::UnknownEnt(ent.0))?;
                if self.objects.get(*object).is_none() {
                    return Err(WorldError::UnknownObject(object.0));
                }
                let dir = {
                    let o = self.objects.get(*object).expect("checked above");
                    let a_tile = self.agents.tile_of(slot);
                    dir_from_delta(o.tile.x - a_tile.x, o.tile.y - a_tile.y)
                };
                self.agents.dir[slot] = dir;
                self.events.push(WorldEvent::Interacted {
                    ent: *ent,
                    object: *object,
                });
                Ok(true)
            }
            Input::PlaceObject {
                object,
                kind,
                tile,
                dir,
                slot,
            } => self.place_object(*object, kind, *tile, *dir, slot.clone()),
            Input::SetAutonomy { ent, enabled } => {
                if !self.agents.contains(*ent) {
                    return Err(WorldError::UnknownEnt(ent.0));
                }
                self.set_autonomy(*ent, *enabled);
                Ok(true)
            }
            Input::SetDelegation { ent, objects } => {
                if !self.agents.contains(*ent) {
                    return Err(WorldError::UnknownEnt(ent.0));
                }
                self.set_delegation(*ent, objects);
                Ok(true)
            }
            Input::CancelTask { ent } => {
                if !self.agents.contains(*ent) {
                    return Err(WorldError::UnknownEnt(ent.0));
                }
                Ok(self.cancel_current(*ent))
            }
            Input::TaskResult {
                task,
                revision,
                ok,
                code,
            } => Ok(self.task_result(*task, *revision, *ok, code)),
            Input::SetTaskParams { params } => {
                self.tasks.params = params.clone();
                Ok(true)
            }
            Input::RemoveObject { object } => {
                let o = self.objects.remove(*object)?;
                if !o.walkable {
                    self.grid.set_footprint(o.tile, o.footprint, false);
                }
                self.tasks_on_removed(*object);
                self.events
                    .push(WorldEvent::ObjectRemoved { object: *object });
                self.obj_log
                    .push_back((self.tick + 1, ObjDelta::Removed(*object)));
                self.trim_obj_log();
                Ok(true)
            }
        }
    }

    fn place_object(
        &mut self,
        id: ObjectId,
        kind: &str,
        tile: Tile,
        dir: u8,
        slot: Option<String>,
    ) -> Result<bool> {
        if !self.map.contains(tile) {
            return Err(WorldError::OutOfBounds {
                x: tile.x,
                y: tile.y,
            });
        }
        if let Some(name) = &slot {
            let def = self
                .map
                .slot(name)
                .ok_or_else(|| WorldError::BadSlot(name.clone()))?;
            if !def.accepts.is_empty() && !def.accepts.iter().any(|a| a == kind) {
                return Err(WorldError::BadSlot(name.clone()));
            }
        }
        // Moving an existing object frees the tiles it used to block first.
        if let Some(old) = self.objects.get(id).cloned() {
            if !old.walkable {
                self.grid.set_footprint(old.tile, old.footprint, false);
            }
        }
        let (walkable, height, footprint) = self.palette_of(kind);
        let o = Object {
            id,
            kind: kind.to_string(),
            tile,
            dir,
            slot,
            footprint,
            walkable,
            height,
        };
        if !o.walkable {
            self.grid.set_footprint(o.tile, o.footprint, true);
        }
        if !o.walkable && !self.tasks.tasks.is_empty() {
            self.tasks_on_blocked();
        }
        let state = object_state(&o);
        self.objects.put(o);
        self.events.push(WorldEvent::ObjectPlaced {
            object: id,
            at: tile,
        });
        self.obj_log
            .push_back((self.tick + 1, ObjDelta::Placed(state)));
        self.trim_obj_log();
        Ok(true)
    }

    /// Route an agent to a tile, snapping to the nearest walkable one so a
    /// decision naming a wall still does something sensible.
    pub(crate) fn route(&mut self, slot: usize, to: Tile) -> Result<()> {
        let from = self.agents.tile_of(slot);
        let goal = self
            .grid
            .nearest_passable(to, 4)
            .ok_or(WorldError::NotWalkable { x: to.x, y: to.y })?;
        let mut buf = std::mem::take(&mut self.path_buf);
        let found = self.astar.find(&self.grid, from, goal, &mut buf);
        if found {
            self.agents.path[slot].set(&buf);
            if buf.is_empty() {
                self.agents.set_activity(slot, Activity::Idle);
            } else {
                self.agents.set_activity(slot, Activity::Walking);
            }
        }
        self.path_buf = buf;
        if !found {
            return Err(WorldError::NoPath {
                fx: from.x,
                fy: from.y,
                tx: goal.x,
                ty: goal.y,
            });
        }
        Ok(())
    }

    // --- snapshots and diffs ---

    /// The whole observable state.
    pub fn snapshot(&self) -> Snapshot {
        let mut agents = Vec::with_capacity(self.agents.len());
        for (id, slot) in self.agents.ids_sorted() {
            agents.push(AgentState {
                id,
                name: self.agents.name[slot].clone(),
                x: self.agents.x[slot],
                y: self.agents.y[slot],
                z: self.agents.z[slot],
                dir: self.agents.dir[slot],
                anim: self.agents.anim[slot],
                energy: self.agents.energy[slot],
                activity: self.agents.activity[slot],
            });
        }
        Snapshot {
            tick: self.tick,
            seed: self.seed,
            rng_state: self.rng.state(),
            map_hash: self.map.hash(),
            sleep: self.sleep,
            agents,
            objects: self.objects.all().iter().map(object_state).collect(),
        }
    }

    /// What changed since `tick`, as seen from `interest`.
    ///
    /// An agent that left the circle is reported as a despawn, so the client
    /// can drop it; the circle used for leaving is slightly wider than the one
    /// used for entering, so an agent walking the boundary does not flicker.
    pub fn diff_since(&self, tick: u64, interest: &Interest) -> Result<Diff> {
        let frame = self
            .frames
            .iter()
            .find(|f| f.tick == tick)
            .ok_or(WorldError::TickTooOld {
                asked: tick,
                oldest: self.frames.front().map(|f| f.tick).unwrap_or(self.tick),
            })?;

        let wide = interest.hysteresis();
        let mut old: BTreeMap<EntId, &ObsAgent> = BTreeMap::new();
        for a in &frame.agents {
            old.insert(a.id, a);
        }

        let mut ents = Vec::new();
        let mut seen = BTreeMap::new();
        for (id, slot) in self.agents.ids_sorted() {
            let now = self.obs(slot);
            seen.insert(id, ());
            let visible = interest.covers(now.tile());
            match old.get(&id) {
                Some(was) => {
                    let was_visible = wide.covers(was.tile());
                    if visible {
                        if was_visible {
                            if let Some(d) = was.delta_to(&now) {
                                ents.push(d);
                            }
                        } else {
                            ents.push(self.spawn_delta(slot, &now));
                        }
                    } else if was_visible {
                        ents.push(EntDelta::despawned(id));
                    }
                }
                None if visible => ents.push(self.spawn_delta(slot, &now)),
                None => {}
            }
        }
        for (id, was) in &old {
            if !seen.contains_key(id) && wide.covers(was.tile()) {
                ents.push(EntDelta::despawned(*id));
            }
        }
        ents.sort_by_key(|d| d.id);

        let mut objects: BTreeMap<ObjectId, ObjDelta> = BTreeMap::new();
        for (t, d) in &self.obj_log {
            if *t <= tick {
                continue;
            }
            let keep = match d {
                ObjDelta::Placed(o) => interest.covers(o.tile),
                ObjDelta::Removed(id) => self
                    .objects
                    .get(*id)
                    .map(|o| interest.covers(o.tile))
                    .unwrap_or(true),
            };
            if keep {
                objects.insert(d.id(), d.clone());
            }
        }

        let mut events = Vec::new();
        for (t, e) in &self.event_log {
            if *t <= tick {
                continue;
            }
            if self.event_visible(e, interest) {
                events.push(e.clone());
            }
        }

        Ok(Diff {
            from: tick,
            to: self.tick,
            map_hash: self.map.hash(),
            rng_state: self.rng.state(),
            sleep: self.sleep,
            ents,
            objects: objects.into_values().collect(),
            events,
        })
    }

    fn event_visible(&self, e: &WorldEvent, interest: &Interest) -> bool {
        if interest.is_everything() {
            return true;
        }
        if let Some(at) = e.at() {
            return interest.covers(at);
        }
        match e.ent() {
            Some(id) => match self.agents.slot(id) {
                Some(slot) => interest.covers(self.agents.tile_of(slot)),
                // The agent is gone; the client is told regardless, or it
                // would keep a ghost forever.
                None => true,
            },
            None => true,
        }
    }

    fn spawn_delta(&self, slot: usize, now: &ObsAgent) -> EntDelta {
        let mut d = EntDelta::spawned(&AgentState {
            id: now.id,
            name: self.agents.name[slot].clone(),
            x: now.x,
            y: now.y,
            z: now.z,
            dir: now.dir,
            anim: now.anim,
            energy: now.energy,
            activity: now.activity,
        });
        d.id = now.id;
        d
    }

    /// Fold a diff into this world. Used by a replica — the webview's mirror,
    /// or a visitor's client — never by the authority itself.
    pub fn apply(&mut self, diff: &Diff) -> Result<()> {
        if diff.map_hash != self.map.hash() {
            return Err(WorldError::MapMismatch {
                blob: diff.map_hash,
                world: self.map.hash(),
            });
        }
        if diff.from != self.tick {
            return Err(WorldError::DiffGap {
                from: diff.from,
                tick: self.tick,
            });
        }
        for d in &diff.ents {
            if d.is_despawn() {
                let _ = self.agents.despawn(d.id);
                self.mailbox.forget(d.id);
                continue;
            }
            let slot = match self.agents.slot(d.id) {
                Some(s) => s,
                None => {
                    if !d.is_spawn() {
                        return Err(WorldError::UnknownEnt(d.id.0));
                    }
                    let rng = self.rng;
                    self.agents.spawn(
                        d.id,
                        d.name.as_deref().unwrap_or(""),
                        Tile::new(d.x.floor_tile(), d.y.floor_tile()),
                        d.z,
                        rng,
                    )?
                }
            };
            if d.mask & FIELD_POS != 0 {
                self.agents.x[slot] = d.x;
                self.agents.y[slot] = d.y;
                self.agents.z[slot] = d.z;
            }
            if d.mask & FIELD_DIR != 0 {
                self.agents.dir[slot] = d.dir;
            }
            if d.mask & FIELD_ANIM != 0 {
                self.agents.anim[slot] = d.anim;
            }
            if d.mask & FIELD_ENERGY != 0 {
                self.agents.energy[slot] = d.energy;
            }
            if d.mask & FIELD_ACT != 0 {
                self.agents.activity[slot] = d.activity;
            }
            if let Some(n) = &d.name {
                self.agents.name[slot] = n.clone();
            }
        }
        for d in &diff.objects {
            match d {
                ObjDelta::Removed(id) => {
                    if let Ok(o) = self.objects.remove(*id) {
                        if !o.walkable {
                            self.grid.set_footprint(o.tile, o.footprint, false);
                        }
                    }
                }
                ObjDelta::Placed(s) => {
                    if let Some(old) = self.objects.get(s.id).cloned() {
                        if !old.walkable {
                            self.grid.set_footprint(old.tile, old.footprint, false);
                        }
                    }
                    let o = Object {
                        id: s.id,
                        kind: s.kind.clone(),
                        tile: s.tile,
                        dir: s.dir,
                        slot: s.slot.clone(),
                        footprint: s.footprint,
                        walkable: s.walkable,
                        height: s.height,
                    };
                    if !o.walkable {
                        self.grid.set_footprint(o.tile, o.footprint, true);
                    }
                    self.objects.put(o);
                }
            }
        }
        self.tick = diff.to;
        self.rng = Rng::from_state(diff.rng_state);
        self.sleep = diff.sleep;
        self.record_frame();
        Ok(())
    }

    // --- history ---

    pub(crate) fn obs(&self, slot: usize) -> ObsAgent {
        ObsAgent {
            id: self.agents.id[slot],
            x: self.agents.x[slot],
            y: self.agents.y[slot],
            z: self.agents.z[slot],
            dir: self.agents.dir[slot],
            anim: self.agents.anim[slot],
            energy: self.agents.energy[slot],
            activity: self.agents.activity[slot],
        }
    }

    pub(crate) fn record_frame(&mut self) {
        let mut frame = if self.frames.len() >= HISTORY_TICKS {
            self.frames.pop_front().unwrap_or_default()
        } else {
            self.spare_frames.pop().unwrap_or_default()
        };
        frame.tick = self.tick;
        frame.agents.clear();
        for (_, slot) in self.agents.ids_sorted() {
            frame.agents.push(self.obs(slot));
        }
        self.frames.push_back(frame);
    }

    fn commit_events(&mut self) {
        for e in &self.events {
            self.event_log.push_back((self.tick, e.clone()));
        }
        while self.event_log.len() > HISTORY_EVENTS {
            self.event_log.pop_front();
        }
    }

    fn trim_obj_log(&mut self) {
        while self.obj_log.len() > HISTORY_OBJ {
            self.obj_log.pop_front();
        }
    }

    pub(crate) fn mailbox_mut(&mut self) -> &mut Mailbox {
        &mut self.mailbox
    }

    /// The events of the last `step()`, in order.
    pub fn last_events(&self) -> &[WorldEvent] {
        &self.events
    }
}

pub(crate) fn object_state(o: &Object) -> ObjectState {
    ObjectState {
        id: o.id,
        kind: o.kind.clone(),
        tile: o.tile,
        dir: o.dir,
        walkable: o.walkable,
        height: o.height,
        footprint: o.footprint,
        slot: o.slot.clone(),
    }
}

fn input_ent(input: &Input) -> EntId {
    match input {
        Input::Spawn { ent, .. }
        | Input::Despawn { ent }
        | Input::Move { ent, .. }
        | Input::Decision { ent, .. }
        | Input::SetEnergy { ent, .. }
        | Input::Caption { ent, .. }
        | Input::SetRoutine { ent, .. }
        | Input::Interact { ent, .. }
        | Input::SetAutonomy { ent, .. }
        | Input::SetDelegation { ent, .. }
        | Input::CancelTask { ent } => *ent,
        _ => EntId::NONE,
    }
}
