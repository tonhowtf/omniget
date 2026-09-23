//! Translator tests against real traffic recorded on 2026-09-22 (OpenCode
//! 1.18.32 `opencode acp`, Gemini CLI 0.60.0 `gemini --acp`), paths
//! anonymized.

use super::*;
use crate::core::llm::drivers::{RequestType, RuntimeEventKind, StreamKind};
use serde_json::{json, Value};

const TURN: &str = include_str!("../fixtures/opencode-turn.ndjson");
const TOOLS: &str = include_str!("../fixtures/opencode-tools.ndjson");
const PERMISSION: &str = include_str!("../fixtures/opencode-permission.ndjson");
const GEMINI: &str = include_str!("../fixtures/gemini-init-auth.ndjson");

fn lines(fixture: &str) -> Vec<Value> {
    fixture
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn inbound(fixture: &str) -> Vec<Value> {
    lines(fixture)
        .into_iter()
        .filter(|l| l["dir"] == "in")
        .map(|l| l["msg"].clone())
        .collect()
}

/// Run a recorded prompt through the translator; returns the emits and the
/// prompt response.
fn replay(fixture: &str) -> (Vec<Emit>, Value) {
    let prompt = lines(fixture)
        .into_iter()
        .find(|l| l["dir"] == "out" && l["msg"]["method"] == "session/prompt")
        .map(|l| {
            l["msg"]["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .unwrap();
    let mut t = Translator::new();
    t.begin_turn(&prompt);
    let mut out = Vec::new();
    let mut response = Value::Null;
    for msg in inbound(fixture) {
        if msg["method"] == "session/update" {
            out.extend(t.on_update(&msg["params"]["update"]));
        } else if msg["id"] == 3 && msg.get("result").is_some() {
            response = msg["result"].clone();
        }
    }
    out.extend(t.end_turn());
    (out, response)
}

fn text_of(emits: &[Emit], kind: StreamKind) -> String {
    emits
        .iter()
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::ContentDelta(p) if p.stream_kind == kind => Some(p.delta.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn text_turn_streams_reasoning_tool_and_answer() {
    let (emits, response) = replay(TURN);
    assert_eq!(
        text_of(&emits, StreamKind::ReasoningText),
        "Let me read the README.txt file in the current directory.The file contains \"hello from fixture\". I should reply with its exact contents, nothing else."
    );
    assert_eq!(
        text_of(&emits, StreamKind::AssistantText),
        "\n\nhello from fixture"
    );
    // The read tool: started + completed, typed, with its path and output.
    let started = emits
        .iter()
        .find(|e| matches!(&e.kind, RuntimeEventKind::ItemStarted(p) if p.tool_name.as_deref() == Some("read")))
        .expect("read started");
    assert_eq!(
        started.item_id.as_deref(),
        Some("call_c79dfdfc87e14269af3723e2")
    );
    let done = emits
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::ItemCompleted(p) if p.tool_name.as_deref() == Some("read") => {
                Some(p.clone())
            }
            _ => None,
        })
        .expect("read completed");
    assert_eq!(done.item_type, ItemType::DynamicToolCall);
    assert_eq!(done.status, Some(ItemStatus::Completed));
    assert_eq!(done.detail.as_deref(), Some("/work/demo/README.txt"));
    assert!(done.data.unwrap()["output"]["output"]
        .as_str()
        .unwrap()
        .contains("hello from fixture"));
    // Every started text item is completed with its full text.
    let completed_texts: Vec<String> = emits
        .iter()
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::ItemCompleted(p) if p.item_type == ItemType::AssistantMessage => {
                p.detail.clone()
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        completed_texts,
        vec!["\n\n".to_string(), "hello from fixture".to_string()]
    );
    let starts = emits
        .iter()
        .filter(|e| e.type_name() == "item.started")
        .count();
    let ends = emits
        .iter()
        .filter(|e| e.type_name() == "item.completed")
        .count();
    assert_eq!(starts, ends, "every item closes");
    // Ids are unique per item and stable across its deltas.
    let reasoning_ids: std::collections::BTreeSet<_> = emits
        .iter()
        .filter(|e| matches!(&e.kind, RuntimeEventKind::ContentDelta(p) if p.stream_kind == StreamKind::ReasoningText))
        .map(|e| e.item_id.clone().unwrap())
        .collect();
    assert_eq!(reasoning_ids.len(), 2, "two thought segments");
    // usage_update and the prompt response usage.
    let usage = emits
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::UsageUpdated(p) => Some(p.usage.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(usage.used_tokens, Some(8241));
    assert_eq!(usage.max_tokens, Some(200000));
    assert_eq!(usage.cost_usd, Some(0.0));
    let u = prompt_usage(&response).unwrap();
    assert_eq!(
        (u.input_tokens, u.output_tokens, u.cached_input_tokens),
        (305, 24, 7936)
    );
    assert_eq!(u.used_tokens, Some(8265));
    assert_eq!(
        turn_end(response["stopReason"].as_str(), false).0,
        TurnEndState::Completed
    );
    // available_commands_update → session.configured.
    assert!(emits.iter().any(|e| matches!(&e.kind, RuntimeEventKind::SessionConfigured(v) if v.value["availableCommands"].is_array())));
}

#[test]
fn tools_turn_types_edit_and_command_with_diff_and_output() {
    let (emits, _) = replay(TOOLS);
    let edit = emits
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::ItemCompleted(p) if p.tool_name.as_deref() == Some("edit") => {
                Some(p.clone())
            }
            _ => None,
        })
        .expect("edit completed");
    assert_eq!(edit.item_type, ItemType::FileChange);
    let data = edit.data.unwrap();
    let diff = data["diffs"][0]["unifiedDiff"].as_str().unwrap();
    assert!(
        diff.contains("-hello from fixture\n+hello edited"),
        "{diff}"
    );
    assert_eq!(data["paths"][0], "/work/demo/README.txt");
    let cmd = emits
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::ItemCompleted(p) if p.item_type == ItemType::CommandExecution => {
                Some(p.clone())
            }
            _ => None,
        })
        .expect("command completed");
    assert_eq!(cmd.detail.as_deref(), Some("ls -la"));
    let out = text_of(&emits, StreamKind::CommandOutput);
    assert!(out.contains("README.txt"), "{out}");
    // Streamed output is never duplicated: it equals the final snapshot.
    assert_eq!(out.matches("total 8").count(), 1);
    // item.updated carries only what changed (no repeated identical data).
    let updates = emits
        .iter()
        .filter(|e| e.type_name() == "item.updated")
        .count();
    assert!(updates >= 2 && updates <= 6, "{updates}");
}

#[test]
fn permission_requests_get_the_right_type_diff_and_options() {
    let asks: Vec<PermissionAsk> = inbound(PERMISSION)
        .into_iter()
        .filter(|m| m["method"] == "session/request_permission")
        .map(|m| permission_ask(&m["params"]))
        .collect();
    assert_eq!(asks.len(), 2);
    let edit = &asks[0];
    assert_eq!(edit.request_type, RequestType::FileChangeApproval);
    assert_eq!(edit.kind, "edit");
    assert!(edit
        .detail
        .diff
        .as_deref()
        .unwrap()
        .contains("-hello from fixture\n+hello edited\n"));
    assert_eq!(edit.detail.paths, vec!["/work/demo/README.txt".to_string()]);
    assert_eq!(edit.options.len(), 3);
    assert_eq!(edit.options[0].decision, Some(ApprovalDecision::Accept));
    assert_eq!(
        edit.options[1].decision,
        Some(ApprovalDecision::AcceptAlways)
    );
    assert_eq!(edit.options[2].decision, Some(ApprovalDecision::Decline));
    let exec = &asks[1];
    assert_eq!(exec.request_type, RequestType::CommandExecutionApproval);
    assert_eq!(exec.detail.command.as_deref(), Some("echo done"));
    assert_eq!(exec.memory_key, "execute:echo");
    for a in &asks {
        assert_ne!(a.request_type, RequestType::Unknown);
    }
    // Outcomes pick the agent's own option ids by kind.
    let opts: Vec<Value> = inbound(PERMISSION)
        .into_iter()
        .find(|m| m["method"] == "session/request_permission")
        .unwrap()["params"]["options"]
        .as_array()
        .unwrap()
        .clone();
    let pick = |d| permission_outcome(&opts, d)["outcome"].clone();
    assert_eq!(pick(ApprovalDecision::Accept)["optionId"], "once");
    assert_eq!(pick(ApprovalDecision::AcceptForSession)["optionId"], "once");
    assert_eq!(pick(ApprovalDecision::AcceptAlways)["optionId"], "always");
    assert_eq!(pick(ApprovalDecision::Decline)["optionId"], "reject");
    assert_eq!(pick(ApprovalDecision::Cancel)["outcome"], "cancelled");
    // The recorded client answers were exactly that.
    let answered: Vec<Value> = lines(PERMISSION)
        .into_iter()
        .filter(|l| l["dir"] == "out" && l["msg"]["result"]["outcome"].is_object())
        .map(|l| l["msg"]["result"].clone())
        .collect();
    assert_eq!(
        answered[0],
        permission_outcome(&opts, ApprovalDecision::Accept)
    );
}

#[test]
fn kinds_map_to_request_types() {
    assert_eq!(request_type("read"), RequestType::FileReadApproval);
    assert_eq!(request_type("delete"), RequestType::FileChangeApproval);
    assert_eq!(request_type("move"), RequestType::FileChangeApproval);
    assert_eq!(request_type("fetch"), RequestType::DynamicToolCall);
    assert_eq!(request_type("switch_mode"), RequestType::PermissionApproval);
    // No kind but a diff: an edit.
    let ask = permission_ask(&json!({
        "sessionId": "s", "options": [],
        "toolCall": { "toolCallId": "t", "title": "Write",
            "content": [{ "type": "diff", "path": "/w/a.txt", "oldText": null, "newText": "x\n" }] }
    }));
    assert_eq!(ask.request_type, RequestType::FileChangeApproval);
    assert!(ask.detail.diff.unwrap().starts_with("--- /dev/null"));
}

#[test]
fn gemini_asks_for_authentication_before_session_new() {
    let msgs = inbound(GEMINI);
    let init = &msgs[0]["result"];
    let methods: Vec<&str> = init["authMethods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        ["oauth-personal", "gemini-api-key", "vertex-ai", "gateway"]
    );
    assert_eq!(init["agentCapabilities"]["loadSession"], true);
    let err = super::super::rpc::RpcError::from_value(&msgs[1]["error"]);
    assert!(err.is_auth_required());
}

#[test]
fn replays_and_stray_chunks_outside_a_turn_are_dropped() {
    let mut t = Translator::new();
    let chunk = json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "old" } });
    assert!(t.on_update(&chunk).is_empty());
    t.begin_turn("hi");
    let replay = json!({ "sessionUpdate": "agent_message_chunk", "_meta": { "isReplay": true },
        "content": { "type": "text", "text": "old" } });
    assert!(t.on_update(&replay).is_empty());
    // The echo of our own prompt is not a new user message.
    let echo = json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "hi" } });
    assert!(t.on_update(&echo).is_empty());
}

#[test]
fn plan_mode_usage_and_terminal_output() {
    let mut t = Translator::new();
    t.begin_turn("go");
    let plan = t.on_update(&json!({ "sessionUpdate": "plan", "entries": [
        { "content": "a", "priority": "high", "status": "completed" },
        { "content": "b", "priority": "low", "status": "in_progress" }] }));
    match &plan[0].kind {
        RuntimeEventKind::PlanUpdated(p) => {
            assert_eq!(p.plan[0].status, "completed");
            assert_eq!(p.plan[1].status, "inProgress");
        }
        other => panic!("{other:?}"),
    }
    t.on_update(&json!({ "sessionUpdate": "current_mode_update", "currentModeId": "plan" }));
    assert_eq!(t.current_mode(), Some("plan"));
    // Terminal output before the tool call names the terminal is buffered.
    assert!(t.on_terminal_output("term-1", "hello ").is_empty());
    let ev = t.on_update(&json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "run",
        "kind": "execute", "status": "in_progress", "content": [{ "type": "terminal", "terminalId": "term-1" }] }));
    assert_eq!(text_of(&ev, StreamKind::CommandOutput), "hello ");
    let more = t.on_terminal_output("term-1", "world");
    assert_eq!(more[0].item_id.as_deref(), Some("c1"));
    // Cumulative cost → per-turn cost.
    t.on_update(&json!({ "sessionUpdate": "usage_update", "used": 1, "size": 10, "cost": { "amount": 0.5, "currency": "USD" } }));
    t.end_turn();
    t.begin_turn("again");
    t.on_update(&json!({ "sessionUpdate": "usage_update", "used": 2, "size": 10, "cost": { "amount": 0.75, "currency": "USD" } }));
    assert_eq!(t.turn_cost(), Some(0.25));
    // Unknown update kinds pass through as raw.
    let raw = t.on_update(&json!({ "sessionUpdate": "_vendor_thing", "x": 1 }));
    assert_eq!(raw[0].type_name(), "raw");
}

#[test]
fn elicitation_form_round_trip() {
    let params = json!({ "mode": "form", "sessionId": "s", "message": "Pick one",
        "requestedSchema": { "type": "object", "properties": {
            "color": { "type": "string", "title": "Color", "enum": ["red", "blue"] },
            "count": { "type": "integer", "title": "Count" },
            "ok": { "type": "boolean", "title": "OK?" } } } });
    let qs = elicitation_questions(&params);
    assert_eq!(qs.len(), 3);
    let color = qs.iter().find(|q| q.id == "color").unwrap();
    assert_eq!(color.options.len(), 2);
    assert!(!color.allow_custom_answer);
    let resp = elicitation_response(
        &params,
        &json!({ "color": "blue", "count": "3", "ok": "yes" }),
    );
    assert_eq!(
        resp,
        json!({ "action": "accept", "content": { "color": "blue", "count": 3, "ok": true } })
    );
    assert_eq!(
        elicitation_response(&params, &Value::Null),
        json!({ "action": "cancel" })
    );
}
