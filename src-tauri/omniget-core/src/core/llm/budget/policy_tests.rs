//! Unit tests of the budget policies (3.3). The end-to-end ones (a real
//! Coordinator, the CLI runtime and `fake_claude.py`) are in `bn7_tests.rs`.

use super::*;

fn engine() -> Engine {
    Engine::memory()
}

fn policy(scope: Scope, metric: Metric, window: Window, amount: f64) -> BudgetPolicy {
    BudgetPolicy {
        id: String::new(),
        scope,
        metric,
        window,
        amount,
        warn_pct: None,
        hard_stop: true,
        enabled: true,
        created_ms: 0,
        updated_ms: 0,
    }
}

fn ctx(agent: &str, conv: &str) -> TurnCtx {
    TurnCtx {
        request_id: uuid::Uuid::new_v4().to_string(),
        agent_id: agent.into(),
        account: Some("acc".into()),
        model: "claude-haiku-4-5".into(),
        conversation_id: conv.into(),
        estimate_tokens: 100,
    }
}

fn priced(e: &Engine) {
    e.set_price(Arc::new(|_m| {
        Box::pin(async {
            Some(Price {
                input_per_m: 1.0,
                output_per_m: 5.0,
            })
        })
    }));
}

fn usd(v: f64) -> TurnUse {
    TurnUse {
        usd: Some(v),
        input_tokens: 1_000,
        output_tokens: 100,
    }
}

#[test]
fn scopes_parse_and_print_the_same() {
    for raw in [
        "global",
        "agent:a",
        "job:j1",
        "loop:l1",
        "mission:m",
        "account:max-1",
    ] {
        assert_eq!(Scope::parse(raw).unwrap().key(), raw);
    }
    assert!(Scope::parse("team:x").is_err());
    assert!(Scope::parse("agent:").is_err());
    let p: BudgetPolicy = serde_json::from_value(json!({
        "scope": "loop:l1", "metric": "quota_pct", "window": "5h", "amount": 5.0
    }))
    .unwrap();
    assert_eq!(p.scope, Scope::Loop("l1".into()));
    assert!(
        p.hard_stop && p.enabled,
        "hard and on unless said otherwise"
    );
    assert!(p.validate().is_ok());
    let bad = policy(Scope::Global, Metric::QuotaPct, Window::Day, 5.0);
    assert!(bad.validate().is_err(), "quota reads a 5h/7d window");
}

/// B0: without a policy the gate says yes and asks nothing, whatever the
/// ledger holds.
#[tokio::test]
async fn no_policy_changes_nothing() {
    let e = engine();
    assert!(!e.has_policies());
    let c = ctx("a", "conv-none");
    for _ in 0..3 {
        assert_eq!(e.gate(&c).await, Verdict::Allow);
        assert!(e.charge(&c, usd(10.0)).await.is_empty());
    }
    assert!(e.incidents(false).is_empty());
    // The ledger still says what the turns cost (a loop's card reads it).
    assert!((e.spent(&Scope::Agent("a".into())).usd - 30.0).abs() < 1e-9);
}

/// C1 + 80#P3: a hard ceiling is crossed by at most the turn that crossed
/// it; the incident opens once, the scope refuses the next turn, and raise
/// and resume lets it go on.
#[tokio::test]
async fn a_hard_ceiling_pauses_the_scope_until_it_is_raised() {
    let e = engine();
    priced(&e);
    let _b = Binding::new("conv-loop", vec![Scope::Loop("l1".into())]);
    let p = e
        .upsert(
            policy(Scope::Loop("l1".into()), Metric::Usd, Window::Run, 0.05),
            Actor::window(),
        )
        .unwrap();
    let c = ctx("a", "conv-loop");
    let mut turns = 0;
    loop {
        match e.gate(&c).await {
            Verdict::Allow => {}
            Verdict::Deny(b) => {
                assert_eq!(b.scope, Scope::Loop("l1".into()));
                assert!(b.incident.is_some());
                break;
            }
            Verdict::Ask(a) => panic!("no question expected: {a:?}"),
        }
        turns += 1;
        let crossed = e.charge(&c, usd(0.012)).await;
        assert!(turns < 20, "never stopped");
        if turns < 5 {
            assert!(crossed.is_empty(), "turn {turns}");
        }
    }
    assert_eq!(turns, 5, "0.012 x 5 = 0.06 >= 0.05");
    let spent = e.spent(&Scope::Loop("l1".into())).usd;
    assert!(
        spent <= 0.05 + 0.012 + 1e-9,
        "at most one turn over: {spent}"
    );
    let open = e.incidents(true);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].state, "open");
    // Another agent, another scope: unaffected.
    assert_eq!(e.gate(&ctx("b", "other")).await, Verdict::Allow);
    // Raising below what is spent is refused.
    assert!(e
        .resolve(&open[0].id, Resolution::Raise(0.05), Actor::window())
        .is_err());
    let settled = e
        .resolve(&open[0].id, Resolution::Raise(0.10), Actor::window())
        .unwrap();
    assert_eq!(settled.state, "resolved");
    assert_eq!(e.policy(&p.id).unwrap().amount, 0.10);
    assert_eq!(e.gate(&c).await, Verdict::Allow, "raised and resumed");
}

#[tokio::test]
async fn keep_paused_keeps_blocking_and_leaves_the_needs_you_list() {
    let e = engine();
    e.upsert(
        policy(Scope::Agent("a".into()), Metric::Usd, Window::Day, 0.01),
        Actor::window(),
    )
    .unwrap();
    let c = ctx("a", "c1");
    e.charge(&c, usd(0.02)).await;
    let inc = e.incidents(true).remove(0);
    let kept = e
        .resolve(&inc.id, Resolution::KeepPaused, Actor::window())
        .unwrap();
    assert_eq!(kept.state, "paused");
    assert!(matches!(e.gate(&c).await, Verdict::Deny(_)));
    // Removing the policy lifts it.
    e.delete(&inc.policy_id, Actor::window()).unwrap();
    assert_eq!(e.gate(&c).await, Verdict::Allow);
    assert_eq!(e.incident(&inc.id).unwrap().state, "resolved");
}

/// A policy never counts the same usage twice, and two policies on
/// different scopes each count it once (80#P3 risk: double counting).
#[tokio::test]
async fn each_scope_counts_a_turn_once() {
    let e = engine();
    e.upsert(
        policy(Scope::Global, Metric::WeightedTokens, Window::Day, 1e9),
        Actor::window(),
    )
    .unwrap();
    e.upsert(
        policy(
            Scope::Agent("a".into()),
            Metric::WeightedTokens,
            Window::Day,
            1e9,
        ),
        Actor::window(),
    )
    .unwrap();
    let _b = Binding::new("cx", vec![Scope::Job("j".into()), Scope::Loop("l".into())]);
    e.charge(&ctx("a", "cx"), usd(0.0)).await;
    for s in [
        Scope::Global,
        Scope::Agent("a".into()),
        Scope::Job("j".into()),
        Scope::Loop("l".into()),
        Scope::Account("acc".into()),
    ] {
        assert_eq!(e.spent(&s).tokens, 1_100, "{s:?}");
        assert_eq!(e.spent(&s).turns, 1, "{s:?}");
    }
}

/// 85#P6 + PROB C2: a subscription account reports no dollars; a dollar
/// ceiling on it counts the price table's equivalent and blocks.
#[tokio::test]
async fn a_subscription_account_with_a_dollar_ceiling_blocks() {
    let e = engine();
    priced(&e);
    e.upsert(
        policy(Scope::Account("acc".into()), Metric::Usd, Window::Day, 0.05),
        Actor::window(),
    )
    .unwrap();
    let c = ctx("a", "sub");
    let unknown = TurnUse {
        usd: None,
        input_tokens: 20_000,
        output_tokens: 2_000,
    };
    // 20k * 1/M + 2k * 5/M = 0.03 per turn.
    assert_eq!(e.gate(&c).await, Verdict::Allow);
    assert!(e.charge(&c, unknown).await.is_empty());
    assert_eq!(e.gate(&c).await, Verdict::Allow);
    let crossed = e.charge(&c, unknown).await;
    assert_eq!(crossed.len(), 1);
    assert_eq!(crossed[0].kind, "hard");
    let spend = e.spent(&Scope::Account("acc".into()));
    assert!((spend.usd - 0.06).abs() < 1e-9);
    assert!(
        (spend.usd_estimated - 0.06).abs() < 1e-9,
        "said to be an estimate"
    );
    assert!(matches!(e.gate(&c).await, Verdict::Deny(_)));
}

/// 85#P6: a model with no price under a dollar ceiling asks once; the
/// answer is kept (yes runs, no refuses without asking again).
#[tokio::test]
async fn a_model_without_a_price_asks_once() {
    let e = engine();
    e.set_price(Arc::new(|_m| Box::pin(async { None })));
    e.upsert(
        policy(Scope::Global, Metric::Usd, Window::Day, 5.0),
        Actor::window(),
    )
    .unwrap();
    let mut c = ctx("a", "unpriced");
    c.model = "mystery-1".into();
    let Verdict::Ask(needs) = e.gate(&c).await else {
        panic!("expected a question")
    };
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].kind, "unpriced");
    assert_eq!(needs[0].action(), "budget_unpriced_model");
    assert!(e.answer(&needs[0], true, Some("a")).is_none());
    assert_eq!(e.gate(&c).await, Verdict::Allow, "asked once");
    let mut d = ctx("a", "unpriced");
    d.model = "mystery-2".into();
    let Verdict::Ask(needs) = e.gate(&d).await else {
        panic!("another model asks its own question")
    };
    assert!(e.answer(&needs[0], false, Some("a")).is_some());
    assert!(
        matches!(e.gate(&d).await, Verdict::Deny(_)),
        "no is kept too"
    );
    // A model that reported its cost before is priced.
    let mut r = ctx("a", "unpriced");
    r.model = "reports-itself".into();
    e.charge(&r, usd(0.001)).await;
    assert_eq!(e.gate(&r).await, Verdict::Allow);
}

/// A soft ceiling is a checkpoint: past it, one question per window; no
/// pauses the scope with an incident.
#[tokio::test]
async fn a_soft_ceiling_is_a_checkpoint_that_asks_once_per_window() {
    let e = engine();
    priced(&e);
    let mut p = policy(Scope::Agent("a".into()), Metric::Usd, Window::Day, 0.01);
    p.hard_stop = false;
    e.upsert(p, Actor::window()).unwrap();
    let c = ctx("a", "soft");
    assert!(
        e.charge(&c, usd(0.02)).await.is_empty(),
        "soft never opens an incident by itself"
    );
    let Verdict::Ask(needs) = e.gate(&c).await else {
        panic!("checkpoint")
    };
    assert_eq!(needs[0].kind, "checkpoint");
    assert!(e.answer(&needs[0], true, Some("a")).is_none());
    assert_eq!(e.gate(&c).await, Verdict::Allow);
    // Refused on another agent: the scope pauses.
    let mut q = policy(Scope::Agent("b".into()), Metric::Usd, Window::Day, 0.01);
    q.hard_stop = false;
    e.upsert(q, Actor::window()).unwrap();
    let cb = ctx("b", "soft-b");
    e.charge(&cb, usd(0.02)).await;
    let Verdict::Ask(needs) = e.gate(&cb).await else {
        panic!("checkpoint")
    };
    let block = e.answer(&needs[0], false, Some("b")).unwrap();
    assert!(block.incident.is_some());
    assert!(matches!(e.gate(&cb).await, Verdict::Deny(_)));
}

/// 80#P4: a quota policy on a loop measures what the loop moved in the
/// account's window since it began, and survives the window rolling over.
#[tokio::test]
async fn quota_pct_counts_the_points_a_scope_moved() {
    let e = engine();
    let reading = Arc::new(Mutex::new(QuotaReading {
        five_hour: Some(QuotaWindow {
            used_pct: 40.0,
            resets_at: Some(1_000),
        }),
        seven_day: None,
    }));
    let r2 = reading.clone();
    e.set_quota(Arc::new(move |acct| {
        (acct == "acc").then(|| *r2.lock().unwrap())
    }));
    let _b = Binding::new("q", vec![Scope::Loop("lq".into())]);
    e.upsert(
        policy(
            Scope::Loop("lq".into()),
            Metric::QuotaPct,
            Window::FiveHours,
            5.0,
        ),
        Actor::window(),
    )
    .unwrap();
    let c = ctx("a", "q");
    let set = |v: f64, resets: i64| {
        *reading.lock().unwrap() = QuotaReading {
            five_hour: Some(QuotaWindow {
                used_pct: v,
                resets_at: Some(resets),
            }),
            seven_day: None,
        }
    };
    let mut turns = 0;
    let mut used = 40.0;
    while matches!(e.gate(&c).await, Verdict::Allow) {
        turns += 1;
        used += 2.0;
        set(used, 1_000);
        e.charge(&c, usd(0.0)).await;
        assert!(turns < 10);
    }
    // 42, 44, 46 (6 points >= 5): stopped after the third turn.
    assert_eq!(turns, 3);
    // An account-wide quota policy is absolute: 46% of the window.
    e.upsert(
        policy(
            Scope::Account("acc".into()),
            Metric::QuotaPct,
            Window::FiveHours,
            80.0,
        ),
        Actor::window(),
    )
    .unwrap();
    let st = e.status();
    let acct = st
        .iter()
        .find(|s| s.policy.scope == Scope::Account("acc".into()))
        .unwrap();
    assert_eq!(acct.observed, Some(46.0));
    assert_eq!(acct.source, "real");
    // Roll-over: the new window starts from 0, what was used is carried.
    let other = BudgetPolicy {
        scope: Scope::Loop("roll".into()),
        ..policy(Scope::Global, Metric::QuotaPct, Window::FiveHours, 50.0)
    };
    let other = e.upsert(other, Actor::window()).unwrap();
    let w = |v: f64, r: i64| QuotaWindow {
        used_pct: v,
        resets_at: Some(r),
    };
    assert_eq!(e.quota_delta(&other, "acc", w(90.0, 1), true), (0.0, 0.0));
    assert_eq!(e.quota_delta(&other, "acc", w(95.0, 1), true).1, 5.0);
    assert_eq!(e.quota_delta(&other, "acc", w(3.0, 2), true).1, 8.0);
}

/// Without a reading a quota policy cannot bite and says so.
#[tokio::test]
async fn quota_without_a_reading_does_not_invent_one() {
    let e = engine();
    e.set_quota(Arc::new(|_| None));
    e.upsert(
        policy(Scope::Global, Metric::QuotaPct, Window::FiveHours, 1.0),
        Actor::window(),
    )
    .unwrap();
    let c = ctx("a", "nq");
    assert_eq!(e.gate(&c).await, Verdict::Allow);
    assert!(e.charge(&c, usd(1.0)).await.is_empty());
    assert_eq!(e.status()[0].observed, None);
    assert_eq!(e.status()[0].source, "none");
}

#[tokio::test]
async fn warn_is_said_once_and_does_not_stop() {
    let e = engine();
    let mut p = policy(Scope::Global, Metric::Usd, Window::Day, 1.0);
    p.warn_pct = Some(50.0);
    e.upsert(p, Actor::window()).unwrap();
    let c = ctx("a", "w");
    assert!(e.charge(&c, usd(0.3)).await.is_empty());
    let crossed = e.charge(&c, usd(0.3)).await;
    assert_eq!(crossed.len(), 1);
    assert_eq!(crossed[0].kind, "warn");
    assert!(e.charge(&c, usd(0.1)).await.is_empty(), "said once");
    assert_eq!(e.gate(&c).await, Verdict::Allow);
}

#[tokio::test]
async fn a_turn_token_ceiling_refuses_a_turn_that_is_too_big() {
    let e = engine();
    e.upsert(
        policy(
            Scope::Agent("a".into()),
            Metric::WeightedTokens,
            Window::Turn,
            50.0,
        ),
        Actor::window(),
    )
    .unwrap();
    let c = ctx("a", "t");
    assert!(matches!(e.gate(&c).await, Verdict::Deny(b) if b.incident.is_none()));
}

#[test]
fn one_policy_per_scope_metric_and_window() {
    let e = engine();
    let a = e
        .upsert(
            policy(Scope::Global, Metric::Usd, Window::Day, 1.0),
            Actor::window(),
        )
        .unwrap();
    let b = e
        .upsert(
            policy(Scope::Global, Metric::Usd, Window::Day, 2.0),
            Actor::window(),
        )
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(e.policies().len(), 1);
    assert_eq!(e.policies()[0].amount, 2.0);
    e.upsert(
        policy(Scope::Global, Metric::Usd, Window::Week, 2.0),
        Actor::window(),
    )
    .unwrap();
    assert_eq!(e.policies().len(), 2);
}

#[test]
fn bindings_come_and_go_with_their_owner() {
    let conv = format!("bind-{}", uuid::Uuid::new_v4());
    {
        let _a = Binding::new(&conv, vec![Scope::Job("j".into())]);
        let _b = Binding::new(&conv, vec![Scope::Mission("m".into())]);
        assert_eq!(bound_scopes(&conv).len(), 2);
        assert_eq!(
            conversations_in(&Scope::Mission("m".into())),
            vec![conv.clone()]
        );
    }
    assert!(bound_scopes(&conv).is_empty());
}

#[test]
fn the_file_keeps_policies_and_incidents() {
    let dir = std::env::temp_dir().join(format!("omniget-bp-{}", uuid::Uuid::new_v4()));
    let path = dir.join(DB_FILE);
    let id = {
        let e = Engine::open(&path).unwrap();
        e.upsert(
            policy(Scope::Global, Metric::Usd, Window::Day, 1.0),
            Actor::window(),
        )
        .unwrap()
        .id
    };
    let again = Engine::open(&path).unwrap();
    assert_eq!(again.policies()[0].id, id);
    let _ = std::fs::remove_dir_all(&dir);
}
