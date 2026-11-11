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
