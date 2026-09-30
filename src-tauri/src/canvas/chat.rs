//! Chat face host: delivers the user's chat messages to the node's terminal
//! and answers the agent's `omniget canvas say` / `recall` (RPC `chat.say`,
//! `chat.recall`). Transcript and envelope live in
//! `omniget_core::core::canvas::chat`. Every new entry is emitted as
//! `canvas://chat {workspaceId, nodeId, thread, message}`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use omniget_core::core::canvas::chat::{self, ChatEntry, RecallArgs, SayArgs};
use omniget_core::core::canvas::model::{CanvasNode, TerminalData};
use omniget_core::core::canvas::store::CanvasStore;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::rpc::{NodeCtx, RpcResult};

pub const EVENT_CHAT: &str = "canvas://chat";

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Gives the chat (and roles) host the handle it emits with. Idempotent; the
/// canvas commands call it, and the app may call it at setup so agent `say`
/// calls reach the UI before any chat command ran.
pub fn bind(app: &AppHandle) {
    if APP.set(app.clone()).is_ok() {
        start_turn_watch();
    }
}

pub fn app() -> Option<&'static AppHandle> {
    APP.get()
}

pub fn emit<S: Serialize + Clone>(event: &str, payload: S) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    pub workspace_id: String,
    pub node_id: String,
    pub thread: u8,
    pub message: ChatEntry,
}

pub fn emit_entry(ws: &str, node: &str, entry: &ChatEntry) {
    emit(
        EVENT_CHAT,
        ChatEvent {
            workspace_id: ws.to_string(),
            node_id: node.to_string(),
            thread: entry.thread,
            message: entry.clone(),
        },
    );
}

fn store() -> std::sync::Arc<CanvasStore> {
    super::doc::host().store().clone()
}

/// The terminal node `node_id` of workspace `ws` (any floor).
pub fn terminal_node(
    store: &CanvasStore,
    ws: &str,
    node_id: &str,
) -> Result<(String, CanvasNode), String> {
    let (w, floor) = store
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    if w != ws {
        return Err(format!(
            "CANVAS_NOT_FOUND: node {node_id} in workspace {ws}"
        ));
    }
    let node = store
        .read(ws, &floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
    if !node.is_terminal() {
        return Err("CANVAS_INVALID: the chat face belongs to terminal nodes".into());
    }
    Ok((floor, node))
}

/// Delivers the envelope through the cords delivery loop (waits for the
/// agent to be idle and the user not typing, pastes, sends Enter apart and
/// confirms). Runs in the background: the entry is already recorded.
fn deliver(node_id: &str, text: &str) -> Result<(), String> {
    let node_id = node_id.to_string();
    let text = text.to_string();
    tauri::async_runtime::spawn(async move {
        if !super::cords::host().deliver_chat(&node_id, text).await {
            tracing::warn!("[canvas.chat] message to {node_id} was not taken");
        }
    });
    Ok(())
}

/// User → agent. Records the message, then delivers the envelope (starting
/// the terminal when this app run has not). Fails before recording anything
/// when its process exited.
pub fn send(
    ws: &str,
    node_id: &str,
    thread: u8,
    text: &str,
    files: &[String],
    client_message_id: Option<String>,
) -> Result<ChatEntry, String> {
    let s = store();
    let (_, node) = terminal_node(&s, ws, node_id)?;
    let data: TerminalData = node
        .data_as()
        .ok_or_else(|| "CANVAS_INVALID: terminal data".to_string())?;
    // No session in this app run (off screen after a restart) is fine: the
    // delivery starts it (`cords::deliver_chat` -> `launch::ensure_running`).
    // A process that exited in this run stays an error.
    let term = crate::pty::term();
    let exited = term.info(&data.terminal_id).is_some_and(|i| !i.alive);
    if exited {
        return Err(
            "CANVAS_NOT_RUNNING: the agent is not running; open the terminal to restart it".into(),
        );
    }
    let out = chat::user_message(&s, ws, node_id, thread, text, files, client_message_id)?;
    emit_entry(ws, node_id, &out.entry);
    emit_state(ws, node_id, &chat::load_state(&s, ws, node_id));
    watch_turn(ws, node_id);
    deliver(node_id, &out.envelope)?;
    Ok(out.entry)
}

/// Emitted when the per-node chat state changes (thread selected, turn
/// opened or closed, unread counts): `{workspaceId, nodeId, activeThread,
/// turnThread, turnOpen, turnInterrupted, unread, unreadCount}`.
pub const EVENT_CHAT_STATE: &str = "canvas://chat-state";

pub fn emit_state(ws: &str, node: &str, st: &chat::ChatState) {
    emit(
        EVENT_CHAT_STATE,
        json!({
            "workspaceId": ws,
            "nodeId": node,
            "activeThread": st.active(),
            "turnThread": st.thread,
            "turnOpen": st.turn_open,
            "turnInterrupted": st.turn_interrupted,
            "unread": st.unread,
            "unreadCount": st.unread_total(),
        }),
    );
}

/// Selects the thread the next message goes to (the UI's dots and
/// `canvas_chat_thread select`). Locked while a turn runs on another thread.
pub fn select(ws: &str, node_id: &str, thread: u8) -> Result<chat::ChatState, String> {
    let s = store();
    terminal_node(&s, ws, node_id)?;
    let locked = turn_active(&s, ws, node_id);
    let st = chat::select_thread(&s, ws, node_id, thread, locked)?;
    emit_state(ws, node_id, &st);
    Ok(st)
}

/// Marks a thread (or all) read.
pub fn mark_read(ws: &str, node_id: &str, thread: Option<u8>) -> Result<chat::ChatState, String> {
    let s = store();
    let st = chat::mark_read(&s, ws, node_id, thread)?;
    emit_state(ws, node_id, &st);
    Ok(st)
}

/// Esc on the chat face: sends ESC to the agent (as Esc in the terminal
/// would) and closes the open turn as interrupted. `None` when no turn was
/// open (the ESC is still sent).
pub fn interrupt(ws: &str, node_id: &str) -> Result<Option<ChatEntry>, String> {
    let s = store();
    let (_, node) = terminal_node(&s, ws, node_id)?;
    if let Some(tid) = node
        .data_as::<TerminalData>()
        .map(|d| d.terminal_id)
        .filter(|t| !t.is_empty())
    {
        let term = crate::pty::term();
        if term.info(&tid).is_some_and(|i| i.alive) {
            let _ = term.write(&tid, b"\x1b");
        }
    }
    close_turn(&s, ws, node_id, "Interrupted")
}

fn close_turn(
    s: &CanvasStore,
    ws: &str,
    node_id: &str,
    why: &str,
) -> Result<Option<ChatEntry>, String> {
    let e = chat::end_turn(s, ws, node_id, why)?;
    if let Some(e) = &e {
        emit_entry(ws, node_id, e);
        emit_state(ws, node_id, &chat::load_state(s, ws, node_id));
    }
    lock_turns().remove(node_id);
    Ok(e)
}

/// Palette search inside every chat of every workspace.
pub fn search(query: &str, limit: Option<usize>) -> Vec<chat::SearchHit> {
    let s = store();
    let ids: Vec<String> = s.workspaces().into_iter().map(|w| w.workspace.id).collect();
    chat::search(&s, &ids, query, limit.unwrap_or(50).clamp(1, 500))
}

// ---------------------------------------------------------------------------
// Turn watch: a turn the agent ended without `say` (it went back to idle
// after working, or exited) closes by itself instead of staying open.

#[derive(Default)]
struct Watched {
    ws: String,
    saw_busy: bool,
    idle_since: Option<std::time::Instant>,
}

static TURNS: OnceLock<Mutex<HashMap<String, Watched>>> = OnceLock::new();

fn lock_turns() -> std::sync::MutexGuard<'static, HashMap<String, Watched>> {
    TURNS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Idle this long after working, with the turn still open = ended silently.
const SILENT_END: Duration = Duration::from_secs(6);
/// Same, when "idle" is only guessed from the screen (plain commands, stubs):
/// a quiet agent is often still thinking, so wait much longer.
const SILENT_END_ESTIMATED: Duration = Duration::from_secs(90);

fn watch_turn(ws: &str, node_id: &str) {
    lock_turns().insert(
        node_id.to_string(),
        Watched {
            ws: ws.to_string(),
            ..Default::default()
        },
    );
}

fn start_turn_watch() {
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let _ = tauri::async_runtime::spawn_blocking(turn_watch_tick).await;
        }
    });
}

fn turn_watch_tick() {
    use omniget_core::core::canvas::state::NodeState;
    let ids: Vec<(String, String)> = lock_turns()
        .iter()
        .map(|(k, v)| (k.clone(), v.ws.clone()))
        .collect();
    if ids.is_empty() {
        return;
    }
    let s = store();
    for (node, ws) in ids {
        if !chat::load_state(&s, &ws, &node).turn_open {
            lock_turns().remove(&node);
            continue;
        }
        let fused = super::cords::host().node_state(&node);
        let exact = fused.as_ref().is_some_and(|f| {
            f.confidence >= omniget_core::core::canvas::state::Confidence::Reported
        });
        let state = fused.map(|f| f.state);
        let why = {
            let mut g = lock_turns();
            let Some(w) = g.get_mut(&node) else { continue };
            match state {
                None => Some("The terminal is gone"),
                Some(NodeState::Exited) => Some("The agent exited before replying"),
                Some(NodeState::Working | NodeState::Starting) => {
                    w.saw_busy = true;
                    w.idle_since = None;
                    None
                }
                Some(NodeState::Ready | NodeState::Done) if w.saw_busy => {
                    let since = *w.idle_since.get_or_insert_with(std::time::Instant::now);
                    let wait = if exact {
                        SILENT_END
                    } else {
                        SILENT_END_ESTIMATED
                    };
                    (since.elapsed() >= wait)
                        .then_some("The agent finished without replying in the chat")
                }
                _ => {
                    w.idle_since = None;
                    None
                }
            }
        };
        if let Some(why) = why {
            let _ = close_turn(&s, &ws, &node, why);
        }
    }
}

/// Emitted when entries leave a transcript (Delete Message, Clear Chat
/// History): `{workspaceId, nodeId, removed: [ids], thread?, cleared}`.
pub const EVENT_CHAT_REMOVED: &str = "canvas://chat-removed";

/// True while the agent is mid-turn on the chat face: a user message was
/// delivered and no final `say`, interrupt or silent end closed it. Agents
/// that never report "working" (plain commands) count too. A turn left open
/// by a previous app run whose process is gone does not.
pub fn turn_active(store: &CanvasStore, ws: &str, node_id: &str) -> bool {
    use omniget_core::core::canvas::state::NodeState;
    if !chat::load_state(store, ws, node_id).turn_open {
        return false;
    }
    let alive = store
        .locate_node(node_id)
        .and_then(|(w, floor)| {
            store
                .read(&w, &floor, |d| d.node(node_id).cloned())
                .ok()
                .flatten()
        })
        .and_then(|n| n.data_as::<TerminalData>())
        .and_then(|d| crate::pty::term().info(&d.terminal_id))
        .is_some_and(|i| i.alive);
    if !alive {
        return super::cords::host()
            .node_state(node_id)
            .is_some_and(|f| matches!(f.state, NodeState::Working | NodeState::Starting));
    }
    true
}

fn emit_removed(ws: &str, node: &str, r: &chat::Removed, thread: Option<u8>, cleared: bool) {
    emit(
        EVENT_CHAT_REMOVED,
        json!({
            "workspaceId": ws,
            "nodeId": node,
            "removed": r.removed,
            "thread": thread,
            "cleared": cleared,
        }),
    );
}

/// Delete Message: one entry (and its attachment files nobody else uses).
pub fn delete_entry(
    ws: &str,
    node_id: &str,
    entry_id: &str,
    thread: Option<u8>,
) -> Result<chat::Removed, String> {
    let s = store();
    terminal_node(&s, ws, node_id)?;
    let r = chat::delete_entry(&s, ws, node_id, entry_id, thread)?;
    emit_removed(ws, node_id, &r, thread, false);
    Ok(r)
}

/// Clear Chat History (every thread, or one): the app's transcript and
/// attachments only, never the provider's memory. Refused mid-turn.
pub fn wipe(ws: &str, node_id: &str, thread: Option<u8>) -> Result<chat::Removed, String> {
    let s = store();
    terminal_node(&s, ws, node_id)?;
    if turn_active(&s, ws, node_id) {
        return Err(
            "CHAT_TURN_ACTIVE: the agent is answering; interrupt it or wait before clearing".into(),
        );
    }
    let r = chat::wipe(&s, ws, node_id, thread)?;
    emit_removed(ws, node_id, &r, thread, true);
    emit_state(ws, node_id, &chat::load_state(&s, ws, node_id));
    Ok(r)
}

// ---------------------------------------------------------------------------
// RPC

fn text_of(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn parse<T: serde::de::DeserializeOwned + Default>(args: &Value) -> Result<T, String> {
    if args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(args.clone()).map_err(|e| format!("CANVAS_INVALID: {e}"))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

/// `chat.say {text?, progress?, file?, attach?: [path], thread?, cwd?}` →
/// `{id, thread, text}`.
/// `chat.recall {thread?, turns?, last?, from?, to?, all?, list?, attachment?}`
/// → `{thread, turns, text}` (`list` → `{threads, text}`; `attachment` →
/// `{attachment, text}`).
pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    match method {
        "chat.say" => Some(rpc_say(ctx.clone(), args).await),
        "chat.recall" => Some(rpc_recall(ctx.clone(), args).await),
        _ => None,
    }
}

async fn rpc_say(ctx: NodeCtx, args: Value) -> Result<Value, String> {
    let mut say: SayArgs = parse(&args)?;
    // The CLI may send the text positionally.
    if say.text.is_none() {
        say.text = args.get("message").and_then(text_of);
    }
    blocking(move || {
        let s = store();
        let (_, node) = terminal_node(&s, &ctx.workspace_id, &ctx.node_id)?;
        let agent = node.data_str("agent").map(str::to_string);
        let entry = chat::say(
            &s,
            &ctx.workspace_id,
            &ctx.node_id,
            say,
            node.title.clone(),
            agent,
        )?;
        emit_entry(&ctx.workspace_id, &ctx.node_id, &entry);
        emit_state(
            &ctx.workspace_id,
            &ctx.node_id,
            &chat::load_state(&s, &ctx.workspace_id, &ctx.node_id),
        );
        let text = if entry.kind == chat::EntryKind::Progress {
            "progress shown to the user".to_string()
        } else {
            format!(
                "sent to the user (thread {})",
                chat::thread_key(entry.thread)
            )
        };
        Ok(json!({ "id": entry.id, "thread": chat::thread_key(entry.thread), "text": text }))
    })
    .await
}

async fn rpc_recall(ctx: NodeCtx, args: Value) -> Result<Value, String> {
    blocking(move || {
        let s = store();
        let ws = ctx.workspace_id.clone();
        let node = ctx.node_id.clone();
        if let Some(id) = args.get("attachment").and_then(|v| v.as_str()) {
            let a = chat::attachment(&s, &ws, &node, id)?;
            let text = a.path.clone();
            return Ok(json!({ "attachment": a, "text": text }));
        }
        if args.get("list").and_then(|v| v.as_bool()).unwrap_or(false) {
            let all = chat::read_all(&s, &ws, &node)?;
            let st = chat::load_state(&s, &ws, &node);
            return Ok(json!({
                "threads": chat::threads_of(&all),
                "active": chat::thread_key(st.active()),
                "text": chat::render_thread_list(&all, st.active()),
            }));
        }
        let ra: RecallArgs = parse(&args)?;
        let (thread, turns) = chat::recall(&s, &ws, &node, &ra)?;
        Ok(json!({
            "thread": chat::thread_key(thread),
            "turns": turns,
            "text": chat::render_recall(thread, &turns),
        }))
    })
    .await
}
