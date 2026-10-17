use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use omniget_core::core::canvas::model::CanvasNode;
use omniget_core::core::llm::drivers::acp::McpServerSpec;
use omniget_core::core::llm::drivers::claude::protocol::mcp_config_json;
use omniget_core::core::llm::drivers::codex::launch::{with_mcp, LaunchSpec};
use omniget_core::core::mcp::types::{resolve_env, resolve_headers, McpServerConfig, Transport};
use serde::Serialize;
use serde_json::Value;

use crate::commands::llm::mcp::{probe, registry};

pub const DATA_KEY: &str = "mcpServers";
const CONFIG_FILE: &str = "mcp.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub transport: &'static str,
    pub target: String,
    pub enabled: bool,
    pub connected: bool,
    pub tool_count: Option<usize>,
}

fn need() -> Result<std::sync::Arc<omniget_core::core::mcp::McpRegistry>, String> {
    registry().ok_or_else(|| "ERR_MCP_NO_STORE: no application data directory".to_string())
}

fn target_of(cfg: &McpServerConfig) -> (&'static str, String) {
    match &cfg.transport {
        Transport::Stdio { command, .. } => {
            let program = std::path::Path::new(command.trim())
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            ("stdio", program)
        }
        Transport::Http { url, .. } => {
            let bare = url.split(['?', '#']).next().unwrap_or("");
            let host = bare
                .split_once("://")
                .map(|(scheme, rest)| {
                    let host = rest.split('/').next().unwrap_or("");
                    let host = host.rsplit('@').next().unwrap_or(host);
                    format!("{scheme}://{host}")
                })
                .unwrap_or_default();
            ("http", host)
        }
    }
}

pub async fn servers() -> Result<Vec<ServerView>, String> {
    let reg = need()?;
    let connected = reg.connected().await;
    let mut out = Vec::new();
    for cfg in reg.list() {
        let live = connected.contains(&cfg.id);
        let tool_count = if live {
            match reg.client_for(&cfg.id).await {
                Ok(c) => c.tools().await.ok().map(|t| t.len()),
                Err(_) => None,
            }
        } else {
            None
        };
        let (transport, target) = target_of(&cfg);
        out.push(ServerView {
            id: cfg.id.clone(),
            name: cfg.name.clone(),
            transport,
            target,
            enabled: cfg.enabled,
            connected: live,
            tool_count,
        });
    }
    Ok(out)
}

pub async fn set_enabled(id: &str, enabled: bool) -> Result<Vec<ServerView>, String> {
    let reg = need()?;
    let mut cfg = reg.get(id).ok_or_else(|| {
        format!("CANVAS_NOT_FOUND: no MCP server '{id}' (see canvas_mcp_servers)")
    })?;
    if cfg.enabled != enabled {
        cfg.enabled = enabled;
        reg.upsert(cfg)
            .await
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
    }
    if !enabled {
        reg.disconnect(id).await;
    }
    servers().await
}

pub async fn test(id: &str) -> Result<Value, String> {
    let reg = need()?;
    if reg.get(id).is_none() {
        return Err(format!(
            "CANVAS_NOT_FOUND: no MCP server '{id}' (see canvas_mcp_servers)"
        ));
    }
    let r = probe(&reg, id).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

pub fn known(ids: &[String]) -> Result<Vec<String>, String> {
    let reg = need()?;
    let all: Vec<String> = reg.list().into_iter().map(|c| c.id).collect();
    let missing: Vec<&String> = ids.iter().filter(|i| !all.contains(i)).collect();
    if !missing.is_empty() {
        return Err(format!(
            "CANVAS_NOT_FOUND: no MCP server {} (configured: {})",
            missing
                .iter()
                .map(|m| format!("'{m}'"))
                .collect::<Vec<_>>()
                .join(", "),
            if all.is_empty() {
                "none".to_string()
            } else {
                all.join(", ")
            }
        ));
    }
    Ok(ids.to_vec())
}

pub fn attached(node: &CanvasNode) -> Vec<String> {
    node.data
        .get(DATA_KEY)
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn key_of(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn specs(ids: &[String]) -> Vec<McpServerSpec> {
    let Some(reg) = registry() else {
        return Vec::new();
    };
    let lookup = reg.lookup();
    let mut out = Vec::new();
    for id in ids {
        let Some(cfg) = reg.get(id).filter(|c| c.enabled) else {
            continue;
        };
        let name = key_of(&cfg.id);
        match &cfg.transport {
            Transport::Stdio {
                command, args, env, ..
            } => match resolve_env(env, &lookup) {
                Ok(env) => out.push(McpServerSpec::Stdio {
                    name,
                    command: command.clone(),
                    args: args.clone(),
                    env: env.into_iter().collect(),
                }),
                Err(e) => tracing::warn!("[canvas.mcp] {id}: {}", e.message),
            },
            Transport::Http { url, headers } => match resolve_headers(headers, &lookup) {
                Ok(headers) => out.push(McpServerSpec::Http {
                    name,
                    url: url.clone(),
                    headers,
                }),
                Err(e) => tracing::warn!("[canvas.mcp] {id}: {}", e.message),
            },
        }
    }
    out
}

pub(super) fn run_dir(node_id: &str) -> Option<PathBuf> {
    let safe = key_of(node_id);
    omniget_core::core::paths::app_data_dir().map(|d| d.join("canvas").join("run").join(safe))
}

pub(super) fn write_private(path: &std::path::Path, body: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(path);
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(body)?;
        f.sync_all()
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body)
    }
}

pub fn forget(node_id: &str) {
    if let Some(dir) = run_dir(node_id) {
        let _ = std::fs::remove_file(dir.join(CONFIG_FILE));
    }
}

pub fn for_launch(agent: &str, node: &CanvasNode) -> (Vec<String>, HashMap<String, String>) {
    let ids = attached(node);
    if ids.is_empty() || !matches!(agent, "claude" | "codex") {
        forget(&node.id);
        return (Vec::new(), HashMap::new());
    }
    let list = specs(&ids);
    if list.is_empty() {
        forget(&node.id);
        return (Vec::new(), HashMap::new());
    }
    match agent {
        "claude" => {
            let Some(path) = run_dir(&node.id).map(|d| d.join(CONFIG_FILE)) else {
                return (Vec::new(), HashMap::new());
            };
            let body = serde_json::to_vec_pretty(&mcp_config_json(&list)).unwrap_or_default();
            match write_private(&path, &body) {
                Ok(()) => (
                    vec!["--mcp-config".into(), path.to_string_lossy().into_owned()],
                    HashMap::new(),
                ),
                Err(e) => {
                    tracing::warn!("[canvas.mcp] {}: {e}", path.display());
                    (Vec::new(), HashMap::new())
                }
            }
        }
        _ => {
            let spec = with_mcp(
                LaunchSpec {
                    program: PathBuf::new(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    scrub: Vec::new(),
                    cwd: None,
                    codex_home: None,
                },
                &list,
            );
            (spec.args, spec.env.into_iter().collect())
        }
    }
}
