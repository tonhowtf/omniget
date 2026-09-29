//! BN7 (3.3): budget policies end to end at the turn level — the real
//! Coordinator (gate before the turn, charge when it settles), the real CLI
//! runtime and `ClaudeParser`, and `scripts/llm/fake_claude.py` as the CLI
//! (cost, usage and `rate_limit_event` from its env, per conversation).
//!
//! The loop driver below does what `Jobs::run_loop` does about budgets and
//! nothing else: before each round it asks the engine whether the loop's
//! scopes are paused (`Engine::blocked`), binds the round to `loop:<id>` and
//! `job:<id>` (`Binding`), and runs one turn. The Tauri side (cancelling the
//! scope's jobs on `BudgetCrossed`, pausing the row, resuming on a raise)
//! is exercised by the app's own tests; this measures the engine and the
//! turn path. `scripts/llm/budget_eval.py` reads the rows these tests write
//! to `$OMNIGET_BENCH_OUT`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::core::assist::db::AssistDb;
use crate::core::assist::runs::Registry;
use crate::core::llm::agent::{AgentDef, AgentRole, Budget, ModelPolicy, RuntimeKind};
use crate::core::llm::broker::{ToolBroker, ToolExecutor};
use crate::core::llm::budget::BudgetStore;
use crate::core::llm::cli_runtime::{
    AccountStore, CliAccount, CliCapacity, CliKind, CliRuntime, CliRuntimeOptions, QuotaSource,
    RateSnapshot, RateStatus, RateWindow, SandboxMode,
};
use crate::core::llm::coordinator::Coordinator;
use crate::core::llm::types::{ModelRef, ProviderId, TurnEvent};
use crate::core::omni::bus::{Bus, BusEvent};

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
        Err(LlmError::new("ERR_TEST", format!("no tool {name}")))
    }
}

fn bench(metric: &str, value: Value) {
    if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{}", json!({ "metric": metric, "value": value }));
        }
    }
}

fn fake_script() -> Option<PathBuf> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/llm/fake_claude.py");
    let ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    (ok && script.is_file()).then_some(script)
}

/// Haiku 4.5 list price (US$ per million tokens), the table's answer for
/// any model named like it.
fn haiku_price() -> PriceFn {
    Arc::new(|m: String| {
        Box::pin(async move {
            m.contains("haiku").then_some(Price {
                input_per_m: 1.0,
                output_per_m: 5.0,
            })
        })
    })
}

struct Lab {
    root: PathBuf,
    runtime: Arc<CliRuntime>,
    coordinator: Arc<Coordinator>,
    engine: Arc<Engine>,
    bus: Arc<Bus>,
    broker: Arc<ToolBroker>,
}

impl Lab {
    fn new(name: &str) -> Option<Self> {
        let script = fake_script()?;
        let root =
            std::env::temp_dir().join(format!("omniget-bn7-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let bin = root.join("claude");
        std::fs::write(
            &bin,
            format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", script.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let store = AccountStore::at(root.join("accounts.json"));
        for id in ["max-1", "sub-1"] {
            std::fs::create_dir_all(root.join(id)).unwrap();
            store
                .create(CliAccount {
                    id: id.into(),
                    cli: CliKind::Claude,
                    config_dir: root.join(id),
                    label: id.into(),
                    disabled: false,
                    sandbox: SandboxMode::default(),
                })
                .unwrap();
        }
        let runtime = Arc::new(
            CliRuntime::new(Arc::new(store), Arc::new(CliCapacity::new())).with_options(
                CliRuntimeOptions {
                    sandbox_root: Some(root.join("sandboxes")),
                    ..CliRuntimeOptions::default()
                },
            ),
        );
        runtime.set_binary(CliKind::Claude, bin);
        runtime.set_caps(
            CliKind::Claude,
            crate::core::llm::caps::cli_caps(
                "claude",
                Some("2.1.282".into()),
                ["--resume", "--include-partial-messages"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
        );
        let engine = Arc::new(Engine::open(&root.join(DB_FILE)).unwrap());
        engine.set_quota(quota_from_capacity(runtime.capacity()));
        engine.set_price(haiku_price());
        let bus = Arc::new(Bus::new());
        let broker = Arc::new(ToolBroker::new(vec![], Arc::new(NoTools), bus.clone()));
        let reg = Registry::with_instance(
            Arc::new(AssistDb::open(&root.join("assist.db")).unwrap()),
            "bn7",
        );
        let coordinator = Arc::new(
            Coordinator::new(
                runtime.clone(),
                broker.clone(),
                Arc::new(BudgetStore::memory()),
                bus.clone(),
            )
            .with_dir(Some(root.join("conversations")))
            .with_runs(reg)
            .with_policies(engine.clone()),
        );
        Some(Self {
            root,
            runtime,
            coordinator,
            engine,
            bus,
            broker,
        })
    }

    fn conversation(&self, tag: &str, env: &[(&str, String)]) -> String {
        let conv = format!(
            "bn7-{tag}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let mut map: std::collections::BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        map.insert("FAKE_CLAUDE_MODE".into(), "ok".into());
        crate::core::llm::cli_runtime::set_conversation_env(&conv, map);
        conv
    }

    /// One turn; `true` when the CLI was actually started (its reply came).
    async fn turn(&self, agent: &AgentDef, conv: &str) -> (bool, Option<LlmError>) {
        let events: Vec<TurnEvent> = self
            .coordinator
            .run_turn(conv, agent, "work on it", CancellationToken::new())
            .collect()
            .await;
        // The CLI answered something: it was started (a refused turn has
        // only its error).
        let ran = events
            .iter()
            .any(|e| matches!(e, TurnEvent::TextDelta { text } if !text.trim().is_empty()));
        let err = events.iter().find_map(|e| match e {
            TurnEvent::Error { error } => Some(error.clone()),
            _ => None,
        });
        (ran, err)
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn agent(id: &str, account: &str) -> AgentDef {
    AgentDef {
        id: id.into(),
        name: id.into(),
        role: AgentRole::Worker,
        system_prompt: String::new(),
        model: ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("cli"),
                model: "haiku".into(),
            },
        },
        tools: vec![],
        skills: vec![],
        budget: Budget::default(),
        runtime: RuntimeKind::Cli {
            cli: "claude".into(),
            account: account.into(),
        },
        skin: None,
    }
}

/// What `Jobs::run_loop` does about budgets, round by round.
struct LoopRun {
    rounds_started: u32,
    crossed_at_round: Option<u32>,
    incident: Option<String>,
    started_after_hard: u32,
}

async fn run_loop(lab: &Lab, a: &AgentDef, loop_id: &str, conv: &str, rounds: u32) -> LoopRun {
    let mut bus = lab.bus.subscribe();
    let scopes = vec![
        Scope::Global,
        Scope::Agent(a.id.clone()),
        Scope::Account(match &a.runtime {
            RuntimeKind::Cli { account, .. } => account.clone(),
            _ => String::new(),
        }),
        Scope::Loop(loop_id.into()),
    ];
    let mut out = LoopRun {
        rounds_started: 0,
        crossed_at_round: None,
        incident: None,
        started_after_hard: 0,
    };
    for round in 1..=rounds {
        // run_loop: a paused scope starts no round (no job is created).
        if let Some(b) = lab.engine.blocked(&scopes, None) {
            out.incident = b.incident;
            break;
        }
        let job = format!("{loop_id}-r{round}");
        let _bind = Binding::new(conv, vec![Scope::Loop(loop_id.into()), Scope::Job(job)]);
        let (ran, _err) = lab.turn(a, conv).await;
        if ran {
            out.rounds_started += 1;
            if out.crossed_at_round.is_some() {
                out.started_after_hard += 1;
            }
        }
        while let Ok(ev) = bus.try_recv() {
            if let BusEvent::BudgetCrossed { kind, incident, .. } = ev {
                if kind == "hard" && out.crossed_at_round.is_none() {
                    out.crossed_at_round = Some(round);
                    out.incident = incident;
                }
            }
        }
    }
    out
}

/// BN7 main case (80#P3): a 20-round loop under a US$ 0.05 hard ceiling
/// stops after the turn that crossed it; nothing starts after; an incident
/// is open; raise and resume starts the next round well under 2 s.
#[tokio::test]
async fn bn7_a_loop_stops_at_its_ceiling_and_resumes_when_raised() {
    let Some(lab) = Lab::new("loop") else {
        eprintln!("python3 or fake_claude.py missing: skipped");
        return;
    };
    let per_turn = 0.012;
    let conv = lab.conversation("loop", &[("FAKE_CLAUDE_COST_USD", per_turn.to_string())]);
    let a = agent("looper", "max-1");
    let ceiling = 0.05;
    lab.engine
        .upsert(
            BudgetPolicy {
                id: String::new(),
                scope: Scope::Loop("l-bn7".into()),
                metric: Metric::Usd,
                window: Window::Run,
                amount: ceiling,
                warn_pct: Some(80.0),
                hard_stop: true,
                enabled: true,
                created_ms: 0,
                updated_ms: 0,
            },
            Actor::window(),
        )
        .unwrap();
    let run = run_loop(&lab, &a, "l-bn7", &conv, 20).await;
    let spent = lab.engine.spent(&Scope::Loop("l-bn7".into()));
    assert_eq!(run.crossed_at_round, Some(5), "0.012 x 5 = 0.06");
    assert_eq!(run.rounds_started, 5);
    assert_eq!(run.started_after_hard, 0);
    assert!(spent.usd <= ceiling + per_turn + 1e-9, "{}", spent.usd);
    let open = lab.engine.incidents(true);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].state, "open");
    assert_eq!(Some(open[0].id.clone()), run.incident);
    // A job someone starts in the paused scope is refused at the gate:
    // the CLI is never spawned.
    let _bind = Binding::new(&conv, vec![Scope::Loop("l-bn7".into())]);
    let (ran, err) = lab.turn(&a, &conv).await;
    drop(_bind);
    assert!(!ran);
    let err = err.expect("refused");
    assert_eq!(err.code, ERR_LLM_BUDGET);
    assert!(err.message.contains(&open[0].id), "{}", err.message);
    // Raise and resume: from the answer to the next round's reply.
    let t0 = Instant::now();
    lab.engine
        .resolve(&open[0].id, Resolution::Raise(0.10), Actor::window())
        .unwrap();
    let resumed = run_loop(&lab, &a, "l-bn7", &conv, 1).await;
    let resume_ms = t0.elapsed().as_millis() as u64;
    assert_eq!(resumed.rounds_started, 1);
    assert!(resume_ms < 2_000, "{resume_ms} ms");
    bench("loop_rounds_planned", json!(20));
    bench("loop_ceiling_usd", json!(ceiling));
    bench("loop_cost_per_turn_usd", json!(per_turn));
    bench("loop_rounds_started", json!(run.rounds_started));
    bench("loop_spent_usd", json!((spent.usd * 1e6).round() / 1e6));
    bench(
        "loop_jobs_started_after_hard",
        json!(run.started_after_hard),
    );
    bench("loop_refused_after_hard", json!(err.code == ERR_LLM_BUDGET));
    bench(
        "loop_incident_open",
        json!(open.len() == 1 && open[0].state == "open"),
    );
    bench("raise_resume_ms", json!(resume_ms));
}

/// BN7 `QuotaPct(5h) = 5%` (80#P4): each turn moves the account's 5 h window
/// by 2 points (the CLI's own `rate_limit_event`); the loop stops within one
/// turn of moving 5 points. The account had been used before (30 %): the
/// first reading comes from its earlier session, as in the app.
#[tokio::test]
async fn bn7_quota_pct_stops_within_one_turn() {
    let Some(lab) = Lab::new("quota") else {
        return;
    };
    let rate = lab.root.join("rate.txt");
    std::fs::write(&rate, "0.30").unwrap();
    let resets = 4_102_444_800i64;
    lab.runtime.capacity().record(
        "max-1",
        RateSnapshot {
            status: RateStatus::Allowed,
            window_5h: Some(RateWindow::from_utilization(0.30, Some(resets))),
            window_7d: None,
            source: QuotaSource::Real,
        },
    );
    let conv = lab.conversation(
        "quota",
        &[
            ("FAKE_CLAUDE_RATE_FILE", rate.display().to_string()),
            ("FAKE_CLAUDE_RATE_STEP", "0.02".into()),
            ("FAKE_CLAUDE_RATE_RESETS", resets.to_string()),
        ],
    );
    let a = agent("quota-looper", "max-1");
    lab.engine
        .upsert(
            BudgetPolicy {
                id: String::new(),
                scope: Scope::Loop("l-q".into()),
                metric: Metric::QuotaPct,
                window: Window::FiveHours,
                amount: 5.0,
                warn_pct: None,
                hard_stop: true,
                enabled: true,
                created_ms: 0,
                updated_ms: 0,
            },
            Actor::window(),
        )
        .unwrap();
    let run = run_loop(&lab, &a, "l-q", &conv, 20).await;
    // 30 → 32 → 34 → 36: 6 points after the third turn, the first >= 5.
    let exact = 3;
    assert_eq!(run.crossed_at_round, Some(exact));
    assert_eq!(run.rounds_started, exact);
    let past = run.rounds_started.saturating_sub(exact);
    let reading = lab.runtime.capacity().get("max-1").unwrap();
    bench("quota_policy_points", json!(5.0));
    bench("quota_step_points", json!(2.0));
    bench("quota_rounds_started", json!(run.rounds_started));
    bench("quota_turns_past_ceiling", json!(past));
    bench(
        "quota_window_used_pct",
        json!((reading.window_5h.unwrap().used as f64 * 1000.0).round() / 10.0),
    );
    bench("quota_incident_open", json!(run.incident.is_some()));
}

/// BN7 subscription (85#P6, PROB C2): the CLI reports no dollars; a US$ 0.05
/// ceiling on the account counts the price table's equivalent and blocks.
#[tokio::test]
async fn bn7_a_subscription_account_with_a_ceiling_blocks() {
    let Some(lab) = Lab::new("sub") else {
        return;
    };
    let conv = lab.conversation(
        "sub",
        &[
            ("FAKE_CLAUDE_COST_USD", "none".into()),
            ("FAKE_CLAUDE_INPUT_TOKENS", "10000".into()),
        ],
    );
    let a = agent("subscriber", "sub-1");
    lab.engine
        .upsert(
            BudgetPolicy {
                id: String::new(),
                scope: Scope::Account("sub-1".into()),
                metric: Metric::Usd,
                window: Window::Day,
                amount: 0.05,
                warn_pct: None,
                hard_stop: true,
                enabled: true,
                created_ms: 0,
                updated_ms: 0,
            },
            Actor::window(),
        )
        .unwrap();
    let run = run_loop(&lab, &a, "l-sub", &conv, 20).await;
    let spent = lab.engine.spent(&Scope::Account("sub-1".into()));
    let per_turn = spent.usd / spent.turns.max(1) as f64;
    assert!(run.crossed_at_round.is_some(), "the subscription is capped");
    assert!(run.rounds_started < 20);
    assert!(spent.usd <= 0.05 + per_turn + 1e-9);
    assert!(
        (spent.usd - spent.usd_estimated).abs() < 1e-9,
        "every dollar is the table's estimate"
    );
    bench("sub_rounds_started", json!(run.rounds_started));
    bench(
        "sub_spent_usd_estimated",
        json!((spent.usd * 1e6).round() / 1e6),
    );
    bench("sub_blocked", json!(run.crossed_at_round.is_some()));
}

/// A soft ceiling asks "continue?" through the broker (the same durable
/// ask every tool uses since 2.2); answering yes lets the turn go.
#[tokio::test]
async fn bn7_a_checkpoint_asks_through_the_approvals() {
    let Some(lab) = Lab::new("ck") else {
        return;
    };
    let conv = lab.conversation("ck", &[("FAKE_CLAUDE_COST_USD", "0.02".into())]);
    let a = agent("checker", "max-1");
    lab.engine
        .upsert(
            BudgetPolicy {
                id: String::new(),
                scope: Scope::Agent("checker".into()),
                metric: Metric::Usd,
                window: Window::Day,
                amount: 0.01,
                warn_pct: None,
                hard_stop: false,
                enabled: true,
                created_ms: 0,
                updated_ms: 0,
            },
            Actor::window(),
        )
        .unwrap();
    let (ran, _) = lab.turn(&a, &conv).await;
    assert!(ran, "under the checkpoint: no question");
    let mut bus = lab.bus.subscribe();
    let broker = lab.broker.clone();
    let answer = tokio::spawn(async move {
        loop {
            if let Ok(BusEvent::ToolAsk {
                tool, tool_call_id, ..
            }) = bus.recv().await
            {
                assert_eq!(tool, "budget_checkpoint");
                broker.answer(&tool_call_id, true);
                return true;
            }
        }
    });
    let (ran, err) = lab.turn(&a, &conv).await;
    assert!(answer.await.unwrap());
    assert!(ran, "{err:?}");
    let (ran, _) = lab.turn(&a, &conv).await;
    assert!(ran, "asked once per window");
    bench("checkpoint_asked_via_approvals", json!(true));
}

/// BN7 `--real` (quota authorized by the owner): the same 20-round loop on
/// the installed `claude` with Haiku and a US$ 0.05 ceiling. Ignored by
/// default; `scripts/llm/budget_eval.py --real` runs it. The account is the
/// CLI's own default login (no config dir is passed, nothing is read).
#[tokio::test]
#[ignore = "spends real quota: scripts/llm/budget_eval.py --real"]
async fn bn7_real_haiku_loop_stops_at_its_ceiling() {
    let Some(claude) = crate::core::dependencies::find_tool("claude").await else {
        eprintln!("claude is not installed: skipped");
        return;
    };
    let root = std::env::temp_dir().join(format!("omniget-bn7-real-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = AccountStore::at(root.join("accounts.json"));
    store
        .create(CliAccount {
            id: "real".into(),
            cli: CliKind::Claude,
            config_dir: PathBuf::new(),
            label: "default login".into(),
            disabled: false,
            sandbox: SandboxMode::default(),
        })
        .unwrap();
    let runtime = Arc::new(
        CliRuntime::new(Arc::new(store), Arc::new(CliCapacity::new())).with_options(
            CliRuntimeOptions {
                sandbox_root: Some(root.join("sandboxes")),
                ..CliRuntimeOptions::default()
            },
        ),
    );
    runtime.set_binary(CliKind::Claude, claude);
    let engine = Arc::new(Engine::open(&root.join(DB_FILE)).unwrap());
    engine.set_quota(quota_from_capacity(runtime.capacity()));
    engine.set_price(haiku_price());
    let bus = Arc::new(Bus::new());
    let broker = Arc::new(ToolBroker::new(vec![], Arc::new(NoTools), bus.clone()));
    let reg = Registry::with_instance(
        Arc::new(AssistDb::open(&root.join("assist.db")).unwrap()),
        "bn7-real",
    );
    let coordinator = Arc::new(
        Coordinator::new(
            runtime.clone(),
            broker.clone(),
            Arc::new(BudgetStore::memory()),
            bus.clone(),
        )
        .with_dir(Some(root.join("conversations")))
        .with_runs(reg)
        .with_policies(engine.clone()),
    );
    let lab = Lab {
        root,
        runtime,
        coordinator,
        engine,
        bus,
        broker,
    };
    let ceiling = 0.05;
    lab.engine
        .upsert(
            BudgetPolicy {
                id: String::new(),
                scope: Scope::Loop("l-real".into()),
                metric: Metric::Usd,
                window: Window::Run,
                amount: ceiling,
                warn_pct: None,
                hard_stop: true,
                enabled: true,
                created_ms: 0,
                updated_ms: 0,
            },
            Actor::window(),
        )
        .unwrap();
    let conv = format!(
        "bn7-real-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let a = agent("real-looper", "real");
    let started = Instant::now();
    let run = run_loop(&lab, &a, "l-real", &conv, 20).await;
    let spent = lab.engine.spent(&Scope::Loop("l-real".into()));
    let per_turn = spent.usd / spent.turns.max(1) as f64;
    bench("real_rounds_started", json!(run.rounds_started));
    bench("real_spent_usd", json!((spent.usd * 1e6).round() / 1e6));
    bench(
        "real_cost_per_turn_usd",
        json!((per_turn * 1e6).round() / 1e6),
    );
    bench("real_crossed_at_round", json!(run.crossed_at_round));
    bench(
        "real_jobs_started_after_hard",
        json!(run.started_after_hard),
    );
    bench(
        "real_incident_open",
        json!(!lab.engine.incidents(true).is_empty()),
    );
    bench(
        "real_wall_s",
        json!(started.elapsed().as_secs_f64().round()),
    );
    assert!(
        run.crossed_at_round.is_some(),
        "the ceiling was never reached"
    );
    assert_eq!(run.started_after_hard, 0);
    assert!(spent.usd <= ceiling + per_turn + 1e-9, "{}", spent.usd);
}

/// B0 without a ceiling: the gate is one read of an empty list and the
/// charge is the ledger rows; no price lookup, no question. Measured on a
/// file-backed engine (WAL), per turn.
#[tokio::test]
async fn bn7_no_policy_overhead_per_turn() {
    let root = std::env::temp_dir().join(format!("omniget-bn7-b0-{}", uuid::Uuid::new_v4()));
    let engine = Engine::open(&root.join(DB_FILE)).unwrap();
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = asked.clone();
    engine.set_price(Arc::new(move |_m| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { None })
    }));
    let ctx = TurnCtx {
        request_id: "r".into(),
        agent_id: "a".into(),
        account: Some("max-1".into()),
        model: "sonnet".into(),
        conversation_id: "b0".into(),
        estimate_tokens: 1_000,
    };
    let n = 200u32;
    let t0 = Instant::now();
    for _ in 0..n {
        assert_eq!(engine.gate(&ctx).await, Verdict::Allow);
        assert!(engine
            .charge(
                &ctx,
                TurnUse {
                    usd: None,
                    input_tokens: 12_000,
                    output_tokens: 300,
                },
            )
            .await
            .is_empty());
    }
    let per_ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no price lookup"
    );
    assert!(engine.incidents(false).is_empty());
    assert!(per_ms < 5.0, "{per_ms} ms per turn");
    bench(
        "b0_no_policy_overhead_ms_per_turn",
        json!((per_ms * 1000.0).round() / 1000.0),
    );
    bench("b0_no_policy_price_lookups", json!(0));
    let _ = std::fs::remove_dir_all(&root);
}
