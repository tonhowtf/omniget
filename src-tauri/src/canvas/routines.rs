use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use omniget_core::core::canvas::floors;
use omniget_core::core::canvas::maestro as core_maestro;
use omniget_core::core::canvas::routines::{self, End, Routine, RoutineInput, RunRecord, Schedule};
use omniget_core::core::canvas::store::CanvasStore;
use omniget_core::core::term::agent::AgentState;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::rpc::{NodeCtx, RpcResult};

pub const EVENT_ROUTINE: &str = "canvas://routine";
pub const EVENT_NOTIFY: &str = "canvas://notify";

const TICK: Duration = Duration::from_secs(5);
const LATE_AFTER_SECS: i64 = 30;
const PRE_RUN_TIMEOUT: Duration = Duration::from_secs(120);
const STEP_MAX: Duration = Duration::from_secs(30 * 60);

static APP: OnceLock<AppHandle> = OnceLock::new();
static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static LATE: Mutex<Option<std::collections::HashMap<String, Option<u64>>>> = Mutex::new(None);

pub fn app() -> Option<&'static AppHandle> {
    APP.get()
}

fn store() -> std::sync::Arc<CanvasStore> {
    super::doc::host().store().clone()
}

pub fn start(app: &AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    super::doc::host().bind(app);
    tauri::async_runtime::spawn(async move {
        loop {
            tick().await;
            tokio::time::sleep(TICK).await;
        }
    });
}

fn claim(id: &str) -> bool {
    let mut g = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashSet::new).insert(id.to_string())
}

fn release(id: &str) {
    if let Some(set) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        set.remove(id);
    }
}

async fn tick() {
    let s = store();
    let due = tauri::async_runtime::spawn_blocking(move || routines::due(&s, chrono::Local::now()))
        .await
        .unwrap_or_default();
    let now = chrono::Local::now();
    for r in due {
        if claim(&r.id) {
            let late = routines::next_fire(&r)
                .map(|n| (now - n).num_seconds())
                .filter(|s| *s > LATE_AFTER_SECS)
                .map(|s| s as u64);
            tauri::async_runtime::spawn(async move {
                let id = r.id.clone();
                LATE.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_or_insert_with(std::collections::HashMap::new)
                    .insert(id.clone(), late);
                let _ = run(r, false).await;
                release(&id);
            });
        }
    }
}

pub fn emit_routine(r: &Routine) {
    if let Some(app) = app() {
        let _ = app.emit(EVENT_ROUTINE, r);
    }
}

pub fn notify(
    ws: &str,
    floor: Option<&str>,
    node: Option<&str>,
    name: Option<&str>,
    message: &str,
    source: &str,
    system: bool,
) {
    crate::mcp::canvas_ui::record_notification(source, name.unwrap_or("OmniGet"), message, system);
    let Some(app) = app() else { return };
    let _ = app.emit(
        EVENT_NOTIFY,
        json!({ "workspaceId": ws, "floorId": floor, "nodeId": node, "name": name, "message": message, "source": source }),
    );
    if system {
        use tauri_plugin_notification::NotificationExt;
        let _ = app
            .notification()
            .builder()
            .title(name.unwrap_or("OmniGet"))
            .body(message)
            .show();
    }
}

pub fn is_busy(tid: &str) -> bool {
    busy(tid)
}

fn busy(tid: &str) -> bool {
    let term = crate::pty::term();
    if term.user_typing(tid) {
        return true;
    }
    if term.is_busy(tid) == Some(true) {
        return true;
    }
    match term.agent_state(tid).map(|s| s.state) {
        Some(
            AgentState::Thinking
            | AgentState::Working
            | AgentState::WaitingPeer
            | AgentState::Blocked
            | AgentState::Compacting
            | AgentState::Looping,
        ) => true,
        Some(AgentState::Unknown) | None => term
            .screen_text(tid, 1)
            .map(|sn| chrono::Utc::now().timestamp_millis() - sn.changed_ms < 2000)
            .unwrap_or(false),
        _ => false,
    }
}

async fn wait_done(tid: &str) {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(10) && !busy(tid) {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let mut quiet = 0;
    while t0.elapsed() < STEP_MAX && quiet < 3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if busy(tid) {
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn record(
    r: &Routine,
    status: &str,
    message: Option<String>,
    steps: u32,
    manual: bool,
) -> Result<Routine, String> {
    record_code(r, status, None, message, steps, manual)
}

fn record_code(
    r: &Routine,
    status: &str,
    code: Option<&str>,
    message: Option<String>,
    steps: u32,
    manual: bool,
) -> Result<Routine, String> {
    let s = store();
    let code = code.map(str::to_string).or_else(|| {
        Some(
            match status {
                "fired" => "sentToTerminal",
                "reminded" => "reminder",
                "failed" => "failed",
                _ => return None,
            }
            .to_string(),
        )
    });
    let late_secs = if manual {
        None
    } else {
        LATE.lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .and_then(|m| m.remove(&r.id))
            .flatten()
    };
    let out = routines::record(
        &s,
        &r.id,
        RunRecord {
            at: now(),
            status: status.into(),
            code,
            late_secs,
            message,
            steps,
            manual,
        },
    )?;
    emit_routine(&out);
    Ok(out)
}

pub async fn run(r: Routine, manual: bool) -> Result<Routine, String> {
    let s = store();
    let Some(node_id) = r.node_id.clone() else {
        notify(
            &r.workspace_id,
            Some(&r.floor_id),
            None,
            Some(&r.name),
            r.prompt.trim(),
            "routine",
            true,
        );
        return record(&r, "reminded", None, 0, manual);
    };
    let node = s
        .read(&r.workspace_id, &r.floor_id, |d| d.node(&node_id).cloned())
        .ok()
        .flatten();
    let Some(node) = node else {
        return record(
            &r,
            "failed",
            Some("the target node is gone".into()),
            0,
            manual,
        );
    };
    let Some(tid) = node.data_str("terminalId").map(str::to_string) else {
        return record(
            &r,
            "failed",
            Some("the target is not a terminal".into()),
            0,
            manual,
        );
    };
    let term = crate::pty::term();
    let was_down = !term.info(&tid).is_some_and(|i| i.alive);
    if was_down && term.info(&tid).is_some() {
        let _ = term.close(&tid, false);
    }
    if term.info(&tid).is_none() {
        if let Err(e) = super::launch::ensure_running(&r.workspace_id, &r.floor_id, &node_id).await
        {
            return record(
                &r,
                "failed",
                Some(format!("could not start the terminal: {e}")),
                0,
                manual,
            );
        }
    }
    if !term.info(&tid).is_some_and(|i| i.alive) {
        return record(
            &r,
            "failed",
            Some("the terminal is not running (restart it on the canvas)".into()),
            0,
            manual,
        );
    }
    if was_down {
        wait_ready(&tid).await;
    }
    if r.skip_if_busy && !manual && !was_down && busy(&tid) {
        return record_code(
            &r,
            "skipped",
            Some("skippedWhileBusy"),
            Some("the terminal was busy".into()),
            0,
            manual,
        );
    }
    let mut output = None;
    if let Some(script) = r.pre_run.as_deref().filter(|s| !s.trim().is_empty()) {
        let dir = floors::remap_cwd(&s, &r.workspace_id, &r.floor_id, node.data_str("cwd"))
            .map(std::path::PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        let env = floors::floor_env(&s, &r.workspace_id, &r.floor_id);
        match floors::run_shell(&dir, script, &env, PRE_RUN_TIMEOUT).await {
            Ok((Some(0), text)) => output = Some(text),
            Ok((code, text)) => {
                let msg = format!(
                    "pre-run exited {}: {}",
                    code.map(|c| c.to_string()).unwrap_or("?".into()),
                    text.trim().chars().take(300).collect::<String>()
                );
                return record_code(&r, "skipped", Some("skippedByScript"), Some(msg), 0, manual);
            }
            Err(e) => return record(&r, "failed", Some(format!("pre-run: {e}")), 0, manual),
        }
    }
    let steps = routines::split_steps(&r.prompt);
    let n = steps.len();
    for (i, step) in steps.iter().enumerate() {
        let text = routines::fill_output(step, output.as_deref());
        if let Err(e) = term.send_prompt(&tid, &text) {
            return record(&r, "failed", Some(e), i as u32, manual);
        }
        if i + 1 < n {
            wait_done(&tid).await;
        }
    }
    if r.notify {
        let name = core_maestro::name_of(&node);
        notify(
            &r.workspace_id,
            Some(&r.floor_id),
            Some(&node_id),
            Some(&r.name),
            &format!("Sent to {name}"),
            "routine",
            true,
        );
    }
    if was_down {
        return record_code(
            &r,
            "fired",
            Some("terminalNotRunning"),
            Some("the terminal was not running: started it and sent the prompt".into()),
            n as u32,
            manual,
        );
    }
    record(&r, "fired", None, n as u32, manual)
}

async fn wait_ready(tid: &str) {
    let t0 = Instant::now();
    let mut quiet = 0;
    while t0.elapsed() < Duration::from_secs(20) && quiet < 2 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if busy(tid) {
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
}

pub async fn run_now(id: &str) -> Result<Routine, String> {
    let s = store();
    let r = routines::get(&s, id)?;
    if !claim(id) {
        return Err("CANVAS_BUSY: this routine is running".into());
    }
    let out = run(r, true).await;
    release(id);
    out
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RoutineArgs {
    id: Option<String>,
    name: Option<String>,
    target: Option<String>,
    prompt: Option<String>,
    schedule: Option<Schedule>,
    every: Option<String>,
    daily: Option<String>,
    at: Option<String>,
    end: Option<End>,
    pre_run: Option<String>,
    skip_if_busy: Option<bool>,
    notify: Option<bool>,
    enabled: Option<bool>,
}

fn parse_every(s: &str) -> Result<u32, String> {
    let s = s.trim().to_ascii_lowercase();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u32 = num
        .parse()
        .map_err(|_| format!("CANVAS_INVALID: interval `{s}` (e.g. 30s, 30m, 2h, 1d)"))?;
    let secs = match unit.trim() {
        "s" | "sec" | "secs" | "seconds" => n,
        "" | "m" | "min" | "mins" | "minutes" => n * 60,
        "h" | "hour" | "hours" => n * 3600,
        "d" | "day" | "days" => n * 86_400,
        u => return Err(format!("CANVAS_INVALID: interval unit `{u}`")),
    };
    if secs < routines::MIN_INTERVAL_SECS {
        return Err(format!(
            "CANVAS_INVALID: interval `{s}` is below the minimum of {} seconds",
            routines::MIN_INTERVAL_SECS
        ));
    }
    Ok(secs)
}

fn schedule_of(a: &RoutineArgs) -> Result<Option<Schedule>, String> {
    if let Some(s) = &a.schedule {
        return Ok(Some(s.clone()));
    }
    if let Some(e) = &a.every {
        let secs = parse_every(e)?;
        return Ok(Some(Schedule::Interval {
            minutes: secs / 60,
            seconds: Some(secs),
            start_at: None,
        }));
    }
    if let Some(t) = &a.daily {
        return Ok(Some(Schedule::Daily {
            time: t.clone(),
            hour: None,
            minute: None,
        }));
    }
    if let Some(at) = &a.at {
        return Ok(Some(Schedule::Once { at: at.clone() }));
    }
    Ok(None)
}

fn resolve_routine(s: &CanvasStore, ws: &str, key: &str) -> Result<Routine, String> {
    let all = routines::list(s, Some(ws));
    all.iter()
        .find(|r| r.id == key)
        .or_else(|| all.iter().find(|r| r.name.eq_ignore_ascii_case(key.trim())))
        .cloned()
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no routine named '{key}'"))
}

pub async fn rpc_create(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    let (doc, me) = super::maestro::require_maestro(ctx)?;
    let a: RoutineArgs = if args.is_null() {
        RoutineArgs::default()
    } else {
        serde_json::from_value(args).map_err(|e| format!("CANVAS_INVALID: {e}"))?
    };
    let s = store();
    let node_id = match a.target.as_deref().map(str::trim) {
        None | Some("") => None,
        Some("me") | Some("self") => Some(me.id.clone()),
        Some(t) => {
            let team = core_maestro::team(&doc, &me.id);
            Some(core_maestro::find(&doc, t, Some(&team))?.id.clone())
        }
    };
    let schedule = schedule_of(&a)?
        .ok_or("CANVAS_INVALID: a schedule is required (every, daily, at or schedule)")?;
    let r = routines::save(
        &s,
        RoutineInput {
            id: None,
            name: a.name.clone().unwrap_or_else(|| "Routine".into()),
            workspace_id: ctx.workspace_id.clone(),
            floor_id: Some(ctx.floor_id.clone()),
            node_id,
            prompt: a
                .prompt
                .clone()
                .ok_or("CANVAS_INVALID: prompt is required")?,
            schedule,
            end: a.end.clone().unwrap_or_default(),
            pre_run: a.pre_run.clone(),
            skip_if_busy: a.skip_if_busy,
            notify: a.notify.unwrap_or(false),
            enabled: a.enabled,
            created_by: Some(me.id.clone()),
        },
    )?;
    emit_routine(&r);
    serde_json::to_value(r).map_err(|e| e.to_string())
}

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    let r = match method {
        "routine.list" => Ok(json!(routines::list(&store(), Some(&ctx.workspace_id)))),
        "routine.show" => routine_key(&args)
            .and_then(|k| resolve_routine(&store(), &ctx.workspace_id, &k))
            .map(|r| json!(r)),
        "routine.create" => rpc_create(ctx, args).await,
        "routine.edit" => routine_edit(ctx, args).await,
        "routine.enable" | "routine.disable" => {
            routine_toggle(ctx, args, method == "routine.enable")
        }
        "routine.run" => match super::maestro::require_maestro(ctx) {
            Ok(_) => match routine_key(&args)
                .and_then(|k| resolve_routine(&store(), &ctx.workspace_id, &k))
            {
                Ok(r) => run_now(&r.id).await.map(|r| json!(r)),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        },
        "routine.delete" => routine_delete(ctx, args),
        "routine.history" => routine_history(ctx, args),
        _ => return None,
    };
    Some(r)
}

fn routine_history(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    let s = store();
    let id = match routine_key(&args) {
        Ok(k) => Some(
            routines::list_all(&s, Some(&ctx.workspace_id))
                .into_iter()
                .find(|r| r.id == k || r.name.eq_ignore_ascii_case(k.trim()))
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: no routine named '{k}'"))?
                .id,
        ),
        Err(_) => None,
    };
    if args.get("clear").and_then(Value::as_bool).unwrap_or(false) {
        super::maestro::require_maestro(ctx)?;
        let n = routines::clear_history(&s, Some(&ctx.workspace_id), id.as_deref())?;
        return Ok(json!({ "cleared": n }));
    }
    Ok(json!(routines::history(
        &s,
        Some(&ctx.workspace_id),
        id.as_deref()
    )))
}

fn routine_key(args: &Value) -> Result<String, String> {
    args.get("id")
        .or_else(|| args.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "CANVAS_INVALID: routine name or id is required".into())
}

fn routine_toggle(ctx: &NodeCtx, args: Value, on: bool) -> Result<Value, String> {
    super::maestro::require_maestro(ctx)?;
    let s = store();
    let r = resolve_routine(&s, &ctx.workspace_id, &routine_key(&args)?)?;
    let r = routines::set_enabled(&s, &r.id, on)?;
    emit_routine(&r);
    Ok(json!(r))
}

fn routine_delete(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    super::maestro::require_maestro(ctx)?;
    let s = store();
    let r = resolve_routine(&s, &ctx.workspace_id, &routine_key(&args)?)?;
    routines::delete(&s, &r.id)?;
    if let Some(app) = app() {
        let _ = app.emit(EVENT_ROUTINE, json!({ "id": r.id, "deleted": true }));
    }
    Ok(json!({ "deleted": r.name }))
}

async fn routine_edit(ctx: &NodeCtx, args: Value) -> Result<Value, String> {
    let (doc, me) = super::maestro::require_maestro(ctx)?;
    let s = store();
    let a: RoutineArgs =
        serde_json::from_value(args).map_err(|e| format!("CANVAS_INVALID: {e}"))?;
    let key =
        a.id.clone()
            .or(a.name.clone())
            .ok_or("CANVAS_INVALID: routine name or id is required")?;
    let cur = resolve_routine(&s, &ctx.workspace_id, &key)?;
    let node_id = match a.target.as_deref().map(str::trim) {
        None => cur.node_id.clone(),
        Some("") => None,
        Some(t) => {
            let team = core_maestro::team(&doc, &me.id);
            Some(core_maestro::find(&doc, t, Some(&team))?.id.clone())
        }
    };
    let r = routines::save(
        &s,
        RoutineInput {
            id: Some(cur.id.clone()),
            name: cur.name.clone(),
            workspace_id: cur.workspace_id.clone(),
            floor_id: Some(cur.floor_id.clone()),
            node_id,
            prompt: a.prompt.clone().unwrap_or(cur.prompt.clone()),
            schedule: schedule_of(&a)?.unwrap_or(cur.schedule.clone()),
            end: a.end.clone().unwrap_or(cur.end.clone()),
            pre_run: a.pre_run.clone().or(cur.pre_run.clone()),
            skip_if_busy: Some(a.skip_if_busy.unwrap_or(cur.skip_if_busy)),
            notify: a.notify.unwrap_or(cur.notify),
            enabled: Some(a.enabled.unwrap_or(cur.enabled)),
            created_by: cur.created_by.clone(),
        },
    )?;
    emit_routine(&r);
    Ok(json!(r))
}
