//! The durable run pipeline end to end — Coordinator → CliRuntime →
//! session → run registry — against a FAKE Claude Code: a shell script that
//! imitates `stream-json` (system/init with a session id, a text delta, a
//! result) and logs every invocation (argv, stdin, cwd) to files. No model is
//! involved and none of this is presented as one. What is exercised is our
//! real code: argv, resume decisions, session records, process groups,
//! reservations, reconciliation.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::core::assist::db::AssistDb;
use crate::core::assist::runs::{Registry, ResumeKind, RunState};
use crate::core::llm::agent::{AgentRole, Budget, ModelPolicy};
use crate::core::llm::broker::{ToolBroker, ToolExecutor};
use crate::core::llm::budget::BudgetStore;
use crate::core::llm::coordinator::Coordinator;
use crate::core::llm::types::{FinishReason, ModelRef, ProviderId};
use crate::core::omni::bus::Bus;

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
        Err(LlmError::new("ERR_TEST", format!("no tool {name}")))
    }
}

struct Lab {
    root: PathBuf,
    fake: PathBuf,
    db_path: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("omniget-durable-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("calls")).unwrap();
        std::fs::create_dir_all(root.join("max-1")).unwrap();
        let fake = root.join("fake-claude.sh");
        let calls = root.join("calls");
        let script = format!(
            r#"#!/bin/sh
# FAKE Claude Code for tests: imitates stream-json, logs each call.
input=$(cat)
dir="{calls}"
n=$(( $(cat "$dir/count" 2>/dev/null || echo 0) + 1 ))
echo $n > "$dir/count"
sid="sess-$n"
prev=""
for a in "$@"; do
  if [ "$prev" = "--resume" ]; then sid="$a"; fi
  prev="$a"
done
printf '%s\n' "$@" > "$dir/call-$n.args"
printf '%s' "$input" > "$dir/call-$n.stdin"
pwd -P > "$dir/call-$n.pwd"
echo '{{"type":"system","subtype":"init","session_id":"'$sid'"}}'
echo '{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"ok"}}}}}}'
if [ -f "$dir/hang" ]; then
  sleep 300 &
  echo $! > "$dir/grandchild.pid"
  echo $$ > "$dir/child.pid"
  wait
fi
echo '{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}'
"#,
            calls = calls.display()
        );
        std::fs::write(&fake, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let db_path = root.join("assist.db");
        Self {
            root,
            fake,
            db_path,
        }
    }

    fn registry(&self, instance: &str) -> Registry {
        Registry::with_instance(Arc::new(AssistDb::open(&self.db_path).unwrap()), instance)
    }

    fn calls(&self) -> usize {
        std::fs::read_to_string(self.root.join("calls/count"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    fn call_file(&self, n: usize, what: &str) -> String {
        std::fs::read_to_string(self.root.join(format!("calls/call-{n}.{what}")))
            .unwrap_or_default()
    }

    fn runtime(&self) -> Arc<CliRuntime> {
        let store = AccountStore::at(self.root.join("accounts.json"));
        if store.get("max-1").is_none() {
            store
                .create(CliAccount {
                    id: "max-1".into(),
                    cli: CliKind::Claude,
                    config_dir: self.root.join("max-1"),
                    label: "Max".into(),
                    disabled: false,
                    sandbox: SandboxMode::default(),
                })
                .unwrap();
        }
        let rt = CliRuntime::new(Arc::new(store), Arc::new(CliCapacity::new())).with_options(
            CliRuntimeOptions {
                sandbox_root: Some(self.root.join("sandboxes")),
                ..CliRuntimeOptions::default()
            },
        );
        rt.set_binary(CliKind::Claude, self.fake.clone());
        // What `claude --help` of 2.1.282 lists (see caps tests); the fake
        // has no help of its own.
        rt.set_caps(
            CliKind::Claude,
            crate::core::llm::caps::cli_caps(
                "claude",
                Some("2.1.282".into()),
                [
                    "--resume",
                    "--mcp-config",
                    "--tools",
                    "--disallowedTools",
                    "--include-partial-messages",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ),
        );
        Arc::new(rt)
    }

    fn coordinator(&self, reg: Registry, budget: Arc<BudgetStore>) -> Arc<Coordinator> {
        let bus = Arc::new(Bus::new());
        let broker = Arc::new(ToolBroker::new(vec![], Arc::new(NoTools), bus.clone()));
        Arc::new(
            Coordinator::new(self.runtime(), broker, budget, bus)
                .with_dir(Some(self.root.join("conversations")))
                .with_runs(reg),
        )
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
        id: "coder".into(),
        name: "Coder".into(),
        role: AgentRole::Worker,
        system_prompt: "be brief".into(),
        model: ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("cli"),
                model: "sonnet".into(),
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
            cli: "claude".into(),
            account: "max-1".into(),
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
        "durable-{tag}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

/// A08: the session identity is on disk; the second turn resumes natively
/// with the captured id and sends only the new message; reopening the app
/// (new registry instance, new coordinator) still resumes; a different folder
/// breaks the pin and the turn is marked as a replay.
#[tokio::test]
async fn a08_resume_by_captured_handle_and_replay_when_a_pin_changes() {
    let lab = Lab::new("a08");
    let conv = conv_id("a08");
    let ws1 = lab.workspace("ws1");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws1.clone())).unwrap();

    let reg = lab.registry("boot-1");
    let c = lab.coordinator(reg.clone(), Arc::new(BudgetStore::memory()));
    let ev = turn(&c, "run-1", &conv, "first question").await;
    assert!(ev
        .iter()
        .any(|e| matches!(e, TurnEvent::TextDelta { text } if text == "ok")));
    let r1 = reg.run("run-1").unwrap();
    assert_eq!(r1.state, RunState::Completed);
    assert_eq!(r1.resume_kind, Some(ResumeKind::New));
    assert!(!lab.call_file(1, "args").lines().any(|l| l == "--resume"));
    assert_eq!(lab.call_file(1, "pwd").trim(), ws1.display().to_string());
    let session = reg.session(&conv, "coder").unwrap();
    assert_eq!(session.provider_handle.as_deref(), Some("sess-1"));
    assert_eq!(session.cwd.as_deref(), Some(&*ws1.display().to_string()));
    assert_eq!(r1.pid.is_some(), true);
    assert!(r1.launch_id.is_some());

    let _ = turn(&c, "run-2", &conv, "second question").await;
    let args2 = lab.call_file(2, "args");
    let lines: Vec<&str> = args2.lines().collect();
    let at = lines
        .iter()
        .position(|l| *l == "--resume")
        .expect("--resume on turn 2");
    assert_eq!(lines[at + 1], "sess-1");
    // Only the new message travels on stdin.
    let stdin2 = lab.call_file(2, "stdin");
    assert_eq!(stdin2, "second question");
    assert_eq!(
        reg.run("run-2").unwrap().resume_kind,
        Some(ResumeKind::Native)
    );

    // "Close and reopen": new instance, new coordinator, same database.
    drop(c);
    let reg2 = lab.registry("boot-2");
    let c2 = lab.coordinator(reg2.clone(), Arc::new(BudgetStore::memory()));
    let _ = turn(&c2, "run-3", &conv, "third question").await;
    let args3 = lab.call_file(3, "args");
    assert!(args3.lines().any(|l| l == "sess-1"), "{args3}");
    assert_eq!(
        reg2.run("run-3").unwrap().resume_kind,
        Some(ResumeKind::Native)
    );

    // Another folder: the handle is not reused, the turn is a replay.
    let ws2 = lab.workspace("ws2");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws2.clone())).unwrap();
    let _ = turn(&c2, "run-4", &conv, "fourth question").await;
    let args4 = lab.call_file(4, "args");
    assert!(!args4.lines().any(|l| l == "--resume"), "{args4}");
    assert!(lab.call_file(4, "stdin").contains("User: first question"));
    assert_eq!(lab.call_file(4, "pwd").trim(), ws2.display().to_string());
    let r4 = reg2.run("run-4").unwrap();
    assert_eq!(r4.resume_kind, Some(ResumeKind::Replay));
    let s = reg2.session(&conv, "coder").unwrap();
    assert_eq!(s.generation, 2);
    assert_eq!(
        s.provider_handle.as_deref(),
        Some("sess-4"),
        "the new session's own id"
    );
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}

/// A personal conversation (no folder) runs in the bot's empty private
/// folder with only web tools, never in some other folder.
#[tokio::test]
async fn a_projectless_turn_runs_in_the_bots_sandbox_without_file_tools() {
    let lab = Lab::new("projectless");
    let conv = conv_id("personal");
    if crate::core::llm::code_tools::workspace_of(&conv).is_some() {
        // A process-wide folder is set by another test (legacy fallback that
        // W5 removes); the premise does not hold in this process.
        return;
    }
    let reg = lab.registry("boot-1");
    let c = lab.coordinator(reg.clone(), Arc::new(BudgetStore::memory()));
    let _ = turn(&c, "run-p", &conv, "hello").await;
    let pwd = lab.call_file(1, "pwd");
    let sandbox = lab
        .root
        .join("sandboxes")
        .join("coder")
        .canonicalize()
        .unwrap();
    assert_eq!(pwd.trim(), sandbox.display().to_string());
    assert_eq!(
        std::fs::read_dir(&sandbox).unwrap().count(),
        0,
        "the sandbox stays empty"
    );
    let args = lab.call_file(1, "args");
    let lines: Vec<&str> = args.lines().collect();
    let at = lines.iter().position(|l| *l == "--tools").expect("--tools");
    assert_eq!(lines[at + 1], "WebSearch,WebFetch");
    assert!(lines.iter().any(|l| l.starts_with("Bash,Edit,Write")));
    let s = reg.session(&conv, "coder").unwrap();
    assert_eq!(s.context_kind, "projectless");
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("condition never held");
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn read_pid(p: &Path) -> Option<i32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

/// A09: the app dies after the CLI was launched and before the turn was
/// acknowledged. The next boot finds the run `unknown`, re-sends nothing on
/// its own, and the test counts the external invocations: exactly one.
#[tokio::test]
async fn a09_a_crash_after_dispatch_leaves_the_run_unknown_and_nothing_is_resent() {
    let lab = Lab::new("a09");
    std::fs::write(lab.root.join("calls/hang"), "").unwrap();
    let conv = conv_id("a09");
    let ws = lab.workspace("ws");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws)).unwrap();

    let old = lab.registry("boot-old");
    let c = lab.coordinator(old.clone(), Arc::new(BudgetStore::memory()));
    let mut stream = c.run_turn_with_id(
        "run-crash".into(),
        &conv,
        &agent(),
        "change the file",
        CancellationToken::new(),
    );
    // Dispatched: the CLI started and spoke.
    let _ = stream.next().await;
    wait_for(|| lab.calls() == 1 && lab.root.join("calls/child.pid").exists()).await;
    assert_eq!(old.run("run-crash").unwrap().state, RunState::Running);
    // "Crash": the old process's memory is gone; its turn is never acked.
    std::mem::forget(stream);

    // Next boot.
    let fresh = lab.registry("boot-new");
    let out = fresh.reconcile().unwrap();
    assert_eq!(out.unknown, vec!["run-crash".to_string()]);
    let run = fresh.run("run-crash").unwrap();
    assert_eq!(run.state, RunState::Unknown);
    assert!(
        run.launch_id.is_some(),
        "the launch is identified beyond the pid"
    );
    // Nothing runs again by itself — not at boot, not a moment later.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(lab.calls(), 1, "the external process ran exactly once");
    // unknown never goes back to queued; a person decides.
    assert!(fresh
        .transition("run-crash", RunState::Queued, None)
        .is_err());
    fresh
        .resolve_run(
            "run-crash",
            crate::core::assist::runs::RunResolution::MarkedDone,
        )
        .unwrap();
    assert_eq!(lab.calls(), 1);

    // Clean the leftover fake (the crashed app would not have).
    if let Some(pid) = read_pid(&lab.root.join("calls/child.pid")) {
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}

/// A12 through the coordinator: cancel stops the CLI and its grandchild,
/// spares a foreign process, releases the reservation, records `cancelled`
/// and never a completion.
#[tokio::test]
async fn a12_cancel_stops_our_tree_releases_the_reservation_and_never_completes() {
    let lab = Lab::new("a12");
    std::fs::write(lab.root.join("calls/hang"), "").unwrap();
    let conv = conv_id("a12");
    let ws = lab.workspace("ws");
    crate::core::llm::code_tools::set_conversation_workspace(&conv, Some(ws)).unwrap();
    let mut foreign = std::process::Command::new("sleep")
        .arg("120")
        .spawn()
        .unwrap();
    let foreign_pid = foreign.id() as i32;

    let reg = lab.registry("boot-1");
    let budget = Arc::new(BudgetStore::memory());
    let c = lab.coordinator(reg.clone(), budget.clone());
    let cancel = CancellationToken::new();
    let mut stream =
        c.run_turn_with_id("run-x".into(), &conv, &agent(), "long work", cancel.clone());
    let _ = stream.next().await;
    wait_for(|| lab.root.join("calls/grandchild.pid").exists()).await;
    let grandchild = read_pid(&lab.root.join("calls/grandchild.pid")).unwrap();
    let child = read_pid(&lab.root.join("calls/child.pid")).unwrap();
    assert!(alive(grandchild) && alive(child));
    assert_eq!(budget.in_flight().len(), 1, "the turn holds a reservation");

    cancel.cancel();
    let rest: Vec<TurnEvent> = stream.collect().await;
    assert!(matches!(
        rest.last(),
        Some(TurnEvent::Finished {
            reason: FinishReason::Cancelled
        })
    ));
    wait_for(|| !alive(grandchild) && !alive(child)).await;
    assert!(alive(foreign_pid), "a process that is not ours survives");
    assert!(
        budget.in_flight().is_empty(),
        "cancel releases the reservation"
    );
    let run = reg.run("run-x").unwrap();
    assert_eq!(run.state, RunState::Cancelled);
    assert!(!reg
        .events("run-x")
        .iter()
        .any(|e| e.kind == "state" && e.payload["state"] == "completed"));
    let _ = foreign.kill();
    let _ = foreign.wait();
    crate::core::llm::code_tools::set_conversation_workspace(&conv, None).unwrap();
}
