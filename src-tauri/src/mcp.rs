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
            "download_url" => {
                let url = s(&a, "url");
                let action =
                    crate::external_url::handle_external_url(app, url.clone(), "mcp").await?;
                Ok(json!({ "url": url, "action": format!("{:?}", action).to_lowercase() }))
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

// ── JSON-RPC ───────────────────────────────────────────────────────────

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Trata uma mensagem. `None` = notificação (sem resposta).
pub async fn handle(app: &AppHandle, msg: &Value) -> Option<Value> {
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
                    "instructions": "OmniGet desktop tools: downloads, PDF, speech, images, files, X/Twitter, Instagram, system. Paths are local to this machine."
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(app, name, args).await {
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
pub async fn handle_body(app: &AppHandle, body: &Value) -> Option<Value> {
    match body {
        Value::Array(items) => {
            let mut out = Vec::new();
            for m in items {
                if let Some(r) = handle(app, m).await {
                    out.push(r);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Array(out))
            }
        }
        m => handle(app, m).await,
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
        assert_eq!(list.len(), 26, "a tabela mudou de tamanho");
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
        assert_eq!(host.len(), 15, "{:?}", host);
    }

    #[test]
    fn snippets_carry_token() {
        for (_, s) in client_snippets("http://127.0.0.1:47720/mcp", "tok123") {
            assert!(s.contains("tok123"));
            assert!(s.contains("47720"));
        }
    }
}
