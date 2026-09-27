//! Tests of the Codex driver against the real 0.156 wire (no login) and a
//! turn built from the schema (see `fixtures/SOURCES.txt`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::super::{
    AccessMode, ApprovalDecision, Driver, DriverError, DriverInstance, ErrorClass, InteractionMode,
    ItemStatus, ItemType, RequestType, RuntimeEvent, RuntimeEventKind, SessionStart, StreamKind,
    TurnEndState, TurnStart,
};
use super::driver::{cursor_thread, is_missing_thread, turn_input, CodexDriver};
use super::launch::LaunchSpec;
use super::protocol as p;
use super::rpc::RpcError;
use super::supervisor::{Connection, Connector};
use super::translate::{
    approval_result, classify_error, elicitation_result, format_wait, user_input_result, AutoReply,
    PendingKind, Translator,
};

const HANDSHAKE: &str = include_str!("fixtures/app-server-0.156-handshake.jsonl");
const UNAUTH_TURN: &str = include_str!("fixtures/app-server-0.156-unauthenticated-turn.jsonl");
const APPROVAL_TURN: &str = include_str!("fixtures/app-server-schema-approval-turn.jsonl");

fn wire(text: &str) -> Vec<(String, Value)> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v.get("msg").is_some())
        .map(|v| (v["dir"].as_str().unwrap().to_string(), v["msg"].clone()))
        .collect()
}

/// Feeds every server → client message to a translator. Returns the events
/// and the requests it opened (`rpc id → outcome reply`).
fn replay(tr: &mut Translator, text: &str) -> (Vec<RuntimeEvent>, Vec<(Value, Option<AutoReply>)>) {
    let mut events = Vec::new();
    let mut replies = Vec::new();
    for (dir, msg) in wire(text) {
        if dir != "in" {
            continue;
        }
        match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            (Some(method), Some(id)) => {
                let out = tr.on_server_request(id.clone(), method, &msg["params"]);
                events.extend(out.events);
                replies.push((id.clone(), out.reply));
            }
            (Some(method), None) => events.extend(tr.on_notification(method, &msg["params"])),
            _ => {}
        }
    }
    (events, replies)
}

fn kinds(events: &[RuntimeEvent]) -> Vec<&'static str> {
    events.iter().map(|e| e.type_name()).collect()
}

// ── protocol ────────────────────────────────────────────────────────────

#[test]
fn generated_types_decode_the_real_wire() {
    let msgs = wire(HANDSHAKE);
    let init = msgs
        .iter()
        .find(|(d, m)| d == "in" && m["id"] == 1)
        .unwrap();
    let r: p::InitializeResponse = serde_json::from_value(init.1["result"].clone()).unwrap();
    assert!(r.user_agent.contains("0.156.0"));
    assert_eq!(r.platform_family, "unix");
    let acc = msgs
        .iter()
        .find(|(d, m)| d == "in" && m["id"] == 2)
        .unwrap();
    let a: p::GetAccountResponse = serde_json::from_value(acc.1["result"].clone()).unwrap();
    assert!(a.account.is_none());
    assert!(a.requires_openai_auth);
    let start = msgs
        .iter()
        .find(|(d, m)| d == "in" && m["id"] == 4)
        .unwrap();
    let s: p::ThreadStartResponse = serde_json::from_value(start.1["result"].clone()).unwrap();
    assert_eq!(s.model, "gpt-6-astra");
    assert!(matches!(
        s.approval_policy,
        p::AskForApproval::Known(p::AskForApprovalKnown::Untrusted)
    ));
    assert!(matches!(s.sandbox, p::SandboxPolicy::ReadOnly { .. }));

    for (dir, msg) in wire(UNAUTH_TURN) {
        if dir != "in" {
            continue;
        }
        match msg.get("method").and_then(Value::as_str) {
            Some("turn/completed") => {
                let n: p::TurnCompletedNotification =
                    serde_json::from_value(msg["params"].clone()).unwrap();
                assert_eq!(n.turn.status, p::TurnStatus::Failed);
                let e = n.turn.error.unwrap();
                assert!(matches!(
                    e.codex_error_info,
                    Some(p::CodexErrorInfo::Known(p::CodexErrorInfoKnown::Other))
                ));
            }
            Some("error") => {
                let n: p::ErrorNotification =
                    serde_json::from_value(msg["params"].clone()).unwrap();
                if n.will_retry {
                    assert!(matches!(
                        n.error.codex_error_info,
                        Some(p::CodexErrorInfo::Known(
                            p::CodexErrorInfoKnown::ResponseStreamDisconnected {
                                http_status_code: Some(401)
                            }
                        ))
                    ));
                }
            }
            Some("item/started") | Some("item/completed") => {
                let item: p::ThreadItem =
                    serde_json::from_value(msg["params"]["item"].clone()).unwrap();
                assert!(matches!(item, p::ThreadItem::UserMessage { .. }));
            }
            _ => {}
        }
    }
}

#[test]
fn unknown_enum_values_do_not_break_the_decode() {
    let v = json!({"type": "someNewItem", "id": "x"});
    assert_eq!(
        serde_json::from_value::<p::ThreadItem>(v).unwrap(),
        p::ThreadItem::Unknown
    );
    let info: p::CodexErrorInfo = serde_json::from_value(json!({"brandNew": {"x": 1}})).unwrap();
    assert!(matches!(info, p::CodexErrorInfo::Other(_)));
    let s: p::CommandExecutionStatus = serde_json::from_value(json!("paused")).unwrap();
    assert_eq!(s, p::CommandExecutionStatus::Unknown);
    // A required field missing still decodes (defaults).
    let t: p::Turn = serde_json::from_value(json!({"id": "t"})).unwrap();
    assert_eq!(t.id, "t");
}

#[test]
fn method_tables_cover_what_the_driver_calls() {
    let has = |list: &[(&str, &str)], m: &str| list.iter().any(|(x, _)| *x == m);
    for m in [
        "initialize",
        "thread/start",
        "thread/resume",
        "thread/fork",
        "thread/revert",
        "thread/turns/list",
        "turn/start",
        "turn/interrupt",
        "account/read",
        "account/rateLimits/read",
    ] {
        assert!(has(p::client_request::ALL, m), "{m}");
    }
    for m in [
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
        "item/permissions/requestApproval",
        "applyPatchApproval",
        "execCommandApproval",
    ] {
        assert!(has(p::server_request::ALL, m), "{m}");
    }
    assert_eq!(p::CODEX_PROTOCOL_VERSION, "0.156.0");
}

// ── translator: real unauthenticated turn ───────────────────────────────

#[test]
fn a_401_turn_becomes_warnings_one_auth_error_and_a_failed_turn() {
    let mut tr = Translator::new("codex", "thr-omni");
    tr.set_provider_thread("01a0cb43-95df-7352-bd76-3558899e01f9");
    tr.begin_turn("eng-1");
    tr.bind_provider_turn("01a0cb43-95ea-7090-8c41-1381d8f58845");
    // Only the part of the capture before the history calls.
    let text: String = UNAUTH_TURN
        .lines()
        .take_while(|l| !l.contains("\"thread/turns/list\""))
        .map(|l| format!("{l}\n"))
        .collect();
    let (events, _) = replay(&mut tr, &text);
    let k = kinds(&events);
    assert!(k.contains(&"turn.started"));
    assert!(
        !k.contains(&"item.started"),
        "user messages are not items: {k:?}"
    );
    let warnings = events
        .iter()
        .filter(|e| e.type_name() == "runtime.warning")
        .count();
    assert!(warnings >= 5, "retries are warnings ({warnings})");
    let errors: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::RuntimeError(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1, "one error for the turn");
    assert_eq!(errors[0].code.as_deref(), Some("ERR_CODEX_AUTH"));
    assert_eq!(errors[0].class, ErrorClass::ProviderError);
    assert!(errors[0].message.contains("codex login"));
    let done = events
        .iter()
        .find(|e| e.type_name() == "turn.completed")
        .unwrap();
    assert_eq!(done.turn_id.as_deref(), Some("eng-1"));
    match &done.kind {
        RuntimeEventKind::TurnCompleted(p) => {
            assert_eq!(p.state, TurnEndState::Failed);
            assert!(p.error_message.as_deref().unwrap().contains("401"));
        }
        _ => unreachable!(),
    }
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::ThreadStateChanged(s) if s.state == "error"
    )));
    // Unknown notification (remoteControl) passes as raw, never persisted.
    assert!(k.contains(&"raw"));
    assert!(tr.active_engine_turn().is_none());
}

// ── translator: approval turn built from the schema ─────────────────────

#[test]
fn the_schema_turn_translates_every_item_request_and_the_usage() {
    let mut tr = Translator::new("codex", "thr-omni");
    tr.set_provider_thread("thr_codex_1");
    tr.set_model(Some("gpt-6-astra".into()));
    tr.begin_turn("eng-7");
    // Everything but the final `turn/completed`: the parked requests are
    // answered while the turn still runs.
    let body: Vec<&str> = APPROVAL_TURN
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let (last, head) = body.split_last().unwrap();
    assert!(last.contains("turn/completed"));
    let (mut events, replies) = replay(&mut tr, &head.join("\n"));

    // Every event of the turn carries the engine's turn id.
    for e in &events {
        if !matches!(e.type_name(), "raw" | "account.rate-limits.updated") {
            assert_eq!(e.turn_id.as_deref(), Some("eng-7"), "{}", e.type_name());
        }
    }

    // Reasoning delta + summary text.
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::ContentDelta(d) if d.stream_kind == StreamKind::ReasoningSummaryText
    )));
    // Plan.
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::PlanUpdated(p) if p.plan.len() == 2 && p.plan[0].status == "inProgress"
    )));

    // Command: item + output delta + completed with exit code.
    let cmd_done = events
        .iter()
        .find(|e| e.item_id.as_deref() == Some("item_c") && e.type_name() == "item.completed")
        .unwrap();
    match &cmd_done.kind {
        RuntimeEventKind::ItemCompleted(p) => {
            assert_eq!(p.item_type, ItemType::CommandExecution);
            assert_eq!(p.status, Some(ItemStatus::Completed));
            assert_eq!(p.data.as_ref().unwrap()["output"]["exitCode"], 0);
        }
        _ => unreachable!(),
    }
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::ContentDelta(d) if d.stream_kind == StreamKind::CommandOutput && d.delta.contains("main.rs")
    ) && e.item_id.as_deref() == Some("item_c")));

    // Requests opened: command, file change (with the diff), permissions,
    // legacy patch; the question is a user-input request; the unknown one
    // is refused on the spot.
    let opened: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::RequestOpened(p) => Some((e.request_id.clone().unwrap(), p.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 4, "{opened:?}");
    let cmd = &opened[0].1;
    assert_eq!(cmd.request_type, RequestType::CommandExecutionApproval);
    let d = cmd.detail.as_ref().unwrap();
    assert_eq!(d.command.as_deref(), Some("ls -la"));
    assert_eq!(d.cwd.as_deref(), Some("/work/repo"));
    let file = &opened[1].1;
    assert_eq!(file.request_type, RequestType::FileChangeApproval);
    let fd = file.detail.as_ref().unwrap();
    assert_eq!(fd.paths, vec!["src/main.rs".to_string()]);
    let diff = fd.diff.as_deref().unwrap();
    assert!(
        diff.starts_with("--- a/src/main.rs\n+++ b/src/main.rs\n@@"),
        "{diff}"
    );
    assert!(diff.contains("+fn main() { println!(\"hi\"); }"));
    assert_eq!(opened[2].1.request_type, RequestType::PermissionApproval);
    assert!(opened[2]
        .1
        .detail
        .as_ref()
        .unwrap()
        .paths
        .contains(&"write: /tmp/out".to_string()));
    let legacy = &opened[3].1;
    assert_eq!(legacy.request_type, RequestType::ApplyPatchApproval);
    assert!(legacy
        .detail
        .as_ref()
        .unwrap()
        .diff
        .as_deref()
        .unwrap()
        .contains("+++ b/README.txt\n@@ -0,0 +1,2 @@\n+hello"));
    let asked = events
        .iter()
        .find(|e| e.type_name() == "user-input.requested")
        .unwrap();
    match &asked.kind {
        RuntimeEventKind::UserInputRequested(q) => {
            assert_eq!(q.questions[0].id, "q1");
            assert_eq!(q.questions[0].options.len(), 2);
            assert!(!q.questions[0].allow_custom_answer);
        }
        _ => unreachable!(),
    }
    let unknown = replies.iter().find(|(id, _)| *id == json!(5)).unwrap();
    assert!(matches!(
        unknown.1,
        Some(AutoReply::Error { code: -32601, .. })
    ));

    // Codex resolved the first two on its side (`serverRequest/resolved`):
    // they are closed as answered elsewhere.
    let resolved: Vec<_> = events
        .iter()
        .filter(|e| e.type_name() == "request.resolved")
        .map(|e| e.request_id.clone().unwrap())
        .collect();
    assert_eq!(resolved, vec![opened[0].0.clone(), opened[1].0.clone()]);

    // The ones still parked are answered with the right JSON-RPC bodies.
    let perm_req = &opened[2].0;
    let ((id, body), ev) = tr
        .answer_request(perm_req, ApprovalDecision::AcceptForSession)
        .unwrap();
    assert_eq!(id, json!(2));
    assert_eq!(body["scope"], "session");
    assert_eq!(body["permissions"]["fileSystem"]["write"][0], "/tmp/out");
    assert_eq!(ev[0].type_name(), "request.resolved");
    let q_req = asked.request_id.clone().unwrap();
    let ((id, body), _) = tr.answer_user_input(&q_req, &json!({"q1": "Yes"})).unwrap();
    assert_eq!(id, json!(3));
    assert_eq!(body, json!({"answers": {"q1": {"answers": ["Yes"]}}}));
    let ((id, body), _) = tr
        .answer_request(&opened[3].0, ApprovalDecision::Decline)
        .unwrap();
    assert_eq!(id, json!(4));
    assert!(body["decision"]["denied"]["rejection"].is_string());
    // Answered once: a second answer is an error (the host marks it stale).
    assert!(tr
        .answer_request(&opened[3].0, ApprovalDecision::Accept)
        .is_err());

    // MCP tool, web search.
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::ItemCompleted(p) if p.item_type == ItemType::McpToolCall
            && p.tool_name.as_deref() == Some("mcp__github__list_issues")
            && p.title.as_deref() == Some("github · list_issues")
    )));
    assert!(kinds(&events).contains(&"tool.progress"));
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::ItemStarted(p) if p.item_type == ItemType::WebSearch && p.detail.as_deref() == Some("rust println")
    )));

    // Assistant text: deltas + final detail.
    let text: String = events
        .iter()
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::ContentDelta(d) if d.stream_kind == StreamKind::AssistantText => {
                Some(d.delta.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(text, "Done.");
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        RuntimeEventKind::ItemCompleted(p) if p.item_type == ItemType::AssistantMessage && p.detail.as_deref() == Some("Done.")
    )));

    let (tail, _) = replay(&mut tr, last);
    events.extend(tail);

    // Diff, rate limits.
    assert!(kinds(&events).contains(&"turn.diff.updated"));
    let rl = events
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::RateLimitsUpdated(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(rl.windows.len(), 1);
    assert_eq!(rl.windows[0].id, "primary");
    assert_eq!(rl.windows[0].used_percent, Some(42.0));
    assert_eq!(rl.windows[0].window_minutes, Some(300));
    assert_eq!(rl.windows[0].label, "5 h");
    assert!(rl.windows[0]
        .resets_at
        .as_deref()
        .unwrap()
        .starts_with("2026-"));

    // turn.completed with the usage of the turn (first turn: the total).
    let done = events
        .iter()
        .find(|e| e.type_name() == "turn.completed")
        .unwrap();
    match &done.kind {
        RuntimeEventKind::TurnCompleted(p) => {
            assert_eq!(p.state, TurnEndState::Completed);
            let u = p.usage.as_ref().unwrap();
            assert_eq!(u.input_tokens, 4000, "input minus cached");
            assert_eq!(u.cached_input_tokens, 8000);
            assert_eq!(u.output_tokens, 500);
            assert_eq!(u.reasoning_output_tokens, 200);
            assert_eq!(u.used_tokens, Some(6300), "context in use = last response");
            assert_eq!(u.max_tokens, Some(272_000));
            assert_eq!(u.duration_ms, Some(4200));
            assert_eq!(u.model.as_deref(), Some("gpt-6-astra"));
            assert!(p.total_cost_usd.is_none());
        }
        _ => unreachable!(),
    }
    assert!(tr.active_engine_turn().is_none());
}

#[test]
fn the_second_turn_is_charged_only_its_own_tokens() {
    let mut tr = Translator::new("codex", "t");
    tr.set_provider_thread("P");
    let usage = |turn: &str, total: i64| {
        json!({"threadId": "P", "turnId": turn, "tokenUsage": {
            "total": {"inputTokens": total, "cachedInputTokens": 0, "outputTokens": 10, "reasoningOutputTokens": 0, "totalTokens": total + 10},
            "last": {"inputTokens": 5, "cachedInputTokens": 0, "outputTokens": 1, "reasoningOutputTokens": 0, "totalTokens": 6}
        }})
    };
    let done = |turn: &str| json!({"threadId": "P", "turn": {"id": turn, "items": [], "status": "completed"}});
    for (eng, pt, total) in [("e1", "t1", 100), ("e2", "t2", 250)] {
        tr.begin_turn(eng);
        tr.bind_provider_turn(pt);
        tr.on_notification("thread/tokenUsage/updated", &usage(pt, total));
        let evs = tr.on_notification("turn/completed", &done(pt));
        let u = evs
            .iter()
            .find_map(|e| match &e.kind {
                RuntimeEventKind::TurnCompleted(p) => p.usage.clone(),
                _ => None,
            })
            .unwrap();
        if eng == "e2" {
            assert_eq!(u.input_tokens, 150);
            assert_eq!(u.output_tokens, 0);
        } else {
            assert_eq!(u.input_tokens, 100);
        }
    }
    // A late update of an old turn only moves the baseline.
    tr.begin_turn("e3");
    tr.bind_provider_turn("t3");
    tr.on_notification("thread/tokenUsage/updated", &usage("t2", 300));
    tr.on_notification("thread/tokenUsage/updated", &usage("t3", 400));
    let evs = tr.on_notification("turn/completed", &done("t3"));
    let u = evs
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::TurnCompleted(p) => p.usage.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(u.input_tokens, 100);
}

#[test]
fn a_notification_before_the_turn_start_answer_binds_the_turn() {
    let mut tr = Translator::new("codex", "t");
    tr.set_provider_thread("P");
    tr.begin_turn("eng");
    let evs = tr.on_notification(
        "turn/started",
        &json!({"threadId": "P", "turn": {"id": "pt", "items": [], "status": "inProgress"}}),
    );
    assert_eq!(evs[0].turn_id.as_deref(), Some("eng"));
    assert_eq!(
        evs[0]
            .provider_refs
            .as_ref()
            .unwrap()
            .provider_turn_id
            .as_deref(),
        Some("pt")
    );
    tr.bind_provider_turn("pt");
    assert_eq!(tr.active_provider_turn(), Some("pt"));
}

#[test]
fn sub_agent_traffic_becomes_tasks_and_is_interruptible() {
    let mut tr = Translator::new("codex", "t");
    tr.set_provider_thread("root");
    tr.begin_turn("eng");
    let started = tr.on_notification(
        "thread/started",
        &json!({"thread": {"id": "child", "parentThreadId": "root", "agentNickname": "scout"}}),
    );
    assert!(
        matches!(&started[0].kind, RuntimeEventKind::TaskStarted(p) if p.title.as_deref() == Some("scout"))
    );
    tr.on_notification(
        "turn/started",
        &json!({"threadId": "child", "turn": {"id": "ct", "items": [], "status": "inProgress"}}),
    );
    assert_eq!(
        tr.child_turns(),
        vec![("child".to_string(), "ct".to_string())]
    );
    let delta = tr.on_notification(
        "item/agentMessage/delta",
        &json!({"threadId": "child", "turnId": "ct", "itemId": "i", "delta": "x"}),
    );
    assert!(delta.is_empty(), "child deltas stay out of the parent chat");
    let done = tr.on_notification(
        "turn/completed",
        &json!({"threadId": "child", "turn": {"id": "ct", "items": [], "status": "completed"}}),
    );
    assert!(
        matches!(&done[0].kind, RuntimeEventKind::TaskUpdated(p) if p.status.as_deref() == Some("idle"))
    );
    assert!(tr.child_turns().is_empty());
    // The root turn is untouched.
    assert_eq!(tr.active_engine_turn(), Some("eng"));
}

#[test]
fn stop_cancels_parked_requests_first() {
    let mut tr = Translator::new("codex", "t");
    tr.set_provider_thread("P");
    tr.begin_turn("eng");
    tr.on_server_request(
        json!(9),
        "item/commandExecution/requestApproval",
        &json!({"itemId": "i", "threadId": "P", "turnId": "pt", "command": "rm -rf x", "startedAtMs": 1}),
    );
    tr.on_server_request(
        json!(10),
        "item/tool/requestUserInput",
        &json!({"itemId": "q", "threadId": "P", "turnId": "pt", "isBlocking": true, "questions": []}),
    );
    let (answers, events) = tr.cancel_all_pending();
    assert_eq!(answers.len(), 2);
    let cmd = answers.iter().find(|(id, _)| *id == json!(9)).unwrap();
    assert_eq!(cmd.1, json!({"decision": "cancel"}));
    let q = answers.iter().find(|(id, _)| *id == json!(10)).unwrap();
    assert_eq!(q.1, json!({"answers": {}}));
    assert_eq!(events.len(), 2);
}

#[test]
fn a_dead_process_closes_the_running_turn() {
    let mut tr = Translator::new("codex", "t");
    tr.begin_turn("eng");
    let evs = tr.abort_active("The Codex app-server stopped (end of stream, exit code 1)");
    assert_eq!(kinds(&evs), vec!["runtime.error", "turn.completed"]);
    assert!(
        matches!(&evs[0].kind, RuntimeEventKind::RuntimeError(p) if p.class == ErrorClass::TransportError)
    );
    assert!(
        matches!(&evs[1].kind, RuntimeEventKind::TurnCompleted(p) if p.state == TurnEndState::Failed)
    );
    assert!(tr.abort_active("again").is_empty());
}

// ── pure helpers ────────────────────────────────────────────────────────

#[test]
fn approval_bodies_follow_the_schema() {
    use ApprovalDecision::*;
    assert_eq!(
        approval_result(PendingKind::Command, AcceptAlways, &json!({})),
        json!({"decision": "acceptForSession"})
    );
    assert_eq!(
        approval_result(PendingKind::FileChange, Cancel, &json!({})),
        json!({"decision": "cancel"})
    );
    assert_eq!(
        approval_result(PendingKind::Permissions, Decline, &json!({})),
        json!({"permissions": {}})
    );
    assert_eq!(
        approval_result(
            PendingKind::Permissions,
            Accept,
            &json!({"permissions": {"network": {"enabled": true}}})
        ),
        json!({"permissions": {"network": {"enabled": true}}, "scope": "turn"})
    );
    assert_eq!(
        approval_result(PendingKind::LegacyExec, AcceptForSession, &json!({})),
        json!({"decision": "approved_for_session"})
    );
    assert_eq!(
        approval_result(PendingKind::LegacyPatch, Cancel, &json!({})),
        json!({"decision": "abort"})
    );
    // The bodies decode as the generated response types.
    let b = approval_result(PendingKind::Command, Accept, &json!({}));
    let _: p::CommandExecutionRequestApprovalResponse = serde_json::from_value(b).unwrap();
    let b = approval_result(PendingKind::LegacyPatch, Decline, &json!({}));
    let r: p::ApplyPatchApprovalResponse = serde_json::from_value(b).unwrap();
    assert!(matches!(
        r.decision,
        p::ReviewDecision::Known(p::ReviewDecisionKnown::Denied { .. })
    ));
    let b = approval_result(
        PendingKind::Permissions,
        AcceptForSession,
        &json!({"permissions": {}}),
    );
    let _: p::PermissionsRequestApprovalResponse = serde_json::from_value(b).unwrap();
}

#[test]
fn elicitation_fills_the_form_or_declines() {
    let params = json!({
        "serverName": "gmail", "threadId": "P", "mode": "form", "message": "Allow ChatGPT to use Gmail?",
        "_meta": {"persist": ["session", "always"]},
        "requestedSchema": {"type": "object", "required": ["choice"], "properties": {
            "choice": {"type": "string", "enum": ["deny", "allow once", "always allow"]},
            "remember": {"type": "boolean", "title": "Remember this"}
        }}
    });
    let r = elicitation_result(ApprovalDecision::Accept, &params);
    assert_eq!(r["action"], "accept");
    assert_eq!(r["content"]["choice"], "allow once");
    assert_eq!(r["content"]["remember"], false);
    let r = elicitation_result(ApprovalDecision::AcceptAlways, &params);
    assert_eq!(r["content"]["choice"], "always allow");
    assert_eq!(r["content"]["remember"], true);
    assert_eq!(r["_meta"]["persist"], "always");
    assert_eq!(
        elicitation_result(ApprovalDecision::Decline, &params),
        json!({"action": "decline"})
    );
    // URL mode is never accepted; an unfillable required field declines.
    let url = json!({"mode": "url", "url": "https://x", "elicitationId": "e", "message": "m"});
    assert_eq!(
        elicitation_result(ApprovalDecision::Accept, &url)["action"],
        "decline"
    );
    let hard = json!({"mode": "form", "requestedSchema": {"type": "object", "required": ["token"], "properties": {"token": {"type": "string"}}}});
    assert_eq!(
        elicitation_result(ApprovalDecision::Accept, &hard)["action"],
        "decline"
    );
    // App name from the message.
    let mut tr = Translator::new("codex", "t");
    let out = tr.on_server_request(json!(1), "mcpServer/elicitation/request", &params);
    match &out.events[0].kind {
        RuntimeEventKind::RequestOpened(p) => {
            assert_eq!(p.app_name.as_deref(), Some("Gmail"));
            assert_eq!(p.options.len(), 5);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn user_input_answers_accept_every_ui_shape() {
    let r =
        user_input_result(&json!({"a": "x", "b": ["y", "z"], "c": {"answers": ["w"]}, "d": null}));
    assert_eq!(r["answers"]["a"]["answers"], json!(["x"]));
    assert_eq!(r["answers"]["b"]["answers"], json!(["y", "z"]));
    assert_eq!(r["answers"]["c"]["answers"], json!(["w"]));
    assert_eq!(r["answers"]["d"]["answers"], json!([]));
    let _: p::ToolRequestUserInputResponse = serde_json::from_value(r).unwrap();
}

#[test]
fn errors_are_classified() {
    assert_eq!(
        classify_error(Some(&json!("usageLimitExceeded")), "x").1,
        "ERR_CODEX_USAGE_LIMIT"
    );
    assert_eq!(
        classify_error(
            Some(&json!({"responseStreamDisconnected": {"httpStatusCode": 401}})),
            "x"
        ),
        (ErrorClass::ProviderError, "ERR_CODEX_AUTH")
    );
    assert_eq!(
        classify_error(
            Some(&json!({"responseStreamDisconnected": {"httpStatusCode": 502}})),
            "x"
        ),
        (ErrorClass::TransportError, "ERR_CODEX_TRANSPORT")
    );
    assert_eq!(
        classify_error(Some(&json!("contextWindowExceeded")), "x").0,
        ErrorClass::ValidationError
    );
    assert_eq!(
        classify_error(Some(&json!("sandboxError")), "x").0,
        ErrorClass::PermissionError
    );
    assert_eq!(classify_error(None, "boom").1, "ERR_CODEX");
    assert_eq!(format_wait(5 * 86_400 + 5 * 3600 + 7), "5d 5h");
    assert_eq!(format_wait(3 * 3600 + 20 * 60), "3h 20m");
    assert_eq!(format_wait(12 * 60), "12m");
}

#[test]
fn usage_limit_message_names_the_window() {
    let mut tr = Translator::new("codex", "t");
    let now = chrono::Utc::now().timestamp();
    let snap: p::RateLimitSnapshot = serde_json::from_value(json!({
        "limitId": "codex",
        "primary": {"usedPercent": 30, "windowDurationMins": 300, "resetsAt": now + 3600},
        "secondary": {"usedPercent": 100, "windowDurationMins": 10080, "resetsAt": now + 5 * 86_400 + 5 * 3600 + 30}
    }))
    .unwrap();
    tr.merge_rate_limits(&snap, None).unwrap();
    let msg = tr.usage_limit_message().unwrap();
    assert!(
        msg.starts_with("Codex usage limit reached. The weekly limit resets in 5d 5h."),
        "{msg}"
    );
    // Sparse update keeps the other window; a model-specific bucket is ignored.
    let sparse: p::RateLimitSnapshot =
        serde_json::from_value(json!({"primary": {"usedPercent": 50}})).unwrap();
    let ev = tr.merge_rate_limits(&sparse, None).unwrap();
    match ev.kind {
        RuntimeEventKind::RateLimitsUpdated(r) => {
            assert_eq!(r.windows.len(), 2);
            assert_eq!(r.windows[0].used_percent, Some(50.0));
            assert_eq!(r.windows[1].label, "Weekly");
        }
        _ => unreachable!(),
    }
    let spark: p::RateLimitSnapshot =
        serde_json::from_value(json!({"limitId": "spark", "primary": {"usedPercent": 99}}))
            .unwrap();
    assert!(tr.merge_rate_limits(&spark, None).is_none());
}

#[test]
fn cursors_inputs_and_resume_errors() {
    assert_eq!(cursor_thread(&json!({"threadId": "a"})), Some("a".into()));
    assert_eq!(cursor_thread(&json!("b")), Some("b".into()));
    assert_eq!(cursor_thread(&json!({})), None);
    let e = RpcError::Remote {
        code: -32600,
        message: "no rollout found for thread id 00000000-0000-7000-8000-000000000000".into(),
        data: None,
    };
    assert!(is_missing_thread(&e), "the real 0.156 answer");
    assert!(!is_missing_thread(&RpcError::Remote {
        code: -32600,
        message: "bad cwd".into(),
        data: None
    }));
    let input = turn_input(
        "fix it",
        &[
            json!({"path": "/tmp/shot.png"}),
            json!({"path": "/repo/notes.txt"}),
            json!({"url": "https://x/y.jpg", "mime": "image/jpeg"}),
        ],
    );
    let v = serde_json::to_value(&input).unwrap();
    assert_eq!(v[0]["type"], "text");
    assert!(v[0]["text"]
        .as_str()
        .unwrap()
        .contains("[attached file: /repo/notes.txt]"));
    assert_eq!(v[1], json!({"type": "localImage", "path": "/tmp/shot.png"}));
    assert_eq!(v[2]["type"], "image");
    assert_eq!(v[2]["url"], "https://x/y.jpg");
}

// ── driver end to end over a scripted app-server ────────────────────────

struct Peer {
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    out: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

impl Peer {
    async fn send(&mut self, v: Value) {
        self.out
            .write_all(format!("{v}\n").as_bytes())
            .await
            .unwrap();
    }

    /// Next message the client sent; answers the background probes
    /// (`account/read`, `account/rateLimits/read`) on the way.
    async fn next(&mut self) -> Value {
        loop {
            let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
                .await
                .expect("client went quiet")
                .unwrap()
                .expect("client closed");
            let v: Value = serde_json::from_str(&line).unwrap();
            match v.get("method").and_then(Value::as_str) {
                Some("account/read") => {
                    let id = v["id"].clone();
                    self.send(json!({"id": id, "result": {"account": {"type": "chatgpt", "email": null, "planType": "pro"}, "requiresOpenaiAuth": true}})).await;
                }
                Some("account/rateLimits/read") => {
                    let id = v["id"].clone();
                    self.send(json!({"id": id, "result": {"rateLimits": {"limitId": "codex", "primary": {"usedPercent": 10, "windowDurationMins": 300}}}})).await;
                }
                _ => return v,
            }
        }
    }

    /// Answers exactly the two probes a fresh session sends.
    async fn serve_probes(&mut self) {
        for method in ["account/read", "account/rateLimits/read"] {
            let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
                .await
                .expect("no probe")
                .unwrap()
                .unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["method"], method);
            let result = if method == "account/read" {
                json!({"account": {"type": "chatgpt", "email": null, "planType": "pro"}, "requiresOpenaiAuth": true})
            } else {
                json!({"rateLimits": {"limitId": "codex", "primary": {"usedPercent": 10, "windowDurationMins": 300}}})
            };
            self.send(json!({"id": v["id"], "result": result})).await;
        }
    }

    async fn expect(&mut self, method: &str) -> Value {
        let v = self.next().await;
        assert_eq!(v.get("method").and_then(Value::as_str), Some(method), "{v}");
        v
    }

    async fn handshake(&mut self) {
        let init = self.expect("initialize").await;
        assert_eq!(init["params"]["capabilities"]["experimentalApi"], true);
        self.send(json!({"id": init["id"], "result": {"userAgent": "omniget/0.156.0", "codexHome": "/h", "platformFamily": "unix", "platformOs": "macos"}})).await;
        self.expect("initialized").await;
    }
}

struct MockConnector {
    peers: mpsc::UnboundedSender<Peer>,
    connects: Arc<AtomicUsize>,
}

#[async_trait]
impl Connector for MockConnector {
    async fn connect(&self, spec: &LaunchSpec) -> Result<Connection, DriverError> {
        assert_eq!(spec.args[0], "app-server");
        self.connects.fetch_add(1, Ordering::SeqCst);
        let (client, server) = tokio::io::duplex(256 * 1024);
        let (cr, cw) = tokio::io::split(client);
        let (sr, sw) = tokio::io::split(server);
        let _ = self.peers.send(Peer {
            lines: BufReader::new(sr).lines(),
            out: sw,
        });
        Ok(Connection {
            reader: Box::new(cr),
            writer: Box::new(cw),
            process: None,
        })
    }
}

async fn until(rx: &mut mpsc::UnboundedReceiver<RuntimeEvent>, ty: &str) -> RuntimeEvent {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no {ty}"))
            .expect("sink closed");
        if ev.type_name() == ty {
            return ev;
        }
    }
}

#[tokio::test]
async fn driver_runs_a_turn_answers_an_approval_rolls_back_and_resumes_after_a_crash() {
    let (sink, mut events) = mpsc::unbounded_channel();
    let (peer_tx, mut peers) = mpsc::unbounded_channel();
    let connects = Arc::new(AtomicUsize::new(0));
    let driver = Arc::new(CodexDriver::new(
        DriverInstance::new("codex", "codex", "Codex"),
        sink,
        Arc::new(MockConnector {
            peers: peer_tx,
            connects: connects.clone(),
        }),
        None,
    ));

    // Session: initialize + thread/start with the approval-required table.
    let d = driver.clone();
    let open = tokio::spawn(async move {
        d.start_session(SessionStart {
            thread_id: "thr-omni".into(),
            instance_id: "codex".into(),
            cwd: Some("/work/repo".into()),
            model: None,
            agent_id: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
            resume_cursor: None,
        })
        .await
    });
    let mut peer = peers.recv().await.unwrap();
    peer.handshake().await;
    let start = peer.expect("thread/start").await;
    assert_eq!(start["params"]["approvalPolicy"], "untrusted");
    assert_eq!(start["params"]["sandbox"], "read-only");
    assert_eq!(start["params"]["approvalsReviewer"], "user");
    assert_eq!(start["params"]["cwd"], "/work/repo");
    peer.send(json!({"id": start["id"], "result": {"thread": {"id": "thr_codex_1"}, "model": "gpt-6-astra"}})).await;
    open.await.unwrap().unwrap();
    let started = until(&mut events, "session.started").await;
    match started.kind {
        RuntimeEventKind::SessionStarted(p) => {
            assert_eq!(p.resume, Some(json!({"threadId": "thr_codex_1"})))
        }
        _ => unreachable!(),
    }
    // The background probes: account/read, then the rate limits.
    peer.serve_probes().await;
    let limits = until(&mut events, "account.rate-limits.updated").await;
    assert_eq!(limits.thread_id, "thr-omni");

    // Turn in plan mode.
    let d = driver.clone();
    let turn = tokio::spawn(async move {
        d.start_turn(TurnStart {
            thread_id: "thr-omni".into(),
            turn_id: "eng-1".into(),
            message_id: "m1".into(),
            text: "fix main".into(),
            attachments: Vec::new(),
            model: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Plan,
        })
        .await
    });
    let ts = peer.expect("turn/start").await;
    assert_eq!(ts["params"]["threadId"], "thr_codex_1");
    assert_eq!(ts["params"]["input"][0]["text"], "fix main");
    assert_eq!(ts["params"]["sandboxPolicy"], json!({"type": "readOnly"}));
    assert_eq!(ts["params"]["collaborationMode"]["mode"], "plan");
    assert_eq!(
        ts["params"]["collaborationMode"]["settings"]["model"],
        "gpt-6-astra"
    );
    peer.send(json!({"id": ts["id"], "result": {"turn": {"id": "turn_1", "items": [], "status": "inProgress"}}})).await;
    let res = turn.await.unwrap().unwrap();
    assert_eq!(res.resume_cursor, Some(json!({"threadId": "thr_codex_1"})));

    peer.send(json!({"method": "turn/started", "params": {"threadId": "thr_codex_1", "turn": {"id": "turn_1", "items": [], "status": "inProgress"}}})).await;
    peer.send(json!({"method": "item/started", "params": {"threadId": "thr_codex_1", "turnId": "turn_1", "startedAtMs": 1, "item": {
        "type": "fileChange", "id": "item_f", "status": "inProgress",
        "changes": [{"path": "src/main.rs", "kind": {"type": "update", "move_path": null}, "diff": "@@ -1 +1 @@\n-a\n+b\n"}]}}})).await;
    peer.send(json!({"id": 0, "method": "item/fileChange/requestApproval", "params": {"threadId": "thr_codex_1", "turnId": "turn_1", "itemId": "item_f", "startedAtMs": 2}})).await;
    let opened = until(&mut events, "request.opened").await;
    assert_eq!(opened.turn_id.as_deref(), Some("eng-1"));
    match &opened.kind {
        RuntimeEventKind::RequestOpened(p) => assert!(p
            .detail
            .as_ref()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .contains("+b")),
        _ => unreachable!(),
    }
    let req = opened.request_id.clone().unwrap();
    driver
        .respond_request("thr-omni", &req, ApprovalDecision::Accept)
        .await
        .unwrap();
    let answer = peer.next().await;
    assert_eq!(answer, json!({"id": 0, "result": {"decision": "accept"}}));
    let resolved = until(&mut events, "request.resolved").await;
    assert_eq!(resolved.request_id.as_deref(), Some(req.as_str()));

    peer.send(json!({"method": "thread/tokenUsage/updated", "params": {"threadId": "thr_codex_1", "turnId": "turn_1", "tokenUsage": {
        "total": {"inputTokens": 100, "cachedInputTokens": 40, "outputTokens": 20, "reasoningOutputTokens": 5, "totalTokens": 120},
        "last": {"inputTokens": 100, "cachedInputTokens": 40, "outputTokens": 20, "reasoningOutputTokens": 5, "totalTokens": 120}}}})).await;
    peer.send(json!({"method": "turn/completed", "params": {"threadId": "thr_codex_1", "turn": {"id": "turn_1", "items": [], "status": "completed", "durationMs": 900}}})).await;
    let done = until(&mut events, "turn.completed").await;
    assert_eq!(done.turn_id.as_deref(), Some("eng-1"));
    match &done.kind {
        RuntimeEventKind::TurnCompleted(p) => {
            assert_eq!(p.state, TurnEndState::Completed);
            assert_eq!(p.usage.as_ref().unwrap().input_tokens, 60);
        }
        _ => unreachable!(),
    }

    // Interrupt with nothing running closes the engine's turn anyway.
    driver.interrupt("thr-omni", Some("eng-x")).await.unwrap();
    let aborted = until(&mut events, "turn.aborted").await;
    assert_eq!(aborted.turn_id.as_deref(), Some("eng-x"));

    // Rollback to 1 turn: revert before the second Codex turn.
    let d = driver.clone();
    let rb = tokio::spawn(async move { d.rollback("thr-omni", 1).await });
    let list = peer.expect("thread/turns/list").await;
    assert_eq!(list["params"]["sortDirection"], "asc");
    peer.send(json!({"id": list["id"], "result": {"data": [{"id": "turn_1", "items": [], "status": "completed"}, {"id": "turn_2", "items": [], "status": "completed"}, {"id": "turn_3", "items": [], "status": "completed"}], "nextCursor": null}})).await;
    let revert = peer.expect("thread/revert").await;
    assert_eq!(
        revert["params"],
        json!({"threadId": "thr_codex_1", "beforeTurnId": "turn_2"})
    );
    peer.send(json!({"id": revert["id"], "result": {"thread": {"id": "thr_codex_1"}}}))
        .await;
    rb.await.unwrap().unwrap();

    // Crash: the process goes away. The session is reported as exited.
    drop(peer);
    let exited = until(&mut events, "session.exited").await;
    match exited.kind {
        RuntimeEventKind::SessionExited(p) => {
            assert_eq!(p.exit_kind.as_deref(), Some("error"));
            assert_eq!(p.recoverable, Some(true));
        }
        _ => unreachable!(),
    }

    // Next turn respawns and resumes the same Codex thread, in full access now.
    let d = driver.clone();
    let turn = tokio::spawn(async move {
        d.start_turn(TurnStart {
            thread_id: "thr-omni".into(),
            turn_id: "eng-2".into(),
            message_id: "m2".into(),
            text: "again".into(),
            attachments: Vec::new(),
            model: Some("gpt-6-luna".into()),
            access_mode: AccessMode::FullAccess,
            interaction_mode: InteractionMode::Default,
        })
        .await
    });
    let mut peer = peers.recv().await.unwrap();
    peer.handshake().await;
    let resume = peer.expect("thread/resume").await;
    assert_eq!(resume["params"]["threadId"], "thr_codex_1");
    assert_eq!(resume["params"]["excludeTurns"], true);
    assert_eq!(resume["params"]["approvalPolicy"], "never");
    assert_eq!(resume["params"]["sandbox"], "danger-full-access");
    peer.send(json!({"id": resume["id"], "result": {"thread": {"id": "thr_codex_1"}, "model": "gpt-6-astra"}})).await;
    let ts = peer.expect("turn/start").await;
    assert_eq!(ts["params"]["model"], "gpt-6-luna");
    assert_eq!(
        ts["params"]["sandboxPolicy"],
        json!({"type": "dangerFullAccess"})
    );
    assert_eq!(
        ts["params"]["collaborationMode"]["mode"], "default",
        "a fresh process states the mode on its first turn"
    );
    peer.send(json!({"id": ts["id"], "result": {"turn": {"id": "turn_9", "items": [], "status": "inProgress"}}})).await;
    turn.await.unwrap().unwrap();
    assert_eq!(connects.load(Ordering::SeqCst), 2);

    // Stop: stdin closes, the exit is graceful.
    driver.stop("thr-omni").await.unwrap();
    // Only the background probe may still be in the pipe; then EOF.
    loop {
        let line = tokio::time::timeout(Duration::from_secs(5), peer.lines.next_line())
            .await
            .unwrap()
            .unwrap();
        match line {
            None => break,
            Some(l) => assert!(l.contains("\"account/"), "unexpected after stop: {l}"),
        }
    }
    drop(peer);
    let exited = until(&mut events, "session.exited").await;
    match exited.kind {
        RuntimeEventKind::SessionExited(p) => assert_eq!(p.exit_kind.as_deref(), Some("graceful")),
        _ => unreachable!(),
    }
    // An answer to a request of a dead session is an error (host: stale).
    assert!(driver
        .respond_request("thr-omni", "nope", ApprovalDecision::Accept)
        .await
        .is_err());
}

#[tokio::test]
async fn resume_of_a_thread_codex_lost_falls_back_to_a_new_one() {
    let (sink, mut events) = mpsc::unbounded_channel();
    let (peer_tx, mut peers) = mpsc::unbounded_channel();
    let driver = Arc::new(CodexDriver::new(
        DriverInstance::new("codex", "codex", "Codex"),
        sink,
        Arc::new(MockConnector {
            peers: peer_tx,
            connects: Arc::new(AtomicUsize::new(0)),
        }),
        None,
    ));
    let d = driver.clone();
    let open = tokio::spawn(async move {
        d.start_session(SessionStart {
            thread_id: "t".into(),
            instance_id: "codex".into(),
            cwd: None,
            model: None,
            agent_id: None,
            access_mode: AccessMode::Auto,
            interaction_mode: InteractionMode::Default,
            resume_cursor: Some(json!({"threadId": "gone"})),
        })
        .await
    });
    let mut peer = peers.recv().await.unwrap();
    peer.handshake().await;
    let resume = peer.expect("thread/resume").await;
    assert_eq!(resume["params"]["approvalsReviewer"], "auto_review");
    // The exact 0.156 answer for an unknown thread.
    peer.send(json!({"id": resume["id"], "error": {"code": -32600, "message": "no rollout found for thread id gone"}})).await;
    let start = peer.expect("thread/start").await;
    peer.send(json!({"id": start["id"], "result": {"thread": {"id": "fresh"}, "model": "m"}}))
        .await;
    open.await.unwrap().unwrap();
    until(&mut events, "runtime.warning").await;
    let started = until(&mut events, "session.started").await;
    match started.kind {
        RuntimeEventKind::SessionStarted(p) => {
            assert_eq!(p.resume, Some(json!({"threadId": "fresh"})))
        }
        _ => unreachable!(),
    }
}

/// Live check against the real CLI, no login needed: `initialize`,
/// `thread/start` and a failing turn. Run with
/// `OMNIGET_CODEX_BIN=/path/to/codex cargo test -p omniget-core codex_live -- --ignored`.
#[tokio::test]
#[ignore]
async fn codex_live_handshake_without_login() {
    let Ok(bin) = std::env::var("OMNIGET_CODEX_BIN") else {
        return;
    };
    let home = std::env::temp_dir().join(format!("omniget-codex-live-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let mut inst = DriverInstance::new("codex-live", "codex", "Codex");
    inst.command = Some(bin);
    inst.config_dir = Some(home.clone());
    let (sink, mut events) = mpsc::unbounded_channel();
    let driver = CodexDriver::new(
        inst,
        sink,
        Arc::new(super::supervisor::ProcessConnector),
        None,
    );
    driver
        .start_session(SessionStart {
            thread_id: "live".into(),
            instance_id: "codex-live".into(),
            cwd: Some(home.clone()),
            model: None,
            agent_id: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
            resume_cursor: None,
        })
        .await
        .unwrap();
    until(&mut events, "session.started").await;
    let auth = until(&mut events, "auth.status").await;
    assert!(matches!(auth.kind, RuntimeEventKind::AuthStatus(_)));
    driver
        .start_turn(TurnStart {
            thread_id: "live".into(),
            turn_id: "eng".into(),
            message_id: "m".into(),
            text: "say hi".into(),
            attachments: Vec::new(),
            model: None,
            access_mode: AccessMode::ApprovalRequired,
            interaction_mode: InteractionMode::Default,
        })
        .await
        .unwrap();
    let done = tokio::time::timeout(
        Duration::from_secs(90),
        until(&mut events, "turn.completed"),
    )
    .await
    .unwrap();
    assert!(
        matches!(done.kind, RuntimeEventKind::TurnCompleted(p) if p.state == TurnEndState::Failed)
    );
    driver.stop("live").await.unwrap();
    until(&mut events, "session.exited").await;
    let _ = std::fs::remove_dir_all(&home);
}
