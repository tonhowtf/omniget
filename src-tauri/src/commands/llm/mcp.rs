//! `llm_*` commands: the external MCP servers (`/llm/mcp`, client half).
//! Owned by f3-mcp-ui.
//!
//! Thin wrappers over [`McpRegistry`] (f3-mcp-core). Three things live here and
//! nowhere else:
//!
//! 1. **The process-wide registry.** One `Arc<McpRegistry>` on the app data
//!    dir, built on first use. It is not in `AppState` (a shared file): whoever
//!    else needs it — the tool broker, above all — takes it from
//!    [`registry`], which is the same instance, so there is never a second
//!    process for the same server.
//! 2. **Validation.** A config is checked before it can reach the registry, so
//!    a typo answers `ERR_MCP_CONFIG` instead of spawning something that fails.
//!    The same rules run in the form (`src/lib/llm/mcp.ts::validateServer`).
//! 3. **Grants.** `llm_mcp_grant` edits one entry of `AgentDef.tools` and saves
//!    the agent through the roster the manager owns — the registry has no
//!    business knowing about agents.
//!
//! Budget: `llm_mcp_list` reads a file. The only command that can start a
//! process is `llm_mcp_test` (and `llm_mcp_tools` when the cache is cold),
//! both behind a button.

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

/// A config the registry must never see.
pub const ERR_CONFIG: &str = "ERR_MCP_CONFIG";
/// There is no app data dir to keep `mcp-servers.json` in.
pub const ERR_NO_STORE: &str = "ERR_MCP_NO_STORE";
/// `agent_id` is not in the roster.
pub const ERR_UNKNOWN_AGENT: &str = "ERR_MCP_UNKNOWN_AGENT";
/// The vault refused to keep a credential; nothing is saved in that case, so a
/// token never ends up on disk because the keychain was locked.
pub const ERR_SECRET: &str = "ERR_MCP_SECRET";

static REGISTRY: OnceLock<Option<Arc<McpRegistry>>> = OnceLock::new();

/// The one registry of the process, or `None` when there is no data dir.
///
/// Public on purpose: the tool broker connects the *same* servers the tab
/// configures, and two registries would mean two processes per server.
pub fn registry() -> Option<Arc<McpRegistry>> {
    REGISTRY
        .get_or_init(|| McpRegistry::default_store().map(Arc::new))
        .clone()
}

fn need_registry() -> Result<Arc<McpRegistry>, String> {
    registry().ok_or_else(|| format!("{ERR_NO_STORE}: no application data directory"))
}

/// `ERR_MCP_*: message`, the shape `mcpErrorKey` in the front reads.
fn err(error: McpError) -> String {
    format!("{}: {}", error.code, error.message)
}

// ── Wire shapes (the config itself is `McpServerConfig`, verbatim) ───────

/// One row of `llm_mcp_list`: the config plus what the UI shows about it.
///
/// `tools` is what the live client already had cached; a server that was never
/// connected answers an empty list and the tab says "never probed" instead of
/// connecting on its own.
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

/// Answer of `llm_mcp_test`: one `initialize` + `tools/list`. The client stays
/// in the registry afterwards and the reaper takes it down after 5 min idle.
#[derive(Debug, Clone, Serialize)]
pub struct McpTestResultDto {
    pub ok: bool,
    pub server_name: Option<String>,
    pub protocol_version: Option<String>,
    /// `false` when the server negotiated a version we have not tested.
    pub protocol_known: bool,
    pub tool_count: usize,
    pub elapsed_ms: u64,
    pub error: Option<McpErrorDto>,
    /// reachable → authenticated → initialized → tool-capable; only the last
    /// is usable (a 401 answer is reachable, not usable).
    pub health: omniget_core::core::assist::missions::diag::McpHealth,
}

/// What `llm_mcp_grant` takes: an MCP tool and the mode, `null` to clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantDto {
    pub server: String,
    pub tool: String,
    /// `auto`, `ask`, `deny`, or absent/`null` to drop the grant entirely.
    #[serde(default)]
    pub mode: Option<GrantMode>,
}

// ── Pure helpers (tested below; no registry, no Tauri) ──────────────────

/// `Ok(())` for a config that may reach the registry. The message names the
/// field; the code stays `ERR_MCP_CONFIG` so the UI can map it.
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

/// Stands in for a value the tab is not allowed to see. Sending it back
/// unchanged means "keep what is stored", which is what [`merge_kept`] does —
/// so editing a server never overwrites a credential with its own mask.
pub const KEPT: &str = "<kept>";

/// The config as the tab may see it: a literal value under a credential name
/// — an HTTP header or a stdio env var alike — becomes [`KEPT`]. The rule is
/// `core::mcp::types::name_is_secret`, the same one `extract_secrets` uses, so
/// nothing can be extracted into the vault yet shown in the clear (that split
/// is exactly the defect the Phase 3 verifier found: env was not masked).
///
/// A `secret:<id>` reference stays as it is — it is a pointer into the vault,
/// not a secret, and the user needs to read and edit it.
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

/// `(name, &mut value)` over whatever the transport carries: headers for HTTP,
/// env for stdio. One walker, so a rule can never apply to one and not the
/// other by accident — which is the defect this pair of functions had.
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

/// Puts back every header or env value the tab sent as [`KEPT`], from the
/// stored config. A `KEPT` with nothing behind it is dropped: a mask must
/// never become a value.
pub fn merge_kept(mut next: McpServerConfig, stored: Option<&McpServerConfig>) -> McpServerConfig {
    // Only a transport of the same kind has anything to give back.
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

/// The tool list of an agent after setting (or clearing, with `None`) one MCP
/// grant. Pure counterpart of `applyGrant` in `src/lib/llm/mcp.ts`; the order
/// of the untouched grants is preserved so saving is not a reshuffle.
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

/// Every configured server with what is known about it right now. Reads the
/// live clients; never creates one.
async fn rows(reg: &Arc<McpRegistry>) -> Vec<McpServerRowDto> {
    let connected = reg.connected().await;
    let mut out = Vec::new();
    for config in reg.list() {
        let live = connected.contains(&config.id);
        // Only a client that is already up is asked, and only for its cache.
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

// ── Commands ────────────────────────────────────────────────────────────

/// Saved servers. Reads the config file; opens no connection.
#[tauri::command]
pub async fn llm_mcp_list() -> Result<Vec<McpServerRowDto>, String> {
    let reg = need_registry()?;
    Ok(rows(&reg).await)
}

/// Creates or replaces one server. Validated here, never connected here; a
/// config that changed drops the live client so the next test gets the new one.
///
/// A credential the user typed — an `Authorization` header, a `GITHUB_TOKEN`
/// env var — goes into the vault and only a `secret:<id>` reference is written
/// to `mcp-servers.json`. So the value exists in exactly two places: the vault
/// and the child process's environment at connect time. Never on disk in
/// plaintext, never back in the webview.
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

// ── Vault writes with an undo ───────────────────────────────────────────

/// The vault of the process: the store installed at boot (keychain or the
/// encrypted file, `crate::secrets::AppSecretStore`), or the core's own file
/// store in a headless run that never installed one.
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

/// What each credential id held before an upsert wrote it: `None` = nothing.
type SecretUndo = Vec<(String, Option<String>)>;

/// Saves one server with its credentials, all or nothing.
///
/// `extract_secrets` (f3-mcp-core) swaps every literal credential for a
/// `secret:<id>` reference and hands the pairs over; the vault writes have to
/// succeed before `register` saves the config, or a reference would point at
/// nothing and the server would stop connecting. When `register` fails after
/// the vault took the new values, every id gets its previous value back (or is
/// deleted when it had none), so the saved config and the vault keep agreeing.
/// `register` only ever sees references: a vault that refuses a write fails
/// the upsert instead of letting the literal reach `mcp-servers.json`.
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

/// Writes `pairs` into the MCP namespace, remembering what each id held. The
/// first failure undoes the writes already made and answers [`ERR_SECRET`].
fn write_secrets(
    vault: &dyn SecretStore,
    pairs: &[(String, String)],
) -> Result<SecretUndo, String> {
    let mut undo: SecretUndo = Vec::with_capacity(pairs.len());
    for (id, value) in pairs {
        // A previous value we cannot read is a previous value we could not put
        // back, so the write does not happen at all.
        let previous = match vault.get(SECRET_NS, id) {
            Ok(previous) => previous,
            Err(e) => {
                restore_secrets(vault, undo);
                return Err(format!("{ERR_SECRET}: {e}"));
            }
        };
        let written = vault.set(SECRET_NS, id, value);
        // Recorded even when the write failed: a store may have changed half
        // of its state before answering, and putting the old value back is
        // harmless when it did not.
        undo.push((id.clone(), previous));
        if let Err(e) = written {
            restore_secrets(vault, undo);
            return Err(format!("{ERR_SECRET}: {e}"));
        }
    }
    Ok(undo)
}

/// Puts every id back to what [`write_secrets`] found, newest first. A restore
/// that fails is logged by id only (never the value) and the others still run.
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

/// Removes a server and stops its process.
#[tauri::command]
pub async fn llm_mcp_remove(id: String) -> Result<Vec<McpServerRowDto>, String> {
    if !valid_id(&id) {
        return Err(format!("{ERR_CONFIG}: id"));
    }
    let reg = need_registry()?;
    reg.remove(&id).await.map_err(err)?;
    Ok(rows(&reg).await)
}

/// The one command that connects on purpose: `initialize` + a fresh
/// `tools/list`. A failure is an answer, not an `Err`, so the row can show the
/// code and the message the server gave.
#[tauri::command]
pub async fn llm_mcp_test(id: String) -> Result<McpTestResultDto, String> {
    if !valid_id(&id) {
        return Err(format!("{ERR_CONFIG}: id"));
    }
    probe(&need_registry()?, &id).await
}

/// Body of `llm_mcp_test`, against any registry — which is what lets the test
/// below drive it with the fixture instead of the user's real config.
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

/// Tools of one server, from the client's cache when it has one. Connects only
/// if there is no live client — which is why the tab calls it when the grant
/// table opens and not on page load.
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

/// Sets one MCP grant on one agent and saves the roster.
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
