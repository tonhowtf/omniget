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

pub fn select(ws: &str, node_id: &str, thread: u8) -> Result<chat::ChatState, String> {
    let s = store();
    terminal_node(&s, ws, node_id)?;
    let locked = turn_active(&s, ws, node_id);
    let st = chat::select_thread(&s, ws, node_id, thread, locked)?;
    emit_state(ws, node_id, &st);
    Ok(st)
}

pub fn mark_read(ws: &str, node_id: &str, thread: Option<u8>) -> Result<chat::ChatState, String> {
    let s = store();
    let st = chat::mark_read(&s, ws, node_id, thread)?;
    emit_state(ws, node_id, &st);
    Ok(st)
}

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

pub fn search(query: &str, limit: Option<usize>) -> Vec<chat::SearchHit> {
    let s = store();
    let ids: Vec<String> = s.workspaces().into_iter().map(|w| w.workspace.id).collect();
    chat::search(&s, &ids, query, limit.unwrap_or(50).clamp(1, 500))
}

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

const SILENT_END: Duration = Duration::from_secs(6);
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

pub const EVENT_CHAT_REMOVED: &str = "canvas://chat-removed";

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

pub async fn rpc(ctx: &NodeCtx, method: &str, args: Value) -> RpcResult {
    match method {
        "chat.say" => Some(rpc_say(ctx.clone(), args).await),
        "chat.recall" => Some(rpc_recall(ctx.clone(), args).await),
        _ => None,
    }
}

async fn rpc_say(ctx: NodeCtx, args: Value) -> Result<Value, String> {
    let mut say: SayArgs = parse(&args)?;
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
