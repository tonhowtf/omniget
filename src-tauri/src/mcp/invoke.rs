//! Reaches every command of the app. The call runs in the main window, so it behaves exactly as
//! if the interface had made it.

use super::access;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use tauri::AppHandle;

const DEFAULT_TIMEOUT_S: u64 = 120;
const MAX_TIMEOUT_S: u64 = 600;
/// These carry the bridge itself; calling them through it would cut the line.
const RESERVED: &[&str] = &["native_agent_register", "native_agent_reply"];
const DENIED: &[&str] = &[
    "cookies_read",
    "cookies_export_to",
    "get_bridge_info",
    "rotate_bridge_token",
    "bridge_open_pairing",
    "guildchat_login_token",
    "canvas_term_ensure",
    "canvas_remote_pair_code",
    "connect_answer",
    "connect_tool",
    "connect_ai_create",
    "tool_mcp_access_set",
    "tool_mcp_client_create",
    "tool_mcp_auth_request_answer",
    "tool_mcp_execution_grant",
    "tool_mcp_gateway_grant",
    "tool_mcp_network_grant",
    "tool_mcp_root_grant",
    "washmybelly_call",
    "washmybelly_approval_answer",
    "social_approval_answer",
    "phonechat_send_file",
    "phonechat_contacts_export",
    "llm_tool_answer",
    "llm_budget_resolve",
    "canvas_checkpoint_restore",
    "canvas_checkpoint_undo",
];
const DENIED_PREFIXES: &[&str] = &["pty_", "term_", "secret", "quicktools_clipboard"];
/// Screen commands that speak for the person; the tool beside each one asks the owner first.
const HAS_TOOL: &[(&str, &str)] = &[
    ("phonechat_send_text", "wa_send"),
    ("phonechat_schedule", "wa_send"),
    ("phonechat_react", "wa_react"),
    ("phonechat_rule_save", "wa_rules"),
    ("phonechat_rule_enable", "wa_rules"),
    ("phonechat_rules_limits", "wa_rules"),
    ("social_call", "social_status"),
    ("telegram_call", "tg_status"),
];

fn tool_for(name: &str) -> Option<&'static str> {
    HAS_TOOL.iter().find(|(c, _)| *c == name).map(|(_, t)| *t)
}

fn denied(name: &str) -> bool {
    DENIED.contains(&name)
        || DENIED_PREFIXES.iter().any(|p| name.starts_with(p))
        || tool_for(name).is_some()
}

fn catalog() -> &'static Vec<Value> {
    static CATALOG: OnceLock<Vec<Value>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("command_catalog.json")).unwrap_or_default()
    })
}

pub fn areas() -> Vec<Value> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for c in catalog() {
        *counts.entry(c["area"].as_str().unwrap_or("")).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(area, commands)| json!({"area": area, "commands": commands}))
        .collect()
}

pub fn commands(a: &Value) -> Result<Value, String> {
    let disabled = access::get().disabled_areas;
    let open = |c: &&Value| !disabled.iter().any(|d| c["area"] == d.as_str());
    let query = a["query"].as_str().unwrap_or("").trim().to_lowercase();
    let area = a["area"].as_str().unwrap_or("");
    if query.is_empty() && area.is_empty() {
        let list: Vec<Value> = areas()
            .into_iter()
            .filter(|v| !disabled.iter().any(|d| v["area"] == d.as_str()))
            .collect();
        return Ok(json!({
            "areas": list,
            "next": "Pass area or query to list commands, then call one with omniget_invoke.",
        }));
    }
    let matches: Vec<&Value> = catalog()
        .iter()
        .filter(open)
        .filter(|c| !c["name"].as_str().is_some_and(denied))
        .filter(|c| area.is_empty() || c["area"] == area)
        .filter(|c| {
            query.is_empty()
                || query
                    .split_whitespace()
                    .all(|w| c.to_string().to_lowercase().contains(w))
        })
        .collect();
    let cursor = a["cursor"].as_u64().unwrap_or(0) as usize;
    let limit = a["limit"].as_u64().unwrap_or(100).clamp(1, 500) as usize;
    let page: Vec<&Value> = matches.iter().skip(cursor).take(limit).copied().collect();
    Ok(json!({
        "total": matches.len(),
        "commands": page,
        "nextCursor": (cursor + limit < matches.len()).then_some(cursor + limit),
    }))
}

pub async fn invoke(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let name = a["command"].as_str().unwrap_or("").trim();
    if let Some(tool) = tool_for(name) {
        return Err(format!(
            "COMMAND_HAS_TOOL: `{name}` is the screen's side of a dedicated tool; start from `{tool}` and call the dedicated tools instead"
        ));
    }
    if denied(name) {
        return Err(format!(
            "COMMAND_DENIED: `{name}` hands out credentials, answers an approval or drives a terminal, so it is not reachable through MCP; ask the person to do it in the OmniGet window"
        ));
    }
    let entry = catalog()
        .iter()
        .find(|c| c["name"] == name)
        .ok_or_else(|| format!("UNKNOWN_COMMAND: `{name}`; find it with omniget_commands"))?;
    let area = entry["area"].as_str().unwrap_or("");
    if RESERVED.contains(&name) {
        return Err(format!("COMMAND_RESERVED: `{name}` runs this bridge"));
    }
    if entry["channel"] == true {
        return Err(format!(
            "COMMAND_STREAMS: `{name}` needs a live channel; use the canvas_terminal_* tools"
        ));
    }
    if access::get().disabled_areas.iter().any(|d| d == area) {
        return Err(format!("AREA_DISABLED: `{area}` is turned off in OmniGet"));
    }
    let args = match &a["args"] {
        Value::Null => json!({}),
        v @ Value::Object(_) => v.clone(),
        _ => return Err("INVALID_ARGUMENT: args must be an object".into()),
    };
    let seconds = a["timeoutSeconds"]
        .as_u64()
        .unwrap_or(DEFAULT_TIMEOUT_S)
        .clamp(1, MAX_TIMEOUT_S);
    let window = a["windowId"].as_str().unwrap_or("main");
    let reply = crate::native_agent::bridge_for(
        app,
        window,
        "invoke",
        json!({"command": name, "args": args}),
        seconds,
    )
    .await?;
    if let Some(error) = reply.get("invokeError") {
        return Err(error
            .as_str()
            .map_or_else(|| error.to_string(), str::to_owned));
    }
    if let Some(code) = reply["error"]["code"].as_str() {
        return Err(code.to_string());
    }
    Ok(json!({"command": name, "result": reply["result"]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered() -> Vec<&'static str> {
        include_str!("../handlers.rs")
            .lines()
            .filter_map(|l| l.trim().strip_suffix(','))
            .filter(|l| l.contains("::") && !l.contains(' '))
            .filter_map(|l| l.rsplit("::").next())
            .collect()
    }

    #[test]
    fn catalog_lists_every_registered_command() {
        let known: Vec<&str> = catalog()
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect();
        let missing: Vec<&str> = registered()
            .into_iter()
            .filter(|n| !known.contains(n))
            .collect();
        assert!(
            missing.is_empty(),
            "command_catalog.json is stale, run node scripts/mcp-commands.mjs: {missing:?}"
        );
        assert!(known.len() >= 1000);
    }

    #[test]
    fn credentials_approvals_and_terminals_are_denied() {
        for name in [
            "cookies_read",
            "cookies_export_to",
            "get_bridge_info",
            "rotate_bridge_token",
            "guildchat_login_token",
            "pty_open",
            "pty_write",
            "term_text",
            "secret_get",
            "secrets_list",
            "llm_budget_resolve",
            "canvas_checkpoint_restore",
            "canvas_checkpoint_undo",
            "connect_answer",
            "washmybelly_approval_answer",
            "tool_mcp_access_set",
        ] {
            assert!(denied(name), "{name}");
        }
        for name in ["music_status", "cookies_list", "empty", ""] {
            assert!(!denied(name), "{name}");
        }
    }

    #[test]
    fn the_clipboard_history_cannot_be_read_or_driven_through_invoke() {
        for name in ["quicktools_clipboard", "quicktools_clipboard_act"] {
            assert!(denied(name), "{name}");
            assert!(
                catalog().iter().any(|c| c["name"] == name),
                "{name} is not in command_catalog.json"
            );
            let found = commands(&json!({"query": name})).unwrap();
            assert!(found["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["name"] != name));
        }
        assert!(!denied("quicktools_status"));
    }

    #[test]
    fn screen_sends_are_not_a_way_around_asking_first() {
        for (command, tool) in HAS_TOOL {
            assert!(denied(command), "{command}");
            assert!(
                catalog().iter().any(|c| c["name"] == *command),
                "{command} is not in command_catalog.json"
            );
            assert!(
                crate::mcp::policy::scope(tool).is_some(),
                "{tool} is not a tool"
            );
            let found = commands(&json!({"query": command})).unwrap();
            assert!(found["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["name"] != *command));
        }
    }

    #[test]
    fn telegram_writes_cannot_skip_the_approval_through_invoke() {
        assert!(denied("telegram_call"));
        assert_eq!(tool_for("telegram_call"), Some("tg_status"));
    }

    #[test]
    fn every_denied_name_is_a_real_command() {
        for name in DENIED {
            assert!(
                catalog().iter().any(|c| c["name"] == *name),
                "{name} is not in command_catalog.json"
            );
        }
    }

    #[test]
    fn denied_commands_stay_out_of_the_listing() {
        for query in ["cookies_read", "pty_open", "rotate_bridge_token"] {
            let found = commands(&json!({"query": query})).unwrap();
            let listed = found["commands"].as_array().unwrap();
            assert!(listed.iter().all(|c| c["name"] != query), "{query}");
        }
        let cookies = commands(&json!({"area": "cookies"})).unwrap();
        assert!(cookies["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "cookies_list"));
    }

    #[test]
    fn listing_starts_with_areas_and_filters_by_query() {
        assert!(
            commands(&json!({})).unwrap()["areas"]
                .as_array()
                .unwrap()
                .len()
                > 20
        );
        let found = commands(&json!({"query": "music_status"})).unwrap();
        assert_eq!(found["commands"][0]["name"], "music_status");
        assert_eq!(found["commands"][0]["area"], "music");
    }
}
