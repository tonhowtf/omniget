use std::collections::{BTreeMap, HashMap};

use omniget_core::core::canvas::envs::{self, Wrapped};
use omniget_core::core::canvas::floors::{
    self, CreateFloor, CreateResult, DiscardOptions, FloorProgress, HookKind, LandOptions,
};
use omniget_core::core::canvas::model::GROUND_FLOOR_ID;
use omniget_core::core::canvas::store::CanvasStore;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Emitter;

use super::rpc::{NodeCtx, RpcResult};

pub const EVENT_FLOOR_PROGRESS: &str = "canvas://floor-progress";
pub const EVENT_FLOORS: &str = "canvas://floors";

fn store() -> std::sync::Arc<CanvasStore> {
    super::doc::host().store().clone()
}

pub fn emit_floors(ws: &str, floor: Option<&str>, reason: &str) {
    if let Some(app) = super::routines::app() {
        let _ = app.emit(
            EVENT_FLOORS,
            json!({ "workspaceId": ws, "floorId": floor, "reason": reason }),
        );
    }
}

pub fn progress_sink() -> impl Fn(FloorProgress) + Send + Sync {
    |p: FloorProgress| {
        if let Some(app) = super::routines::app() {
            let _ = app.emit(EVENT_FLOOR_PROGRESS, &p);
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFloor {
    pub token: String,
    pub workspace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_id: Option<String>,
    pub name: String,
    pub phase: String,
    pub completed_items: u32,
    pub total_items: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: String,
}

const PHASES: [&str; 5] = ["clone", "branch", "layout", "setup", "done"];

fn pending() -> &'static std::sync::Mutex<Vec<PendingFloor>> {
    static P: std::sync::OnceLock<std::sync::Mutex<Vec<PendingFloor>>> = std::sync::OnceLock::new();
    P.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn with_pending<R>(f: impl FnOnce(&mut Vec<PendingFloor>) -> R) -> R {
    let mut g = pending().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

pub fn pending_floors(ws: &str) -> Vec<PendingFloor> {
    with_pending(|p| p.iter().filter(|f| f.workspace_id == ws).cloned().collect())
}

pub fn dismiss_pending(ws: &str, key: &str) -> bool {
    let key = key.trim();
    let hit = with_pending(|p| {
        let n = p.len();
        p.retain(|f| {
            !(f.workspace_id == ws
                && (f.token == key
                    || f.floor_id.as_deref() == Some(key)
                    || f.name.eq_ignore_ascii_case(key)))
        });
        p.len() != n
    });
    if hit {
        emit_floors(ws, None, "pending");
    }
    hit
}

fn start_copied_terminals(ws: &str, floor: &str) {
    let ids: Vec<String> = store()
        .read(ws, floor, |d| {
            d.nodes
                .iter()
                .filter(|n| {
                    matches!(
                        n.kind,
                        omniget_core::core::canvas::model::NodeKind::Terminal
                            | omniget_core::core::canvas::model::NodeKind::Agent
                    )
                })
                .map(|n| n.id.clone())
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        super::launch::spawn_ensure(ws, floor, &id);
    }
}

pub async fn create(ws: &str, input: CreateFloor) -> Result<CreateResult, String> {
    let copy_ground = input.copy_ground;
    let token = omniget_core::core::canvas::model::new_id();
    with_pending(|p| {
        p.push(PendingFloor {
            token: token.clone(),
            workspace_id: ws.to_string(),
            floor_id: None,
            name: input.name.trim().to_string(),
            phase: "start".into(),
            completed_items: 0,
            total_items: PHASES.len() as u32,
            error: None,
            started_at: chrono::Utc::now().to_rfc3339(),
        })
    });
    let s = store();
    let base = progress_sink();
    let tok = token.clone();
    let sink = move |pr: FloorProgress| {
        with_pending(|p| {
            if let Some(f) = p.iter_mut().find(|f| f.token == tok) {
                f.floor_id = Some(pr.floor_id.clone());
                f.phase = pr.phase.clone();
                if let Some(i) = PHASES.iter().position(|x| *x == pr.phase) {
                    f.completed_items = f.completed_items.max(i as u32 + 1);
                }
                if pr.phase == "failed" {
                    f.error = pr.message.clone().or(Some("failed".into()));
                }
            }
        });
        base(pr);
    };
    match floors::create(&s, ws, input, &sink).await {
        Ok(r) => {
            with_pending(|p| p.retain(|f| f.token != token));
            emit_floors(ws, Some(&r.floor.meta.id), "created");
            if copy_ground {
                start_copied_terminals(ws, &r.floor.meta.id);
            }
            Ok(r)
        }
        Err(e) => {
            with_pending(|p| {
                if let Some(f) = p.iter_mut().find(|f| f.token == token) {
                    f.phase = "failed".into();
                    f.error = Some(e.clone());
                }
            });
            emit_floors(ws, None, "pending");
            Err(e)
        }
    }
}

pub fn create_background(ws: &str, input: CreateFloor) -> PendingFloor {
    let name = input.name.trim().to_string();
    let ws_owned = ws.to_string();
    tauri::async_runtime::spawn(async move {
        let _ = create(&ws_owned, input).await;
    });
    PendingFloor {
        token: String::new(),
        workspace_id: ws.to_string(),
        floor_id: None,
        name,
        phase: "start".into(),
        completed_items: 0,
        total_items: PHASES.len() as u32,
        error: None,
        started_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorActivity {
    pub floor_id: String,
    pub attention: u32,
    pub working: u32,
    pub alive: u32,
    pub terminals: u32,
}

pub fn activity(ws: &str) -> Vec<FloorActivity> {
    use omniget_core::core::term::agent::AgentState;
    let s = store();
    let Ok(meta) = s.workspace(ws) else {
        return Vec::new();
    };
    let term = crate::pty::term();
    meta.floors
        .iter()
        .map(|f| {
            let tids: Vec<String> = s
                .read(ws, &f.id, |d| {
                    d.nodes
                        .iter()
                        .filter(|n| n.is_terminal())
                        .filter_map(|n| n.data_str("terminalId").map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let mut a = FloorActivity {
                floor_id: f.id.clone(),
                terminals: tids.len() as u32,
                ..Default::default()
            };
            for t in &tids {
                if term.info(t).is_some_and(|i| i.alive) {
                    a.alive += 1;
                }
                match term.agent_state(t).map(|x| x.state) {
                    Some(AgentState::Blocked) if !super::cords::host().terminal_seen(t) => {
                        a.attention += 1
                    }
                    Some(
                        AgentState::Thinking
                        | AgentState::Working
                        | AgentState::Compacting
                        | AgentState::Looping,
                    ) => a.working += 1,
                    _ => {}
                }
            }
            a
        })
        .collect()
}

pub fn spawn_context(
    ws: &str,
    floor: &str,
    cwd: Option<&str>,
    argv: &[String],
    env: &HashMap<String, String>,
) -> Wrapped {
    let s = store();
    let mut merged: BTreeMap<String, String> = if floor == GROUND_FLOOR_ID {
        BTreeMap::new()
    } else {
        floors::floor_env(&s, ws, floor)
    };
    for (k, v) in env {
        merged.insert(k.clone(), v.clone());
    }
    let cwd = floors::remap_cwd(&s, ws, floor, cwd);
    let rec = envs::load_for(
        &s,
        ws,
        merged.get("OMNIGET_CANVAS_NODE").map(String::as_str),
    );
    envs::wrap(&rec, argv, cwd.as_deref(), &merged)
}

pub fn resolve_floor(s: &CanvasStore, ws: &str, key: &str) -> Result<String, String> {
    let meta = s.workspace(ws)?;
    let key = key.trim();
    meta.floors
        .iter()
        .find(|f| f.id == key)
        .or_else(|| {
            meta.floors
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(key))
        })
        .map(|f| f.id.clone())
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no floor named '{key}'"))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct FloorArgs {
    name: Option<String>,
    floor: Option<String>,
    branch: Option<String>,
    existing_branch: bool,
    no_git: bool,
    copy_ground: bool,
    into: Option<String>,
    delete_branch: bool,
    keep_floor: bool,
    keep_branch: bool,
    force: bool,
    kind: Option<String>,
    message: Option<String>,
}

fn parse<T: for<'de> Deserialize<'de> + Default>(args: Value) -> Result<T, String> {
    if args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(args).map_err(|e| format!("CANVAS_INVALID: {e}"))
}

pub async fn rpc_create(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    super::maestro::require_maestro(ctx)?;
    let a: FloorArgs = parse(args)?;
    let name = a
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .ok_or("CANVAS_INVALID: floor name is required")?;
    let r = create(
        &ctx.workspace_id,
        CreateFloor {
            name,
            branch: a.branch,
            existing_branch: a.existing_branch,
            no_git: a.no_git,
            copy_ground: a.copy_ground,
            ..Default::default()
        },
    )
    .await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    let r = match method {
        "floor.list" => floor_list(ctx).await,
        "floor.create" => rpc_create(ctx, args).await,
        "floor.land" => floor_land(ctx, args).await,
        "floor.delete" => floor_delete(ctx, args).await,
        "floor.run" => floor_run(ctx, args).await,
        _ => return None,
    };
    Some(r)
}

async fn floor_list(ctx: &NodeCtx) -> Result<Value, String> {
    let s = store();
    let list = floors::list(&s, &ctx.workspace_id).await?;
    Ok(json!({
        "you": ctx.floor_id,
        "floors": list.iter().map(|f| json!({
            "id": f.meta.id,
            "name": f.meta.name,
            "branch": f.meta.branch,
            "path": f.meta.path,
            "mode": f.mode,
            "port": f.port,
            "nodes": f.node_count,
            "terminals": f.terminal_count,
            "ahead": f.ahead,
            "dirty": f.dirty,
        })).collect::<Vec<_>>(),
    }))
}

fn from_ground(ctx: &NodeCtx, what: &str) -> Result<(), String> {
    if ctx.floor_id != GROUND_FLOOR_ID {
        return Err(format!(
            "CANVAS_FORBIDDEN: `floor {what}` only runs from a terminal on the ground floor"
        ));
    }
    Ok(())
}

async fn floor_land(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    super::maestro::require_maestro(ctx)?;
    from_ground(ctx, "land")?;
    let a: FloorArgs = parse(args)?;
    let s = store();
    let key = a
        .name
        .or(a.floor)
        .ok_or("CANVAS_INVALID: floor name is required")?;
    let floor = resolve_floor(&s, &ctx.workspace_id, &key)?;
    let out = land(
        &ctx.workspace_id,
        &floor,
        LandOptions {
            into: a.into,
            delete_branch: a.delete_branch,
            keep_floor: a.keep_floor,
            message: a.message,
        },
    )
    .await?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

async fn floor_delete(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    super::maestro::require_maestro(ctx)?;
    from_ground(ctx, "delete")?;
    let a: FloorArgs = parse(args)?;
    let s = store();
    let key = a
        .name
        .or(a.floor)
        .ok_or("CANVAS_INVALID: floor name is required")?;
    let floor = resolve_floor(&s, &ctx.workspace_id, &key)?;
    let out = discard(
        &ctx.workspace_id,
        &floor,
        DiscardOptions {
            keep_branch: a.keep_branch,
            force: a.force,
        },
    )
    .await?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

async fn floor_run(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    super::maestro::require_maestro(ctx)?;
    let a: FloorArgs = parse(args)?;
    let s = store();
    let floor = match a.name.or(a.floor) {
        Some(k) => resolve_floor(&s, &ctx.workspace_id, &k)?,
        None => ctx.floor_id.clone(),
    };
    let kind = HookKind::parse(a.kind.as_deref().unwrap_or("run"))?;
    let runs = floors::run_hooks(&s, &ctx.workspace_id, &floor, kind, None).await?;
    serde_json::to_value(runs).map_err(|e| e.to_string())
}

pub async fn land(ws: &str, floor: &str, opts: LandOptions) -> Result<floors::LandOutcome, String> {
    let s = store();
    if !opts.keep_floor {
        let p = floors::land_preview(&s, ws, floor, opts.into.as_deref()).await?;
        if let Some(b) = p.blocker {
            return Err(format!("CANVAS_LAND_BLOCKED: {b}"));
        }
        close_floor_terminals(&s, ws, floor);
    }
    let out = floors::land(&s, ws, floor, opts).await?;
    emit_floors(ws, Some(floor), "landed");
    Ok(out)
}

pub async fn discard(
    ws: &str,
    floor: &str,
    opts: DiscardOptions,
) -> Result<floors::DiscardOutcome, String> {
    let s = store();
    close_floor_terminals(&s, ws, floor);
    let out = floors::discard(&s, ws, floor, opts).await?;
    emit_floors(ws, Some(floor), "deleted");
    Ok(out)
}

pub fn close_floor_terminals(s: &CanvasStore, ws: &str, floor: &str) {
    let ids: Vec<String> = s
        .read(ws, floor, |d| {
            d.nodes
                .iter()
                .filter(|n| n.is_terminal())
                .filter_map(|n| n.data_str("terminalId").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let term = crate::pty::term();
    for id in ids {
        let _ = term.close(&id, true);
    }
}
