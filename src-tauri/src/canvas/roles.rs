use std::sync::Arc;

use omniget_core::core::canvas::model::CanvasNode;
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::roles::{self, RoleInput, Scope};
use omniget_core::core::canvas::store::{agent_origin, CanvasStore};
use serde::Serialize;
use serde_json::{json, Value};

use super::rpc::{NodeCtx, RpcResult};

pub const EVENT_ROLE: &str = "canvas://role";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEvent {
    pub workspace_id: String,
    pub floor_id: String,
    pub node_id: String,
    pub terminal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    pub restart: bool,
}

fn store() -> Arc<CanvasStore> {
    super::doc::host().store().clone()
}

pub fn restart_node(ws: &str, floor: &str, node: &CanvasNode, role_id: Option<String>) {
    let tid = node.data_str("terminalId").unwrap_or_default().to_string();
    let term = crate::pty::term();
    let alive = !tid.is_empty() && term.info(&tid).is_some_and(|i| i.alive);
    if alive {
        let _ = term.close(&tid, false);
    }
    super::chat::emit(
        EVENT_ROLE,
        RoleEvent {
            workspace_id: ws.to_string(),
            floor_id: floor.to_string(),
            node_id: node.id.clone(),
            terminal_id: tid,
            role_id,
            restart: alive,
        },
    );
}

pub fn assign(
    ws: &str,
    node_id: &str,
    role_id: Option<&str>,
    origin: &str,
) -> Result<Option<roles::Role>, String> {
    let s = store();
    let (floor, node) = super::chat::terminal_node(&s, ws, node_id)?;
    let role = match role_id.filter(|r| !r.is_empty()) {
        Some(r) => Some(roles::adopt(&s, ws, r)?),
        None => None,
    };
    let new_id = role.as_ref().map(|r| r.id.clone());
    let old_id = node.data_str("roleId").map(str::to_string);
    if old_id == new_id {
        return Ok(role);
    }
    let patch =
        json!({ "data": { "roleId": new_id.clone().map(Value::String).unwrap_or(Value::Null) } });
    s.apply(
        ws,
        &floor,
        vec![Op::update_node(node_id, patch)],
        origin,
        None,
    )?;
    restart_node(ws, &floor, &node, new_id);
    Ok(role)
}

pub fn nodes_using(s: &CanvasStore, ws: &str, role_id: &str) -> Vec<(String, CanvasNode)> {
    let Ok(meta) = s.workspace(ws) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in meta.floors {
        let _ = s.read(ws, &f.id, |d| {
            for n in d
                .nodes
                .iter()
                .filter(|n| n.is_terminal() && n.data_str("roleId") == Some(role_id))
            {
                out.push((f.id.clone(), n.clone()));
            }
        });
    }
    out
}

pub fn restart_holders(ws: &str, role_id: &str) {
    let s = store();
    for (floor, node) in nodes_using(&s, ws, role_id) {
        restart_node(ws, &floor, &node, Some(role_id.to_string()));
    }
}

pub fn unassign_all(ws: &str, role_id: &str, origin: &str) -> Result<usize, String> {
    let s = store();
    let holders = nodes_using(&s, ws, role_id);
    for (_, node) in &holders {
        assign(ws, &node.id, None, origin)?;
    }
    Ok(holders.len())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

fn arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn is_maestro(s: &CanvasStore, ctx: &NodeCtx) -> bool {
    if ctx
        .scopes
        .iter()
        .any(|x| x == "maestro" || x == "canvas:maestro")
    {
        return true;
    }
    super::chat::terminal_node(s, &ctx.workspace_id, &ctx.node_id)
        .ok()
        .and_then(|(_, n)| n.data.get("isManager").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

fn need_maestro(s: &CanvasStore, ctx: &NodeCtx) -> Result<(), String> {
    if is_maestro(s, ctx) {
        Ok(())
    } else {
        Err("CANVAS_DENIED: This terminal is not the Maestro".into())
    }
}

fn node_by_name(s: &CanvasStore, ws: &str, name: &str) -> Result<CanvasNode, String> {
    let meta = s.workspace(ws)?;
    let mut hits = Vec::new();
    for f in meta.floors {
        let _ = s.read(ws, &f.id, |d| {
            for n in d.nodes.iter().filter(|n| n.is_terminal()) {
                if n.id == name || n.title.as_deref() == Some(name) {
                    hits.push(n.clone());
                }
            }
        });
    }
    match hits.len() {
        0 => Err(format!("CANVAS_NOT_FOUND: no terminal named '{name}'")),
        1 => Ok(hits.remove(0)),
        _ => Err(format!(
            "CANVAS_INVALID: Multiple terminals named '{name}'. Rename them to have unique names."
        )),
    }
}

fn role_line(i: &roles::RoleInfo) -> String {
    let mut s = format!("- {}", i.role.name);
    if let Some(sum) = &i.role.summary {
        s.push_str(&format!(": {sum}"));
    }
    if !i.terminals_in_use.is_empty() {
        s.push_str(&format!(" (in use by {})", i.terminals_in_use.len()));
    }
    s
}

fn scope_of(args: &Value) -> Result<Option<Scope>, String> {
    match arg(args, &["scope"]) {
        None => Ok(None),
        Some(v) => Scope::parse(v)
            .map(Some)
            .ok_or_else(|| format!("CANVAS_INVALID: --scope {v} (current|global)")),
    }
}

fn ws_of(scope: Scope, ws: &str) -> Option<&str> {
    match scope {
        Scope::Workspace => Some(ws),
        Scope::Global => None,
    }
}

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    if !method.starts_with("role.") {
        return None;
    }
    let ctx = ctx.clone();
    let method = method.to_string();
    Some(blocking(move || rpc_sync(&ctx, &method, &args)).await)
}

fn rpc_sync(ctx: &NodeCtx, method: &str, args: &Value) -> Result<Value, String> {
    let s = store();
    let ws = ctx.workspace_id.as_str();
    let origin = agent_origin(&ctx.node_id);
    match method {
        "role.list" => {
            let list = roles::list(&s, ws);
            let mut text = String::from("Current workspace:\n");
            let mine: Vec<_> = list
                .iter()
                .filter(|i| i.scope == Scope::Workspace)
                .collect();
            if mine.is_empty() {
                text.push_str("  (none)\n");
            }
            for i in mine {
                text.push_str(&format!("  {}\n", role_line(i)));
            }
            text.push_str("Global:\n");
            for i in list.iter().filter(|i| i.scope == Scope::Global) {
                text.push_str(&format!("  {}\n", role_line(i)));
            }
            Ok(json!({ "roles": list, "text": text }))
        }
        "role.get" | "role.show" => {
            let name = arg(args, &["name", "role", "id"]).ok_or("CANVAS_INVALID: role name")?;
            let (role, scope) = roles::find(&s, ws, name)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: role '{name}'"))?;
            let text = format!(
                "{} ({})\n\n{}",
                role.name,
                if scope == Scope::Workspace {
                    "current workspace"
                } else {
                    "global"
                },
                role.prompt
            );
            Ok(json!({ "role": role, "scope": scope, "text": text }))
        }
        "role.assign" => {
            need_maestro(&s, ctx)?;
            let target = arg(args, &["target", "recruit", "node", "agent"])
                .ok_or("CANVAS_INVALID: target terminal")?;
            let node = node_by_name(&s, ws, target)?;
            let none = args.get("none").and_then(|v| v.as_bool()).unwrap_or(false);
            let role_id = if none {
                None
            } else {
                let name =
                    arg(args, &["role", "name"]).ok_or("CANVAS_INVALID: role name (or --none)")?;
                let (r, _) = roles::find(&s, ws, name)
                    .ok_or_else(|| format!("CANVAS_NOT_FOUND: role '{name}'"))?;
                Some(r.id)
            };
            let role = assign(ws, &node.id, role_id.as_deref(), &origin)?;
            let who = node.title.clone().unwrap_or_else(|| node.id.clone());
            let text = match &role {
                Some(r) => format!(
                    "OK: '{who}' now has the role '{}' and is restarting.",
                    r.name
                ),
                None => format!("OK: '{who}' has no role now and is restarting."),
            };
            Ok(json!({ "node": node.id, "role": role, "text": text }))
        }
        "role.create" => {
            need_maestro(&s, ctx)?;
            let scope = scope_of(args)?.unwrap_or(Scope::Workspace);
            let input = RoleInput {
                id: None,
                name: arg(args, &["name"])
                    .ok_or("CANVAS_INVALID: role name")?
                    .to_string(),
                prompt: args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                summary: arg(args, &["summary"]).map(str::to_string),
                icon: arg(args, &["icon"]).map(str::to_string),
                color: arg(args, &["color"]).map(str::to_string),
                agent: arg(args, &["agent"])
                    .map(omniget_core::core::canvas::model::AgentKind::parse),
            };
            if input.prompt.trim().is_empty() {
                return Err("CANVAS_INVALID: role prompt is required".into());
            }
            let role = roles::save(&s, ws_of(scope, ws), input)?;
            let text = format!("OK: created role '{}'.", role.name);
            Ok(json!({ "role": role, "text": text }))
        }
        "role.write" | "role.edit" => {
            need_maestro(&s, ctx)?;
            let name = arg(args, &["name", "role"]).ok_or("CANVAS_INVALID: role name")?;
            let (mut role, scope) = roles::find(&s, ws, name)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: role '{name}'"))?;
            let old_prompt = role.prompt.clone();
            if method == "role.write" {
                role.prompt = args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or("CANVAS_INVALID: prompt")?
                    .to_string();
            } else if let Some(old) = args.get("old").and_then(|v| v.as_str()) {
                let new = args.get("new").and_then(|v| v.as_str()).unwrap_or("");
                match role.prompt.matches(old).count() {
                    0 => {
                        return Err("CANVAS_INVALID: the old text was not found in the role".into())
                    }
                    1 => role.prompt = role.prompt.replacen(old, new, 1),
                    _ => return Err(
                        "CANVAS_INVALID: the old text appears more than once; give more context"
                            .into(),
                    ),
                }
            }
            let mut role = roles::save(
                &s,
                ws_of(scope, ws),
                RoleInput {
                    id: Some(role.id.clone()),
                    name: role.name.clone(),
                    prompt: role.prompt.clone(),
                    summary: role.summary.clone(),
                    icon: role.icon.clone(),
                    color: role.color.clone(),
                    agent: role.agent.clone(),
                },
            )?;
            if let Some(to) = scope_of(args)? {
                role = roles::set_scope(&s, ws, &role.id, to)?;
            }
            if role.prompt != old_prompt {
                restart_holders(ws, &role.id);
            }
            let text = format!("OK: role '{}' updated.", role.name);
            Ok(json!({ "role": role, "text": text }))
        }
        "role.delete" => {
            need_maestro(&s, ctx)?;
            let name = arg(args, &["name", "role"]).ok_or("CANVAS_INVALID: role name")?;
            let (role, scope) = roles::find(&s, ws, name)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: role '{name}'"))?;
            let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
            let used = nodes_using(&s, ws, &role.id).len();
            if used > 0 && !force {
                return Err(format!(
                    "CANVAS_CONFLICT: role '{}' is in use by {used} terminal(s); pass --force to unassign and delete",
                    role.name
                ));
            }
            if used > 0 {
                unassign_all(ws, &role.id, &origin)?;
            }
            roles::delete(&s, ws_of(scope, ws), &role.id)?;
            Ok(json!({ "text": format!("OK: role '{}' deleted.", role.name) }))
        }
        "role.discover" | "role.import" => {
            let dir = match arg(args, &["dir", "path"]) {
                Some(d) => std::path::PathBuf::from(d),
                None => s
                    .workspace(ws)?
                    .working_directory
                    .map(std::path::PathBuf::from)
                    .ok_or("CANVAS_INVALID: the workspace has no folder; pass --dir")?,
            };
            if method == "role.discover" {
                let found = roles::discover(&dir);
                let mut text = format!("Roles found under {}:\n", dir.display());
                if found.is_empty() {
                    text.push_str("  (none: no role.json in .omniget/roles or .maestri/roles)\n");
                }
                for r in &found {
                    text.push_str(&format!("  {}\n", r.name));
                }
                return Ok(json!({ "roles": found, "text": text }));
            }
            need_maestro(&s, ctx)?;
            let mut names: Vec<String> = args
                .get("names")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(n) = arg(args, &["name"]) {
                names.push(n.to_string());
            }
            if let Some(argv) = args.get("argv").and_then(|v| v.as_array()) {
                names.extend(argv.iter().filter_map(|x| x.as_str().map(str::to_string)));
            }
            let done = roles::import_discovered(&s, ws, &dir, &names)?;
            let mut text = String::new();
            for (r, updated) in &done {
                if *updated {
                    restart_holders(ws, &r.id);
                }
                text.push_str(&format!(
                    "OK: {} role '{}'.\n",
                    if *updated { "updated" } else { "imported" },
                    r.name
                ));
            }
            let roles: Vec<_> = done.into_iter().map(|(r, _)| r).collect();
            Ok(json!({ "roles": roles, "text": text }))
        }
        other => Err(format!("unknown canvas method `{other}`")),
    }
}
