//! The translator against real stdout of Claude Code 2.1.280 (recorded
//! 22/09/2026 with `--model haiku` in a temporary directory, paths and the
//! account e-mail scrubbed). Each fixture is one long-lived process.

use std::path::Path;

use serde_json::{json, Value};

use super::cursor::{ClaudeCursor, TurnMark};
use super::protocol;
use super::translate::{Output, Translator};
use crate::core::llm::drivers::{
    ApprovalDecision, ItemStatus, ItemType, RequestType, RuntimeEvent, RuntimeEventKind as K,
    StreamKind, TurnEndState,
};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("core")
        .join("llm")
        .join("drivers")
        .join("claude")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct Replay {
    tr: Translator,
    events: Vec<RuntimeEvent>,
    sends: Vec<Value>,
}

impl Replay {
    fn new(cursor: ClaudeCursor, prompts: &[(&str, &str)]) -> Self {
        let mut tr = Translator::new(
            "claude",
            "thr_1",
            Some("/tmp/omniget-fixture/r1".into()),
            cursor,
        );
        let mut events = Vec::new();
        for (turn, uuid) in prompts {
            events.extend(tr.register_prompt(turn, uuid, None));
        }
        Replay {
            tr,
            events,
            sends: Vec::new(),
        }
    }

    /// Feeds lines; `on_request` answers approvals/questions as they open.
    fn run(
        &mut self,
        name: &str,
        mut on_request: impl FnMut(&mut Translator, &RuntimeEvent) -> Option<(Value, Vec<RuntimeEvent>)>,
    ) {
        let text = fixture(name);
        for line in text.lines() {
            // Files do not exist in the test: a Write is a new file.
            for out in self.tr.on_line(line, &|_| None) {
                match out {
                    Output::Event(ev) => {
                        let answer = match &ev.kind {
                            K::RequestOpened(_) | K::UserInputRequested(_) => {
                                on_request(&mut self.tr, &ev)
                            }
                            _ => None,
                        };
                        self.events.push(ev);
                        if let Some((line, evs)) = answer {
                            self.sends.push(line);
                            self.events.extend(evs);
                        }
                    }
                    Output::Send(v) => self.sends.push(v),
                    Output::ControlResult { .. } => {}
                }
            }
        }
    }

    fn of_turn(&self, turn: &str) -> Vec<&RuntimeEvent> {
        self.events
            .iter()
            .filter(|e| e.turn_id.as_deref() == Some(turn))
            .collect()
    }

    fn text(&self, turn: Option<&str>) -> String {
        self.events
            .iter()
            .filter(|e| e.turn_id.as_deref() == turn)
            .filter_map(|e| match &e.kind {
                K::ContentDelta(p) if p.stream_kind == StreamKind::AssistantText => {
                    Some(p.delta.as_str())
                }
                _ => None,
            })
            .collect()
    }

    fn completed(&self, turn: &str) -> Vec<&crate::core::llm::drivers::TurnCompletedPayload> {
        self.of_turn(turn)
            .into_iter()
            .filter_map(|e| match &e.kind {
                K::TurnCompleted(p) => Some(p),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn approval_allow_for_session_then_edit_in_accept_edits() {
    let mut r = Replay::new(ClaudeCursor::default(), &[("t1", "1"), ("t2", "2")]);
    let mut opened = Vec::new();
    r.run("claude-2.1.280-approve-edit.jsonl", |tr, ev| {
        opened.push(ev.clone());
        Some(
            tr.answer_approval(
                ev.request_id.as_deref().unwrap(),
                ApprovalDecision::AcceptForSession,
            )
            .unwrap(),
        )
    });
    // Turn 1 starts at once (nothing else running); turn 2 when the CLI says.
    assert!(matches!(r.events[0].kind, K::TurnStarted(_)));
    assert_eq!(r.events[0].turn_id.as_deref(), Some("t1"));

    // Exactly one approval: the Write. The session permission it granted
    // (setMode acceptEdits) let the Edit of turn 2 through without asking.
    assert_eq!(opened.len(), 1);
    let K::RequestOpened(p) = &opened[0].kind else {
        unreachable!()
    };
    assert_eq!(p.request_type, RequestType::FileChangeApproval);
    assert_eq!(opened[0].turn_id.as_deref(), Some("t1"));
    let detail = p.detail.as_ref().unwrap();
    assert_eq!(detail.tool_name.as_deref(), Some("Write"));
    assert_eq!(
        detail.paths,
        vec!["/tmp/omniget-fixture/r1/a.txt".to_string()]
    );
    let diff = detail.diff.as_deref().unwrap();
    assert!(
        diff.starts_with("--- /dev/null\n+++ b/tmp/omniget-fixture/r1/a.txt\n"),
        "{diff}"
    );
    assert!(diff.ends_with("+oi\n"), "{diff}");
    assert_eq!(p.options.len(), 5);
    assert_eq!(
        opened[0]
            .provider_refs
            .as_ref()
            .unwrap()
            .provider_item_id
            .as_deref(),
        Some("toolu_01X11NSByopRmUSof7AXVzgU")
    );

    // The answer is the one the live run sent (and the CLI accepted).
    assert_eq!(
        r.sends[0],
        protocol::control_success(
            opened[0].request_id.as_deref().unwrap(),
            json!({
                "behavior": "allow",
                "updatedInput": {"file_path": "/tmp/omniget-fixture/r1/a.txt", "content": "oi"},
                "updatedPermissions": [{"type": "setMode", "mode": "acceptEdits", "destination": "session"}]
            })
        )
    );
    assert!(r.events.iter().any(|e| matches!(&e.kind, K::RequestResolved(p) if p.decision == Some(ApprovalDecision::AcceptForSession))));
    assert!(r.events.iter().any(
        |e| matches!(&e.kind, K::SessionConfigured(v) if v.value["permissionMode"] == "acceptEdits")
    ));

    // Tool items: started → updated (input) → completed with the output.
    let write: Vec<&RuntimeEvent> = r
        .events
        .iter()
        .filter(|e| e.item_id.as_deref() == Some("toolu_01X11NSByopRmUSof7AXVzgU"))
        .collect();
    assert!(matches!(&write[0].kind, K::ItemStarted(p) if p.item_type == ItemType::FileChange));
    assert!(
        matches!(&write.last().unwrap().kind, K::ItemCompleted(p) if p.status == Some(ItemStatus::Completed))
    );
    let edit_done = r
        .events
        .iter()
        .find_map(|e| match &e.kind {
            K::ItemCompleted(p)
                if e.item_id.as_deref() == Some("toolu_01WPiQFxkuCkWwBY4H6sM73M") =>
            {
                Some(p.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        edit_done.data.as_ref().unwrap()["input"]["new_string"],
        "ola"
    );
    assert!(edit_done.data.as_ref().unwrap()["result"]["structuredPatch"].is_array());
    assert_eq!(
        r.of_turn("t2")
            .iter()
            .filter(|e| matches!(e.kind, K::ItemStarted(_)))
            .count(),
        1
    );

    // Text, usage, cost per turn (cumulative total → delta).
    assert_eq!(
        r.text(Some("t1")),
        "Done\u{2014}created a.txt with the text \"oi\"."
    );
    assert_eq!(r.text(Some("t2")), "Changed a.txt from 'oi' to 'ola'.");
    let c1 = r.completed("t1");
    assert_eq!(c1.len(), 1);
    assert_eq!(c1[0].state, TurnEndState::Completed);
    let u1 = c1[0].usage.as_ref().unwrap();
    assert_eq!(
        (u1.input_tokens, u1.output_tokens, u1.cached_input_tokens),
        (18, 376, 38_133)
    );
    assert_eq!(u1.reasoning_output_tokens, 223);
    assert_eq!(u1.max_tokens, Some(200_000));
    assert!((c1[0].total_cost_usd.unwrap() - 0.0267513).abs() < 1e-9);
    let c2 = r.completed("t2");
    assert!((c2[0].total_cost_usd.unwrap() - (0.0346718 - 0.0267513)).abs() < 1e-9);

    // Resume cursor: session id and the boundary of each turn.
    let cursor = r.tr.cursor.clone();
    assert_eq!(cursor.session_id, "a00fe044-a480-48c4-ac8b-b3d6bdd759ed");
    assert_eq!(cursor.turns.len(), 2);
    assert_eq!(
        cursor.turns[0].last_uuid.as_deref(),
        Some("bef506b5-1936-436c-807f-a1286c3f12ce")
    );
    assert_eq!(
        cursor.turns[1].last_uuid.as_deref(),
        Some("70661c9e-e6bf-484b-91e9-ae66f60b5b34")
    );
    let last_started = r
        .events
        .iter()
        .rev()
        .find_map(|e| match &e.kind {
            K::SessionStarted(p) => p.resume.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(ClaudeCursor::from_value(&last_started), Some(cursor));

    // The opt-in get_usage answer (0..100) of the live run.
    let usage = r
        .events
        .iter()
        .find_map(|e| match &e.kind {
            K::RateLimitsUpdated(p)
                if p.credits
                    .as_ref()
                    .map_or(false, |c| c.get("subscriptionType").is_some()) =>
            {
                Some(p.windows.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!((usage[0].used_percent.unwrap() - 28.0).abs() < 1e-6);
    // Quota from rate_limit_event, 0..1 → percent.
    let windows = r
        .events
        .iter()
        .find_map(|e| match &e.kind {
            K::RateLimitsUpdated(p)
                if p.credits
                    .as_ref()
                    .map_or(false, |c| c.get("status").is_some()) =>
            {
                Some(p.windows.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(windows[0].id, "five_hour");
    assert!((windows[0].used_percent.unwrap() - 27.0).abs() < 1e-6);
    assert_eq!(
        windows[0].resets_at.as_deref(),
        Some("2026-09-23T02:40:00Z")
    );
    assert_eq!(windows[1].id, "seven_day");

    // initialize → configured; the account e-mail never leaves the driver.
    assert!(r.events.iter().any(|e| matches!(&e.kind, K::SessionConfigured(v) if v.value["subscriptionType"] == "Claude Max")));
    let all = serde_json::to_string(&r.events).unwrap();
    assert!(!all.contains("example.com"));
}

#[test]
fn ask_user_question_answer_and_a_declined_write() {
    let mut r = Replay::new(ClaudeCursor::default(), &[("t1", "1")]);
    let mut asked = None;
    r.run("claude-2.1.280-question-deny.jsonl", |tr, ev| {
        let id = ev.request_id.clone().unwrap();
        match &ev.kind {
            K::UserInputRequested(p) => {
                asked = Some(p.clone());
                let q = &p.questions[0];
                Some(
                    tr.answer_questions(&id, &json!({ q.id.clone(): "Red" }))
                        .unwrap(),
                )
            }
            _ => Some(tr.answer_approval(&id, ApprovalDecision::Decline).unwrap()),
        }
    });
    let asked = asked.expect("AskUserQuestion must become user-input.requested");
    let q = &asked.questions[0];
    assert_eq!(q.id, "Which color do you prefer?");
    assert_eq!(q.header, "Color");
    assert_eq!(
        q.options
            .iter()
            .map(|o| o.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Red", "Blue"]
    );
    assert!(!q.multi_select);
    // Same answer the live run sent; the CLI echoed it in the tool result.
    assert_eq!(
        r.sends[0]["response"]["response"]["updatedInput"]["answers"],
        json!({"Which color do you prefer?": "Red"})
    );
    assert!(r.events.iter().any(|e| matches!(&e.kind, K::UserInputResolved(p) if p.answers["Which color do you prefer?"] == "Red")));
    // No approval card for the question itself.
    let approvals: Vec<&RuntimeEvent> = r
        .events
        .iter()
        .filter(|e| matches!(e.kind, K::RequestOpened(_)))
        .collect();
    assert_eq!(approvals.len(), 1);
    assert_eq!(
        r.sends[1]["response"]["response"],
        json!({"behavior": "deny", "message": protocol::DECLINE_MESSAGE})
    );
    // The declined Write fails; the turn itself completes.
    assert!(r.events.iter().any(|e| e.item_id.as_deref()
        == Some("toolu_0153q3sPFzoAn5KUs9kBSSvT")
        && matches!(&e.kind, K::ItemCompleted(p) if p.status == Some(ItemStatus::Failed))));
    assert_eq!(r.completed("t1")[0].state, TurnEndState::Completed);
}

#[test]
fn exit_plan_mode_is_captured_once_and_denied() {
    let mut r = Replay::new(ClaudeCursor::default(), &[("t1", "1")]);
    r.run("claude-2.1.280-plan-exit.jsonl", |_, _| {
        panic!("no approval expected in plan mode")
    });
    let plans: Vec<String> = r
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            K::ProposedCompleted(p) => Some(p.plan_markdown.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        plans.len(),
        1,
        "stream, snapshot and can_use_tool carry the same plan"
    );
    assert!(plans[0].contains("Create `b.txt`"), "{}", plans[0]);
    assert_eq!(r.sends.len(), 1);
    assert_eq!(r.sends[0]["response"]["response"]["behavior"], "deny");
    assert_eq!(
        r.sends[0]["response"]["response"]["message"],
        protocol::PLAN_CAPTURED_MESSAGE
    );
    assert_eq!(r.completed("t1")[0].state, TurnEndState::Completed);
}

#[test]
fn an_interrupt_ends_the_turn_and_the_process_takes_the_next_prompt() {
    let mut r = Replay::new(ClaudeCursor::default(), &[("t1", "1"), ("t2", "2")]);
    r.tr.mark_interrupt_requested();
    r.run("claude-2.1.280-interrupt.jsonl", |_, _| None);
    let c1 = r.completed("t1");
    assert_eq!(c1.len(), 1);
    assert_eq!(c1[0].state, TurnEndState::Interrupted);
    let c2 = r.completed("t2");
    assert_eq!(c2[0].state, TurnEndState::Completed);
    assert_eq!(r.text(Some("t2")), "ok");
    // The interrupted result reports 0 total: the next turn's cost is its own.
    assert!((c2[0].total_cost_usd.unwrap() - 0.0055289).abs() < 1e-9);
    // No error row for a user interrupt.
    assert!(!r
        .events
        .iter()
        .any(|e| matches!(e.kind, K::RuntimeError(_))));
}

#[test]
fn resume_at_a_boundary_with_fork_reports_the_new_session() {
    let old = ClaudeCursor {
        session_id: "a00fe044-a480-48c4-ac8b-b3d6bdd759ed".into(),
        turns: vec![TurnMark {
            turn_id: "t1".into(),
            prompt_uuid: "1".into(),
            last_uuid: Some("bef506b5-1936-436c-807f-a1286c3f12ce".into()),
        }],
        resume_at: Some("bef506b5-1936-436c-807f-a1286c3f12ce".into()),
        fork: true,
    };
    let mut r = Replay::new(old, &[("t2", "5e0c1a2b-0000-4000-8000-000000000001")]);
    r.run("claude-2.1.280-resume-fork.jsonl", |_, _| None);
    let c = &r.tr.cursor;
    assert_eq!(c.session_id, "9ac64eb7-f69d-404a-ba76-57883eaaeb9d");
    assert_eq!(c.resume_at, None);
    assert!(!c.fork);
    assert_eq!(c.turns.len(), 2);
    assert!(c.turns[1].last_uuid.is_some());
    // The truncated history answered about turn 1 only.
    assert!(
        r.text(Some("t2")).contains("a.txt"),
        "{}",
        r.text(Some("t2"))
    );
    assert_eq!(r.completed("t2")[0].state, TurnEndState::Completed);
}

#[test]
fn a_background_subagent_becomes_task_events_and_a_loose_reply() {
    let mut r = Replay::new(ClaudeCursor::default(), &[("t1", "1")]);
    r.run("claude-2.1.280-subagent.jsonl", |_, _| None);
    let task = "ad2627fcde126c402";
    let kinds: Vec<&str> = r
        .events
        .iter()
        .filter(|e| match &e.kind {
            K::TaskStarted(p) | K::TaskProgress(p) | K::TaskUpdated(p) | K::TaskCompleted(p) => {
                p.task_id == task
            }
            _ => false,
        })
        .map(|e| e.type_name())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "task.started",
            "task.progress",
            "task.updated",
            "task.completed"
        ]
    );
    // Every task event stays on the turn that launched it.
    assert!(r
        .events
        .iter()
        .filter(|e| e.type_name().starts_with("task."))
        .all(|e| e.turn_id.as_deref() == Some("t1")));
    let done = r
        .events
        .iter()
        .find_map(|e| match &e.kind {
            K::TaskCompleted(p) => Some(p.clone()),
            _ => None,
        })
        .unwrap();
    assert!(done.summary.unwrap().contains("hello"));
    assert_eq!(
        done.tool_use_id.as_deref(),
        Some("toolu_01SjxTnshYKbjXAc1MeSAaFd")
    );

    // The sub-agent's Read is an item tagged with the agent, not main text.
    let read = r
        .events
        .iter()
        .find_map(|e| match &e.kind {
            K::ItemStarted(p) if p.tool_name.as_deref() == Some("Read") => Some(p.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(read.agent_id.as_deref(), Some(task));
    assert_eq!(
        read.parent_tool_use_id.as_deref(),
        Some("toolu_01SjxTnshYKbjXAc1MeSAaFd")
    );
    assert!(!r.text(Some("t1")).contains("complete content"));

    // Bash: a command item with its output.
    let bash = r
        .events
        .iter()
        .filter(|e| e.item_id.as_deref() == Some("toolu_01HbGk6dsCzRxq83Vut8B4Kz"))
        .collect::<Vec<_>>();
    assert!(bash.iter().any(|e| matches!(&e.kind, K::ContentDelta(p) if p.stream_kind == StreamKind::CommandOutput && p.delta == "hi")));
    assert!(bash.iter().any(|e| matches!(&e.kind, K::ItemCompleted(p) if p.item_type == ItemType::CommandExecution && p.status == Some(ItemStatus::Completed))));
    // The Agent tool (background) is not failed when the turn ends.
    assert!(!r.events.iter().any(|e| e.item_id.as_deref()
        == Some("toolu_01SjxTnshYKbjXAc1MeSAaFd")
        && matches!(&e.kind, K::ItemCompleted(p) if p.status == Some(ItemStatus::Failed))));

    // One turn, closed once; the late reply is loose (no turn id).
    assert_eq!(r.completed("t1").len(), 1);
    assert!(r.text(None).contains("hello"), "{}", r.text(None));
    // Its boundary still moves the cursor forward.
    assert!(r.tr.cursor.turns[0].last_uuid.is_some());
}

#[test]
fn an_unknown_control_request_is_answered_with_an_error() {
    let mut tr = Translator::new("claude", "thr", None, ClaudeCursor::default());
    let out = tr.on_line(
        r#"{"type":"control_request","request_id":"r9","request":{"subtype":"hook_callback","callback_id":"x"}}"#,
        &|_| None,
    );
    assert!(
        matches!(&out[0], Output::Send(v) if v["response"]["subtype"] == "error" && v["response"]["request_id"] == "r9")
    );
    // A withdrawn approval resolves as cancelled.
    tr.register_prompt("t1", "u1", None);
    let out = tr.on_line(
        r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf build","description":"clean"},"tool_use_id":"toolu_x"}}"#,
        &|_| None,
    );
    let opened = out
        .iter()
        .find_map(|o| match o {
            Output::Event(e) => match &e.kind {
                K::RequestOpened(p) => Some(p.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(opened.request_type, RequestType::CommandExecutionApproval);
    assert_eq!(
        opened.detail.as_ref().unwrap().command.as_deref(),
        Some("rm -rf build")
    );
    assert_eq!(
        opened.detail.as_ref().unwrap().reason.as_deref(),
        Some("clean")
    );
    let out = tr.on_line(
        r#"{"type":"control_cancel_request","request_id":"r1"}"#,
        &|_| None,
    );
    assert!(
        matches!(&out[0], Output::Event(e) if matches!(&e.kind, K::RequestResolved(p) if p.resolution.as_deref() == Some("cancelled")))
    );
    assert!(tr.answer_approval("r1", ApprovalDecision::Accept).is_err());
}

#[test]
fn an_edit_approval_carries_the_diff_against_the_file_on_disk() {
    let mut tr = Translator::new("claude", "thr", Some("/w".into()), ClaudeCursor::default());
    tr.register_prompt("t1", "u1", None);
    let out = tr.on_line(
        r#"{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Edit","input":{"file_path":"src/a.rs","old_string":"let x = 1;","new_string":"let x = 2;"},"tool_use_id":"toolu_e"}}"#,
        &|p| (p == Path::new("/w/src/a.rs")).then(|| "fn main() {\n    let x = 1;\n}\n".to_string()),
    );
    let detail = out
        .iter()
        .find_map(|o| match o {
            Output::Event(e) => match &e.kind {
                K::RequestOpened(p) => p.detail.clone(),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(detail.paths, vec!["/w/src/a.rs".to_string()]);
    let diff = detail.diff.unwrap();
    assert!(
        diff.contains("-    let x = 1;\n+    let x = 2;\n"),
        "{diff}"
    );
    assert!(diff.contains(" fn main() {\n"), "{diff}");
    // cancel = deny + interrupt.
    let (line, _) = tr.answer_approval("r2", ApprovalDecision::Cancel).unwrap();
    assert_eq!(line["response"]["response"]["interrupt"], true);
}

// ── the driver end to end over a fake CLI ───────────────────────────────

#[cfg(unix)]
mod driver_e2e {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::core::llm::drivers::claude::cursor::CursorStore;
    use crate::core::llm::drivers::claude::ClaudeDriver;
    use crate::core::llm::drivers::{
        AccessMode, Driver, DriverInstance, InteractionMode, SessionStart, TurnStart,
    };

    fn tmp(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("omniget-claude-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fake `claude` that logs its argv and stdin, answers the first user
    /// message with a recorded turn, then waits for EOF.
    fn fake_cli(dir: &Path, fixture_name: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/core/llm/drivers/claude/fixtures")
            .join(fixture_name);
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" > '{argv}'\necho \"CFG=$CLAUDE_CONFIG_DIR NEST=${{CLAUDECODE:-none}}\" > '{env}'\nsent=0\nwhile IFS= read -r line; do\n  echo \"$line\" >> '{stdin}'\n  case \"$line\" in\n    *'\"type\":\"user\"'*) if [ $sent = 0 ]; then cat '{fx}'; sent=1; fi ;;\n  esac\ndone\n",
                argv = dir.join("argv.txt").display(),
                env = dir.join("env.txt").display(),
                stdin = dir.join("stdin.jsonl").display(),
                fx = fixture.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[tokio::test]
    async fn a_turn_runs_through_the_process_and_stop_ends_it() {
        let dir = tmp("turn");
        let script = fake_cli(&dir, "claude-2.1.280-interrupt.jsonl");
        let mut inst = DriverInstance::new("claude-max-1", "claude", "Claude");
        inst.command = Some(script.display().to_string());
        inst.config_dir = Some(dir.join("profile"));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let store = Arc::new(CursorStore::at(Some(dir.join("claude-threads.json"))));
        let driver = ClaudeDriver::with_store(inst, tx, store.clone());
        std::env::set_var("CLAUDECODE", "1");
        driver
            .start_session(SessionStart {
                thread_id: "thr_e2e".into(),
                instance_id: "claude-max-1".into(),
                cwd: Some(dir.clone()),
                model: Some("haiku".into()),
                agent_id: None,
                access_mode: AccessMode::ApprovalRequired,
                interaction_mode: InteractionMode::Default,
                resume_cursor: None,
            })
            .await
            .unwrap();
        driver
            .start_turn(TurnStart {
                thread_id: "thr_e2e".into(),
                turn_id: "turn-1".into(),
                message_id: "m1".into(),
                text: "Write a 400-word story about a cat. No tools.".into(),
                attachments: vec![],
                model: None,
                access_mode: AccessMode::ApprovalRequired,
                interaction_mode: InteractionMode::Default,
            })
            .await
            .unwrap();
        // The recorded process interrupted turn 1 and ran a second prompt;
        // our single turn is closed by the first result.
        let mut events = Vec::new();
        let done = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(ev) = rx.recv().await {
                let end = matches!(ev.kind, K::TurnCompleted(_))
                    && ev.turn_id.as_deref() == Some("turn-1");
                events.push(ev);
                if end {
                    break;
                }
            }
        })
        .await;
        assert!(
            done.is_ok(),
            "no turn.completed: {:?}",
            events.iter().map(|e| e.type_name()).collect::<Vec<_>>()
        );
        assert!(matches!(events[0].kind, K::TurnStarted(_)));
        assert!(events
            .iter()
            .all(|e| e.thread_id == "thr_e2e" && e.instance_id == "claude-max-1"));

        driver.stop("thr_e2e").await.unwrap();
        std::env::remove_var("CLAUDECODE");
        let argv = std::fs::read_to_string(dir.join("argv.txt")).unwrap();
        assert!(argv.contains("--permission-prompt-tool stdio"), "{argv}");
        assert!(argv.contains("--permission-mode default"), "{argv}");
        assert!(argv.contains("--model haiku"), "{argv}");
        let env = std::fs::read_to_string(dir.join("env.txt")).unwrap();
        assert!(
            env.contains(&format!("CFG={}", dir.join("profile").display())),
            "{env}"
        );
        assert!(
            env.contains("NEST=none"),
            "nesting marker must be scrubbed: {env}"
        );
        let stdin = std::fs::read_to_string(dir.join("stdin.jsonl")).unwrap();
        let lines: Vec<Value> = stdin
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["request"]["subtype"], "initialize");
        assert_eq!(lines[1]["type"], "user");
        assert_eq!(
            lines[1]["message"]["content"][0]["text"],
            "Write a 400-word story about a cat. No tools."
        );
        // The session id reached the cursor file.
        assert_eq!(
            store.get("thr_e2e").unwrap().session_id,
            "e67cfc11-6cec-4138-9770-dada33dd2794"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live run against the installed CLI (costs one tiny haiku turn on the
    /// default profile). `OMNIGET_TEST_CLAUDE=1 cargo test -p omniget-core
    /// --lib drivers::claude::tests::driver_e2e::live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "spawns the installed claude; set OMNIGET_TEST_CLAUDE=1"]
    async fn live_turn_approval_and_resume() {
        if std::env::var("OMNIGET_TEST_CLAUDE").ok().as_deref() != Some("1") {
            return;
        }
        let dir = tmp("live");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let store = Arc::new(CursorStore::at(Some(dir.join("claude-threads.json"))));
        let driver = ClaudeDriver::with_store(
            DriverInstance::new("claude", "claude", "Claude"),
            tx,
            store.clone(),
        );
        let start = |turn: &str, text: &str| TurnStart {
            thread_id: "thr_live".into(),
            turn_id: turn.into(),
            message_id: format!("m-{turn}"),
            text: text.into(),
            attachments: vec![],
            model: Some("haiku".into()),
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
        };
        driver
            .start_session(SessionStart {
                thread_id: "thr_live".into(),
                instance_id: "claude".into(),
                cwd: Some(dir.clone()),
                model: Some("haiku".into()),
                agent_id: None,
                access_mode: AccessMode::ApprovalRequired,
                interaction_mode: InteractionMode::Default,
                resume_cursor: None,
            })
            .await
            .unwrap();
        driver
            .start_turn(start(
                "t1",
                "Create x.txt containing exactly: oi (Write tool). Then reply: done",
            ))
            .await
            .unwrap();
        let mut log = Vec::new();
        let ok = tokio::time::timeout(Duration::from_secs(120), async {
            while let Some(ev) = rx.recv().await {
                eprintln!("{}", serde_json::to_string(&ev).unwrap());
                if let K::RequestOpened(_) = &ev.kind {
                    driver
                        .respond_request(
                            "thr_live",
                            ev.request_id.as_deref().unwrap(),
                            ApprovalDecision::Accept,
                        )
                        .await
                        .unwrap();
                }
                let end = matches!(ev.kind, K::TurnCompleted(_));
                log.push(ev);
                if end {
                    break;
                }
            }
        })
        .await;
        assert!(ok.is_ok());
        assert_eq!(
            std::fs::read_to_string(dir.join("x.txt")).unwrap_or_default(),
            "oi"
        );
        assert!(log.iter().any(|e| matches!(&e.kind, K::RequestOpened(p) if p.detail.as_ref().and_then(|d| d.diff.as_ref()).is_some())));
        let sid = store.get("thr_live").unwrap().session_id;
        assert!(!sid.is_empty());
        driver.stop("thr_live").await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
