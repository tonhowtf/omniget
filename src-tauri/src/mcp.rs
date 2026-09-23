//! Servidor MCP embutido (estudos 22 e 23): as tools da seção Tools expostas
//! como ferramentas MCP em `POST /mcp` no bridge local, com o mesmo bearer
//! da extensão. Transporte "Streamable HTTP" só com respostas JSON (sem SSE),
//! que é o que Claude Code, Cursor, Goose e o `mcp-remote` do Claude Desktop
//! aceitam. Sem crate de MCP: o protocolo aqui é JSON-RPC com quatro métodos.

use async_trait::async_trait;
use omniget_core::core::llm::tool_table::{self, need_id, need_str, num, s, to_json, HostTools};
use omniget_core::core::tools;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

pub const PROTOCOL: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// A lista de tools vem inteira de `core::llm::tool_table`: a mesma tabela que
/// alimenta o broker do /llm e as concessoes da UI. Aqui so muda o formato do
/// JSON que o protocolo MCP espera (`inputSchema` em camelCase).
pub fn tools() -> Vec<ToolDef> {
    tool_table::table()
        .iter()
        .map(|e| ToolDef {
            name: e.name,
            description: e.description,
            input_schema: e.input_schema.clone(),
        })
        .collect()
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ── Downloads: o engine que ja existe, exposto como tools ──────────────
//
// Nada de capacidade nova aqui: cada arm abaixo chama exatamente o que os
// comandos Tauri de `commands/downloads.rs` chamam.

fn status_key(status: &crate::core::queue::QueueStatus) -> &'static str {
    use crate::core::queue::QueueStatus as S;
    match status {
        S::Queued => "queued",
        S::Active => "active",
        S::Paused => "paused",
        S::Seeding => "seeding",
        S::Complete { .. } => "complete",
        S::Error { .. } => "error",
    }
}

/// As chaves da fila vivem na tabela (o schema de `downloads_queue` sai de la);
/// aqui so reexportamos para o filtro e para os testes nao poderem divergir.
pub(crate) use omniget_core::core::llm::tool_table::QUEUE_STATUS_KEYS;

async fn queue_snapshot(app: &AppHandle) -> Vec<crate::core::queue::QueueItemInfo> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let q = state.download_queue.lock().await;
    q.get_state()
}

async fn queue_item(app: &AppHandle, id: u64) -> Result<crate::core::queue::QueueItemInfo, String> {
    queue_snapshot(app)
        .await
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| format!("no download with id {}", id))
}

/// Fecha a sessao do torrent quando o item cancelado era um magnet.
async fn drop_torrent(app: &AppHandle, torrent_id: Option<usize>, pause_only: bool) {
    use tauri::Manager;
    let Some(tid) = torrent_id else { return };
    let state = app.state::<crate::AppState>();
    let session = state.torrent_session.lock().await;
    let Some(session) = session.as_ref() else {
        return;
    };
    let handle = librqbit::api::TorrentIdOrHash::Id(tid);
    if pause_only {
        if let Some(h) = session.get(handle) {
            let _ = session.pause(&h).await;
        }
    } else {
        let _ = session.delete(handle, false).await;
    }
}

/// Cancelar / pausar / retomar: a mesma sequencia dos comandos Tauri
/// (mexe na fila, emite o estado novo e tenta comecar o proximo).
async fn queue_control(app: &AppHandle, id: u64, action: &str) -> Result<Value, String> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let (changed, torrent_id) = {
        let mut q = state.download_queue.lock().await;
        match action {
            "cancel" => q.cancel(id),
            "pause" => {
                let ok = q.pause(id);
                let tid = q
                    .items
                    .iter()
                    .find(|i| i.id == id)
                    .and_then(|i| i.torrent_id);
                (ok, if ok { tid } else { None })
            }
            _ => (q.resume(id), None),
        }
    };
    if !changed {
        let current = queue_item(app, id)
            .await
            .map(|i| status_key(&i.status).to_string())
            .unwrap_or_else(|e| e);
        return Err(format!("cannot {} download {} ({})", action, id, current));
    }
    if action != "resume" {
        drop_torrent(app, torrent_id, action == "pause").await;
    }
    let snapshot = {
        let q = state.download_queue.lock().await;
        q.get_state()
    };
    crate::core::queue::emit_queue_state_from_state(app, snapshot);
    crate::core::queue::try_start_next(app.clone(), state.download_queue.clone()).await;
    let item = queue_item(app, id).await.ok();
    Ok(json!({ "download_id": id, "action": action, "item": item }))
}

/// A metade "desktop" da tabela: as tools que precisam do `AppHandle` (fila de
/// downloads, sessao de torrent, cookies da extensao). O resto do corpo das
/// tools mora em `core::llm::tool_table` e roda sem o app.
struct AppHost {
    app: AppHandle,
}

#[async_trait]
impl HostTools for AppHost {
    async fn call(&self, name: &str, a: Value) -> Result<Value, String> {
        let app = &self.app;
        match name {
            n if n.starts_with("help_") => crate::commands::llm::help::dispatch(app, n, a).await,
            n if n.starts_with("preview_") => {
                // A thread's agent drives its own thread's Browser preview: a
                // driver session token pins the thread; a native turn runs
                // under its conversation id, which is the thread id.
                let thread = current_caller()
                    .and_then(|c| c.thread_id)
                    .or_else(|| {
                        omniget_core::core::llm::code_tools::current_turn()
                            .map(|t| t.conversation.clone())
                    });
                let mut a = a;
                if let (Some(thread), Some(o)) = (thread, a.as_object_mut()) {
                    o.insert("threadId".into(), Value::String(thread));
                }
                crate::preview::tools::dispatch(app, n, a).await
            }
            "download_url" => {
                let url = s(&a, "url");
                let action =
                    crate::external_url::handle_external_url(app, url.clone(), "mcp").await?;
                Ok(json!({ "url": url, "action": format!("{:?}", action).to_lowercase() }))
            }
            "instagram_profile" => {
                let account = {
                    let v = s(&a, "account");
                    if v.is_empty() {
                        None
                    } else {
                        Some(v)
                    }
                };
                let client = crate::commands::tools::instagram::load_client(account.as_deref())?;
                to_json(
                    tools::instagram::profile::resolve_user(&client, &s(&a, "username"))
                        .await
                        .map_err(err)?,
                )
            }
            "download_enqueue" => {
                let url = need_str(&a, "url")?;
                let mode = match s(&a, "mode").as_str() {
                    "" | "video" => None,
                    "audio" => Some("audio".to_string()),
                    other => {
                        return Err(format!(
                            "argument \"mode\" must be \"video\" or \"audio\", got \"{}\"",
                            other
                        ))
                    }
                };
                // Optional stable intent journal shared by Help and external MCP clients.
                // A crash after enqueue never silently repeats the effect.
                static ENQUEUE_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
                    std::sync::OnceLock::new();
                let _enqueue_guard = ENQUEUE_LOCK
                    .get_or_init(|| tokio::sync::Mutex::new(()))
                    .lock()
                    .await;
                let receipt = if let Some(key) = a["idempotencyKey"].as_str() {
                    if key.is_empty()
                        || key.len() > 100
                        || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    {
                        return Err("ERR_DOWNLOAD_IDEMPOTENCY_KEY".into());
                    }
                    Some(crate::commands::llm::help::folder()?.join(format!("download-{key}.json")))
                } else {
                    None
                };
                if let Some(path) = &receipt {
                    if path.exists() {
                        let record: Value =
                            serde_json::from_slice(&std::fs::read(path).map_err(err)?)
                                .map_err(err)?;
                        if record["url"] != url || record["mode"] != json!(mode) {
                            return Err("ERR_DOWNLOAD_IDEMPOTENCY_CONFLICT".into());
                        }
                        if !record["result"].is_null() {
                            let mut result = record["result"].clone();
                            if let Some(item) = queue_snapshot(app)
                                .await
                                .into_iter()
                                .find(|i| Some(i.id) == result["item"]["id"].as_u64())
                            {
                                result["item"] = serde_json::to_value(item).map_err(err)?;
                                result["historical"] = json!(false);
                            } else {
                                result["historical"] = json!(true);
                                result["outcome"] = json!("recorded");
                                result["note"]=json!("Saved enqueue receipt, not current download status. Check Downloads or recovery; no new download was started.");
                            }
                            return Ok(result);
                        }
                        // URL equality cannot identify this intent: an older completed
                        // download may have the same URL. Never attach an unrelated item.
                        return Err("ERR_DOWNLOAD_OUTCOME_UNKNOWN: interrupted intent; inspect Downloads/recovery before starting another intent".into());
                    }
                    crate::commands::llm::help::write(
                        path,
                        &json!({"url":url,"mode":mode,"result":null}),
                    )?;
                }
                let before: std::collections::HashSet<u64> =
                    queue_snapshot(app).await.iter().map(|i| i.id).collect();
                let outcome = crate::external_url::queue_url_with_defaults(
                    app,
                    url.clone(),
                    false,
                    mode.clone(),
                )
                .await;
                let outcome = match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        if let Some(path) = &receipt {
                            let _ = std::fs::remove_file(path);
                        }
                        return Err(error);
                    }
                };
                let after = queue_snapshot(app).await;
                let item = after
                    .iter()
                    .find(|i| !before.contains(&i.id) && i.url == url)
                    .or_else(|| after.iter().find(|i| i.url == url));
                let outcome = match outcome {
                    crate::external_url::QueueUrlOutcome::Queued => "queued",
                    crate::external_url::QueueUrlOutcome::AlreadyQueued => "already-queued",
                };
                let result = json!({ "url": url, "outcome": outcome, "item": item });
                if let Some(path) = receipt {
                    crate::commands::llm::help::write(
                        &path,
                        &json!({"url":url,"mode":mode,"result":result}),
                    )?;
                }
                Ok(result)
            }
            "downloads_queue" => {
                let want = s(&a, "status").trim().to_lowercase();
                if !want.is_empty() && !QUEUE_STATUS_KEYS.contains(&want.as_str()) {
                    return Err(format!(
                        "argument \"status\" must be one of {}, got \"{}\"",
                        QUEUE_STATUS_KEYS.join(", "),
                        want
                    ));
                }
                let limit = num(&a, "limit").unwrap_or(50).max(1) as usize;
                let items = queue_snapshot(app).await;
                let mut by_status: std::collections::BTreeMap<&str, u64> = Default::default();
                for i in &items {
                    *by_status.entry(status_key(&i.status)).or_insert(0) += 1;
                }
                let total = items.len();
                let matched: Vec<_> = items
                    .into_iter()
                    .filter(|i| want.is_empty() || status_key(&i.status) == want)
                    .collect();
                let shown = matched.len().min(limit);
                Ok(
                    json!({ "total": total, "matched": matched.len(), "shown": shown, "by_status": by_status, "items": matched.into_iter().take(limit).collect::<Vec<_>>() }),
                )
            }
            "download_status" => {
                let id = need_id(&a, "download_id")?;
                let item = queue_item(app, id).await?;
                let command = omniget_core::core::ytdlp::get_command(id);
                let log = crate::core::download_log::get(id);
                let log: Vec<String> = log.into_iter().rev().take(20).rev().collect();
                to_json(json!({ "item": item, "command": command, "log": log }))
            }
            "download_cancel" => queue_control(app, need_id(&a, "download_id")?, "cancel").await,
            "download_pause" => queue_control(app, need_id(&a, "download_id")?, "pause").await,
            "download_resume" => queue_control(app, need_id(&a, "download_id")?, "resume").await,
            "agent_delegate" => agent_delegate(app, &a).await,
            "catalog_search" | "catalog_get" | "catalog_plan_install" => {
                catalog_tool(name, a).await
            }
            "catalog_install" => catalog_install(app, &a).await,
            _ => Err(format!("unknown tool: {}", name)),
        }
    }
}

/// Sub-agente: um turno inteiro de outro agente do roster numa conversa filha,
/// no mesmo workspace. A resposta final volta como resultado da tool.
async fn agent_delegate(app: &AppHandle, a: &Value) -> Result<Value, String> {
    use futures::StreamExt;
    use omniget_core::core::llm::code_tools;
    use omniget_core::core::llm::types::TurnEvent;
    use tauri::Manager;

    let agent_id = need_str(a, "agent_id")?;
    let task = need_str(a, "task")?;
    let state = app.state::<crate::AppState>();
    let manager = state.llm.clone();
    let to = manager.agent(&agent_id).ok_or_else(|| {
        let ids: Vec<String> = manager.roster().into_iter().map(|r| r.id).collect();
        format!("no agent `{agent_id}`; roster: {}", ids.join(", "))
    })?;
    let parent = code_tools::current_turn();
    let parent_conv = parent
        .as_ref()
        .map(|c| c.conversation.clone())
        .unwrap_or_else(|| "mcp".into());
    if parent_conv.matches("-sub-").count() >= 2 {
        return Err("delegation is limited to two levels".into());
    }
    if parent
        .as_ref()
        .map(|c| c.agent == agent_id)
        .unwrap_or(false)
    {
        return Err("an agent cannot delegate to itself".into());
    }
    let child = format!(
        "{parent_conv}-sub-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    if let Some(ws) = code_tools::workspace() {
        let _ = code_tools::set_conversation_workspace(&child, Some(ws));
    }
    if let Some(from) = parent.as_ref().and_then(|c| manager.agent(&c.agent)) {
        let _ = manager.coordinator().handoff(&child, &from, &to, &task);
    }
    let (request_id, _cancel, mut stream) = manager.turn_stream(&child, &agent_id, &task).await?;
    let mut text = String::new();
    let mut error = None;
    while let Some(event) = stream.next().await {
        manager.note_event(&agent_id, &event);
        match event {
            TurnEvent::TextDelta { text: t } => text.push_str(&t),
            TurnEvent::Error { error: e } => error = Some(format!("{}: {}", e.code, e.message)),
            _ => {}
        }
    }
    manager.finish_turn(&request_id, &agent_id);
    match (text.trim().is_empty(), error) {
        (true, Some(e)) => Err(e),
        _ => Ok(json!({ "agent": agent_id, "conversation_id": child, "answer": text })),
    }
}

/// Uma chamada de tool. A tabela decide quem roda: o core, ou o `AppHost`
/// acima quando a tool precisa do app.
pub async fn call(app: &AppHandle, name: &str, a: Value) -> Result<Value, String> {
    let host = AppHost { app: app.clone() };
    tool_table::dispatch(name, a, Some(&host)).await
}

/// A call from the MCP endpoint: scope check, caller context for the host
/// tools (the owner ask names the thread) and one line in the call log.
/// Without an app (tests) the host tools fail instead of pretending.
async fn call_as(
    app: Option<&AppHandle>,
    caller: &Caller,
    name: &str,
    a: Value,
) -> Result<Value, String> {
    if !caller.allows(name) {
        let e = format!("ERR_MCP_SCOPE: tool `{name}` is not allowed for this session");
        log_call(caller, name, false, 0, Some(&e));
        return Err(e);
    }
    let started = std::time::Instant::now();
    let out = CALLER
        .scope(caller.clone(), async {
            match app {
                Some(app) => call(app, name, a).await,
                None => tool_table::dispatch(name, a, None).await,
            }
        })
        .await;
    log_call(
        caller,
        name,
        out.is_ok(),
        started.elapsed().as_millis() as u64,
        out.as_ref().err().map(String::as_str),
    );
    out
}

// ── Estado (ligado/desligado) ──────────────────────────────────────────

fn config_file() -> Option<std::path::PathBuf> {
    tools::tools_dir().map(|d| d.join("mcp.json"))
}

pub fn enabled() -> bool {
    config_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let p = config_file().ok_or("no data dir")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(err)?;
    std::fs::write(
        &p,
        serde_json::to_string_pretty(&json!({ "enabled": on })).unwrap(),
    )
    .map_err(err)
}

// ── Credenciais por sessão de driver (plan §3.2 "MCP de volta") ─────────
//
// O bearer global (o da extensão) continua valendo para quem o usuário
// configurou à mão. Cada sessão de driver da Central (Claude, Codex, ACP,
// OpenCode) ganha o seu próprio token: gerado pelo host quando a sessão
// abre, só o sha256 fica em memória, vale 24 h, cai quando a sessão para e
// carrega o escopo de tools e a thread que o log registra.

/// How long a session token lives.
pub const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// Tokens kept per thread (a relaunched process mints a new one; the oldest go).
const TOKENS_PER_THREAD: usize = 4;
/// Name of the embedded server in every driver config.
pub const SERVER_NAME: &str = "omniget";
/// Env var that carries the session token to Codex (`bearer_token_env_var`).
pub const TOKEN_ENV: &str = "OMNIGET_MCP_TOKEN";

/// Tools a driver session does not get by default: the coding harness (the
/// CLI has its own, confined to its own workspace), delegating to the paid
/// roster and saving agents of the app.
pub const SESSION_EXCLUDED: &[&str] = &[
    "fs_read",
    "fs_list",
    "fs_glob",
    "fs_grep",
    "fs_edit",
    "fs_write",
    "fs_apply_patch",
    "shell_exec",
    "todo_write",
    "agent_delegate",
    "help_agent_plan",
    "help_agent_apply",
];

/// The default scope of a driver session: the table minus [`SESSION_EXCLUDED`].
pub fn default_session_scope() -> Vec<String> {
    tool_table::names()
        .into_iter()
        .filter(|n| !SESSION_EXCLUDED.contains(n))
        .map(String::from)
        .collect()
}

/// Who is calling `/mcp`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Caller {
    /// `global` (the extension bearer) or `session` (a driver session token).
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// `None` = every tool.
    #[serde(skip)]
    pub scope: Option<std::sync::Arc<std::collections::BTreeSet<String>>>,
}

impl Caller {
    pub fn global() -> Caller {
        Caller {
            kind: "global",
            thread_id: None,
            instance_id: None,
            scope: None,
        }
    }

    pub fn allows(&self, tool: &str) -> bool {
        self.scope
            .as_ref()
            .map(|s| s.contains(tool))
            .unwrap_or(true)
    }
}

tokio::task_local! {
    /// The caller of the MCP call running on this task (the host tools read it).
    static CALLER: Caller;
}

fn current_caller() -> Option<Caller> {
    CALLER.try_with(|c| c.clone()).ok()
}

struct SessionEntry {
    thread_id: String,
    instance_id: String,
    scope: std::sync::Arc<std::collections::BTreeSet<String>>,
    minted: std::time::Instant,
    expires: std::time::Instant,
}

type TokenHash = [u8; 32];

fn sessions() -> &'static std::sync::Mutex<std::collections::HashMap<TokenHash, SessionEntry>> {
    static S: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<TokenHash, SessionEntry>>,
    > = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

fn hash_token(token: &str) -> TokenHash {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes()).into()
}

/// Mints a token for one driver session of `thread_id`. `scope: None` = the
/// default session scope. Only the hash is kept.
pub fn mint_session_token(
    thread_id: &str,
    instance_id: &str,
    scope: Option<Vec<String>>,
) -> String {
    let token = format!("omg_s_{}", crate::local_bridge::generate_token());
    let now = std::time::Instant::now();
    let scope: std::collections::BTreeSet<String> = scope
        .unwrap_or_else(default_session_scope)
        .into_iter()
        .collect();
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, e| e.expires > now);
    let mut mine: Vec<(TokenHash, std::time::Instant)> = map
        .iter()
        .filter(|(_, e)| e.thread_id == thread_id)
        .map(|(h, e)| (*h, e.minted))
        .collect();
    mine.sort_by_key(|(_, at)| *at);
    while mine.len() >= TOKENS_PER_THREAD {
        let (h, _) = mine.remove(0);
        map.remove(&h);
    }
    map.insert(
        hash_token(&token),
        SessionEntry {
            thread_id: thread_id.to_string(),
            instance_id: instance_id.to_string(),
            scope: std::sync::Arc::new(scope),
            minted: now,
            expires: now + SESSION_TTL,
        },
    );
    token
}

/// Revokes every token of a thread (session stopped, thread deleted, MCP
/// switched off for it). Returns how many went.
pub fn revoke_thread(thread_id: &str) -> usize {
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let before = map.len();
    map.retain(|_, e| e.thread_id != thread_id);
    before - map.len()
}

/// Revokes one token.
pub fn revoke_token(token: &str) -> bool {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&hash_token(token))
        .is_some()
}

/// The caller behind a session token, if it is known and not expired.
pub fn session_caller(token: &str) -> Option<Caller> {
    let now = std::time::Instant::now();
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let key = hash_token(token);
    match map.get(&key) {
        Some(e) if e.expires > now => Some(Caller {
            kind: "session",
            thread_id: Some(e.thread_id.clone()),
            instance_id: Some(e.instance_id.clone()),
            scope: Some(e.scope.clone()),
        }),
        Some(_) => {
            map.remove(&key);
            None
        }
        None => None,
    }
}

/// Live session tokens per thread (counts only; never the token).
pub fn session_count(thread_id: &str) -> usize {
    let now = std::time::Instant::now();
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|e| e.thread_id == thread_id && e.expires > now)
        .count()
}

/// Authorization of `POST /mcp`. The global bearer needs the MCP server
/// switched on (Tools → MCP server); a session token does not, since the
/// host minted it for a Central thread whose MCP is on.
pub enum Auth {
    Ok(Caller),
    Unauthorized,
    Disabled,
}

pub fn authorize(authorization: Option<&str>, global_token: &str) -> Auth {
    let Some(raw) = authorization else {
        return Auth::Unauthorized;
    };
    let Some(provided) = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
    else {
        return Auth::Unauthorized;
    };
    if !global_token.is_empty()
        && constant_time_eq::constant_time_eq(provided.as_bytes(), global_token.as_bytes())
    {
        return if enabled() {
            Auth::Ok(Caller::global())
        } else {
            Auth::Disabled
        };
    }
    match session_caller(provided) {
        Some(c) => Auth::Ok(c),
        None => Auth::Unauthorized,
    }
}

/// `tools/list` for one caller: the table filtered by its scope.
pub fn tools_for(caller: &Caller) -> Vec<ToolDef> {
    tools()
        .into_iter()
        .filter(|t| caller.allows(t.name))
        .collect()
}

// ── Log das chamadas ───────────────────────────────────────────────────

const LOG_CAP: usize = 200;

fn call_log() -> &'static std::sync::Mutex<std::collections::VecDeque<Value>> {
    static L: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<Value>>> =
        std::sync::OnceLock::new();
    L.get_or_init(Default::default)
}

fn log_call(caller: &Caller, tool: &str, ok: bool, ms: u64, error: Option<&str>) {
    tracing::info!(
        "[mcp] {tool} caller={} thread={} ok={ok} {ms}ms",
        caller.kind,
        caller.thread_id.as_deref().unwrap_or("-")
    );
    let mut log = call_log().lock().unwrap_or_else(|e| e.into_inner());
    if log.len() >= LOG_CAP {
        log.pop_front();
    }
    log.push_back(json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "tool": tool,
        "caller": caller.kind,
        "threadId": caller.thread_id,
        "instanceId": caller.instance_id,
        "ok": ok,
        "ms": ms,
        "error": error.map(|e| e.chars().take(300).collect::<String>()),
    }));
}

/// The last MCP calls, newest last, optionally of one thread.
pub fn recent_calls(thread_id: Option<&str>) -> Vec<Value> {
    call_log()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|v| thread_id.map(|t| v["threadId"] == t).unwrap_or(true))
        .cloned()
        .collect()
}

// ── Opção por thread (ligado por padrão) ───────────────────────────────

fn threads_file() -> Option<std::path::PathBuf> {
    omniget_core::core::llm::roster_store::llm_dir().map(|d| d.join("mcp-threads.json"))
}

fn read_disabled(p: Option<&std::path::Path>) -> std::collections::BTreeSet<String> {
    p.and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("disabled").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn write_disabled(p: &std::path::Path, thread_id: &str, on: bool) -> Result<(), String> {
    let mut set = read_disabled(Some(p));
    if on {
        set.remove(thread_id);
    } else {
        set.insert(thread_id.to_string());
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("ERR_MCP_IO: {e}"))?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&json!({ "disabled": set })).unwrap_or_default(),
    )
    .map_err(|e| format!("ERR_MCP_IO: {e}"))?;
    std::fs::rename(&tmp, p).map_err(|e| format!("ERR_MCP_IO: {e}"))
}

pub fn thread_enabled(thread_id: &str) -> bool {
    !read_disabled(threads_file().as_deref()).contains(thread_id)
}

/// Turns the embedded MCP on/off for one thread. Off revokes its tokens at
/// once; either way the next session start follows the new value.
pub fn set_thread_enabled(thread_id: &str, on: bool) -> Result<bool, String> {
    let p = threads_file().ok_or("ERR_MCP_IO: no data dir")?;
    write_disabled(&p, thread_id, on)?;
    if !on {
        revoke_thread(thread_id);
    }
    Ok(on)
}

/// What the drivers inject for one session (`acp::set_mcp_provider`, also
/// read by the Claude and Codex launchers): the embedded server with a fresh
/// session token, or nothing when the bridge is off or the thread opted out.
pub fn driver_servers(
    app: &AppHandle,
    thread_id: &str,
    instance_id: &str,
) -> Vec<omniget_core::core::llm::drivers::acp::McpServerSpec> {
    if thread_id.is_empty() || !thread_enabled(thread_id) {
        return vec![];
    }
    let settings = crate::storage::config::load_settings(app);
    if !settings.bridge.enabled || settings.bridge.port == 0 {
        return vec![];
    }
    let token = mint_session_token(thread_id, instance_id, None);
    vec![omniget_core::core::llm::drivers::acp::McpServerSpec::Http {
        name: SERVER_NAME.into(),
        url: format!("http://127.0.0.1:{}/mcp", settings.bridge.port),
        headers: vec![("Authorization".into(), format!("Bearer {token}"))],
    }]
}

#[tauri::command]
pub fn mcp_thread_get(thread_id: String) -> Value {
    json!({
        "threadId": thread_id,
        "enabled": thread_enabled(&thread_id),
        "sessions": session_count(&thread_id),
        "calls": recent_calls(Some(&thread_id)).len(),
    })
}

#[tauri::command]
pub fn mcp_thread_set(thread_id: String, enabled: bool) -> Result<bool, String> {
    set_thread_enabled(&thread_id, enabled)
}

#[tauri::command]
pub fn mcp_call_log(thread_id: Option<String>) -> Vec<Value> {
    recent_calls(thread_id.as_deref())
}

// ── Catálogo no MCP (plan §6 linha 51) ─────────────────────────────────

/// Keeps a plan readable for a model: no file bodies, diffs clipped.
fn compact_plan(plan: Value) -> Value {
    const PER_FILE: usize = 16 * 1024;
    const TOTAL: usize = 96 * 1024;
    let mut plan = plan;
    if let Some(units) = plan.get_mut("units").and_then(Value::as_array_mut) {
        for u in units {
            if let Some(o) = u.as_object_mut() {
                o.remove("files");
            }
        }
    }
    let mut used = 0usize;
    if let Some(files) = plan.get_mut("files").and_then(Value::as_array_mut) {
        for f in files {
            let Some(d) = f.get("diff").and_then(Value::as_str) else {
                continue;
            };
            let room = PER_FILE.min(TOTAL.saturating_sub(used));
            if d.len() > room {
                let mut cut = room;
                while cut > 0 && !d.is_char_boundary(cut) {
                    cut -= 1;
                }
                let clipped = format!("{}\n… (diff clipped, {} bytes)", &d[..cut], d.len());
                used += cut;
                f["diff"] = Value::String(clipped);
                f["diffClipped"] = json!(true);
            } else {
                used += d.len();
            }
        }
    }
    let id = plan.get("id").cloned().unwrap_or(Value::Null);
    if let Some(o) = plan.as_object_mut() {
        o.insert("plan_id".into(), id);
        o.insert(
            "next".into(),
            json!("call catalog_install with this plan_id; the owner is asked and sees this diff"),
        );
    }
    plan
}

fn agentkit_body(name: &str, a: &Value) -> Value {
    let pick = |keys: &[&str]| -> Value {
        let mut o = serde_json::Map::new();
        for k in keys {
            if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
                o.insert((*k).to_string(), v.clone());
            }
        }
        Value::Object(o)
    };
    match name {
        "catalog_search" => {
            let mut b = pick(&["query", "kinds", "categories", "offset"]);
            b["limit"] = json!(num(a, "limit").unwrap_or(10).clamp(1, 50));
            b
        }
        "catalog_get" => pick(&["id", "project"]),
        _ => pick(&["ids", "targets", "scope", "project", "policy"]),
    }
}

/// `catalog_search`, `catalog_get`, `catalog_plan_install`: the same code the
/// `omniget agentkit` bridge runs (catalog downloads block, so off the runtime).
async fn catalog_tool(name: &str, a: Value) -> Result<Value, String> {
    if name == "catalog_get" {
        need_str(&a, "id")?;
    }
    if name == "catalog_plan_install" && tool_table::list(&a, "ids").is_empty() {
        return Err("argument \"ids\" is required (array of catalog ids)".into());
    }
    let action = match name {
        "catalog_search" => "search",
        "catalog_get" => "show",
        _ => "plan",
    };
    let body = agentkit_body(name, &a);
    let out = tauri::async_runtime::spawn_blocking(move || {
        crate::local_bridge_agentkit::run(action, &body)
    })
    .await
    .map_err(err)??;
    Ok(if action == "plan" {
        compact_plan(out)
    } else {
        out
    })
}

/// What the owner reads before saying yes: files, targets, commands, diff.
fn install_preview(
    plan: &omniget_core::core::agentkit::plan::InstallPlan,
    reason: &str,
) -> (String, String, Vec<String>, Vec<String>) {
    let mut text = String::new();
    let changed: Vec<_> = plan
        .files
        .iter()
        .filter(|f| f.action != "unchanged")
        .collect();
    let targets: std::collections::BTreeSet<&str> =
        plan.units.iter().map(|u| u.target.as_str()).collect();
    let comps: std::collections::BTreeSet<String> =
        plan.units.iter().map(|u| u.component.id.clone()).collect();
    text.push_str(&format!(
        "Install {} into {} ({} file(s), scope {})\n",
        comps.iter().cloned().collect::<Vec<_>>().join(", "),
        targets.iter().copied().collect::<Vec<_>>().join(", "),
        changed.len(),
        plan.scope.as_str()
    ));
    if !reason.trim().is_empty() {
        text.push_str(&format!("Why: {}\n", reason.trim()));
    }
    let mut commands: Vec<String> = Vec::new();
    let mut paths = Vec::new();
    for f in &changed {
        text.push_str(&format!("  {} {}\n", f.action, f.path.display()));
        paths.push(f.path.display().to_string());
        for c in &f.commands {
            if !commands.contains(c) {
                commands.push(c.clone());
            }
        }
    }
    for u in &plan.units {
        for c in &u.commands {
            if !commands.contains(c) {
                commands.push(c.clone());
            }
        }
    }
    if !commands.is_empty() {
        text.push_str("Commands it makes a tool run:\n");
        for c in &commands {
            text.push_str(&format!("  $ {c}\n"));
        }
    }
    for w in &plan.warnings {
        text.push_str(&format!("! {w}\n"));
    }
    let mut diff = String::new();
    for f in &changed {
        if diff.len() > 256 * 1024 {
            diff.push_str("… (more files)\n");
            break;
        }
        diff.push_str(&f.diff);
        if !f.diff.ends_with('\n') {
            diff.push('\n');
        }
    }
    (text, diff, paths, commands)
}

/// `catalog_install`: always asks the owner (whatever the caller's grant),
/// through the broker's ToolAsk (pet, chat, observatory) and, for a Central
/// thread, as an approval card in that thread too.
async fn catalog_install(app: &AppHandle, a: &Value) -> Result<Value, String> {
    use omniget_core::core::agentkit as ak;
    let plan_id = need_str(a, "plan_id")?;
    let plan = ak::plan::get(&plan_id).ok_or_else(|| {
        format!("ERR_CATALOG_PLAN: plan `{plan_id}` not found or already used (call catalog_plan_install again)")
    })?;
    if !plan.has_changes() {
        return Ok(
            json!({ "plan_id": plan_id, "status": "unchanged", "message": "nothing to write: already installed" }),
        );
    }
    let (preview, diff, paths, commands) = install_preview(&plan, &s(a, "reason"));
    let allowed = owner_ask(app, "catalog_install", &preview, &diff, paths, commands).await;
    if !allowed {
        return Err(
            "ERR_TOOL_DENIED: the owner refused catalog_install (nothing was written)".into(),
        );
    }
    let body = json!({ "plan_id": plan_id });
    let report = tauri::async_runtime::spawn_blocking(move || {
        crate::local_bridge_agentkit::run("apply", &body)
    })
    .await
    .map_err(err)??;
    Ok(json!({ "plan_id": plan_id, "status": "installed", "report": report }))
}

/// Asks the owner and waits (the broker's timeout applies to asks outside a
/// durable thread turn). True = allowed.
async fn owner_ask(
    app: &AppHandle,
    label: &str,
    preview: &str,
    diff: &str,
    paths: Vec<String>,
    commands: Vec<String>,
) -> bool {
    use omniget_core::core::llm::broker::Answer;
    use tauri::Manager;
    let caller = current_caller();
    let turn = omniget_core::core::llm::code_tools::current_turn();
    let (agent, request_id) = match (&caller, &turn) {
        (Some(c), _) if c.thread_id.is_some() => (
            c.instance_id.clone().unwrap_or_else(|| "mcp".into()),
            c.thread_id.clone().unwrap_or_default(),
        ),
        (_, Some(t)) => (t.agent.clone(), t.request.clone()),
        _ => ("mcp".to_string(), "mcp".to_string()),
    };
    let call_id = format!(
        "{}{}",
        crate::threads_host::MCP_ASK_PREFIX,
        &uuid::Uuid::new_v4().simple().to_string()[..16]
    );
    let thread = caller.as_ref().and_then(|c| c.thread_id.clone());
    if let Some(thread) = &thread {
        crate::threads_host::mcp_ask_opened(
            app, thread, &call_id, label, preview, diff, paths, commands,
        );
    }
    let broker = app.state::<crate::AppState>().llm.broker();
    let answer = broker
        .ask_user(&agent, &request_id, &call_id, label, preview)
        .await;
    let allowed = !matches!(answer, Answer::Deny);
    if let Some(thread) = &thread {
        crate::threads_host::mcp_ask_resolved(app, thread, &call_id, allowed);
    }
    allowed
}

// ── JSON-RPC ───────────────────────────────────────────────────────────

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Trata uma mensagem. `None` = notificação (sem resposta).
pub async fn handle(app: Option<&AppHandle>, caller: &Caller, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    if method.starts_with("notifications/") {
        return None;
    }
    let id = id?;
    Some(match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or(PROTOCOL);
            let version = if matches!(requested, "2024-11-05" | "2025-03-26" | "2025-06-18") {
                requested
            } else {
                PROTOCOL
            };
            rpc_ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "OmniGet", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "OmniGet desktop tools: downloads, PDF, speech, images, files, X/Twitter, Instagram, system, and the agent catalog (catalog_search → catalog_plan_install → catalog_install, which the owner approves). Paths are local to this machine."
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tools_for(caller) })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_as(app, caller, name, args).await {
                Ok(v) => {
                    let text = if let Some(t) = v.as_str() {
                        t.to_string()
                    } else {
                        serde_json::to_string_pretty(&v).unwrap_or_default()
                    };
                    rpc_ok(
                        id,
                        json!({ "content": [{ "type": "text", "text": text }], "structuredContent": v, "isError": false }),
                    )
                }
                Err(e) => rpc_ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                ),
            }
        }
        "resources/list" => rpc_ok(id, json!({ "resources": [] })),
        "prompts/list" => rpc_ok(id, json!({ "prompts": [] })),
        _ => rpc_error(id, -32601, format!("method not found: {}", method)),
    })
}

/// Corpo inteiro (mensagem única ou lote) → resposta pronta para o HTTP.
pub async fn handle_body(app: Option<&AppHandle>, caller: &Caller, body: &Value) -> Option<Value> {
    match body {
        Value::Array(items) => {
            let mut out = Vec::new();
            for m in items {
                if let Some(r) = handle(app, caller, m).await {
                    out.push(r);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Array(out))
            }
        }
        m => handle(app, caller, m).await,
    }
}

/// Trechos de configuração para os clientes, com a URL e o token deste app.
pub fn client_snippets(url: &str, token: &str) -> Vec<(String, String)> {
    let auth = format!("Authorization: Bearer {}", token);
    vec![
        ("Claude Code".into(), format!("claude mcp add --transport http omniget {} --header \"{}\"", url, auth)),
        (
            "Cursor".into(),
            serde_json::to_string_pretty(&json!({ "mcpServers": { "omniget": { "url": url, "headers": { "Authorization": format!("Bearer {}", token) } } } })).unwrap_or_default(),
        ),
        (
            "VS Code".into(),
            serde_json::to_string_pretty(&json!({ "servers": { "omniget": { "type": "http", "url": url, "headers": { "Authorization": format!("Bearer {}", token) } } } })).unwrap_or_default(),
        ),
        (
            "Goose".into(),
            format!("extensions:\n  omniget:\n    enabled: true\n    type: streamable_http\n    name: omniget\n    uri: {}\n    headers:\n      Authorization: Bearer {}\n    timeout: 300", url, token),
        ),
        (
            "Claude Desktop".into(),
            serde_json::to_string_pretty(&json!({ "mcpServers": { "omniget": { "command": "npx", "args": ["-y", "mcp-remote", url, "--header", auth] } } })).unwrap_or_default(),
        ),
        (
            "Codex".into(),
            format!("[mcp_servers.omniget]\nurl = \"{}\"\nhttp_headers = {{ Authorization = \"Bearer {}\" }}", url, token),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_objects() {
        let list = tools();
        assert!(list.len() > 20);
        for t in &list {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert!(!t.description.is_empty());
            // Todo campo obrigatorio precisa existir em `properties`.
            let props = t.input_schema["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{} sem properties", t.name));
            let required = t.input_schema["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{} sem required", t.name));
            for r in required {
                let key = r.as_str().unwrap_or_default();
                assert!(props.contains_key(key), "{}: required {:?}", t.name, key);
            }
        }
        let names: std::collections::HashSet<_> = list.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), list.len(), "nomes repetidos");

        // Controle da fila de downloads (mcp-downloads-control).
        for name in [
            "download_enqueue",
            "downloads_queue",
            "download_status",
            "download_cancel",
            "download_pause",
            "download_resume",
        ] {
            assert!(names.contains(name), "faltou a tool {}", name);
        }
        let by_name = |n: &str| {
            list.iter()
                .find(|t| t.name == n)
                .unwrap_or_else(|| panic!("sem {}", n))
        };
        assert_eq!(
            by_name("download_enqueue").input_schema["required"],
            json!(["url"])
        );
        assert_eq!(
            by_name("download_enqueue").input_schema["properties"]["mode"]["enum"],
            json!(["video", "audio"])
        );
        // Listar a fila nao exige argumento nenhum.
        assert_eq!(
            by_name("downloads_queue").input_schema["required"],
            json!([])
        );
        for n in [
            "download_status",
            "download_cancel",
            "download_pause",
            "download_resume",
        ] {
            assert_eq!(
                by_name(n).input_schema["required"],
                json!(["download_id"]),
                "{}",
                n
            );
            assert_eq!(
                by_name(n).input_schema["properties"]["download_id"]["type"],
                "integer",
                "{}",
                n
            );
        }
        // O filtro de status da lista tem que casar com o que `status_key` devolve.
        let allowed = by_name("downloads_queue").input_schema["properties"]["status"]["enum"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let allowed: Vec<String> = allowed
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        assert_eq!(allowed, QUEUE_STATUS_KEYS.to_vec());
    }

    #[test]
    fn argumento_errado_da_erro_legivel() {
        let a = json!({ "download_id": "42", "url": "  " });
        assert_eq!(need_id(&a, "download_id"), Ok(42));
        let Err(e) = need_id(&json!({}), "download_id") else {
            panic!("id faltando deveria falhar");
        };
        assert!(e.contains("download_id") && e.contains("integer"), "{}", e);
        let Err(e) = need_str(&a, "url") else {
            panic!("url em branco deveria falhar");
        };
        assert!(e.contains("url") && e.contains("required"), "{}", e);
        let Err(e) = need_str(&json!({}), "query") else {
            panic!("query faltando deveria falhar");
        };
        assert!(e.contains("query"), "{}", e);
    }

    #[test]
    fn status_da_fila_vira_chave_estavel() {
        use crate::core::queue::QueueStatus as S;
        assert_eq!(status_key(&S::Queued), "queued");
        assert_eq!(status_key(&S::Active), "active");
        assert_eq!(status_key(&S::Paused), "paused");
        assert_eq!(status_key(&S::Seeding), "seeding");
        assert_eq!(status_key(&S::Complete { success: true }), "complete");
        assert_eq!(status_key(&S::Complete { success: false }), "complete");
        assert_eq!(
            status_key(&S::Error {
                message: "Cancelled".into(),
                retryable: false
            }),
            "error"
        );
    }

    /// O servidor MCP nao tem mais lista propria: `tools()` e a tabela do core,
    /// nome por nome, na mesma ordem, com o mesmo schema. Se alguem acrescentar
    /// uma tool so aqui (ou so la), este teste cai.
    #[test]
    fn a_lista_do_servidor_e_a_tabela_do_core() {
        let table = omniget_core::core::llm::tool_table::table();
        let list = tools();
        assert_eq!(list.len(), 67, "a tabela mudou de tamanho");
        assert_eq!(list.len(), table.len());
        for (def, entry) in list.iter().zip(table) {
            assert_eq!(def.name, entry.name);
            assert_eq!(def.description, entry.description);
            assert_eq!(def.input_schema, entry.input_schema);
        }
        // O JSON do protocolo usa `inputSchema`, nao `input_schema`.
        let wire = serde_json::to_value(&list[0]).unwrap();
        assert!(wire.get("inputSchema").is_some(), "{}", wire);

        // Toda tool que precisa do app tem arm no `AppHost`; nenhuma outra tem.
        let host: Vec<&str> = table
            .iter()
            .filter(|e| e.needs_host())
            .map(|e| e.name)
            .collect();
        assert_eq!(host.len(), 27, "{:?}", host);
    }

    #[test]
    fn session_tokens_scope_expire_and_revoke() {
        let t = mint_session_token("thr_scope", "codex-a", None);
        let c = session_caller(&t).expect("fresh token");
        assert_eq!(c.kind, "session");
        assert_eq!(c.thread_id.as_deref(), Some("thr_scope"));
        assert!(c.allows("catalog_search") && c.allows("catalog_install"));
        assert!(!c.allows("fs_read") && !c.allows("agent_delegate"));
        let listed: Vec<&str> = tools_for(&c).iter().map(|t| t.name).collect();
        assert!(listed.contains(&"downloads_queue") && !listed.contains(&"shell_exec"));
        // Only the hash is kept: another string never matches.
        assert!(session_caller(&format!("{t}x")).is_none());
        // A relaunched process mints again; the oldest beyond the cap go.
        let later: Vec<String> = (0..TOKENS_PER_THREAD)
            .map(|_| mint_session_token("thr_scope", "codex-a", None))
            .collect();
        assert!(session_caller(&t).is_none(), "oldest token dropped");
        assert_eq!(session_count("thr_scope"), TOKENS_PER_THREAD);
        assert_eq!(revoke_thread("thr_scope"), TOKENS_PER_THREAD);
        assert!(later.iter().all(|t| session_caller(t).is_none()));
        // Explicit scope.
        let t = mint_session_token("thr_one", "i", Some(vec!["catalog_get".into()]));
        let c = session_caller(&t).unwrap();
        assert!(c.allows("catalog_get") && !c.allows("catalog_search"));
        assert!(revoke_token(&t));
        assert!(matches!(
            authorize(Some(&format!("Bearer {t}")), "g"),
            Auth::Unauthorized
        ));
        assert!(matches!(authorize(None, "g"), Auth::Unauthorized));
    }

    /// A real HTTP round trip through the same `authorize` + `handle_body`
    /// the bridge's `POST /mcp` runs (no `AppHandle`: host tools refuse).
    /// With `OMNIGET_MCP_SMOKE_NODE=1` it also drives
    /// `scripts/mcp-client-smoke.mjs` as the outside client.
    #[tokio::test]
    async fn session_token_over_http() {
        use axum::http::{HeaderMap, StatusCode};
        use axum::response::IntoResponse;
        let global = std::sync::Arc::new("global-bridge-token".to_string());
        let router = axum::Router::new().route(
            "/mcp",
            axum::routing::post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let global = global.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    let caller = match authorize(auth, &global) {
                        Auth::Ok(c) => c,
                        Auth::Unauthorized => return StatusCode::UNAUTHORIZED.into_response(),
                        Auth::Disabled => return StatusCode::FORBIDDEN.into_response(),
                    };
                    let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    match handle_body(None, &caller, &msg).await {
                        Some(r) => axum::Json(r).into_response(),
                        None => StatusCode::ACCEPTED.into_response(),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let token = mint_session_token("thr_http", "claude-main", None);
        let client = reqwest::Client::new();
        let rpc = |tok: String, method: &'static str, params: Value| {
            let (client, url) = (client.clone(), url.clone());
            async move {
                client
                    .post(&url)
                    .bearer_auth(tok)
                    .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
                    .send()
                    .await
                    .unwrap()
            }
        };
        let init: Value = rpc(token.clone(), "initialize", json!({}))
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "OmniGet");
        let list: Value = rpc(token.clone(), "tools/list", json!({}))
            .await
            .json()
            .await
            .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        for n in [
            "catalog_search",
            "catalog_get",
            "catalog_plan_install",
            "catalog_install",
        ] {
            assert!(names.contains(&n), "missing {n}: {names:?}");
        }
        assert!(!names.contains(&"fs_write"), "{names:?}");
        // Out of scope → a tool error, not a crash.
        let out: Value = rpc(
            token.clone(),
            "tools/call",
            json!({ "name": "shell_exec", "arguments": { "command": "true", "description": "x" } }),
        )
        .await
        .json()
        .await
        .unwrap();
        assert_eq!(out["result"]["isError"], true);
        assert!(out["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ERR_MCP_SCOPE"));
        assert!(recent_calls(Some("thr_http"))
            .iter()
            .any(|c| c["tool"] == "shell_exec" && c["ok"] == false));

        let node = std::env::var("OMNIGET_MCP_SMOKE_NODE").ok().as_deref() == Some("1");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("mcp-client-smoke.mjs");
        fn smoke(script: std::path::PathBuf, args: Vec<String>) -> bool {
            let out = std::process::Command::new("node")
                .arg(&script)
                .args(args)
                .output()
                .expect("node");
            eprintln!("{}", String::from_utf8_lossy(&out.stdout));
            out.status.success()
        }
        if node {
            let url2 = url.clone();
            let tok2 = token.clone();
            let sc = script.clone();
            let ok = tokio::task::spawn_blocking(move || {
                smoke(
                    sc,
                    vec![
                        url2,
                        tok2,
                        "--list".into(),
                        "--expect".into(),
                        "catalog_search,catalog_get,catalog_plan_install,catalog_install".into(),
                        "--forbid".into(),
                        "fs_read,shell_exec,agent_delegate".into(),
                    ],
                )
            })
            .await
            .unwrap();
            assert!(ok, "node client with the session token");
        }

        // Session stopped → the same token is refused.
        assert!(revoke_thread("thr_http") >= 1);
        let res = rpc(token.clone(), "tools/list", json!({})).await;
        assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);
        if node {
            let (url2, tok2) = (url.clone(), token.clone());
            let sc = script.clone();
            let ok = tokio::task::spawn_blocking(move || {
                smoke(sc, vec![url2, tok2, "--expect-unauthorized".into()])
            })
            .await
            .unwrap();
            assert!(ok, "node client refused after revoke");
        }
    }

    #[test]
    fn thread_opt_out_is_persisted() {
        let p = std::env::temp_dir()
            .join(format!("omniget-mcp-{}", uuid::Uuid::new_v4().simple()))
            .join("mcp-threads.json");
        assert!(read_disabled(Some(&p)).is_empty());
        write_disabled(&p, "thr_a", false).unwrap();
        write_disabled(&p, "thr_b", false).unwrap();
        write_disabled(&p, "thr_a", true).unwrap();
        let set = read_disabled(Some(&p));
        assert!(!set.contains("thr_a") && set.contains("thr_b"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn compact_plan_drops_bodies_and_clips_diffs() {
        let big = "+x\n".repeat(20_000);
        let plan = json!({
            "id": "p1",
            "units": [{ "target": "claude", "files": [{ "content": "AAAA" }] }],
            "files": [{ "path": "/a", "action": "create", "diff": big }],
        });
        let v = compact_plan(plan);
        assert_eq!(v["plan_id"], "p1");
        assert!(v["units"][0].get("files").is_none());
        assert_eq!(v["files"][0]["diffClipped"], true);
        assert!(v["files"][0]["diff"].as_str().unwrap().len() < 17 * 1024);
    }

    #[test]
    fn snippets_carry_token() {
        for (_, s) in client_snippets("http://127.0.0.1:47720/mcp", "tok123") {
            assert!(s.contains("tok123"));
            assert!(s.contains("47720"));
        }
    }
}
