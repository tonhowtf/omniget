use serde_json::{json, Value};
use tauri::AppHandle;

use super::{arr, b, def, flag, i, out, ready, s, st, ty, ws, ws_floor, ws_of, Def};

const MAX: usize = 500;
const DEFAULT_LIMIT: usize = 30;
const DEFAULT_BYTES: usize = 32 * 1024;

pub(super) fn defs() -> Vec<Def> {
    vec![def(
        "canvas_activity",
        "Recent writes on a workspace, oldest first: who did what and where. Each entry has origin (user = the person, mcp:<client>, agent:<node>; a trailing :undo marks an undo), actor (display name), verb (created, deleted, moved, resized, renamed, edited, wrote, corded, uncorded, drew, erased...), summary (\"created note 'Plan'\", \"moved 3 nodes\") and nodeIds. Your own writes carry canvas_start.you.origin. Entries are compact {id, at, origin, actor, verb, summary, nodeIds, floorId (absent when floorId is given)}. The answer stays under maxBytes, keeping the newest entries (truncated: true when older ones were left out). Pass since = the cursor of the previous answer to get only what is new.",
        false,
        json!({
            "workspaceId": ws(),
            "floorId": s("Only this floor (default: every floor of the workspace)"),
            "since": i("Cursor of a previous answer: only newer entries", 0, 9_007_199_254_740_991),
            "limit": i("Entries (default 30)", 1, MAX as u64),
            "maxBytes": i("Size cap of the entries in bytes (default 32768)", 1024, 1_048_576),
            "origin": s("Only this author: an exact origin or a prefix such as mcp: or agent:"),
            "excludeUser": b("Leave out what the person did by hand"),
            "mine": b("Only your own writes")
        }),
        &[],
        out(
            json!({"workspaceId": ty("string"), "entries": arr(ty("object")), "cursor": ty("integer"), "truncated": ty("boolean")}),
            &["entries", "cursor"],
        ),
    )]
}

pub(super) fn owns(name: &str) -> bool {
    name == "canvas_activity"
}

pub(super) async fn call(app: &AppHandle, _name: &str, a: &Value) -> Result<Value, String> {
    let store = ready(app);
    let ws = ws_of(a);
    let floor = match st(a, "floorId") {
        Some(_) => Some(ws_floor(&store, a)?.1),
        None => None,
    };
    let since = a.get("since").and_then(Value::as_u64);
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| l as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX);
    let max_bytes = a
        .get("maxBytes")
        .and_then(Value::as_u64)
        .map(|b| b as usize)
        .unwrap_or(DEFAULT_BYTES)
        .clamp(1024, 1_048_576);
    let all = crate::canvas::activity::recent(&ws, floor.as_deref(), since, MAX);
    let cursor = all.last().map(|e| e.id).or(since).unwrap_or(0);
    let me = super::who::origin();
    let only = st(a, "origin");
    let mine = flag(a, "mine") == Some(true);
    let no_user = flag(a, "excludeUser") == Some(true);
    let mut picked: Vec<_> = all
        .into_iter()
        .filter(|e| {
            let base = e.origin.strip_suffix(":undo").unwrap_or(&e.origin);
            (!mine || base == me)
                && (!no_user || base != "user")
                && only
                    .as_deref()
                    .is_none_or(|o| base == o || (o.ends_with(':') && base.starts_with(o)))
        })
        .collect();
    let mut truncated = false;
    if picked.len() > limit {
        picked.drain(..picked.len() - limit);
        truncated = true;
    }
    let scoped = floor.is_some();
    let mut entries: Vec<Value> = Vec::with_capacity(picked.len());
    let mut used = 0usize;
    for e in picked.iter().rev() {
        let mut v = json!({
            "id": e.id, "at": e.at, "origin": e.origin, "actor": e.actor,
            "verb": e.verb, "summary": e.summary, "nodeIds": e.node_ids,
        });
        if !scoped {
            v["floorId"] = json!(e.floor_id);
        }
        let size = v.to_string().len() + 1;
        if used + size > max_bytes && !entries.is_empty() {
            truncated = true;
            break;
        }
        used += size;
        entries.push(v);
    }
    entries.reverse();
    Ok(json!({ "workspaceId": ws, "entries": entries, "cursor": cursor, "truncated": truncated }))
}
