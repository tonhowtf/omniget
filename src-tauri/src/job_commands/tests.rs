//! Tests of the command surface (2.5) and the BN5 `delegate` scenario
//! (`scripts/llm/delegation_e2e.py --scenario delegate`).
//!
//! The engine is a stand-in (`Fake`) that keeps job rows in memory with the
//! production `state_seq` rule and runs a scripted turn on a **real** broker
//! and run record: the turn asks `shell_exec`, waits for the answer, then
//! "writes" one file and ends in `review` with its `OUTCOME:`. Everything in
//! front of it is production code: `Commands::execute`, the receipts table,
//! `wait`, the HTTP handlers behind the real token guard, and the MCP tool
//! bodies (`mcp_call`).

use super::*;
use crate::bridge_tokens::{self as bt, Guard, Kind, LegacyLog, Lookup};
use crate::mcp::policy::Principal;
use omniget_core::core::assist::db::AssistDb;
use omniget_core::core::assist::runs::{self, NewRun, Registry, RunState};
use omniget_core::core::llm::agent::{GrantMode, ToolGrant, ToolSource};
use omniget_core::core::llm::broker::ToolBroker;
use omniget_core::core::llm::error::LlmError;
use omniget_core::core::llm::snapshot::FileChange;
use omniget_core::core::llm::types::ToolSpec;
use omniget_core::core::omni::bus::Bus;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

struct Ran(Arc<Mutex<Vec<String>>>);

#[async_trait::async_trait]
impl omniget_core::core::llm::broker::ToolExecutor for Ran {
    async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
        self.0.lock().unwrap().push(name.to_string());
        Ok(format!("ran {name}"))
    }
}

struct Inner {
    jobs: Mutex<BTreeMap<String, Job>>,
    db: Mutex<Connection>,
    tx: tokio::sync::watch::Sender<u64>,
    n: AtomicU64,
    /// Run the scripted turn (ask → answer → write → end).
    script: bool,
    door: Approvals,
}

#[derive(Clone)]
struct Fake(Arc<Inner>);

impl Fake {
    fn new(script: bool) -> (Self, Arc<Mutex<Vec<String>>>) {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let broker = Arc::new(
            ToolBroker::new(
                vec![ToolSpec {
                    name: "shell_exec".into(),
                    description: String::new(),
                    input_schema: json!({"type": "object"}),
                }],
                Arc::new(Ran(ran.clone())),
                Arc::new(Bus::new()),
            )
            .with_ask_timeout(Duration::from_secs(60)),
        );
        let registry = Registry::with_instance(
            Arc::new(AssistDb::open_in_memory().unwrap()),
            "boot-commands",
        );
        let door = Approvals {
            broker,
            registry,
            job_of: Arc::new(|run| run.strip_prefix("req-").map(|j| format!("job-{j}"))),
            runs_of_job: Arc::new(|job| {
                job.strip_prefix("job-")
                    .map(|j| vec![format!("req-{j}")])
                    .unwrap_or_default()
            }),
            on_cancel: Arc::new(|_| {}),
        };
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        (
            Self(Arc::new(Inner {
                jobs: Mutex::new(BTreeMap::new()),
                db: Mutex::new(conn),
                tx: tokio::sync::watch::channel(0).0,
                n: AtomicU64::new(0),
                script,
                door,
            })),
            ran,
        )
    }

    fn commands(&self) -> Commands {
        Commands {
            engine: Arc::new(self.clone()),
            approvals: Some(self.0.door.clone()),
        }
    }

    /// The production rule: `state_seq` grows by one per state change.
    fn save(&self, mut job: Job) {
        let mut all = self.0.jobs.lock().unwrap();
        let prev = all.get(&job.id).map(|j| (j.state.clone(), j.state_seq));
        job.state_seq =
            jobs::next_state_seq(prev.as_ref().map(|(s, n)| (s.as_str(), *n)), &job.state);
        all.insert(job.id.clone(), job);
        drop(all);
        self.0.tx.send_modify(|n| *n += 1);
    }

    fn set(&self, id: &str, f: impl FnOnce(&mut Job)) {
        let Some(mut j) = self.0.jobs.lock().unwrap().get(id).cloned() else {
            return;
        };
        if j.state == "cancelled" {
            return;
        }
        f(&mut j);
        self.save(j);
    }

    async fn turn(self, id: String, n: u64) {
        tokio::time::sleep(Duration::from_millis(2)).await;
        self.set(&id, |j| j.state = "running".into());
        let (run, call) = (format!("req-{n}"), format!("call-{n}"));
        let reg = self.0.door.registry.clone();
        let _ = reg.open_run(NewRun {
            id: run.clone(),
            conversation_id: "c".into(),
            bot_id: "worker".into(),
            ..Default::default()
        });
        let _ = reg.transition(&run, RunState::Running, None);
        let grants = vec![ToolGrant {
            source: ToolSource::Internal {
                name: "shell_exec".into(),
            },
            mode: GrantMode::Ask,
        }];
        let broker = self.0.door.broker.clone();
        let asked = {
            let (me, id, call, reg) = (self.clone(), id.clone(), call.clone(), reg.clone());
            tokio::spawn(async move {
                for _ in 0..5_000 {
                    if reg.pending_of_call(&call).is_some() {
                        me.set(&id, |j| j.state = "waiting_approval".into());
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
        };
        let out = runs::scope(
            Some(reg),
            broker.call(
                "worker",
                &grants,
                &run,
                &call,
                "shell_exec",
                json!({"command": "npm test"}),
            ),
        )
        .await;
        let _ = asked.await;
        match out {
            Ok(o) if o.ok => {
                self.set(&id, |j| j.state = "running".into());
                let text = "Added the test.\nOUTCOME: succeeded — ran npm test and wrote a.txt.";
                self.set(&id, |j| {
                    j.result = Some(text.into());
                    j.outcome = jobs::parse_outcome(text);
                    j.diff_files = Some(vec![FileChange {
                        path: "a.txt".into(),
                        status: "A".into(),
                        additions: Some(1),
                        deletions: Some(0),
                        binary: false,
                    }]);
                    j.additions = Some(1);
                    j.deletions = Some(0);
                    j.state = jobs::settle_state("done", j.diff_files.as_deref()).into();
                    j.finished_ms = Some(crate::llm_manager::now_ms());
                });
            }
            _ => self.set(&id, |j| {
                j.state = "failed".into();
                j.error = Some("the call was refused".into());
            }),
        }
    }
}

impl Engine for Fake {
    fn start(&self, spec: &StartSpec, owner: &str) -> Result<Job, String> {
        if spec.agent_id == "nobody" {
            return Err("ERR_LLM_JOBS: no agent `nobody`".into());
        }
        let n = self.0.n.fetch_add(1, Ordering::SeqCst) + 1;
        let job = Job {
            id: format!("job-{n}"),
            kind: "run".into(),
            agent_id: spec.agent_id.clone(),
            prompt: spec.prompt.clone(),
            state: "queued".into(),
            title: spec.title.clone(),
            owner: Some(owner.into()),
            run_mode: spec
                .run_mode
                .as_deref()
                .map(crate::jobs::RunMode::parse)
                .transpose()?
                .unwrap_or(crate::jobs::RunMode::ApprovalRequired),
            created_ms: crate::llm_manager::now_ms(),
            ..Default::default()
        };
        self.save(job);
        let job = self.0.jobs.lock().unwrap()[&format!("job-{n}")].clone();
        if self.0.script {
            tokio::spawn(self.clone().turn(job.id.clone(), n));
        }
        Ok(job)
    }
    fn job(&self, id: &str) -> Option<Job> {
        self.0.jobs.lock().unwrap().get(id).cloned()
    }
    fn cancel(&self, id: &str) -> Result<Job, String> {
        let mut j = self.job(id).ok_or("no job")?;
        if jobs::is_active(&j.state) {
            j.state = "cancelled".into();
            self.save(j);
        }
        Ok(self.job(id).unwrap())
    }
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.tx.subscribe()
    }
    fn recent(&self, limit: u32) -> Vec<Job> {
        let mut v: Vec<Job> = self.0.jobs.lock().unwrap().values().cloned().collect();
        v.sort_by_key(|j| std::cmp::Reverse(j.created_ms));
        v.truncate(limit as usize);
        v
    }
    fn receipts(&self) -> MutexGuard<'_, Connection> {
        self.0.db.lock().unwrap()
    }
    fn diff<'a>(&'a self, id: &'a str) -> BoxFuture<'a, DiffAnswer> {
        Box::pin(async move {
            let j = self.job(id).ok_or("no job")?;
            Ok((
                j,
                Some("--- /dev/null\n+++ b/a.txt\n@@ -0,0 +1 @@\n+ok\n".into()),
                true,
            ))
        })
    }
}

fn operator() -> Caller {
    Caller {
        owner: "token:t1".into(),
        channel: "http",
        can_operate: true,
        can_respond: true,
        owned_only: false,
        from_job: None,
        may_accept_always: true,
    }
}

fn start(id: &str, prompt: &str) -> Value {
    json!({"command_id": id, "type": "job.start", "agent_id": "claude-code", "prompt": prompt, "title": "t"})
}

fn client(scopes: &[&str]) -> Principal {
    Principal {
        id: "client-a".into(),
        name: "A".into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
    }
}

const ALL_MCP: &[&str] = &["agents_run", "orchestration_read", "approvals_respond"];

// ── idempotency ────────────────────────────────────────────────────────

#[tokio::test]
async fn the_same_command_id_runs_once_and_another_payload_conflicts() {
    let (f, _) = Fake::new(false);
    let cmds = f.commands();
    let c = operator();
    let (first, replayed) = cmds.execute(&c, &start("c-1", "fix it")).unwrap();
    assert!(!replayed);
    for _ in 0..4 {
        // Keys in another order: the same command.
        let again = json!({"prompt": "fix it", "title": "t", "agent_id": "claude-code", "type": "job.start", "command_id": "c-1"});
        let (r, replayed) = cmds.execute(&c, &again).unwrap();
        assert!(replayed);
        assert_eq!(r, first, "the same result");
    }
    assert_eq!(f.recent(10).len(), 1, "5 sends, 1 job");
    let e = cmds
        .execute(&c, &start("c-1", "something else"))
        .unwrap_err();
    assert_eq!((e.status, e.code), (409, ERR_COMMAND_CONFLICT));
    // Another caller's ids are its own.
    let other = Caller {
        owner: "token:t2".into(),
        ..operator()
    };
    let (r2, replayed) = cmds.execute(&other, &start("c-1", "fix it")).unwrap();
    assert!(!replayed);
    assert_ne!(r2["job_id"], first["job_id"]);
    // A refused command leaves no receipt: fixed, it runs.
    let bad =
        json!({"command_id": "c-2", "type": "job.start", "agent_id": "nobody", "prompt": "x"});
    assert_eq!(cmds.execute(&c, &bad).unwrap_err().status, 400);
    let good =
        json!({"command_id": "c-2", "type": "job.start", "agent_id": "claude-code", "prompt": "x"});
    assert!(!cmds.execute(&c, &good).unwrap().1);
}

#[tokio::test]
async fn a_command_checks_its_own_scope_and_fields() {
    let (f, _) = Fake::new(false);
    let cmds = f.commands();
    let reader = Caller {
        can_operate: false,
        can_respond: false,
        ..operator()
    };
    assert_eq!(
        cmds.execute(&reader, &start("r-1", "x")).unwrap_err().code,
        ERR_COMMAND_FORBIDDEN
    );
    let (r, _) = cmds.execute(&operator(), &start("o-1", "x")).unwrap();
    let job = r["job_id"].as_str().unwrap();
    let stop = json!({"command_id": "r-2", "type": "job.interrupt", "job_id": job});
    assert_eq!(
        cmds.execute(&reader, &stop).unwrap_err().code,
        ERR_COMMAND_FORBIDDEN
    );
    let answer = json!({"command_id": "r-3", "type": "approval.respond", "approval_id": "apr_x", "decision": "accept"});
    assert_eq!(
        cmds.execute(&reader, &answer).unwrap_err().code,
        ERR_COMMAND_FORBIDDEN
    );
    for (body, why) in [
        (
            json!({"command_id": "v1", "type": "job.start", "agent_id": "a", "prompt": "p", "task": "t"}),
            "both",
        ),
        (
            json!({"command_id": "v2", "type": "job.start", "agent_id": "a"}),
            "none",
        ),
        (
            json!({"command_id": "v3", "type": "job.start", "agent_id": "a", "prompt": "p", "isolation": "nope"}),
            "isolation",
        ),
        (
            json!({"command_id": "v4", "type": "job.start", "agent_id": "a", "prompt": "p", "workspace": "relative/dir"}),
            "workspace",
        ),
        (
            json!({"command_id": "v5", "type": "job.start", "agent_id": "a", "prompt": "p", "promt": "typo"}),
            "unknown field",
        ),
        (
            json!({"command_id": "v6", "type": "job.start", "agent_id": "a", "prompt": "p", "effort": "huge"}),
            "effort",
        ),
        (
            json!({"command_id": "v7", "type": "job.start", "agent_id": "a", "prompt": "p", "budget": -1}),
            "budget",
        ),
        (json!({"command_id": "v8", "type": "job.nope"}), "type"),
        (
            json!({"command_id": "", "type": "job.start", "agent_id": "a", "prompt": "p"}),
            "id",
        ),
    ] {
        assert_eq!(
            cmds.execute(&operator(), &body).unwrap_err().code,
            ERR_COMMAND_INVALID,
            "{why}"
        );
    }
    // `task`, `budget: {usd}` and `run_mode` are accepted and stored.
    let ok = json!({"command_id": "v9", "type": "job.start", "agent_id": "a", "task": "t", "budget": {"usd": 0.5}, "run_mode": "full_access"});
    let (r, _) = cmds.execute(&operator(), &ok).unwrap();
    assert_eq!(
        f.job(r["job_id"].as_str().unwrap()).unwrap().run_mode,
        crate::jobs::RunMode::FullAccess
    );
    assert_eq!(parse_start(ok).unwrap().budget_usd, Some(0.5));
}

/// 2.6 depth 1, enforced by the host: a request from a job (its header or
/// its own token) and an agent session on MCP never start a job.
#[tokio::test]
async fn a_job_context_never_starts_a_job() {
    let (f, _) = Fake::new(false);
    let cmds = f.commands();
    let from_job = Caller {
        from_job: Some("job j9".into()),
        ..operator()
    };
    let e = cmds.execute(&from_job, &start("d-1", "x")).unwrap_err();
    assert_eq!((e.status, e.code), (403, ERR_DELEGATION_DEPTH));
    let mut headers = HeaderMap::new();
    headers.insert(JOB_HEADER, "j9".parse().unwrap());
    let cli = Principal {
        id: "token:cli".into(),
        name: "cli".into(),
        scopes: bt::SCOPES.iter().map(|s| s.to_string()).collect(),
    };
    assert!(http_caller(&cli, &headers, true).from_job.is_some());
    let job_token = Principal {
        id: "job:j9".into(),
        ..cli.clone()
    };
    assert!(http_caller(&job_token, &HeaderMap::new(), false)
        .from_job
        .is_some());
    assert!(http_caller(&cli, &HeaderMap::new(), false)
        .from_job
        .is_none());
    let session = Principal {
        id: format!("{}thread-1", crate::mcp::SESSION_PREFIX),
        name: "s".into(),
        scopes: ALL_MCP.iter().map(|s| s.to_string()).collect(),
    };
    let e = mcp_call(
        &cmds,
        &session,
        "agent_run",
        json!({"commandId": "d-2", "agentId": "a", "prompt": "x"}),
    )
    .await
    .unwrap_err();
    assert!(e.starts_with(ERR_DELEGATION_DEPTH), "{e}");
    assert!(f.recent(10).is_empty());
}

// ── wait ───────────────────────────────────────────────────────────────

#[test]
fn a_wait_never_matches_a_state_it_already_saw() {
    let job = |state: &str, seq: i64| Job {
        state: state.into(),
        state_seq: seq,
        ..Default::default()
    };
    let t = parse_until("terminal");
    assert_eq!(
        wait_verdict(&job("done", 4), &t, 4),
        Verdict::Wait,
        "stale done"
    );
    assert_eq!(wait_verdict(&job("done", 5), &t, 4), Verdict::Match);
    assert_eq!(wait_verdict(&job("running", 5), &t, 4), Verdict::Wait);
    assert_eq!(
        wait_verdict(&job("running", 5), &[], 4),
        Verdict::Match,
        "any change"
    );
    let ask = vec!["waiting_approval".to_string()];
    assert_eq!(wait_verdict(&job("failed", 6), &ask, 4), Verdict::Never);
    assert!(t.contains(&"done_unreported".to_string()) && t.contains(&"review".to_string()));
    assert_eq!(
        query_param(Some("after_seq=3&until=done"), "until"),
        Some("done")
    );
    assert_eq!(query_param(Some("x_after_seq=3"), "after_seq"), None);
}

#[tokio::test]
async fn a_wait_wakes_on_the_change_and_a_timeout_is_an_error() {
    let (f, _) = Fake::new(false);
    let cmds = f.commands();
    let (r, _) = cmds.execute(&operator(), &start("w-1", "x")).unwrap();
    let id = r["job_id"].as_str().unwrap().to_string();
    let seq = r["job"]["state_seq"].as_i64().unwrap();
    let e = cmds
        .wait(
            &operator(),
            &id,
            &parse_until("terminal"),
            seq,
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
    assert_eq!((e.status, e.code), (408, ERR_WAIT_TIMEOUT));
    assert_eq!(e.job.unwrap()["state"], "queued");
    let (f2, id2) = (f.clone(), id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        f2.set(&id2, |j| j.state = "running".into());
        f2.set(&id2, |j| j.state = "done".into());
    });
    let t0 = Instant::now();
    let job = cmds
        .wait(
            &operator(),
            &id,
            &parse_until("terminal"),
            seq,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(job.state, "done");
    assert!(t0.elapsed() < Duration::from_secs(1), "woken, not polled");
    let e = cmds
        .wait(
            &operator(),
            &id,
            &["waiting_approval".into()],
            seq,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ERR_WAIT_UNREACHABLE);
    // MCP sees only its own jobs.
    let e = mcp_call(&cmds, &client(ALL_MCP), "job_get", json!({"jobId": id}))
        .await
        .unwrap_err();
    assert_eq!(e, "JOB_NOT_AUTHORIZED");
}

// ── outcome (2.6) ─────────────────────────────────────────────────────

#[test]
fn the_outcome_line_is_read_and_its_absence_is_a_state() {
    let v =
        jobs::parse_outcome("Done.\n\n**OUTCOME: succeeded** — fixed the parser. Tests pass.\n")
            .unwrap();
    assert_eq!(v["status"], "succeeded");
    assert_eq!(v["summary"], "fixed the parser. Tests pass.");
    let v = jobs::parse_outcome("OUTCOME: failed - the build is broken").unwrap();
    assert_eq!(
        (v["status"].as_str(), v["summary"].as_str()),
        (Some("failed"), Some("the build is broken"))
    );
    let v = jobs::parse_outcome("`OUTCOME: succeeded`: ok").unwrap();
    assert_eq!(v["summary"], "ok");
    // The last line wins; anything else is no outcome.
    assert_eq!(
        jobs::parse_outcome("OUTCOME: failed — a\nOUTCOME: succeeded — b").unwrap()["summary"],
        "b"
    );
    for none in [
        "",
        "all good",
        "OUTCOME: maybe — hm",
        "the outcome: succeeded",
    ] {
        assert!(jobs::parse_outcome(none).is_none(), "{none}");
    }
    assert_eq!(
        jobs::settle_outcome("run", "done", false),
        "done_unreported"
    );
    assert_eq!(jobs::settle_outcome("run", "done", true), "done");
    assert_eq!(jobs::settle_outcome("run", "review", false), "review");
    assert_eq!(
        jobs::settle_outcome("mission", "done", false),
        "done",
        "criteria decide"
    );
    assert_eq!(jobs::settle_outcome("run", "failed", false), "failed");
    assert!(jobs::is_success("done_unreported") && jobs::is_terminal("done_unreported"));
    assert!(jobs::asks_outcome("run") && jobs::asks_outcome("mission"));
    assert!(
        !jobs::asks_outcome("trigger")
            && !jobs::asks_outcome("loop")
            && !jobs::asks_outcome("chat")
    );
    // B0 mirrors this text: keep scripts/llm/missions_eval.py in step.
    let harness = include_str!("../../../scripts/llm/missions_eval.py");
    let py = jobs::OUTCOME_SUFFIX.replace('\n', "\\n");
    assert!(
        harness.contains(&py),
        "missions_eval.py must send the same OUTCOME suffix"
    );
}

#[test]
fn a_budget_rides_on_the_guard_it_finds() {
    assert!(jobs::with_budget(None, None).is_none());
    let g = jobs::with_budget(None, Some(0.10)).unwrap();
    let check = g.check.unwrap();
    assert!(check(10, Some(0.05)).is_ok());
    assert!(check(10, None).is_ok(), "unknown cost never trips it");
    assert!(check(10, Some(0.11)).unwrap_err().contains("budget"));
    let deadline = jobs::JobGuard {
        deadline_ms: Some(42),
        check: Some(Arc::new(
            |t, _| if t > 100 { Err("cap".into()) } else { Ok(()) },
        )),
        deadline_reason: "d".into(),
    };
    let g = jobs::with_budget(Some(deadline), Some(1.0)).unwrap();
    assert_eq!(g.deadline_ms, Some(42), "the deadline stays");
    let check = g.check.unwrap();
    assert_eq!(
        check(101, Some(0.1)).unwrap_err(),
        "cap",
        "the earlier check still runs"
    );
    assert!(check(1, Some(2.0)).is_err());
}

// ── BN5 `delegate` ─────────────────────────────────────────────────────

/// Pending ask id of a job, as `job_get` shows it to its MCP owner.
async fn pending_of(cmds: &Commands, p: &Principal, job: &str) -> String {
    let v = mcp_call(cmds, p, "job_get", json!({"jobId": job}))
        .await
        .unwrap();
    v["pendingApprovals"][0]["approvalId"]
        .as_str()
        .unwrap()
        .to_string()
}

fn record(slowest: &mut f64, t0: Instant) {
    *slowest = slowest.max(t0.elapsed().as_secs_f64() * 1000.0);
}

/// agent_run → approval → approval_respond → job_diff, 10/10 over MCP
/// (without a window) and 10/10 over HTTP (`/v1/commands` +
/// `/v1/jobs/{id}/wait` behind the production guard); the same command id
/// 5× → 1 job on both; no non-wait call blocks > 2 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bn5_delegate_end_to_end_without_a_window() {
    use axum::extract::{Path, RawQuery};
    use axum::routing::{get, post};

    let (f, ran) = Fake::new(true);
    let cmds = f.commands();
    let p = client(ALL_MCP);
    let mut slowest_ms = 0.0f64;
    let mut e2e_ms = Vec::new();
    let mut mcp_ok = 0;
    let until = json!(["waiting_approval", "terminal"]);

    // ── MCP ──
    for i in 0..10 {
        let t_start = Instant::now();
        let t0 = Instant::now();
        let r = mcp_call(&cmds, &p, "agent_run", json!({"commandId": format!("m-{i}"), "agentId": "claude-code", "prompt": "add a test", "title": format!("delegated {i}")}))
            .await
            .unwrap();
        record(&mut slowest_ms, t0);
        let job = r["jobId"].as_str().unwrap().to_string();
        let seq = r["stateSeq"].as_i64().unwrap();
        let w = mcp_call(
            &cmds,
            &p,
            "job_wait",
            json!({"jobId": job, "until": until, "afterSeq": seq, "timeoutMs": 10_000}),
        )
        .await
        .unwrap();
        assert_eq!(w["state"], "waiting_approval", "{w}");
        let seen = w["state_seq"].as_i64().unwrap();
        let t0 = Instant::now();
        let ask = pending_of(&cmds, &p, &job).await;
        let a = mcp_call(
            &cmds,
            &p,
            "approval_respond",
            json!({"approvalId": ask, "decision": "accept"}),
        )
        .await
        .unwrap();
        record(&mut slowest_ms, t0);
        assert_eq!(
            (a["state"].as_str(), a["jobId"].as_str()),
            (Some("resolved"), Some(job.as_str()))
        );
        let end = mcp_call(
            &cmds,
            &p,
            "job_wait",
            json!({"jobId": job, "until": ["terminal"], "afterSeq": seen, "timeoutMs": 10_000}),
        )
        .await
        .unwrap();
        let t0 = Instant::now();
        let d = mcp_call(&cmds, &p, "job_diff", json!({"jobId": job}))
            .await
            .unwrap();
        record(&mut slowest_ms, t0);
        e2e_ms.push(t_start.elapsed().as_secs_f64() * 1000.0);
        if end["state"] == "review"
            && end["outcome"]["status"] == "succeeded"
            && d["available"] == true
            && d["files"][0]["path"] == "a.txt"
        {
            mcp_ok += 1;
        }
    }
    // Same command id 5× (MCP) → 1 job.
    let before = f.recent(500).len();
    let mut ids = std::collections::HashSet::new();
    for _ in 0..5 {
        let r = mcp_call(
            &cmds,
            &p,
            "agent_run",
            json!({"commandId": "same-mcp", "agentId": "claude-code", "prompt": "once"}),
        )
        .await
        .unwrap();
        ids.insert(r["jobId"].as_str().unwrap().to_string());
    }
    let mcp_same = f.recent(500).len() - before;
    assert_eq!((ids.len(), mcp_same), (1, 1));
    let conflict = mcp_call(
        &cmds,
        &p,
        "agent_run",
        json!({"commandId": "same-mcp", "agentId": "claude-code", "prompt": "twice"}),
    )
    .await
    .unwrap_err();
    assert!(conflict.starts_with(ERR_COMMAND_CONFLICT));
    // Another client neither sees nor answers these jobs.
    let other = Principal {
        id: "client-b".into(),
        ..client(ALL_MCP)
    };
    let first = ids.iter().next().unwrap().clone();
    assert_eq!(
        mcp_call(&cmds, &other, "job_get", json!({"jobId": first}))
            .await
            .unwrap_err(),
        "JOB_NOT_AUTHORIZED"
    );
    // Without the grant: no run.
    let no_grant = client(&["orchestration_read"]);
    assert!(mcp_call(
        &cmds,
        &no_grant,
        "agent_run",
        json!({"commandId": "x", "agentId": "a", "prompt": "p"})
    )
    .await
    .unwrap_err()
    .starts_with(ERR_COMMAND_FORBIDDEN));

    // ── HTTP, behind the production guard ──
    let conn = Connection::open_in_memory().unwrap();
    bt::ensure_schema(&conn).unwrap();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let cli = bt::mint(&conn, Kind::Cli, "cli", &s(bt::SCOPES), None, None, 1_000)
        .unwrap()
        .token;
    let reader = bt::mint(
        &conn,
        Kind::User,
        "r",
        &s(&[bt::SCOPE_JOBS_READ]),
        None,
        None,
        1_000,
    )
    .unwrap()
    .token;
    let db = Arc::new(Mutex::new(conn));
    let ext = "ext-key-0123456789abcdefghijklmnopqrstuvw";
    let guard = Guard {
        extension: Arc::new(ext.into()),
        legacy_allowed: Arc::new(AtomicBool::new(true)),
        legacy_log: Arc::new(LegacyLog::default()),
        lookup: {
            let db = db.clone();
            Arc::new(move |t: &str| -> Lookup { bt::lookup(&db.lock().unwrap(), t, 2_000) })
        },
        now: Arc::new(|| 2_000),
    };
    let (c1, c2, c3) = (cmds.clone(), cmds.clone(), cmds.clone());
    let router = axum::Router::new()
        .route(
            "/v1/commands",
            post(
                move |headers: HeaderMap,
                      pr: Option<axum::Extension<Principal>>,
                      axum::Json(b): axum::Json<Value>| async move {
                    http_command(&c1, pr.as_ref().map(|p| &p.0), &headers, &b)
                },
            ),
        )
        .route(
            "/v1/jobs/{id}",
            get(
                move |Path(id): Path<String>, RawQuery(q): RawQuery| async move {
                    http_job(&c2, &id, q.as_deref())
                },
            ),
        )
        .route(
            "/v1/jobs/{id}/wait",
            get(
                move |Path(id): Path<String>,
                      RawQuery(q): RawQuery,
                      headers: HeaderMap,
                      pr: Option<axum::Extension<Principal>>| async move {
                    http_wait(&c3, pr.as_ref().map(|p| &p.0), &headers, &id, q.as_deref()).await
                },
            ),
        )
        .route_layer(axum::middleware::from_fn_with_state(guard, bt::guard));
    let base = bt::bn5::serve(router).await;
    let http = reqwest::Client::new();
    let post_cmd = |body: Value, token: &str| {
        http.post(format!("{base}/v1/commands"))
            .bearer_auth(token)
            .json(&body)
            .send()
    };
    let mut http_ok = 0;
    for i in 0..10 {
        let t_start = Instant::now();
        let t0 = Instant::now();
        let r = post_cmd(start(&format!("h-{i}"), "add a test"), &cli)
            .await
            .unwrap();
        record(&mut slowest_ms, t0);
        assert_eq!(r.status().as_u16(), 200);
        let r: Value = r.json().await.unwrap();
        let job = r["job_id"].as_str().unwrap().to_string();
        let seq = r["job"]["state_seq"].as_i64().unwrap();
        let w: Value = http
            .get(format!("{base}/v1/jobs/{job}/wait?until=waiting_approval,terminal&after_seq={seq}&timeout_ms=10000"))
            .bearer_auth(&cli)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(w["state"], "waiting_approval");
        let seen = w["state_seq"].as_i64().unwrap();
        // The owner here is a bridge token: the ask id comes from the list
        // the CLI reads (`GET /v1/approvals?job_id=`).
        let ask = crate::approvals::list(
            &f.0.door,
            &crate::approvals::Filter {
                state: None,
                job_id: Some(job.clone()),
            },
        )
        .unwrap()[0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let t0 = Instant::now();
        let a = post_cmd(
            json!({"command_id": format!("h-{i}-ok"), "type": "approval.respond", "approval_id": ask, "decision": "accept", "channel": "cli"}),
            &cli,
        )
        .await
        .unwrap();
        record(&mut slowest_ms, t0);
        assert_eq!(a.status().as_u16(), 200);
        let end: Value = http
            .get(format!(
                "{base}/v1/jobs/{job}/wait?until=terminal&after_seq={seen}&timeout_ms=10000"
            ))
            .bearer_auth(&cli)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        e2e_ms.push(t_start.elapsed().as_secs_f64() * 1000.0);
        let row = f.0.door.registry.permission(&ask).unwrap();
        if end["state"] == "review"
            && row.channel.as_deref() == Some("cli")
            && end["outcome"]["status"] == "succeeded"
        {
            http_ok += 1;
        }
    }
    // Same command id 5× over HTTP → 1 job, the copies flagged.
    let before = f.recent(500).len();
    let mut replays = 0;
    for _ in 0..5 {
        let r = post_cmd(start("same-http", "once"), &cli).await.unwrap();
        if r.headers().get("idempotent-replayed").is_some() {
            replays += 1;
        }
    }
    let http_same = f.recent(500).len() - before;
    assert_eq!((http_same, replays), (1, 4));
    let clash = post_cmd(start("same-http", "other"), &cli).await.unwrap();
    assert_eq!(clash.status().as_u16(), 409);
    // Scopes and depth on the route.
    let mut statuses = BTreeMap::new();
    statuses.insert(
        "reader_starts",
        post_cmd(start("r-1", "x"), &reader)
            .await
            .unwrap()
            .status()
            .as_u16(),
    );
    statuses.insert(
        "extension_key",
        post_cmd(start("e-1", "x"), ext)
            .await
            .unwrap()
            .status()
            .as_u16(),
    );
    statuses.insert(
        "from_a_job",
        http.post(format!("{base}/v1/commands"))
            .bearer_auth(&cli)
            .header(JOB_HEADER, "j42")
            .json(&start("j-1", "x"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
    );
    let timeout = http
        .get(format!(
            "{base}/v1/jobs/{first}/wait?until=running&after_seq=999&timeout_ms=20"
        ))
        .bearer_auth(&cli)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    statuses.insert("wait_timeout", timeout);
    assert_eq!(statuses["reader_starts"], 403);
    assert_eq!(statuses["extension_key"], 403);
    assert_eq!(statuses["from_a_job"], 403);
    assert!(matches!(timeout, 408 | 409));

    assert_eq!((mcp_ok, http_ok), (10, 10));
    assert_eq!(
        ran.lock().unwrap().len(),
        20,
        "every approved call ran once"
    );
    assert!(slowest_ms < 2_000.0, "{slowest_ms} ms");
    e2e_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = e2e_ms[e2e_ms.len() / 2];
    for (metric, value) in [
        ("mcp_delegations_ok", json!(mcp_ok)),
        ("http_delegations_ok", json!(http_ok)),
        ("mcp_same_command_5x_jobs", json!(mcp_same)),
        ("http_same_command_5x_jobs", json!(http_same)),
        ("http_replays_flagged", json!(replays)),
        ("slowest_non_wait_call_ms", json!(slowest_ms)),
        ("e2e_ms_p50", json!(p50)),
        ("e2e_ms_max", json!(e2e_ms.last())),
        ("route_statuses", json!(statuses)),
        ("tools_ran", json!(ran.lock().unwrap().len())),
    ] {
        bt::bn5::emit(json!({"scenario": "delegate", "metric": metric, "value": value}));
    }
}
