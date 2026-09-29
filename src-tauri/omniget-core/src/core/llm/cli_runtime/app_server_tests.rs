//! Codex over `codex app-server` end to end (item 3.6) — Coordinator →
//! CliRuntime → session (duplex) → run registry and broker — against a FAKE
//! app-server (`scripts/llm/fake_codex_app_server.py`) that speaks the
//! JSON-RPC subset of codex 0.158 and logs what OmniGet wrote. No model is
//! involved. What is exercised is our real code: the thread resume and its
//! fallback, what travels per turn, the approvals as durable asks, the
//! interrupt, the watchdog and the `exec` fallback.
//!
//! The `bn10_*` test also writes its measurements to `$OMNIGET_BENCH_OUT`
//! (`scripts/llm/codex_eval.py --fake`).

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::core::assist::db::AssistDb;
use crate::core::assist::runs::{Actor, PermissionState, Registry, ResumeKind};
use crate::core::llm::agent::{AgentRole, Budget, ModelPolicy};
use crate::core::llm::broker::{Decision, ToolBroker, ToolExecutor};
use crate::core::llm::budget::BudgetStore;
use crate::core::llm::coordinator::Coordinator;
use crate::core::llm::types::{FinishReason, GenParams, Message, ModelRef, ProviderId, Role};
use crate::core::omni::bus::Bus;

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
        Err(LlmError::new("ERR_TEST", format!("no tool {name}")))
    }
}

fn fake_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/llm/fake_codex_app_server.py")
}

fn available() -> bool {
    fake_script().exists()
        && std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_ok()
}

struct Lab {
    root: PathBuf,
    fake: PathBuf,
    db_path: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("omniget-appsrv-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("calls")).unwrap();
        std::fs::create_dir_all(root.join("cx-1")).unwrap();
        // A wrapper so the fake finds its state dir whatever env the turn has.
        let fake = root.join("codex");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nFAKE_CODEX_DIR='{}' exec python3 '{}' \"$@\"\n",
                root.join("calls").display(),
                fake_script().display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let db_path = root.join("assist.db");
        Self {
            root,
            fake,
            db_path,
        }
    }

    fn mode(&self, mode: &str) {
        std::fs::write(self.root.join("calls/mode"), mode).unwrap();
    }

    fn registry(&self) -> Registry {
        Registry::with_instance(Arc::new(AssistDb::open(&self.db_path).unwrap()), "boot-1")
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join("calls").join(name)).unwrap_or_default()
    }

    /// What OmniGet wrote on stdin in call `n`, one JSON value per line.
    fn sent(&self, n: usize) -> Vec<Value> {
        self.file(&format!("call-{n}.in.jsonl"))
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn sent_method(&self, n: usize, method: &str) -> Option<Value> {
        self.sent(n).into_iter().find(|v| v["method"] == method)
    }

    fn runtime(&self, broker: Option<&Arc<ToolBroker>>, app_server: bool) -> Arc<CliRuntime> {
        let store = AccountStore::at(self.root.join("accounts.json"));
        if store.get("cx-1").is_none() {
            store
                .create(CliAccount {
                    id: "cx-1".into(),
                    cli: CliKind::Codex,
                    config_dir: self.root.join("cx-1"),
                    label: "Codex".into(),
                    disabled: false,
                    sandbox: SandboxMode::Write,
                })
                .unwrap();
        }
        let mut rt = CliRuntime::new(Arc::new(store), Arc::new(CliCapacity::new())).with_options(
            CliRuntimeOptions {
                sandbox_root: Some(self.root.join("sandboxes")),
                ..CliRuntimeOptions::default()
            },
        );
        if let Some(b) = broker {
            rt = rt.with_broker(Arc::downgrade(b));
        }
        rt.set_binary(CliKind::Codex, self.fake.clone());
        let mut caps =
            crate::core::llm::caps::cli_caps("codex", Some("0.158.0".into()), Vec::new());
        if app_server {
            crate::core::llm::caps::with_app_server(&mut caps);
        }
        rt.set_caps(CliKind::Codex, caps);
        Arc::new(rt)
    }

    fn coordinator(&self, reg: Registry) -> (Arc<Coordinator>, Arc<ToolBroker>) {
        let bus = Arc::new(Bus::new());
        let broker = Arc::new(ToolBroker::new(vec![], Arc::new(NoTools), bus.clone()));
        let c = Coordinator::new(
            self.runtime(Some(&broker), true),
            broker.clone(),
            Arc::new(BudgetStore::memory()),
            bus,
        )
        .with_dir(Some(self.root.join("conversations")))
        .with_runs(reg);
        (Arc::new(c), broker)
    }

    fn workspace(&self, name: &str) -> PathBuf {
        let d = self.root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn agent() -> AgentDef {
    AgentDef {
        id: "codex-coder".into(),
        name: "Codex".into(),
        role: AgentRole::Worker,
        system_prompt: "be brief".into(),
        model: ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("cli"),
                model: "gpt-5.6-luna".into(),
            },
        },
        tools: vec![],
        skills: vec![],
        budget: Budget {
            usd_per_day: Some(1.0),
            tokens_per_turn: None,
            max_tool_calls_per_turn: 0,
        },
        runtime: RuntimeKind::Cli {
            cli: "codex".into(),
            account: "cx-1".into(),
        },
        skin: None,
    }
}

async fn turn(c: &Arc<Coordinator>, run: &str, conv: &str, input: &str) -> Vec<TurnEvent> {
    c.run_turn_with_id(run.into(), conv, &agent(), input, CancellationToken::new())
        .collect()
        .await
}

fn conv_id(tag: &str) -> String {
    format!(
        "appsrv-{tag}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

fn text_of(ev: &[TurnEvent]) -> String {
    ev.iter()
        .filter_map(|e| match e {
            TurnEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn turn_text(v: &Value) -> String {
    v["params"]["input"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Acceptance: `thread/start` on the first turn, `thread/resume` of the same
/// thread afterwards with only the new message, `CODEX_HOME` = the account's
/// own directory, the workspace as cwd, the run mode on the thread and turn.
#[tokio::test]
async fn a_codex_conversation_resumes_its_thread_and_sends_only_the_new_message() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    let lab = Lab::new("resume");
    let conv = conv_id("resume");
    let ws = lab.workspace("ws");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws.clone())).unwrap();
    let reg = lab.registry();
    let (c, _broker) = lab.coordinator(reg.clone());

    let ev = turn(&c, "run-1", &conv, "first question").await;
    assert_eq!(text_of(&ev), "ok 1", "{ev:?}");
    assert_eq!(lab.file("call-1.args").trim(), "app-server");
    assert_eq!(
        lab.file("call-1.env"),
        lab.root.join("cx-1").display().to_string()
    );
    assert_eq!(lab.file("call-1.pwd").trim(), ws.display().to_string());
    let start = lab.sent_method(1, "thread/start").expect("thread/start");
    assert_eq!(start["params"]["approvalPolicy"], "on-request");
    assert_eq!(start["params"]["sandbox"], "workspace-write");
    assert!(start["params"]["developerInstructions"]
        .as_str()
        .unwrap()
        .contains("be brief"));
    assert_eq!(
        turn_text(&lab.sent_method(1, "turn/start").unwrap()),
        "first question"
    );
    let s = reg.session(&conv, "codex-coder").unwrap();
    assert_eq!(s.provider_handle.as_deref(), Some("th-1"));
    assert_eq!(reg.run("run-1").unwrap().resume_kind, Some(ResumeKind::New));

    let ev = turn(&c, "run-2", &conv, "second question").await;
    assert_eq!(text_of(&ev), "ok 2", "{ev:?}");
    assert!(lab.sent_method(2, "thread/start").is_none());
    let resume = lab.sent_method(2, "thread/resume").expect("thread/resume");
    assert_eq!(resume["params"]["threadId"], "th-1");
    assert_eq!(
        turn_text(&lab.sent_method(2, "turn/start").unwrap()),
        "second question",
        "only the new message travels"
    );
    assert_eq!(
        reg.run("run-2").unwrap().resume_kind,
        Some(ResumeKind::Native)
    );
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}

/// Acceptance: a thread Codex no longer has (`no rollout found`) falls back
/// to `thread/start` with the replayed transcript, and the new id is kept.
#[tokio::test]
async fn a_thread_codex_lost_falls_back_to_a_fresh_thread_with_the_replay() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    let lab = Lab::new("fallback");
    let conv = conv_id("fallback");
    let ws = lab.workspace("ws");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws)).unwrap();
    let reg = lab.registry();
    let (c, _broker) = lab.coordinator(reg.clone());
    let _ = turn(&c, "run-1", &conv, "first question").await;
    std::fs::remove_file(lab.root.join("calls/threads/th-1.json")).unwrap();

    let ev = turn(&c, "run-2", &conv, "second question").await;
    assert_eq!(text_of(&ev), "ok 1", "a fresh thread answered: {ev:?}");
    assert!(lab.sent_method(2, "thread/resume").is_some());
    assert!(lab.sent_method(2, "thread/start").is_some(), "fallback");
    let text = turn_text(&lab.sent_method(2, "turn/start").unwrap());
    assert!(text.contains("User: first question"), "{text}");
    assert!(text.contains("User: second question"), "{text}");
    assert_eq!(
        reg.session(&conv, "codex-coder")
            .unwrap()
            .provider_handle
            .as_deref(),
        Some("th-2")
    );
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}

/// Acceptance: the agent's effort rides on `turn/start`.
#[tokio::test]
async fn the_effort_rides_on_turn_start() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    let lab = Lab::new("effort");
    let rt = lab.runtime(None, true);
    let req = TurnRequest {
        model: ModelRef {
            provider: ProviderId::new("cli"),
            model: "gpt-5.6-luna".into(),
        },
        messages: vec![Message::text(Role::User, "hi")],
        tools: vec![],
        params: GenParams {
            reasoning_effort: Some("high".into()),
            ..GenParams::default()
        },
        cancel: CancellationToken::new(),
        agent_id: None,
    };
    let conv = conv_id("effort");
    let ev: Vec<TurnEvent> =
        crate::core::llm::code_tools::scope(&conv, "codex-coder", "req-e", async {
            rt.turn(&agent(), req).await.unwrap().collect().await
        })
        .await;
    assert_eq!(text_of(&ev), "ok 1", "{ev:?}");
    let t = lab.sent_method(1, "turn/start").unwrap();
    assert_eq!(t["params"]["effort"], "high");
    assert_eq!(t["params"]["model"], "gpt-5.6-luna");
}

async fn pending_ask(reg: &Registry) -> crate::core::assist::runs::PermissionRequest {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(p) = reg
            .permissions_in(PermissionState::Pending, 10)
            .into_iter()
            .next()
        {
            return p;
        }
        assert!(Instant::now() < deadline, "no ask was opened");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Acceptance: Codex's `item/commandExecution/requestApproval` is one durable
/// ask (`runs_permissions`) of this turn, answered through the broker's single
/// door (`respond_in`, what `approvals.rs` serves to the window, the bridge,
/// the CLI, MCP and the phone); the answer reaches Codex as the JSON-RPC
/// response, and a "no" comes back to the turn with its reason.
#[tokio::test]
async fn an_approval_is_a_durable_ask_answerable_from_any_channel() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    for (decision, word, made) in [
        (Decision::Accept, "accept", true),
        (Decision::Decline, "decline", false),
    ] {
        let lab = Lab::new("approve");
        lab.mode("approve");
        let conv = conv_id("approve");
        let ws = lab.workspace("ws");
        crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws.clone())).unwrap();
        let reg = lab.registry();
        let (c, broker) = lab.coordinator(reg.clone());
        let c2 = c.clone();
        let conv2 = conv.clone();
        let task = tokio::spawn(async move { turn(&c2, "run-a", &conv2, "make a file").await });

        let ask = pending_ask(&reg).await;
        assert_eq!(ask.run_id, "run-a");
        assert_eq!(ask.action, "shell_exec");
        assert!(ask.preview.contains("touch made.txt"), "{}", ask.preview);
        assert_eq!(ask.agent_id, "codex-coder");
        // The turn waits for a person: the watchdog must not count it.
        assert!(broker.has_pending("run-a"));

        broker
            .respond_in(
                Some(&reg),
                &ask.id,
                decision,
                &Actor::new("http", "token:phone"),
            )
            .unwrap();
        let ev = tokio::time::timeout(Duration::from_secs(20), task)
            .await
            .expect("the turn ends after the answer")
            .unwrap();
        assert_eq!(lab.file("decision-1"), word);
        assert_eq!(ws.join("made.txt").exists(), made);
        let row = reg.permission(&ask.id).unwrap();
        assert_eq!(row.decided_by.as_deref(), Some("token:phone"));
        assert_eq!(row.channel.as_deref(), Some("http"));
        if !made {
            assert_eq!(row.state, PermissionState::Denied);
            assert!(
                ev.iter().all(|e| !matches!(e, TurnEvent::Error { .. })),
                "a refusal is not a failed turn: {ev:?}"
            );
        } else {
            assert_eq!(row.state, PermissionState::Approved);
        }
        crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
    }
}

/// Acceptance: cancel sends `turn/interrupt`, waits for Codex to close the
/// turn, and the stream ends in well under 2 s. A server that ignores the
/// interrupt is killed on the same budget.
#[tokio::test]
async fn cancel_interrupts_the_turn_in_under_two_seconds() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    for mode in ["hang", "deaf"] {
        let lab = Lab::new("cancel");
        lab.mode(mode);
        let rt = lab.runtime(None, true);
        let cancel = CancellationToken::new();
        let req = TurnRequest {
            model: ModelRef {
                provider: ProviderId::new("cli"),
                model: String::new(),
            },
            messages: vec![Message::text(Role::User, "work forever")],
            tools: vec![],
            params: GenParams::default(),
            cancel: cancel.clone(),
            agent_id: None,
        };
        let conv = conv_id("cancel");
        let (took, ev) =
            crate::core::llm::code_tools::scope(&conv, "codex-coder", "req-c", async {
                let mut stream = rt.turn(&agent(), req).await.unwrap();
                let mut ev = Vec::new();
                while let Some(e) = stream.next().await {
                    let working = matches!(&e, TurnEvent::TextDelta { .. });
                    ev.push(e);
                    if working {
                        break;
                    }
                }
                let t0 = Instant::now();
                cancel.cancel();
                while let Some(e) = stream.next().await {
                    ev.push(e);
                }
                (t0.elapsed(), ev)
            })
            .await;
        assert!(
            took < Duration::from_secs(2),
            "{mode}: cancel took {took:?}"
        );
        assert!(matches!(
            ev.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Cancelled
            })
        ));
        let interrupt = lab
            .sent_method(1, "turn/interrupt")
            .expect("turn/interrupt");
        assert_eq!(interrupt["params"]["turnId"], "tu-1");
        if mode == "hang" {
            assert!(
                lab.root.join("calls/interrupt-1").exists(),
                "the server saw the interrupt"
            );
        }
        bench_row(serde_json::json!({
            "kind": "cancel", "server": mode, "ms": took.as_millis() as u64,
        }));
    }
}

/// Acceptance (1.2): a silent app-server is a stalled turn like any CLI.
#[tokio::test]
async fn the_watchdog_stops_a_silent_app_server() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    let lab = Lab::new("stall");
    lab.mode("deaf");
    let rt = lab.runtime(None, true);
    rt.set_watchdog(WatchdogConfig {
        idle: Some(Duration::from_millis(900)),
        tool_idle: None,
        start: Some(Duration::from_secs(10)),
    });
    let req = TurnRequest {
        model: ModelRef {
            provider: ProviderId::new("cli"),
            model: String::new(),
        },
        messages: vec![Message::text(Role::User, "hang")],
        tools: vec![],
        params: GenParams::default(),
        cancel: CancellationToken::new(),
        agent_id: None,
    };
    let conv = conv_id("stall");
    let t0 = Instant::now();
    let ev: Vec<TurnEvent> =
        crate::core::llm::code_tools::scope(&conv, "codex-coder", "req-s", async {
            rt.turn(&agent(), req).await.unwrap().collect().await
        })
        .await;
    assert!(t0.elapsed() < Duration::from_secs(8));
    assert!(
        ev.iter()
            .any(|e| matches!(e, TurnEvent::Error { error } if error.code == ERR_CLI_STALLED)),
        "{ev:?}"
    );
}

/// Acceptance: without the app-server (older Codex, or forced) the turn is
/// the one-shot `exec --json` with the transcript on stdin, as before.
#[test]
fn an_old_codex_keeps_exec() {
    let lab = Lab::new("exec");
    let rt = lab.runtime(None, false);
    let account = rt.accounts().get("cx-1").unwrap();
    let req = TurnRequest {
        model: ModelRef {
            provider: ProviderId::new("cli"),
            model: String::new(),
        },
        messages: vec![Message::text(Role::User, "hi")],
        tools: vec![],
        params: GenParams::default(),
        cancel: CancellationToken::new(),
        agent_id: None,
    };
    let (spec, _) = rt.plan(&account, lab.fake.clone(), &req, None);
    assert_eq!(spec.args.first().map(String::as_str), Some("exec"));
    assert!(spec.args.iter().any(|a| a == "--json"));
    let caps = crate::core::llm::caps::cli_caps("codex", Some("0.155.0".into()), Vec::new());
    assert!(!caps.app_server);
}

fn bench_row(row: Value) {
    if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{row}");
        }
    }
}

/// BN10 (fake): a 10-turn conversation, app-server with resume against the
/// `exec` fallback that flattens the history. Measured on our side: the
/// characters OmniGet sends per turn (the prompt of `exec`'s stdin, the
/// `turn/start` text of the app-server) and the fake's usage for them.
#[tokio::test]
async fn bn10_turn_ten_input_stops_growing_with_resume() {
    if !available() {
        return eprintln!("python3 or the fake app-server missing: skipped");
    }
    let lab = Lab::new("bn10");
    let conv = conv_id("bn10");
    let ws = lab.workspace("ws");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws)).unwrap();
    let reg = lab.registry();
    let (c, _broker) = lab.coordinator(reg.clone());
    let exec_rt = lab.runtime(None, false);
    let account = exec_rt.accounts().get("cx-1").unwrap();
    let question = |i: usize| {
        format!(
            "Question {i}: summarise in two sentences why incremental builds matter for a Rust workspace with {i} crates."
        )
    };
    let mut history: Vec<Message> = Vec::new();
    for i in 1..=10 {
        let q = question(i);
        let ev = turn(&c, &format!("run-{i}"), &conv, &q).await;
        let sent = turn_text(&lab.sent_method(i, "turn/start").unwrap());
        let resumed = lab.sent_method(i, "thread/resume").is_some();
        let (input, cached) = ev
            .iter()
            .find_map(|e| match e {
                TurnEvent::Usage { usage } => Some((usage.input_tokens, usage.cache_read_tokens)),
                _ => None,
            })
            .unwrap_or_default();
        // The same conversation through `exec`: what its stdin would carry.
        history.push(Message::text(Role::User, &q));
        let req = TurnRequest {
            model: ModelRef {
                provider: ProviderId::new("cli"),
                model: String::new(),
            },
            messages: history.clone(),
            tools: vec![],
            params: GenParams::default(),
            cancel: CancellationToken::new(),
            agent_id: None,
        };
        let (spec, _) = exec_rt.plan(&account, lab.fake.clone(), &req, None);
        let exec_chars = spec.stdin.as_deref().map(str::len).unwrap_or(0);
        // The fake's usage model for a fresh thread per turn (what `exec` is).
        let exec_input = 1000 + exec_chars.div_ceil(4);
        bench_row(serde_json::json!({
            "kind": "turn", "turn": i, "resumed": resumed,
            "app_server": {"sent_chars": sent.len(), "input_tokens": input, "cached_tokens": cached,
                           "uncached_tokens": input.saturating_sub(cached)},
            "exec": {"sent_chars": exec_chars, "input_tokens": exec_input, "cached_tokens": 1000,
                     "uncached_tokens": exec_input - 1000},
        }));
        history.push(Message::text(Role::Assistant, text_of(&ev)));
        if i >= 2 {
            assert!(resumed, "turn {i} resumed");
            assert_eq!(sent, q, "turn {i} sends only the new message");
        }
    }
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}

/// The real Codex, logged in on the default profile (`~/.codex`): the probe
/// sees the app-server, two turns go through `thread/start` then
/// `thread/resume`, and whatever Codex answers (text, or its usage limit as
/// `ERR_CLI_RATE`) comes back as a clean turn. Costs one or two small turns.
/// `cargo test … live_codex_app_server -- --ignored --nocapture`
#[tokio::test]
#[ignore = "runs the installed codex against the real service"]
async fn live_codex_app_server_two_turns() {
    let Some(binary) = crate::core::dependencies::find_tool("codex").await else {
        return eprintln!("codex is not installed: skipped");
    };
    let caps = crate::core::llm::caps::probe_binary("codex", &binary).await;
    eprintln!("codex {:?} app_server={}", caps.version, caps.app_server);
    assert!(caps.app_server, "codex ≥ 0.156 answers `app-server --help`");
    let lab = Lab::new("live");
    let store = AccountStore::at(lab.root.join("accounts.json"));
    store
        .create(CliAccount {
            id: "cx-1".into(),
            cli: CliKind::Codex,
            config_dir: PathBuf::new(),
            label: "Codex (default profile)".into(),
            disabled: false,
            sandbox: SandboxMode::Write,
        })
        .unwrap();
    let bus = Arc::new(Bus::new());
    let broker = Arc::new(ToolBroker::new(vec![], Arc::new(NoTools), bus.clone()));
    let rt = CliRuntime::new(Arc::new(store), Arc::new(CliCapacity::new()))
        .with_broker(Arc::downgrade(&broker));
    rt.set_binary(CliKind::Codex, binary);
    rt.set_caps(CliKind::Codex, caps);
    let reg = lab.registry();
    let c = Arc::new(
        Coordinator::new(Arc::new(rt), broker, Arc::new(BudgetStore::memory()), bus)
            .with_dir(Some(lab.root.join("conversations")))
            .with_runs(reg.clone()),
    );
    let conv = conv_id("live");
    let ws = lab.workspace("ws");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws)).unwrap();
    let mut live = agent();
    live.model = ModelPolicy::Fixed {
        model: ModelRef {
            provider: ProviderId::new("cli"),
            model: std::env::var("OMNIGET_LIVE_CODEX_MODEL")
                .unwrap_or_else(|_| "gpt-5.6-luna".into()),
        },
    };
    for (i, q) in [
        "Reply with the single word: alpha",
        "Reply with the single word: beta",
    ]
    .iter()
    .enumerate()
    {
        let t0 = Instant::now();
        let ev: Vec<TurnEvent> = c
            .run_turn_with_id(
                format!("live-{i}"),
                &conv,
                &live,
                q,
                CancellationToken::new(),
            )
            .collect()
            .await;
        let errors: Vec<String> = ev
            .iter()
            .filter_map(|e| match e {
                TurnEvent::Error { error } => Some(format!("{}: {}", error.code, error.message)),
                _ => None,
            })
            .collect();
        eprintln!(
            "turn {i}: {:?} text={:?} errors={errors:?} resume={:?}",
            t0.elapsed(),
            text_of(&ev),
            reg.run(&format!("live-{i}")).and_then(|r| r.resume_kind)
        );
        assert!(
            errors.iter().all(|e| e.starts_with("ERR_CLI_RATE")),
            "only the account's own limit may stop it: {errors:?}"
        );
        assert!(!errors.iter().any(|e| e.contains("thread/resume failed")));
    }
    // Cancel a real turn one second in: the stream ends well under 2 s.
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        c2.cancel();
    });
    let t0 = Instant::now();
    let ev: Vec<TurnEvent> = c
        .run_turn_with_id(
            "live-cancel".into(),
            &conv,
            &live,
            "Count slowly from 1 to 500.",
            cancel,
        )
        .collect()
        .await;
    let took = t0.elapsed().saturating_sub(Duration::from_secs(1));
    eprintln!("cancel: {took:?} after the signal, last={:?}", ev.last());
    assert!(took < Duration::from_secs(2), "cancel took {took:?}");
    eprintln!(
        "handle={:?}",
        reg.session(&conv, "codex-coder")
            .and_then(|s| s.provider_handle)
    );
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}
