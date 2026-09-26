//! Session 06: residents with tasks. The acceptance scenarios MV01–MV05 at
//! the simulation level (the server's half — persistence across processes,
//! the farm domain, delegation in the database — is in omnidisc-server's
//! `tests/world.rs`).

use omniget_world::ents::{Activity, EntId};
use omniget_world::map::{ChunkDef, MapDef, Tile, TileDef};
use omniget_world::sim::{Reason, TaskAction, TaskParams, TaskState};
use omniget_world::{Input, ObjectId, World};

const N: usize = 16 * 16;

fn map() -> MapDef {
    let t = |key: &str, walkable: bool, fp: [u8; 2]| TileDef {
        key: key.into(),
        height: 0,
        occludes: false,
        footprint: fp,
        walkable,
    };
    MapDef {
        version: 1,
        id: "tasks-test".into(),
        atlas: "x".into(),
        chunk_tiles: 16,
        palette: vec![
            t("floor/wood", true, [1, 1]),
            t("object/chair", false, [1, 1]),
            t("object/bed", false, [2, 2]),
            t("object/workbench", false, [2, 1]),
            t("prop/fence", false, [1, 1]),
            t("prop/soil-empty", true, [1, 1]),
            t("crop/carrot/ripe", true, [1, 1]),
            t("prop/chair-sculpture", false, [1, 1]),
        ],
        chunks: vec![ChunkDef {
            cx: 0,
            cy: 0,
            floor: vec![0; N],
            wall: vec![255; N],
            object: vec![],
            height: vec![0; N],
            tint: vec![],
        }],
        slots: vec![],
        objects: vec![],
        rooms: vec![],
        markers: vec![],
    }
}

fn place(id: u32, kind: &str, x: i32, y: i32) -> Input {
    Input::PlaceObject {
        object: ObjectId(id),
        kind: kind.into(),
        tile: Tile::new(x, y),
        dir: 0,
        slot: None,
    }
}
fn spawn(ent: u32, x: i32, y: i32) -> Input {
    Input::Spawn {
        ent: EntId(ent),
        name: format!("r{ent}"),
        at: Tile::new(x, y),
    }
}
fn auto(ent: u32) -> Input {
    Input::SetAutonomy {
        ent: EntId(ent),
        enabled: true,
    }
}
fn energy(ent: u32, e: u8) -> Input {
    Input::SetEnergy {
        ent: EntId(ent),
        energy: e,
    }
}

/// Fast parameters so the scenarios fit in a few thousand ticks.
fn fast() -> TaskParams {
    TaskParams {
        sit_ticks: 60,
        work_ticks: 80,
        farm_ticks: 10,
        loiter_ticks: 20,
        retry_base: 5,
        target_cooldown: 200,
        need_period: 10,
        ..TaskParams::default()
    }
}

fn world(inputs: Vec<Input>) -> World {
    let mut w = World::new(7, map()).unwrap();
    let mut first = vec![Input::SetTaskParams { params: fast() }];
    first.extend(inputs);
    w.step(&first);
    w
}

fn run(w: &mut World, ticks: u32, mut each: impl FnMut(&World)) {
    for _ in 0..ticks {
        w.step(&[]);
        each(w);
    }
}

fn activity(w: &World, ent: u32) -> Activity {
    w.snapshot()
        .agents
        .iter()
        .find(|a| a.id == EntId(ent))
        .unwrap()
        .activity
}

#[test]
fn affordances_are_exact_kinds_not_suffixes() {
    use omniget_world::sim::affordance;
    assert_eq!(affordance("object/chair"), Some(TaskAction::Sit));
    assert_eq!(affordance("prop/chair-sculpture"), None);
    assert_eq!(
        affordance("decor/chair"),
        None,
        "a chair-looking leaf in another family is decoration"
    );
    assert_eq!(affordance("crop/wheat/ripe"), Some(TaskAction::Harvest));
    assert_eq!(affordance("crop/wheat/withered"), Some(TaskAction::Clear));
    assert_eq!(affordance("crop/wheat/sprout"), Some(TaskAction::Water));
    assert_eq!(
        affordance("prop/bench"),
        Some(TaskAction::Sit),
        "the street bench is where residents rest outside"
    );
    assert_eq!(affordance("prop/flowers"), None);
}

/// MV01: two residents, one chair. Never two on it, never two reservations,
/// and both get to sit eventually (or show a valid reason while waiting).
#[test]
fn mv01_one_chair_two_residents() {
    let mut w = world(vec![
        place(1, "object/chair", 8, 8),
        spawn(1, 2, 12),
        spawn(2, 14, 12),
        energy(1, 100),
        energy(2, 100),
        auto(1),
        auto(2),
    ]);
    let mut sat = [false, false];
    let mut busy_seen = false;
    run(&mut w, 4000, |w| {
        let on_chair = [1, 2]
            .iter()
            .filter(|&&e| activity(w, e) == Activity::Sitting(ObjectId(1)))
            .count();
        assert!(on_chair <= 1, "capacity one");
        assert!(w.tasks().reservations.len() <= 1);
        for (i, e) in [1u32, 2].iter().enumerate() {
            if activity(w, *e) == Activity::Sitting(ObjectId(1)) {
                sat[i] = true;
            }
        }
        busy_seen |= w.tasks().trace.iter().any(|t| t.reason == "busy");
    });
    assert!(
        sat[0] && sat[1],
        "both residents sat at some point: {sat:?}"
    );
    // Every blocked/failed line has a code the inspector can show.
    for t in w
        .tasks()
        .trace
        .iter()
        .filter(|t| matches!(t.state, TaskState::Blocked | TaskState::Failed))
    {
        assert!(!t.reason.is_empty(), "{t:?}");
    }
    let _ = busy_seen; // contention may resolve by selection (reserved seats are skipped) or by `busy`
}

/// MV02: rest recovers energy and the resident goes back to choosing; a
/// checkpoint taken mid-walk restores to the same world, with one task.
#[test]
fn mv02_rest_recovers_and_restart_does_not_duplicate() {
    let mut w = world(vec![
        place(1, "object/bed", 10, 3),
        place(2, "object/workbench", 3, 10),
        spawn(1, 2, 13),
        energy(1, 20),
        auto(1),
    ]);
    // Walk towards the bed for a few ticks, then checkpoint.
    run(&mut w, 6, |_| {});
    let t = w.tasks().current(EntId(1)).cloned().expect("a task");
    assert_eq!(t.action, TaskAction::Sleep);
    assert_eq!(t.state, TaskState::Travelling);
    let cp = w.checkpoint();
    let json = serde_json::to_string(&cp).unwrap();
    let mut restored =
        World::from_checkpoint(map(), &serde_json::from_str(&json).unwrap()).unwrap();
    for _ in 0..400 {
        w.step(&[]);
        restored.step(&[]);
        assert_eq!(
            w.snapshot().fingerprint(),
            restored.snapshot().fingerprint(),
            "restart diverged"
        );
        assert!(
            restored
                .tasks()
                .tasks
                .values()
                .filter(|t| t.agent == EntId(1))
                .count()
                <= 1,
            "one task at a time"
        );
    }
    // Same book (the per-process counters in `stats` are not state).
    let (a, b) = (w.tasks(), restored.tasks());
    assert_eq!(
        (&a.tasks, &a.reservations, &a.autonomy, &a.trace, a.next_id),
        (&b.tasks, &b.reservations, &b.autonomy, &b.trace, b.next_id)
    );
    let slept = restored
        .tasks()
        .trace
        .iter()
        .any(|t| t.action == TaskAction::Sleep && t.state == TaskState::Completed);
    let e = restored.snapshot().agents[0].energy;
    assert!(slept, "the sleep task completed");
    assert!(
        e >= fast().energy_rested || e > 200,
        "energy recovered: {e}"
    );
    // …and after waking it chose again (work is now worth it).
    run(&mut restored, 200, |_| {});
    assert!(restored
        .tasks()
        .trace
        .iter()
        .any(|t| t.action == TaskAction::Work));
}

/// MV03: a fence cuts the way after the reservation. The resident tries the
/// other approach tiles; with none left it releases and says `no_path`,
/// gives up after the retries, and does not loop.
#[test]
fn mv03_fence_after_reservation() {
    // Workbench 2x1 at (7,7); resident far below.
    let mut w = world(vec![
        place(1, "object/workbench", 7, 7),
        spawn(1, 7, 14),
        energy(1, 250),
        auto(1),
    ]);
    run(&mut w, 3, |_| {});
    let t = w.tasks().current(EntId(1)).cloned().unwrap();
    assert_eq!(
        (t.action, t.state),
        (TaskAction::Work, TaskState::Travelling)
    );
    assert!(w.tasks().holder(ObjectId(1)).is_some());
    // Wall off the workbench completely (every approach tile).
    let mut fence = Vec::new();
    let mut id = 100;
    for x in 5..=10 {
        for y in 5..=9 {
            let inside = (7..=8).contains(&x) && y == 7;
            let ring = x == 5 || x == 10 || y == 5 || y == 9;
            if ring && !inside {
                fence.push(place(id, "prop/fence", x, y));
                id += 1;
            }
        }
    }
    w.step(&fence);
    let lines_before = w.tasks().trace.len();
    run(&mut w, 1500, |w| {
        // Never on the bench while it is unreachable.
        assert_ne!(activity(w, 1), Activity::Working(ObjectId(1)));
    });
    let trace: Vec<_> = w
        .tasks()
        .trace
        .iter()
        .filter(|t| t.target == ObjectId(1))
        .collect();
    assert!(trace.iter().any(|t| t.reason == "no_path"), "{trace:?}");
    assert!(trace
        .iter()
        .any(|t| t.state == TaskState::Failed && t.reason == "no_path"));
    assert!(
        w.tasks().holder(ObjectId(1)).is_none(),
        "reservation released"
    );
    // Bounded: retries then a cooldown, not a line per tick.
    let lines = w
        .tasks()
        .trace
        .iter()
        .skip(lines_before)
        .filter(|t| t.target == ObjectId(1))
        .count();
    assert!(lines < 40, "{lines} trace lines for the fenced bench");
}

#[test]
fn mv03b_a_blocked_side_is_walked_around() {
    let mut w = world(vec![
        place(1, "object/workbench", 7, 7),
        spawn(1, 7, 14),
        energy(1, 250),
        auto(1),
    ]);
    run(&mut w, 3, |_| {});
    // Block only the south approach tiles.
    w.step(&[
        place(100, "prop/fence", 7, 8),
        place(101, "prop/fence", 8, 8),
    ]);
    let mut worked = false;
    run(&mut w, 600, |w| {
        worked |= activity(w, 1) == Activity::Working(ObjectId(1))
    });
    assert!(worked, "reached the bench from another side");
}

fn farm_world() -> World {
    world(vec![
        place(1, "crop/carrot/ripe", 8, 8),
        spawn(1, 8, 15),
        energy(1, 250),
        auto(1),
    ])
}

/// MV04: delegation revoked while walking to the bed: no effect is asked
/// for; an answer that arrives anyway does not complete anything.
#[test]
fn mv04_revoked_delegation_stops_the_harvest() {
    let mut w = farm_world();
    run(&mut w, 30, |_| {});
    assert!(
        w.tasks()
            .trace
            .iter()
            .all(|t| t.action != TaskAction::Harvest),
        "no delegation, no harvest"
    );
    w.step(&[Input::SetDelegation {
        ent: EntId(1),
        objects: vec![ObjectId(1)],
    }]);
    // Revoke the moment it is on its way.
    let mut t = None;
    for _ in 0..60 {
        w.step(&[]);
        if let Some(c) = w.tasks().current(EntId(1)) {
            if c.state == TaskState::Travelling {
                t = Some(c.clone());
                break;
            }
        }
    }
    let t = t.expect("harvest task on its way");
    assert_eq!(t.action, TaskAction::Harvest);
    assert_eq!(t.state, TaskState::Travelling);
    w.step(&[Input::SetDelegation {
        ent: EntId(1),
        objects: vec![],
    }]);
    let last = w
        .tasks()
        .trace
        .iter()
        .rev()
        .find(|x| x.task == t.id)
        .unwrap()
        .clone();
    assert_eq!(
        (last.state, last.reason.as_str()),
        (TaskState::Cancelled, "revoked")
    );
    assert!(w.take_effects().is_empty(), "no effect requested");
    // A (forged or late) result for it changes nothing.
    w.step(&[Input::TaskResult {
        task: t.id,
        revision: t.revision,
        ok: true,
        code: String::new(),
    }]);
    assert!(!w
        .tasks()
        .trace
        .iter()
        .any(|x| x.task == t.id && x.state == TaskState::Completed));
    run(&mut w, 200, |w| {
        assert!(w
            .tasks()
            .tasks
            .values()
            .all(|x| x.action != TaskAction::Harvest))
    });
}

/// MV05: the effect is asked once per revision, answers are fenced by
/// revision, a restart while waiting resends the same revision, and an agent
/// that disappears does not keep its reservation.
#[test]
fn mv05_effects_are_fenced_and_survive_restart() {
    let mut w = farm_world();
    w.step(&[Input::SetDelegation {
        ent: EntId(1),
        objects: vec![ObjectId(1)],
    }]);
    let mut effects = Vec::new();
    run(&mut w, 200, |_| {});
    effects.extend(w.take_effects());
    // One task, one revision; a resend after the timeout repeats it exactly.
    assert!(
        !effects.is_empty() && effects.len() <= fast().effect_max_sends as usize,
        "{effects:?}"
    );
    assert!(effects
        .iter()
        .all(|x| (x.task, x.revision) == (effects[0].task, effects[0].revision)));
    let e = effects[0].clone();
    assert_eq!(e.action, TaskAction::Harvest);
    assert_eq!(
        w.tasks().task(e.task).unwrap().state,
        TaskState::AwaitingEffect
    );
    assert!(
        w.tasks().holder(ObjectId(1)).is_some(),
        "bed held while the server works"
    );
    // Restart while waiting: the book says what to resend, same revision.
    let cp = w.checkpoint();
    let mut w2 = World::from_checkpoint(
        map(),
        &serde_json::from_str(&serde_json::to_string(&cp).unwrap()).unwrap(),
    )
    .unwrap();
    let again = w2.tasks().awaiting_effects();
    assert_eq!(again.len(), 1);
    assert_eq!((again[0].task, again[0].revision), (e.task, e.revision));
    // Wrong revision: ignored. Right one: completes, once.
    w2.step(&[Input::TaskResult {
        task: e.task,
        revision: e.revision + 1,
        ok: true,
        code: String::new(),
    }]);
    assert_eq!(
        w2.tasks().task(e.task).unwrap().state,
        TaskState::AwaitingEffect
    );
    w2.step(&[Input::TaskResult {
        task: e.task,
        revision: e.revision,
        ok: true,
        code: String::new(),
    }]);
    assert!(w2.tasks().task(e.task).is_none());
    let done = w2
        .tasks()
        .trace
        .iter()
        .filter(|t| t.task == e.task && t.state == TaskState::Completed)
        .count();
    w2.step(&[Input::TaskResult {
        task: e.task,
        revision: e.revision,
        ok: true,
        code: String::new(),
    }]);
    let done_after = w2
        .tasks()
        .trace
        .iter()
        .filter(|t| t.task == e.task && t.state == TaskState::Completed)
        .count();
    assert_eq!(
        (done, done_after),
        (1, 1),
        "a duplicate answer completes nothing"
    );
    assert!(w2.tasks().holder(ObjectId(1)).is_none());
}

#[test]
fn mv05b_refused_effect_fails_with_the_code_and_cools_the_target() {
    let mut w = farm_world();
    w.step(&[Input::SetDelegation {
        ent: EntId(1),
        objects: vec![ObjectId(1)],
    }]);
    run(&mut w, 200, |_| {});
    let e = w.take_effects().pop().unwrap();
    w.step(&[Input::TaskResult {
        task: e.task,
        revision: e.revision,
        ok: false,
        code: "ERR_WORLD_FARM_NOT_RIPE".into(),
    }]);
    let last = w
        .tasks()
        .trace
        .iter()
        .rev()
        .find(|t| t.task == e.task)
        .unwrap();
    assert_eq!(last.reason, "effect_refused:ERR_WORLD_FARM_NOT_RIPE");
    run(&mut w, 100, |w| {
        assert!(
            w.tasks().tasks.values().all(|t| t.target != ObjectId(1)),
            "cooled down"
        )
    });
}

#[test]
fn mv05c_timeout_resends_then_gives_up() {
    let mut w = farm_world();
    w.step(&[Input::SetDelegation {
        ent: EntId(1),
        objects: vec![ObjectId(1)],
    }]);
    run(&mut w, 1000, |_| {});
    let effects = w.take_effects();
    let first = effects[0].task;
    let sent = effects.iter().filter(|e| e.task == first).count();
    assert_eq!(
        sent,
        fast().effect_max_sends as usize,
        "resent up to the limit, never more"
    );
    assert!(w
        .tasks()
        .trace
        .iter()
        .any(|t| t.task == first && t.reason == "effect_timeout"));
    // The timed-out task let go; a later attempt may hold the bed, not it.
    assert!(w
        .tasks()
        .holder(ObjectId(1))
        .map(|r| r.task != first)
        .unwrap_or(true));
}

#[test]
fn a_despawned_resident_frees_what_it_held() {
    let mut w = world(vec![
        place(1, "object/workbench", 7, 3),
        spawn(1, 7, 14),
        energy(1, 250),
        auto(1),
    ]);
    run(&mut w, 3, |_| {});
    assert!(w.tasks().holder(ObjectId(1)).is_some());
    w.step(&[Input::Despawn { ent: EntId(1) }]);
    w.step(&[]);
    assert!(w.tasks().holder(ObjectId(1)).is_none());
    assert!(w.tasks().tasks.is_empty());
}

#[test]
fn removing_the_target_cancels_with_target_gone() {
    let mut w = world(vec![
        place(1, "object/workbench", 7, 3),
        spawn(1, 7, 14),
        energy(1, 250),
        auto(1),
    ]);
    run(&mut w, 3, |_| {});
    w.step(&[Input::RemoveObject {
        object: ObjectId(1),
    }]);
    assert!(w.tasks().trace.iter().any(|t| t.reason == "target_gone"));
    assert!(w.tasks().reservations.is_empty());
}

/// The evaluation budget holds with many residents, and the same inputs give
/// the same books on two worlds (replay).
#[test]
fn budget_and_determinism_with_forty_residents() {
    let mut setup = vec![
        place(1, "object/chair", 3, 3),
        place(2, "object/chair", 12, 3),
        place(3, "object/workbench", 7, 12),
        place(4, "object/bed", 2, 10),
    ];
    for i in 0..40u32 {
        setup.push(spawn(10 + i, (i % 14) as i32 + 1, (i / 14) as i32 + 5));
        setup.push(energy(10 + i, (40 + i * 5) as u8));
        setup.push(auto(10 + i));
    }
    let mut a = world(setup.clone());
    let mut b = world(setup);
    let mut last = a.tasks().stats.evaluations;
    for _ in 0..1500 {
        a.step(&[]);
        b.step(&[]);
        let now = a.tasks().stats.evaluations;
        assert!(
            now - last <= fast().eval_per_tick as u64,
            "evaluation budget"
        );
        last = now;
        let seats = [1u32, 2].map(|o| {
            a.snapshot()
                .agents
                .iter()
                .filter(|x| x.activity == Activity::Sitting(ObjectId(o)))
                .count()
        });
        assert!(seats.iter().all(|&n| n <= 1), "{seats:?}");
    }
    assert_eq!(a.snapshot().fingerprint(), b.snapshot().fingerprint());
    assert_eq!(a.tasks(), b.tasks());
    assert!(
        a.tasks().stats.completed > 20,
        "work got done: {:?}",
        a.tasks().stats
    );
}

#[test]
fn a_model_intent_becomes_a_validated_task() {
    use omniget_world::sim::Decision;
    let mut w = world(vec![
        place(1, "object/chair", 8, 8),
        place(2, "prop/chair-sculpture", 4, 4),
        spawn(1, 8, 12),
        energy(1, 250),
        auto(1),
    ]);
    // An intent naming decoration is refused; one naming the chair is a task.
    let r = w.step(&[Input::Decision {
        ent: EntId(1),
        decision: Decision::Sit(ObjectId(2)),
    }]);
    assert!(
        r.rejected >= 1
            || w.last_events()
                .iter()
                .any(|e| matches!(e, omniget_world::WorldEvent::Rejected { .. }))
    );
    w.step(&[Input::Decision {
        ent: EntId(1),
        decision: Decision::Sit(ObjectId(1)),
    }]);
    let t = w.tasks().current(EntId(1)).cloned().unwrap();
    assert_eq!((t.action, t.target), (TaskAction::Sit, ObjectId(1)));
    assert!(matches!(t.source, omniget_world::sim::TaskSource::Intent));
    let _ = Reason::None;
}
