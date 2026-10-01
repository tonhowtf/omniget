use std::collections::HashMap;

use omniget_core::core::canvas::layout;
use omniget_core::core::canvas::model::Point;
use omniget_core::core::canvas::partituras::{self, StampOptions};
use omniget_core::core::canvas::store;
use serde_json::{json, Value};

use super::rpc::{NodeCtx, RpcResult};

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    let r = match method {
        "partitura.list" => list(ctx, args),
        "partitura.stamp" => stamp(ctx, args),
        _ => return None,
    };
    Some(r)
}

fn list(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    let s = super::doc::host().store().clone();
    let q = args.get("query").and_then(Value::as_str);
    let items = partituras::list(&s, Some(&ctx.workspace_id), q);
    Ok(json!(items
        .iter()
        .map(|p| json!({
            "id": p.id,
            "name": p.name,
            "summary": p.summary,
            "roles": p.role_names,
            "terminals": p.terminal_count,
            "nodes": p.node_count,
        }))
        .collect::<Vec<_>>()))
}

fn stamp(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    let (doc, me) = super::maestro::require_maestro(ctx)?;
    let s = super::doc::host().store().clone();
    let key = args
        .get("name")
        .or_else(|| args.get("id"))
        .and_then(Value::as_str)
        .ok_or("CANVAS_INVALID: partitura name or id is required")?;
    let all = partituras::list(&s, Some(&ctx.workspace_id), None);
    let hit = all
        .iter()
        .find(|p| p.id == key)
        .or_else(|| all.iter().find(|p| p.name.eq_ignore_ascii_case(key.trim())))
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no partitura named '{key}'"))?;
    let p = partituras::get(&s, &hit.id)?;
    let at = layout::free_spot_in(
        &doc,
        p.doc.bounds.w.max(1.0),
        p.doc.bounds.h.max(1.0),
        Some(Point {
            x: me.x,
            y: me.y + me.h + 80.0,
        }),
        &[],
    );
    let r = partituras::stamp(
        &s,
        &ctx.workspace_id,
        &ctx.floor_id,
        &p,
        at,
        &StampOptions {
            commands: HashMap::new(),
            roles: HashMap::new(),
            apply_hooks: false,
            origin: Some(store::agent_origin(&ctx.node_id)),
            accept_chat: args
                .get("acceptChat")
                .or_else(|| args.get("withChat"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
    )?;
    let cmds: Vec<String> = p
        .doc
        .nodes
        .iter()
        .filter(|n| n.is_terminal())
        .map(|n| {
            let who = n.title.clone().unwrap_or_else(|| n.id.clone());
            let what = match n.data.get("command") {
                Some(Value::String(c)) if !c.trim().is_empty() => c.clone(),
                Some(Value::Array(a)) if !a.is_empty() => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => n.data_str("agent").unwrap_or("shell").to_string(),
            };
            format!("  {who}: {what}")
        })
        .collect();
    let mut text = format!(
        "OK: stamped '{}' ({} nodes) under you.",
        p.doc.name,
        r.node_ids.len()
    );
    if !cmds.is_empty() {
        text.push_str("\nStarted terminals (their commands run now):\n");
        text.push_str(&cmds.join("\n"));
    }
    Ok(json!({ "nodeIds": r.node_ids, "name": p.doc.name, "commands": cmds, "text": text }))
}
