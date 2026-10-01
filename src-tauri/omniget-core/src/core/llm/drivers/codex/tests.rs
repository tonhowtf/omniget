//! Tests of the Codex driver against the real 0.156 wire (no login) and a
//! turn built from the schema (see `fixtures/SOURCES.txt`).

use serde_json::{json, Value};

use super::super::{
    ApprovalDecision, ErrorClass, ItemStatus, ItemType, RequestType, RuntimeEvent,
    RuntimeEventKind, StreamKind, TurnEndState,
};
use super::protocol as p;
use super::translate::{AutoReply, Translator};

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

// ── pure helpers ────────────────────────────────────────────────────────

// ── driver end to end over a scripted app-server ────────────────────────
