//! How an instance becomes a `codex app-server` process: binary, argv,
//! `CODEX_HOME`, scrubbed environment, and the access-mode table.
//!
//! `CODEX_HOME` comes from the account (`cli_runtime/accounts.rs`): the
//! instance's `config_dir`, or the account's directory looked up by
//! `account_id`. Empty means the CLI's own default profile (`~/.codex`), the
//! login the user already has in a terminal. This module never opens anything
//! inside that directory (no `auth.json`, no `config.toml`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::super::super::cli_runtime::accounts::{self, AccountStore, CliKind};
use super::super::{AccessMode, DriverInstance};
use super::protocol::{
    ApprovalsReviewer, AskForApproval, AskForApprovalKnown, SandboxMode, SandboxPolicy,
};

/// Codex's own knobs for one OmniGet access mode.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeConfig {
    pub approval_policy: AskForApproval,
    /// `thread/start|resume|fork.sandbox`.
    pub sandbox: SandboxMode,
    /// `turn/start.sandboxPolicy`.
    pub sandbox_policy: SandboxPolicy,
    /// Always sent: omitting it on resume keeps the previous reviewer, so an
    /// `auto_review` thread would stay auto forever (T3, 01-server.md §3.2).
    pub reviewer: ApprovalsReviewer,
}

/// The T3 table (estudo 76, 01-server.md §3.2):
///
/// | mode | approvalPolicy | sandbox | turn sandboxPolicy | reviewer |
/// |---|---|---|---|---|
/// | approval-required | untrusted | read-only | readOnly | user |
/// | auto-accept-edits | on-request | workspace-write | workspaceWrite | user |
/// | auto | on-request | workspace-write | workspaceWrite | auto_review |
/// | full-access | never | danger-full-access | dangerFullAccess | user |
///
/// `full-access` is the only path to `danger-full-access`, and only because
/// the user picked that mode for the thread (the safe default is the first
/// row: `AccessMode::default()` is `approval-required`).
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

/// Everything needed to spawn the app-server of one instance.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Set on the child.
    pub env: BTreeMap<String, String>,
    /// Removed from the child (API keys that would pick the billing account).
    pub scrub: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// The `CODEX_HOME` in use, `None` for the CLI default.
    pub codex_home: Option<PathBuf>,
}

/// `CODEX_HOME` of an instance. Pure except for the account lookup, which is
/// a read of `accounts.json` (paths only, never a credential).
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

/// `~` is not expanded for a spawned process (no shell), so do it here.
pub fn expand_home(path: &str) -> String {
    if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

/// Builds the spawn spec. `program` is the resolved binary.
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
    // An API key set on purpose in the instance env is the user's choice; the
    // OmniGet process's own keys are not.
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

/// Env var that carries the first MCP bearer token (never on argv).
pub const MCP_TOKEN_ENV: &str = "OMNIGET_MCP_TOKEN";

fn toml_str(v: &str) -> String {
    // A JSON string is a valid TOML basic string for what we pass here.
    serde_json::to_string(v).unwrap_or_else(|_| "\"\"".into())
}

/// Adds the host's MCP servers (the embedded OmniGet MCP with the session
/// token) as `-c mcp_servers.<name>.*` overrides. A `Bearer` header goes
/// through `bearer_token_env_var` so the token stays out of `ps`; other
/// headers become `http_headers`. SSE has no Codex transport and is skipped.
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

/// `initialize` params. `experimentalApi` is what unlocks
/// `turn/start.collaborationMode` (plan mode) on 0.156.
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
