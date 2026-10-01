use omniget_core::core::canvas::ops::Op;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::{
    any, apply, arr, b, blocking, def, list, need, out, ready, s, scoped, strs, terminal_node,
    to_value, ty, Def,
};
use crate::canvas::mcp_attach;

const READ: &str = "orchestration_read";
const CONTROL: &str = "orchestration_control";

pub(super) fn defs() -> Vec<Def> {
    let servers_out = out(json!({"servers": arr(ty("object"))}), &["servers"]);
    let attach_out = out(
        json!({"nodeId": ty("string"), "mcpServers": arr(ty("string")), "agent": ty("string"), "receives": ty("boolean"), "restartNeeded": ty("boolean"), "rev": ty("integer")}),
        &["nodeId", "mcpServers"],
    );
    vec![
        scoped(def("canvas_mcp_servers", "The MCP servers configured in OmniGet → LLM → MCP: id, name, transport (stdio or http), target (program or host, never secrets), enabled, connected and toolCount (known once connected). Attach them to a terminal with canvas_mcp_attach.", false, json!({}), &[], servers_out.clone()), READ),
        scoped(def("canvas_mcp_server_set", "Turn a configured MCP server on or off (off also disconnects it; terminals launched afterwards no longer receive it).", true, json!({"serverId": s("Server id from canvas_mcp_servers"), "enabled": b("On or off")}), &["serverId", "enabled"], servers_out), CONTROL),
        scoped(def("canvas_mcp_server_test", "Connect to a configured MCP server and list its tools: ok, server_name, protocol_version, tool_count, elapsed_ms, health and the error when it fails.", true, json!({"serverId": s("Server id from canvas_mcp_servers")}), &["serverId"], out(json!({"serverId": ty("string"), "result": any()}), &["serverId", "result"])), CONTROL),
        scoped(def("canvas_mcp_attach", "Give a Claude or Codex terminal access to configured MCP servers: the ids are stored on the node (data.mcpServers) and, at its next launch, Claude gets them through --mcp-config and Codex through -c mcp_servers.*, with secrets read from the keychain then. The terminal keeps OmniGet's own canvas access. A running terminal picks them up when it restarts (restart: true, or canvas_terminal_restart).", true, json!({"nodeId": s("Terminal node (id or unique title)"), "serverIds": strs("Server ids from canvas_mcp_servers"), "replace": b("Replace the current list instead of adding to it"), "restart": b("Restart a running terminal now so it picks them up")}), &["nodeId", "serverIds"], attach_out.clone()), CONTROL),
        scoped(def("canvas_mcp_detach", "Take MCP servers away from a terminal (all of them when serverIds is absent). A running terminal keeps them until it restarts.", true, json!({"nodeId": s("Terminal node (id or unique title)"), "serverIds": strs("Server ids to remove (default: all)"), "restart": b("Restart a running terminal now")}), &["nodeId"], attach_out), CONTROL),
    ]
}

pub(super) fn owns(name: &str) -> bool {
    matches!(
        name,
        "canvas_mcp_servers"
            | "canvas_mcp_server_set"
            | "canvas_mcp_server_test"
            | "canvas_mcp_attach"
            | "canvas_mcp_detach"
    )
}

pub(super) async fn call(app: &AppHandle, name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "canvas_mcp_servers" => Ok(json!({ "servers": to_value(mcp_attach::servers().await?)? })),
        "canvas_mcp_server_set" => {
            let id = need(a, "serverId")?;
            let on = a
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("MISSING_ARGUMENT: enabled")?;
            Ok(json!({ "servers": to_value(mcp_attach::set_enabled(&id, on).await?)? }))
        }
        "canvas_mcp_server_test" => {
            let id = need(a, "serverId")?;
            Ok(json!({ "serverId": id, "result": mcp_attach::test(&id).await? }))
        }
        _ => attach(app, name == "canvas_mcp_attach", a).await,
    }
}

async fn attach(app: &AppHandle, adding: bool, a: &Value) -> Result<Value, String> {
    let store = ready(app);
    let (ws, floor, node, tid) = terminal_node(&store, a, "nodeId")?;
    let given = list(a, "serverIds");
    let current = mcp_attach::attached(&node);
    let next: Vec<String> = if adding {
        let given = mcp_attach::known(&given)?;
        let mut v = if a.get("replace").and_then(Value::as_bool) == Some(true) {
            Vec::new()
        } else {
            current.clone()
        };
        for id in given {
            if !v.contains(&id) {
                v.push(id);
            }
        }
        v
    } else if a.get("serverIds").is_none() {
        Vec::new()
    } else {
        current
            .iter()
            .filter(|c| !given.contains(c))
            .cloned()
            .collect()
    };
    let value = if next.is_empty() {
        Value::Null
    } else {
        json!(next)
    };
    let id = node.id.clone();
    let op = Op::update_node(&id, json!({ "data": { mcp_attach::DATA_KEY: value } }));
    let (s2, w2, f2) = (store.clone(), ws.clone(), floor.clone());
    let rev = blocking(move || apply(&s2, &w2, &f2, vec![op])).await?.rev;
    if next.is_empty() {
        mcp_attach::forget(&id);
    }
    let agent = crate::canvas::launch::agent_of(&node);
    let running = crate::canvas::term::host(app).info(&tid).is_some();
    let mut pending = running && next != current;
    if pending && a.get("restart").and_then(Value::as_bool) == Some(true) {
        let args = json!({ "nodeId": id, "workspaceId": ws });
        Box::pin(super::dispatch(app, "canvas_terminal_restart", args)).await?;
        pending = false;
    }
    Ok(json!({
        "nodeId": id,
        "mcpServers": next,
        "agent": agent,
        "receives": matches!(agent.as_str(), "claude" | "codex"),
        "restartNeeded": pending,
        "rev": rev,
    }))
}
