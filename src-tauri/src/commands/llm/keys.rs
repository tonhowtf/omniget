use omniget_core::core::tools::ai_keys;
use serde::Serialize;

#[derive(Serialize)]
pub struct KindView {
    #[serde(flatten)]
    kind: ai_keys::Kind,
    needs_key: bool,
    accepts_key: bool,
    base_url_editable: bool,
}

#[tauri::command]
pub fn tool_keys_kinds() -> Vec<KindView> {
    ai_keys::KINDS
        .iter()
        .map(|kind| KindView {
            needs_key: kind.needs_key(),
            accepts_key: kind.accepts_key(),
            base_url_editable: kind.base_url_editable(),
            kind: kind.clone(),
        })
        .collect()
}

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

#[tauri::command]
pub async fn tool_mcp_executors(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
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

#[tauri::command]
pub fn tool_mcp_execution_grant_revoke(grant_id: String) -> Result<(), String> {
    let db = omniget_core::core::assist::db::global()?;
    omniget_core::core::assist::authority::revoke(&db, &grant_id)
}

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

#[tauri::command]
pub fn tool_mcp_execution_grants_list(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    use serde_json::json;
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
