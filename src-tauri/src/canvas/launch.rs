use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use omniget_core::core::canvas::model::{AgentKind as CanvasAgent, CanvasNode, NodeKind};
use omniget_core::core::canvas::roles;
use omniget_core::core::term::agent::AgentKind as TermAgent;
use omniget_core::core::term::session::{TermInfo, TermOpen};
use tauri::AppHandle;

pub const DEFAULT_OLLAMA_MODEL: &str = "llama3.2";

const TERM_HEADER_H: f64 = 30.0;
const TERM_PAD: f64 = 6.0;
const CELL_W: f64 = 7.8;
const CELL_H: f64 = 13.0 * 1.25;

#[derive(Default)]
pub struct LaunchOpts {
    pub node: Option<CanvasNode>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub restore: bool,
}

pub struct Launched {
    pub info: TermInfo,
    pub fresh: bool,
}

fn locks() -> &'static Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    static L: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_for(tid: &str) -> Arc<tokio::sync::Mutex<()>> {
    locks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(tid.to_string())
        .or_default()
        .clone()
}

fn app() -> Result<&'static AppHandle, String> {
    super::routines::app()
        .or_else(super::chat::app)
        .ok_or_else(|| "CANVAS_NOT_READY: the app is still starting".to_string())
}

pub fn is_terminal(n: &CanvasNode) -> bool {
    matches!(n.kind, NodeKind::Terminal | NodeKind::Agent)
}

pub fn terminal_of(n: &CanvasNode) -> Option<String> {
    n.data_str("terminalId")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn agent_of(n: &CanvasNode) -> String {
    match n.data_str("agent") {
        Some(a @ ("shell" | "claude" | "codex" | "gemini" | "opencode" | "ollama" | "native")) => {
            a.to_string()
        }
        _ => "shell".to_string(),
    }
}

fn preset(agent: &str) -> Vec<String> {
    match agent {
        "claude" | "codex" | "gemini" | "opencode" => vec![agent.to_string()],
        "ollama" => vec!["ollama".into(), "run".into(), DEFAULT_OLLAMA_MODEL.into()],
        _ => Vec::new(),
    }
}

pub fn agent_command(n: &CanvasNode) -> Vec<String> {
    match n.data.get("command") {
        Some(serde_json::Value::Array(a)) if !a.is_empty() => {
            return a
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
        }
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
            return omniget_core::core::canvas::maestro::split_command(s);
        }
        _ => {}
    }
    let agent = agent_of(n);
    if agent == "ollama" {
        let model = n
            .data_str("model")
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(DEFAULT_OLLAMA_MODEL);
        return vec!["ollama".into(), "run".into(), model.into()];
    }
    preset(&agent)
}

pub fn grid_for(w: f64, h: f64) -> (u16, u16) {
    let bw = (w - TERM_PAD * 2.0).max(0.0);
    let bh = (h - TERM_HEADER_H - TERM_PAD).max(0.0);
    let cols = ((bw / CELL_W).floor() as i64).clamp(2, 500) as u16;
    let rows = ((bh / CELL_H).floor() as i64).clamp(1, 300) as u16;
    (cols, rows)
}

fn base_env(n: &CanvasNode, ws: &str, tid: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("OMNIGET_CANVAS".into(), "1".into());
    env.insert("OMNIGET_CANVAS_NODE".into(), n.id.clone());
    env.insert(
        "OMNIGET_CANVAS_NAME".into(),
        n.title.clone().unwrap_or_default(),
    );
    env.insert("OMNIGET_TERMINAL_ID".into(), tid.to_string());
    if !ws.is_empty() {
        env.insert("OMNIGET_CANVAS_WORKSPACE".into(), ws.to_string());
    }
    if let Some(extra) = n.data.get("env").and_then(|v| v.as_object()) {
        for (k, v) in extra {
            if let Some(s) = v.as_str() {
                env.insert(k.clone(), s.to_string());
            }
        }
    }
    env
}

fn shell_integration(
    tid: &str,
    env: &HashMap<String, String>,
) -> Option<omniget_core::core::term::agent::launch::Launch> {
    use omniget_core::core::term::agent::launch::{recipe, LaunchCtx};
    let data = omniget_core::core::paths::app_data_dir()?;
    let ctx = LaunchCtx {
        terminal_id: tid.to_string(),
        env: env.clone(),
        work_dir: data.join("shell"),
        ..Default::default()
    };
    match recipe(TermAgent::Shell, &ctx) {
        Ok(l) if !l.command.is_empty() => Some(l),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("[canvas.launch] shell integration for {tid}: {e}");
            None
        }
    }
}

fn stored_node(ws: &str, floor: &str, node_id: &str) -> Result<CanvasNode, String> {
    super::doc::host()
        .store()
        .read(ws, floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))
}

pub async fn ensure_running(ws: &str, floor: &str, node_id: &str) -> Result<String, String> {
    let l = ensure(
        ws,
        floor,
        node_id,
        LaunchOpts {
            restore: true,
            ..Default::default()
        },
    )
    .await?;
    if l.fresh {
        wait_ready(&l.info.id).await;
    }
    Ok(l.info.id)
}

pub fn spawn_ensure(ws: &str, floor: &str, node_id: &str) {
    let (ws, floor, node_id) = (ws.to_string(), floor.to_string(), node_id.to_string());
    tauri::async_runtime::spawn(async move {
        if let Err(e) = ensure(
            &ws,
            &floor,
            &node_id,
            LaunchOpts {
                restore: true,
                ..Default::default()
            },
        )
        .await
        {
            tracing::warn!("[canvas.launch] {node_id}: {e}");
        }
    });
}

pub fn wake_workspace(ws: &str) {
    let ws = ws.to_string();
    tauri::async_runtime::spawn(async move {
        let s = super::doc::host().store().clone();
        let Ok(w) = s.workspace(&ws) else { return };
        let Ok(app) = app() else { return };
        let term = super::term::host(app);
        for f in &w.floors {
            let nodes = s.read(&ws, &f.id, |d| d.nodes.clone()).unwrap_or_default();
            for n in nodes {
                if !is_terminal(&n) || agent_of(&n) == "native" {
                    continue;
                }
                let Some(tid) = terminal_of(&n) else { continue };
                if term.info(&tid).is_some() {
                    continue;
                }
                let opts = LaunchOpts {
                    restore: true,
                    ..Default::default()
                };
                if let Err(e) = ensure(&ws, &f.id, &n.id, opts).await {
                    tracing::warn!("[canvas.launch] wake {}: {e}", n.id);
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    });
}

pub async fn ensure(
    ws: &str,
    floor: &str,
    node_id: &str,
    opts: LaunchOpts,
) -> Result<Launched, String> {
    let app = app()?;
    let m = super::term::host(app);
    let node = match (stored_node(ws, floor, node_id), opts.node) {
        (Ok(n), _) => n,
        (Err(_), Some(n)) if n.id == node_id => n,
        (Err(e), _) => return Err(e),
    };
    if !is_terminal(&node) {
        return Err(format!("CANVAS_INVALID: '{node_id}' is not a terminal"));
    }
    if agent_of(&node) == "native" {
        return Err("CANVAS_INVALID: a native agent has no terminal".into());
    }
    let tid = terminal_of(&node)
        .ok_or_else(|| format!("CANVAS_INVALID: node {node_id} has no terminal id"))?;

    let guard = lock_for(&tid);
    let _held = guard.lock().await;
    if let Some(info) = m.info(&tid) {
        return Ok(Launched { info, fresh: false });
    }

    let agent = agent_of(&node);
    let (gc, gr) = grid_for(node.w, node.h);
    let mut command = agent_command(&node);
    let generic = agent == "shell"
        && command.first().is_some_and(|p| {
            !matches!(
                omniget_core::core::term::session::foreground::label_for(p).as_str(),
                "zsh"
                    | "bash"
                    | "fish"
                    | "sh"
                    | "dash"
                    | "ksh"
                    | "nu"
                    | "pwsh"
                    | "powershell"
                    | "cmd"
            )
        });
    if opts.restore && command.len() == 1 {
        if let Some(sid) = omniget_core::core::term::session::history::agent_session(&tid) {
            match agent.as_str() {
                "claude" if omniget_core::core::canvas::adapters::claude::session_exists(&sid) => {
                    command.extend(["--resume".to_string(), sid]);
                }
                "codex" => {
                    command.extend(["resume".to_string(), sid]);
                }
                _ => {}
            }
        }
    }
    let mut cwd = node
        .data_str("cwd")
        .filter(|c| !c.trim().is_empty())
        .map(str::to_string);
    let mut env = base_env(&node, ws, &tid);
    let mut env_remove = None;

    let mut first_prompt = None;
    if let Some(role_id) = node.data_str("roleId").filter(|r| !r.is_empty()) {
        if !ws.is_empty() {
            let s = super::doc::host().store().clone();
            let (ws2, role, ag, c) = (
                ws.to_string(),
                role_id.to_string(),
                agent.clone(),
                cwd.clone(),
            );
            let plan = tauri::async_runtime::spawn_blocking(move || {
                roles::launch_plan(&s, &ws2, &role, &CanvasAgent::parse(&ag), c.as_deref())
            })
            .await
            .map_err(|e| format!("CANVAS_TASK: {e}"))
            .and_then(|r| r);
            match plan {
                Ok(plan) => {
                    if command.is_empty() {
                        command = preset(&agent);
                    }
                    if !command.is_empty() {
                        command.extend(plan.args.iter().cloned());
                    }
                    if plan.cwd.is_some() {
                        cwd = plan.cwd.clone();
                    }
                    env.extend(plan.env.clone());
                    first_prompt = plan.initial_prompt.clone();
                }
                Err(e) => tracing::warn!("[canvas.launch] role plan for {node_id}: {e}"),
            }
        }
    }

    if !ws.is_empty() {
        let (h, ws2, fl2, n2) = (app.clone(), ws.to_string(), floor.to_string(), node.clone());
        let ne = tauri::async_runtime::spawn_blocking(move || {
            super::cords::host().terminal_env_for(&h, &ws2, &fl2, &n2)
        })
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))
        .and_then(|r| r);
        match ne {
            Ok(ne) => {
                env.extend(ne.env);
                if !ne.env_remove.is_empty() {
                    env_remove = Some(ne.env_remove);
                }
                if !command.is_empty() && !ne.args.is_empty() {
                    let rest = command.split_off(1);
                    command.extend(ne.args);
                    command.extend(rest);
                }
            }
            Err(e) => tracing::warn!("[canvas.launch] {node_id} has no cords: {e}"),
        }

        let w = super::floors::spawn_context(ws, floor, cwd.as_deref(), &command, &env);
        command = w.command;
        if w.cwd.is_some() {
            cwd = w.cwd;
        }
        env = w.env.into_iter().collect();
    }

    if command.is_empty() && cfg!(unix) {
        if let Some(sh) = shell_integration(&tid, &env) {
            command = sh.command;
            env = sh.env;
        }
    }

    let req = TermOpen {
        id: Some(tid.clone()),
        cwd,
        command: Some(command),
        env: Some(env),
        cols: opts.cols.filter(|c| *c > 0).or(Some(gc)),
        rows: opts.rows.filter(|r| *r > 0).or(Some(gr)),
        title: node.title.clone(),
        agent: if generic {
            Some(TermAgent::Unknown)
        } else {
            serde_json::from_value::<TermAgent>(serde_json::Value::String(agent)).ok()
        },
        restore: Some(opts.restore),
        shell_fallback: Some(super::prefs::get(app).terminal.shell_fallback),
        env_remove,
        ..Default::default()
    };
    let m2 = m.clone();
    let info = tauri::async_runtime::spawn_blocking(move || m2.open(req))
        .await
        .map_err(|e| format!("PTY_OPEN: {e}"))??;
    drop(_held);

    if let Some(text) = first_prompt.filter(|t| !t.trim().is_empty()) {
        let id = tid.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let m = crate::pty::term();
            if m.paste(&id, &text).is_ok() {
                tokio::time::sleep(Duration::from_millis(150)).await;
                let _ = m.submit(&id);
            }
        });
    }
    tracing::info!("[canvas.launch] started {tid} for node {node_id}");
    Ok(Launched { info, fresh: true })
}

pub async fn wait_ready(tid: &str) {
    use omniget_core::core::term::agent::AgentState;
    let m = crate::pty::term();
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(800)).await;
    while start.elapsed() < Duration::from_secs(20) {
        let Some(info) = m.info(tid) else { return };
        if !info.alive {
            return;
        }
        if matches!(info.agent.state, AgentState::Idle | AgentState::Success) {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if let Ok(s) = m.screen_text(tid, 5) {
            if !s.text.trim().is_empty() && now - s.changed_ms > 1000 {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
