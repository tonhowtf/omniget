//! Omni as the canvas companion (the Maestri "Ombro"): watches every canvas
//! terminal through `term://state`, and when one finishes real work (at
//! least `minWorkSecs`), gets blocked or fails, reads the end of its screen,
//! writes a short summary plus three next steps and emits
//! `canvas://companion` (the canvas panel and the floating pet both listen).
//!
//! Model: the local one first (Ollama, smallest installed chat model), then
//! the configured cheap agent (its side model, same account), then a
//! heuristic with no model. Pure parts live in
//! `omniget_core::core::canvas::companion`.
//!
//! Clicking a suggestion pastes it into that agent's terminal and submits
//! it (`TermManager::send_prompt`), never anything else.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use omniget_core::core::canvas::companion::{
    self as core, CompanionEntry, CompanionKind, CompanionSettings, ModelMode, Summary, Tracker,
};
use omniget_core::core::canvas::model::{CanvasNode, TerminalData};
use omniget_core::core::term::agent::AgentState;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Listener, Manager};

use super::rpc::{NodeCtx, RpcResult};

pub const EVENT_COMPANION: &str = "canvas://companion";
pub const EVENT_DISMISS: &str = "canvas://companion-dismiss";
const SIDE_TIMEOUT: Duration = Duration::from_secs(60);

struct Companion {
    settings: Mutex<CompanionSettings>,
    history: Mutex<VecDeque<CompanionEntry>>,
    tracker: Mutex<Tracker>,
    in_flight: Mutex<HashSet<String>>,
    app: OnceLock<AppHandle>,
    started: AtomicBool,
}

static COMPANION: OnceLock<Companion> = OnceLock::new();

fn dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("canvas"))
}

fn me() -> &'static Companion {
    COMPANION.get_or_init(|| {
        let settings = dir()
            .and_then(|d| std::fs::read(d.join("companion.json")).ok())
            .and_then(|b| serde_json::from_slice::<CompanionSettings>(&b).ok())
            .unwrap_or_default()
            .sanitized();
        let history = dir()
            .and_then(|d| std::fs::read(d.join("companion-history.json")).ok())
            .and_then(|b| serde_json::from_slice::<VecDeque<CompanionEntry>>(&b).ok())
            .unwrap_or_default();
        Companion {
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            tracker: Mutex::new(Tracker::new()),
            in_flight: Mutex::new(HashSet::new()),
            app: OnceLock::new(),
            started: AtomicBool::new(false),
        }
    })
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn write_json(name: &str, v: &impl serde::Serialize) {
    let Some(d) = dir() else { return };
    let _ = std::fs::create_dir_all(&d);
    match serde_json::to_vec_pretty(v) {
        Ok(bytes) => {
            let tmp = d.join(format!("{name}.tmp"));
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, d.join(name));
            }
        }
        Err(e) => tracing::warn!("[companion] saving {name}: {e}"),
    }
}

fn save_history() {
    let h = me().history.lock().unwrap().clone();
    write_json("companion-history.json", &h);
}

pub fn settings() -> CompanionSettings {
    me().settings.lock().unwrap().clone()
}

/// Settings with the language filled from the app's own when the front has
/// not set one yet (MCP calls, the pet before the canvas opened).
fn effective(app: &AppHandle) -> CompanionSettings {
    let mut cfg = settings();
    if cfg.language.is_none() {
        let l = crate::storage::config::load_settings(app)
            .appearance
            .language;
        if !l.trim().is_empty() {
            cfg.language = Some(l);
        }
    }
    cfg
}

pub fn set_settings(next: CompanionSettings) -> CompanionSettings {
    let next = next.sanitized();
    *me().settings.lock().unwrap() = next.clone();
    write_json("companion.json", &next);
    next
}

/// Starts watching (idempotent). The canvas commands and the app setup call
/// it; nothing runs before the first call.
pub fn start(app: &AppHandle) {
    let c = me();
    let _ = c.app.set(app.clone());
    if c.started.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    app.listen_any("term://state", move |ev| {
        let Ok(v) = serde_json::from_str::<Value>(ev.payload()) else {
            return;
        };
        let (Some(id), Some(state)) = (v["id"].as_str(), v["state"].as_str()) else {
            return;
        };
        let Ok(state) = serde_json::from_value::<AgentState>(json!(state)) else {
            return;
        };
        on_state(&handle, id, state);
    });
    register_shortcut(app);
}

// ---------------------------------------------------------------- window

/// Label of Omni's own floating window (Maestri: the Ombro window, ⌘⇧O).
pub const OMNI_WINDOW: &str = "omni";
/// Global shortcut that shows and hides it.
pub const OMNI_SHORTCUT: &str = "CommandOrControl+Shift+O";

/// Registers ⌘⇧O (Ctrl+Shift+O) system-wide. Called at start and again by
/// `hotkey::reregister`, which clears every global shortcut first.
pub fn register_shortcut(app: &AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let Ok(sc) = OMNI_SHORTCUT.parse::<Shortcut>() else {
        return;
    };
    let gs = app.global_shortcut();
    if gs.is_registered(sc) {
        return;
    }
    if let Err(e) = gs.register(sc) {
        tracing::warn!("[omni] {OMNI_SHORTCUT} not registered (taken by another app?): {e}");
    }
}

/// The global-shortcut handler hands every press here first; true when it
/// was Omni's.
pub fn on_shortcut(app: &AppHandle, pressed: &tauri_plugin_global_shortcut::Shortcut) -> bool {
    let mine = OMNI_SHORTCUT
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .is_ok_and(|s| s == *pressed);
    if mine {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = toggle_window(&app, None) {
                tracing::warn!("[omni] window: {e}");
            }
        });
    }
    mine
}

/// Shows (`Some(true)`), hides (`Some(false)`) or toggles (`None`) Omni's
/// window. Returns whether it is visible now.
pub fn toggle_window(app: &AppHandle, show: Option<bool>) -> Result<bool, String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if let Some(w) = app.get_webview_window(OMNI_WINDOW) {
        let visible = w.is_visible().unwrap_or(false);
        let want = show.unwrap_or(!visible || !w.is_focused().unwrap_or(false));
        if want {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        } else {
            let _ = w.hide();
        }
        return Ok(want);
    }
    if show == Some(false) {
        return Ok(false);
    }
    let w = crate::core::portable::with_dev_webview_dir(WebviewWindowBuilder::new(
        app,
        OMNI_WINDOW,
        WebviewUrl::App("/omni".into()),
    ))
    .title("Omni")
    .inner_size(400.0, 600.0)
    .min_inner_size(320.0, 360.0)
    .resizable(true)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .focused(true)
    .build()
    .map_err(|e| format!("CANVAS_WINDOW: {e}"))?;
    let _ = w.set_focus();
    Ok(true)
}

/// Event every window listens to for a live language change: `{language}`.
pub const EVENT_LANGUAGE: &str = "app://language";

/// The app language changed (settings screen or `app_settings_set`): Omni
/// writes its next summaries in it and every window re-renders in it.
pub fn on_language(app: &AppHandle, language: &str) {
    let lang = language.trim();
    if lang.is_empty() {
        return;
    }
    let mut cfg = settings();
    if cfg.language.as_deref() != Some(lang) {
        cfg.language = Some(lang.to_string());
        set_settings(cfg);
    }
    let _ = app.emit(EVENT_LANGUAGE, json!({ "language": lang }));
}

fn app() -> Option<&'static AppHandle> {
    me().app.get()
}

/// The app handle once anything started the companion (or the chat face).
pub fn app_handle() -> Option<&'static AppHandle> {
    app().or_else(super::chat::app)
}

fn store() -> std::sync::Arc<omniget_core::core::canvas::store::CanvasStore> {
    super::doc::host().store().clone()
}

/// A canvas terminal node and where it lives: `(ws, floor, node)`.
fn locate(terminal_id: &str) -> Option<(String, String, CanvasNode)> {
    let s = store();
    let (ws, floor, node_id) = s.locate_terminal(terminal_id)?;
    let node = s.read(&ws, &floor, |d| d.node(&node_id).cloned()).ok()??;
    Some((ws, floor, node))
}

fn on_state(app: &AppHandle, terminal_id: &str, state: AgentState) {
    let cfg = settings();
    if !cfg.enabled {
        return;
    }
    let fired = me().tracker.lock().unwrap().observe(
        terminal_id,
        state,
        now_ms(),
        cfg.min_work_secs as i64 * 1000,
    );
    let Some((kind, worked)) = fired else { return };
    if !cfg.wants(kind) {
        return;
    }
    // Only canvas nodes: the Central's terminals are not Omni's business here.
    let Some((ws, floor, node)) = locate(terminal_id) else {
        return;
    };
    let app = app.clone();
    let tid = terminal_id.to_string();
    tauri::async_runtime::spawn(async move {
        if kind == CompanionKind::Blocked {
            // A permission prompt flickers through blocked; speak only when
            // it is still there a moment later.
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            let still = crate::pty::term()
                .agent_state(&tid)
                .is_some_and(|s| s.state == AgentState::Blocked);
            if !still {
                return;
            }
        }
        let _ = produce(&app, kind, worked, &ws, &floor, &node, &tid).await;
    });
}

fn node_agent(node: &CanvasNode) -> (String, Option<String>, String) {
    let data: Option<TerminalData> = node.data_as();
    let agent = node.data_str("agent").unwrap_or("shell").to_string();
    let role = data.as_ref().and_then(|d| d.role_id.clone());
    let tid = data.map(|d| d.terminal_id).unwrap_or_default();
    (agent, role, tid)
}

/// Writes one entry for `node` and emits it.
async fn produce(
    app: &AppHandle,
    kind: CompanionKind,
    worked_ms: i64,
    ws: &str,
    floor: &str,
    node: &CanvasNode,
    terminal_id: &str,
) -> Result<CompanionEntry, String> {
    {
        let mut f = me().in_flight.lock().unwrap();
        if !f.insert(node.id.clone()) {
            return Err("CANVAS_BUSY: Omni is already looking at this node".into());
        }
    }
    let out = produce_inner(app, kind, worked_ms, ws, floor, node, terminal_id).await;
    me().in_flight.lock().unwrap().remove(&node.id);
    out
}

async fn produce_inner(
    app: &AppHandle,
    kind: CompanionKind,
    worked_ms: i64,
    ws: &str,
    floor: &str,
    node: &CanvasNode,
    terminal_id: &str,
) -> Result<CompanionEntry, String> {
    let cfg = effective(app);
    let name = super::cords::node_name(node);
    let (agent, role, _) = node_agent(node);
    let raw = crate::pty::term()
        .screen_text(terminal_id, 120)
        .map(|s| s.text)
        .unwrap_or_default();
    let screen = core::clean_screen(&raw, core::SCREEN_MAX_CHARS);
    let lang = cfg.language.as_deref();
    // A "failed" with no error on the screen is a program that stopped.
    let kind = core::refine_kind(kind, &screen);
    let fallback = core::heuristic(kind, &name, &agent, worked_ms, &screen, lang);
    let (summary, source) = generate(
        app,
        &cfg,
        kind,
        &name,
        &agent,
        role.as_deref(),
        worked_ms,
        &screen,
        &fallback,
    )
    .await;
    let i18n = if source == "heuristic" {
        summary.i18n.clone()
    } else {
        None
    };
    let entry = CompanionEntry {
        id: format!("cmp_{}", uuid::Uuid::new_v4().simple()),
        node_id: node.id.clone(),
        workspace_id: ws.to_string(),
        floor_id: floor.to_string(),
        terminal_id: terminal_id.to_string(),
        name,
        agent,
        kind,
        summary: summary.summary,
        suggestions: summary.suggestions,
        command: summary.command,
        source,
        at_ms: now_ms(),
        worked_ms,
        celebrate: kind == CompanionKind::Done && worked_ms >= cfg.min_work_secs as i64 * 1000,
        dismissed: false,
        i18n,
    };
    {
        let mut h = me().history.lock().unwrap();
        h.push_back(entry.clone());
        while h.len() > core::HISTORY_CAP {
            h.pop_front();
        }
    }
    save_history();
    let mut payload = serde_json::to_value(&entry).unwrap_or(Value::Null);
    payload["petBalloon"] = json!(cfg.pet_balloon && kind != CompanionKind::Manual);
    let _ = app.emit(EVENT_COMPANION, payload);
    Ok(entry)
}

#[allow(clippy::too_many_arguments)]
async fn generate(
    app: &AppHandle,
    cfg: &CompanionSettings,
    kind: CompanionKind,
    name: &str,
    agent: &str,
    role: Option<&str>,
    worked_ms: i64,
    screen: &str,
    fallback: &Summary,
) -> (Summary, String) {
    if screen.trim().is_empty() || cfg.model_mode == ModelMode::Heuristic {
        return (fallback.clone(), "heuristic".into());
    }
    let lang = cfg.language.as_deref();
    let (system, user) = core::prompt(kind, name, agent, role, worked_ms, screen, lang);
    if matches!(cfg.model_mode, ModelMode::Auto | ModelMode::Local) {
        match core::ollama_generate(&cfg.ollama_url, cfg.ollama_model.as_deref(), &system, &user)
            .await
        {
            Ok((model, text)) => {
                if let Some(s) =
                    core::parse_reply(&text, fallback).or_else(|| plain(&text, fallback))
                {
                    return (s, format!("ollama:{model}"));
                }
                tracing::debug!("[companion] {model} gave no usable reply");
            }
            Err(e) => tracing::debug!("[companion] local model: {e}"),
        }
    }
    if matches!(cfg.model_mode, ModelMode::Auto | ModelMode::Cheap) {
        let agent_id = cfg.cheap_agent_id.clone().or_else(|| {
            (cfg.model_mode == ModelMode::Cheap)
                .then(|| {
                    app.state::<crate::AppState>()
                        .llm
                        .roster()
                        .first()
                        .map(|a| a.id.clone())
                })
                .flatten()
        });
        if let Some(agent_id) = agent_id {
            match side_call(app, &agent_id, &system, &user).await {
                Ok((model, text)) => {
                    if let Some(s) = core::parse_reply(&text, fallback) {
                        return (s, format!("llm:{model}"));
                    }
                }
                Err(e) => tracing::debug!("[companion] cheap model: {e}"),
            }
        }
    }
    if cfg.cli_fallback && matches!(cfg.model_mode, ModelMode::Auto | ModelMode::Cheap) {
        if let Some((cli, bin)) = core::pick_cli(cfg.cli_agent.as_deref(), &[agent.to_string()]) {
            match core::cli_generate(&cli, &bin, &system, &user).await {
                Ok((label, text)) => {
                    if let Some(s) =
                        core::parse_reply(&text, fallback).or_else(|| plain(&text, fallback))
                    {
                        return (s, label);
                    }
                    tracing::debug!("[companion] {cli} gave no usable reply");
                }
                Err(e) => tracing::debug!("[companion] cli: {e}"),
            }
        }
    }
    (fallback.clone(), "heuristic".into())
}

/// A model that answered in prose instead of JSON still said something
/// true about the screen: keep its words, the heuristic's steps.
fn plain(text: &str, fallback: &Summary) -> Option<Summary> {
    let t = text.trim();
    if t.is_empty() || t.starts_with('{') || t.starts_with("```") {
        return None;
    }
    let one: String = t
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    let summary: String = one.chars().take(core::SUMMARY_MAX_CHARS).collect();
    Some(Summary {
        summary,
        suggestions: fallback.suggestions.clone(),
        command: fallback.command.clone(),
        i18n: None,
    })
}

/// A tool-less side turn of `agent_id` on its cheapest model.
async fn side_call(
    app: &AppHandle,
    agent_id: &str,
    system: &str,
    user: &str,
) -> Result<(String, String), String> {
    use omniget_core::core::llm::router;
    use omniget_core::core::llm::types::TurnEvent;
    let llm = app.state::<crate::AppState>().llm.clone();
    let conv = format!(
        "{}companion-{}",
        router::SIDE_PREFIX,
        uuid::Uuid::new_v4().simple()
    );
    let (request, cancel, mut stream, model) = llm
        .side_turn_stream(&conv, agent_id, system, user, None)
        .await?;
    let mut text = String::new();
    let mut error: Option<String> = None;
    let run = async {
        while let Some(ev) = stream.next().await {
            match &ev {
                TurnEvent::TextDelta { text: t } => text.push_str(t),
                TurnEvent::Error { error: e } => error = Some(e.to_string()),
                _ => {}
            }
        }
    };
    if tokio::time::timeout(SIDE_TIMEOUT, run).await.is_err() {
        cancel.cancel();
        error = Some("timeout".into());
    }
    llm.finish_turn(&request, agent_id);
    llm.drop_conversation(&conv);
    match error {
        Some(e) if text.trim().is_empty() => Err(e),
        _ => Ok((model.model, text)),
    }
}

// ---------------------------------------------------------------- API

/// Newest first; `ws` filters; dismissed entries are skipped unless asked.
pub fn history(ws: Option<&str>, limit: usize, include_dismissed: bool) -> Vec<CompanionEntry> {
    me().history
        .lock()
        .unwrap()
        .iter()
        .rev()
        .filter(|e| ws.is_none_or(|w| e.workspace_id == w))
        .filter(|e| include_dismissed || !e.dismissed)
        .take(limit.clamp(1, core::HISTORY_CAP))
        .cloned()
        .collect()
}

pub fn dismiss(id: &str) -> bool {
    let found = {
        let mut h = me().history.lock().unwrap();
        match h.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.dismissed = true;
                true
            }
            None => false,
        }
    };
    if found {
        save_history();
        if let Some(app) = app() {
            let _ = app.emit(EVENT_DISMISS, json!({ "id": id }));
        }
    }
    found
}

/// Sends suggestion `index` (0-2; 3 = the direct command) of entry `id`, or
/// `text` when given, to the entry's node. The entry is dismissed after.
pub fn act(id: &str, index: Option<usize>, text: Option<String>) -> Result<Value, String> {
    let entry = me()
        .history
        .lock()
        .unwrap()
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: companion entry {id}"))?;
    let text = match text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => {
            let i = index.unwrap_or(0);
            if i == 3 {
                entry.command.clone()
            } else {
                entry.suggestions.get(i).cloned()
            }
            .ok_or_else(|| "CANVAS_INVALID: no such suggestion".to_string())?
        }
    };
    send_to_node(&entry.node_id, &text)?;
    dismiss(id);
    Ok(json!({ "ok": true, "nodeId": entry.node_id, "text": text }))
}

/// Pastes `text` into a canvas terminal node and submits it.
pub fn send_to_node(node_id: &str, text: &str) -> Result<(), String> {
    let s = store();
    let (ws, floor) = s
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    let node = s
        .read(&ws, &floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    let (_, _, tid) = node_agent(&node);
    if tid.is_empty() {
        return Err("CANVAS_INVALID: not a terminal node".into());
    }
    let term = crate::pty::term();
    if !term.info(&tid).is_some_and(|i| i.alive) {
        return Err("CANVAS_NOT_RUNNING: the agent is not running".into());
    }
    let text: String = text
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    term.send_prompt(&tid, text.trim())
}

/// "Ask Omni" on one node, now, whatever its state.
pub async fn summarize_node(app: &AppHandle, node_id: &str) -> Result<CompanionEntry, String> {
    let s = store();
    let (ws, floor) = s
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    let node = s
        .read(&ws, &floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    let (_, _, tid) = node_agent(&node);
    if tid.is_empty() {
        return Err("CANVAS_INVALID: not a terminal node".into());
    }
    produce(app, CompanionKind::Manual, 0, &ws, &floor, &node, &tid).await
}

// ---------------------------------------------------------------- free questions

/// Everything Omni reads to answer a free question about a workspace.
struct AskScene {
    active_floor: String,
    floor_names: Vec<(String, String)>,
    terminals: Vec<core::AskTerminal>,
    notes: Vec<core::AskNote>,
    /// Agents run on this workspace (`claude`, `codex`...), for the CLI pick.
    used: Vec<String>,
    /// The asked-about node in depth: role, recent chat, connected notes.
    focus: String,
}

/// Role, recent chat and connected notes of one terminal node, as text.
fn focus_context(ws: &str, floor: &str, node: &CanvasNode) -> String {
    use omniget_core::core::canvas::{chat, notes, roles};
    let s = store();
    let name = super::cords::node_name(node);
    let (agent, role, _) = node_agent(node);
    let mut out = format!("## Focus: \"{name}\" (id {}, agent {agent})\n", node.id);
    if let Some((r, _)) = role.as_deref().and_then(|id| roles::get(&s, ws, id)) {
        let p: String = r.prompt.chars().take(400).collect();
        out.push_str(&format!("Role: {} - {p}\n", r.name));
    }
    if let Ok(entries) = chat::read_all(&s, ws, &node.id) {
        let tail: Vec<_> = entries.iter().rev().take(12).collect();
        if !tail.is_empty() {
            out.push_str("Recent chat:\n");
            for e in tail.into_iter().rev() {
                let who = serde_json::to_value(e.role)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                let t: String = e.text.chars().take(300).collect();
                out.push_str(&format!("- {who}: {t}\n"));
            }
        }
    }
    let linked: Vec<CanvasNode> = s
        .read(ws, floor, |d| {
            d.connections
                .iter()
                .filter_map(|c| c.other_end(&node.id))
                .filter_map(|id| d.node(id))
                .filter(|n| n.kind == omniget_core::core::canvas::model::NodeKind::Note)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for n in linked.iter().take(5) {
        if let Ok(c) = notes::read(&s, ws, floor, &n.id) {
            let t: String = c.text.chars().take(800).collect();
            out.push_str(&format!("Connected note \"{}\":\n{t}\n", c.display_name));
        }
    }
    out
}

fn is_agent_node(n: &CanvasNode) -> bool {
    matches!(
        n.kind,
        omniget_core::core::canvas::model::NodeKind::Terminal
            | omniget_core::core::canvas::model::NodeKind::Agent
    )
}

fn fused_word(node_id: &str) -> String {
    super::cords::host()
        .node_state(node_id)
        .and_then(|f| serde_json::to_value(f.state).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "no_signal".into())
}

/// Reads terminals (state + last lines) and notes of every floor of `ws`.
/// `first` (a node id) goes to the top of the terminal list.
fn ask_scene(ws: &str, first: Option<&str>) -> Result<AskScene, String> {
    use omniget_core::core::canvas::notes;
    let s = store();
    let meta = s.workspace(ws)?;
    let term = crate::pty::term();
    let mut terminals = Vec::new();
    let mut floor_names = Vec::new();
    for f in &meta.floors {
        floor_names.push((f.id.clone(), f.name.clone()));
        let nodes = s.read(ws, &f.id, |d| {
            d.nodes
                .iter()
                .filter(|n| is_agent_node(n))
                .cloned()
                .collect::<Vec<_>>()
        })?;
        for n in nodes {
            let (agent, _, tid) = node_agent(&n);
            let raw = if tid.is_empty() {
                String::new()
            } else {
                term.screen_text(&tid, 60)
                    .map(|s| s.text)
                    .unwrap_or_default()
            };
            terminals.push(core::AskTerminal {
                node_id: n.id.clone(),
                name: super::cords::node_name(&n),
                agent,
                floor: f.name.clone(),
                state: fused_word(&n.id),
                screen: core::clean_screen(&raw, 1_500),
            });
        }
    }
    if let Some(id) = first {
        if let Some(i) = terminals.iter().position(|t| t.node_id == id) {
            let t = terminals.remove(i);
            terminals.insert(0, t);
        }
    }
    let mut out_notes = Vec::new();
    for n in notes::list(&s, ws).unwrap_or_default().into_iter().take(40) {
        let text = match (&n.node_id, &n.floor_id) {
            (Some(id), Some(fl)) => notes::read(&s, ws, fl, id)
                .map(|c| c.text)
                .unwrap_or(n.preview.clone()),
            _ => std::fs::read_to_string(&n.path).unwrap_or(n.preview.clone()),
        };
        out_notes.push(core::AskNote {
            node_id: n.node_id.clone(),
            title: n.display_name.clone(),
            text,
        });
    }
    let mut used: Vec<String> = Vec::new();
    for t in &terminals {
        if !used.contains(&t.agent) {
            used.push(t.agent.clone());
        }
    }
    let focus = first
        .and_then(|id| {
            let (w, f) = s.locate_node(id)?;
            let n = s.read(&w, &f, |d| d.node(id).cloned()).ok()??;
            Some(focus_context(&w, &f, &n))
        })
        .unwrap_or_default();
    Ok(AskScene {
        active_floor: meta.active_floor_id.clone(),
        floor_names,
        terminals,
        notes: out_notes,
        used,
        focus,
    })
}

/// Appends `item` to the workspace's "Omni Notes" note (created on the
/// active floor the first time). Returns `(node_id, created)`.
fn omni_note_append(ws: &str, active_floor: &str, item: &str) -> Result<(String, bool), String> {
    use omniget_core::core::canvas::model::NodeKind;
    use omniget_core::core::canvas::{layout, notes};
    let s = store();
    let meta = s.workspace(ws)?;
    let mut floors: Vec<String> = meta.floors.iter().map(|f| f.id.clone()).collect();
    floors.sort_by_key(|f| f != active_floor);
    for f in &floors {
        let hit = s.read(ws, f, |d| {
            d.nodes
                .iter()
                .find(|n| {
                    n.kind == NodeKind::Note
                        && n.title.as_deref().map(str::trim) == Some(core::OMNI_NOTES_TITLE)
                })
                .map(|n| n.id.clone())
        })?;
        if let Some(id) = hit {
            let cur = notes::read(&s, ws, f, &id)?;
            let next = core::append_note(&cur.text, item);
            notes::write(&s, ws, f, &id, &next, Some(&cur.revision), "omni")?;
            return Ok((id, false));
        }
    }
    let size = omniget_core::core::canvas::model::default_size(&NodeKind::Note);
    let at = s.read(ws, active_floor, |d| {
        layout::free_spot_in(d, size.w, size.h, None, &[])
    })?;
    let text = core::append_note("", item);
    let id = notes::create(&s, ws, active_floor, at.x, at.y, &text, "omni")?;
    Ok((id, true))
}

/// Local model first, then the cheap agent (per the companion settings).
/// `(answer, sources, source)` or `None` when no model answered.
async fn ask_model(
    app: &AppHandle,
    cfg: &CompanionSettings,
    system: &str,
    user: &str,
    known: &[String],
    used: &[String],
) -> Option<(String, Vec<String>, String)> {
    if cfg.model_mode == ModelMode::Heuristic {
        return None;
    }
    if matches!(cfg.model_mode, ModelMode::Auto | ModelMode::Local) {
        match core::ollama_generate(&cfg.ollama_url, cfg.ollama_model.as_deref(), system, user)
            .await
        {
            Ok((model, text)) => {
                if let Some((a, s)) = core::parse_answer(&text, known) {
                    return Some((a, s, format!("ollama:{model}")));
                }
                // A plain-text reply from a small model is still an answer.
                let t = text.trim();
                if !t.is_empty() && !t.starts_with('{') {
                    return Some((t.to_string(), vec![], format!("ollama:{model}")));
                }
            }
            Err(e) => tracing::debug!("[companion] ask local: {e}"),
        }
    }
    if matches!(cfg.model_mode, ModelMode::Auto | ModelMode::Cheap) {
        let agent_id = cfg.cheap_agent_id.clone().or_else(|| {
            (cfg.model_mode == ModelMode::Cheap)
                .then(|| {
                    app.state::<crate::AppState>()
                        .llm
                        .roster()
                        .first()
                        .map(|a| a.id.clone())
                })
                .flatten()
        });
        if let Some(agent_id) = agent_id {
            match side_call(app, &agent_id, system, user).await {
                Ok((model, text)) => {
                    if let Some((a, s)) = core::parse_answer(&text, known) {
                        return Some((a, s, format!("llm:{model}")));
                    }
                }
                Err(e) => tracing::debug!("[companion] ask cheap: {e}"),
            }
        }
    }
    if cfg.cli_fallback && matches!(cfg.model_mode, ModelMode::Auto | ModelMode::Cheap) {
        if let Some((cli, bin)) = core::pick_cli(cfg.cli_agent.as_deref(), used) {
            match core::cli_generate(&cli, &bin, system, user).await {
                Ok((label, text)) => {
                    if let Some((a, s)) = core::parse_answer(&text, known) {
                        return Some((a, s, label));
                    }
                    let t = text.trim();
                    if !t.is_empty() {
                        return Some((t.to_string(), vec![], label));
                    }
                }
                Err(e) => tracing::debug!("[companion] ask cli: {e}"),
            }
        }
    }
    None
}

/// A free question to Omni about workspace `ws` (optionally centred on
/// `node_id`). "Note that ..." appends to the Omni Notes note; "summarize my
/// notes" reads the notes; anything else is answered from the terminals'
/// states and screens and the notes. Without a model, a deterministic
/// digest (who works, who needs you, last lines).
pub async fn ask(
    app: &AppHandle,
    ws: &str,
    question: &str,
    node_id: Option<&str>,
) -> Result<Value, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("MISSING_ARGUMENT: question".into());
    }
    let cfg = effective(app);
    let lang = core::question_language(&question)
        .map(str::to_string)
        .or_else(|| cfg.language.clone());
    let pt = lang.as_deref().unwrap_or("en").starts_with("pt");
    let (ws2, first) = (ws.to_string(), node_id.map(str::to_string));
    let scene = tauri::async_runtime::spawn_blocking(move || ask_scene(&ws2, first.as_deref()))
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))??;
    match core::ask_intent(&question) {
        core::AskIntent::Note(item) => {
            let (ws2, fl) = (ws.to_string(), scene.active_floor.clone());
            let (id, created) =
                tauri::async_runtime::spawn_blocking(move || omni_note_append(&ws2, &fl, &item))
                    .await
                    .map_err(|e| format!("CANVAS_TASK: {e}"))??;
            let answer = match (created, pt) {
                (true, true) => format!("Criei a nota \"{}\" e anotei.", core::OMNI_NOTES_TITLE),
                (true, false) => format!(
                    "Created the note \"{}\" and wrote it down.",
                    core::OMNI_NOTES_TITLE
                ),
                (false, true) => format!("Anotei em \"{}\".", core::OMNI_NOTES_TITLE),
                (false, false) => format!("Added to \"{}\".", core::OMNI_NOTES_TITLE),
            };
            Ok(json!({
                "intent": "note",
                "answer": answer,
                "noteId": id,
                "created": created,
                "appended": !created,
                "sources": [id],
                "source": "omni",
                "workspaceId": ws,
            }))
        }
        core::AskIntent::SummarizeNotes => {
            let known: Vec<String> = scene
                .notes
                .iter()
                .filter_map(|n| n.node_id.clone())
                .collect();
            let digest = core::notes_digest(&scene.notes, lang.as_deref());
            let ctx = core::ask_context(ws, &[], &scene.notes, 6_000);
            let (system, user) = core::ask_prompt(&question, &ctx, lang.as_deref());
            let (answer, sources, source) = if scene.notes.is_empty() {
                (digest, vec![], "digest".to_string())
            } else {
                match ask_model(app, &cfg, &system, &user, &known, &scene.used).await {
                    Some((a, s, src)) => (a, if s.is_empty() { known.clone() } else { s }, src),
                    None => (digest, known.clone(), "digest".into()),
                }
            };
            Ok(json!({
                "intent": "summarize_notes",
                "answer": answer,
                "sources": sources,
                "source": source,
                "notes": scene.notes.iter().map(|n| json!({"nodeId": n.node_id, "title": n.title})).collect::<Vec<_>>(),
                "workspaceId": ws,
            }))
        }
        core::AskIntent::Free => {
            let known: Vec<String> = scene
                .terminals
                .iter()
                .map(|t| t.node_id.clone())
                .chain(scene.notes.iter().filter_map(|n| n.node_id.clone()))
                .collect();
            let name = store()
                .workspace(ws)
                .map(|w| w.name)
                .unwrap_or_else(|_| ws.to_string());
            let mut ctx = core::ask_context(&name, &scene.terminals, &scene.notes, 9_000);
            if !scene.focus.is_empty() {
                ctx = format!("{}\n\n{ctx}", scene.focus);
            }
            let (system, user) = core::ask_prompt(&question, &ctx, lang.as_deref());
            let (answer, sources, source) =
                match ask_model(app, &cfg, &system, &user, &known, &scene.used).await {
                    Some(r) => r,
                    None => {
                        let digest =
                            core::status_digest(&question, &scene.terminals, lang.as_deref());
                        let head = if pt {
                            "Modelo offline; o que vejo agora:"
                        } else {
                            "Model offline; what I can see right now:"
                        };
                        let q = question.to_lowercase();
                        let named: Vec<String> = scene
                            .terminals
                            .iter()
                            .filter(|t| core::mentions(&q, &t.name))
                            .map(|t| t.node_id.clone())
                            .collect();
                        (format!("{head}\n{digest}"), named, "digest".into())
                    }
                };
            Ok(json!({
                "intent": "free",
                "answer": answer,
                "sources": sources,
                "source": source,
                "terminals": scene.terminals.iter().map(|t| json!({"nodeId": t.node_id, "name": t.name, "agent": t.agent, "floor": t.floor, "state": t.state})).collect::<Vec<_>>(),
                "floors": scene.floor_names.iter().map(|(id, n)| json!({"id": id, "name": n})).collect::<Vec<_>>(),
                "workspaceId": ws,
            }))
        }
    }
}

/// The companion has no agent-facing methods: its summaries read other
/// nodes' screens, which an agent may only do through a cord (`check`).
pub async fn rpc(_ctx: &NodeCtx, _method: &str, _args: Value) -> RpcResult {
    None
}
