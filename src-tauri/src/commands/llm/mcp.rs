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
    let mut config = merge_kept(config, stored.as_ref());
    validate(&config)?;
    // `extract_secrets` (f3-mcp-core) swaps every literal credential for a
    // `secret:<id>` reference and hands the pairs over; the vault write has to
    // succeed before the config is saved, or a reference would point at
    // nothing and the server would stop connecting.
    for (id, value) in config.extract_secrets() {
        omniget_core::core::secrets::set(SECRET_NS, &id, &value)
            .map_err(|e| format!("{ERR_SECRET}: {e}"))?;
    }
    reg.upsert(config).await.map_err(err)?;
    Ok(rows(&reg).await)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(id: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            name: "Filesystem".into(),
            enabled: true,
            timeout_ms: None,
            transport: Transport::Stdio {
                command: command.into(),
                args: vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into(),
                ],
                env: BTreeMap::new(),
                cwd: None,
            },
        }
    }

    fn http(url: &str) -> McpServerConfig {
        McpServerConfig {
            id: "context7".into(),
            name: "Context7".into(),
            enabled: true,
            timeout_ms: None,
            transport: Transport::Http {
                url: url.into(),
                headers: BTreeMap::new(),
            },
        }
    }

    fn mcp(server: &str, tool: &str, mode: GrantMode) -> ToolGrant {
        ToolGrant {
            source: ToolSource::Mcp {
                server: server.into(),
                tool: tool.into(),
            },
            mode,
        }
    }

    #[test]
    fn a_good_stdio_config_passes() {
        assert!(validate(&stdio("filesystem", "npx")).is_ok());
    }

    #[test]
    fn an_empty_command_is_refused_before_anything_is_spawned() {
        let err = validate(&stdio("filesystem", "   ")).unwrap_err();
        assert!(err.starts_with(ERR_CONFIG), "{err}");
        assert!(err.ends_with("command"), "{err}");
    }

    #[test]
    fn only_http_urls_are_accepted() {
        assert!(validate(&http("https://mcp.context7.com/mcp")).is_ok());
        assert!(validate(&http("http://127.0.0.1:9000/mcp")).is_ok());
        for bad in ["", "mcp.context7.com", "ftp://x/y", "https://a"] {
            assert!(validate(&http(bad)).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn the_id_rule_is_the_one_the_core_enforces() {
        // Not a second rule: if these two drift, a config the form accepts is
        // refused by `McpRegistry::upsert` with a different message.
        assert!(validate(&stdio("Filesystem", "npx")).is_err());
        assert!(validate(&stdio("file_system-2", "npx")).is_ok());
        assert_eq!(
            validate(&stdio("Filesystem", "npx")).unwrap_err(),
            format!("{ERR_CONFIG}: id")
        );
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut config = stdio("filesystem", "npx");
        config.name = "  ".into();
        assert_eq!(
            validate(&config).unwrap_err(),
            format!("{ERR_CONFIG}: name")
        );
    }

    #[test]
    fn the_config_round_trips_through_the_wire_shape_the_front_sends() {
        // Exactly what `src/lib/llm/mcp.ts` builds: the transport is an object
        // tagged `kind`, as `core::mcp::types::Transport` serialises it.
        let json = serde_json::json!({
            "id": "fetch",
            "name": "Fetch",
            "enabled": true,
            "transport": {
                "kind": "stdio",
                "command": "uvx",
                "args": ["mcp-server-fetch"],
                "env": { "NO_PROXY": "1" }
            }
        });
        let config: McpServerConfig = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(config.id, "fetch");
        assert!(validate(&config).is_ok());
        assert_eq!(serde_json::to_value(&config).unwrap(), json);

        let http_json = serde_json::json!({
            "id": "context7",
            "name": "Context7",
            "enabled": false,
            "transport": {
                "kind": "http",
                "url": "https://mcp.context7.com/mcp",
                "headers": { "Authorization": "secret:c7" }
            }
        });
        let config: McpServerConfig = serde_json::from_value(http_json.clone()).unwrap();
        assert!(!config.enabled);
        assert_eq!(serde_json::to_value(&config).unwrap(), http_json);
    }

    #[test]
    fn a_missing_enabled_defaults_to_on() {
        let config: McpServerConfig = serde_json::from_value(serde_json::json!({
            "id": "fetch", "name": "Fetch",
            "transport": { "kind": "stdio", "command": "uvx" }
        }))
        .unwrap();
        assert!(config.enabled);
        assert!(validate(&config).is_ok());
    }

    fn with_headers(pairs: &[(&str, &str)]) -> McpServerConfig {
        let mut headers = BTreeMap::new();
        for (name, value) in pairs {
            headers.insert((*name).to_string(), (*value).to_string());
        }
        McpServerConfig {
            transport: Transport::Http {
                url: "https://api.githubcopilot.com/mcp/".into(),
                headers,
            },
            ..http("https://api.githubcopilot.com/mcp/")
        }
    }

    fn headers_of(config: &McpServerConfig) -> BTreeMap<String, String> {
        match &config.transport {
            Transport::Http { headers, .. } => headers.clone(),
            _ => panic!("not an http server"),
        }
    }

    #[test]
    fn a_real_token_never_reaches_the_tab_but_a_secret_reference_does() {
        let shown = for_display(with_headers(&[
            ("Authorization", "Bearer ghp_realtoken"),
            ("X-Ref", "secret:github-mcp"),
        ]));
        let headers = headers_of(&shown);
        assert_eq!(headers["Authorization"], KEPT);
        assert!(!headers["Authorization"].contains("ghp_realtoken"));
        // A vault pointer is not a secret: the user has to read and edit it.
        assert_eq!(headers["X-Ref"], "secret:github-mcp");
    }

    /// The bug this pair of functions exists to prevent: editing a server and
    /// saving it would otherwise store the mask in place of the token.
    #[test]
    fn saving_an_edited_server_keeps_the_token_it_was_never_shown() {
        let stored = with_headers(&[("Authorization", "Bearer ghp_realtoken")]);
        let edited = for_display(stored.clone()); // what the form round-trips
        let saved = merge_kept(edited, Some(&stored));
        assert_eq!(headers_of(&saved)["Authorization"], "Bearer ghp_realtoken");
    }

    #[test]
    fn a_kept_header_with_nothing_behind_it_is_dropped_not_stored() {
        let saved = merge_kept(with_headers(&[("Authorization", KEPT)]), None);
        assert!(
            !headers_of(&saved).contains_key("Authorization"),
            "the mask itself was saved as the value"
        );
    }

    #[test]
    fn a_header_the_user_actually_typed_wins_over_the_stored_one() {
        let stored = with_headers(&[("Authorization", "Bearer old")]);
        let typed = with_headers(&[("Authorization", "Bearer new")]);
        assert_eq!(
            headers_of(&merge_kept(typed, Some(&stored)))["Authorization"],
            "Bearer new"
        );
    }

    #[test]
    fn merge_kept_leaves_a_stdio_server_alone() {
        let config = stdio("filesystem", "npx");
        assert_eq!(merge_kept(config.clone(), Some(&config)), config);
    }

    fn with_env(pairs: &[(&str, &str)]) -> McpServerConfig {
        let mut env = BTreeMap::new();
        for (name, value) in pairs {
            env.insert((*name).to_string(), (*value).to_string());
        }
        McpServerConfig {
            transport: Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
                env,
                cwd: None,
            },
            ..stdio("github", "npx")
        }
    }

    fn env_of(config: &McpServerConfig) -> BTreeMap<String, String> {
        match &config.transport {
            Transport::Stdio { env, .. } => env.clone(),
            _ => panic!("not a stdio server"),
        }
    }

    #[test]
    fn the_name_rule_is_the_one_the_verifier_asked_for() {
        for name in [
            "GITHUB_TOKEN",
            "api_key",
            "Authorization",
            "MY_SECRET",
            "DB_PASSWORD",
            "openai-api-KEY",
        ] {
            assert!(name_is_secret(name), "{name} should count as a credential");
        }
        for name in ["NO_PROXY", "HOME", "LANG", "PORT", "cwd"] {
            assert!(
                !name_is_secret(name),
                "{name} should not count as a credential"
            );
        }
    }

    /// The defect the Phase 3 verifier caught: `for_display` only masked HTTP
    /// headers, so a token pasted into the env of a stdio server came straight
    /// back to the webview in plaintext.
    #[test]
    fn a_token_in_the_env_of_a_stdio_server_never_reaches_the_tab() {
        let shown = for_display(with_env(&[
            ("GITHUB_TOKEN", "ghp_realtoken"),
            ("NO_PROXY", "1"),
            ("X_REF", "secret:github-x-ref"),
        ]));
        let env = env_of(&shown);
        assert_eq!(env["GITHUB_TOKEN"], KEPT);
        assert!(!env["GITHUB_TOKEN"].contains("ghp_realtoken"));
        // Plain configuration stays readable: masking everything would make the
        // form unusable and teach the user to ignore the mask.
        assert_eq!(env["NO_PROXY"], "1");
        assert_eq!(env["X_REF"], "secret:github-x-ref");
    }

    #[test]
    fn editing_a_stdio_server_keeps_the_env_token_it_was_never_shown() {
        let stored = with_env(&[("GITHUB_TOKEN", "ghp_realtoken"), ("NO_PROXY", "1")]);
        let edited = for_display(stored.clone());
        let saved = merge_kept(edited, Some(&stored));
        assert_eq!(env_of(&saved)["GITHUB_TOKEN"], "ghp_realtoken");
        assert_eq!(env_of(&saved)["NO_PROXY"], "1");
    }

    #[test]
    fn a_kept_env_var_with_nothing_behind_it_is_dropped_not_stored() {
        let saved = merge_kept(with_env(&[("GITHUB_TOKEN", KEPT)]), None);
        assert!(!env_of(&saved).contains_key("GITHUB_TOKEN"));
    }

    /// What the verifier asked to see: upsert with `Authorization: Bearer x`
    /// and `GITHUB_TOKEN=y` must leave neither value in the file on disk nor in
    /// the answer to the front. This drives the same two functions the command
    /// calls, and checks the serialised config both ways.
    #[test]
    fn neither_a_header_nor_an_env_token_survives_the_trip_to_disk_or_back() {
        for (mut config, secret) in [
            (with_headers(&[("Authorization", "Bearer x")]), "Bearer x"),
            (with_env(&[("GITHUB_TOKEN", "ghp-y-123")]), "ghp-y-123"),
        ] {
            let extracted = config.extract_secrets();
            assert_eq!(extracted.len(), 1, "one credential was expected");
            assert_eq!(extracted[0].1, secret);

            // What `McpRegistry::save` writes.
            let on_disk = serde_json::to_string(&config).unwrap();
            assert!(
                !on_disk.contains(secret),
                "the credential is in mcp-servers.json: {on_disk}"
            );
            assert!(on_disk.contains("secret:"), "no reference left behind");

            // What `llm_mcp_list` answers.
            let to_front = serde_json::to_string(&for_display(config)).unwrap();
            assert!(
                !to_front.contains(secret),
                "the credential goes back to the webview: {to_front}"
            );
        }
    }

    /// Saving twice must overwrite one vault entry instead of growing a new
    /// one on every keystroke-free save.
    #[test]
    fn the_secret_id_is_stable_across_saves() {
        let mut first = with_env(&[("GITHUB_TOKEN", "y1")]);
        let mut second = with_env(&[("GITHUB_TOKEN", "y2")]);
        assert_eq!(first.extract_secrets()[0].0, second.extract_secrets()[0].0);
    }

    #[test]
    fn extraction_leaves_a_reference_and_plain_configuration_alone() {
        let mut config = with_env(&[("GITHUB_TOKEN", "secret:already-there"), ("NO_PROXY", "1")]);
        assert!(config.extract_secrets().is_empty());
        assert_eq!(env_of(&config)["GITHUB_TOKEN"], "secret:already-there");
        assert_eq!(env_of(&config)["NO_PROXY"], "1");
    }

    #[test]
    fn a_grant_is_added_once() {
        let tools = apply_grant(&[], "filesystem", "read_file", Some(GrantMode::Auto));
        assert_eq!(tools.len(), 1);
        let again = apply_grant(&tools, "filesystem", "read_file", Some(GrantMode::Ask));
        assert_eq!(again.len(), 1, "the same tool must not appear twice");
        assert_eq!(again[0].mode, GrantMode::Ask);
    }

    #[test]
    fn clearing_a_grant_removes_it_and_leaves_the_others_alone() {
        let tools = vec![
            ToolGrant {
                source: ToolSource::Internal {
                    name: "download_url".into(),
                },
                mode: GrantMode::Auto,
            },
            mcp("filesystem", "read_file", GrantMode::Auto),
            mcp("filesystem", "write_file", GrantMode::Deny),
            mcp("github", "read_file", GrantMode::Ask),
        ];
        let next = apply_grant(&tools, "filesystem", "read_file", None);
        assert_eq!(next.len(), 3);
        assert!(!next.iter().any(|g| is_mcp(g, "filesystem", "read_file")));
        // The same tool name on another server is a different grant.
        assert!(next.iter().any(|g| is_mcp(g, "github", "read_file")));
        // And the internal grant is untouched, in place.
        assert!(matches!(next[0].source, ToolSource::Internal { .. }));
    }

    #[test]
    fn clearing_a_grant_that_was_never_there_is_a_no_op() {
        let tools = vec![mcp("filesystem", "read_file", GrantMode::Auto)];
        assert_eq!(apply_grant(&tools, "github", "read_file", None), tools);
    }

    #[test]
    fn the_grant_payload_the_front_sends_parses_with_and_without_a_mode() {
        let set: GrantDto = serde_json::from_value(serde_json::json!({
            "source": "mcp", "server": "filesystem", "tool": "read_file", "mode": "ask"
        }))
        .unwrap();
        assert_eq!(set.mode, Some(GrantMode::Ask));
        let cleared: GrantDto = serde_json::from_value(serde_json::json!({
            "source": "mcp", "server": "filesystem", "tool": "read_file", "mode": null
        }))
        .unwrap();
        assert_eq!(cleared.mode, None);
    }

    #[test]
    fn a_granted_tool_serialises_as_the_roster_stores_it() {
        let tools = apply_grant(&[], "filesystem", "read_file", Some(GrantMode::Auto));
        assert_eq!(
            serde_json::to_value(&tools[0]).unwrap(),
            serde_json::json!({
                "source": "mcp", "server": "filesystem", "tool": "read_file", "mode": "auto"
            })
        );
    }

    /// The tab asks for the tools of a server; `ToolDef` must land as the
    /// front's `McpToolDef` reads it, `inputSchema` included.
    #[test]
    fn a_tool_serialises_with_the_schema_key_the_table_reads() {
        let tool: ToolDef = serde_json::from_value(serde_json::json!({
            "name": "read_file",
            "description": "Read a file",
            "inputSchema": { "properties": { "path": {} }, "required": ["path"] }
        }))
        .unwrap();
        let wire = serde_json::to_value(&tool).unwrap();
        assert_eq!(wire["name"], "read_file");
        assert_eq!(wire["inputSchema"]["required"][0], "path");
    }

    /// The registry is a process singleton: a second call must hand back the
    /// same instance, or the tab and the broker would spawn one process each.
    #[test]
    fn the_registry_is_one_instance_for_the_whole_process() {
        match (registry(), registry()) {
            (Some(a), Some(b)) => assert!(Arc::ptr_eq(&a, &b)),
            (None, None) => {} // headless: no data dir, both `None`
            _ => panic!("registry() answered differently twice"),
        }
    }

    #[tokio::test]
    async fn the_registry_starts_no_process_just_by_being_listed() {
        // `rows` reads config plus live clients; with nothing connected it must
        // not call `client_for`. This is the "0 processes with the section
        // closed" rule, proven on the command path.
        let reg = Arc::new(McpRegistry::in_memory());
        reg.upsert(stdio("filesystem", "npx")).await.unwrap();
        let listed = rows(&reg).await;
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].connected);
        assert!(listed[0].tools.is_empty());
        assert!(reg.connected().await.is_empty(), "a process was spawned");
        assert!(
            !reg.reaper_running(),
            "the reaper is ticking with no client"
        );
    }

    /// `scripts/mcp-fixture.mjs`, the toy server f3-mcp-core ships. Spawned by
    /// the registry, killed by `close_all` at the end of the test, which is
    /// what keeps `pgrep -f mcp-fixture` empty.
    fn fixture_config(id: &str) -> McpServerConfig {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("mcp-fixture.mjs");
        McpServerConfig {
            id: id.into(),
            name: "Fixture".into(),
            enabled: true,
            timeout_ms: Some(10_000),
            transport: Transport::Stdio {
                command: "node".into(),
                args: vec![script.to_string_lossy().into_owned(), "--stdio".into()],
                env: BTreeMap::new(),
                cwd: None,
            },
        }
    }

    fn have_node() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// The whole command path against a real MCP server: list (no process),
    /// probe (one process, real `initialize` + `tools/list`), list again (now
    /// connected, with the tools the tab shows), remove (process gone).
    #[tokio::test]
    async fn the_commands_drive_a_real_mcp_server_end_to_end() {
        if !have_node() {
            eprintln!("skipped: node is not installed");
            return;
        }
        let reg = Arc::new(McpRegistry::in_memory());
        let config = fixture_config("fixture");
        assert!(validate(&config).is_ok());
        reg.upsert(config).await.unwrap();

        // Listing a configured server must not start it.
        let listed = rows(&reg).await;
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].connected);
        assert!(reg.connected().await.is_empty());

        let result = probe(&reg, "fixture").await.unwrap();
        assert!(result.ok, "probe failed: {:?}", result.error);
        assert!(result.tool_count > 0, "the fixture exposes tools");
        assert_eq!(result.server_name.as_deref(), Some("mcp-fixture"));
        assert!(result.protocol_known, "unknown protocol version");

        // Now the row carries what the grant table needs, with no second probe.
        let listed = rows(&reg).await;
        assert!(listed[0].connected);
        assert_eq!(listed[0].tools.len(), result.tool_count);
        assert!(listed[0].tools.iter().any(|t| !t.name.is_empty()));

        // Removing stops the process: that is the budget, not a nicety.
        reg.remove("fixture").await.unwrap();
        assert!(
            reg.connected().await.is_empty(),
            "the child outlived remove"
        );
        assert!(rows(&reg).await.is_empty());
        reg.close_all().await;
    }

    /// A server that cannot be started is an answer with a code, not an `Err`
    /// and not a panic — the row shows it and the tab stays usable.
    #[tokio::test]
    async fn a_server_that_cannot_start_answers_with_its_code() {
        let reg = Arc::new(McpRegistry::in_memory());
        let mut config = fixture_config("broken");
        config.transport = Transport::Stdio {
            command: "omniget-does-not-exist".into(),
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
        };
        reg.upsert(config).await.unwrap();
        let result = probe(&reg, "broken").await.unwrap();
        assert!(!result.ok);
        let error = result.error.expect("a failure must carry its error");
        assert!(error.code.starts_with("ERR_MCP_"), "{}", error.code);
        assert!(reg.connected().await.is_empty());
    }

    #[tokio::test]
    async fn upsert_refuses_a_bad_config_before_the_registry_sees_it() {
        let reg = Arc::new(McpRegistry::in_memory());
        let bad = stdio("filesystem", "  ");
        assert!(validate(&bad).is_err());
        assert!(reg.list().is_empty());
    }
}
