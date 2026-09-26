//! What happens inside one tick.
//!
//! Order matters and is fixed: routine, then mailbox, then energy, then
//! movement. It is written as an `impl World` in its own module so that
//! `world.rs` stays the contract and this stays the behaviour.
//!
//! Nothing here calls a language model, allocates in steady state, or reads a
//! clock. Agents are visited in ascending id order, never in slot order, so
//! the result cannot depend on the order they happened to be spawned in.

use crate::ents::agent::{Activity, ENERGY_SPENT, ENERGY_TIRED};
use crate::ents::id::{EntId, ObjectId};
use crate::ents::soa::Intent;
use crate::ents::{dir_from_delta, kind_leaf};
use crate::fixed::Fixed;
use crate::map::Tile;
use crate::sim::mailbox::Decision;
use crate::sim::routine::minute_of_day;
use crate::snapshot::diff::WorldEvent;
use crate::world::{StepReport, World};

/// Ticks a spoken line keeps the talking pose.
pub const SAY_TICKS: u16 = 20;
/// Ticks a wave lasts.
pub const WAVE_TICKS: u16 = 12;
/// Ticks a yawn lasts.
pub const YAWN_TICKS: u16 = 15;
/// One chance in this many, per tick, that a tired idle agent yawns.
pub const YAWN_ODDS: u32 = 100;

impl World {
    pub(crate) fn run_tick(&mut self, report: &mut StepReport) {
        self.fire_routines();
        self.take_decisions(report);
        self.run_tasks();
        self.energy_behaviour();
        self.move_agents(report);
    }

    /// Order buffer: slots in ascending id order, reused so the tick does not
    /// allocate.
    fn order(&mut self) -> Vec<usize> {
        let mut buf = std::mem::take(&mut self.order_buf);
        buf.clear();
        buf.extend(self.agents.ids_sorted().map(|(_, slot)| slot));
        buf
    }

    fn fire_routines(&mut self) {
        let minute = minute_of_day(self.tick());
        let order = self.order();
        for &slot in &order {
            if self.agents.last_routine_min[slot] == minute {
                continue;
            }
            self.agents.last_routine_min[slot] = minute;
            if self.agents.routine[slot].is_empty() {
                continue;
            }
            // An autonomous agent chooses its own tasks; its routine is not
            // replayed on top of them.
            if self.tasks.is_autonomous(self.agents.id[slot]) {
                continue;
            }
            if !self.agents.activity[slot].interruptible_by_routine() {
                continue;
            }
            let id = self.agents.id[slot];
            let due: Vec<Decision> = self.agents.routine[slot].due(minute).cloned().collect();
            for d in due {
                self.mailbox_mut().post(id, d);
            }
        }
        self.order_buf = order;
    }

    fn take_decisions(&mut self, report: &mut StepReport) {
        let order = self.order();
        for &slot in &order {
            let id = self.agents.id[slot];
            let Some(decision) = self.mailbox_mut().take(id) else {
                continue;
            };
            report.decisions += 1;
            let handled = self.decision_as_task(slot, id, &decision);
            let result = match handled {
                Some(r) => r,
                None => self.apply_decision(slot, id, decision),
            };
            if let Err(e) = result {
                report.rejected += 1;
                self.events.push(WorldEvent::Rejected {
                    ent: id,
                    code: e.code().to_string(),
                });
            }
        }
        self.order_buf = order;
    }

    fn apply_decision(
        &mut self,
        slot: usize,
        id: EntId,
        decision: Decision,
    ) -> crate::error::Result<()> {
        match decision {
            Decision::Idle => {
                self.agents.path[slot].clear();
                self.agents.intent[slot] = Intent::None;
                self.set_activity(slot, Activity::Idle);
                self.agents.timer[slot] = 0;
            }
            Decision::GoTo(tile) => {
                self.agents.intent[slot] = Intent::None;
                self.route(slot, tile)?;
            }
            Decision::Say(text) => {
                self.set_activity(slot, Activity::Talking);
                self.agents.timer[slot] = SAY_TICKS;
                self.events.push(WorldEvent::Said { ent: id, text });
            }
            Decision::Wave(target) => {
                // A wave stops the walk: an agent asking for attention stands
                // still, it does not wave over its shoulder on the way out.
                self.agents.path[slot].clear();
                self.agents.intent[slot] = Intent::None;
                if let Some(other) = self.agents.slot(target) {
                    let a = self.agents.tile_of(slot);
                    let b = self.agents.tile_of(other);
                    self.agents.dir[slot] = dir_from_delta(b.x - a.x, b.y - a.y);
                }
                self.set_activity(slot, Activity::Waving(target));
                self.agents.timer[slot] = WAVE_TICKS;
            }
            Decision::Sit(object) => self.head_for(slot, object, Intent::Sit(object), seatable)?,
            Decision::Work(object) => {
                self.head_for(slot, object, Intent::Work(object), workable)?
            }
            Decision::Sleep => {
                let bed = self.nearest_bed(slot);
                match bed {
                    Some(b) => self.head_for(slot, b, Intent::Sleep, |_| true)?,
                    None => {
                        self.agents.path[slot].clear();
                        self.agents.intent[slot] = Intent::None;
                        self.start_sleeping(slot, id);
                    }
                }
            }
        }
        Ok(())
    }

    /// Walk to an object and remember what to do on arrival. Already standing
    /// next to it counts as arriving.
    fn head_for(
        &mut self,
        slot: usize,
        object: ObjectId,
        intent: Intent,
        ok: fn(&str) -> bool,
    ) -> crate::error::Result<()> {
        let Some(o) = self.objects.get(object) else {
            return Err(crate::error::WorldError::UnknownObject(object.0));
        };
        if !ok(&o.kind) {
            return Err(crate::error::WorldError::UnknownObject(object.0));
        }
        let target = self.free_spot_by(slot, o.approach(), o.tile);
        self.agents.intent[slot] = intent;
        self.route(slot, target)?;
        if self.agents.path[slot].is_empty() {
            let id = self.agents.id[slot];
            self.finish_intent(slot, id);
        }
        Ok(())
    }

    /// The approach tile, or the nearest free tile beside it when another
    /// agent already stands there or is on its way: two agents sharing one
    /// workbench work side by side instead of inside each other.
    fn free_spot_by(&self, slot: usize, approach: Tile, object: Tile) -> Tile {
        const RING: [(i32, i32); 8] = [
            (0, 0),
            (1, 0),
            (-1, 0),
            (0, 1),
            (1, 1),
            (-1, 1),
            (2, 0),
            (-2, 0),
        ];
        let taken = |t: Tile| {
            self.agents.ids_sorted().any(|(_, other)| {
                other != slot
                    && (self.agents.path[other].goal() == Some(t)
                        || (self.agents.path[other].is_empty() && self.agents.tile_of(other) == t))
            })
        };
        for (dx, dy) in RING {
            let t = Tile::new(approach.x + dx, approach.y + dy);
            if t != object && self.grid().passable(t) && !taken(t) {
                return t;
            }
        }
        approach
    }

    fn nearest_bed(&self, slot: usize) -> Option<ObjectId> {
        let here = self.agents.tile_of(slot);
        let mut best: Option<(u64, ObjectId)> = None;
        for o in self.objects.all() {
            if !o.is_bed() {
                continue;
            }
            let d = o.tile.dist2(here);
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, o.id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn energy_behaviour(&mut self) {
        let order = self.order();
        for &slot in &order {
            let id = self.agents.id[slot];
            // Needs of autonomous agents are the task step's business.
            if self.tasks.is_autonomous(id) {
                continue;
            }
            let energy = self.agents.energy[slot];
            let activity = self.agents.activity[slot];
            if energy >= ENERGY_TIRED {
                // Fully rested agents wake up on their own: the quota window
                // turning over is the world's sunrise (plan §9.1).
                if activity == Activity::Sleeping {
                    self.set_activity(slot, Activity::Idle);
                    self.events.push(WorldEvent::Woke { ent: id });
                }
                continue;
            }
            if activity == Activity::Sleeping || !self.agents.path[slot].is_empty() {
                continue;
            }
            if energy < ENERGY_SPENT && activity == Activity::Idle {
                match self.nearest_bed(slot) {
                    Some(b) => {
                        let _ = self.head_for(slot, b, Intent::Sleep, |_| true);
                    }
                    None => self.start_sleeping(slot, id),
                }
                continue;
            }
            if activity == Activity::Idle && self.agents.rng[slot].chance(1, YAWN_ODDS) {
                self.set_activity(slot, Activity::Yawning);
                self.agents.timer[slot] = YAWN_TICKS;
                self.events.push(WorldEvent::Yawned { ent: id });
            }
        }
        self.order_buf = order;
    }

    fn move_agents(&mut self, report: &mut StepReport) {
        let order = self.order();
        for &slot in &order {
            let id = self.agents.id[slot];
            if self.agents.timer[slot] > 0 {
                self.agents.timer[slot] -= 1;
                if self.agents.timer[slot] == 0 && self.agents.path[slot].is_empty() {
                    self.set_activity(slot, Activity::Idle);
                }
            }
            let Some(next) = self.agents.path[slot].next() else {
                continue;
            };
            if self.agents.activity[slot] != Activity::Walking {
                self.set_activity(slot, Activity::Walking);
            }
            let tx = Fixed::from_tiles(next.x) + Fixed::HALF;
            let ty = Fixed::from_tiles(next.y) + Fixed::HALF;
            let dx = tx - self.agents.x[slot];
            let dy = ty - self.agents.y[slot];
            let dist = Fixed::len(dx, dy);
            let speed = self.agents.speed(slot);
            self.agents.dir[slot] = dir_from_delta(dx.0, dy.0);
            if dist.0 <= speed.0 {
                let z = self.map().z_at(next);
                self.agents.x[slot] = tx;
                self.agents.y[slot] = ty;
                self.agents.z[slot] = z;
                self.agents.path[slot].advance();
                report.moved += 1;
                if self.agents.path[slot].is_empty() {
                    self.events.push(WorldEvent::Arrived { ent: id, at: next });
                    self.finish_intent(slot, id);
                }
            } else {
                // Exact integer scaling: no float, so every machine lands on
                // the same sub-unit.
                let nx = dx.0 as i64 * speed.0 as i64 / dist.0 as i64;
                let ny = dy.0 as i64 * speed.0 as i64 / dist.0 as i64;
                self.agents.x[slot] += Fixed(nx as i32);
                self.agents.y[slot] += Fixed(ny as i32);
                report.moved += 1;
            }
        }
        self.order_buf = order;
    }

    /// Carry out what the agent walked here for.
    fn finish_intent(&mut self, slot: usize, id: EntId) {
        match self.agents.intent[slot] {
            Intent::None => {
                if self.agents.activity[slot] == Activity::Walking {
                    self.set_activity(slot, Activity::Idle);
                }
            }
            Intent::Sit(o) => {
                self.face_object(slot, o);
                self.set_activity(slot, Activity::Sitting(o));
                self.events
                    .push(WorldEvent::Interacted { ent: id, object: o });
            }
            Intent::Work(o) => {
                self.face_object(slot, o);
                self.set_activity(slot, Activity::Working(o));
                self.events
                    .push(WorldEvent::Interacted { ent: id, object: o });
            }
            Intent::Sleep => self.start_sleeping(slot, id),
        }
        self.agents.intent[slot] = Intent::None;
    }

    pub(crate) fn face_object(&mut self, slot: usize, object: ObjectId) {
        if let Some(o) = self.objects.get(object) {
            let here = self.agents.tile_of(slot);
            self.agents.dir[slot] = dir_from_delta(o.tile.x - here.x, o.tile.y - here.y);
        }
    }

    fn start_sleeping(&mut self, slot: usize, id: EntId) {
        if self.agents.activity[slot] == Activity::Sleeping {
            return;
        }
        self.set_activity(slot, Activity::Sleeping);
        self.agents.timer[slot] = 0;
        self.events.push(WorldEvent::Slept { ent: id });
    }

    pub(crate) fn set_activity(&mut self, slot: usize, a: Activity) {
        let changed = self.agents.activity[slot] != a;
        self.agents.set_activity(slot, a);
        if changed {
            let id = self.agents.id[slot];
            self.events.push(WorldEvent::StartedAnim {
                ent: id,
                anim: a.anim(),
                dir: self.agents.dir[slot],
            });
        }
    }

    /// Place every agent where its routine says it should be at the current
    /// game minute, without simulating the ticks in between. This is the whole
    /// point of `catch_up`: eight hours cost one pass over the agents.
    pub(crate) fn settle_to_routine(&mut self) {
        let minute = minute_of_day(self.tick());
        self.mailbox_mut().clear();
        let order = self.order();
        for &slot in &order {
            let id = self.agents.id[slot];
            if self.tasks.is_autonomous(id) {
                continue; // settle_tasks
            }
            self.agents.path[slot].clear();
            self.agents.intent[slot] = Intent::None;
            self.agents.timer[slot] = 0;
            self.agents.last_routine_min[slot] = minute;
            let current = self.agents.routine[slot].current(minute).cloned();
            match current {
                Some(Decision::GoTo(tile)) => {
                    self.teleport(slot, tile);
                    self.set_activity(slot, Activity::Idle);
                }
                Some(Decision::Sit(o)) => {
                    if let Some(t) = self.objects.get(o).map(|x| x.approach()) {
                        self.teleport(slot, t);
                        self.face_object(slot, o);
                        self.set_activity(slot, Activity::Sitting(o));
                    }
                }
                Some(Decision::Work(o)) => {
                    if let Some(t) = self.objects.get(o).map(|x| x.approach()) {
                        self.teleport(slot, t);
                        self.face_object(slot, o);
                        self.set_activity(slot, Activity::Working(o));
                    }
                }
                Some(Decision::Sleep) => {
                    if let Some(bed) = self.nearest_bed(slot) {
                        if let Some(t) = self.objects.get(bed).map(|x| x.approach()) {
                            self.teleport(slot, t);
                        }
                    }
                    if self.agents.activity[slot] != Activity::Sleeping {
                        self.set_activity(slot, Activity::Sleeping);
                        self.events.push(WorldEvent::Slept { ent: id });
                    }
                }
                _ => self.set_activity(slot, Activity::Idle),
            }
        }
        self.order_buf = order;
    }

    fn teleport(&mut self, slot: usize, to: Tile) {
        let Some(tile) = self.grid().nearest_passable(to, 8) else {
            return;
        };
        let z = self.map().z_at(tile);
        self.agents.x[slot] = Fixed::from_tiles(tile.x) + Fixed::HALF;
        self.agents.y[slot] = Fixed::from_tiles(tile.y) + Fixed::HALF;
        self.agents.z[slot] = z;
    }
}

fn seatable(kind: &str) -> bool {
    matches!(kind_leaf(kind), "chair" | "bed" | "sofa" | "stool")
}

fn workable(kind: &str) -> bool {
    matches!(
        kind_leaf(kind),
        "workbench" | "desk" | "table" | "bookshelf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ents::agent::{ANIM_SLEEP, ANIM_WALK};
    use crate::map::fixtures::one_room;
    use crate::sim::routine::{hm, Routine, RoutineEntry, TICKS_PER_GAME_MINUTE};
    use crate::world::Input;

    fn world() -> World {
        let mut w = World::new(7, one_room()).unwrap();
        w.step(&[Input::Spawn {
            ent: EntId(1),
            name: "omni".into(),
            at: Tile::new(2, 8),
        }]);
        w
    }

    #[test]
    fn an_agent_walks_to_where_it_was_sent_and_stops() {
        let mut w = world();
        w.step(&[Input::Move {
            ent: EntId(1),
            to: Tile::new(10, 8),
        }]);
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Walking));
        let mut ticks = 0;
        while w.activity_of(EntId(1)) == Some(Activity::Walking) && ticks < 400 {
            w.step(&[]);
            ticks += 1;
        }
        assert!(ticks < 400, "never arrived");
        assert_eq!(w.tile_of(EntId(1)), Some(Tile::new(10, 8)));
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Idle));
    }

    #[test]
    fn walking_speed_matches_the_budget() {
        let mut w = world();
        w.step(&[Input::Move {
            ent: EntId(1),
            to: Tile::new(12, 8),
        }]);
        let mut ticks = 0;
        while w.activity_of(EntId(1)) == Some(Activity::Walking) && ticks < 400 {
            w.step(&[]);
            ticks += 1;
        }
        // Ten tiles at 38/256 of a tile per tick is about 68 ticks, 6.8 s.
        assert!((60..=80).contains(&ticks), "{ticks} ticks for ten tiles");
    }

    #[test]
    fn a_decision_goes_through_the_mailbox_not_straight_in() {
        let mut w = world();
        w.step(&[Input::Decision {
            ent: EntId(1),
            decision: Decision::Say("oi".into()),
        }]);
        // Posted and taken in the same step, because the mailbox is drained
        // inside the tick that follows the inputs.
        assert!(w
            .last_events()
            .iter()
            .any(|e| matches!(e, WorldEvent::Said { .. })));
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Talking));
        assert_eq!(w.mailbox_pending(), 0);
    }

    #[test]
    fn a_spoken_line_wears_off() {
        let mut w = world();
        w.step(&[Input::Decision {
            ent: EntId(1),
            decision: Decision::Say("oi".into()),
        }]);
        for _ in 0..SAY_TICKS + 1 {
            w.step(&[]);
        }
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Idle));
    }

    #[test]
    fn work_walks_to_the_bench_and_starts_working() {
        let mut w = world();
        w.step(&[Input::Decision {
            ent: EntId(1),
            decision: Decision::Work(ObjectId(1)),
        }]);
        for _ in 0..200 {
            w.step(&[]);
            if matches!(w.activity_of(EntId(1)), Some(Activity::Working(_))) {
                break;
            }
        }
        assert_eq!(
            w.activity_of(EntId(1)),
            Some(Activity::Working(ObjectId(1)))
        );
        assert_eq!(w.tile_of(EntId(1)), Some(Tile::new(4, 5)));
    }

    #[test]
    fn working_at_a_plant_is_refused() {
        let mut w = world();
        w.step(&[Input::PlaceObject {
            object: ObjectId(2),
            kind: "object/plant".into(),
            tile: Tile::new(9, 9),
            dir: 0,
            slot: None,
        }]);
        let r = w.step(&[Input::Decision {
            ent: EntId(1),
            decision: Decision::Work(ObjectId(2)),
        }]);
        assert_eq!(r.rejected, 1);
        assert!(w
            .last_events()
            .iter()
            .any(|e| matches!(e, WorldEvent::Rejected { .. })));
    }

    #[test]
    fn a_spent_agent_falls_asleep_by_itself() {
        let mut w = world();
        // The room has no bed, so it sleeps where it stands, in the same tick
        // the quota runs out.
        w.step(&[Input::SetEnergy {
            ent: EntId(1),
            energy: 1,
        }]);
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Sleeping));
        assert!(w
            .last_events()
            .iter()
            .any(|e| matches!(e, WorldEvent::Slept { .. })));
        w.step(&[]);
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Sleeping));
        assert!(
            !w.last_events()
                .iter()
                .any(|e| matches!(e, WorldEvent::Slept { .. })),
            "it only falls asleep once"
        );
    }

    #[test]
    fn a_refilled_quota_wakes_the_agent_up() {
        let mut w = world();
        w.step(&[Input::SetEnergy {
            ent: EntId(1),
            energy: 1,
        }]);
        w.step(&[]);
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Sleeping));
        w.step(&[Input::SetEnergy {
            ent: EntId(1),
            energy: 255,
        }]);
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Idle));
        assert!(w
            .last_events()
            .iter()
            .any(|e| matches!(e, WorldEvent::Woke { .. })));
    }

    #[test]
    fn a_tired_agent_yawns_eventually_and_a_rested_one_never_does() {
        let mut w = world();
        w.step(&[Input::SetEnergy {
            ent: EntId(1),
            energy: ENERGY_TIRED - 1,
        }]);
        let mut yawned = false;
        for _ in 0..2000 {
            w.step(&[]);
            yawned |= w
                .last_events()
                .iter()
                .any(|e| matches!(e, WorldEvent::Yawned { .. }));
            if yawned {
                break;
            }
        }
        assert!(yawned);

        let mut w = world();
        for _ in 0..2000 {
            w.step(&[]);
            assert!(!w
                .last_events()
                .iter()
                .any(|e| matches!(e, WorldEvent::Yawned { .. })));
        }
    }

    #[test]
    fn a_tired_agent_walks_slower() {
        let mut fresh = world();
        let mut tired = world();
        tired.step(&[Input::SetEnergy {
            ent: EntId(1),
            energy: 20,
        }]);
        for w in [&mut fresh, &mut tired] {
            w.step(&[Input::Move {
                ent: EntId(1),
                to: Tile::new(12, 8),
            }]);
        }
        for _ in 0..30 {
            fresh.step(&[]);
            tired.step(&[]);
        }
        let a = fresh.tile_of(EntId(1)).unwrap().x;
        let b = tired.tile_of(EntId(1)).unwrap().x;
        assert!(a > b, "rested {a} should be ahead of tired {b}");
    }

    #[test]
    fn a_routine_fires_on_its_minute() {
        let mut w = world();
        let routine = Routine::new(vec![RoutineEntry {
            minute: hm(0, 2),
            decision: Decision::Work(ObjectId(1)),
        }]);
        w.step(&[Input::SetRoutine {
            ent: EntId(1),
            routine,
        }]);
        // Minute two of the game day is tick 16.
        for _ in 0..2 * TICKS_PER_GAME_MINUTE + 4 {
            w.step(&[]);
        }
        let act = w.activity_of(EntId(1)).unwrap();
        assert!(
            matches!(act, Activity::Walking | Activity::Working(_)),
            "{act:?}"
        );
    }

    #[test]
    fn catch_up_places_agents_without_replaying() {
        let mut w = world();
        let routine = Routine::new(vec![
            RoutineEntry {
                minute: hm(8, 0),
                decision: Decision::Work(ObjectId(1)),
            },
            RoutineEntry {
                minute: hm(22, 0),
                decision: Decision::Sleep,
            },
        ]);
        w.step(&[Input::SetRoutine {
            ent: EntId(1),
            routine,
        }]);
        let before = w.tick();
        // Nine game hours in one call.
        let r = w.catch_up(9 * 60 * TICKS_PER_GAME_MINUTE * 100);
        assert!(r.caught_up > 0);
        assert_eq!(w.tick(), before + r.caught_up);
        assert_eq!(
            w.activity_of(EntId(1)),
            Some(Activity::Working(ObjectId(1)))
        );
        assert!(w
            .last_events()
            .iter()
            .any(|e| matches!(e, WorldEvent::CaughtUp { .. })));
    }

    #[test]
    fn catch_up_of_less_than_a_tick_does_nothing() {
        let mut w = world();
        let before = w.tick();
        let r = w.catch_up(50);
        assert_eq!(r.caught_up, 0);
        assert_eq!(w.tick(), before);
    }

    #[test]
    fn the_anim_byte_always_matches_the_activity() {
        let mut w = world();
        w.step(&[Input::Move {
            ent: EntId(1),
            to: Tile::new(10, 8),
        }]);
        let s = w.snapshot();
        assert_eq!(s.agents[0].anim, ANIM_WALK);
        w.step(&[Input::SetEnergy {
            ent: EntId(1),
            energy: 0,
        }]);
        for _ in 0..200 {
            w.step(&[]);
            if w.activity_of(EntId(1)) == Some(Activity::Sleeping) {
                break;
            }
        }
        assert_eq!(w.snapshot().agents[0].anim, ANIM_SLEEP);
    }

    #[test]
    fn waving_faces_the_other_agent() {
        let mut w = world();
        w.step(&[Input::Spawn {
            ent: EntId(2),
            name: "ada".into(),
            at: Tile::new(9, 8),
        }]);
        w.step(&[Input::Decision {
            ent: EntId(1),
            decision: Decision::Wave(EntId(2)),
        }]);
        assert_eq!(w.activity_of(EntId(1)), Some(Activity::Waving(EntId(2))));
        assert_eq!(w.snapshot().agents[0].dir, crate::ents::DIR_E);
    }
}
