use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::eval::*;
use super::*;
use crate::core::assist::db::AssistDb;

const BOT: &str = "curador";
const SKILL: &str = "curadoria-filmes-leitura";

fn db() -> Arc<AssistDb> {
    Arc::new(AssistDb::open_in_memory().unwrap())
}

fn feedback(db: &AssistDb, task: &str, text: &str) -> String {
    observe(
        db,
        NewObservation {
            bot_id: BOT.into(),
            skill: SKILL.into(),
            task_ref: task.into(),
            source: Some(Source::ExplicitFeedback),
            kind: Some("procedure".into()),
            text: text.into(),
            polarity: 1,
            outcome: Some("verified_pass".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap()
    .id
}

fn new_cand(db: &AssistDb, obs: Vec<String>, overlay: &str) -> Result<Candidate, String> {
    propose(
        db,
        NewCandidate {
            bot_id: BOT.into(),
            skill: SKILL.into(),
            title: "light option when tired".into(),
            overlay: overlay.into(),
            reason: "reader said twice they were tired".into(),
            observations: obs,
            ..Default::default()
        },
    )
}

fn case(db: &AssistDb, split: &str, name: &str, checks: Vec<Check>) -> EvalCase {
    add_case(
        db,
        NewCase {
            bot_id: BOT.into(),
            skill: SKILL.into(),
            split: split.into(),
            name: name.into(),
            input: format!("input of {name}"),
            checks,
        },
    )
    .unwrap()
}

fn contains(t: &str) -> Check {
    Check::Contains { text: t.into() }
}

#[test]
fn a14_model_statements_duplicates_and_one_task_repeated_do_not_support_a_candidate() {
    let db = db();
    let a = feedback(&db, "round-1", "prefer one light film when I'm tired");
    // Same feedback repeated on the same round: stored once as duplicate.
    let dup = feedback(&db, "round-1", "prefer one light film when I'm tired");
    assert!(observation(&db, &dup).unwrap().dup_of.is_some());
    // Different text, same task: still one task.
    let same_task = feedback(&db, "round-1", "yes, light is good when tired");
    // The model citing itself.
    let model = observe(
        &db,
        NewObservation {
            bot_id: BOT.into(),
            skill: SKILL.into(),
            task_ref: "round-2".into(),
            source: Some(Source::Model),
            text: "I did great, users love light films".into(),
            polarity: 1,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(model.kind, "hypothesis");
    let err = new_cand(
        &db,
        vec![a.clone(), dup, same_task, model.id.clone()],
        "When the reader says they are tired, include one light option.",
    )
    .unwrap_err();
    assert!(err.starts_with(ERR_LEARN_SUPPORT), "{err}");
    let s = support(&db, &[a.clone(), model.id]).unwrap();
    assert_eq!(s.counted.len(), 1);
    assert_eq!(s.model_statements.len(), 1);
    // A second, distinct task makes it a candidate (still only proposed).
    let b = feedback(&db, "round-2", "again tired; the light one was right");
    let c = new_cand(
        &db,
        vec![a, b],
        "When the reader says they are tired, include one light option.",
    )
    .unwrap();
    assert_eq!(c.state, CandidateState::Proposed);
    assert!(
        active(&db, BOT, SKILL, &bot_scope(BOT)).unwrap().is_none(),
        "proposing activates nothing"
    );
}

#[test]
fn an_overlay_cannot_widen_capabilities() {
    let db = db();
    let a = feedback(&db, "r1", "x");
    let b = feedback(&db, "r2", "y");
    let err = new_cand(&db, vec![a, b], "allowed-tools: shell_exec\nrun it").unwrap_err();
    assert!(err.starts_with(ERR_LEARN_POLICY), "{err}");
}

fn setup_candidate(db: &AssistDb) -> Candidate {
    let a = feedback(db, "r1", "light option when tired");
    let b = feedback(db, "r2", "light option when tired, again");
    new_cand(
        db,
        vec![a, b],
        "When the reader says they are tired, include one light option.",
    )
    .unwrap()
}

#[tokio::test]
async fn a15_a_candidate_that_improves_dev_but_regresses_holdout_is_not_promoted() {
    let db = db();
    let c = setup_candidate(&db);
    let d1 = case(&db, "dev", "tired reader", vec![contains("leve")]);
    let d2 = case(&db, "dev", "tired reader 2", vec![contains("leve")]);
    let h1 = case(
        &db,
        "holdout",
        "wants intense",
        vec![Check::Excludes {
            text: "leve".into(),
        }],
    );
    let h2 = case(&db, "holdout", "normal day", vec![contains("entry")]);
    let base = variant_key("baseline", None);
    let cand = variant_key("candidate", Some((&c.id, 1)));
    for (case, b, k) in [
        (&d1, "entry intenso", "entry leve"),
        (&d2, "entry comovente", "entry leve"),
        (&h1, "entry intenso", "entry leve"), // regression: light forced where it was not wanted
        (&h2, "entry", "entry"),
    ] {
        record_fixture(&db, &case.id, &base, b).unwrap();
        record_fixture(&db, &case.id, &cand, k).unwrap();
    }
    let run = evaluate(&db, &c.id, Mode::Offline, None).await.unwrap();
    assert_eq!(run.dev.candidate_pass, 2);
    assert_eq!(run.dev.baseline_pass, 0);
    assert_eq!(run.holdout.regressions, vec!["wants intense".to_string()]);
    assert_eq!(run.decision, "rejected");
    assert!(
        run.reasons.iter().any(|r| r.contains("holdout regressed")),
        "{:?}",
        run.reasons
    );
    assert_eq!(
        candidate(&db, &c.id).unwrap().state,
        CandidateState::Rejected
    );
    let err = promote(&db, &c.id, &run.id, "user").unwrap_err();
    assert!(err.starts_with(ERR_LEARN_POLICY), "{err}");
    assert!(active(&db, BOT, SKILL, &bot_scope(BOT)).unwrap().is_none());
}

async fn eligible(db: &AssistDb, c: &Candidate) -> EvalRun {
    let cs = [
        case(
            db,
            "dev",
            format!("d1-{}", c.id).as_str(),
            vec![contains("leve")],
        ),
        case(
            db,
            "dev",
            format!("d2-{}", c.id).as_str(),
            vec![contains("leve")],
        ),
        case(
            db,
            "holdout",
            format!("h1-{}", c.id).as_str(),
            vec![contains("entry")],
        ),
        case(
            db,
            "holdout",
            format!("h2-{}", c.id).as_str(),
            vec![contains("entry")],
        ),
    ];
    let base = super::active(db, BOT, SKILL, &bot_scope(BOT)).unwrap();
    let base_key = variant_key(
        "baseline",
        base.as_ref().map(|b| (b.candidate_id.as_str(), b.version)),
    );
    let cand = variant_key("candidate", Some((&c.id, c.current_version)));
    for x in super::eval::cases(db, BOT, SKILL).unwrap() {
        record_fixture(db, &x.id, &base_key, "entry intenso").unwrap();
        record_fixture(db, &x.id, &cand, "entry leve").unwrap();
    }
    let _ = cs;
    evaluate(db, &c.id, Mode::Offline, None).await.unwrap()
}

#[tokio::test]
async fn a16_rollback_restores_the_previous_overlay_and_a_pinned_run_keeps_its_version() {
    let db = db();
    let c1 = setup_candidate(&db);
    let r1 = eligible(&db, &c1).await;
    assert_eq!(r1.decision, "eligible", "{:?}", r1.reasons);
    promote(&db, &c1.id, &r1.id, "user").unwrap();
    // Second candidate supersedes the first.
    let a = feedback(&db, "r3", "and shorter justifications");
    let b = feedback(&db, "r4", "shorter justifications please");
    let c2 = new_cand(&db, vec![a, b], "When the reader says they are tired, include one light option.\nKeep each justification to one sentence.").unwrap();
    let r2 = eligible(&db, &c2).await;
    assert_eq!(r2.decision, "eligible", "{:?}", r2.reasons);
    promote(&db, &c2.id, &r2.id, "user").unwrap();
    assert_eq!(
        active(&db, BOT, SKILL, &bot_scope(BOT))
            .unwrap()
            .unwrap()
            .candidate_id,
        c2.id
    );
    // A run starts and pins what is active (c2).
    let pinned = pin(&db, "mission-conv-1", BOT).unwrap();
    assert_eq!(pinned[0].candidate_id, c2.id);
    // The user rolls back while the run goes on.
    let p = rollback(&db, BOT, SKILL, &bot_scope(BOT), "user", "too terse").unwrap();
    assert_eq!(p.action, "rollback");
    assert_eq!(
        active(&db, BOT, SKILL, &bot_scope(BOT))
            .unwrap()
            .unwrap()
            .candidate_id,
        c1.id,
        "previous overlay restored"
    );
    assert_eq!(
        overlays_for(&db, Some("mission-conv-1"), BOT).unwrap()[0].candidate_id,
        c2.id,
        "the running mission keeps its pinned version"
    );
    assert_eq!(
        overlays_for(&db, Some("new-conv"), BOT).unwrap()[0].candidate_id,
        c1.id,
        "the next runs use the restored one"
    );
    unpin("mission-conv-1");
    assert_eq!(
        overlays_for(&db, Some("mission-conv-1"), BOT).unwrap()[0].candidate_id,
        c1.id
    );
    // History is kept.
    assert_eq!(promotions(&db, BOT).unwrap().len(), 3);
    assert_eq!(
        candidate(&db, &c2.id).unwrap().state,
        CandidateState::Reverted
    );
    assert!(overlay_text(&overlays_for(&db, None, BOT).unwrap())
        .unwrap()
        .contains("cannot grant tools"));
}

struct CountingRunner(AtomicUsize);

#[async_trait::async_trait]
impl OutputRunner for CountingRunner {
    async fn output(
        &self,
        _case: &EvalCase,
        _overlays: &[ActiveOverlay],
    ) -> Result<String, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("entry leve".into())
    }
    fn label(&self) -> String {
        "counting".into()
    }
}

/// A stub model that works like the app's live runner: it pins the variant's
/// overlays on a fresh conversation, builds the system context the way a turn
/// does (the learning augment reads `overlays_for`), and records it.
struct RecordingRunner {
    db: Arc<AssistDb>,
    prompts: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl OutputRunner for RecordingRunner {
    async fn output(&self, case: &EvalCase, overlays: &[ActiveOverlay]) -> Result<String, String> {
        let conv = format!("learn-eval-{}", crate::core::assist::new_id());
        let _pin = pin_exact(&conv, overlays.to_vec());
        let system = overlay_text(&overlays_for(&self.db, Some(&conv), BOT)?).unwrap_or_default();
        self.prompts
            .lock()
            .unwrap()
            .push(format!("{system}\n---\n{}", case.input));
        Ok("entry leve".into())
    }
    fn label(&self) -> String {
        "recording".into()
    }
}

#[tokio::test]
async fn live_evaluation_measures_the_candidate_in_place_of_the_active_overlay() {
    // Live demo 2026-09-25: the candidate variant carried the active overlay
    // in the system prompt plus the candidate in the user message.
    let db = db();
    let c1 = setup_candidate(&db);
    let r1 = eligible(&db, &c1).await;
    promote(&db, &c1.id, &r1.id, "user").unwrap();
    let a = feedback(&db, "r7", "name the director");
    let b = feedback(&db, "r8", "always the director");
    let c2 = new_cand(&db, vec![a, b], "Always name the director of each film.").unwrap();
    let runner = RecordingRunner {
        db: db.clone(),
        prompts: Default::default(),
    };
    let run = evaluate(&db, &c2.id, Mode::Live, Some(&runner))
        .await
        .unwrap();
    assert_eq!(run.baseline, variant_key("baseline", Some((&c1.id, 1))));
    let prompts = runner.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2 * run.results.len());
    // Calls alternate baseline, candidate per case.
    for pair in prompts.chunks(2) {
        let (base, cand) = (&pair[0], &pair[1]);
        assert!(
            base.contains("include one light option") && !base.contains("director"),
            "baseline = active only: {base}"
        );
        assert!(
            cand.contains("Always name the director") && !cand.contains("include one light option"),
            "candidate replaces the active one: {cand}"
        );
        assert!(
            !cand.contains("[Local procedure notes]\n"),
            "no overlay stacked in the user message: {cand}"
        );
    }
    // Nothing stays pinned: a new turn gets what is active.
    assert_eq!(
        overlays_for(&db, Some("any-conv"), BOT).unwrap()[0].candidate_id,
        c1.id
    );
}

/// Promotes the candidate (with an earlier eligible run) the first time it
/// is called: a promotion that lands while a live evaluation is running.
struct PromotingRunner {
    db: Arc<AssistDb>,
    cid: String,
    run_id: String,
    done: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl OutputRunner for PromotingRunner {
    async fn output(
        &self,
        _case: &EvalCase,
        _overlays: &[ActiveOverlay],
    ) -> Result<String, String> {
        if !self.done.swap(true, Ordering::SeqCst) {
            promote(&self.db, &self.cid, &self.run_id, "user").unwrap();
        }
        Ok("entry leve".into())
    }
    fn label(&self) -> String {
        "promoting".into()
    }
}

#[tokio::test]
async fn a_late_evaluation_never_downgrades_a_promoted_candidate() {
    // Live demo 2026-09-25: two evaluations of C; C promoted after the
    // first; the second one ended and put C back to `eligible`.
    let db = db();
    let c = setup_candidate(&db);
    let first = eligible(&db, &c).await;
    assert_eq!(
        candidate(&db, &c.id).unwrap().state,
        CandidateState::Eligible
    );
    let runner = PromotingRunner {
        db: db.clone(),
        cid: c.id.clone(),
        run_id: first.id.clone(),
        done: Default::default(),
    };
    let late = evaluate(&db, &c.id, Mode::Live, Some(&runner))
        .await
        .unwrap();
    // Same output on both sides: `rejected`, which used to overwrite `active`.
    assert_eq!(late.decision, "rejected");
    assert_eq!(
        candidate(&db, &c.id).unwrap().state,
        CandidateState::Active,
        "the promotion stands"
    );
    assert_eq!(
        active(&db, BOT, SKILL, &bot_scope(BOT))
            .unwrap()
            .unwrap()
            .candidate_id,
        c.id
    );
    assert_eq!(late.meta["lifecycle"], "stale");
    assert!(
        runs_of(&db, &c.id)
            .unwrap()
            .iter()
            .any(|r| r.id == late.id && r.meta["lifecycle"] == "stale"),
        "the stale run is still recorded"
    );
    // Re-evaluating an active candidate keeps it active.
    let again = evaluate(&db, &c.id, Mode::Offline, None).await.unwrap();
    assert_eq!(again.meta["lifecycle"], "stale");
    assert_eq!(candidate(&db, &c.id).unwrap().state, CandidateState::Active);
    // A revision during an evaluation also makes it stale.
    let a = feedback(&db, "r11", "x");
    let b = feedback(&db, "r12", "y");
    let c3 = new_cand(&db, vec![a.clone(), b.clone()], "Say the running time.").unwrap();
    struct RevisingRunner(
        Arc<AssistDb>,
        String,
        Vec<String>,
        std::sync::atomic::AtomicBool,
    );
    #[async_trait::async_trait]
    impl OutputRunner for RevisingRunner {
        async fn output(&self, _case: &EvalCase, _o: &[ActiveOverlay]) -> Result<String, String> {
            if !self.3.swap(true, Ordering::SeqCst) {
                revise(
                    &self.0,
                    &self.1,
                    "Say the running time in minutes.",
                    "clearer",
                    &self.2,
                )
                .unwrap();
            }
            Ok("entry leve".into())
        }
        fn label(&self) -> String {
            "revising".into()
        }
    }
    let r = RevisingRunner(db.clone(), c3.id.clone(), vec![a, b], Default::default());
    let run = evaluate(&db, &c3.id, Mode::Live, Some(&r)).await.unwrap();
    assert_eq!(run.meta["lifecycle"], "stale");
    assert_ne!(
        candidate(&db, &c3.id).unwrap().state,
        CandidateState::Eligible,
        "v1's result is not v2's"
    );
}

#[tokio::test]
async fn a17_offline_replay_without_fixture_calls_nothing_and_executables_are_unsupported() {
    let db = db();
    let c = setup_candidate(&db);
    case(&db, "dev", "d1", vec![contains("leve")]);
    case(&db, "dev", "d2", vec![contains("leve")]);
    case(&db, "holdout", "h1", vec![contains("entry")]);
    case(&db, "holdout", "h2", vec![contains("entry")]);
    let runner = CountingRunner(AtomicUsize::new(0));
    let run = evaluate(&db, &c.id, Mode::Offline, Some(&runner))
        .await
        .unwrap();
    assert_eq!(
        runner.0.load(Ordering::SeqCst),
        0,
        "offline never calls the runner"
    );
    assert_eq!(run.decision, "insufficient");
    assert!(run.results.iter().all(
        |r| r.candidate == "unknown" && r.candidate_notes[0].starts_with(ERR_LEARN_NO_FIXTURE)
    ));
    // An executable candidate is not run at all.
    let a = feedback(&db, "r9", "a");
    let b = feedback(&db, "r10", "b");
    let exe = propose(
        &db,
        NewCandidate {
            bot_id: BOT.into(),
            skill: SKILL.into(),
            title: "script".into(),
            overlay: "#!/bin/sh\necho hi".into(),
            observations: vec![a, b],
            kind: Some("executable".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let run = evaluate(&db, &exe.id, Mode::Live, Some(&runner))
        .await
        .unwrap();
    assert_eq!(run.decision, "unsupported");
    assert_eq!(runner.0.load(Ordering::SeqCst), 0);
    assert!(promote(&db, &exe.id, &run.id, "user")
        .unwrap_err()
        .starts_with(ERR_LEARN_UNSUPPORTED));
    // Live mode records fixtures so the next offline replay is possible.
    let run = evaluate(&db, &c.id, Mode::Live, Some(&runner))
        .await
        .unwrap();
    assert_eq!(runner.0.load(Ordering::SeqCst), 8);
    let replay = evaluate(&db, &c.id, Mode::Offline, None).await.unwrap();
    assert_eq!(
        replay
            .results
            .iter()
            .filter(|r| r.candidate == "unknown")
            .count(),
        0
    );
    let _ = run;
}

#[tokio::test]
async fn a13_revoking_the_source_invalidates_the_candidate_and_rolls_back_an_active_one() {
    let db = db();
    let a = observe(
        &db,
        NewObservation {
            bot_id: BOT.into(),
            skill: SKILL.into(),
            task_ref: "r1".into(),
            source: Some(Source::ExplicitFeedback),
            text: "light when tired".into(),
            polarity: 1,
            evidence_ref: Some("memory:mem-1".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    let b = feedback(&db, "r2", "light when tired again");
    let c = new_cand(
        &db,
        vec![a.id.clone(), b],
        "include one light option when tired",
    )
    .unwrap();
    let run = eligible(&db, &c).await;
    promote(&db, &c.id, &run.id, "user").unwrap();
    assert!(active(&db, BOT, SKILL, &bot_scope(BOT)).unwrap().is_some());
    // The person deletes the memory record the observation came from.
    let touched = on_memory_changed(&db, "mem-1", "memory forgotten by the user").unwrap();
    assert_eq!(touched, vec![c.id.clone()]);
    assert_eq!(
        candidate(&db, &c.id).unwrap().state,
        CandidateState::Rejected
    );
    assert!(
        active(&db, BOT, SKILL, &bot_scope(BOT)).unwrap().is_none(),
        "active overlay rolled back"
    );
    assert_eq!(
        observation(&db, &a.id).unwrap().text,
        "[revoked]",
        "the revoked text is gone"
    );
}

#[tokio::test]
async fn automatic_promotion_is_opt_in_and_only_for_local_overlays() {
    let db = db();
    let c = setup_candidate(&db);
    let run = eligible(&db, &c).await;
    assert_eq!(
        candidate(&db, &c.id).unwrap().state,
        CandidateState::Eligible,
        "default: nothing promotes itself"
    );
    assert!(promote(&db, &c.id, &run.id, "auto").is_err());
    let mut s = settings(&db, BOT).unwrap();
    s.auto_promote_local = true;
    set_settings(&db, &s).unwrap();
    let a = feedback(&db, "r5", "p");
    let b = feedback(&db, "r6", "q");
    let c2 = new_cand(
        &db,
        vec![a, b],
        "include one light option when tired; say why in one line",
    )
    .unwrap();
    eligible(&db, &c2).await;
    assert_eq!(
        candidate(&db, &c2.id).unwrap().state,
        CandidateState::Active,
        "opt-in local overlay promoted by its eligible evaluation"
    );
}

#[test]
fn turning_learning_off_stops_observations_but_not_memory() {
    let db = db();
    set_settings(
        &db,
        &Settings {
            bot_id: BOT.into(),
            enabled: false,
            auto_promote_local: false,
            policy: PromotionPolicy::default(),
        },
    )
    .unwrap();
    assert!(observe(
        &db,
        NewObservation {
            bot_id: BOT.into(),
            text: "x".into(),
            source: Some(Source::ExplicitFeedback),
            ..Default::default()
        }
    )
    .unwrap()
    .is_none());
    let ctx = crate::core::assist::ctx::direct(BOT, Some("c1"));
    use crate::core::assist::memory::{remember, Category, NewMemory, Source as MemSource};
    let now = crate::core::assist::now_ms();
    let w = remember(
        &db,
        &ctx,
        NewMemory::new(
            crate::core::assist::ctx::Scope::User,
            Category::Declared,
            "depth without solemnity",
            MemSource::user(now),
        )
        .subject("tone"),
        now,
    );
    assert!(w.is_ok(), "explicit memory still works: {w:?}");
}

#[tokio::test]
async fn a_changed_case_set_needs_a_new_evaluation() {
    let db = db();
    let c = setup_candidate(&db);
    let run = eligible(&db, &c).await;
    case(&db, "holdout", "new case", vec![contains("entry")]);
    assert!(promote(&db, &c.id, &run.id, "user")
        .unwrap_err()
        .contains("case set changed"));
}

fn obs_for(db: &AssistDb, bot: &str, task: &str, text: &str) -> String {
    observe(
        db,
        NewObservation {
            bot_id: bot.into(),
            skill: SKILL.into(),
            task_ref: task.into(),
            source: Some(Source::ExplicitFeedback),
            kind: Some("procedure".into()),
            text: text.into(),
            polarity: 1,
            outcome: Some("verified_pass".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap()
    .id
}

/// Audit F-L2 (reproduction turned regression): revoking an observation
/// withdraws the overlay text derived from it even when support remains,
/// drops it from the active overlays and from pinned runs, and keeps no hash
/// or free-text reason of the revoked text.
#[tokio::test]
async fn f_l2_revoking_withdraws_derived_text_even_with_support_left() {
    let db = db();
    let a = feedback(
        &db,
        "t1",
        "Maria my daughter has epilepsy, never recommend strobe films",
    );
    // Only `a` names the person; b and c stay and keep the support at 2.
    let b = feedback(&db, "t2", "avoid strobe effects");
    let c = feedback(&db, "t3", "no strobe films please");
    let personal = "Maria (the user's daughter) has epilepsy: never recommend strobe films.";
    let cand = new_cand(&db, vec![a.clone(), b, c], personal).unwrap();
    let run = eligible(&db, &cand).await;
    promote(&db, &cand.id, &run.id, "user").unwrap();
    let conv = format!("pinned-{}", cand.id);
    assert_eq!(pin(&db, &conv, BOT).unwrap()[0].overlay, personal);
    let touched = revoke_observation(&db, &a, "user deleted: Maria epilepsy").unwrap();
    assert_eq!(touched, vec![cand.id.clone()]);
    let v = version(&db, &cand.id, 1).unwrap();
    assert_eq!(v.overlay, WITHDRAWN_OVERLAY);
    assert!(v.reason.is_empty() && v.diff.is_empty());
    let k = candidate(&db, &cand.id).unwrap();
    assert_eq!(k.state, CandidateState::Rejected);
    assert!(k.reason.unwrap().contains("needs review"));
    assert!(
        active(&db, BOT, SKILL, &bot_scope(BOT)).unwrap().is_none(),
        "dropped from the active overlays"
    );
    assert!(
        overlays_for(&db, Some(&conv), BOT).unwrap().is_empty(),
        "dropped from pinned runs"
    );
    unpin(&conv);
    let row: (String, String, Option<String>) = db
        .with(|c| {
            c.query_row(
                "SELECT text, content_hash, revoked_reason FROM learn_observations WHERE id=?1",
                [&a],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .unwrap();
    assert_eq!(
        row,
        (
            "[revoked]".to_string(),
            "revoked".to_string(),
            Some(REVOKED_LABEL.to_string())
        )
    );
    // Fixtures produced with the withdrawn text are gone.
    let n: i64 = db
        .with(|c| {
            c.query_row(
                "SELECT count(*) FROM learn_fixtures WHERE variant = ?1",
                [variant_key("candidate", Some((&cand.id, 1)))],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(n, 0);
    let everything = serde_json::to_string(&export(&db, BOT).unwrap()).unwrap();
    assert!(
        !everything.contains("Maria"),
        "no trace of the revoked text: {everything}"
    );
    // It comes back only through a revision with new text.
    let again = revise(
        &db,
        &cand.id,
        "Never recommend films with strobe effects.",
        "re-proposed without the personal detail",
        &[],
    )
    .unwrap();
    assert_eq!(again.state, CandidateState::Proposed);
}

/// Audit F-L1/F-L3/F-L4 (reproduction turned regression): `forget` deletes
/// evaluation cases, fixtures and runs, clears pinned overlays, and `export`
/// is complete (no silent caps) and includes evaluation data.
#[tokio::test]
async fn f_l1_forget_deletes_eval_data_and_pins_and_export_is_complete() {
    let db = db();
    let bot = "forget-bot";
    for i in 0..1005 {
        obs_for(&db, bot, &format!("bulk-{i}"), &format!("note {i}"));
    }
    let a = obs_for(&db, bot, "t1", "Maria my daughter has epilepsy");
    let b = obs_for(&db, bot, "t2", "Maria has epilepsy, avoid strobe");
    let cand = propose(
        &db,
        NewCandidate {
            bot_id: bot.into(),
            skill: SKILL.into(),
            title: "no strobe".into(),
            overlay: "Maria has epilepsy: no strobe.".into(),
            reason: "Maria".into(),
            observations: vec![a, b],
            ..Default::default()
        },
    )
    .unwrap();
    let case = add_case(
        &db,
        NewCase {
            bot_id: bot.into(),
            skill: SKILL.into(),
            split: "dev".into(),
            name: "maria".into(),
            input: "Recommend a film for Maria, who has epilepsy".into(),
            checks: vec![contains("film")],
        },
    )
    .unwrap();
    record_fixture(
        &db,
        &case.id,
        "baseline:none",
        "For Maria (epileptic) I suggest ...",
    )
    .unwrap();
    record_fixture(
        &db,
        &case.id,
        &variant_key("candidate", Some((&cand.id, 1))),
        "For Maria: no strobe ...",
    )
    .unwrap();
    evaluate(&db, &cand.id, Mode::Offline, None).await.unwrap();
    let ex = export(&db, bot).unwrap();
    assert_eq!(ex["complete"], true);
    assert_eq!(ex["counts"]["observations"], 1007, "no 1000 cap");
    assert_eq!(ex["observations"].as_array().unwrap().len(), 1007);
    assert_eq!(ex["counts"]["eval_cases"], 1);
    assert_eq!(ex["counts"]["fixtures"], 2);
    assert_eq!(ex["counts"]["eval_runs"], 1);
    // A pinned run of this bot (simulated: a pin taken while an overlay was active).
    let conv = format!("forget-conv-{}", cand.id);
    PINS.write()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            conv.clone(),
            vec![ActiveOverlay {
                bot_id: bot.into(),
                skill: SKILL.into(),
                scope: bot_scope(bot),
                candidate_id: cand.id.clone(),
                version: 1,
                overlay: "Maria has epilepsy: no strobe.".into(),
                since_ms: 0,
            }],
        );
    forget(&db, bot).unwrap();
    for t in [
        "learn_observations",
        "learn_candidates",
        "learn_candidate_versions",
        "learn_eval_cases",
        "learn_fixtures",
        "learn_eval_runs",
        "learn_active",
    ] {
        let n: i64 = db
            .with(|c| c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 0, "{t} still has rows after forget");
    }
    assert!(
        overlays_for(&db, Some(&conv), bot).unwrap().is_empty(),
        "pinned overlay no longer injected"
    );
    unpin(&conv);
}
