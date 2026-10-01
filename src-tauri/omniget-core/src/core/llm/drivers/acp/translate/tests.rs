use super::*;
use crate::core::llm::drivers::{RequestType, RuntimeEventKind, StreamKind};
use serde_json::Value;

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
    let reasoning_ids: std::collections::BTreeSet<_> = emits
        .iter()
        .filter(|e| matches!(&e.kind, RuntimeEventKind::ContentDelta(p) if p.stream_kind == StreamKind::ReasoningText))
        .map(|e| e.item_id.clone().unwrap())
        .collect();
    assert_eq!(reasoning_ids.len(), 2, "two thought segments");
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
    assert_eq!(out.matches("total 8").count(), 1);
    let updates = emits
        .iter()
        .filter(|e| e.type_name() == "item.updated")
        .count();
    assert!((2..=6).contains(&updates), "{updates}");
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
