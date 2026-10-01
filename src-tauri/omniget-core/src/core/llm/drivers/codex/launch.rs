use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::super::super::cli_runtime::accounts::{self, AccountStore, CliKind};
use super::super::{AccessMode, DriverInstance};
use super::protocol::{
    ApprovalsReviewer, AskForApproval, AskForApprovalKnown, SandboxMode, SandboxPolicy,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ModeConfig {
    pub approval_policy: AskForApproval,
    pub sandbox: SandboxMode,
    pub sandbox_policy: SandboxPolicy,
    pub reviewer: ApprovalsReviewer,
}

pub fn mode_config(mode: AccessMode) -> ModeConfig {
    let known = |k| AskForApproval::Known(k);
    match mode {
        AccessMode::ApprovalRequired => ModeConfig {
            approval_policy: known(AskForApprovalKnown::Untrusted),
            sandbox: SandboxMode::ReadOnly,
            sandbox_policy: SandboxPolicy::ReadOnly {
                network_access: None,
            },
            reviewer: ApprovalsReviewer::User,
        },
        AccessMode::AutoAcceptEdits => ModeConfig {
            approval_policy: known(AskForApprovalKnown::OnRequest),
            sandbox: SandboxMode::WorkspaceWrite,
            sandbox_policy: workspace_write(),
            reviewer: ApprovalsReviewer::User,
        },
        AccessMode::Auto => ModeConfig {
            approval_policy: known(AskForApprovalKnown::OnRequest),
            sandbox: SandboxMode::WorkspaceWrite,
            sandbox_policy: workspace_write(),
            reviewer: ApprovalsReviewer::AutoReview,
        },
        AccessMode::FullAccess => ModeConfig {
            approval_policy: known(AskForApprovalKnown::Never),
            sandbox: SandboxMode::DangerFullAccess,
            sandbox_policy: SandboxPolicy::DangerFullAccess {},
            reviewer: ApprovalsReviewer::User,
        },
    }
}

fn workspace_write() -> SandboxPolicy {
    SandboxPolicy::WorkspaceWrite {
        exclude_slash_tmp: None,
        exclude_tmpdir_env_var: None,
        network_access: None,
        writable_roots: None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub scrub: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
}

pub fn codex_home(instance: &DriverInstance, store: Option<&AccountStore>) -> Option<PathBuf> {
    if let Some(dir) = instance
        .config_dir
        .as_ref()
        .filter(|d| !d.as_os_str().is_empty())
    {
        return Some(dir.clone());
    }
    if let Some(explicit) = instance
        .env
        .iter()
        .find(|v| v.name == "CODEX_HOME" && !v.value.is_empty())
    {
        return Some(PathBuf::from(expand_home(&explicit.value)));
    }
    let id = instance.account_id.as_deref()?;
    let account = store?.get(id)?;
    if account.cli != CliKind::Codex || account.config_dir.as_os_str().is_empty() {
        return None;
    }
    Some(account.config_dir)
}

pub fn expand_home(path: &str) -> String {
    if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

pub fn launch_spec(
    instance: &DriverInstance,
    program: PathBuf,
    cwd: Option<PathBuf>,
    codex_home: Option<PathBuf>,
) -> LaunchSpec {
    let mut env = BTreeMap::new();
    for var in &instance.env {
        if var.name.is_empty() || var.name == "CODEX_HOME" {
            continue;
        }
        env.insert(var.name.clone(), var.value.clone());
    }
    if let Some(home) = &codex_home {
        env.insert("CODEX_HOME".into(), home.display().to_string());
    }
    let mut args = vec!["app-server".to_string()];
    args.extend(instance.args.iter().cloned());
    let scrub = accounts::SCRUB_ENV
        .iter()
        .filter(|k| !env.contains_key(**k))
        .map(|k| k.to_string())
        .collect();
    LaunchSpec {
        program,
        args,
        env,
        scrub,
        cwd,
        codex_home,
    }
}

pub const MCP_TOKEN_ENV: &str = "OMNIGET_MCP_TOKEN";

fn toml_str(v: &str) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "\"\"".into())
}

pub fn with_mcp(mut spec: LaunchSpec, servers: &[super::super::acp::McpServerSpec]) -> LaunchSpec {
    use super::super::acp::McpServerSpec as S;
    let mut n = 0usize;
    for server in servers {
        let (name, pairs): (&String, Vec<(String, String)>) = match server {
            S::Http { name, url, headers } => {
                let mut pairs = vec![
                    ("url".to_string(), toml_str(url)),
                    ("tool_timeout_sec".to_string(), "300".to_string()),
                ];
                let mut extra = Vec::new();
                for (k, v) in headers {
                    let bearer = v
                        .strip_prefix("Bearer ")
                        .filter(|_| k.eq_ignore_ascii_case("authorization"));
                    match bearer {
                        Some(token) => {
                            let var = if n == 0 {
                                MCP_TOKEN_ENV.to_string()
                            } else {
                                format!("{MCP_TOKEN_ENV}_{n}")
                            };
                            n += 1;
                            spec.env.insert(var.clone(), token.to_string());
                            pairs.push(("bearer_token_env_var".into(), toml_str(&var)));
                        }
                        None => extra.push(format!("{}={}", toml_str(k), toml_str(v))),
                    }
                }
                if !extra.is_empty() {
                    pairs.push(("http_headers".into(), format!("{{{}}}", extra.join(","))));
                }
                (name, pairs)
            }
            S::Stdio {
                name,
                command,
                args,
                env,
            } => {
                let mut pairs = vec![("command".to_string(), toml_str(command))];
                pairs.push((
                    "args".into(),
                    format!(
                        "[{}]",
                        args.iter()
                            .map(|a| toml_str(a))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                ));
                if !env.is_empty() {
                    pairs.push((
                        "env".into(),
                        format!(
                            "{{{}}}",
                            env.iter()
                                .map(|(k, v)| format!("{}={}", toml_str(k), toml_str(v)))
                                .collect::<Vec<_>>()
                                .join(",")
                        ),
                    ));
                }
                (name, pairs)
            }
            S::Sse { .. } => continue,
        };
        let key: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if key.is_empty() {
            continue;
        }
        for (k, v) in pairs {
            spec.args.push("-c".into());
            spec.args.push(format!("mcp_servers.{key}.{k}={v}"));
        }
    }
    spec
}

pub fn initialize_params() -> Value {
    json!({
        "clientInfo": {
            "name": "omniget",
            "title": "OmniGet",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": { "experimentalApi": true },
    })
}
