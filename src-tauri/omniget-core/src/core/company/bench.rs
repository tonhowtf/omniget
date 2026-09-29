//! BN13 — companies of agents. Correctness tests that double as benchmarks:
//! each writes JSON lines `{bench, metric, value, unit}` to
//! `$OMNIGET_BENCH_OUT` (when set) and prints them. The numbers compare with
//! the Paperclip baseline measured in study 92 (`areas/E-baseline.md`).
//!
//! Run: `cargo test -p omniget-core --lib company::bench -- --nocapture --test-threads=1`
//! (`--release` for the headline numbers).

use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use super::gov::{CostEvent, NewPolicy};
use super::heartbeat::RunOutcome;
use super::issues::{IssueFilter, NewIssue};
use super::org::{NewCompany, NewGoal, NewStaff, StaffPatch};
use super::portable::ImportOptions;
use super::routines::NewRoutine;
use super::{now_ms, test_store, Actor, CompanyStore};

fn emit(bench: &str, metric: &str, value: f64, unit: &str) {
    let line = json!({"bench": bench, "metric": metric, "value": value, "unit": unit});
    println!("{line}");
    if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() {
        return 0.0;
    }
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn company(st: &CompanyStore) -> (String, Vec<String>) {
    let c = st
        .company_create(
            NewCompany {
                name: "Acme Labs".into(),
                mission: "Ship it".into(),
                require_hire_approval: Some(false),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    let ceo = st
        .staff_hire(
            &c.id,
            NewStaff {
                agent_id: "a-ceo".into(),
                name: "Ada".into(),
                role: "CEO".into(),
                can_hire: true,
                max_concurrent: Some(2),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    let mut ids = vec![ceo.id.clone()];
    for i in 0..4 {
        let s = st
            .staff_hire(
                &c.id,
                NewStaff {
                    agent_id: format!("a-eng{i}"),
                    name: format!("Eng {i}"),
                    role: "Engineer".into(),
                    reports_to: Some(ceo.id.clone()),
                    ..Default::default()
                },
                &Actor::user(),
            )
            .unwrap();
        ids.push(s.id);
    }
    (c.id, ids)
}

#[test]
fn bn13_model_and_template_import() {
    let (_d, st) = test_store();
    let t0 = Instant::now();
    let pkg = super::portable::template("startup").unwrap();
    let o = ImportOptions {
        include_issues: true,
        ..Default::default()
    };
    let plan = st.import_preview(&pkg, &o).unwrap();
    let c = st
        .import_apply(&pkg, &o, &plan.plan_hash, &Actor::user())
        .unwrap();
    emit("bn13", "template_import_ms", ms(t0), "ms");
    assert_eq!(c.issue_prefix, "STA");
    let staff = st.staff_list(&c.id).unwrap();
    assert_eq!(staff.len(), 5);
    let chart = st.org_chart(&c.id).unwrap();
    assert_eq!(chart.len(), 1, "one root: the CEO");
    assert_eq!(chart[0].staff.role, "CEO");
    assert_eq!(st.routine_list(&c.id).unwrap().len(), 1);
    // Export → import round trip keeps the org chart.
    let pkg2 = st.export(&c.id, true).unwrap();
    let plan2 = st.import_preview(&pkg2, &o).unwrap();
    let c2 = st
        .import_apply(&pkg2, &o, &plan2.plan_hash, &Actor::user())
        .unwrap();
    assert_ne!(c2.issue_prefix, c.issue_prefix);
    let chart2 = st.org_chart(&c2.id).unwrap();
    assert_eq!(chart2[0].reports.len(), chart[0].reports.len());
    // A changed package is refused after the preview.
    let mut changed = pkg2.clone();
    changed.company.name = "Other".into();
    assert!(st
        .import_apply(&changed, &o, &plan2.plan_hash, &Actor::user())
        .is_err());
    emit("bn13", "roundtrip_org_chart_ok", 1.0, "bool");
}

#[test]
fn bn13_checkout_contention() {
    let (_d, st) = test_store();
    let st = Arc::new(st);
    let (cid, staff) = company(&st);
    let rounds = 30;
    let clients = 50;
    let mut resolve = Vec::new();
    for r in 0..rounds {
        let i = st
            .issue_create(
                &cid,
                NewIssue {
                    title: format!("race {r}"),
                    ..Default::default()
                },
                &Actor::user(),
            )
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(clients));
        let t0 = Instant::now();
        let handles: Vec<_> = (0..clients)
            .map(|k| {
                let st = st.clone();
                let (cid, iid, who) = (cid.clone(), i.id.clone(), staff[k % staff.len()].clone());
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    st.issue_checkout(&cid, &iid, &who, &format!("run-{r}-{k}"), None)
                        .is_ok()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|w| *w)
            .count();
        resolve.push(ms(t0));
        assert_eq!(wins, 1, "exactly one checkout wins (round {r})");
    }
    emit("bn13", "checkout_50_winners_per_round", 1.0, "count");
    emit(
        "bn13",
        "checkout_50_resolve_p50_ms",
        pct(&mut resolve.clone(), 0.5),
        "ms",
    );
    emit(
        "bn13",
        "checkout_50_resolve_max_ms",
        pct(&mut resolve, 1.0),
        "ms",
    );
}

#[test]
fn bn13_issue_api_latency() {
    let (_d, st) = test_store();
    let (cid, staff) = company(&st);
    let mut create = Vec::new();
    let mut ids = Vec::new();
    let t_all = Instant::now();
    for n in 0..1000 {
        let t = Instant::now();
        let i = st
            .issue_create(
                &cid,
                NewIssue {
                    title: format!("Issue number {n} about the login page"),
                    body: "Steps to reproduce: open the app, click sign in.".into(),
                    assignee_staff_id: Some(staff[n % staff.len()].clone()),
                    ..Default::default()
                },
                &Actor::user(),
            )
            .unwrap();
        create.push(ms(t));
        ids.push(i.id);
    }
    emit("bn13", "create_1000_total_ms", ms(t_all), "ms");
    emit("bn13", "create_p50_ms", pct(&mut create.clone(), 0.5), "ms");
    emit("bn13", "create_p99_ms", pct(&mut create, 0.99), "ms");
    let mut list = Vec::new();
    for _ in 0..50 {
        let t = Instant::now();
        let l = st
            .issue_list(
                &cid,
                &IssueFilter {
                    limit: Some(500),
                    ..Default::default()
                },
            )
            .unwrap();
        list.push(ms(t));
        assert_eq!(l.len(), 500);
    }
    emit("bn13", "list500_p50_ms", pct(&mut list.clone(), 0.5), "ms");
    emit("bn13", "list500_p99_ms", pct(&mut list, 0.99), "ms");
    let (mut get, mut comment, mut status) = (Vec::new(), Vec::new(), Vec::new());
    for id in ids.iter().take(200) {
        let t = Instant::now();
        st.issue_detail(&cid, id).unwrap();
        get.push(ms(t));
        let t = Instant::now();
        st.comment_add(&cid, id, "Looking into it", None, &Actor::user())
            .unwrap();
        comment.push(ms(t));
        let t = Instant::now();
        st.issue_transition(&cid, id, "cancelled", None, None, &Actor::user())
            .unwrap();
        status.push(ms(t));
    }
    emit("bn13", "get_p50_ms", pct(&mut get.clone(), 0.5), "ms");
    emit("bn13", "get_p99_ms", pct(&mut get, 0.99), "ms");
    emit(
        "bn13",
        "comment_p50_ms",
        pct(&mut comment.clone(), 0.5),
        "ms",
    );
    emit("bn13", "comment_p99_ms", pct(&mut comment, 0.99), "ms");
    emit("bn13", "status_p50_ms", pct(&mut status.clone(), 0.5), "ms");
    emit("bn13", "status_p99_ms", pct(&mut status, 0.99), "ms");
    // Identifiers are unique and dense.
    let n: i64 = st
        .read(|c| {
            c.query_row(
                "SELECT COUNT(DISTINCT identifier) FROM issues WHERE company_id = ?1",
                [&cid],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(n, 1000);
}

#[test]
fn bn13_search_at_scale() {
    let (_d, st) = test_store();
    let (cid, _) = company(&st);
    let words = [
        "login",
        "payment",
        "invoice",
        "dashboard",
        "export",
        "latency",
        "onboarding",
        "search",
        "mobile",
        "billing",
    ];
    st.write(|tx, ch| {
        for n in 0..10_000 {
            let w = words[n % words.len()];
            let w2 = words[(n / 7) % words.len()];
            super::issues::issue_create_tx(
                tx,
                ch,
                &cid,
                NewIssue {
                    title: format!("{w} problem {n}"),
                    body: format!("The {w2} flow breaks when {w} runs"),
                    ..Default::default()
                },
                &Actor::user(),
            )?;
        }
        Ok(())
    })
    .unwrap();
    let mut lat = Vec::new();
    let prefix = st.company_get(&cid).unwrap().issue_prefix;
    let ident = format!("{prefix}-5000");
    for q in [
        "payment",
        "invoice problem",
        ident.as_str(),
        "latency",
        "breaks when",
        "zzz-none",
    ] {
        for _ in 0..10 {
            let t = Instant::now();
            let hits = st.issue_search(&cid, q, 20).unwrap();
            lat.push(ms(t));
            if std::env::var("BN13_VERBOSE").is_ok() {
                println!("q={q:?} {:.2} ms hits={}", ms(t), hits.len());
            }
            if q == ident {
                assert_eq!(hits[0].issue.identifier, ident);
            }
        }
    }
    emit(
        "bn13",
        "search_10k_p50_ms",
        pct(&mut lat.clone(), 0.5),
        "ms",
    );
    emit("bn13", "search_10k_p95_ms", pct(&mut lat, 0.95), "ms");
}

#[test]
fn bn13_heartbeat_coalescing_and_throughput() {
    let (_d, st) = test_store();
    let (cid, staff) = company(&st);
    let eng = staff[1].clone();
    let i = st
        .issue_create(
            &cid,
            NewIssue {
                title: "Fix the build".into(),
                assignee_staff_id: Some(eng.clone()),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    // A burst of 50 comments while the agent is idle → one run.
    for k in 0..50 {
        st.comment_add(&cid, &i.id, &format!("ping {k}"), None, &Actor::user())
            .unwrap();
    }
    let t = Instant::now();
    let claims = st.claim(now_ms(), 8).unwrap();
    emit("bn13", "claim_ms", ms(t), "ms");
    let mine: Vec<_> = claims.iter().filter(|c| c.staff.id == eng).collect();
    assert_eq!(mine.len(), 1, "the burst became one run");
    assert_eq!(
        mine[0].wakes.len(),
        51,
        "assignment + 50 comments ride along"
    );
    assert_eq!(super::heartbeat::lost_wakes(&st, &cid).unwrap(), 0);
    emit("bn13", "burst50_runs", mine.len() as f64, "count");
    emit("bn13", "lost_wakes", 0.0, "count");
    // The checkout happened on the run's behalf.
    let got = st.issue_get(&cid, &i.id).unwrap();
    assert_eq!(got.status, "in_progress");
    assert_eq!(got.lease_run_id.as_deref(), Some(mine[0].run.id.as_str()));
    // A run that leaves no trace is continued, then handed to the owner.
    let mut run_id = mine[0].run.id.clone();
    for attempt in 1..=super::heartbeat::MAX_CONTINUATIONS {
        st.run_finish(
            &run_id,
            RunOutcome {
                state: "succeeded".into(),
                ..Default::default()
            },
        )
        .unwrap();
        if attempt == super::heartbeat::MAX_CONTINUATIONS {
            break;
        }
        // Make the continuation due now and claim it.
        st.write(|tx, _| {
            tx.execute(
                "UPDATE wakeups SET not_before_ms = 0 WHERE state = 'queued'",
                [],
            )
            .map_err(super::sql)?;
            Ok(())
        })
        .unwrap();
        let c = st.claim(now_ms(), 8).unwrap();
        let next = c
            .into_iter()
            .find(|c| c.staff.id == eng)
            .expect("continuation claimed");
        run_id = next.run.id;
    }
    let got = st.issue_get(&cid, &i.id).unwrap();
    assert_eq!(
        got.status,
        "blocked",
        "handed to the owner after {} empty runs",
        super::heartbeat::MAX_CONTINUATIONS
    );
    // Throughput: 200 agents, one wake each.
    let c2 = st
        .company_create(
            NewCompany {
                name: "Swarm".into(),
                max_parallel: Some(64),
                require_hire_approval: Some(false),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    let mut who = Vec::new();
    for k in 0..200 {
        who.push(
            st.staff_hire(
                &c2.id,
                NewStaff {
                    agent_id: format!("s{k}"),
                    name: format!("S{k}"),
                    role: "Worker".into(),
                    ..Default::default()
                },
                &Actor::user(),
            )
            .unwrap()
            .id,
        );
    }
    let t = Instant::now();
    for w in &who {
        st.wake(&c2.id, w, None, "go", &Actor::user()).unwrap();
    }
    let enqueue = ms(t);
    let t = Instant::now();
    let mut claimed = 0;
    while claimed < 200 {
        let c = st.claim(now_ms(), 64).unwrap();
        claimed += c.len();
        for x in c {
            st.run_finish(
                &x.run.id,
                RunOutcome {
                    state: "succeeded".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }
    let secs = t.elapsed().as_secs_f64();
    emit(
        "bn13",
        "wake_enqueue_per_s",
        200.0 / (enqueue / 1000.0),
        "wakes/s",
    );
    emit("bn13", "claim_finish_runs_per_s", 200.0 / secs, "runs/s");
}

#[test]
fn bn13_budget_micros_and_hard_stop() {
    let (_d, st) = test_store();
    let (cid, staff) = company(&st);
    st.policy_upsert(
        &cid,
        NewPolicy {
            scope_type: "company".into(),
            scope_id: None,
            metric: "usd".into(),
            window: "month".into(),
            amount: 0.05,
            warn_pct: Some(80),
            hard_stop: Some(true),
        },
        &Actor::user(),
    )
    .unwrap();
    st.policy_upsert(
        &cid,
        NewPolicy {
            scope_type: "agent".into(),
            scope_id: Some(staff[2].clone()),
            metric: "tokens".into(),
            window: "day".into(),
            amount: 10_000.0,
            warn_pct: Some(80),
            hard_stop: Some(true),
        },
        &Actor::user(),
    )
    .unwrap();
    st.wake(&cid, &staff[1], None, "go", &Actor::user())
        .unwrap();
    let claim = st.claim(now_ms(), 8).unwrap().remove(0);
    // Twelve US$ 0.004 requests: 0.048 < 0.05; Paperclip rounds each to 0.
    for _ in 0..12 {
        st.record_cost(
            &claim.run.id,
            &CostEvent {
                provider: "anthropic".into(),
                model: "haiku".into(),
                billing_type: "metered".into(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: Some(0.004),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let s = st.cost_summary(&cid, "month").unwrap();
    assert!(
        (s.cost_usd - 0.048).abs() < 1e-9,
        "micro-dollars keep cheap turns: {}",
        s.cost_usd
    );
    emit(
        "bn13",
        "ledger_error_usd_12x0.004",
        (s.cost_usd - 0.048).abs(),
        "usd",
    );
    assert!(st.budget_block(&cid, &staff[1], None).unwrap().is_none());
    // The guard stops a run that would cross mid-way.
    assert!(st
        .would_exceed(&cid, &staff[1], None, 0, 0.003)
        .unwrap()
        .is_some());
    let crossings = st
        .record_cost(
            &claim.run.id,
            &CostEvent {
                cost_usd: Some(0.004),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(crossings.iter().any(|c| c.kind == "hard"));
    assert!(
        st.budget_block(&cid, &staff[3], None).unwrap().is_some(),
        "the whole company stops"
    );
    // One incident and one approval, never twice.
    st.record_cost(
        &claim.run.id,
        &CostEvent {
            cost_usd: Some(0.004),
            ..Default::default()
        },
    )
    .unwrap();
    let open: Vec<_> = st
        .incidents(&cid, true)
        .unwrap()
        .into_iter()
        .filter(|i| i.kind == "hard")
        .collect();
    assert_eq!(open.len(), 1);
    let appr = st.approvals(Some(&cid), Some("open")).unwrap();
    assert_eq!(
        appr.iter().filter(|a| a.kind == "budget_override").count(),
        1
    );
    // No new run starts while blocked.
    st.wake(&cid, &staff[3], None, "go", &Actor::user())
        .unwrap();
    assert!(st.claim(now_ms(), 8).unwrap().is_empty());
    emit("bn13", "runs_started_after_hard_stop", 0.0, "count");
    // Raise and resume.
    st.incident_resolve(&open[0].id, Some(1.0), &Actor::user())
        .unwrap();
    assert!(st.budget_block(&cid, &staff[3], None).unwrap().is_none());
    assert_eq!(st.claim(now_ms(), 8).unwrap().len(), 1);
    // Subscription turns (no price) still count against a token policy.
    st.wake(&cid, &staff[2], None, "go", &Actor::user())
        .unwrap();
    let c = st
        .claim(now_ms(), 8)
        .unwrap()
        .into_iter()
        .find(|c| c.staff.id == staff[2])
        .unwrap();
    st.record_cost(
        &c.run.id,
        &CostEvent {
            billing_type: "subscription".into(),
            input_tokens: 9_000,
            output_tokens: 2_000,
            cost_usd: None,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        st.budget_block(&cid, &staff[2], None).unwrap().is_some(),
        "tokens bite on a subscription"
    );
    emit("bn13", "subscription_token_cap_enforced", 1.0, "bool");
}

#[test]
fn bn13_audit_chain_and_invariants() {
    let (_d, st) = test_store();
    let (cid, staff) = company(&st);
    for n in 0..200 {
        st.issue_create(
            &cid,
            NewIssue {
                title: format!("t{n}"),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    }
    let t = Instant::now();
    let lines = st.audit_verify(&cid).unwrap();
    emit("bn13", "audit_verify_ms", ms(t), "ms");
    assert!(lines > 200);
    st.write(|tx, _| {
        tx.execute(
            "UPDATE audit SET action = 'issue.forged' WHERE seq = 10",
            [],
        )
        .map_err(super::sql)?;
        Ok(())
    })
    .unwrap();
    assert!(st.audit_verify(&cid).is_err(), "tampering is detected");
    emit("bn13", "audit_tamper_detected", 1.0, "bool");
    // Org chart loops are refused, and an existing loop never hangs.
    assert!(st
        .staff_update(
            &cid,
            &staff[0],
            StaffPatch {
                reports_to: Some(Some(staff[1].clone())),
                ..Default::default()
            },
            &Actor::user()
        )
        .is_err());
    st.write(|tx, _| {
        tx.execute(
            "UPDATE staff SET reports_to = ?1 WHERE id = ?2",
            [&staff[2], &staff[1]],
        )
        .map_err(super::sql)?;
        tx.execute(
            "UPDATE staff SET reports_to = ?1 WHERE id = ?2",
            [&staff[1], &staff[2]],
        )
        .map_err(super::sql)?;
        Ok(())
    })
    .unwrap();
    let t = Instant::now();
    let _ = st.staff_update(
        &cid,
        &staff[3],
        StaffPatch {
            reports_to: Some(Some(staff[1].clone())),
            ..Default::default()
        },
        &Actor::user(),
    );
    let _ = st.chain_of_command(&cid, &staff[1]);
    assert!(ms(t) < 1000.0, "a pre-existing loop terminates");
    // Goal levels.
    assert!(st
        .goal_create(
            &cid,
            NewGoal {
                level: "company".into(),
                title: "second mission".into(),
                ..Default::default()
            },
            &Actor::user()
        )
        .is_err());
    let team = st
        .goal_create(
            &cid,
            NewGoal {
                level: "task".into(),
                title: "t".into(),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    assert!(st
        .goal_create(
            &cid,
            NewGoal {
                level: "team".into(),
                parent_id: Some(team.id),
                title: "x".into(),
                ..Default::default()
            },
            &Actor::user()
        )
        .is_err());
    // Blocks loops.
    let a = st
        .issue_create(
            &cid,
            NewIssue {
                title: "a".into(),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    let b = st
        .issue_create(
            &cid,
            NewIssue {
                title: "b".into(),
                blocked_by: vec![a.id.clone()],
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    assert_eq!(b.status, "blocked");
    assert!(st
        .relate(&cid, &b.id, &a.id, "blocks", &Actor::user())
        .is_err());
    // Cross-company links are refused by the database.
    let other = st
        .company_create(
            NewCompany {
                name: "Other Co".into(),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    assert!(st
        .issue_create(
            &other.id,
            NewIssue {
                title: "x".into(),
                parent_id: Some(a.id.clone()),
                ..Default::default()
            },
            &Actor::user()
        )
        .is_err());
    // Done unblocks the dependent.
    st.issue_transition(&cid, &a.id, "cancelled", None, None, &Actor::user())
        .unwrap();
    st.issue_transition(&cid, &a.id, "todo", None, None, &Actor::user())
        .unwrap();
    st.issue_update(
        &cid,
        &a.id,
        super::issues::IssuePatch {
            assignee_staff_id: Some(Some(staff[1].clone())),
            ..Default::default()
        },
        &Actor::user(),
    )
    .unwrap();
    st.issue_transition(&cid, &a.id, "in_progress", None, None, &Actor::user())
        .unwrap();
    st.issue_transition(&cid, &a.id, "done", None, None, &Actor::user())
        .unwrap();
    assert_eq!(st.issue_get(&cid, &b.id).unwrap().status, "todo");
    emit("bn13", "invariant_violations", 0.0, "count");
}

#[test]
fn bn13_review_gate_and_depth() {
    let (_d, st) = test_store();
    let (cid, staff) = company(&st);
    let eng = Actor::agent(&staff[1], Some("r1"));
    let i = st
        .issue_create(
            &cid,
            NewIssue {
                title: "feature".into(),
                assignee_staff_id: Some(staff[1].clone()),
                review_policy: Some("not_creator".into()),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    st.issue_transition(&cid, &i.id, "in_progress", None, None, &eng)
        .unwrap();
    let after = st
        .issue_transition(&cid, &i.id, "done", Some("shipped"), None, &eng)
        .unwrap();
    assert_eq!(after.status, "in_review", "an agent sends work to review");
    assert!(
        st.issue_transition(&cid, &i.id, "done", None, None, &eng)
            .is_err(),
        "the author cannot close it"
    );
    let ok = st
        .issue_transition(
            &cid,
            &i.id,
            "done",
            None,
            None,
            &Actor::agent(&staff[0], None),
        )
        .unwrap();
    assert_eq!(ok.status, "done");
    // Delegation depth.
    let mut parent = st
        .issue_create(
            &cid,
            NewIssue {
                title: "root".into(),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap()
        .id;
    let mut depth_hit = false;
    for _ in 0..12 {
        match st.issue_create(
            &cid,
            NewIssue {
                title: "sub".into(),
                parent_id: Some(parent.clone()),
                ..Default::default()
            },
            &Actor::agent(&staff[0], None),
        ) {
            Ok(i) => parent = i.id,
            Err(_) => {
                depth_hit = true;
                break;
            }
        }
    }
    assert!(depth_hit);
}

#[test]
fn bn13_routine_catch_up() {
    let (_d, st) = test_store();
    let (cid, staff) = company(&st);
    // Slots at -3, -2, -1 min and the current minute: `skip` fires only the
    // on-time one, `once` one, `all_capped` all four.
    for (policy, want) in [("skip", 1usize), ("once", 1), ("all_capped", 4)] {
        let r = st
            .routine_create(
                &cid,
                NewRoutine {
                    title: format!("{policy} {{{{date}}}}"),
                    staff_id: staff[1].clone(),
                    cron: "*/1 * * * *".into(),
                    catch_up: Some(policy.into()),
                    concurrency: Some("always".into()),
                    tz: Some("UTC".into()),
                    ..Default::default()
                },
                &Actor::user(),
            )
            .unwrap();
        let now = now_ms();
        // Three minutes missed (the app was closed).
        let first = now - now % 60_000 - 3 * 60_000;
        st.write(|tx, _| {
            tx.execute(
                "UPDATE routines SET next_run_ms = ?2 WHERE id = ?1",
                rusqlite::params![r.id, first],
            )
            .map_err(super::sql)?;
            Ok(())
        })
        .unwrap();
        let filed: Vec<_> = st.tick_routines(now).unwrap();
        let n = filed
            .iter()
            .filter(|i| {
                st.issue_get(&cid, i)
                    .map(|x| x.title.starts_with(policy))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(n, want, "{policy}");
        // The same slot never fires twice.
        assert!(st
            .tick_routines(now)
            .unwrap()
            .iter()
            .all(|i| !filed.contains(i)));
        st.routine_delete(&cid, &r.id, &Actor::user()).unwrap();
    }
    emit("bn13", "routine_catch_up_ok", 1.0, "bool");
}

/// The Paperclip baseline scenario (study 92, E §heartbeat): a `process`
/// agent running `/usr/bin/true`, a burst of 50 wakes, measured from the
/// wake to the process start and to the run's end, through the same claim /
/// exec / finish path the engine uses (the engine adds its 150 ms settle).
#[test]
fn bn13_process_heartbeat_e2e() {
    let (_d, st) = test_store();
    let st = Arc::new(st);
    let c = st
        .company_create(
            NewCompany {
                name: "Proc".into(),
                max_parallel: Some(8),
                require_hire_approval: Some(false),
                ..Default::default()
            },
            &Actor::user(),
        )
        .unwrap();
    let mut who = Vec::new();
    for k in 0..50 {
        who.push(
            st.staff_hire(
                &c.id,
                NewStaff {
                    agent_id: format!("p{k}"),
                    name: format!("P{k}"),
                    role: "Worker".into(),
                    adapter: super::org::Adapter::Process {
                        command: "/usr/bin/true".into(),
                        args: vec![],
                        timeout_sec: Some(30),
                    },
                    ..Default::default()
                },
                &Actor::user(),
            )
            .unwrap()
            .id,
        );
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let mut to_start = Vec::new();
    let mut to_end = Vec::new();
    let t_all = Instant::now();
    let woke: std::collections::HashMap<String, Instant> = who
        .iter()
        .map(|w| {
            let t = Instant::now();
            st.wake(&c.id, w, None, "go", &Actor::user()).unwrap();
            (w.clone(), t)
        })
        .collect();
    let mut done = 0;
    rt.block_on(async {
        while done < who.len() {
            let claims = st.claim(now_ms(), 8).unwrap();
            if claims.is_empty() {
                tokio::task::yield_now().await;
                continue;
            }
            let mut tasks = Vec::new();
            for cl in claims {
                let started = woke[&cl.staff.id].elapsed().as_secs_f64() * 1000.0;
                to_start.push(started);
                let st = st.clone();
                let t0 = woke[&cl.staff.id];
                tasks.push(tokio::spawn(async move {
                    let body = super::exec::heartbeat_json(&cl, "");
                    let out = super::exec::run_process(
                        &cl,
                        &body,
                        "/usr/bin/true",
                        &[],
                        None,
                        Some(30),
                        &[],
                    )
                    .await;
                    assert_eq!(out.state, "succeeded");
                    st.run_finish(&cl.run.id, out).unwrap();
                    t0.elapsed().as_secs_f64() * 1000.0
                }));
            }
            for t in tasks {
                to_end.push(t.await.unwrap());
                done += 1;
            }
        }
    });
    let secs = t_all.elapsed().as_secs_f64();
    emit(
        "bn13",
        "proc_wake_to_start_p50_ms",
        pct(&mut to_start.clone(), 0.5),
        "ms",
    );
    emit(
        "bn13",
        "proc_wake_to_start_max_ms",
        pct(&mut to_start, 1.0),
        "ms",
    );
    emit(
        "bn13",
        "proc_wake_to_end_p50_ms",
        pct(&mut to_end.clone(), 0.5),
        "ms",
    );
    emit(
        "bn13",
        "proc_wake_to_end_max_ms",
        pct(&mut to_end, 1.0),
        "ms",
    );
    emit("bn13", "proc_burst50_runs_per_s", 50.0 / secs, "runs/s");
    let stats = st.heartbeat_stats(&c.id).unwrap();
    assert_eq!(stats["runs"]["succeeded"], 50);
}
