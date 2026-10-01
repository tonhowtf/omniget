use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use omniget_core::core::llm::agent::{AgentDef, GrantMode, ToolGrant, ToolSource};
use omniget_core::core::mcp::types::{
    name_is_secret, secret_ref, valid_id, McpServerConfig, Transport, SECRET_NS,
};
use omniget_core::core::mcp::{McpError, McpRegistry, ToolDef};
use omniget_core::core::secrets::SecretStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::AppState;

pub const ERR_CONFIG: &str = "ERR_MCP_CONFIG";
pub const ERR_NO_STORE: &str = "ERR_MCP_NO_STORE";
pub const ERR_UNKNOWN_AGENT: &str = "ERR_MCP_UNKNOWN_AGENT";
pub const ERR_SECRET: &str = "ERR_MCP_SECRET";

static REGISTRY: OnceLock<Option<Arc<McpRegistry>>> = OnceLock::new();

pub fn registry() -> Option<Arc<McpRegistry>> {
    REGISTRY
        .get_or_init(|| McpRegistry::default_store().map(Arc::new))
        .clone()
}

fn need_registry() -> Result<Arc<McpRegistry>, String> {
    registry().ok_or_else(|| format!("{ERR_NO_STORE}: no application data directory"))
}

fn err(error: McpError) -> String {
    format!("{}: {}", error.code, error.message)
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerRowDto {
    pub config: McpServerConfig,
    pub tools: Vec<ToolDef>,
    pub connected: bool,
    pub last_error: Option<McpErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpErrorDto {
    pub code: String,
    pub message: String,
}

impl From<McpError> for McpErrorDto {
    fn from(error: McpError) -> Self {
        Self {
            code: error.code.to_string(),
            message: error.message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpTestResultDto {
    pub ok: bool,
    pub server_name: Option<String>,
    pub protocol_version: Option<String>,
    pub protocol_known: bool,
    pub tool_count: usize,
    pub elapsed_ms: u64,
    pub error: Option<McpErrorDto>,
    pub health: omniget_core::core::assist::missions::diag::McpHealth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantDto {
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub mode: Option<GrantMode>,
}

pub fn validate(config: &McpServerConfig) -> Result<(), String> {
    if !valid_id(&config.id) {
        return Err(format!("{ERR_CONFIG}: id"));
    }
    if config.name.trim().is_empty() {
        return Err(format!("{ERR_CONFIG}: name"));
    }
    match &config.transport {
        Transport::Stdio { command, .. } => {
            if command.trim().is_empty() {
                return Err(format!("{ERR_CONFIG}: command"));
            }
        }
        Transport::Http { url, .. } => {
            let url = url.trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() < 12 {
                return Err(format!("{ERR_CONFIG}: url"));
            }
        }
    }
    Ok(())
}

pub const KEPT: &str = "<kept>";

pub fn for_display(mut config: McpServerConfig) -> McpServerConfig {
    for (name, value) in pairs_mut(&mut config.transport) {
        if value.is_empty() || secret_ref(value).is_some() {
            continue;
        }
        if name_is_secret(&name) {
            *value = KEPT.to_string();
        }
    }
    config
}

fn pairs_mut(transport: &mut Transport) -> Vec<(String, &mut String)> {
    match transport {
        Transport::Http { headers, .. } => headers
            .iter_mut()
            .map(|(name, value)| (name.clone(), value))
            .collect(),
        Transport::Stdio { env, .. } => env
            .iter_mut()
            .map(|(name, value)| (name.clone(), value))
            .collect(),
    }
}

fn pairs_of(transport: &Transport) -> &BTreeMap<String, String> {
    match transport {
        Transport::Http { headers, .. } => headers,
        Transport::Stdio { env, .. } => env,
    }
}

fn pairs_mut_map(transport: &mut Transport) -> &mut BTreeMap<String, String> {
    match transport {
        Transport::Http { headers, .. } => headers,
        Transport::Stdio { env, .. } => env,
    }
}

pub fn merge_kept(mut next: McpServerConfig, stored: Option<&McpServerConfig>) -> McpServerConfig {
    let previous = stored
        .map(|c| &c.transport)
        .filter(|t| std::mem::discriminant(*t) == std::mem::discriminant(&next.transport))
        .map(pairs_of)
        .cloned();
    let map = pairs_mut_map(&mut next.transport);
    let names: Vec<String> = map
        .iter()
        .filter(|(_, value)| value.as_str() == KEPT)
        .map(|(name, _)| name.clone())
        .collect();
    for name in names {
        match previous.as_ref().and_then(|p| p.get(&name)) {
            Some(value) => {
                map.insert(name, value.clone());
            }
            None => {
                map.remove(&name);
            }
        }
    }
    next
}

pub fn apply_grant(
    tools: &[ToolGrant],
    server: &str,
    tool: &str,
    mode: Option<GrantMode>,
) -> Vec<ToolGrant> {
    let mut out: Vec<ToolGrant> = tools
        .iter()
        .filter(|grant| !is_mcp(grant, server, tool))
        .cloned()
        .collect();
    if let Some(mode) = mode {
        out.push(ToolGrant {
            source: ToolSource::Mcp {
                server: server.to_string(),
                tool: tool.to_string(),
            },
            mode,
        });
    }
    out
}

fn is_mcp(grant: &ToolGrant, server: &str, tool: &str) -> bool {
    matches!(
        &grant.source,
        ToolSource::Mcp { server: s, tool: t } if s == server && t == tool
    )
}

async fn rows(reg: &Arc<McpRegistry>) -> Vec<McpServerRowDto> {
    let connected = reg.connected().await;
    let mut out = Vec::new();
    for config in reg.list() {
        let live = connected.contains(&config.id);
        let tools = if live {
            match reg.client_for(&config.id).await {
                Ok(client) => client
                    .tools()
                    .await
                    .map(|t| (*t).clone())
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        out.push(McpServerRowDto {
            config: for_display(config),
            tools,
            connected: live,
            last_error: None,
        });
    }
    out
}

#[tauri::command]
pub async fn llm_mcp_list() -> Result<Vec<McpServerRowDto>, String> {
    let reg = need_registry()?;
    Ok(rows(&reg).await)
}

#[tauri::command]
pub async fn llm_mcp_upsert(config: McpServerConfig) -> Result<Vec<McpServerRowDto>, String> {
    let reg = need_registry()?;
    let stored = reg.get(&config.id);
    let config = merge_kept(config, stored.as_ref());
    validate(&config)?;
    upsert_with_rollback(&ProcessVault, config, |config| {
        let reg = reg.clone();
        async move { reg.upsert(config).await.map(|_| ()).map_err(err) }
    })
    .await?;
    Ok(rows(&reg).await)
}

struct ProcessVault;

impl SecretStore for ProcessVault {
    fn get(&self, namespace: &str, account: &str) -> Result<Option<String>, String> {
        omniget_core::core::secrets::get(namespace, account)
    }

    fn set(&self, namespace: &str, account: &str, value: &str) -> Result<(), String> {
        omniget_core::core::secrets::set(namespace, account, value)
    }

    fn delete(&self, namespace: &str, account: &str) -> Result<(), String> {
        omniget_core::core::secrets::delete(namespace, account)
    }
}

type SecretUndo = Vec<(String, Option<String>)>;

async fn upsert_with_rollback<F, Fut>(
    vault: &dyn SecretStore,
    mut config: McpServerConfig,
    register: F,
) -> Result<(), String>
where
    F: FnOnce(McpServerConfig) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let pairs = config.extract_secrets();
    let undo = write_secrets(vault, &pairs)?;
    if let Err(error) = register(config).await {
        restore_secrets(vault, undo);
        return Err(error);
    }
    Ok(())
}

fn write_secrets(
    vault: &dyn SecretStore,
    pairs: &[(String, String)],
) -> Result<SecretUndo, String> {
    let mut undo: SecretUndo = Vec::with_capacity(pairs.len());
    for (id, value) in pairs {
        let previous = match vault.get(SECRET_NS, id) {
            Ok(previous) => previous,
            Err(e) => {
                restore_secrets(vault, undo);
                return Err(format!("{ERR_SECRET}: {e}"));
            }
        };
        let written = vault.set(SECRET_NS, id, value);
        undo.push((id.clone(), previous));
        if let Err(e) = written {
            restore_secrets(vault, undo);
            return Err(format!("{ERR_SECRET}: {e}"));
        }
    }
    Ok(undo)
}

fn restore_secrets(vault: &dyn SecretStore, undo: SecretUndo) {
    for (id, previous) in undo.into_iter().rev() {
        let restored = match previous {
            Some(value) => vault.set(SECRET_NS, &id, &value),
            None => vault.delete(SECRET_NS, &id),
        };
        if let Err(e) = restored {
            tracing::warn!("[mcp] could not restore the secret {id:?} after a failed save: {e}");
        }
    }
}

#[tauri::command]
pub async fn llm_mcp_remove(id: String) -> Result<Vec<McpServerRowDto>, String> {
    if !valid_id(&id) {
        return Err(format!("{ERR_CONFIG}: id"));
    }
    let reg = need_registry()?;
    reg.remove(&id).await.map_err(err)?;
    Ok(rows(&reg).await)
}

#[tauri::command]
pub async fn llm_mcp_test(id: String) -> Result<McpTestResultDto, String> {
    if !valid_id(&id) {
        return Err(format!("{ERR_CONFIG}: id"));
    }
    probe(&need_registry()?, &id).await
}

pub async fn probe(reg: &Arc<McpRegistry>, id: &str) -> Result<McpTestResultDto, String> {
    let started = Instant::now();
    let fail = |error: McpError, started: Instant| {
        let error: McpErrorDto = error.into();
        McpTestResultDto {
            ok: false,
            server_name: None,
            protocol_version: None,
            protocol_known: true,
            tool_count: 0,
            elapsed_ms: started.elapsed().as_millis() as u64,
            health: omniget_core::core::assist::missions::diag::mcp_health(
                false,
                Some(&error.code),
                Some(&error.message),
                0,
            ),
            error: Some(error),
        }
    };
    let client = match reg.client_for(id).await {
        Ok(client) => client,
        Err(error) => return Ok(fail(error, started)),
    };
    match client.refresh_tools().await {
        Ok(tools) => Ok(McpTestResultDto {
            ok: true,
            server_name: Some(client.server_info().name.clone()),
            protocol_version: Some(client.protocol_version().to_string()),
            protocol_known: client.protocol_is_known(),
            tool_count: tools.len(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            error: None,
            health: omniget_core::core::assist::missions::diag::mcp_health(
                true,
                None,
                None,
                tools.len(),
            ),
        }),
        Err(error) => Ok(fail(error, started)),
    }
}

#[tauri::command]
pub async fn llm_mcp_tools(id: String) -> Result<Vec<ToolDef>, String> {
    if !valid_id(&id) {
        return Err(format!("{ERR_CONFIG}: id"));
    }
    let reg = need_registry()?;
    let client = reg.client_for(&id).await.map_err(err)?;
    let tools = client.tools().await.map_err(err)?;
    Ok((*tools).clone())
}

#[tauri::command]
pub async fn llm_mcp_grant(
    state: State<'_, AppState>,
    agent_id: String,
    grant: GrantDto,
) -> Result<Value, String> {
    if grant.server.trim().is_empty() || grant.tool.trim().is_empty() {
        return Err(format!("{ERR_CONFIG}: grant"));
    }
    let mut agent: AgentDef = state
        .llm
        .roster()
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| format!("{ERR_UNKNOWN_AGENT}: {agent_id}"))?;
    agent.tools = apply_grant(&agent.tools, &grant.server, &grant.tool, grant.mode);
    let roster = state.llm.roster_update(agent)?;
    serde_json::to_value(roster).map_err(|e| e.to_string())
}
