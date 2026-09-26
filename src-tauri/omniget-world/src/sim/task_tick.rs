//! The task step of the tick: expire reservations, update needs, advance
//! every live task one small step, then let a budgeted number of idle
//! autonomous agents choose. Runs after decisions and before movement, in
//! ascending id order everywhere, reading no clock.

use crate::ents::agent::Activity;
use crate::ents::id::{EntId, ObjectId};
use crate::ents::soa::Intent;
use crate::map::Tile;
use crate::sim::mailbox::Decision;
use crate::sim::tasks::{
    affordance, approach_tiles, need_delta, needs_delegation, EffectRequest, Reason, Reservation,
    Task, TaskAction, TaskId, TaskSource, TaskState,
};
use crate::world::World;

impl World {
    pub(crate) fn run_tasks(&mut self) {
        if self.tasks.autonomy.is_empty() && self.tasks.tasks.is_empty() {
            return;
        }
        let tick = self.tick();
        self.expire_reservations(tick);
        self.update_needs();
        self.advance_tasks(tick);
        self.select_tasks(tick);
    }

    fn expire_reservations(&mut self, tick: u64) {
        let stale: Vec<ObjectId> = self
            .tasks
            .reservations
            .iter()
            .filter(|(_, r)| {
                r.expires < tick
                    || self
                        .tasks
                        .tasks
                        .get(&r.task)
                        .map(|t| {
                            !t.state.holds() && t.state != TaskState::Queued
                                || t.revision != r.revision
                        })
                        .unwrap_or(true)
            })
            .map(|(o, _)| *o)
            .collect();
        for o in stale {
            self.tasks.reservations.remove(&o);
        }
    }

    fn update_needs(&mut self) {
        let period = self.tasks.params.need_period.max(1);
        let ids: Vec<EntId> = self
            .tasks
            .autonomy
            .iter()
            .filter(|(_, a)| a.enabled)
            .map(|(e, _)| *e)
            .collect();
        for ent in ids {
            let Some(slot) = self.agents.slot(ent) else {
                continue;
            };
            let a = self.tasks.autonomy.get_mut(&ent).expect("listed");
            a.need_ticks += 1;
            if a.need_ticks < period {
                continue;
            }
            a.need_ticks = 0;
            self.agents.energy[slot] = need_delta(
                &self.tasks.params,
                self.agents.activity[slot],
                self.agents.energy[slot],
            );
        }
    }

    // --- task lifecycle -------------------------------------------------------

    fn advance_tasks(&mut self, tick: u64) {
        let ids: Vec<TaskId> = self.tasks.tasks.keys().copied().collect();
        let mut routes = 0u16;
        for id in ids {
            let Some(task) = self.tasks.tasks.get(&id).cloned() else {
                continue;
            };
            let Some(slot) = self.agents.slot(task.agent) else {
                self.end_task(id, TaskState::Cancelled, Reason::Preempted, tick);
                continue;
            };
            match task.state {
                TaskState::Queued => {
                    if routes >= self.tasks.params.route_per_tick {
                        self.tasks.stats.deferred += 1;
                        continue;
                    }
                    routes += 1;
                    self.start_task(id, slot, tick);
                }
                TaskState::Blocked => {
                    if tick >= task.retry_at {
                        self.set_state(id, TaskState::Queued, Reason::None, tick);
                    }
                }
                TaskState::Travelling => {
                    self.renew(id, tick);
                    if self.agents.path[slot].is_empty() {
                        let here = self.agents.tile_of(slot);
                        if Some(here) == task.approach {
                            self.begin_execution(id, slot, tick);
                        } else {
                            // The route was cleared under it (an edit, a push):
                            // back to the queue, reservation kept.
                            self.set_state(id, TaskState::Queued, Reason::None, tick);
                        }
                    }
                }
                TaskState::Executing => {
                    self.renew(id, tick);
                    self.execute_step(id, slot, tick);
                }
                TaskState::AwaitingEffect => {
                    self.renew(id, tick);
                    if tick.saturating_sub(task.since) > self.tasks.params.effect_timeout as u64 {
                        if task.effect_sends < self.tasks.params.effect_max_sends {
                            self.send_effect(id, tick);
                        } else {
                            self.end_task(id, TaskState::Failed, Reason::EffectTimeout, tick);
                        }
                    }
                }
                TaskState::Completed | TaskState::Cancelled | TaskState::Failed => {}
            }
        }
    }

    /// Reserve, pick an approach tile that can be reached, set off.
    fn start_task(&mut self, id: TaskId, slot: usize, tick: u64) {
        let task = self.tasks.tasks[&id].clone();
        if task.action == TaskAction::Loiter || task.target == ObjectId(0) {
            // Nothing to reserve or reach: sleep on the spot, stand around.
            self.begin_execution(id, slot, tick);
            return;
        }
        let Some(object) = self.objects.get(task.target).cloned() else {
            self.end_task(id, TaskState::Cancelled, Reason::TargetGone, tick);
            return;
        };
        if affordance(&object.kind).map(|a| compatible(a, task.action)) != Some(true) {
            self.end_task(id, TaskState::Failed, Reason::NotAllowed, tick);
            return;
        }
        if needs_delegation(task.action)
            && !self
                .tasks
                .autonomy
                .get(&task.agent)
                .map(|a| a.delegated.contains(&task.target))
                .unwrap_or(false)
        {
            self.end_task(id, TaskState::Cancelled, Reason::Revoked, tick);
            return;
        }
        // Capacity one: someone else's live reservation means busy.
        if let Some(r) = self.tasks.reservations.get(&task.target) {
            if r.task != id {
                self.block(id, Reason::Busy, tick);
                return;
            }
        }
        let revision = task.revision + 1;
        self.tasks.stats.routes += 1;
        let mut chosen = None;
        let here = self.agents.tile_of(slot);
        for t in approach_tiles(&object) {
            if !self.grid().passable(t) || self.approach_taken(slot, t) {
                continue;
            }
            if t == here || self.route_exact(slot, t) {
                chosen = Some(t);
                break;
            }
        }
        let Some(approach) = chosen else {
            self.tasks.reservations.remove(&task.target);
            self.block(id, Reason::NoPath, tick);
            return;
        };
        self.tasks.reservations.insert(
            task.target,
            Reservation {
                task: id,
                agent: task.agent,
                revision,
                expires: tick + self.tasks.params.reservation_ttl as u64,
            },
        );
        {
            let t = self.tasks.tasks.get_mut(&id).expect("live");
            t.revision = revision;
            t.approach = Some(approach);
            t.effect_sends = 0;
        }
        self.agents.intent[slot] = Intent::None;
        if approach == here {
            self.agents.path[slot].clear();
            self.begin_execution(id, slot, tick);
        } else {
            self.set_state(id, TaskState::Travelling, Reason::None, tick);
        }
    }

    /// Another agent stands on (or is walking to) this tile for a task.
    fn approach_taken(&self, slot: usize, t: Tile) -> bool {
        self.agents.ids_sorted().any(|(_, other)| {
            other != slot
                && (self.agents.path[other].goal() == Some(t)
                    || (self.agents.path[other].is_empty() && self.agents.tile_of(other) == t))
        })
    }

    /// A route that ends exactly on `to` (no snapping to a nearby tile).
    pub(crate) fn route_exact(&mut self, slot: usize, to: Tile) -> bool {
        if !self.grid().passable(to) {
            return false;
        }
        let from = self.agents.tile_of(slot);
        let mut buf = std::mem::take(&mut self.path_buf);
        let found = self.astar.find(&self.grid, from, to, &mut buf)
            && buf.last().copied().map(|g| g == to).unwrap_or(from == to);
        if found {
            self.agents.path[slot].set(&buf);
        }
        self.path_buf = buf;
        found
    }

    fn begin_execution(&mut self, id: TaskId, slot: usize, tick: u64) {
        let task = self.tasks.tasks[&id].clone();
        let p = &self.tasks.params;
        let duration = match task.action {
            TaskAction::Sit => p.sit_ticks,
            TaskAction::Work => p.work_ticks,
            TaskAction::Sleep => 0,
            TaskAction::Loiter => p.loiter_ticks,
            _ => p.farm_ticks,
        };
        {
            let t = self.tasks.tasks.get_mut(&id).expect("live");
            t.duration = duration;
            t.progress = 0;
        }
        if task.target != ObjectId(0) {
            self.face_object(slot, task.target);
        }
        let activity = match task.action {
            TaskAction::Sit => Activity::Sitting(task.target),
            TaskAction::Sleep => Activity::Sleeping,
            TaskAction::Loiter => Activity::Idle,
            // Work and the farm actions draw as work until the V2 presentation
            // carries the action itself (session 02).
            _ => Activity::Working(task.target),
        };
        self.set_activity(slot, activity);
        if task.action == TaskAction::Sleep {
            self.events
                .push(crate::snapshot::diff::WorldEvent::Slept { ent: task.agent });
        } else if task.target != ObjectId(0) {
            self.events
                .push(crate::snapshot::diff::WorldEvent::Interacted {
                    ent: task.agent,
                    object: task.target,
                });
        }
        self.set_state(id, TaskState::Executing, Reason::None, tick);
    }

    fn execute_step(&mut self, id: TaskId, slot: usize, tick: u64) {
        let (action, progress, duration) = {
            let t = self.tasks.tasks.get_mut(&id).expect("live");
            t.progress += 1;
            (t.action, t.progress, t.duration)
        };
        let energy = self.agents.energy[slot];
        let p = self.tasks.params.clone();
        let done = match action {
            TaskAction::Sleep => energy >= p.energy_rested || progress >= p.sleep_max_ticks,
            // Too tired to go on: stop working, the next choice is rest.
            TaskAction::Work => progress >= duration || energy < p.energy_sleep,
            _ => progress >= duration,
        };
        if !done {
            return;
        }
        if action.is_farm() {
            self.send_effect(id, tick);
            self.set_state(id, TaskState::AwaitingEffect, Reason::None, tick);
            // Hold the pose until the server answers.
            return;
        }
        if action == TaskAction::Sleep {
            self.events.push(crate::snapshot::diff::WorldEvent::Woke {
                ent: self.agents.id[slot],
            });
        }
        self.end_task(id, TaskState::Completed, Reason::None, tick);
    }

    fn send_effect(&mut self, id: TaskId, tick: u64) {
        let t = self.tasks.tasks.get_mut(&id).expect("live");
        t.effect_sends += 1;
        t.since = tick;
        let req = EffectRequest {
            task: t.id,
            revision: t.revision,
            agent: t.agent,
            action: t.action,
            object: t.target,
            tile: t.approach.unwrap_or_default(),
        };
        self.tasks.effects.push(req);
        self.tasks.stats.effects_sent += 1;
    }

    fn renew(&mut self, id: TaskId, tick: u64) {
        let ttl = self.tasks.params.reservation_ttl as u64;
        let Some(t) = self.tasks.tasks.get(&id) else {
            return;
        };
        let (target, rev) = (t.target, t.revision);
        if let Some(r) = self.tasks.reservations.get_mut(&target) {
            if r.task == id && r.revision == rev {
                r.expires = tick + ttl;
            }
        }
    }

    fn block(&mut self, id: TaskId, reason: Reason, tick: u64) {
        let (attempts, target) = {
            let t = self.tasks.tasks.get_mut(&id).expect("live");
            t.attempts += 1;
            (t.attempts, t.target)
        };
        if attempts > self.tasks.params.max_attempts {
            self.cool(id, target, tick);
            self.end_task(id, TaskState::Failed, reason, tick);
            return;
        }
        let wait = (self.tasks.params.retry_base as u64) << attempts.min(6);
        self.tasks.tasks.get_mut(&id).expect("live").retry_at = tick + wait;
        self.set_state(id, TaskState::Blocked, reason, tick);
    }

    fn cool(&mut self, id: TaskId, target: ObjectId, tick: u64) {
        let Some(agent) = self.tasks.tasks.get(&id).map(|t| t.agent) else {
            return;
        };
        let until = tick + self.tasks.params.target_cooldown as u64;
        if let Some(a) = self.tasks.autonomy.get_mut(&agent) {
            a.cooldown.insert(target, until);
        }
    }

    fn set_state(&mut self, id: TaskId, state: TaskState, reason: Reason, tick: u64) {
        let snapshot = {
            let t = self.tasks.tasks.get_mut(&id).expect("live");
            if t.state == state && t.reason == reason {
                return;
            }
            t.state = state;
            t.reason = reason;
            t.since = tick;
            t.clone()
        };
        self.tasks.trace(tick, &snapshot);
    }

    /// Terminal: release everything, leave the agent standing, schedule the
    /// next choice. The task leaves the book; the trace keeps its story.
    pub(crate) fn end_task(&mut self, id: TaskId, state: TaskState, reason: Reason, tick: u64) {
        let Some(mut t) = self.tasks.tasks.remove(&id) else {
            return;
        };
        t.state = state;
        t.reason = reason;
        t.since = tick;
        self.tasks.trace(tick, &t);
        match state {
            TaskState::Completed => {
                self.tasks.stats.completed += 1;
                self.tasks.stats.completed_by_action[t.action.index()] += 1;
            }
            TaskState::Failed => self.tasks.stats.failed += 1,
            _ => self.tasks.stats.cancelled += 1,
        }
        if let Some(r) = self.tasks.reservations.get(&t.target) {
            if r.task == id {
                self.tasks.reservations.remove(&t.target);
            }
        }
        if let Some(slot) = self.agents.slot(t.agent) {
            let a = self.agents.activity[slot];
            let ours = matches!(a, Activity::Sitting(o) | Activity::Working(o) if o == t.target)
                || (t.action == TaskAction::Sleep && a == Activity::Sleeping)
                || a == Activity::Walking;
            if ours {
                self.agents.path[slot].clear();
                self.set_activity(slot, Activity::Idle);
            }
        }
        let settle = self.tasks.params.settle_ticks as u64;
        if let Some(a) = self.tasks.autonomy.get_mut(&t.agent) {
            if a.current == Some(id) {
                a.current = None;
            }
            a.next_eval = a.next_eval.max(tick + settle);
        }
    }

    // --- choosing -----------------------------------------------------------

    fn select_tasks(&mut self, tick: u64) {
        let budget = self.tasks.params.eval_per_tick as usize;
        let cursor = self.tasks.cursor;
        let mut due: Vec<EntId> = self
            .tasks
            .autonomy
            .iter()
            .filter(|(e, a)| {
                a.enabled && a.current.is_none() && a.next_eval <= tick && self.agents.contains(**e)
            })
            .map(|(e, _)| *e)
            .collect();
        if due.is_empty() {
            return;
        }
        // Round robin: those after the cursor first, then wrap.
        due.sort_by_key(|e| (e.0 <= cursor, e.0));
        for ent in due.into_iter().take(budget) {
            self.tasks.cursor = ent.0;
            self.tasks.stats.evaluations += 1;
            let Some(slot) = self.agents.slot(ent) else {
                continue;
            };
            // A walk or a pose from elsewhere (a player's move, a routine) is
            // left to finish.
            if !self.agents.path[slot].is_empty() {
                continue;
            }
            let (action, target) = self.choose(ent, slot, tick);
            self.create_task(ent, action, target, TaskSource::Local, tick);
        }
    }

    /// Utility over needs and what is around, integers only. Ties go to the
    /// lower object id; the agent's own stream adds a small jitter so two
    /// identical residents do not always pick the same seat.
    fn choose(&mut self, ent: EntId, slot: usize, tick: u64) -> (TaskAction, ObjectId) {
        let p = self.tasks.params.clone();
        let energy = self.agents.energy[slot] as i32;
        let here = self.agents.tile_of(slot);
        let auto = self.tasks.autonomy.get(&ent).cloned().unwrap_or_default();
        let mut best: (i32, u32, TaskAction, ObjectId) =
            (p.score_loiter, u32::MAX, TaskAction::Loiter, ObjectId(0));
        if energy < p.energy_sleep as i32 {
            // Sleeping on the spot beats nothing; a bed beats the spot.
            best = (p.score_sleep / 2, u32::MAX, TaskAction::Sleep, ObjectId(0));
        }
        let mut cands: Vec<(i32, u32, TaskAction, ObjectId)> = Vec::new();
        for o in self.objects.all() {
            let Some(action) = affordance(&o.kind) else {
                continue;
            };
            let dist = (o.tile.x - here.x).abs().max((o.tile.y - here.y).abs());
            if dist > p.reach {
                continue;
            }
            if auto
                .cooldown
                .get(&o.id)
                .map(|&until| until > tick)
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(r) = self.tasks.reservations.get(&o.id) {
                if r.agent != ent {
                    continue;
                }
            }
            if needs_delegation(action) && !auto.delegated.contains(&o.id) {
                continue;
            }
            let base = match action {
                TaskAction::Sleep => {
                    if energy >= p.energy_sleep as i32 {
                        continue;
                    }
                    p.score_sleep + (p.energy_sleep as i32 - energy) * 4
                }
                TaskAction::Sit => {
                    if energy < p.energy_sit as i32 {
                        p.score_sit + (p.energy_sit as i32 - energy)
                    } else {
                        p.score_sit / 4
                    }
                }
                TaskAction::Work => {
                    if energy < p.energy_sleep as i32 {
                        continue;
                    }
                    p.score_work * energy / 255
                }
                TaskAction::Harvest => p.score_harvest,
                TaskAction::Plant => p.score_plant,
                TaskAction::Clear => p.score_clear,
                TaskAction::Water => {
                    if self
                        .tasks
                        .watered
                        .get(&o.id)
                        .map(|&until| until > tick)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    p.score_water
                }
                TaskAction::Loiter => continue,
            };
            cands.push((base - dist * p.distance_cost, o.id.0, action, o.id));
        }
        for (score, oid, action, target) in cands {
            let jitter = self.agents.rng[slot].below(16) as i32;
            let s = score + jitter;
            if s > best.0 || (s == best.0 && oid < best.1) {
                best = (s, oid, action, target);
            }
        }
        (best.2, best.3)
    }

    pub(crate) fn create_task(
        &mut self,
        ent: EntId,
        action: TaskAction,
        target: ObjectId,
        source: TaskSource,
        tick: u64,
    ) -> TaskId {
        self.tasks.next_id += 1;
        let id = TaskId(self.tasks.next_id);
        let t = Task {
            id,
            agent: ent,
            action,
            target,
            source,
            state: TaskState::Queued,
            revision: 0,
            attempts: 0,
            created: tick,
            since: tick,
            retry_at: 0,
            progress: 0,
            duration: 0,
            reason: Reason::None,
            approach: None,
            effect_sends: 0,
        };
        self.tasks.trace(tick, &t);
        self.tasks.tasks.insert(id, t);
        if let Some(a) = self.tasks.autonomy.get_mut(&ent) {
            a.current = Some(id);
        }
        id
    }

    // --- hooks from inputs and decisions ---------------------------------------

    /// A mailbox decision for an autonomous agent. Sit/Work/Sleep become a
    /// task (validated like a local one); the rest applies as before, and a
    /// walk or an idle order cancels the task in hand. Returns `Some(result)`
    /// when handled here.
    pub(crate) fn decision_as_task(
        &mut self,
        slot: usize,
        ent: EntId,
        d: &Decision,
    ) -> Option<crate::error::Result<()>> {
        if !self.tasks.is_autonomous(ent) {
            return None;
        }
        let tick = self.tick();
        let want = match d {
            Decision::Sit(o) => Some((TaskAction::Sit, *o)),
            Decision::Work(o) => Some((TaskAction::Work, *o)),
            Decision::Sleep => Some((
                TaskAction::Sleep,
                self.nearest_affordance(slot, TaskAction::Sleep)
                    .unwrap_or(ObjectId(0)),
            )),
            _ => None,
        };
        let Some((action, target)) = want else {
            if matches!(d, Decision::GoTo(_) | Decision::Idle) {
                if let Some(cur) = self.tasks.autonomy.get(&ent).and_then(|a| a.current) {
                    self.end_task(cur, TaskState::Cancelled, Reason::Preempted, tick);
                }
            }
            return None;
        };
        if target != ObjectId(0) {
            let ok = self
                .objects
                .get(target)
                .and_then(|o| affordance(&o.kind))
                .map(|a| compatible(a, action))
                .unwrap_or(false);
            if !ok {
                return Some(Err(crate::error::WorldError::UnknownObject(target.0)));
            }
        }
        if let Some(cur) = self.tasks.autonomy.get(&ent).and_then(|a| a.current) {
            self.end_task(cur, TaskState::Cancelled, Reason::Preempted, tick);
        }
        self.create_task(ent, action, target, TaskSource::Intent, tick);
        Some(Ok(()))
    }

    fn nearest_affordance(&self, slot: usize, want: TaskAction) -> Option<ObjectId> {
        let here = self.agents.tile_of(slot);
        self.objects
            .all()
            .iter()
            .filter(|o| affordance(&o.kind) == Some(want))
            .min_by_key(|o| (o.tile.dist2(here), o.id.0))
            .map(|o| o.id)
    }

    /// An object left the world: every task on it ends.
    pub(crate) fn tasks_on_removed(&mut self, object: ObjectId) {
        let tick = self.tick();
        let hit: Vec<TaskId> = self
            .tasks
            .tasks
            .values()
            .filter(|t| t.target == object)
            .map(|t| t.id)
            .collect();
        for id in hit {
            self.end_task(id, TaskState::Cancelled, Reason::TargetGone, tick);
        }
        self.tasks.reservations.remove(&object);
        self.tasks.watered.remove(&object);
    }

    /// A footprint got blocked: routes through it are stale. The task goes
    /// back to the queue (reservation kept) and finds another approach or
    /// gives up with `no_path`.
    pub(crate) fn tasks_on_blocked(&mut self) {
        let tick = self.tick();
        let travelling: Vec<(TaskId, EntId)> = self
            .tasks
            .tasks
            .values()
            .filter(|t| t.state == TaskState::Travelling)
            .map(|t| (t.id, t.agent))
            .collect();
        for (id, ent) in travelling {
            let Some(slot) = self.agents.slot(ent) else {
                continue;
            };
            let blocked = self.agents.path[slot]
                .remaining_tiles()
                .iter()
                .any(|t| !self.grid().passable(*t));
            if blocked {
                self.agents.path[slot].clear();
                self.set_state(id, TaskState::Queued, Reason::None, tick);
            }
        }
    }

    pub(crate) fn set_autonomy(&mut self, ent: EntId, enabled: bool) {
        let tick = self.tick();
        let cur = {
            let a = self.tasks.autonomy.entry(ent).or_default();
            a.enabled = enabled;
            if enabled {
                a.next_eval = tick;
            }
            a.current
        };
        if !enabled {
            if let Some(cur) = cur {
                self.end_task(cur, TaskState::Cancelled, Reason::Preempted, tick);
            }
        }
    }

    pub(crate) fn set_delegation(&mut self, ent: EntId, objects: &[ObjectId]) {
        let tick = self.tick();
        let now: std::collections::BTreeSet<ObjectId> = objects.iter().copied().collect();
        let a = self.tasks.autonomy.entry(ent).or_default();
        a.delegated = now.clone();
        let cur = a.current;
        // A farm task on a bed no longer delegated stops here; its effect, if
        // already sent, is fenced by the revision and by the server's check.
        if let Some(cur) = cur {
            let revoke = self
                .tasks
                .tasks
                .get(&cur)
                .map(|t| needs_delegation(t.action) && !now.contains(&t.target))
                .unwrap_or(false);
            if revoke {
                self.end_task(cur, TaskState::Cancelled, Reason::Revoked, tick);
            }
        }
    }

    pub(crate) fn cancel_current(&mut self, ent: EntId) -> bool {
        let tick = self.tick();
        match self.tasks.autonomy.get(&ent).and_then(|a| a.current) {
            Some(cur) => {
                self.end_task(cur, TaskState::Cancelled, Reason::Cancelled, tick);
                true
            }
            None => false,
        }
    }

    /// The server's answer to an effect. Only the current revision of a task
    /// still awaiting it counts.
    pub(crate) fn task_result(
        &mut self,
        task: TaskId,
        revision: u32,
        ok: bool,
        code: &str,
    ) -> bool {
        let tick = self.tick();
        let Some(t) = self.tasks.tasks.get(&task) else {
            return false;
        };
        if t.state != TaskState::AwaitingEffect || t.revision != revision {
            return false;
        }
        let (action, target) = (t.action, t.target);
        if ok {
            if action == TaskAction::Water {
                let until = tick + self.tasks.params.water_cooldown as u64;
                self.tasks.watered.insert(target, until);
            }
            self.end_task(task, TaskState::Completed, Reason::None, tick);
        } else {
            self.cool(task, target, tick);
            self.end_task(
                task,
                TaskState::Failed,
                Reason::EffectRefused(code.to_string()),
                tick,
            );
        }
        true
    }

    /// After a skipped stretch (region asleep): needs move by a bounded
    /// delta, tasks without outside effect finish, travelling tasks resume
    /// from where the agent stands (re-queued, a fresh route). Nothing is
    /// harvested or credited by the skip: `awaiting_effect` stays for the
    /// server to resend.
    pub(crate) fn settle_tasks(&mut self, skipped: u64) {
        let tick = self.tick();
        let ids: Vec<TaskId> = self.tasks.tasks.keys().copied().collect();
        for id in ids {
            let Some(t) = self.tasks.tasks.get(&id).cloned() else {
                continue;
            };
            match t.state {
                TaskState::Executing if !t.action.is_farm() => {
                    self.end_task(id, TaskState::Completed, Reason::None, tick)
                }
                TaskState::Travelling => {
                    if let Some(slot) = self.agents.slot(t.agent) {
                        self.agents.path[slot].clear();
                    }
                    self.set_state(id, TaskState::Queued, Reason::None, tick);
                }
                TaskState::Blocked => self.set_state(id, TaskState::Queued, Reason::None, tick),
                _ => {}
            }
        }
        // Rest recovered while nobody watched, capped: a night, not a week.
        let gain = ((skipped / self.tasks.params.need_period.max(1) as u64) as u32).min(64) as i32;
        let ids: Vec<EntId> = self
            .tasks
            .autonomy
            .iter()
            .filter(|(_, a)| a.enabled)
            .map(|(e, _)| *e)
            .collect();
        for ent in ids {
            if let Some(slot) = self.agents.slot(ent) {
                let e = self.agents.energy[slot] as i32 + gain;
                self.agents.energy[slot] = e.clamp(0, 255) as u8;
            }
            if let Some(a) = self.tasks.autonomy.get_mut(&ent) {
                a.next_eval = tick;
                a.cooldown.retain(|_, until| *until > tick);
            }
        }
        self.tasks.watered.retain(|_, until| *until > tick);
    }
}

/// A seat is for sitting, a bed for sleeping (and sitting), a workstation
/// for working, a bed of crops for whatever its stage asks.
fn compatible(offered: TaskAction, wanted: TaskAction) -> bool {
    offered == wanted
        || (offered == TaskAction::Sleep && wanted == TaskAction::Sit)
        || (offered.is_farm() && wanted.is_farm())
}
