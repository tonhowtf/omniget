//! Full checkpoints: everything a `World` needs to carry on exactly where it
//! stopped, including what a `Snapshot` deliberately leaves out.
//!
//! A snapshot is the *observable* state — a renderer needs nothing more. A
//! server that restarts needs the rest too: each agent's path and parked
//! intent, its routine and the minute it last fired, its private RNG stream,
//! the decisions still queued in the mailbox. Without those a replica would be
//! visually identical and then diverge on the next tick, which is exactly the
//! failure `apply(diff_since)` was built to rule out.
//!
//! `Checkpoint` is plain data with `serde` derives and no binary format of its
//! own: the server keeps it as JSONB, where an operator can read it. The map
//! is not inside — it is identified by hash and supplied by the caller, as
//! `World::new` does — so a checkpoint of a 64-agent region is a few dozen KB.
//!
//! Replay contract (server side): `World::from_checkpoint(map, cp)` followed
//! by the same `step(inputs)` calls that were applied after `cp.tick` yields
//! the same `snapshot().fingerprint()`; `tests/checkpoint.rs` proves the round
//! trip on the crate's own fixtures.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::ents::agent::Activity;
use crate::ents::id::EntId;
use crate::ents::object::Object;
use crate::ents::soa::{Intent, Path};
use crate::ents::{Agents, Objects};
use crate::error::{Result, WorldError};
use crate::fixed::Fixed;
use crate::map::{Map, MapDef};
use crate::path::{AStar, Grid};
use crate::rng::Rng;
use crate::sim::mailbox::Mailbox;
use crate::sim::routine::Routine;
use crate::sim::sleep::SleepState;
use crate::world::{World, HISTORY_EVENTS, HISTORY_OBJ, HISTORY_TICKS};

/// Bumped when a field is added or its meaning changes. A server refuses a
/// checkpoint from a newer crate instead of guessing.
/// 2: `tasks` (session 06). A version-1 checkpoint loads with an empty book.
pub const CHECKPOINT_VERSION: u32 = 2;

/// One agent, every component.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentCheckpoint {
    pub id: EntId,
    pub name: String,
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
    pub dir: u8,
    pub anim: u8,
    pub energy: u8,
    pub activity: Activity,
    pub path: Path,
    pub intent: Intent,
    pub timer: u16,
    pub routine: Routine,
    pub rng: Rng,
    pub last_routine_min: u16,
}

/// The whole simulation state at one tick.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub seed: u64,
    pub tick: u64,
    pub rng: Rng,
    pub sleep: SleepState,
    /// Hash of the static map this state was taken on. The map itself is
    /// supplied again on restore and must hash the same.
    pub map_hash: u64,
    /// Ascending by id.
    pub agents: Vec<AgentCheckpoint>,
    /// Ascending by id.
    pub objects: Vec<Object>,
    pub mailbox: Mailbox,
    #[serde(default)]
    pub tasks: crate::sim::tasks::TaskBook,
}

impl World {
    /// Everything, for a durable checkpoint. Never in the tick path: it
    /// allocates and clones every routine.
    pub fn checkpoint(&self) -> Checkpoint {
        let mut agents = Vec::with_capacity(self.agents.len());
        for (id, slot) in self.agents.ids_sorted() {
            let a = &self.agents;
            agents.push(AgentCheckpoint {
                id,
                name: a.name[slot].clone(),
                x: a.x[slot],
                y: a.y[slot],
                z: a.z[slot],
                dir: a.dir[slot],
                anim: a.anim[slot],
                energy: a.energy[slot],
                activity: a.activity[slot],
                path: a.path[slot].clone(),
                intent: a.intent[slot],
                timer: a.timer[slot],
                routine: a.routine[slot].clone(),
                rng: a.rng[slot],
                last_routine_min: a.last_routine_min[slot],
            });
        }
        Checkpoint {
            version: CHECKPOINT_VERSION,
            seed: self.seed,
            tick: self.tick,
            rng: self.rng,
            sleep: self.sleep,
            map_hash: self.map.hash(),
            agents,
            objects: self.objects.all().to_vec(),
            mailbox: self.mailbox.clone(),
            tasks: self.tasks.clone(),
        }
    }

    /// Rebuild a world from a checkpoint and the map it was taken on.
    ///
    /// The map's objects are *not* placed: the checkpoint's object list is the
    /// truth, since the player may have removed or moved what the map ships
    /// with. Blocked footprints are rebuilt from that list, so the grid agrees
    /// with the objects exactly as it did before.
    pub fn from_checkpoint(map: MapDef, cp: &Checkpoint) -> Result<World> {
        if cp.version > CHECKPOINT_VERSION {
            return Err(WorldError::MapInvalid(format!(
                "checkpoint version {} is newer than {CHECKPOINT_VERSION}",
                cp.version
            )));
        }
        let map = Map::load(&map)?;
        if map.hash() != cp.map_hash {
            return Err(WorldError::MapMismatch {
                blob: cp.map_hash,
                world: map.hash(),
            });
        }
        let mut grid = Grid::from_map(&map);
        let mut objects = Objects::default();
        for o in &cp.objects {
            if !o.walkable {
                grid.set_footprint(o.tile, o.footprint, true);
            }
            objects.put(o.clone());
        }
        let mut agents = Agents::default();
        for a in &cp.agents {
            // `spawn` centres the agent on a tile and seeds a stream; both are
            // overwritten below with the exact values from the checkpoint.
            let at = crate::map::Tile::new(a.x.floor_tile(), a.y.floor_tile());
            let slot = agents.spawn(a.id, &a.name, at, a.z, cp.rng)?;
            agents.x[slot] = a.x;
            agents.y[slot] = a.y;
            agents.z[slot] = a.z;
            agents.dir[slot] = a.dir;
            agents.anim[slot] = a.anim;
            agents.energy[slot] = a.energy;
            agents.activity[slot] = a.activity;
            agents.path[slot] = a.path.clone();
            agents.intent[slot] = a.intent;
            agents.timer[slot] = a.timer;
            agents.routine[slot] = a.routine.clone();
            agents.rng[slot] = a.rng;
            agents.last_routine_min[slot] = a.last_routine_min;
        }
        let mut w = World {
            seed: cp.seed,
            rng: cp.rng,
            tick: cp.tick,
            sleep: cp.sleep,
            map,
            grid,
            agents,
            objects,
            mailbox: cp.mailbox.clone(),
            tasks: cp.tasks.clone(),
            astar: AStar::new(),
            path_buf: Vec::with_capacity(64),
            order_buf: Vec::with_capacity(32),
            events: Vec::with_capacity(32),
            frames: VecDeque::with_capacity(HISTORY_TICKS),
            spare_frames: Vec::new(),
            event_log: VecDeque::with_capacity(HISTORY_EVENTS),
            obj_log: VecDeque::with_capacity(HISTORY_OBJ),
        };
        w.record_frame();
        Ok(w)
    }
}

// Silence an unused-import lint when the compiler decides `BTreeMap` is not
// needed by a future edit; it documents that mailbox queues are id-ordered.
#[allow(dead_code)]
type IdOrdered<T> = BTreeMap<EntId, T>;
