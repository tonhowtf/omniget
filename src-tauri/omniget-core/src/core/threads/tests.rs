//! Decider and projector tests, plus the acceptance test: a simulated turn
//! (delta, tool, pending approval) survives closing and reopening the
//! database with the same snapshot.

use serde_json::json;

use super::decider::*;
use super::engine::ThreadsEngine;
use super::model::*;
use crate::core::llm::drivers::*;

const NOW: &str = "2026-09-22T12:00:00.000Z";

fn ctx() -> DecideCtx<'static> {
    DecideCtx {
        now: NOW,
        fork: None,
    }
}

fn stored(seq: i64, event: DomainEvent) -> StoredEvent {
    let (kind, id) = event.aggregate();
    StoredEvent {
        sequence: seq,
        event_id: format!("e{seq}"),
        aggregate_kind: kind,
        stream_id: id.to_string(),
        stream_version: seq,
        occurred_at: NOW.into(),
        command_id: None,
        causation_id: None,
        correlation_id: None,
        actor_kind: ActorKind::Client,
        event,
        metadata: json!({}),
    }
}

/// Decide and fold into the model, like the engine does.
fn run(model: &mut ReadModel, cmd: Command) -> Result<Vec<DomainEvent>, DecideError> {
    let events = decide(model, &cmd, &ctx())?;
    for e in &events {
        let seq = model.sequence + 1;
        model.apply(&stored(seq, e.clone()));
    }
    Ok(events)
}

fn base() -> ReadModel {
    let mut m = ReadModel::default();
    run(
        &mut m,
        Command::ProjectCreate {
            project_id: Some("p1".into()),
            title: "".into(),
            workspace_root: "/tmp/demo".into(),
        },
    )
    .unwrap();
    run(
        &mut m,
        Command::ThreadCreate {
            thread_id: Some("t1".into()),
            project_id: "p1".into(),
            project_path: None,
            worktree: false,
            base_branch: None,
            title: None,
            instance_id: "native".into(),
            driver: "native".into(),
            model: None,
            agent_id: Some("coder".into()),
            runtime_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
            branch: None,
            worktree_path: None,
        },
    )
    .unwrap();
    m
}

fn start(m: &mut ReadModel, turn: &str, text: &str) -> Vec<DomainEvent> {
    run(
        m,
        Command::TurnStart {
            thread_id: "t1".into(),
            turn_id: Some(turn.into()),
            message_id: Some(format!("m-{turn}")),
            text: text.into(),
            attachments: vec![],
            model: None,
        },
    )
    .unwrap()
}

fn rt(turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
    RuntimeEvent::new("native", "native", "t1", turn, kind)
}

fn append(m: &mut ReadModel, ev: RuntimeEvent) -> Vec<DomainEvent> {
    run(m, Command::RuntimeAppend { event: ev }).unwrap()
}

#[test]
fn project_title_defaults_to_the_folder_name() {
    let m = base();
    assert_eq!(m.projects["p1"].title, "demo");
}

#[test]
fn a_duplicate_workspace_root_is_refused() {
    let m = base();
    let err = decide(
        &m,
        &Command::ProjectCreate {
            project_id: Some("p2".into()),
            title: "x".into(),
            workspace_root: "/tmp/demo".into(),
        },
        &ctx(),
    )
    .unwrap_err();
    assert_eq!(err.code, ERR_THREADS_EXISTS);
}

#[test]
fn turn_start_emits_user_message_then_request_and_titles_the_thread() {
    let mut m = base();
    let events = start(&mut m, "u1", "Fix the login bug\nmore");
    let types: Vec<&str> = events.iter().map(|e| e.type_name()).collect();
    assert_eq!(
        types,
        vec![
            "thread.meta-updated",
            "thread.message-sent",
            "thread.turn-start-requested"
        ]
    );
    assert_eq!(m.threads["t1"].title, "Fix the login bug");
    match &events[2] {
        DomainEvent::TurnStartRequested { ordinal, .. } => assert_eq!(*ordinal, 1),
        _ => unreachable!(),
    }
    assert_eq!(m.threads["t1"].active_turn.as_deref(), Some("u1"));
}

#[test]
fn a_second_turn_while_one_runs_is_busy() {
    let mut m = base();
    start(&mut m, "u1", "one");
    let err = decide(
        &m,
        &Command::TurnStart {
            thread_id: "t1".into(),
            turn_id: Some("u2".into()),
            message_id: Some("m2".into()),
            text: "two".into(),
            attachments: vec![],
            model: None,
        },
        &ctx(),
    )
    .unwrap_err();
    assert_eq!(err.code, ERR_THREADS_BUSY);
}

#[test]
fn deltas_fold_into_one_streaming_assistant_message() {
    let mut m = base();
    start(&mut m, "u1", "hi");
    let ev = append(
        &mut m,
        rt(
            Some("u1"),
            RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                stream_kind: StreamKind::AssistantText,
                delta: "Hel".into(),
                content_index: None,
                summary_index: None,
            }),
        )
        .with_item("seg0"),
    );
    match &ev[0] {
        DomainEvent::MessageSent {
            message_id,
            streaming,
            role,
            ..
        } => {
            assert_eq!(message_id, "u1:seg0");
            assert!(*streaming);
            assert_eq!(role, "assistant");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn approval_lifecycle_and_double_answer() {
    let mut m = base();
    start(&mut m, "u1", "hi");
    append(
        &mut m,
        rt(
            Some("u1"),
            RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                request_type: RequestType::CommandExecutionApproval,
                detail: Some(RequestDetail {
                    command: Some("rm -rf build".into()),
                    ..Default::default()
                }),
                app_name: None,
                options: vec![],
                args: None,
            }),
        )
        .with_request("call-1"),
    );
    assert!(m.threads["t1"].open_approvals.contains_key("call-1"));
    let respond = Command::ApprovalRespond {
        thread_id: "t1".into(),
        request_id: "call-1".into(),
        decision: ApprovalDecision::Accept,
    };
    run(&mut m, respond.clone()).unwrap();
    assert_eq!(
        decide(&m, &respond, &ctx()).unwrap_err().code,
        ERR_THREADS_ANSWERED
    );
    append(
        &mut m,
        rt(
            Some("u1"),
            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                request_type: RequestType::CommandExecutionApproval,
                decision: Some(ApprovalDecision::Accept),
                resolution: None,
            }),
        )
        .with_request("call-1"),
    );
    assert!(m.threads["t1"].open_approvals.is_empty());
    assert_eq!(
        decide(&m, &respond, &ctx()).unwrap_err().code,
        ERR_THREADS_NO_REQUEST
    );
}

#[test]
fn turn_completion_closes_stale_requests_and_records_usage() {
    let mut m = base();
    start(&mut m, "u1", "hi");
    append(
        &mut m,
        rt(
            Some("u1"),
            RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                request_type: RequestType::FileChangeApproval,
                detail: None,
                app_name: None,
                options: vec![],
                args: None,
            }),
        )
        .with_request("r1"),
    );
    let events = append(
        &mut m,
        rt(
            Some("u1"),
            RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                state: TurnEndState::Interrupted,
                stop_reason: None,
                usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                }),
                total_cost_usd: Some(0.01),
                error_message: None,
            }),
        ),
    );
    let types: Vec<&str> = events.iter().map(|e| e.type_name()).collect();
    assert_eq!(
        types,
        vec![
            "thread.turn-completed",
            "thread.turn-usage",
            "thread.approval-resolved"
        ]
    );
    match &events[1] {
        DomainEvent::TurnUsage { usage, .. } => assert_eq!(usage.cost_usd, Some(0.01)),
        _ => unreachable!(),
    }
    assert!(m.threads["t1"].active_turn.is_none());
    assert!(m.threads["t1"].open_approvals.is_empty());
}

#[test]
fn revert_and_interrupt_rules() {
    let mut m = base();
    assert_eq!(
        decide(
            &m,
            &Command::TurnInterrupt {
                thread_id: "t1".into(),
                turn_id: None
            },
            &ctx()
        )
        .unwrap_err()
        .code,
        ERR_THREADS_IDLE
    );
    start(&mut m, "u1", "one");
    assert_eq!(
        decide(
            &m,
            &Command::Revert {
                thread_id: "t1".into(),
                turn_count: 0
            },
            &ctx()
        )
        .unwrap_err()
        .code,
        ERR_THREADS_BUSY
    );
    append(
        &mut m,
        rt(
            Some("u1"),
            RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                state: TurnEndState::Completed,
                stop_reason: None,
                usage: None,
                total_cost_usd: None,
                error_message: None,
            }),
        ),
    );
    start(&mut m, "u2", "two");
    assert_eq!(m.threads["t1"].turn_count, 2);
    assert_eq!(
        decide(
            &m,
            &Command::Revert {
                thread_id: "t1".into(),
                turn_count: 5
            },
            &ctx()
        )
        .unwrap_err()
        .code,
        ERR_THREADS_BUSY
    );
}

#[test]
fn snooze_in_the_past_is_refused() {
    let m = base();
    let err = decide(
        &m,
        &Command::Snooze {
            thread_id: "t1".into(),
            until: "2020-01-01T00:00:00Z".into(),
        },
        &ctx(),
    )
    .unwrap_err();
    assert_eq!(err.code, ERR_THREADS_INVALID);
}

#[test]
fn history_import_builds_turns_and_tool_activities() {
    let m = base();
    let events = decide(
        &m,
        &Command::HistoryImport {
            thread_id: "old".into(),
            project_id: "p1".into(),
            title: "Old".into(),
            instance_id: "native".into(),
            driver: "native".into(),
            agent_id: None,
            created_at: None,
            messages: vec![
                ImportMessage {
                    message_id: None,
                    role: "user".into(),
                    text: "a".into(),
                    created_at: None,
                    tools: vec![],
                },
                ImportMessage {
                    message_id: None,
                    role: "assistant".into(),
                    text: "b".into(),
                    created_at: None,
                    tools: vec![
                        json!({"id": "c1", "name": "shell_exec", "input": {}, "output": "ok"}),
                    ],
                },
                ImportMessage {
                    message_id: None,
                    role: "user".into(),
                    text: "c".into(),
                    created_at: None,
                    tools: vec![],
                },
            ],
        },
        &ctx(),
    )
    .unwrap();
    let imported: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::TurnImported { ordinal, .. } => Some(*ordinal),
            _ => None,
        })
        .collect();
    assert_eq!(imported, vec![1, 2]);
    assert!(events
        .iter()
        .any(|e| matches!(e, DomainEvent::ActivityAppended { activity, .. } if activity.summary == "shell_exec")));
}

#[test]
fn raw_runtime_events_are_not_persisted() {
    let mut m = base();
    let events = append(
        &mut m,
        rt(
            None,
            RuntimeEventKind::Raw(RawPayload {
                source: "x".into(),
                method: None,
                payload: json!({}),
            }),
        ),
    );
    assert!(events.is_empty());
}

// ── Engine + SQLite ─────────────────────────────────────────────────────

fn temp_db(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omniget-threads-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("threads.db")
}

fn env(command: Command) -> CommandEnvelope {
    CommandEnvelope {
        command_id: None,
        command,
    }
}

fn rt_env(ev: RuntimeEvent) -> CommandEnvelope {
    CommandEnvelope {
        command_id: Some(format!("provider:{}", ev.event_id)),
        command: Command::RuntimeAppend { event: ev },
    }
}

fn delta(turn: &str, item: &str, text: &str) -> RuntimeEvent {
    rt(
        Some(turn),
        RuntimeEventKind::ContentDelta(ContentDeltaPayload {
            stream_kind: StreamKind::AssistantText,
            delta: text.into(),
            content_index: None,
            summary_index: None,
        }),
    )
    .with_item(item)
}

#[test]
fn acceptance_simulated_turn_survives_reopen() {
    let path = temp_db("accept");
    let engine = ThreadsEngine::open(&path).unwrap();
    engine
        .dispatch_blocking(env(Command::ProjectCreate {
            project_id: Some("p1".into()),
            title: "Demo".into(),
            workspace_root: "/tmp/omniget-demo".into(),
        }))
        .unwrap();
    engine
        .dispatch_blocking(env(Command::ThreadCreate {
            thread_id: Some("t1".into()),
            project_id: "p1".into(),
            project_path: None,
            worktree: false,
            base_branch: None,
            title: None,
            instance_id: "native".into(),
            driver: "native".into(),
            model: None,
            agent_id: Some("coder".into()),
            runtime_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
            branch: None,
            worktree_path: None,
        }))
        .unwrap();
    engine
        .dispatch_blocking(env(Command::TurnStart {
            thread_id: "t1".into(),
            turn_id: Some("u1".into()),
            message_id: Some("m1".into()),
            text: "Refactor main.rs".into(),
            attachments: vec![],
            model: None,
        }))
        .unwrap();
    engine
        .dispatch_blocking(rt_env(rt(
            Some("u1"),
            RuntimeEventKind::TurnStarted(TurnStartedPayload {
                model: Some("fake/m".into()),
                effort: None,
            }),
        )))
        .unwrap();
    for piece in ["I will ", "edit ", "main.rs."] {
        engine
            .dispatch_blocking(rt_env(delta("u1", "seg0", piece)))
            .unwrap();
    }
    let mut item = ItemPayload::new(ItemType::FileChange);
    item.status = Some(ItemStatus::InProgress);
    item.title = Some("fs_write".into());
    item.tool_name = Some("fs_write".into());
    item.data = Some(json!({"input": {"path": "main.rs"}}));
    engine
        .dispatch_blocking(rt_env(
            rt(Some("u1"), RuntimeEventKind::ItemStarted(item.clone())).with_item("call-1"),
        ))
        .unwrap();
    engine
        .dispatch_blocking(rt_env(
            rt(
                Some("u1"),
                RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                    request_type: RequestType::FileChangeApproval,
                    detail: Some(RequestDetail {
                        tool_name: Some("fs_write".into()),
                        paths: vec!["main.rs".into()],
                        diff: Some("@@ -1 +1 @@\n-a\n+b".into()),
                        ..Default::default()
                    }),
                    app_name: None,
                    options: vec![],
                    args: None,
                }),
            )
            .with_request("call-1"),
        ))
        .unwrap();

    // A duplicate provider event (same command id) is deduplicated.
    let dup = rt(
        Some("u1"),
        RuntimeEventKind::TurnStarted(Default::default()),
    );
    let first = engine.dispatch_blocking(rt_env(dup.clone())).unwrap();
    let again = engine.dispatch_blocking(rt_env(dup)).unwrap();
    assert!(!first.deduplicated);
    assert!(again.deduplicated);
    assert_eq!(first.sequence, again.sequence);

    let snap = engine.snapshot_blocking().unwrap();
    let page = engine.turns_page_blocking("t1", None, 10).unwrap();
    assert_eq!(snap.threads.len(), 1);
    let t = &snap.threads[0];
    assert_eq!(t.status, "approval");
    assert_eq!(t.pending_approval_count, 1);
    assert_eq!(t.title, "Refactor main.rs");
    assert_eq!(t.active_turn_id.as_deref(), Some("u1"));
    assert_eq!(snap.pending_approvals.len(), 1);
    assert_eq!(snap.pending_approvals[0].request_id, "call-1");
    assert_eq!(
        snap.pending_approvals[0].detail.as_ref().unwrap()["paths"][0],
        "main.rs"
    );
    assert_eq!(page.turns.len(), 1);
    let turn = &page.turns[0];
    assert_eq!(turn.turn.state, "running");
    let assistant = turn
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .unwrap();
    assert_eq!(assistant.text, "I will edit main.rs.");
    assert!(assistant.streaming);
    assert_eq!(turn.activities.len(), 1);
    assert_eq!(turn.activities[0].tone, "tool");

    // Replay from zero sees every event in order.
    let replay = engine.events_after_blocking(0, 1000).unwrap();
    assert!(!replay.reset);
    assert_eq!(replay.events.last().unwrap().sequence, snap.sequence);
    let seqs: Vec<i64> = replay.events.iter().map(|e| e.sequence).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));

    drop(engine);
    // Give the writer thread a moment to drop its connection.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let reopened = ThreadsEngine::open(&path).unwrap();
    let snap2 = reopened.snapshot_blocking().unwrap();
    let page2 = reopened.turns_page_blocking("t1", None, 10).unwrap();
    assert_eq!(snap, snap2);
    assert_eq!(page, page2);

    // The reopened engine still knows the approval is open.
    let answered = reopened
        .dispatch_blocking(env(Command::ApprovalRespond {
            thread_id: "t1".into(),
            request_id: "call-1".into(),
            decision: ApprovalDecision::Accept,
        }))
        .unwrap();
    assert_eq!(answered.events.len(), 1);
    let snap3 = reopened.snapshot_blocking().unwrap();
    assert_eq!(snap3.threads[0].pending_approval_count, 0);
    assert!(snap3.pending_approvals.is_empty());

    // Rebuilding every projection from the log gives the same state.
    drop(reopened);
    std::thread::sleep(std::time::Duration::from_millis(50));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DELETE FROM projects; DELETE FROM threads; DELETE FROM turns; DELETE FROM messages;
             DELETE FROM activities; DELETE FROM approvals; DELETE FROM user_inputs; DELETE FROM plans;
             DELETE FROM turn_usage; DELETE FROM provider_sessions;
             UPDATE projector_cursors SET last_applied_sequence = 0;",
        )
        .unwrap();
    }
    let rebuilt = ThreadsEngine::open(&path).unwrap();
    let snap4 = rebuilt.snapshot_blocking().unwrap();
    assert_eq!(snap3, snap4);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn revert_and_fork_through_the_engine() {
    let path = temp_db("fork");
    let e = ThreadsEngine::open(&path).unwrap();
    e.dispatch_blocking(env(Command::ProjectCreate {
        project_id: Some("p".into()),
        title: "P".into(),
        workspace_root: String::new(),
    }))
    .unwrap();
    e.dispatch_blocking(env(Command::ThreadCreate {
        thread_id: Some("t1".into()),
        project_id: "p".into(),
        project_path: None,
        worktree: false,
        base_branch: None,
        title: Some("T".into()),
        instance_id: "native".into(),
        driver: "native".into(),
        model: None,
        agent_id: None,
        runtime_mode: AccessMode::FullAccess,
        interaction_mode: InteractionMode::Default,
        branch: None,
        worktree_path: None,
    }))
    .unwrap();
    for (turn, text) in [("a", "first"), ("b", "second"), ("c", "third")] {
        e.dispatch_blocking(env(Command::TurnStart {
            thread_id: "t1".into(),
            turn_id: Some(turn.into()),
            message_id: Some(format!("m{turn}")),
            text: text.into(),
            attachments: vec![],
            model: None,
        }))
        .unwrap();
        e.dispatch_blocking(rt_env(delta(turn, "s", &format!("answer {text}"))))
            .unwrap();
        e.dispatch_blocking(rt_env(rt(
            Some(turn),
            RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                state: TurnEndState::Completed,
                stop_reason: None,
                usage: None,
                total_cost_usd: None,
                error_message: None,
            }),
        )))
        .unwrap();
    }
    // Pagination: 2 newest, then the rest.
    let p1 = e.turns_page_blocking("t1", None, 2).unwrap();
    assert_eq!(
        p1.turns.iter().map(|t| t.turn.ordinal).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(p1.has_more);
    let p2 = e.turns_page_blocking("t1", p1.before_turn, 20).unwrap();
    assert_eq!(
        p2.turns.iter().map(|t| t.turn.ordinal).collect::<Vec<_>>(),
        vec![1]
    );
    assert!(!p2.has_more);

    let forked = e
        .dispatch_blocking(env(Command::Fork {
            thread_id: "t1".into(),
            new_thread_id: Some("t2".into()),
            turn_count: Some(2),
            title: None,
        }))
        .unwrap();
    assert!(!forked.events.is_empty());
    let f = e.turns_page_blocking("t2", None, 10).unwrap();
    assert_eq!(f.turns.len(), 2);
    assert_eq!(f.turns[1].messages[1].text, "answer second");

    e.dispatch_blocking(env(Command::Revert {
        thread_id: "t1".into(),
        turn_count: 1,
    }))
    .unwrap();
    let r = e.turns_page_blocking("t1", None, 10).unwrap();
    assert_eq!(r.turns.len(), 1);
    let snap = e.snapshot_blocking().unwrap();
    let t1 = snap.threads.iter().find(|t| t.thread_id == "t1").unwrap();
    assert_eq!(t1.turn_count, 1);
    // A new turn after the revert takes ordinal 2 again.
    e.dispatch_blocking(env(Command::TurnStart {
        thread_id: "t1".into(),
        turn_id: Some("d".into()),
        message_id: Some("md".into()),
        text: "again".into(),
        attachments: vec![],
        model: None,
    }))
    .unwrap();
    let r = e.turns_page_blocking("t1", None, 10).unwrap();
    assert_eq!(r.turns.last().unwrap().turn.ordinal, 2);
    drop(e);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn replay_over_budget_asks_for_a_reset() {
    let path = temp_db("budget");
    let e = ThreadsEngine::open(&path).unwrap();
    e.dispatch_blocking(env(Command::ProjectCreate {
        project_id: Some("p".into()),
        title: "P".into(),
        workspace_root: String::new(),
    }))
    .unwrap();
    let page = e.events_after_blocking(0, 1000).unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(!page.reset);
    let ahead = e.events_after_blocking(99, 1000).unwrap();
    assert!(ahead.reset);
    drop(e);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn migration_imports_once() {
    use crate::core::llm::coordinator::append_message;
    use crate::core::llm::types::{ContentPart, Message, Role};
    let path = temp_db("migrate");
    let conv_dir = path.parent().unwrap().join("conversations");
    append_message(
        &conv_dir,
        "chat-1",
        Some("coder"),
        &Message::text(Role::User, "hello"),
    );
    append_message(
        &conv_dir,
        "chat-1",
        Some("coder"),
        &Message {
            role: Role::Assistant,
            parts: vec![
                ContentPart::Text { text: "hi".into() },
                ContentPart::ToolUse {
                    id: "c1".into(),
                    name: "fs_read".into(),
                    input: json!({"path": "a"}),
                },
            ],
        },
    );
    append_message(
        &conv_dir,
        "chat-1",
        Some("coder"),
        &Message {
            role: Role::Tool,
            parts: vec![ContentPart::ToolResult {
                tool_use_id: "c1".into(),
                content: "file".into(),
                is_error: false,
            }],
        },
    );
    append_message(
        &conv_dir,
        "help-x",
        None,
        &Message::text(Role::User, "help"),
    );
    let e = ThreadsEngine::open(&path).unwrap();
    let r1 = super::migrate::migrate_conversations(&e, &conv_dir, |_| None);
    assert_eq!(r1.imported, 1, "{r1:?}");
    let r2 = super::migrate::migrate_conversations(&e, &conv_dir, |_| None);
    assert_eq!(r2.imported, 0);
    assert_eq!(r2.skipped, 1);
    let page = e.turns_page_blocking("chat-1", None, 10).unwrap();
    assert_eq!(page.turns.len(), 1);
    assert_eq!(
        page.turns[0].activities[0].payload["data"]["output"],
        "file"
    );
    drop(e);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
