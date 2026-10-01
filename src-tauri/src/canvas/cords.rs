use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use omniget_core::core::canvas::adapters::{
    self, claude as claude_adapter, native as native_adapter, pty,
};
use omniget_core::core::canvas::answer::{self, Answer, AnswerSource};
use omniget_core::core::canvas::envelope::{self, Envelope, Kind};
use omniget_core::core::canvas::graph::WaitGraph;
use omniget_core::core::canvas::limits::{self, Admission, CanvasLimits, Ledger};
use omniget_core::core::canvas::model::{
    CanvasNode, Condition, Connection, ConnectionKind, NodeKind,
};
use omniget_core::core::canvas::notes;
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::state::{self as node_state, Fused, Inputs, NodeState};
use omniget_core::core::canvas::store::{self, CanvasStore, StoreEvent};
use omniget_core::core::term::agent::{AgentSignal, AgentState, SignalSource};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::rpc::{NodeCtx, RpcResult};

pub const EVENT_NODE_STATE: &str = "canvas://node-state";
pub const EVENT_MESSAGE: &str = "canvas://message";
pub const EVENT_ASK: &str = "canvas://ask";

pub const WAIT_CAP_MS: u64 = 110_000;
const TICK: Duration = Duration::from_millis(250);
const COLLECT_GRACE_MS: u64 = 3_000;
const WAIT_GONE_MS: u64 = 5_000;
const VERIFY_TIMEOUT: Duration = Duration::from_secs(300);

fn now() -> u64 {
    crate::llm_manager::now_ms()
}

fn short_id(prefix: &str) -> String {
    format!(
        "{prefix}_{:011x}{}",
        now(),
        &uuid::Uuid::new_v4().simple().to_string()[..4]
    )
}

#[derive(Debug, Clone)]
pub struct ApiErr {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl ApiErr {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn body(&self) -> Value {
        json!({ "error": self.message, "code": self.code })
    }
}

fn not_found(what: impl std::fmt::Display) -> ApiErr {
    ApiErr::new(404, "not_found", what.to_string())
}

#[derive(Debug, Clone)]
enum AskState {
    Pending,
    Answered(Answer),
    Failed { error: String, message: String },
}

#[derive(Debug, Clone)]
struct Ask {
    id: String,
    from: String,
    to: String,
    ws: String,
    text: String,
    state: AskState,
    released_by: Option<String>,
    created_ms: u64,
    delivered_ms: Option<u64>,
    answered_ms: Option<u64>,
    deadline_ms: u64,
    waiters: u32,
    last_poll_ms: u64,
    collected: bool,
    reply_sent: bool,
    chain: String,
    hop: u32,
    asker_in_turn: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MsgState {
    Queued,
    Delivering,
    Delivered,
    Stuck,
    Failed,
}

#[derive(Debug, Clone)]
struct Msg {
    id: String,
    from: Option<String>,
    from_name: String,
    to: String,
    kind: Kind,
    text: String,
    ask_id: Option<String>,
    in_reply_to: Option<String>,
    note: Option<String>,
    chain: String,
    hop: u32,
    state: MsgState,
    created_ms: u64,
}

#[derive(Debug, Clone)]
struct Tracked {
    ws: String,
    floor: String,
    terminal: Option<String>,
    name: String,
    started_ms: u64,
    seen_alive: bool,
    exited: bool,
    last_state: AgentState,
    fused: Option<Fused>,
    native_busy: bool,
    native_failed: bool,
}

#[derive(Debug, Clone, Default)]
struct Turn {
    ask_ids: Vec<String>,
    baseline: String,
    started_ms: u64,
    echo: String,
    by_screen: bool,
    root_pid: Option<u32>,
    kids_before: HashSet<u32>,
}

const QUIET_TURN_END_MS: i64 = 3_500;

#[derive(Default)]
struct Inner {
    limits: CanvasLimits,
    asks: HashMap<String, Ask>,
    msgs: HashMap<String, Msg>,
    mail: HashMap<String, VecDeque<String>>,
    delivering: HashSet<String>,
    graph: WaitGraph,
    ledger: Ledger,
    inbound: HashMap<String, (String, u32)>,
    turns: HashMap<String, Turn>,
    confirming: HashSet<String>,
    submitted: HashSet<String>,
    tracked: HashMap<String, Tracked>,
    seen: HashSet<String>,
    stuck: HashSet<String>,
    permission: HashMap<String, String>,
    hook_turn_end_ms: HashMap<String, u64>,
    typing_ms: HashMap<String, u64>,
}

pub struct CanvasHost {
    inner: Mutex<Inner>,
    app: OnceLock<AppHandle>,
    changes: tokio::sync::watch::Sender<u64>,
    attention: OnceLock<Arc<crate::llm_attention::Attention>>,
}

static HOST: OnceLock<CanvasHost> = OnceLock::new();

pub fn host() -> &'static CanvasHost {
    HOST.get_or_init(|| CanvasHost {
        inner: Mutex::new(Inner::default()),
        app: OnceLock::new(),
        changes: tokio::sync::watch::channel(0).0,
        attention: OnceLock::new(),
    })
}

fn store() -> Arc<CanvasStore> {
    super::doc::host().store().clone()
}

fn term() -> Arc<omniget_core::core::term::session::TermManager> {
    crate::pty::term()
}

struct CanvasSink {
    app: AppHandle,
}

impl crate::llm_attention::NotifySink for CanvasSink {
    fn show(&self, title: &str, body: &str) {
        if !crate::canvas::prefs::get(&self.app).notifications.attention {
            return;
        }
        crate::mcp::canvas_ui::record_notification("attention", title, body, true);
        use tauri_plugin_notification::NotificationExt;
        let _ = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(if body.is_empty() { " " } else { body })
            .show();
    }

    fn focus(&self) -> crate::llm_attention::UiFocus {
        let mut focus = crate::llm_attention::reported_focus();
        focus.focused = self
            .app
            .get_webview_window("main")
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false);
        focus
    }
}

pub fn node_name(n: &CanvasNode) -> String {
    if let Some(t) = n.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    let short: String = n.id.chars().take(4).collect();
    let base = match n.kind {
        NodeKind::Terminal | NodeKind::Agent => {
            let a = n.data_str("agent").unwrap_or("terminal");
            let mut c = a.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => "Terminal".into(),
            }
        }
        NodeKind::Portal => n.data_str("name").unwrap_or("Portal").to_string(),
        ref k => {
            let s = k.as_str();
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => "Node".into(),
            }
        }
    };
    format!("{base} {short}")
}

fn agent_of(n: &CanvasNode) -> String {
    n.data_str("agent").unwrap_or("shell").to_string()
}

fn own_command(n: &CanvasNode) -> bool {
    let cmd = match n.data.get("command") {
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::String(s)) => !s.trim().is_empty(),
        _ => false,
    };
    cmd || n.data_str("presetId").is_some_and(|p| !p.trim().is_empty())
}

fn delivery_of_node(n: &CanvasNode) -> adapters::Delivery {
    adapters::delivery_for(&agent_of(n), own_command(n))
}

fn terminal_of(n: &CanvasNode) -> Option<String> {
    n.data_str("terminalId")
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

async fn wake(p: &Peer, kind: Kind) {
    let takes = match delivery_of_node(&p.node) {
        adapters::Delivery::Pty => true,
        adapters::Delivery::ShellOnly => kind == Kind::Raw,
        adapters::Delivery::Native => false,
    };
    if !takes || !super::launch::is_terminal(&p.node) {
        return;
    }
    let Some(tid) = terminal_of(&p.node) else {
        return;
    };
    if term().info(&tid).is_some() {
        return;
    }
    if let Err(e) = super::launch::ensure_running(&p.ws, &p.floor, &p.node.id).await {
        tracing::warn!("[canvas] could not start '{}': {e}", p.name);
    }
}

pub fn cord_allowed(a: &CanvasNode, b: &CanvasNode) -> Result<(), String> {
    if a.id == b.id {
        return Err("CORD_INVALID: a node cannot connect to itself".into());
    }
    let notey = |n: &CanvasNode| matches!(n.kind, NodeKind::Note | NodeKind::Stack);
    if (notey(a) && b.kind == NodeKind::Portal) || (notey(b) && a.kind == NodeKind::Portal) {
        return Err(format!(
            "CORD_INVALID: a {} cannot connect to a {} (notes connect to terminals and other notes; portals to terminals and other portals)",
            a.kind, b.kind
        ));
    }
    Ok(())
}

fn role_name(ws: &str, n: &CanvasNode) -> Option<String> {
    let id = n.data_str("roleId").filter(|r| !r.trim().is_empty())?;
    Some(
        omniget_core::core::canvas::roles::get(&store(), ws, id)
            .map(|(r, _)| r.name)
            .unwrap_or_else(|| id.to_string()),
    )
}

fn perms_of(c: &Connection) -> Vec<String> {
    if !c.perms.is_empty() {
        return c.perms.clone();
    }
    match c.kind {
        ConnectionKind::Note => vec!["note_rw".into()],
        ConnectionKind::Portal => vec!["portal_drive".into()],
        _ => vec!["ask".into(), "check".into()],
    }
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub node: CanvasNode,
    pub ws: String,
    pub floor: String,
    pub conn: Connection,
    pub perms: Vec<String>,
    pub name: String,
}

impl Peer {
    fn can(&self, perm: &str) -> bool {
        self.perms.iter().any(|p| p == perm)
    }
}

fn locate(node_id: &str) -> Result<(String, String, CanvasNode), ApiErr> {
    let s = store();
    let (ws, floor) = s
        .locate_node(node_id)
        .ok_or_else(|| not_found(format!("node {node_id} is not on any canvas")))?;
    let node = s
        .read(&ws, &floor, |d| d.node(node_id).cloned())
        .map_err(|e| ApiErr::new(500, "store", e))?
        .ok_or_else(|| not_found(format!("node {node_id}")))?;
    Ok((ws, floor, node))
}

fn peers_of(me: &str) -> Result<(String, String, CanvasNode, Vec<Peer>), ApiErr> {
    let (ws, floor, node) = locate(me)?;
    let s = store();
    let (conns, nodes): (Vec<Connection>, HashMap<String, CanvasNode>) = s
        .read(&ws, &floor, |d| {
            let conns: Vec<Connection> = d
                .connections
                .iter()
                .filter(|c| c.touches(me))
                .cloned()
                .collect();
            let nodes = d.nodes.iter().map(|n| (n.id.clone(), n.clone())).collect();
            (conns, nodes)
        })
        .map_err(|e| ApiErr::new(500, "store", e))?;
    let mut out: Vec<Peer> = Vec::new();
    for c in conns {
        let Some(other) = c.other_end(me) else {
            continue;
        };
        let Some(n) = nodes.get(other) else { continue };
        let (n, pws, pfloor) = if n.kind == NodeKind::Connector {
            match n.data_str("remoteNodeId").and_then(|r| locate(r).ok()) {
                Some((w, f, rn)) => (rn, w, f),
                None => continue,
            }
        } else {
            (n.clone(), ws.clone(), floor.clone())
        };
        if n.id == me || out.iter().any(|p| p.node.id == n.id) {
            continue;
        }
        let mut name = node_name(&n);
        if pws != ws {
            if let Ok(w) = s.workspace(&pws) {
                name = format!("{name} @ {}", w.name);
            }
        }
        out.push(Peer {
            perms: perms_of(&c),
            node: n,
            ws: pws,
            floor: pfloor,
            conn: c,
            name,
        });
    }
    Ok((ws, floor, node, out))
}

fn resolve_peer(peers: &[Peer], name: &str) -> Result<Peer, ApiErr> {
    let exact: Vec<&Peer> = peers.iter().filter(|p| p.name == name).collect();
    let hits = if exact.is_empty() {
        peers
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(name.trim()))
            .collect()
    } else {
        exact
    };
    let hits = if hits.is_empty() && name.trim().len() >= 4 {
        peers
            .iter()
            .filter(|p| p.node.id.starts_with(name.trim()))
            .collect()
    } else {
        hits
    };
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(not_found(format!(
            "no connected node named '{name}'. Run `omniget canvas list` for the exact names."
        ))),
        _ => {
            let ids: Vec<String> = hits
                .iter()
                .map(|p| p.node.id.chars().take(8).collect())
                .collect();
            Err(ApiErr::new(
                409,
                "ambiguous",
                format!(
                    "Multiple nodes named '{name}' (ids {}). Use one of the ids instead of the name, or ask the user to rename them to have unique names.",
                    ids.join(", ")
                ),
            ))
        }
    }
}

impl CanvasHost {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bump(&self) {
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        if let Some(app) = self.app.get() {
            let _ = app.emit(event, payload);
        }
    }

    fn app(&self) -> Option<&AppHandle> {
        self.app.get()
    }

    pub fn bind(&'static self, app: &AppHandle) -> &'static Self {
        if self.app.set(app.clone()).is_ok() {
            super::doc::host().bind(app);
            let att =
                crate::llm_attention::Attention::new(Arc::new(CanvasSink { app: app.clone() }));
            att.set_strings(crate::llm_attention::pushed_strings());
            let _ = self.attention.set(att);
            store().subscribe(move |ev| {
                if let StoreEvent::Patch(p) = ev {
                    for op in &p.ops {
                        if let Op::RemoveNode { id } = op {
                            host().on_deleted(id);
                        }
                    }
                }
            });
            if let Err(e) = install_plugin() {
                tracing::warn!("[canvas] plugin: {e}");
            }
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(TICK).await;
                    host().tick();
                }
            });
        }
        self
    }

    fn msg_event(&self, m: &Msg) {
        self.emit(
            EVENT_MESSAGE,
            json!({
                "id": m.id, "from": m.from, "to": m.to, "kind": m.kind.as_str(),
                "state": m.state, "chain": m.chain, "hop": m.hop, "askId": m.ask_id,
            }),
        );
    }

    fn ask_event(&self, a: &Ask) {
        self.emit(
            EVENT_ASK,
            json!({ "id": a.id, "from": a.from, "to": a.to, "state": ask_state_str(a) }),
        );
    }

    pub fn me(&self, node_id: &str) -> Result<Value, ApiErr> {
        let (ws, floor, node) = locate(node_id)?;
        Ok(json!({
            "node": node.id,
            "name": node_name(&node),
            "role": role_name(&ws, &node),
            "roleId": node.data_str("roleId"),
            "agent": node.data_str("agent"),
            "workspace": ws,
            "floor": floor,
        }))
    }

    pub fn peers(&self, node_id: &str) -> Result<Value, ApiErr> {
        let (ws, _floor, me, peers) = peers_of(node_id)?;
        let mut list = Vec::new();
        for p in &peers {
            let mut j = self.peer_json(p, &peers, true);
            let dir = if p.conn.to == node_id && p.conn.from != node_id {
                "in"
            } else {
                "out"
            };
            if let Some(m) = j.as_object_mut() {
                m.insert("direction".into(), json!(dir));
                if matches!(p.node.kind, NodeKind::Terminal | NodeKind::Agent) {
                    m.insert("canAskYou".into(), json!(p.can("ask")));
                }
            }
            list.push(j);
        }
        Ok(json!({
            "you": { "name": node_name(&me), "role": role_name(&ws, &me), "roleId": me.data_str("roleId"), "node": me.id, "workspace": ws },
            "peers": list,
        }))
    }

    fn peer_json(&self, p: &Peer, all: &[Peer], nested: bool) -> Value {
        let n = &p.node;
        match n.kind {
            NodeKind::Terminal | NodeKind::Agent => {
                let state = self.fused_of(&n.id, &p.ws, &p.floor, n);
                json!({
                    "name": p.name, "kind": "agent", "agent": agent_of(n),
                    "role": role_name(&p.ws, n), "roleId": n.data_str("roleId"),
                    "state": state.state.as_str(), "confidence": state.confidence, "perms": p.perms,
                })
            }
            NodeKind::Note => {
                let mut children = Vec::new();
                if nested {
                    let mut seen: HashSet<String> = all
                        .iter()
                        .filter(|o| o.node.kind == NodeKind::Note)
                        .map(|o| o.node.id.clone())
                        .collect();
                    seen.insert(p.conn.from.clone());
                    seen.insert(p.conn.to.clone());
                    children = note_children_json(&p.ws, &p.floor, &n.id, &mut seen);
                }
                let locked = n
                    .data
                    .get("isContentLocked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let perms: Vec<String> = if locked {
                    vec!["note_ro".into()]
                } else {
                    p.perms.clone()
                };
                json!({
                    "name": p.name, "kind": "note", "locked": locked,
                    "perms": perms, "children": children,
                })
            }
            NodeKind::Stack => {
                let mut children = Vec::new();
                if nested {
                    for (nid, nn) in stack_notes(&p.ws, &p.floor, n) {
                        let _ = nid;
                        children.push(json!({
                            "name": node_name(&nn), "kind": "note",
                            "locked": nn.data.get("isContentLocked").and_then(|v| v.as_bool()).unwrap_or(false),
                        }));
                    }
                }
                json!({ "name": p.name, "kind": "stack", "children": children, "perms": p.perms })
            }
            NodeKind::Portal => json!({
                "name": p.name, "kind": "portal", "url": n.data_str("url"),
                "portalKind": n.data_str("portalKind"), "perms": p.perms,
            }),
            ref k => json!({ "name": p.name, "kind": k.as_str() }),
        }
    }

    pub fn changes_rx(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn node_state(&self, node_id: &str) -> Option<Fused> {
        let (ws, floor, n) = locate(node_id).ok()?;
        Some(self.fused_of(&n.id, &ws, &floor, &n))
    }

    fn fused_of(&self, node_id: &str, ws: &str, floor: &str, n: &CanvasNode) -> Fused {
        if let Some(f) = self
            .lock()
            .tracked
            .get(node_id)
            .and_then(|t| t.fused.clone())
        {
            return f;
        }
        let _ = (ws, floor);
        let now_ms = now() as i64;
        let (signal, alive) = match terminal_of(n).and_then(|t| term().info(&t)) {
            Some(info) => (Some(info.agent.clone()), info.alive),
            None => (None, agent_of(n) == "native"),
        };
        node_state::fuse(
            &Inputs {
                signal,
                alive,
                started_ms: now_ms - node_state::STARTING_GRACE_MS,
                ..Default::default()
            },
            now_ms,
        )
    }

    pub fn terminal_env(
        &'static self,
        app: &AppHandle,
        ws: &str,
        floor: &str,
        node_id: &str,
    ) -> Result<NodeEnv, String> {
        let node = store()
            .read(ws, floor, |d| d.node(node_id).cloned())?
            .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))?;
        self.terminal_env_for(app, ws, floor, &node)
    }

    pub fn terminal_env_for(
        &'static self,
        app: &AppHandle,
        ws: &str,
        floor: &str,
        node: &CanvasNode,
    ) -> Result<NodeEnv, String> {
        self.bind(app);
        let node_id = node.id.as_str();
        let name = node_name(node);
        let agent = agent_of(node);
        let terminal = terminal_of(node).unwrap_or_else(|| node_id.to_string());
        let token = {
            let jobs = crate::jobs::get(app)?;
            let db = jobs.db();
            crate::bridge_tokens::mint_canvas(&db, node_id, &name, now())?.token
        };
        let bridge_port = crate::local_bridge::ensure_running_for_canvas(app)?;
        let data = omniget_core::core::paths::app_data_dir().ok_or("no app data dir")?;
        let bin = data.join("bin");
        let cli = bin.join(if cfg!(windows) {
            "omniget.exe"
        } else {
            "omniget"
        });
        let plugin = plugin_dir().ok_or("no app data dir")?;
        let _ = install_plugin();
        if let Err(e) = install_skill_root(app, &agent) {
            tracing::warn!("[canvas] skill folder for {agent}: {e}");
        }
        let mut env = HashMap::new();
        env.insert("OMNIGET_CANVAS".into(), "1".into());
        env.insert("OMNIGET_CANVAS_NODE".into(), node_id.to_string());
        env.insert("OMNIGET_CANVAS_NAME".into(), name.clone());
        env.insert("OMNIGET_CANVAS_WORKSPACE".into(), ws.to_string());
        env.insert("OMNIGET_CANVAS_FLOOR".into(), floor.to_string());
        env.insert("OMNIGET_CANVAS_TOKEN".into(), token);
        env.insert(
            "OMNIGET_BRIDGE_URL".into(),
            format!("http://127.0.0.1:{bridge_port}"),
        );
        env.insert("OMNIGET_CLI".into(), cli.to_string_lossy().into_owned());
        env.insert("OMNIGET_TERMINAL_ID".into(), terminal.clone());
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut parts = vec![bin.clone()];
        parts.extend(std::env::split_paths(&path).filter(|p| p != &bin));
        if let Ok(joined) = std::env::join_paths(parts) {
            env.insert("PATH".into(), joined.to_string_lossy().into_owned());
        }
        let args = adapters::launch_args(&agent, &cli, &plugin);
        {
            let mut g = self.lock();
            g.tracked.insert(
                node_id.to_string(),
                Tracked {
                    ws: ws.to_string(),
                    floor: floor.to_string(),
                    terminal: Some(terminal),
                    name,
                    started_ms: now(),
                    seen_alive: false,
                    exited: false,
                    last_state: AgentState::Unknown,
                    fused: None,
                    native_busy: false,
                    native_failed: false,
                },
            );
            g.stuck.remove(node_id);
        }
        let env_remove = omniget_core::core::term::session::inherited_agent_session_vars();
        Ok(NodeEnv {
            env,
            args,
            env_remove,
        })
    }

    pub async fn submit(&'static self, me: &str, body: Value) -> Result<Value, ApiErr> {
        let request_id = body["request_id"].as_str().unwrap_or("").to_string();
        let wait = body["wait"].as_bool().unwrap_or(true);
        if let Some(ids) = self.lock().ledger.replay(me, &request_id) {
            return Ok(self.asks_json(&ids, "replayed"));
        }
        let (_ws, _floor, me_node, peers) = peers_of(me)?;
        let me_name = node_name(&me_node);
        if let Some(raw) = body["raw"].as_str() {
            let to = body["to"].as_str().unwrap_or("");
            let p = resolve_peer(&peers, to)?;
            return self.raw(me, &p, raw, &request_id).await;
        }
        let mut items: Vec<(String, String)> = Vec::new();
        if let Some(batch) = body["batch"].as_object() {
            let mut seen = HashSet::new();
            for (k, v) in batch {
                if !seen.insert(k.clone()) {
                    return Err(ApiErr::new(400, "usage", "batch targets must be distinct"));
                }
                items.push((k.clone(), v.as_str().unwrap_or("").to_string()));
            }
        } else {
            items.push((
                body["to"].as_str().unwrap_or("").to_string(),
                body["text"].as_str().unwrap_or("").to_string(),
            ));
        }
        if items.iter().any(|(_, t)| t.trim().is_empty()) {
            return Err(ApiErr::new(400, "usage", "empty text"));
        }
        let mut targets = Vec::new();
        for (name, text) in items {
            let p = resolve_peer(&peers, &name)?;
            if !p.can("ask") {
                return Err(ApiErr::new(
                    403,
                    "denied",
                    format!("the cord to '{}' does not allow ask", p.name),
                ));
            }
            wake(&p, Kind::Ask).await;
            self.check_target(&p, Kind::Ask)?;
            targets.push((p, text));
        }
        let mut ids = Vec::new();
        {
            let mut g = self.lock();
            let open = g
                .asks
                .values()
                .filter(|a| {
                    a.from == me && matches!(a.state, AskState::Pending) && a.released_by.is_none()
                })
                .count() as u32;
            let lim = g.limits;
            if open + targets.len() as u32 > lim.max_pending_asks_per_node {
                return Err(ApiErr::new(
                    429,
                    limits::Refusal::Pending.code(),
                    limits::Refusal::Pending.message(&lim),
                ));
            }
            for (p, text) in &targets {
                let id = self.admit_locked(
                    &mut g,
                    Some((me, &me_name)),
                    p,
                    Kind::Ask,
                    text,
                    None,
                    None,
                    wait,
                )?;
                ids.push(id);
            }
            g.ledger.remember(me, &request_id, ids.clone(), now());
        }
        self.bump();
        Ok(self.asks_json(&ids, "queued"))
    }

    fn asks_json(&self, ids: &[String], delivery: &str) -> Value {
        let g = self.lock();
        let asks: Vec<Value> = ids
            .iter()
            .map(|id| {
                let to = g
                    .asks
                    .get(id)
                    .and_then(|a| {
                        g.tracked
                            .get(&a.to)
                            .map(|t| t.name.clone())
                            .or_else(|| locate(&a.to).ok().map(|x| node_name(&x.2)))
                    })
                    .unwrap_or_default();
                let d = match g.asks.get(id).map(|a| &a.state) {
                    Some(AskState::Answered(_)) => "answered",
                    _ => delivery,
                };
                json!({ "id": id, "to": to, "delivery": d })
            })
            .collect();
        json!({ "asks": asks })
    }

    fn check_target(&self, p: &Peer, kind: Kind) -> Result<(), ApiErr> {
        if !matches!(p.node.kind, NodeKind::Terminal | NodeKind::Agent) {
            return Err(ApiErr::new(
                409,
                "not_an_agent",
                format!(
                    "'{}' is a {}, not an agent (use `omniget canvas note`/`portal`)",
                    p.name, p.node.kind
                ),
            ));
        }
        let delivery = delivery_of_node(&p.node);
        if delivery == adapters::Delivery::ShellOnly && kind != Kind::Raw {
            return Err(ApiErr::new(
                409,
                "target_is_shell",
                format!("'{}' is a plain shell: it takes only `--raw` keys", p.name),
            ));
        }
        if delivery == adapters::Delivery::Pty {
            let alive = terminal_of(&p.node)
                .and_then(|t| term().info(&t))
                .is_some_and(|i| i.alive);
            if !alive {
                return Err(ApiErr::new(
                    409,
                    "target_exited",
                    format!(
                        "'{}' exists but its process is not running (it may have exited, or not been opened since the app started). Ask the user to open it on the canvas.",
                        p.name
                    ),
                ));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn admit_locked(
        &self,
        g: &mut Inner,
        from: Option<(&str, &str)>,
        p: &Peer,
        kind: Kind,
        text: &str,
        in_reply_to: Option<&str>,
        note: Option<&str>,
        wait: bool,
    ) -> Result<String, ApiErr> {
        let to = p.node.id.clone();
        let t = now();
        let (chain, hop) = match from.and_then(|(f, _)| g.inbound.get(f).cloned()) {
            Some((c, h)) => (c, h + 1),
            None => {
                let c = limits::new_chain_id();
                if let Some((f, _)) = from {
                    g.inbound.insert(f.to_string(), (c.clone(), 0));
                }
                (c, 1)
            }
        };
        if let Some((f, _)) = from {
            if kind != Kind::Reply {
                let lim = g.limits;
                let adm = Admission {
                    from: f,
                    to: &to,
                    chain: &chain,
                    hop,
                    open_asks: 0,
                    new_asks: 0,
                };
                if let Err(r) = g.ledger.admit(&lim, &adm, t) {
                    if r == limits::Refusal::PingPong {
                        let name = g.tracked.get(f).map(|x| x.name.clone()).unwrap_or_default();
                        self.needs_you(f, &name, "Two agents are messaging back and forth; the chain is paused until you step in.");
                    }
                    return Err(ApiErr::new(429, r.code(), r.message(&lim)));
                }
            }
        }
        let from_id = from.map(|(f, _)| f.to_string());
        let from_name = from
            .map(|(_, n)| n.to_string())
            .unwrap_or_else(|| "OmniGet".into());

        if let Some(f) = from_id.as_deref() {
            let late_reply = kind == Kind::Reply
                && in_reply_to.is_some_and(|r| g.graph.ask_back(f, &to).is_some_and(|w| w != r));
            if let Some(waiting) = g.graph.ask_back(f, &to).filter(|_| !late_reply) {
                let waiter_live = g.asks.get(&waiting).is_some_and(|a| a.asker_in_turn);
                let back_id = (kind == Kind::Ask && waiter_live).then(|| short_id("ask"));
                let released = if kind == Kind::Ask {
                    let how = match &back_id {
                        Some(id) => format!(
                            "Answer it by finishing your turn, or with `omniget canvas reply {id} \"...\"`."
                        ),
                        None => "It also arrives as a new message.".to_string(),
                    };
                    format!(
                        "[releasing this ask to prevent a deadlock: '{from_name}' asked you back while you were waiting on it]\n{text}\n\n[{how}]"
                    )
                } else {
                    text.to_string()
                };
                self.resolve_locked(g, &waiting, Answer::new(released, AnswerSource::Reply));
                if let Some(id) = back_id {
                    let ask = Ask {
                        id: id.clone(),
                        from: f.to_string(),
                        to: to.clone(),
                        ws: p.ws.clone(),
                        text: text.to_string(),
                        state: AskState::Pending,
                        released_by: None,
                        created_ms: t,
                        delivered_ms: Some(t),
                        answered_ms: None,
                        deadline_ms: t + g.limits.max_ask_ms,
                        waiters: 0,
                        last_poll_ms: t,
                        collected: false,
                        reply_sent: false,
                        chain: chain.clone(),
                        hop,
                        asker_in_turn: in_turn(g, f),
                    };
                    let baseline = terminal_of(&p.node)
                        .and_then(|tid| term().screen_text(&tid, 200).ok())
                        .map(|s| s.text)
                        .unwrap_or_default();
                    let turn = g.turns.entry(to.clone()).or_default();
                    if turn.started_ms == 0 {
                        turn.started_ms = t;
                        turn.baseline = baseline;
                    }
                    turn.ask_ids.push(id.clone());
                    if wait {
                        g.graph.add(&id, f, &to);
                    }
                    self.ask_event(&ask);
                    g.asks.insert(id.clone(), ask);
                    return Ok(id);
                }
                if kind != Kind::Ask {
                    return Ok(waiting);
                }
            }
            if let Some(edge) = g.graph.cycle_release(f, &to) {
                g.graph.remove(&edge.ask_id);
                if let Some(a) = g.asks.get_mut(&edge.ask_id) {
                    a.released_by = Some(from_name.clone());
                    let a = a.clone();
                    self.ask_event(&a);
                }
                tracing::info!(
                    "[canvas] ring-break: released {} ({} -> {})",
                    edge.ask_id,
                    edge.from,
                    edge.to
                );
            }
        }

        let msg_id = short_id("msg");
        let ask_id = (kind == Kind::Ask).then(|| short_id("ask"));
        let m = Msg {
            id: msg_id.clone(),
            from: from_id.clone(),
            from_name,
            to: to.clone(),
            kind,
            text: text.to_string(),
            ask_id: ask_id.clone(),
            in_reply_to: in_reply_to.map(str::to_string),
            note: note.map(str::to_string),
            chain: chain.clone(),
            hop,
            state: MsgState::Queued,
            created_ms: t,
        };
        if let (Some(id), Some(f)) = (&ask_id, &from_id) {
            let ask = Ask {
                id: id.clone(),
                from: f.clone(),
                to: to.clone(),
                ws: p.ws.clone(),
                text: text.to_string(),
                state: AskState::Pending,
                released_by: None,
                created_ms: t,
                delivered_ms: None,
                answered_ms: None,
                deadline_ms: t + g.limits.max_ask_ms,
                waiters: 0,
                last_poll_ms: t,
                collected: false,
                reply_sent: false,
                chain,
                hop,
                asker_in_turn: in_turn(g, f),
            };
            if wait {
                g.graph.add(id, f, &to);
            }
            self.ask_event(&ask);
            g.asks.insert(id.clone(), ask);
        }
        if agent_of(&p.node) == "native" && !g.tracked.contains_key(&to) {
            g.tracked.insert(
                to.clone(),
                Tracked {
                    ws: p.ws.clone(),
                    floor: p.floor.clone(),
                    terminal: None,
                    name: p.name.clone(),
                    started_ms: t,
                    seen_alive: true,
                    exited: false,
                    last_state: AgentState::Idle,
                    fused: None,
                    native_busy: false,
                    native_failed: false,
                },
            );
        }
        self.msg_event(&m);
        g.mail.entry(to).or_default().push_back(msg_id.clone());
        g.msgs.insert(msg_id.clone(), m);
        Ok(ask_id.unwrap_or(msg_id))
    }

    fn resolve_locked(&self, g: &mut Inner, ask_id: &str, ans: Answer) {
        let Some(a) = g.asks.get_mut(ask_id) else {
            return;
        };
        if !matches!(a.state, AskState::Pending) {
            return;
        }
        let dir = store().inbox_dir(&a.ws, &a.from);
        a.state = AskState::Answered(ans.capped(&dir, ask_id));
        a.answered_ms = Some(now());
        let a = a.clone();
        g.graph.remove(ask_id);
        self.ask_event(&a);
        release_asker(g, &a);
        self.bump();
    }

    fn fail_locked(&self, g: &mut Inner, ask_id: &str, error: &str, message: &str) {
        let Some(a) = g.asks.get_mut(ask_id) else {
            return;
        };
        if !matches!(a.state, AskState::Pending) {
            return;
        }
        a.state = AskState::Failed {
            error: error.into(),
            message: message.into(),
        };
        let a = a.clone();
        g.graph.remove(ask_id);
        self.ask_event(&a);
        release_asker(g, &a);
        self.bump();
    }

    async fn raw(
        &'static self,
        me: &str,
        p: &Peer,
        raw: &str,
        request_id: &str,
    ) -> Result<Value, ApiErr> {
        if !p.can("raw") {
            return Err(ApiErr::new(
                403,
                "denied",
                format!(
                    "the cord to '{}' does not allow raw keys (the user can turn it on)",
                    p.name
                ),
            ));
        }
        wake(p, Kind::Raw).await;
        self.check_target(p, Kind::Raw)?;
        let tid =
            terminal_of(&p.node).ok_or_else(|| ApiErr::new(409, "target_exited", "no terminal"))?;
        tracing::warn!(
            "[canvas] raw keys from {me} into {} ({} bytes)",
            p.name,
            raw.len()
        );
        let m = term();
        m.write(&tid, raw.as_bytes())
            .map_err(|e| ApiErr::new(409, "target_exited", e))?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let text = m.screen_text(&tid, 60).map(|s| s.text).unwrap_or_default();
        let id = short_id("ask");
        let t = now();
        {
            let mut g = self.lock();
            let ask = Ask {
                id: id.clone(),
                from: me.to_string(),
                to: p.node.id.clone(),
                ws: p.ws.clone(),
                text: raw.to_string(),
                state: AskState::Answered(Answer::new(text, AnswerSource::Screen)),
                released_by: None,
                created_ms: t,
                delivered_ms: Some(t),
                answered_ms: Some(t),
                deadline_ms: t,
                waiters: 0,
                last_poll_ms: t,
                collected: false,
                reply_sent: true,
                chain: String::new(),
                hop: 0,
                asker_in_turn: false,
            };
            g.asks.insert(id.clone(), ask);
            g.ledger.remember(me, request_id, vec![id.clone()], t);
        }
        Ok(json!({ "asks": [{ "id": id, "to": p.name, "delivery": "answered" }] }))
    }

    pub async fn wait(&'static self, me: &str, ask_id: &str, timeout_ms: u64) -> (u16, Value) {
        let timeout = Duration::from_millis(timeout_ms.min(WAIT_CAP_MS));
        let mut rx = self.changes.subscribe();
        {
            let mut g = self.lock();
            let Some(a) = g.asks.get(ask_id).cloned() else {
                return (404, json!({ "error": format!("no ask {ask_id}") }));
            };
            if a.from != me {
                return (403, json!({ "error": "this ask belongs to another node" }));
            }
            if let Some(a) = g.asks.get_mut(ask_id) {
                a.waiters += 1;
                a.last_poll_ms = now();
            }
            if matches!(a.state, AskState::Pending)
                && a.released_by.is_none()
                && !g.graph.contains(ask_id)
            {
                g.graph.add(ask_id, &a.from, &a.to);
            }
            if let Some(t) = g.tracked.get(me).and_then(|t| t.terminal.clone()) {
                if matches!(a.state, AskState::Pending) {
                    let who = g
                        .tracked
                        .get(&a.to)
                        .map(|t| t.name.clone())
                        .unwrap_or_default();
                    let _ = term().set_agent_state(
                        &t,
                        AgentState::WaitingPeer,
                        SignalSource::Hook,
                        Some(format!("waiting for {who}")),
                    );
                }
            }
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let out = loop {
            if let Some(v) = self.final_view(ask_id) {
                break (200, v);
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) | Err(_) => {
                    if let Some(v) = self.final_view(ask_id) {
                        break (200, v);
                    }
                    break (408, self.pending_view(ask_id));
                }
            }
        };
        let mut g = self.lock();
        if let Some(a) = g.asks.get_mut(ask_id) {
            a.waiters = a.waiters.saturating_sub(1);
            a.last_poll_ms = now();
            if out.0 == 200 {
                a.collected = true;
            }
        }
        out
    }

    fn final_view(&self, ask_id: &str) -> Option<Value> {
        let g = self.lock();
        let a = g.asks.get(ask_id)?;
        match &a.state {
            AskState::Answered(ans) => {
                let mut v = serde_json::to_value(ans).unwrap_or(Value::Null);
                if let Some(m) = v.as_object_mut() {
                    m.insert("status".into(), json!("answered"));
                    if let Some(at) = a.answered_ms {
                        m.insert("duration_ms".into(), json!(at.saturating_sub(a.created_ms)));
                    }
                }
                Some(json!({ "state": "answered", "answer": v }))
            }
            AskState::Failed { error, message } => {
                Some(json!({ "state": "failed", "error": error, "message": message }))
            }
            AskState::Pending => a
                .released_by
                .as_ref()
                .map(|by| json!({ "state": "released", "reason": "deadlock", "by": by })),
        }
    }

    fn pending_view(&self, ask_id: &str) -> Value {
        let g = self.lock();
        let Some(a) = g.asks.get(ask_id) else {
            return json!({ "state": "pending" });
        };
        let target = g
            .tracked
            .get(&a.to)
            .and_then(|t| t.fused.as_ref())
            .map(|f| f.state.as_str())
            .unwrap_or("unknown");
        json!({
            "state": "pending",
            "target_state": target,
            "delivered_ms": a.delivered_ms.map(|d| now().saturating_sub(d)),
        })
    }

    pub fn reply(
        &'static self,
        me: &str,
        ask_id: Option<&str>,
        text: &str,
        file: Option<&str>,
    ) -> Result<Value, ApiErr> {
        if text.trim().is_empty() {
            return Err(ApiErr::new(400, "usage", "empty reply"));
        }
        let mut g = self.lock();
        let id = match ask_id.filter(|s| !s.is_empty()) {
            Some(id) => {
                let a = g
                    .asks
                    .get(id)
                    .ok_or_else(|| not_found(format!("no ask {id}")))?;
                if a.to != me {
                    return Err(ApiErr::new(403, "denied", "that ask was not sent to you"));
                }
                if !matches!(a.state, AskState::Pending) {
                    return Err(ApiErr::new(
                        409,
                        "already_answered",
                        format!("{id} is already answered"),
                    ));
                }
                id.to_string()
            }
            None => {
                let chain = g.inbound.get(me).map(|(c, _)| c.clone());
                let mut open: Vec<&Ask> = g
                    .asks
                    .values()
                    .filter(|a| a.to == me && matches!(a.state, AskState::Pending))
                    .collect();
                open.sort_by(|a, b| a.id.cmp(&b.id));
                let pick = open
                    .iter()
                    .find(|a| Some(&a.chain) == chain.as_ref())
                    .or(open.first())
                    .ok_or_else(|| not_found("nobody is waiting for an answer from you"))?;
                pick.id.clone()
            }
        };
        let mut ans = Answer::new(text.to_string(), AnswerSource::Reply);
        ans.file = file.map(str::to_string);
        self.resolve_locked(&mut g, &id, ans);
        if let Some(t) = g.turns.get_mut(me) {
            t.ask_ids.retain(|a| a != &id);
        }
        Ok(json!({ "resolved": id }))
    }

    pub fn screen(&self, me: &str, name: &str, lines: usize) -> Result<Value, ApiErr> {
        let (_ws, _f, _me, peers) = peers_of(me)?;
        let p = resolve_peer(&peers, name)?;
        if !p.can("check") {
            return Err(ApiErr::new(
                403,
                "denied",
                format!("the cord to '{}' does not allow check", p.name),
            ));
        }
        let tid = terminal_of(&p.node).ok_or_else(|| {
            ApiErr::new(
                409,
                "not_a_terminal",
                format!("'{}' has no terminal", p.name),
            )
        })?;
        let snap = term()
            .screen_text(&tid, lines.clamp(1, 500))
            .map_err(|e| ApiErr::new(409, "target_exited", e))?;
        let state = self.fused_of(&p.node.id, &p.ws, &p.floor, &p.node);
        Ok(json!({
            "text": snap.text,
            "state": state.state.as_str(),
            "cursor_row": snap.cursor_row,
            "changed_ms": (now() as i64 - snap.changed_ms).max(0),
            "alt_screen": snap.alt_screen,
        }))
    }

    pub async fn hook(&'static self, me: &str, payload: Value) -> Value {
        let event = payload["hook_event_name"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let tid = {
            let g = self.lock();
            g.tracked.get(me).and_then(|t| t.terminal.clone())
        }
        .or_else(|| locate(me).ok().and_then(|(_, _, n)| terminal_of(&n)));
        if let Some(t) = &tid {
            let _ = term().hook(t, &event, &payload);
        }
        match event.as_str() {
            "UserPromptSubmit" => {
                let mut g = self.lock();
                if g.confirming.contains(me) {
                    g.submitted.insert(me.to_string());
                } else {
                    self.user_turn_locked(&mut g, me);
                }
            }
            "Stop" | "StopFailure" => {
                self.lock().hook_turn_end_ms.insert(me.to_string(), now());
                self.turn_end(me, Some(payload.clone()), event == "StopFailure");
            }
            "Notification" => {
                let kind = payload["notification_type"].as_str().unwrap_or("");
                if kind == "permission_prompt" || kind == "elicitation_dialog" {
                    let name = self.name_of(me);
                    let msg = payload["message"]
                        .as_str()
                        .unwrap_or("needs your answer")
                        .to_string();
                    self.needs_you(me, &name, &msg);
                }
            }
            "PermissionRequest" => return self.permission(me, &payload).await,
            _ => {}
        }
        json!({ "ok": true })
    }

    fn name_of(&self, node: &str) -> String {
        if let Some(t) = self.lock().tracked.get(node) {
            return t.name.clone();
        }
        locate(node)
            .map(|x| node_name(&x.2))
            .unwrap_or_else(|_| node.to_string())
    }

    fn user_turn_locked(&self, g: &mut Inner, node: &str) {
        if let Some((old, _)) = g.inbound.remove(node) {
            g.ledger.resume(&old);
        }
        g.inbound
            .insert(node.to_string(), (limits::new_chain_id(), 0));
        g.seen.insert(node.to_string());
    }

    async fn permission(&'static self, me: &str, v: &Value) -> Value {
        let Some(app) = self.app().cloned() else {
            return json!({ "decision": "ask" });
        };
        let tool_name = v["tool_name"].as_str().unwrap_or("").to_string();
        let tool = v["omniget"]["tool"]
            .as_str()
            .unwrap_or("claude")
            .to_string();
        let wait_ms = v["omniget"]["wait_ms"]
            .as_u64()
            .unwrap_or(100_000)
            .min(omniget_core::core::agentkit::convert::hook_shim::MAX_WAIT_SECS * 1000);
        let name = self.name_of(me);
        let agent = format!("canvas:{me}");
        let call_id = format!("{agent}:{}", uuid::Uuid::new_v4().simple());
        self.lock()
            .permission
            .insert(me.to_string(), call_id.clone());
        self.needs_you(me, &name, &format!("Allow {tool_name}?"));
        let state = app.state::<crate::AppState>();
        let broker = state.llm.broker();
        let label = if tool_name.is_empty() {
            format!("{name} ({tool})")
        } else {
            format!("{tool_name} ({name})")
        };
        let input = v.get("tool_input").cloned().unwrap_or(Value::Null);
        let preview: String = ["command", "file_path", "url", "pattern", "prompt"]
            .iter()
            .find_map(|k| input.get(*k).and_then(|x| x.as_str()).map(str::to_string))
            .unwrap_or_else(|| input.to_string())
            .chars()
            .take(400)
            .collect();
        let decision = if state.llm.bus().receiver_count() == 0 {
            json!({ "decision": "ask" })
        } else {
            let ans = tokio::time::timeout(
                Duration::from_millis(wait_ms),
                broker.ask_user(&agent, &agent, &call_id, &label, &preview),
            )
            .await;
            use omniget_core::core::llm::broker::Answer as B;
            match ans {
                Ok(B::Deny) => json!({ "decision": "deny", "reason": "Denied in OmniGet" }),
                Ok(B::Once) | Ok(B::Always) => json!({ "decision": "allow" }),
                Err(_) => {
                    broker.answer_with(&call_id, B::Deny);
                    let _ = app.emit(
                        "omni://tool-resolved",
                        json!({ "request_id": agent, "tool_call_id": call_id, "allow": false, "expired": true }),
                    );
                    json!({ "decision": "ask" })
                }
            }
        };
        self.lock().permission.remove(me);
        decision
    }

    fn turn_done_notice(&self, node: &str, failed: bool) {
        let Some(att) = self.attention.get().cloned() else {
            return;
        };
        let tid = self
            .lock()
            .tracked
            .get(node)
            .and_then(|t| t.terminal.clone());
        let m = term();
        let shell = tid.as_deref().and_then(|t| m.agent_kind(t))
            == Some(omniget_core::core::term::AgentKind::Shell);
        if shell {
            let st = store();
            let watched = st
                .locate_node(node)
                .and_then(|(ws, fl)| {
                    st.read(&ws, &fl, |d| {
                        d.node(node)
                            .and_then(|n| n.data.get("monitorActivity")?.as_bool())
                    })
                    .ok()
                    .flatten()
                })
                .unwrap_or(false);
            if !watched {
                return;
            }
        }
        let name = self.name_of(node);
        let target = crate::llm_attention::AttentionTarget::canvas_node(node, &name);
        let msg = tid
            .as_deref()
            .and_then(|t| m.agent_state(t))
            .and_then(|s| s.message)
            .unwrap_or_default();
        att.job_ended(&target, !failed, &msg);
    }

    fn needs_you(&self, node: &str, name: &str, what: &str) {
        let Some(att) = self.attention.get().cloned() else {
            return;
        };
        let node_id = node.to_string();
        let target = crate::llm_attention::AttentionTarget::canvas_node(node, name);
        let still: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
            let g = host().lock();
            g.permission.contains_key(&node_id)
                || g.stuck.contains(&node_id)
                || g.tracked
                    .get(&node_id)
                    .and_then(|t| t.fused.as_ref())
                    .is_some_and(|f| f.state == NodeState::NeedsYou && !g.seen.contains(&node_id))
                || g.ledger
                    .is_paused(g.inbound.get(&node_id).map(|x| x.0.as_str()).unwrap_or(""))
        });
        let what = what.to_string();
        tauri::async_runtime::spawn(async move {
            let _ = att.node_needs_you(target, what, still).await;
        });
    }

    fn turn_end(&'static self, node: &str, payload: Option<Value>, failed: bool) {
        if !failed {
            if let Some(path) = payload.as_ref().and_then(answer::transcript_path) {
                let since = self
                    .lock()
                    .turns
                    .get(node)
                    .filter(|t| !t.ask_ids.is_empty())
                    .map(|t| t.started_ms);
                if let Some(since) = since.filter(|s| *s > 0) {
                    let bg = answer::pending_background(&path, since as i64 - 5_000);
                    if !bg.is_empty() {
                        tracing::info!(
                            "[canvas] {node}: turn paused on background task(s) {bg:?}; the ask stays open"
                        );
                        return;
                    }
                }
            }
        }
        let (turn, tracked) = {
            let mut g = self.lock();
            let turn = g.turns.remove(node);
            g.inbound.remove(node);
            g.seen.remove(node);
            (turn, g.tracked.get(node).cloned())
        };
        let answer = self.extract_answer(node, payload.as_ref(), turn.as_ref(), tracked.as_ref());
        if let Some(turn) = &turn {
            let mut g = self.lock();
            for id in &turn.ask_ids {
                match &answer {
                    Some(a) => self.resolve_locked(&mut g, id, a.clone()),
                    None => self.resolve_locked(
                        &mut g,
                        id,
                        Answer::new(
                            "(the agent finished its turn without a text answer)",
                            AnswerSource::Screen,
                        ),
                    ),
                }
            }
        }
        if let Some(t) = tracked {
            self.run_contracts(node, &t, answer.map(|a| a.text).unwrap_or_default(), failed);
        }
        self.bump();
    }

    fn extract_answer(
        &self,
        node: &str,
        payload: Option<&Value>,
        turn: Option<&Turn>,
        t: Option<&Tracked>,
    ) -> Option<Answer> {
        if let Some(p) = payload {
            if let Some(text) = answer::from_hook(p) {
                return Some(Answer::new(text, AnswerSource::Hook));
            }
            if let Some(text) =
                answer::transcript_path(p).and_then(|path| answer::from_transcript(&path))
            {
                return Some(Answer::new(text, AnswerSource::Transcript));
            }
        }
        let _ = node;
        let tid = t?.terminal.clone()?;
        let now_text = term().screen_text(&tid, 200).ok()?.text;
        let base = turn.map(|t| t.baseline.as_str()).unwrap_or("");
        let diff = answer::screen_diff(base, &now_text, answer::SCREEN_LINES);
        let diff = strip_echo(&diff, turn.map(|t| t.echo.as_str()).unwrap_or(""));
        (!diff.trim().is_empty()).then(|| Answer::new(diff, AnswerSource::Screen))
    }

    fn run_contracts(&'static self, node: &str, t: &Tracked, output: String, failed: bool) {
        let conns: Vec<Connection> = store()
            .read(&t.ws, &t.floor, |d| {
                d.connections
                    .iter()
                    .filter(|c| c.from == node && c.kind == ConnectionKind::Handoff)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if conns.is_empty() {
            return;
        }
        let cwd = locate(node)
            .ok()
            .and_then(|(_, _, n)| n.data_str("cwd").map(str::to_string));
        let node = node.to_string();
        let name = t.name.clone();
        tauri::async_runtime::spawn(async move {
            for c in conns {
                let (ok, note) = match c.verify.as_deref().map(str::trim).filter(|v| !v.is_empty())
                {
                    Some(cmd) => {
                        let (ok, tail) = run_verify(cmd, cwd.as_deref()).await;
                        let word = if ok { "passed" } else { "failed" };
                        (ok, Some(format!("Check `{cmd}` {word}.{tail}")))
                    }
                    None => (!failed, None),
                };
                let go = match c.condition.as_ref().unwrap_or(&Condition::Always) {
                    Condition::Success => ok,
                    Condition::Failure => !ok,
                    _ => true,
                };
                if !go {
                    continue;
                }
                let text = if output.trim().is_empty() {
                    format!("{name} finished its turn.")
                } else {
                    output.clone()
                };
                let Ok((_, _, _, peers)) = peers_of(&node) else {
                    continue;
                };
                let Some(p) = peers.into_iter().find(|p| p.node.id == c.to) else {
                    continue;
                };
                wake(&p, Kind::Handoff).await;
                if host().check_target(&p, Kind::Handoff).is_err() {
                    continue;
                }
                let r = {
                    let mut g = host().lock();
                    host().admit_locked(
                        &mut g,
                        Some((&node, &name)),
                        &p,
                        Kind::Handoff,
                        &text,
                        None,
                        note.as_deref(),
                        false,
                    )
                };
                match r {
                    Ok(_) => host().bump(),
                    Err(e) => {
                        tracing::warn!("[canvas] handoff {} -> {}: {}", node, c.to, e.message)
                    }
                }
            }
        });
    }

    fn tick(&'static self) {
        let t = now();
        let m = term();
        let tracked: Vec<(String, Tracked)> = self
            .lock()
            .tracked
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut ends: Vec<String> = Vec::new();
        let mut exits: Vec<String> = Vec::new();
        let mut events: Vec<Value> = Vec::new();
        let open: Vec<(String, u32, HashSet<u32>)> = self
            .lock()
            .turns
            .iter()
            .filter(|(_, turn)| turn.by_screen && !turn.ask_ids.is_empty())
            .filter_map(|(n, turn)| {
                turn.root_pid
                    .map(|p| (n.clone(), p, turn.kids_before.clone()))
            })
            .collect();
        let working: HashSet<String> = open
            .into_iter()
            .filter(|(_, pid, before)| pty::working_children(&pty::descendants_now(*pid), before))
            .map(|(n, _, _)| n)
            .collect();
        {
            let mut g = self.lock();
            for (node, tr) in &tracked {
                if tr.exited {
                    continue;
                }
                let (sig, alive) = match &tr.terminal {
                    Some(tid) => match m.info(tid) {
                        Some(info) => (Some(info.agent.clone()), info.alive),
                        None => (None, !tr.seen_alive),
                    },
                    None => {
                        let state = if tr.native_busy {
                            AgentState::Working
                        } else if tr.native_failed {
                            AgentState::Failed
                        } else {
                            AgentState::Idle
                        };
                        (
                            Some(AgentSignal {
                                state,
                                source: SignalSource::Hook,
                                since_ms: t as i64,
                                message: None,
                                progress: None,
                            }),
                            true,
                        )
                    }
                };
                if tr.terminal.is_some() && sig.is_some() && alive {
                    if let Some(x) = g.tracked.get_mut(node) {
                        x.seen_alive = true;
                    }
                }
                if !alive && (tr.seen_alive || tr.terminal.is_some() && sig.is_some()) {
                    exits.push(node.clone());
                    continue;
                }
                let now_state = sig.as_ref().map(|s| s.state).unwrap_or(AgentState::Unknown);
                let kid_works = working.contains(node);
                if now_state == AgentState::Blocked && tr.last_state != AgentState::Blocked {
                    g.seen.remove(node);
                }
                let mut sig = sig;
                if kid_works {
                    if let Some(s) = sig.as_mut() {
                        if !node_state::is_busy(s.state) {
                            s.state = AgentState::Working;
                            s.since_ms = t as i64;
                        }
                    }
                }
                if let Some(tid) = &tr.terminal {
                    if !g.delivering.contains(node) && m.user_typing(tid) {
                        g.typing_ms.insert(node.clone(), t);
                    }
                    let prev = tr.last_state;
                    let hook_closed = g
                        .hook_turn_end_ms
                        .get(node)
                        .is_some_and(|x| t.saturating_sub(*x) < 3_000);
                    if node_state::turn_ended(prev, now_state) && !hook_closed && !kid_works {
                        tracing::info!("[canvas] {node}: turn ended ({prev:?} -> {now_state:?})");
                        ends.push(node.clone());
                    } else if !kid_works
                        && !node_state::is_busy(now_state)
                        && !g.confirming.contains(node)
                    {
                        let quiet_end = g.turns.get(node).is_some_and(|turn| {
                            turn.by_screen
                                && !turn.ask_ids.is_empty()
                                && turn.started_ms > 0
                                && m.screen_text(tid, 1).is_ok_and(|s| {
                                    s.changed_ms > turn.started_ms as i64
                                        && t as i64 - s.changed_ms >= QUIET_TURN_END_MS
                                })
                        });
                        if quiet_end {
                            tracing::info!(
                                "[canvas] {node}: turn ended (screen quiet, no child working)"
                            );
                            ends.push(node.clone());
                        }
                    }
                    if !node_state::is_busy(prev)
                        && node_state::is_busy(now_state)
                        && !g.confirming.contains(node)
                        && !g.delivering.contains(node)
                    {
                        self.user_turn_locked(&mut g, node);
                    }
                }
                let fused = node_state::fuse(
                    &Inputs {
                        signal: sig.clone(),
                        alive,
                        started_ms: tr.started_ms as i64,
                        from_bus: tr.terminal.is_none(),
                        stuck: g.stuck.contains(node),
                        seen: g.seen.contains(node),
                        permission_pending: g.permission.contains_key(node),
                    },
                    t as i64,
                );
                if let Some(x) = g.tracked.get_mut(node) {
                    x.last_state = now_state;
                    let changed = x
                        .fused
                        .as_ref()
                        .map(|f| (f.state, f.source, f.confidence, f.detail.clone()))
                        != Some((
                            fused.state,
                            fused.source,
                            fused.confidence,
                            fused.detail.clone(),
                        ));
                    if changed {
                        let newly_needs = fused.state == NodeState::NeedsYou
                            && x.fused.as_ref().map(|f| f.state) != Some(NodeState::NeedsYou);
                        let newly_done = fused.state == NodeState::Done
                            && x.fused.as_ref().map(|f| f.state) != Some(NodeState::Done);
                        events.push(json!({
                            "id": node, "workspaceId": x.ws, "floorId": x.floor,
                            "state": fused.state, "source": fused.source,
                            "confidence": fused.confidence, "detail": fused.detail, "since_ms": fused.since_ms,
                            "newlyNeedsYou": newly_needs, "newlyDone": newly_done,
                        }));
                        x.fused = Some(fused);
                    }
                }
            }
            let gone: Vec<String> = g
                .asks
                .values()
                .filter(|a| {
                    a.waiters == 0
                        && matches!(a.state, AskState::Pending)
                        && t.saturating_sub(a.last_poll_ms) > WAIT_GONE_MS
                        && g.graph.contains(&a.id)
                })
                .map(|a| a.id.clone())
                .collect();
            for id in gone {
                g.graph.remove(&id);
            }
            let expired: Vec<String> = g
                .asks
                .values()
                .filter(|a| matches!(a.state, AskState::Pending) && t > a.deadline_ms)
                .map(|a| a.id.clone())
                .collect();
            for id in expired {
                self.fail_locked(&mut g, &id, "timeout", "the ask went past its time limit");
            }
            g.ledger.gc(t);
        }
        for ev in events {
            let newly = ev["newlyNeedsYou"].as_bool().unwrap_or(false);
            if newly {
                let id = ev["id"].as_str().unwrap_or("").to_string();
                let name = self.name_of(&id);
                let detail = ev["detail"].as_str().unwrap_or("needs you").to_string();
                self.needs_you(&id, &name, &detail);
            }
            if ev["newlyDone"].as_bool().unwrap_or(false) {
                let id = ev["id"].as_str().unwrap_or("").to_string();
                let failed = ev["detail"].as_str() == Some("failed");
                self.turn_done_notice(&id, failed);
            }
            self.emit(EVENT_NODE_STATE, ev);
        }
        for node in exits {
            self.on_exit(&node);
        }
        for node in ends {
            self.turn_end(&node, None, false);
        }
        self.deliver_answers_nobody_collected();
        self.start_deliveries();
    }

    fn deliver_answers_nobody_collected(&'static self) {
        let t = now();
        let mut g = self.lock();
        let due: Vec<Ask> = g
            .asks
            .values()
            .filter(|a| {
                matches!(a.state, AskState::Answered(_))
                    && !a.collected
                    && !a.reply_sent
                    && a.waiters == 0
                    && a.answered_ms
                        .is_some_and(|x| t.saturating_sub(x) > COLLECT_GRACE_MS)
            })
            .cloned()
            .collect();
        for a in due {
            if let Some(x) = g.asks.get_mut(&a.id) {
                x.reply_sent = true;
            }
            let AskState::Answered(ans) = &a.state else {
                continue;
            };
            let Ok((ws, floor, asker)) = locate(&a.from) else {
                continue;
            };
            let Ok((_, _, target)) = locate(&a.to) else {
                continue;
            };
            let p = Peer {
                name: node_name(&asker),
                perms: vec!["ask".into()],
                conn: Connection {
                    id: String::new(),
                    from: a.to.clone(),
                    to: a.from.clone(),
                    kind: ConnectionKind::Ask,
                    perms: vec![],
                    condition: None,
                    verify: None,
                    label: None,
                    points: vec![],
                    from_note_id: None,
                    to_note_id: None,
                    is_active: None,
                },
                node: asker,
                ws,
                floor,
            };
            let mut text = ans.text.clone();
            if let Some(f) = &ans.file {
                if !text.contains(f.as_str()) {
                    text.push_str(&format!("\n(full answer: {f})"));
                }
            }
            let tname = node_name(&target);
            let _ = self.admit_locked(
                &mut g,
                Some((&a.to, &tname)),
                &p,
                Kind::Reply,
                &text,
                Some(&a.id),
                None,
                false,
            );
        }
    }

    fn start_deliveries(&'static self) {
        let ready: Vec<String> = {
            let g = self.lock();
            g.mail
                .iter()
                .filter(|(n, q)| !q.is_empty() && !g.delivering.contains(*n))
                .map(|(n, _)| n.clone())
                .collect()
        };
        for node in ready {
            let msg = {
                let mut g = self.lock();
                let Some(id) = g.mail.get(&node).and_then(|q| q.front().cloned()) else {
                    continue;
                };
                g.delivering.insert(node.clone());
                let m = g.msgs.get(&id).cloned();
                if let Some(x) = g.msgs.get_mut(&id) {
                    x.state = MsgState::Delivering;
                }
                m
            };
            let Some(msg) = msg else {
                let mut g = self.lock();
                if let Some(q) = g.mail.get_mut(&node) {
                    q.pop_front();
                }
                g.delivering.remove(&node);
                continue;
            };
            tauri::async_runtime::spawn(async move {
                let ok = host().deliver(&msg).await;
                let mut g = host().lock();
                if let Some(q) = g.mail.get_mut(&msg.to) {
                    q.retain(|x| x != &msg.id);
                }
                g.delivering.remove(&msg.to);
                if let Some(x) = g.msgs.get_mut(&msg.id) {
                    x.state = if ok {
                        MsgState::Delivered
                    } else {
                        MsgState::Stuck
                    };
                    let x = x.clone();
                    host().msg_event(&x);
                }
                let cut = now().saturating_sub(60 * 60 * 1000);
                g.msgs.retain(|_, m| {
                    m.created_ms > cut || matches!(m.state, MsgState::Queued | MsgState::Delivering)
                });
                drop(g);
                host().bump();
            });
        }
    }

    async fn deliver(&'static self, msg: &Msg) -> bool {
        let Ok((ws, _floor, node)) = locate(&msg.to) else {
            self.fail_msg(msg, "target_exited", "the node is gone");
            return false;
        };
        let agent = agent_of(&node);
        let inbox = store().inbox_dir(&ws, &msg.to);
        let (text, _spilled) = envelope::build(
            &Envelope {
                from: &msg.from_name,
                kind: msg.kind,
                id: msg.ask_id.as_deref().unwrap_or(&msg.id),
                hop: msg.hop,
                text: &msg.text,
                in_reply_to: msg.in_reply_to.as_deref(),
                note: msg.note.as_deref(),
            },
            &inbox,
        );
        if agent == "native" {
            return self.deliver_native(msg, &node, text).await;
        }
        let text = if adapters::reads_envelope(&agent) {
            text
        } else {
            msg.text.clone()
        };
        let Some(tid) = terminal_of(&node) else {
            self.fail_msg(msg, "target_exited", "no terminal");
            return false;
        };
        self.deliver_pty(msg, &tid, &agent, text).await
    }

    fn fail_msg(&self, msg: &Msg, code: &str, why: &str) {
        let mut g = self.lock();
        if let Some(id) = &msg.ask_id {
            self.fail_locked(&mut g, id, code, why);
        }
        if let Some(x) = g.msgs.get_mut(&msg.id) {
            x.state = MsgState::Failed;
        }
    }

    async fn deliver_pty(&'static self, msg: &Msg, tid: &str, agent: &str, text: String) -> bool {
        let m = term();
        let is_reply = msg.kind == Kind::Reply;
        let deadline = now() + pty::GATE_TIMEOUT_MS;
        loop {
            if msg.ask_id.as_ref().is_some_and(|id| {
                self.lock()
                    .asks
                    .get(id)
                    .is_some_and(|a| !matches!(a.state, AskState::Pending))
            }) {
                return true;
            }
            let Some(info) = m.info(tid) else {
                self.fail_msg(msg, "target_exited", "the terminal is closed");
                return false;
            };
            if !info.alive {
                self.fail_msg(msg, "target_exited", "the process exited");
                return false;
            }
            let t = now();
            let typing = m.user_typing(tid)
                || self
                    .lock()
                    .typing_ms
                    .get(&msg.to)
                    .is_some_and(|x| t.saturating_sub(*x) < pty::USER_TYPING_HOLD_MS);
            let changed_ago = m
                .screen_text(tid, 1)
                .map(|s| t as i64 - s.changed_ms)
                .unwrap_or(0);
            let busy_cli = self.lock().graph.is_waiting(&msg.to);
            let busy_cli = busy_cli
                || (!adapters::has_turn_hooks(agent)
                    && self
                        .lock()
                        .turns
                        .get(&msg.to)
                        .is_some_and(|t| t.by_screen && !t.ask_ids.is_empty()));
            if !typing
                && !busy_cli
                && node_state::deliverable(&info.agent, changed_ago, t as i64, is_reply)
            {
                break;
            }
            if t > deadline {
                self.fail_msg(msg, "delivery_stuck", "the target never became ready");
                return false;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let text = if adapters::bracketed(agent) {
            text
        } else {
            pty::flatten(&text)
        };
        let before = m.screen_text(tid, 200).map(|s| s.text).unwrap_or_default();
        let hookless = !adapters::has_turn_hooks(agent);
        let root_pid = m.info(tid).and_then(|i| i.pid).filter(|_| hookless);
        let kids_before = root_pid.map(pty::descendants_now).unwrap_or_default();
        {
            let mut g = self.lock();
            g.confirming.insert(msg.to.clone());
            g.submitted.remove(&msg.to);
        }
        if m.paste(tid, &text).is_err() {
            self.lock().confirming.remove(&msg.to);
            self.fail_msg(msg, "target_exited", "paste failed");
            return false;
        }
        tokio::time::sleep(Duration::from_millis(pty::ECHO_MIN_MS)).await;
        let echo_deadline = now() + pty::ECHO_MAX_MS - pty::ECHO_MIN_MS;
        loop {
            let now_text = m.screen_text(tid, 200).map(|s| s.text).unwrap_or_default();
            if pty::echoed(&before, &now_text, &text) || now() >= echo_deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        let echoed_screen = m.screen_text(tid, 200).map(|s| s.text).unwrap_or_default();
        let _ = m.submit(tid);
        let mut retries = 0u32;
        let mut since = now();
        let taken = loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let submitted = self.lock().submitted.contains(&msg.to);
            let busy = m
                .agent_state(tid)
                .is_some_and(|s| node_state::is_busy(s.state));
            let moved = hookless
                && m.screen_text(tid, 200)
                    .map(|s| s.text != echoed_screen)
                    .unwrap_or(false);
            match pty::confirm_step(
                now().saturating_sub(since),
                retries,
                submitted || busy || moved,
            ) {
                pty::Confirm::Taken => break true,
                pty::Confirm::Wait => {}
                pty::Confirm::Retry => {
                    retries += 1;
                    since = now();
                    let _ = m.submit(tid);
                }
                pty::Confirm::Stuck => break false,
            }
        };
        let mut g = self.lock();
        g.confirming.remove(&msg.to);
        g.submitted.remove(&msg.to);
        if !taken {
            g.stuck.insert(msg.to.clone());
            drop(g);
            let name = self.name_of(&msg.to);
            self.needs_you(
                &msg.to,
                &name,
                "A message was pasted but not sent: check its input.",
            );
            self.fail_msg(
                msg,
                "delivery_stuck",
                "the message was pasted but never taken",
            );
            return false;
        }
        g.stuck.remove(&msg.to);
        self.mark_delivered_locked(&mut g, msg, before);
        if let Some(t) = g.turns.get_mut(&msg.to) {
            if t.echo.is_empty() {
                t.echo = echo_tail(&text);
            }
            t.by_screen = hookless;
            if hookless && t.root_pid.is_none() {
                t.root_pid = root_pid;
                t.kids_before = kids_before;
            }
        }
        true
    }

    pub async fn deliver_chat(&'static self, node_id: &str, text: String) -> bool {
        let Ok((ws, floor, node)) = locate(node_id) else {
            return false;
        };
        let started = terminal_of(&node).is_some_and(|t| term().info(&t).is_some());
        if !started && agent_of(&node) != "native" {
            if let Err(e) = super::launch::ensure_running(&ws, &floor, node_id).await {
                tracing::warn!("[canvas] chat to {node_id}: {e}");
            }
        }
        let agent = agent_of(&node);
        let Some(tid) = terminal_of(&node) else {
            return false;
        };
        let msg = Msg {
            id: short_id("chat"),
            from: None,
            from_name: "User".into(),
            to: node_id.to_string(),
            kind: Kind::Message,
            text: text.clone(),
            ask_id: None,
            in_reply_to: None,
            note: None,
            chain: short_id("chain"),
            hop: 0,
            state: MsgState::Delivering,
            created_ms: now(),
        };
        self.deliver_pty(&msg, &tid, &agent, text).await
    }

    fn mark_delivered_locked(&self, g: &mut Inner, msg: &Msg, baseline: String) {
        let t = now();
        g.inbound
            .insert(msg.to.clone(), (msg.chain.clone(), msg.hop));
        let turn = g.turns.entry(msg.to.clone()).or_default();
        if turn.started_ms == 0 {
            turn.started_ms = t;
            turn.baseline = baseline;
        }
        if let Some(id) = &msg.ask_id {
            turn.ask_ids.push(id.clone());
            if let Some(a) = g.asks.get_mut(id) {
                a.delivered_ms = Some(t);
                let a = a.clone();
                self.ask_event(&a);
            }
        }
        g.seen.remove(&msg.to);
    }

    async fn deliver_native(&'static self, msg: &Msg, node: &CanvasNode, text: String) -> bool {
        use futures::StreamExt;
        use omniget_core::core::llm::types::TurnEvent;
        let Some(app) = self.app().cloned() else {
            self.fail_msg(msg, "no_app", "the app is not ready");
            return false;
        };
        let Some(agent_id) = native_adapter::agent_id(&node.data) else {
            self.fail_msg(msg, "no_agent", "the node has no agent configured");
            return false;
        };
        loop {
            let busy = self
                .lock()
                .tracked
                .get(&msg.to)
                .is_some_and(|t| t.native_busy);
            if !busy {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        {
            let mut g = self.lock();
            if let Some(t) = g.tracked.get_mut(&msg.to) {
                t.native_busy = true;
                t.native_failed = false;
            }
            self.mark_delivered_locked(&mut g, msg, String::new());
        }
        let llm = app.state::<crate::AppState>().llm.clone();
        let conv = native_adapter::conversation_id(&msg.to);
        let mut out = String::new();
        let mut failed = false;
        match llm.turn_stream(&conv, &agent_id, &text).await {
            Ok((_rid, _cancel, mut stream)) => {
                while let Some(ev) = stream.next().await {
                    match ev {
                        TurnEvent::TextDelta { text } => out.push_str(&text),
                        TurnEvent::Error { error } => {
                            failed = true;
                            if out.is_empty() {
                                out = format!("(error: {error:?})");
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                failed = true;
                out = format!("(error: {e})");
            }
        }
        {
            let mut g = self.lock();
            if let Some(t) = g.tracked.get_mut(&msg.to) {
                t.native_busy = false;
                t.native_failed = failed;
            }
            let ids: Vec<String> = g
                .turns
                .get(&msg.to)
                .map(|t| t.ask_ids.clone())
                .unwrap_or_default();
            for id in ids {
                self.resolve_locked(&mut g, &id, Answer::new(out.clone(), AnswerSource::Native));
            }
        }
        self.turn_end(
            &msg.to,
            Some(json!({ "last_assistant_message": out })),
            failed,
        );
        true
    }

    fn on_exit(&'static self, node: &str) {
        let mut g = self.lock();
        let Some(t) = g.tracked.get_mut(node) else {
            return;
        };
        if t.exited {
            return;
        }
        t.exited = true;
        let (ws, floor) = (t.ws.clone(), t.floor.clone());
        let pending: Vec<String> = g
            .asks
            .values()
            .filter(|a| a.to == node && matches!(a.state, AskState::Pending))
            .map(|a| a.id.clone())
            .collect();
        for id in pending {
            self.fail_locked(&mut g, &id, "target_exited", "the target's process exited");
        }
        let mine: Vec<String> = g
            .graph
            .waiting_on(node)
            .into_iter()
            .map(|e| e.ask_id)
            .collect();
        for id in mine {
            g.graph.remove(&id);
        }
        g.mail.remove(node);
        g.turns.remove(node);
        g.inbound.remove(node);
        drop(g);
        self.revoke(node);
        self.emit(
            EVENT_NODE_STATE,
            json!({ "id": node, "workspaceId": ws, "floorId": floor, "state": "exited", "source": "exit", "confidence": "exact" }),
        );
    }

    fn on_deleted(&'static self, node: &str) {
        let tracked = self.lock().tracked.contains_key(node);
        if tracked {
            self.on_exit(node);
            self.lock().tracked.remove(node);
        } else {
            self.revoke(node);
        }
    }

    fn revoke(&self, node: &str) {
        let Some(app) = self.app() else { return };
        if let Ok(jobs) = crate::jobs::get(app) {
            let db = jobs.db();
            if let Err(e) = crate::bridge_tokens::revoke_canvas_node(&db, node, now()) {
                tracing::warn!("[canvas] revoke token of {node}: {e}");
            }
        }
    }

    pub fn asks_list(&self, ws: Option<&str>) -> Vec<Value> {
        let g = self.lock();
        let name = |id: &str| {
            g.tracked
                .get(id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let mut v: Vec<&Ask> = g
            .asks
            .values()
            .filter(|a| ws.is_none_or(|w| a.ws == w))
            .collect();
        v.sort_by(|a, b| b.id.cmp(&a.id));
        v.into_iter()
            .take(200)
            .map(|a| {
                let answer = match &a.state {
                    AskState::Answered(ans) => Some(ans.text.chars().take(400).collect::<String>()),
                    _ => None,
                };
                json!({
                    "id": a.id, "from": a.from, "fromName": name(&a.from), "to": a.to, "toName": name(&a.to),
                    "workspaceId": a.ws, "state": ask_state_str(a), "text": a.text.chars().take(400).collect::<String>(),
                    "answer": answer, "createdMs": a.created_ms, "deliveredMs": a.delivered_ms,
                    "answeredMs": a.answered_ms, "chain": a.chain, "hop": a.hop,
                })
            })
            .collect()
    }

    pub fn active_pairs(&self) -> Vec<(String, String)> {
        self.lock()
            .asks
            .values()
            .filter(|a| matches!(a.state, AskState::Pending) && a.released_by.is_none())
            .map(|a| (a.from.clone(), a.to.clone()))
            .collect()
    }

    pub fn mark_active(&self, doc: &mut Value) {
        let pairs = self.active_pairs();
        let remote: HashMap<String, String> = doc["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|n| n["kind"] == "connector")
            .filter_map(|n| {
                Some((
                    n["id"].as_str()?.to_string(),
                    n["data"]["remoteNodeId"].as_str()?.to_string(),
                ))
            })
            .collect();
        let end = |v: &Value| -> String {
            let id = v.as_str().unwrap_or("");
            remote.get(id).cloned().unwrap_or_else(|| id.to_string())
        };
        if let Some(conns) = doc["connections"].as_array_mut() {
            for c in conns {
                let (a, b) = (end(&c["from"]), end(&c["to"]));
                let lit = pairs
                    .iter()
                    .any(|(x, y)| (x == &a && y == &b) || (x == &b && y == &a));
                if let Some(m) = c.as_object_mut() {
                    m.insert("isActive".into(), json!(lit));
                }
            }
        }
    }

    pub fn inbox(&self, ws: Option<&str>) -> Vec<Value> {
        let g = self.lock();
        let mut out = Vec::new();
        for (id, t) in &g.tracked {
            if ws.is_some_and(|w| w != t.ws) || t.exited {
                continue;
            }
            let Some(f) = &t.fused else { continue };
            let reason = if g.permission.contains_key(id) {
                "permission"
            } else if g.stuck.contains(id) {
                "stuck"
            } else {
                match f.state {
                    NodeState::NeedsYou if g.seen.contains(id) => continue,
                    NodeState::NeedsYou => "blocked",
                    NodeState::Done => "review",
                    _ => continue,
                }
            };
            let queued = g.mail.get(id).map(|q| q.len()).unwrap_or(0);
            out.push(json!({
                "nodeId": id, "name": t.name, "workspaceId": t.ws, "floorId": t.floor,
                "reason": reason, "state": f.state, "detail": f.detail, "sinceMs": f.since_ms,
                "queued": queued,
            }));
        }
        out.sort_by_key(|v| v["sinceMs"].as_i64().unwrap_or(0));
        out
    }

    pub fn is_seen(&self, node: &str) -> bool {
        self.lock().seen.contains(node)
    }

    pub fn terminal_seen(&self, tid: &str) -> bool {
        let g = self.lock();
        g.tracked
            .iter()
            .any(|(n, t)| t.terminal.as_deref() == Some(tid) && g.seen.contains(n))
    }

    pub fn mark_seen(&self, node: &str) {
        let mut g = self.lock();
        g.seen.insert(node.to_string());
        g.stuck.remove(node);
        if let Some(t) = g.tracked.get(node).and_then(|t| t.terminal.clone()) {
            let _ = term().focus(&t, true);
        }
    }

    pub fn cord_test(
        &self,
        app: &AppHandle,
        ws: &str,
        floor: &str,
        conn_id: &str,
    ) -> Result<Value, String> {
        let s = store();
        let (conn, from, to) = s.read(ws, floor, |d| {
            let c = d.connection(conn_id).cloned();
            let f = c.as_ref().and_then(|c| d.node(&c.from).cloned());
            let t = c.as_ref().and_then(|c| d.node(&c.to).cloned());
            (c, f, t)
        })?;
        let conn = conn.ok_or_else(|| format!("CANVAS_NOT_FOUND: connection {conn_id}"))?;
        let settings = crate::storage::config::load_settings(app);
        let mut issues: Vec<String> = Vec::new();
        if !settings.bridge.enabled && crate::local_bridge::canvas_port().is_none() {
            issues.push("bridge_off".into());
        }
        let end = |n: &Option<CanvasNode>| -> Value {
            match n {
                None => json!({ "exists": false }),
                Some(n) => {
                    let tid = terminal_of(n);
                    let info = tid.as_ref().and_then(|t| term().info(t));
                    let f = self.fused_of(&n.id, ws, floor, n);
                    json!({
                        "exists": true, "id": n.id, "name": node_name(n), "kind": n.kind.as_str(),
                        "agent": n.data_str("agent"), "alive": info.as_ref().map(|i| i.alive),
                        "state": f.state, "confidence": f.confidence,
                        "tokenMinted": self.lock().tracked.contains_key(&n.id),
                    })
                }
            }
        };
        let (fv, tv) = (end(&from), end(&to));
        for (label, v) in [("from", &fv), ("to", &tv)] {
            if v["exists"] == false {
                issues.push(format!("{label}_missing"));
            } else if v["alive"] == false {
                issues.push(format!("{label}_not_running"));
            } else if v["kind"] == "terminal" && v["tokenMinted"] == false {
                issues.push(format!("{label}_without_cords_env"));
            }
        }
        Ok(json!({
            "ok": issues.is_empty(), "issues": issues, "kind": conn.kind.as_str(), "perms": perms_of(&conn),
            "bridge": { "enabled": settings.bridge.enabled, "port": settings.bridge.port },
            "from": fv, "to": tv,
        }))
    }
}

fn release_asker(g: &Inner, a: &Ask) {
    let still_waiting = g
        .asks
        .values()
        .any(|x| x.from == a.from && x.id != a.id && matches!(x.state, AskState::Pending));
    if still_waiting {
        return;
    }
    if let Some(t) = g.tracked.get(&a.from).and_then(|t| t.terminal.clone()) {
        let state = if a.asker_in_turn {
            AgentState::Working
        } else {
            AgentState::Idle
        };
        let _ = term().set_agent_state(&t, state, SignalSource::Hook, None);
    }
}

fn in_turn(g: &Inner, node: &str) -> bool {
    let tid = g
        .tracked
        .get(node)
        .and_then(|t| t.terminal.clone())
        .or_else(|| locate(node).ok().and_then(|(_, _, n)| terminal_of(&n)));
    match tid.and_then(|t| term().agent_state(&t)) {
        Some(s) => !matches!(
            s.state,
            AgentState::Idle | AgentState::Success | AgentState::Failed
        ),
        None => g.tracked.get(node).is_some_and(|t| t.native_busy),
    }
}

fn echo_tail(pasted: &str) -> String {
    let flat = pty::flatten(pasted);
    let chars: Vec<char> = flat.chars().collect();
    chars[chars.len().saturating_sub(24)..]
        .iter()
        .collect::<String>()
        .trim()
        .to_string()
}

fn strip_echo(diff: &str, echo: &str) -> String {
    let key: String = echo.chars().filter(|c| !c.is_whitespace()).collect();
    if key.is_empty() {
        return diff.to_string();
    }
    let lines: Vec<&str> = diff.lines().collect();
    let mut joined = String::new();
    let mut ends = Vec::with_capacity(lines.len());
    for l in &lines {
        joined.extend(l.chars().filter(|c| !c.is_whitespace()));
        ends.push(joined.len());
    }
    match joined.find(&key) {
        Some(at) => {
            let end = at + key.len();
            let i = ends.iter().position(|e| *e >= end).unwrap_or(lines.len());
            lines[(i + 1).min(lines.len())..]
                .join("\n")
                .trim()
                .to_string()
        }
        None => diff.to_string(),
    }
}

fn ask_state_str(a: &Ask) -> &'static str {
    match &a.state {
        AskState::Answered(_) => "answered",
        AskState::Failed { .. } => "failed",
        AskState::Pending if a.released_by.is_some() => "released",
        AskState::Pending if a.delivered_ms.is_some() => "delivered",
        AskState::Pending => "queued",
    }
}

async fn run_verify(cmd: &str, cwd: Option<&str>) -> (bool, String) {
    let mut c = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    if let Some(d) = cwd.filter(|d| std::path::Path::new(d).is_dir()) {
        c.current_dir(d);
    }
    c.kill_on_drop(true);
    match tokio::time::timeout(VERIFY_TIMEOUT, c.output()).await {
        Ok(Ok(o)) => {
            let mut all = String::from_utf8_lossy(&o.stdout).into_owned();
            all.push_str(&String::from_utf8_lossy(&o.stderr));
            let tail: Vec<&str> = all.lines().rev().take(20).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            let t = if tail.is_empty() {
                String::new()
            } else {
                format!("\nLast lines of the check:\n{}", tail.join("\n"))
            };
            (o.status.success(), t)
        }
        Ok(Err(e)) => (false, format!(" ({e})")),
        Err(_) => (false, " (timed out after 5 min)".into()),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeEnv {
    pub env: HashMap<String, String>,
    pub args: Vec<String>,
    pub env_remove: Vec<String>,
}

pub fn plugin_dir() -> Option<std::path::PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("canvas").join("plugin"))
}

pub fn install_plugin() -> Result<(), String> {
    let dir = plugin_dir().ok_or("no app data dir")?;
    claude_adapter::install_plugin(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}

pub fn skill_root(app: &AppHandle, agent: &str) -> Option<std::path::PathBuf> {
    let raw = crate::canvas::prefs::get(app)
        .skills
        .roots
        .get(agent)
        .cloned()
        .unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = match raw.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()?.join(rest),
        None if raw == "~" => dirs::home_dir()?,
        None => std::path::PathBuf::from(raw),
    };
    Some(path)
}

pub fn install_skill_root(
    app: &AppHandle,
    agent: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    let Some(root) = skill_root(app, agent) else {
        return Ok(None);
    };
    let file = root.join(claude_adapter::PLUGIN_NAME).join("SKILL.md");
    let body = claude_adapter::skill_md();
    if std::fs::read_to_string(&file).ok().as_deref() != Some(body.as_str()) {
        std::fs::create_dir_all(file.parent().unwrap_or(&root))
            .and_then(|_| std::fs::write(&file, body))
            .map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(Some(file))
}

pub fn agents_md_snippet() -> String {
    adapters::agents_md_snippet()
}

fn rpc_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

type NotePeer = (String, String, String, CanvasNode, Vec<String>);

fn note_peers(ctx: &NodeCtx) -> Result<Vec<NotePeer>, String> {
    let (_ws, _f, _me, peers) = peers_of(&ctx.node_id).map_err(|e| e.message)?;
    let mut out: Vec<NotePeer> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(ctx.node_id.clone());
    for p in &peers {
        if p.node.kind == NodeKind::Note {
            seen.insert(p.node.id.clone());
        }
    }
    let push = |out: &mut Vec<NotePeer>, ws: &str, floor: &str, n: CanvasNode, perms: &[String]| {
        if out.iter().any(|o| o.3.id == n.id) {
            return;
        }
        out.push((
            node_name(&n),
            ws.to_string(),
            floor.to_string(),
            n,
            perms.to_vec(),
        ));
    };
    for p in peers {
        match p.node.kind {
            NodeKind::Note => {
                push(&mut out, &p.ws, &p.floor, p.node.clone(), &p.perms);
                for (n, _) in note_chain(&p.ws, &p.floor, &p.node.id, &mut seen, 0) {
                    push(&mut out, &p.ws, &p.floor, n, &p.perms);
                }
            }
            NodeKind::Stack => {
                for (_, n) in stack_notes(&p.ws, &p.floor, &p.node) {
                    seen.insert(n.id.clone());
                    push(&mut out, &p.ws, &p.floor, n, &p.perms);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn note_chain(
    ws: &str,
    floor: &str,
    note_id: &str,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Vec<(CanvasNode, usize)> {
    const MAX_DEPTH: usize = 16;
    if depth >= MAX_DEPTH {
        return Vec::new();
    }
    let kids: Vec<CanvasNode> = store()
        .read(ws, floor, |d| {
            d.connections
                .iter()
                .filter_map(|c| c.other_end(note_id))
                .filter_map(|o| d.node(o))
                .filter(|n| n.kind == NodeKind::Note)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for k in kids {
        if !seen.insert(k.id.clone()) {
            continue;
        }
        let id = k.id.clone();
        out.push((k, depth + 1));
        out.extend(note_chain(ws, floor, &id, seen, depth + 1));
    }
    out
}

fn note_children_json(
    ws: &str,
    floor: &str,
    note_id: &str,
    seen: &mut HashSet<String>,
) -> Vec<Value> {
    let flat = note_chain(ws, floor, note_id, seen, 0);
    fn build(flat: &[(CanvasNode, usize)], i: &mut usize, depth: usize) -> Vec<Value> {
        let mut out = Vec::new();
        while *i < flat.len() && flat[*i].1 == depth {
            let n = &flat[*i].0;
            *i += 1;
            let children = build(flat, i, depth + 1);
            out.push(json!({
                "name": node_name(n), "kind": "note",
                "locked": n.data.get("isContentLocked").and_then(|v| v.as_bool()).unwrap_or(false),
                "children": children,
            }));
        }
        out
    }
    let mut i = 0;
    build(&flat, &mut i, 1)
}

fn stack_notes(ws: &str, floor: &str, stack: &CanvasNode) -> Vec<(String, CanvasNode)> {
    let tabs: Vec<String> = stack
        .data
        .get("tabs")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    store()
        .read(ws, floor, |d| {
            let mut v: Vec<(String, CanvasNode)> = tabs
                .iter()
                .filter_map(|id| d.node(id).cloned().map(|n| (id.clone(), n)))
                .collect();
            for n in d
                .nodes
                .iter()
                .filter(|n| n.stack_id.as_deref() == Some(stack.id.as_str()))
            {
                if !v.iter().any(|(id, _)| id == &n.id) {
                    v.push((n.id.clone(), n.clone()));
                }
            }
            v
        })
        .unwrap_or_default()
}

fn find_note(
    ctx: &NodeCtx,
    name: &str,
) -> Result<(String, String, CanvasNode, Vec<String>), String> {
    let all = note_peers(ctx)?;
    let exact: Vec<_> = all.iter().filter(|x| x.0 == name).collect();
    let hits: Vec<_> = if exact.is_empty() {
        all.iter()
            .filter(|x| x.0.eq_ignore_ascii_case(name.trim()))
            .collect()
    } else {
        exact
    };
    match hits.len() {
        1 => {
            let (_, ws, floor, n, perms) = hits[0].clone();
            Ok((ws, floor, n, perms))
        }
        0 => Err(format!(
            "no connected note named '{name}'. Run `omniget canvas list`."
        )),
        _ => Err(format!(
            "Multiple notes named '{name}'. Rename them to have unique names."
        )),
    }
}

fn writable(perms: &[String]) -> Result<(), String> {
    if perms.iter().any(|p| p == "note_ro") {
        return Err("this note is connected read-only".into());
    }
    Ok(())
}

fn numbered(text: &str, offset: usize, limit: usize) -> String {
    text.lines()
        .enumerate()
        .skip(offset)
        .take(limit)
        .map(|(i, l)| format!("{:>5}\t{l}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn rpc(ctx: &NodeCtx, method: &str, args: serde_json::Value) -> RpcResult {
    let ctx = ctx.clone();
    let method = method.to_string();
    if !(method.starts_with("note.") || method.starts_with("cords.")) {
        return None;
    }
    let r = tauri::async_runtime::spawn_blocking(move || rpc_blocking(&ctx, &method, &args))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    Some(r)
}

fn rpc_blocking(ctx: &NodeCtx, method: &str, a: &Value) -> Result<Value, String> {
    let s = store();
    let origin = store::agent_origin(&ctx.node_id);
    let name = a["name"].as_str().unwrap_or("");
    match method {
        "cords.peers" => host().peers(&ctx.node_id).map_err(|e| e.message),
        "cords.me" => host().me(&ctx.node_id).map_err(|e| e.message),
        "note.list" => {
            let notes: Vec<Value> = note_peers(ctx)?
                .into_iter()
                .map(|(n, _, _, node, perms)| json!({ "name": n, "id": node.id, "perms": perms }))
                .collect();
            Ok(json!(notes))
        }
        "note.read" => {
            let (ws, floor, n, _) = find_note(ctx, name)?;
            let c = notes::read(&s, &ws, &floor, &n.id)?;
            let offset = a["offset"].as_u64().unwrap_or(0) as usize;
            let limit = a["limit"].as_u64().unwrap_or(2_000) as usize;
            let body = numbered(&c.text, offset, limit.max(1));
            Ok(
                json!({ "text": body, "name": c.display_name, "revision": c.revision, "lines": c.text.lines().count() }),
            )
        }
        "note.write" => {
            let (ws, floor, n, perms) = find_note(ctx, name)?;
            writable(&perms)?;
            let text = a["text"].as_str().unwrap_or("");
            let c = notes::write(&s, &ws, &floor, &n.id, text, None, &origin)?;
            let renamed = c.display_name != node_name(&n);
            let msg = if renamed {
                format!(
                    "Wrote '{}' (the note is now named '{}').",
                    name, c.display_name
                )
            } else {
                format!("Wrote '{}'.", c.display_name)
            };
            Ok(json!({ "message": msg, "name": c.display_name, "revision": c.revision }))
        }
        "note.edit" => {
            let (ws, floor, n, perms) = find_note(ctx, name)?;
            writable(&perms)?;
            let old = a["old"].as_str().unwrap_or("");
            let new = a["new"].as_str().unwrap_or("");
            if old.is_empty() {
                return Err("note edit needs the old text".into());
            }
            let cur = notes::read(&s, &ws, &floor, &n.id)?;
            let count = cur.text.matches(old).count();
            if count == 0 {
                return Err(format!("the text to replace was not found in '{name}'"));
            }
            if count > 1 {
                return Err(format!(
                    "the text to replace appears {count} times in '{name}'; include more context so it is unique"
                ));
            }
            let text = cur.text.replacen(old, new, 1);
            let c = notes::write(&s, &ws, &floor, &n.id, &text, Some(&cur.revision), &origin)?;
            Ok(
                json!({ "message": format!("Edited '{}'.", c.display_name), "name": c.display_name, "revision": c.revision }),
            )
        }
        "note.create" => {
            let (ws, floor, me) = locate(&ctx.node_id).map_err(|e| e.message)?;
            let text = a["text"].as_str().unwrap_or("");
            let size = omniget_core::core::canvas::model::default_size(&NodeKind::Note);
            let near = omniget_core::core::canvas::model::Point {
                x: me.x - size.w - 40.0,
                y: me.y,
            };
            let spot = s
                .read(&ws, &floor, |d| {
                    omniget_core::core::canvas::layout::free_spot_in(
                        d,
                        size.w,
                        size.h,
                        Some(near),
                        &[],
                    )
                })
                .unwrap_or(near);
            let id = notes::create(&s, &ws, &floor, spot.x, spot.y, text, &origin)?;
            let stack = a
                .get("stack")
                .filter(|v| !v.is_null() && *v != &json!(false));
            let mut ops = if stack.is_some() {
                Vec::new()
            } else {
                vec![Op::AddConnection {
                    connection: Connection {
                        id: omniget_core::core::canvas::model::new_id(),
                        from: ctx.node_id.clone(),
                        to: id.clone(),
                        kind: ConnectionKind::Note,
                        perms: vec![],
                        condition: None,
                        verify: None,
                        label: None,
                        points: vec![],
                        from_note_id: None,
                        to_note_id: None,
                        is_active: None,
                    },
                }]
            };
            let mut shown = notes::derive_name(text);
            if let Some(n) = a["name"].as_str().filter(|n| !n.trim().is_empty()) {
                shown = n.trim().to_string();
                ops.push(Op::update_node(
                    &id,
                    json!({ "title": shown, "data": { "hasCustomName": true } }),
                ));
            }
            if !ops.is_empty() {
                s.apply(&ws, &floor, ops, &origin, None)?;
            }
            if let Some(stack) = stack {
                let stack_name = stack.as_str().map(str::to_string);
                stack_note(
                    &s,
                    &ws,
                    &floor,
                    &ctx.node_id,
                    &id,
                    stack_name.as_deref(),
                    &origin,
                )?;
            }
            Ok(json!({ "message": format!("Created note '{shown}'."), "name": shown, "id": id }))
        }
        "note.stack" => {
            let (ws, floor, n, perms) = find_note(ctx, name)?;
            writable(&perms)?;
            let st = stack_note(
                &s,
                &ws,
                &floor,
                &ctx.node_id,
                &n.id,
                a["stack"].as_str(),
                &origin,
            )?;
            Ok(json!({ "message": format!("Stacked '{name}' into '{st}'."), "stack": st }))
        }
        "note.unstack" => {
            let (ws, floor, n, perms) = find_note(ctx, name)?;
            writable(&perms)?;
            if n.stack_id.is_none() {
                return Err(format!("'{name}' is not in a stack"));
            }
            let (_sid, ops) = s
                .read(&ws, &floor, |d| stack_unfile_ops(d, &n.id, None))
                .map_err(rpc_err)??;
            s.apply(&ws, &floor, ops, &origin, None)?;
            Ok(json!({ "message": format!("Took '{name}' out of its stack.") }))
        }
        "note.delete" => {
            let (ws, floor, n, perms) = find_note(ctx, name)?;
            writable(&perms)?;
            s.apply(
                &ws,
                &floor,
                vec![Op::RemoveNode { id: n.id.clone() }],
                &origin,
                None,
            )?;
            Ok(json!({ "message": format!("Deleted '{name}'.") }))
        }
        other => Err(format!("unknown canvas method `{other}`")),
    }
}

fn stack_note(
    s: &CanvasStore,
    ws: &str,
    floor: &str,
    me: &str,
    note_id: &str,
    stack: Option<&str>,
    origin: &str,
) -> Result<String, String> {
    let (_w, _f, _me, peers) = peers_of(me).map_err(|e| e.message)?;
    let existing = peers
        .iter()
        .filter(|p| p.node.kind == NodeKind::Stack)
        .find(|p| stack.is_none_or(|n| p.name == n || p.name.eq_ignore_ascii_case(n)))
        .map(|p| (p.node.id.clone(), p.name.clone()));
    let on_floor = || -> Option<(String, String)> {
        let n = stack?.trim();
        if n.is_empty() {
            return None;
        }
        s.read(ws, floor, |d| {
            d.nodes
                .iter()
                .filter(|x| x.kind == NodeKind::Stack)
                .find(|x| {
                    let nm = node_name(x);
                    nm == n || nm.eq_ignore_ascii_case(n)
                })
                .map(|x| (x.id.clone(), node_name(x)))
        })
        .ok()
        .flatten()
    };
    let note_cord = |to: &str| Connection {
        id: omniget_core::core::canvas::model::new_id(),
        from: me.to_string(),
        to: to.to_string(),
        kind: ConnectionKind::Note,
        perms: vec![],
        condition: None,
        verify: None,
        label: None,
        points: vec![],
        from_note_id: None,
        to_note_id: None,
        is_active: None,
    };
    let existing = existing
        .map(|(id, name)| (id, name, false))
        .or_else(|| on_floor().map(|(id, name)| (id, name, true)));
    let (sid, sname, mut ops) = match existing {
        Some((id, name, false)) => (id, name, Vec::new()),
        Some((id, name, true)) => {
            let conn = note_cord(&id);
            (id, name, vec![Op::AddConnection { connection: conn }])
        }
        None => {
            let (x, y, z) = s
                .read(ws, floor, |d| {
                    d.node(note_id)
                        .map(|n| (n.x, n.y + n.h + 40.0, d.max_z() + 1.0))
                })?
                .unwrap_or((0.0, 0.0, 1.0));
            let title = s
                .read(ws, floor, |d| d.node(note_id).map(node_name))?
                .unwrap_or_default();
            let node = new_stack_node(x, y, z, stack, Some(&title));
            let id = node.id.clone();
            let sname = node_name(&node);
            let conn = note_cord(&id);
            (
                id,
                sname,
                vec![Op::AddNode { node }, Op::AddConnection { connection: conn }],
            )
        }
    };
    let file = s.read(ws, floor, |d| {
        stack_file_ops(d, &sid, &[note_id.to_string()], None)
    })?;
    ops.extend(file);
    s.apply(ws, floor, ops, origin, None)?;
    Ok(sname)
}

fn stack_tabs(stack: &CanvasNode) -> Vec<String> {
    stack
        .data
        .get("tabs")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn stack_named(stack: &CanvasNode) -> bool {
    stack
        .data
        .get("hasCustomName")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn stack_leave_ops(stack: &CanvasNode, leaving: &[String]) -> Vec<Op> {
    let mut tabs = stack_tabs(stack);
    tabs.retain(|t| !leaving.contains(t));
    if tabs.is_empty() && !stack_named(stack) {
        return vec![Op::RemoveNode {
            id: stack.id.clone(),
        }];
    }
    let front = stack.data_str("frontNoteId").map(str::to_string);
    let front = match front {
        Some(f) if !leaving.contains(&f) => Some(f),
        _ => tabs.first().cloned(),
    };
    vec![Op::update_node(
        &stack.id,
        json!({ "data": { "tabs": tabs, "frontNoteId": front } }),
    )]
}

pub fn stack_file_ops(
    d: &omniget_core::core::canvas::model::FloorDoc,
    stack_id: &str,
    notes: &[String],
    index: Option<usize>,
) -> Vec<Op> {
    let mut tabs = d.node(stack_id).map(stack_tabs).unwrap_or_default();
    tabs.retain(|t| !notes.contains(t));
    let at = index.unwrap_or(0).min(tabs.len());
    for (i, n) in notes.iter().enumerate() {
        tabs.insert(at + i, n.clone());
    }
    let mut old: Vec<(String, Vec<String>)> = Vec::new();
    for nid in notes {
        let Some(sid) = d.node(nid).and_then(|n| n.stack_id.clone()) else {
            continue;
        };
        if sid == stack_id {
            continue;
        }
        match old.iter_mut().find(|(s, _)| s == &sid) {
            Some((_, v)) => v.push(nid.clone()),
            None => old.push((sid, vec![nid.clone()])),
        }
    }
    let mut ops: Vec<Op> = notes
        .iter()
        .map(|n| Op::update_node(n, json!({ "stackId": stack_id })))
        .collect();
    ops.push(Op::update_node(
        stack_id,
        json!({ "data": { "tabs": tabs, "frontNoteId": notes.first() } }),
    ));
    for (sid, ids) in old {
        if let Some(st) = d.node(&sid) {
            ops.extend(stack_leave_ops(st, &ids));
        }
    }
    ops
}

pub fn stack_unfile_ops(
    d: &omniget_core::core::canvas::model::FloorDoc,
    note_id: &str,
    at: Option<(f64, f64)>,
) -> Result<(String, Vec<Op>), String> {
    let note = d
        .node(note_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {note_id}"))?;
    let sid = note
        .stack_id
        .clone()
        .ok_or_else(|| format!("'{}' is not in a stack", node_name(note)))?;
    let (x, y) = match (at, d.node(&sid)) {
        (Some(p), _) => p,
        (None, Some(st)) => (st.x + st.w + 40.0, st.y),
        (None, None) => (note.x, note.y),
    };
    let mut ops = vec![Op::update_node(
        note_id,
        json!({ "stackId": null, "x": x, "y": y, "z": d.max_z() + 1.0 }),
    )];
    if let Some(st) = d.node(&sid) {
        ops.extend(stack_leave_ops(st, &[note_id.to_string()]));
    }
    Ok((sid, ops))
}

pub fn new_stack_node(
    x: f64,
    y: f64,
    z: f64,
    name: Option<&str>,
    title: Option<&str>,
) -> CanvasNode {
    let size = omniget_core::core::canvas::model::default_size(&NodeKind::Stack);
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let data = match name {
        Some(n) => json!({ "name": n, "hasCustomName": true, "tabs": [] }),
        None => json!({ "tabs": [] }),
    };
    CanvasNode {
        id: omniget_core::core::canvas::model::new_id(),
        kind: NodeKind::Stack,
        x,
        y,
        w: size.w,
        h: size.h,
        z,
        title: Some(name.or(title).unwrap_or("Notes").to_string()),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data,
    }
}
