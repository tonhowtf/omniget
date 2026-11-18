use std::collections::HashMap;

use omniget_core::core::canvas::envs;
use omniget_core::core::canvas::export::{self, ExportOptions, Selection};
use omniget_core::core::canvas::floors;
use omniget_core::core::canvas::layout;
use omniget_core::core::canvas::lead::{self as core_lead, RecruitSpec, NOTIFY_MAX, NOT_LEAD};
use omniget_core::core::canvas::model::{
    new_id, CanvasNode, FloorDoc, NodeKind, Point, WorkspaceInput, GROUND_FLOOR_ID,
};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::partituras;
use omniget_core::core::canvas::store::{self, CanvasStore};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::rpc::{NodeCtx, RpcResult};

fn store() -> std::sync::Arc<CanvasStore> {
    super::doc::host().store().clone()
}

pub fn require_lead(ctx: &NodeCtx) -> Result<(FloorDoc, CanvasNode), String> {
    let s = store();
    let doc = s.read(&ctx.workspace_id, &ctx.floor_id, |d| d.clone())?;
    let me = doc
        .node(&ctx.node_id)
        .cloned()
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {}", ctx.node_id))?;
    if !core_lead::is_lead(&me) {
        return Err(NOT_LEAD.into());
    }
    Ok((doc, me))
}

pub fn set_lead(ws: &str, floor: &str, node_id: &str, on: bool) -> Result<(), String> {
    let s = store();
    let is_term = s
        .read(ws, floor, |d| d.node(node_id).map(|n| n.is_terminal()))?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    if !is_term {
        return Err("CANVAS_INVALID: only terminals can be the Lead".into());
    }
    let patch =
        json!({ "data": { "isManager": if on { Value::Bool(true) } else { Value::Null } } });
    s.apply(
        ws,
        floor,
        vec![Op::update_node(node_id, patch)],
        store::ORIGIN_USER,
        None,
    )?;
    Ok(())
}

fn args<T: for<'de> Deserialize<'de> + Default>(v: Value) -> Result<T, String> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).map_err(|e| format!("CANVAS_INVALID: {e}"))
}

pub async fn rpc(ctx: &NodeCtx, method: &str, a: Value) -> RpcResult {
    let legacy = method
        .strip_prefix("maestro.")
        .map(|rest| format!("lead.{rest}"));
    let method = legacy.as_deref().unwrap_or(method);
    if !method.starts_with("lead.") {
        return None;
    }
    let r = match method {
        "lead.recruit" => recruit(ctx, a).await,
        "lead.connect" => connect(ctx, a),
        "lead.disconnect" => disconnect(ctx, a),
        "lead.swap_role" => swap_role(ctx, a),
        "lead.swap_agent" => swap_agent(ctx, a),
        "lead.dismiss" => dismiss(ctx, a),
        "lead.routine.create" => super::routines::rpc_create(ctx, a).await,
        "lead.workspace.create" => workspace_create(ctx, a).await,
        "lead.floor.create" => super::floors::rpc_create(ctx, a).await,
        "lead.notify" => notify(ctx, a),
        "lead.status" => status(ctx),
        "lead.preset_list" | "lead.presets" => preset_list(),
        _ => return None,
    };
    Some(r)
}

fn preset_list() -> Result<Value, String> {
    let mut text = String::from("Built-in agents:\n");
    for a in omniget_core::core::canvas::model::AgentKind::KNOWN {
        text.push_str(&format!("  {a}\n"));
    }
    let mut presets = Vec::new();
    if let Some(app) = super::chat::app() {
        let c = super::prefs::get(app);
        if !c.agent_presets.is_empty() {
            text.push_str("Your presets:\n");
        }
        let mut names: Vec<_> = c.agent_presets.keys().cloned().collect();
        names.sort_by_key(|n| n.to_lowercase());
        for name in names {
            let p = &c.agent_presets[&name];
            text.push_str(&format!(
                "  {name} ({}: {})\n",
                super::prefs::preset_agent(p),
                p.command
            ));
            presets.push(json!({ "name": name, "agent": super::prefs::preset_agent(p), "command": p.command }));
        }
    }
    Ok(json!({
        "builtins": omniget_core::core::canvas::model::AgentKind::KNOWN,
        "presets": presets,
        "text": text,
    }))
}

fn user_preset_patch(key: &str) -> Option<(String, Value)> {
    let app = super::chat::app()?;
    let (name, p) = super::prefs::preset(app, key)?;
    let mut n = core_lead::recruit_node(
        None,
        &RecruitSpec::default(),
        "x",
        Point { x: 0.0, y: 0.0 },
        0.0,
    );
    super::prefs::apply_preset(&mut n, &name, &p);
    let mut data = n.data.clone();
    if let Some(m) = data.as_object_mut() {
        m.remove("isManager");
        m.remove("terminalId");
        m.remove("cwd");
        if p.cwd.as_deref().is_some_and(|c| !c.trim().is_empty()) {
            m.insert("cwd".into(), n.data["cwd"].clone());
        }
    }
    let mut patch = json!({ "data": data });
    if let Some(c) = n.color {
        patch["color"] = json!(c);
    }
    if let Some(i) = n.icon {
        patch["icon"] = json!(i);
    }
    Some((name, patch))
}

fn merge_patch(node: &mut CanvasNode, patch: &Value) {
    if let (Some(dst), Some(src)) = (node.data.as_object_mut(), patch["data"].as_object()) {
        for (k, v) in src {
            if v.is_null() {
                dst.remove(k);
            } else {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(c) = patch["color"].as_str() {
        node.color = Some(c.to_string());
    }
    if let Some(i) = patch["icon"].as_str() {
        node.icon = Some(i.to_string());
    }
}

fn status(ctx: &NodeCtx) -> Result<Value, String> {
    let s = store();
    let doc = s.read(&ctx.workspace_id, &ctx.floor_id, |d| d.clone())?;
    let me = doc.node(&ctx.node_id).ok_or("CANVAS_NOT_FOUND: caller")?;
    Ok(json!({ "lead": core_lead::is_lead(me), "name": core_lead::name_of(me) }))
}

fn resolve_workspace(s: &CanvasStore, key: &str) -> Result<String, String> {
    let all = s.workspaces();
    all.iter()
        .find(|w| w.workspace.id == key)
        .or_else(|| {
            all.iter()
                .find(|w| w.workspace.name.eq_ignore_ascii_case(key.trim()))
        })
        .map(|w| w.workspace.id.clone())
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no workspace named '{key}'"))
}

fn resolve_role(s: &CanvasStore, ws: &str, key: &str) -> Result<String, String> {
    let roles = partituras::local_roles(s, ws);
    roles
        .iter()
        .find(|r| r.id == key)
        .or_else(|| {
            roles
                .iter()
                .find(|r| r.name.eq_ignore_ascii_case(key.trim()))
        })
        .map(|r| r.id.clone())
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no role named '{key}' (see `role list`)"))
}

fn close_terminal(node: &CanvasNode) {
    if let Some(t) = node.data_str("terminalId") {
        let _ = crate::pty::term().close(t, true);
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RecruitArgs {
    name: Option<String>,
    agent: Option<String>,
    preset: Option<String>,
    command: Option<String>,
    role: Option<String>,
    floor: Option<String>,
    workspace: Option<String>,
    dir: Option<String>,
    replace: Option<String>,
}

async fn recruit(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let (doc, me) = require_lead(ctx)?;
    let a: RecruitArgs = args(a)?;
    let s = store();
    let origin = store::agent_origin(&ctx.node_id);
    let ws = match &a.workspace {
        Some(w) => resolve_workspace(&s, w)?,
        None => ctx.workspace_id.clone(),
    };
    let floor = match &a.floor {
        Some(f) => super::floors::resolve_floor(&s, &ws, f)?,
        None if ws == ctx.workspace_id => ctx.floor_id.clone(),
        None => GROUND_FLOOR_ID.to_string(),
    };
    let role_id = a
        .role
        .as_deref()
        .map(|r| resolve_role(&s, &ws, r))
        .transpose()?;
    let user_preset = a
        .preset
        .as_deref()
        .or(a.agent.as_deref())
        .and_then(user_preset_patch);
    if user_preset.is_none() {
        if let Some(p) = a.preset.as_deref() {
            if !omniget_core::core::canvas::model::AgentKind::parse(p).is_known() {
                return Err(format!(
                    "CANVAS_INVALID: unknown preset '{p}' (run `omniget canvas preset list`)"
                ));
            }
        }
    }
    let agent = if user_preset.is_some() {
        None
    } else {
        a.agent.clone().or(a.preset.clone())
    };

    if let Some(old) = &a.replace {
        if a.floor.is_some() || a.workspace.is_some() {
            return Err(
                "CANVAS_INVALID: --replace does not combine with --floor or --workspace".into(),
            );
        }
        let team = core_lead::team(&doc, &me.id);
        let target = core_lead::find(&doc, old, Some(&team))?.clone();
        if target.id == me.id {
            return Err("CANVAS_INVALID: the Lead cannot replace itself".into());
        }
        let command = a.command.as_deref().filter(|c| !c.trim().is_empty());
        let mut patch = match (&user_preset, &agent, command) {
            (Some((_, p)), _, _) => {
                let mut p = p.clone();
                p["data"]["terminalId"] = json!(new_id());
                if let Some(c) = command {
                    p["data"]["command"] = json!(c);
                }
                p
            }
            (None, Some(ag), _) => {
                let mut p = core_lead::swap_agent_patch(ag, command)?;
                p["data"]["presetId"] = Value::Null;
                p
            }
            (None, None, Some(c)) => core_lead::swap_command_patch(c)?,
            (None, None, None) => json!({ "data": { "terminalId": new_id() } }),
        };
        if let Some(r) = &role_id {
            patch["data"]["roleId"] = json!(r);
        }
        if let Some(n) = a.name.as_deref().filter(|n| !n.trim().is_empty()) {
            let mut taken = core_lead::names_on(&doc);
            taken.remove(&core_lead::name_of(&target));
            patch["title"] = json!(core_lead::unique_name(&taken, n));
        }
        close_terminal(&target);
        s.apply(
            &ws,
            &floor,
            vec![Op::update_node(target.id.clone(), patch)],
            &origin,
            None,
        )?;
        super::launch::spawn_ensure(&ws, &floor, &target.id);
        return Ok(
            json!({ "nodeId": target.id, "replaced": old, "workspaceId": ws, "floorId": floor }),
        );
    }

    let dir = a
        .dir
        .clone()
        .or_else(|| me.data_str("cwd").map(str::to_string));
    let spec = RecruitSpec {
        name: a.name.clone(),
        agent,
        command: a.command.clone(),
        role_id,
        cwd: floors::remap_cwd(&s, &ws, &floor, dir.as_deref()),
        preset_id: None,
    };
    let dress = |node: &mut CanvasNode| {
        if let Some((_, p)) = &user_preset {
            let keep_cwd = node.data.get("cwd").cloned();
            merge_patch(node, p);
            node.kind = NodeKind::Agent;
            if p["data"].get("cwd").is_none() {
                if let Some(c) = keep_cwd {
                    node.data["cwd"] = c;
                }
            }
            if let Some(c) = a.command.as_deref().filter(|c| !c.trim().is_empty()) {
                node.data["command"] = json!(c);
            }
        }
    };

    if ws == ctx.workspace_id && floor == ctx.floor_id {
        let (mut ops, made) = core_lead::recruit_ops(&doc, &me, std::slice::from_ref(&spec));
        for op in ops.iter_mut() {
            if let Op::AddNode { node } = op {
                dress(node);
            }
        }
        s.apply(&ws, &floor, ops, &origin, None)?;
        let (id, name) = made
            .into_iter()
            .next()
            .ok_or("CANVAS_INVALID: nothing recruited")?;
        super::launch::spawn_ensure(&ws, &floor, &id);
        return Ok(json!({ "nodeId": id, "name": name, "workspaceId": ws, "floorId": floor }));
    }

    let there = s.read(&ws, &floor, |d| d.clone())?;
    let name = core_lead::unique_name(
        &core_lead::names_on(&there),
        a.name.as_deref().unwrap_or("Recruit"),
    );
    let at = layout::free_spot_in(&there, me.w, me.h, None, &[]);
    let mut node = core_lead::recruit_node(Some(&me), &spec, &name, at, there.max_z() + 1.0);
    dress(&mut node);
    let node_id = node.id.clone();
    let tid = node.data_str("terminalId").unwrap_or_default().to_string();
    let floor_name = |w: &str, f: &str| {
        s.workspace(w)
            .ok()
            .and_then(|m| m.floors.iter().find(|x| x.id == f).map(|x| x.name.clone()))
            .unwrap_or_default()
    };
    let chip = |title: String, at: Point, z: f64, remote: &CanvasNode, rws: &str, rfloor: &str| {
        CanvasNode {
            id: new_id(),
            kind: NodeKind::Connector,
            x: at.x,
            y: at.y,
            w: 160.0,
            h: 48.0,
            z,
            title: Some(title),
            subtitle: None,
            icon: None,
            color: None,
            group_id: None,
            locked: false,
            stack_id: None,
            data: json!({
                "remoteNodeId": remote.id,
                "remoteTerminalId": remote.data_str("terminalId"),
                "remoteFloorId": rfloor,
                "remoteWorkspaceId": rws,
            }),
        }
    };
    let back_at = layout::free_spot_in(
        &there,
        160.0,
        48.0,
        Some(Point {
            x: at.x,
            y: at.y - 80.0,
        }),
        &[],
    );
    let back = chip(
        format!(
            "{} @ {}",
            core_lead::name_of(&me),
            floor_name(&ctx.workspace_id, &ctx.floor_id)
        ),
        back_at,
        there.max_z() + 2.0,
        &me,
        &ctx.workspace_id,
        &ctx.floor_id,
    );
    let back_cord = core_lead::cord(&back.id, &node_id);
    s.apply(
        &ws,
        &floor,
        vec![
            Op::AddNode { node: node.clone() },
            Op::AddNode { node: back },
            Op::AddConnection {
                connection: back_cord,
            },
        ],
        &origin,
        None,
    )?;
    let spot = layout::place_recruits(&doc, &me.rect(), 1, (160.0, 48.0))
        .into_iter()
        .next()
        .unwrap_or(Point {
            x: me.x,
            y: me.y + me.h + 40.0,
        });
    let there_name = floor_name(&ws, &floor);
    let fwd = chip(
        format!("{name} @ {there_name}"),
        spot,
        doc.max_z() + 1.0,
        &node,
        &ws,
        &floor,
    );
    let fwd_cord = core_lead::cord(&me.id, &fwd.id);
    s.apply(
        &ctx.workspace_id,
        &ctx.floor_id,
        vec![
            Op::AddNode { node: fwd },
            Op::AddConnection {
                connection: fwd_cord,
            },
        ],
        &origin,
        None,
    )?;
    super::launch::spawn_ensure(&ws, &floor, &node_id);
    Ok(
        json!({ "nodeId": node_id, "terminalId": tid, "name": name, "workspaceId": ws, "floorId": floor }),
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Pair {
    from: String,
    to: String,
}

fn pair(ctx: &NodeCtx, a: Value) -> Result<(FloorDoc, CanvasNode, String, String), String> {
    let (doc, me) = require_lead(ctx)?;
    let p: Pair = args(a)?;
    let team = core_lead::team(&doc, &me.id);
    let id_of = |name: &str| -> Result<String, String> {
        if matches!(name.trim(), "me" | "self" | "You") || name.trim() == core_lead::name_of(&me) {
            return Ok(me.id.clone());
        }
        Ok(core_lead::find(&doc, name, Some(&team))?.id.clone())
    };
    let (from, to) = (id_of(&p.from)?, id_of(&p.to)?);
    if from == to {
        return Err("CANVAS_INVALID: cannot connect a node to itself".into());
    }
    Ok((doc, me, from, to))
}

fn connect(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let (doc, _me, from, to) = pair(ctx, a)?;
    let exists = doc
        .connections
        .iter()
        .any(|c| (c.from == from && c.to == to) || (c.from == to && c.to == from));
    if exists {
        return Ok(json!({ "connected": true, "existing": true }));
    }
    let kinds = |id: &str| doc.node(id).map(|n| n.kind.clone());
    let mut c = core_lead::cord(&from, &to);
    if matches!(kinds(&from), Some(NodeKind::Note)) || matches!(kinds(&to), Some(NodeKind::Note)) {
        c.kind = omniget_core::core::canvas::model::ConnectionKind::Note;
        c.perms = vec!["read".into(), "write".into()];
    } else if matches!(kinds(&from), Some(NodeKind::Portal))
        || matches!(kinds(&to), Some(NodeKind::Portal))
    {
        c.kind = omniget_core::core::canvas::model::ConnectionKind::Portal;
        c.perms = vec!["drive".into()];
    }
    let id = c.id.clone();
    store().apply(
        &ctx.workspace_id,
        &ctx.floor_id,
        vec![Op::AddConnection { connection: c }],
        &store::agent_origin(&ctx.node_id),
        None,
    )?;
    Ok(json!({ "connected": true, "connectionId": id }))
}

fn disconnect(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let (doc, _me, from, to) = pair(ctx, a)?;
    let ops: Vec<Op> = doc
        .connections
        .iter()
        .filter(|c| (c.from == from && c.to == to) || (c.from == to && c.to == from))
        .map(|c| Op::RemoveConnection { id: c.id.clone() })
        .collect();
    let n = ops.len();
    if n > 0 {
        store().apply(
            &ctx.workspace_id,
            &ctx.floor_id,
            ops,
            &store::agent_origin(&ctx.node_id),
            None,
        )?;
    }
    Ok(json!({ "removed": n }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SwapArgs {
    name: String,
    role: Option<String>,
    agent: Option<String>,
    command: Option<String>,
}

fn team_member(ctx: &NodeCtx, name: &str) -> Result<(FloorDoc, CanvasNode, CanvasNode), String> {
    let (doc, me) = require_lead(ctx)?;
    let team = core_lead::team(&doc, &me.id);
    let target = core_lead::find(&doc, name, Some(&team))?.clone();
    if !target.is_terminal() {
        return Err(format!("CANVAS_INVALID: '{name}' is not a terminal"));
    }
    Ok((doc, me, target))
}

fn swap_role(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let a: SwapArgs = args(a)?;
    let (_doc, _me, target) = team_member(ctx, &a.name)?;
    let s = store();
    let role = match a.role.as_deref().map(str::trim) {
        None | Some("") | Some("none") => None,
        Some(r) => Some(resolve_role(&s, &ctx.workspace_id, r)?),
    };
    close_terminal(&target);
    s.apply(
        &ctx.workspace_id,
        &ctx.floor_id,
        vec![Op::update_node(
            target.id.clone(),
            core_lead::swap_role_patch(role.as_deref()),
        )],
        &store::agent_origin(&ctx.node_id),
        None,
    )?;
    super::launch::spawn_ensure(&ctx.workspace_id, &ctx.floor_id, &target.id);
    Ok(json!({ "nodeId": target.id, "roleId": role }))
}

fn swap_agent(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let a: SwapArgs = args(a)?;
    let (_doc, _me, target) = team_member(ctx, &a.name)?;
    let agent = a
        .agent
        .as_deref()
        .ok_or("CANVAS_INVALID: agent is required")?;
    let patch = core_lead::swap_agent_patch(agent, a.command.as_deref())?;
    close_terminal(&target);
    store().apply(
        &ctx.workspace_id,
        &ctx.floor_id,
        vec![Op::update_node(target.id.clone(), patch)],
        &store::agent_origin(&ctx.node_id),
        None,
    )?;
    super::launch::spawn_ensure(&ctx.workspace_id, &ctx.floor_id, &target.id);
    Ok(json!({ "nodeId": target.id, "agent": agent }))
}

fn dismiss(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let a: SwapArgs = args(a)?;
    let (_doc, me, target) = team_member(ctx, &a.name)?;
    if target.id == me.id {
        return Err("CANVAS_INVALID: the Lead cannot dismiss itself".into());
    }
    close_terminal(&target);
    let s = store();
    s.apply(
        &ctx.workspace_id,
        &ctx.floor_id,
        vec![Op::RemoveNode {
            id: target.id.clone(),
        }],
        &store::agent_origin(&ctx.node_id),
        None,
    )?;
    let _ = omniget_core::core::canvas::routines::forget_node(&s, &target.id);
    Ok(json!({ "dismissed": core_lead::name_of(&target) }))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct WorkspaceArgs {
    name: String,
    dir: Option<String>,
    from: Option<String>,
    group: Option<String>,
    folder: Option<String>,
}

async fn workspace_create(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    require_lead(ctx)?;
    let a: WorkspaceArgs = args(a)?;
    let s = store();
    let mine = s.workspace(&ctx.workspace_id)?;
    let meta = s.create_workspace(WorkspaceInput {
        name: a.name.clone(),
        icon: None,
        color: None,
        working_directory: a.dir.clone().or(mine.working_directory.clone()),
        environment: mine.environment.clone(),
        id: None,
    })?;
    let rec = envs::load(&s, &ctx.workspace_id);
    if rec.kind != envs::EnvKind::Local {
        let _ = envs::save(
            &s,
            &meta.id,
            envs::EnvRecord {
                dir: a.dir.clone().or(rec.dir.clone()),
                ..rec
            },
        );
    }
    let mut patch = Map::new();
    if let Some(g) = a.group.filter(|g| !g.is_empty()) {
        patch.insert("groupName".into(), json!(g));
    }
    if let Some(f) = a.folder.filter(|f| !f.is_empty()) {
        patch.insert("folderName".into(), json!(f));
    }
    if !patch.is_empty() {
        s.update_workspace(&meta.id, patch)?;
    }
    let mut copied = 0;
    if let Some(src) = &a.from {
        let src = resolve_workspace(&s, src)?;
        let (node_ids, drawing_ids) = s.read(&src, GROUND_FLOOR_ID, |d| {
            (
                d.nodes
                    .iter()
                    .filter(|n| n.stack_id.is_none())
                    .map(|n| n.id.clone())
                    .collect::<Vec<_>>(),
                d.drawings.iter().map(|x| x.id.clone()).collect::<Vec<_>>(),
            )
        })?;
        if !node_ids.is_empty() || !drawing_ids.is_empty() {
            let p = export::export_from_store(
                &s,
                &src,
                GROUND_FLOOR_ID,
                &Selection {
                    node_ids,
                    drawing_ids,
                },
                &ExportOptions {
                    name: a.name.clone(),
                    ..Default::default()
                },
            )?;
            let role_map = partituras::resolve_roles(&s, &meta.id, &p.roles, &HashMap::new())?;
            let r = export::import_partitura(
                &s,
                &meta.id,
                GROUND_FLOOR_ID,
                &p,
                Point { x: 0.0, y: 0.0 },
                &store::agent_origin(&ctx.node_id),
                &role_map,
            )?;
            copied = r.node_ids.len();
        }
    }
    Ok(json!({ "workspaceId": meta.id, "name": meta.name, "copiedNodes": copied }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NotifyArgs {
    message: String,
}

fn notify(ctx: &NodeCtx, a: Value) -> Result<Value, String> {
    let (_doc, me) = require_lead(ctx)?;
    let a: NotifyArgs = args(a)?;
    let msg = a.message.trim();
    if msg.is_empty() {
        return Err("CANVAS_INVALID: message is empty".into());
    }
    if msg.chars().count() > NOTIFY_MAX {
        return Err(format!(
            "CANVAS_INVALID: message is longer than {NOTIFY_MAX} characters"
        ));
    }
    super::routines::notify(
        &ctx.workspace_id,
        Some(&ctx.floor_id),
        Some(&me.id),
        Some(&core_lead::name_of(&me)),
        msg,
        "lead",
        true,
    );
    Ok(json!({ "sent": true }))
}
