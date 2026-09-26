use omniget_core::core::tools::{ai_keys, ollama, pricing, usage};
use serde::Serialize;

use super::{err, progress};

#[tauri::command]
pub async fn tool_ollama_status(host: Option<String>) -> ollama::OllamaStatus {
    ollama::status(host.as_deref().unwrap_or("")).await
}

#[tauri::command]
pub fn tool_ollama_recommended() -> Vec<ollama::Recommended> {
    ollama::recommended()
}

#[tauri::command]
pub async fn tool_ollama_pull(
    app: tauri::AppHandle,
    host: Option<String>,
    name: String,
) -> Result<(), String> {
    ollama::pull(host.as_deref().unwrap_or(""), &name, progress(&app))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_ollama_delete(host: Option<String>, name: String) -> Result<(), String> {
    ollama::delete(host.as_deref().unwrap_or(""), &name)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pricing_info(force: Option<bool>) -> Result<pricing::PricingInfo, String> {
    pricing::info(force.unwrap_or(false)).await.map_err(err)
}

#[tauri::command]
pub async fn tool_pricing_search(
    query: String,
    mode: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<pricing::ModelPrice>, String> {
    pricing::search(&query, mode.as_deref().unwrap_or(""), limit.unwrap_or(60))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_pricing_for(model: String) -> Option<pricing::ModelPrice> {
    pricing::price_for(&model).await
}

#[tauri::command]
pub async fn tool_usage_report(days: Option<u32>) -> usage::UsageReport {
    usage::report(days.unwrap_or(30)).await
}

#[tauri::command]
pub fn tool_usage_clear() -> Result<(), String> {
    usage::clear().map_err(err)
}

// ── Chaves de API (estudo 24) ──

#[tauri::command]
pub fn tool_keys_kinds() -> Vec<ai_keys::Kind> {
    ai_keys::KINDS.to_vec()
}

#[tauri::command]
pub fn tool_keys_list() -> Vec<ai_keys::KeyView> {
    ai_keys::list()
}

#[tauri::command]
pub fn tool_keys_save(entry: ai_keys::KeyEntry) -> Result<ai_keys::KeyView, String> {
    ai_keys::upsert(entry).map_err(err)
}

#[tauri::command]
pub fn tool_keys_delete(id: String) -> Result<(), String> {
    ai_keys::delete(&id).map_err(err)
}

#[tauri::command]
pub async fn tool_keys_test(id: String) -> Result<ai_keys::KeyView, String> {
    ai_keys::test(&id).await.map_err(err)
}

#[tauri::command]
pub async fn tool_keys_balance(id: String) -> Result<ai_keys::KeyView, String> {
    ai_keys::balance(&id).await.map_err(err)
}

#[tauri::command]
pub async fn tool_keys_models(entry: ai_keys::KeyEntry) -> Result<Vec<String>, String> {
    // Chave salva: a UI manda `key` vazia e o id; buscamos o segredo aqui.
    let e = if entry.key.is_empty() && !entry.id.is_empty() {
        ai_keys::entry_with_secret(&entry.id).map_err(err)?
    } else {
        entry
    };
    ai_keys::models(&e).await.map_err(err)
}

#[tauri::command]
pub fn tool_keys_export(format: String, ids: Vec<String>) -> Result<String, String> {
    ai_keys::export(&format, &ids).map_err(err)
}

#[tauri::command]
pub fn tool_keys_use(id: String) -> Result<(), String> {
    ai_keys::use_in_app(&id).map_err(err)
}

// ── Sign in with OpenRouter (OAuth PKCE, sem backend) ──
//
// Two steps, because the browser round-trip happens in between: the UI opens
// `url`, OpenRouter sends the user back to `omniget://openrouter-auth?state=…`
// with `code` appended, and the deep-link handler hands both to `_finish`. The
// state is mandatory there: a callback without the one this process generated is
// refused. The verifier stays in the core and the key goes straight into the
// secret store — neither ever reaches the frontend.

/// Starts the flow: returns the URL to open and the state to echo back.
#[tauri::command]
pub fn tool_ai_keys_openrouter_pkce() -> ai_keys::PkceStart {
    ai_keys::pkce_start()
}

/// Exchanges the code for a key and files it in the vault. Returns the masked
/// view of the entry it created or updated.
#[tauri::command]
pub async fn tool_ai_keys_openrouter_pkce_finish(
    code: String,
    state: Option<String>,
) -> Result<ai_keys::KeyView, String> {
    ai_keys::pkce_finish(&code, state.as_deref())
        .await
        .map_err(err)
}

// ── Servidor MCP ──

#[derive(Serialize)]
pub struct McpStatus {
    pub enabled: bool,
    pub bridge_enabled: bool,
    pub port: u16,
    pub url: String,
    pub token: String,
    pub tools: Vec<crate::mcp::ToolDef>,
    pub snippets: Vec<(String, String)>,
}

#[tauri::command]
pub fn tool_mcp_status(app: tauri::AppHandle) -> McpStatus {
    let settings = crate::storage::config::load_settings(&app);
    let url = if settings.bridge.port == 0 {
        String::new()
    } else {
        format!("http://127.0.0.1:{}/mcp", settings.bridge.port)
    };
    McpStatus {
        enabled: crate::mcp::enabled(),
        bridge_enabled: settings.bridge.enabled,
        port: settings.bridge.port,
        snippets: Vec::new(),
        url,
        token: String::new(),
        tools: crate::mcp::downloads::tools(&crate::mcp::policy::Principal {
            id: String::new(),
            name: String::new(),
            scopes: crate::mcp::policy::SCOPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }),
    }
}

#[tauri::command]
pub fn tool_mcp_set_enabled(enabled: bool) -> Result<bool, String> {
    crate::mcp::set_enabled(enabled)?;
    Ok(crate::mcp::enabled())
}

/// Faz um `initialize` + `tools/list` de verdade contra o proprio endpoint,
/// para provar que a porta, o token e o servidor respondem.
#[tauri::command]
pub async fn tool_mcp_selftest(app: tauri::AppHandle) -> Result<String, String> {
    let settings = crate::storage::config::load_settings(&app);
    if settings.bridge.port == 0 {
        return Err("bridge sem porta".into());
    }
    let url = format!("http://127.0.0.1:{}/mcp", settings.bridge.port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(err)?;
    let grant =
        crate::mcp::policy::create("Local connection test".into(), vec!["discover".into()])?;
    struct Revoke(String);
    impl Drop for Revoke {
        fn drop(&mut self) {
            let _ = crate::mcp::policy::revoke(&self.0);
        }
    }
    let _revoke = Revoke(grant.principal.id.clone());
    let init = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": crate::mcp::PROTOCOL, "capabilities": {}, "clientInfo": { "name": "omniget-selftest", "version": "1" } } });
    let r: serde_json::Value = client
        .post(&url)
        .bearer_auth(&grant.token)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", crate::mcp::PROTOCOL)
        .json(&init)
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let version = r["result"]["protocolVersion"]
        .as_str()
        .unwrap_or("?")
        .to_string();
    let list = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let r: serde_json::Value = client
        .post(&url)
        .bearer_auth(&grant.token)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", crate::mcp::PROTOCOL)
        .json(&list)
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    let n = r["result"]["tools"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(format!("{} · {} tools · {}", version, n, url))
}

// ── Contagem de tokens e custo por modelo (estende ai-prices) ──

use omniget_core::core::tools::{ai_tokens, arxiv};

/// As famílias de tokenizador que a heurística conhece, com o fator de cada uma.
#[tauri::command]
pub fn tool_tokens_families() -> Vec<ai_tokens::Family> {
    ai_tokens::FAMILIES.to_vec()
}

/// Modelos comparados quando a UI não escolhe nenhum.
#[tauri::command]
pub fn tool_tokens_default_models() -> Vec<String> {
    ai_tokens::DEFAULT_MODELS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Só a contagem (sem tabela de preços e sem rede).
#[tauri::command]
pub async fn tool_tokens_count(
    app: tauri::AppHandle,
    opts: ai_tokens::Options,
) -> Result<ai_tokens::Report, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || ai_tokens::count(&opts, &p))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Contagem estimada + custo por modelo. O número é sempre estimativa:
/// `margin_pct` e `estimated` vêm no resultado para a UI dizer isso.
#[tauri::command]
pub async fn tool_tokens_cost(
    app: tauri::AppHandle,
    opts: ai_tokens::Options,
) -> Result<ai_tokens::Report, String> {
    ai_tokens::run(opts, progress(&app)).await.map_err(err)
}

// ── arXiv → Markdown ──

#[tauri::command]
pub async fn tool_arxiv_md(
    app: tauri::AppHandle,
    opts: arxiv::Options,
) -> Result<arxiv::ArxivDoc, String> {
    arxiv::fetch(opts, progress(&app)).await.map_err(err)
}

#[tauri::command]
pub fn tool_mcp_clients() -> Result<Vec<crate::mcp::policy::Principal>, String> {
    crate::mcp::policy::list()
}
#[tauri::command]
pub fn tool_mcp_client_create(
    name: String,
    scopes: Vec<String>,
) -> Result<crate::mcp::policy::ConnectionGrant, String> {
    crate::mcp::policy::create(name, scopes)
}
#[tauri::command]
pub fn tool_mcp_client_revoke(id: String) -> Result<(), String> {
    crate::mcp::policy::revoke(&id)
}

#[tauri::command]
pub fn tool_mcp_root_grant(principal: String, path: String) -> Result<String, String> {
    crate::mcp::artifacts::grant_root(&principal, std::path::Path::new(&path))
}
#[tauri::command]
pub fn tool_mcp_root_revoke(id: String) -> Result<(), String> {
    crate::mcp::artifacts::revoke_root(&id)
}

#[tauri::command]
pub fn tool_mcp_client_snippets(
    app: tauri::AppHandle,
    token: String,
) -> Result<Vec<(String, String)>, String> {
    crate::mcp::policy::authenticate(&token)?;
    let settings = crate::storage::config::load_settings(&app);
    Ok(crate::mcp::client_snippets(
        &format!("http://127.0.0.1:{}/mcp", settings.bridge.port),
        &token,
    ))
}

/// Trusted desktop grant creation. Remote tools can only select this ceiling.
#[tauri::command]
pub async fn tool_mcp_executors(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    // Claude Code qualifies only after its installed flags are probed.
    for a in state.llm.roster() {
        if matches!(&a.runtime,omniget_core::core::llm::agent::RuntimeKind::Cli{cli,..} if cli=="claude")
        {
            let _ = omniget_core::core::llm::caps::probe(&a.runtime).await;
            break;
        }
    }
    Ok(executors_now(&state))
}
fn executors_now(state: &tauri::State<'_, crate::AppState>) -> serde_json::Value {
    // Bots derived by an external client belong to its grant; they are not
    // executors a person can hand to another grant.
    let derived = omniget_core::core::assist::db::global()
        .ok()
        .and_then(|db| omniget_core::core::assist::external_config::derived_bot_marks(&db).ok())
        .unwrap_or_default();
    serde_json::json!(state
        .llm
        .roster()
        .into_iter()
        .filter(|a| !derived.contains_key(&a.id))
        .filter_map(
            |a| omniget_core::core::assist::authority::external_runtime(&a)
                .ok()
                .map(|rt| serde_json::json!({"id":a.id,"name":a.name,"runtime":rt}))
        )
        .collect::<Vec<_>>())
}
#[tauri::command]
pub fn tool_mcp_execution_grant(
    state: tauri::State<'_, crate::AppState>,
    principal: String,
    path: String,
    bots: Vec<String>,
    max_tokens: u64,
    media: bool,
) -> Result<String, String> {
    use omniget_core::core::assist::{authority, db};
    if !crate::mcp::policy::list()?
        .iter()
        .any(|p| p.id == principal)
    {
        return Err("PRINCIPAL_INACTIVE".into());
    }
    let mut revisions = std::collections::BTreeMap::new();
    for bot in &bots {
        let agent = state.llm.agent(bot).ok_or("EXECUTOR_UNAVAILABLE")?;
        omniget_core::core::assist::authority::external_runtime(&agent)?;
        revisions.insert(bot.clone(), authority::agent_revision(&agent)?);
    }
    let database = db::global()?;
    let mut tools = vec![
        "fs_read".into(),
        "fs_list".into(),
        "fs_glob".into(),
        "fs_grep".into(),
        "fs_write".into(),
        "fs_edit".into(),
        "fs_apply_patch".into(),
    ];
    if media {
        tools.extend(
            [
                "media_models_list",
                "media_model_info",
                "media_estimate",
                "media_submit",
                "media_get",
                "media_collect",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    authority::grant(
        &database,
        authority::Ceiling {
            id: String::new(),
            principal,
            workspace_id: uuid::Uuid::new_v4().to_string(),
            workspace: path,
            workspace_identity: None,
            bots,
            bot_revisions: revisions,
            tools,
            max_tokens,
            max_usd: None,
        },
    )
}

/// Trusted desktop revocation of ONE execution grant (L4): its derived bots
/// are disabled, its rooms archived and its pending tasks failed; running
/// missions stop at the next authority check. The client and its other
/// grants stay live.
#[tauri::command]
pub fn tool_mcp_execution_grant_revoke(grant_id: String) -> Result<(), String> {
    let db = omniget_core::core::assist::db::global()?;
    omniget_core::core::assist::authority::revoke(&db, &grant_id)
}

/// `/Users/me/x` → `~/x`, for display only.
fn short_path(p: &str) -> String {
    match dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned)) {
        Some(h)
            if !h.is_empty()
                && (p == h
                    || p.starts_with(&format!("{h}/"))
                    || p.starts_with(&format!("{h}\\"))) =>
        {
            format!("~{}", &p[h.len()..])
        }
        _ => p.to_string(),
    }
}

/// Trusted, read-only view of what each MCP client may execute (execution
/// grants) and which folder it may transfer from (file roots). Tokens and
/// identities are never returned.
#[tauri::command]
pub fn tool_mcp_execution_grants_list(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    use serde_json::json;
    // Names of every client, revoked ones included (a revoked client's grants
    // still show who they belonged to). The policy store is opened read-only.
    let policy = omniget_core::core::tools::tools_dir()
        .map(|d| d.join("mcp").join("policy.sqlite3"))
        .filter(|p| p.is_file())
        .and_then(|p| {
            rusqlite::Connection::open_with_flags(
                p,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .ok()
        });
    let mut names = std::collections::HashMap::<String, (String, bool)>::new();
    let mut roots = Vec::new();
    if let Some(c) = &policy {
        let _ = c.busy_timeout(std::time::Duration::from_secs(2));
        if let Ok(mut st) = c.prepare("SELECT id,name,revoked FROM clients") {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            }) {
                for (id, name, revoked) in rows.flatten() {
                    names.insert(id, (name, revoked != 0));
                }
            }
        }
        // The table only exists once a transfer folder was granted.
        if let Ok(mut st) = c.prepare(
            "SELECT id,principal,path FROM file_roots WHERE revoked=0 AND id NOT LIKE 'exec:%'",
        ) {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            }) {
                for (id, principal, path) in rows.flatten() {
                    let Some((name, false)) = names.get(&principal).cloned() else {
                        continue;
                    };
                    roots.push(json!({"id":id,"principal":principal,"principal_name":name,"path":short_path(&path),"path_full":path}));
                }
            }
        }
    }
    let db = omniget_core::core::assist::db::global()?;
    let bodies: Vec<(String, String, String, i64)> = db.with(|c| {
        let mut st = c.prepare("SELECT id,principal,body,revoked FROM external_grants")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    })?;
    let mut grants = Vec::new();
    for (id, principal, body, revoked) in bodies {
        let Ok(c) = serde_json::from_str::<omniget_core::core::assist::authority::Ceiling>(&body)
        else {
            continue;
        };
        let (name, client_revoked) = names
            .get(&principal)
            .cloned()
            .unwrap_or_else(|| (principal.clone(), false));
        let executors:Vec<serde_json::Value>=c.bots.iter().map(|b|json!({"id":b,"name":state.llm.agent(b).map(|a|a.name).unwrap_or_else(||b.clone())})).collect();
        grants.push(json!({
            "id":id,"principal":principal,"principal_name":name,
            "workspace":short_path(&c.workspace),"workspace_full":c.workspace,
            "executors":executors,"max_tokens":c.max_tokens,
            "media":c.tools.iter().any(|t|t.starts_with("media_")),
            "revoked":revoked!=0||client_revoked,
        }));
    }
    // Bots the clients derived (`agents_prepare`), read-only, under their
    // client's grants; revoked with `assist_mcp_derived_bot_revoke`.
    let derived:Vec<serde_json::Value>=omniget_core::core::assist::external_config::derived_local(&db).unwrap_or_default().into_iter().map(|d|{
        let (name,client_revoked)=names.get(&d.principal).cloned().unwrap_or_else(||(d.principal.clone(),false));
        json!({
            "bot":d.bot,"name":state.llm.agent(&d.bot).map(|a|a.name).unwrap_or_else(||d.bot.clone()),
            "grant_id":d.grant_id,"principal":d.principal,"principal_name":name,
            "source_name":state.llm.agent(&d.source_bot).map(|a|a.name).unwrap_or_else(||d.source_bot.clone()),
            "created_ms":d.created_ms,"retired":d.retired,"grant_active":d.grant_active&&!client_revoked,
        })
    }).collect();
    Ok(json!({"grants":grants,"roots":roots,"derived":derived}))
}

#[tauri::command]
pub fn tool_mcp_network_grant(principal: String, endpoints: Vec<String>) -> Result<(), String> {
    crate::mcp::network_grants::set(&principal, &endpoints)
}

#[tauri::command]
pub fn tool_mcp_gateway_grant(
    principal: String,
    write: bool,
    transfer: bool,
) -> Result<(), String> {
    crate::mcp::gateway_grants::set(&principal, write, transfer)
}

#[tauri::command]
pub fn tool_mcp_media_configure(path: String) -> Result<(), String> {
    let db = omniget_core::core::assist::db::global()?;
    omniget_core::core::assist::media_tools::configure_local_cli(&db, std::path::Path::new(&path))
}
#[tauri::command]
pub fn tool_mcp_media_budget(
    principal: String,
    mission: String,
    microcredits: u64,
) -> Result<(), String> {
    if !crate::mcp::policy::list()?
        .iter()
        .any(|p| p.id == principal)
    {
        return Err("PRINCIPAL_INACTIVE".into());
    }
    let db = omniget_core::core::assist::db::global()?;
    omniget_core::core::assist::media_provider::set_local_budget(
        &db,
        &principal,
        &mission,
        i64::try_from(microcredits).map_err(|_| "PROVIDER_INVALID_BUDGET")?,
    )
}
