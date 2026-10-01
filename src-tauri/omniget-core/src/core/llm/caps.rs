use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::agent::RuntimeKind;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCaps {
    pub label: String,
    pub version: Option<String>,
    pub native_resume: bool,
    pub history_replay: bool,
    pub streaming: bool,
    pub steering: bool,
    pub attachments: bool,
    pub managed_tools: bool,
    pub mcp_projection: bool,
    pub interactive_permissions: bool,
    pub usage_reporting: bool,
    pub builtin_web: bool,
    #[serde(default)]
    pub limits: Vec<String>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub permission_modes: Vec<String>,
    #[serde(default)]
    pub app_server: bool,
    #[serde(default)]
    pub probed: bool,
    #[serde(default)]
    pub missing: bool,
}

impl RuntimeCaps {
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

static CACHE: RwLock<Option<HashMap<String, RuntimeCaps>>> = RwLock::new(None);

fn cache_key(runtime: &RuntimeKind) -> String {
    match runtime {
        RuntimeKind::Native => "native".into(),
        RuntimeKind::Cli { cli, .. } => format!("cli:{}", cli.to_ascii_lowercase()),
        RuntimeKind::Acp { command, .. } => format!("acp:{command}"),
    }
}

fn remember(key: &str, caps: &RuntimeCaps) {
    let mut guard = CACHE.write().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(key.to_string(), caps.clone());
}

pub fn cached_cli(cli: &str) -> Option<RuntimeCaps> {
    CACHE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&format!("cli:{}", cli.to_ascii_lowercase())).cloned())
}

pub fn for_runtime(runtime: &RuntimeKind) -> RuntimeCaps {
    if let Some(found) = CACHE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&cache_key(runtime)).cloned())
    {
        return found;
    }
    conservative(runtime)
}

fn conservative(runtime: &RuntimeKind) -> RuntimeCaps {
    match runtime {
        RuntimeKind::Native => RuntimeCaps {
            label: "native".into(),
            history_replay: true,
            streaming: true,
            managed_tools: true,
            interactive_permissions: true,
            usage_reporting: true,
            limits: vec!["The conversation is rebuilt from OmniGet's own transcript each turn.".into()],
            ..Default::default()
        },
        RuntimeKind::Cli { cli, .. } => RuntimeCaps {
            label: cli.clone(),
            history_replay: true,
            streaming: true,
            limits: vec!["Not probed yet: capabilities are the conservative defaults.".into()],
            ..Default::default()
        },
        RuntimeKind::Acp { command, .. } => RuntimeCaps {
            label: command.clone(),
            history_replay: true,
            streaming: true,
            interactive_permissions: true,
            mcp_projection: true,
            limits: vec![
                "The agent's session lives while OmniGet is open; after a restart the transcript is replayed unless the agent announces session loading.".into(),
            ],
            ..Default::default()
        },
    }
}

pub fn parse_help_flags(help: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in help.split(|c: char| {
        c.is_whitespace() || c == ',' || c == '(' || c == ')' || c == '"' || c == '`'
    }) {
        let Some(rest) = raw.strip_prefix("--") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if name.len() < 2 {
            continue;
        }
        let flag = format!("--{name}");
        if !out.contains(&flag) {
            out.push(flag);
        }
    }
    out
}

pub fn parse_help_choices(help: &str, flag: &str) -> Vec<String> {
    let Some(at) = help.find(&format!("{flag} ")) else {
        return Vec::new();
    };
    let rest = &help[at..];
    let Some(open) = rest.find("(choices:") else {
        return Vec::new();
    };
    let next_flag = rest[flag.len()..]
        .find("\n  -")
        .map(|i| i + flag.len())
        .unwrap_or(rest.len());
    if open > next_flag {
        return Vec::new();
    }
    let body = &rest[open + "(choices:".len()..];
    let body = &body[..body.find(')').unwrap_or(body.len())];
    body.split(',')
        .map(|c| c.trim().trim_matches('"').to_string())
        .filter(|c| !c.is_empty() && !c.contains(' '))
        .collect()
}

pub fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| {
            let parts: Vec<&str> = t.split('.').collect();
            parts.len() >= 2
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
}

pub fn cli_caps(cli: &str, version: Option<String>, flags: Vec<String>) -> RuntimeCaps {
    let has = |f: &str| flags.iter().any(|x| x == f);
    let mut limits = Vec::new();
    let caps = match cli {
        "claude" => {
            let native_resume = has("--resume");
            let mcp = has("--mcp-config");
            let prompt_tool = has("--permission-prompt-tool");
            if !native_resume {
                limits.push(
                    "This Claude Code has no --resume: each turn replays the transcript.".into(),
                );
            }
            if !prompt_tool {
                limits.push("Permission prompts cannot be routed to OmniGet: anything that would ask is denied.".into());
            }
            limits.push(
                "One process per turn (--print): new input waits for the running turn.".into(),
            );
            RuntimeCaps {
                label: "claude".into(),
                version,
                native_resume,
                history_replay: true,
                streaming: has("--include-partial-messages"),
                steering: false,
                attachments: false,
                managed_tools: false,
                mcp_projection: mcp,
                interactive_permissions: prompt_tool && mcp,
                usage_reporting: true,
                builtin_web: true,
                limits,
                flags,
                permission_modes: Vec::new(),
                app_server: false,
                probed: true,
                missing: false,
            }
        }
        "codex" => {
            let native_resume = has("--resume") || flags.iter().any(|f| f == "--last");
            limits.push("Codex runs one non-interactive exec per turn; permissions follow its sandbox, not OmniGet prompts.".into());
            RuntimeCaps {
                label: "codex".into(),
                version,
                native_resume,
                history_replay: true,
                streaming: true,
                usage_reporting: true,
                limits,
                flags,
                probed: true,
                ..Default::default()
            }
        }
        other => RuntimeCaps {
            label: other.into(),
            version,
            history_replay: true,
            flags,
            probed: true,
            ..Default::default()
        },
    };
    caps
}

pub fn with_app_server(caps: &mut RuntimeCaps) {
    caps.app_server = true;
    caps.native_resume = true;
    caps.interactive_permissions = true;
    caps.limits
        .retain(|l| !l.starts_with("Codex runs one non-interactive exec"));
    caps.limits.push(
        "Codex runs over its app-server: the conversation is resumed, and each command or edit it wants approved is asked in OmniGet."
            .into(),
    );
}

pub fn missing_cli(cli: &str) -> RuntimeCaps {
    RuntimeCaps {
        label: cli.into(),
        missing: true,
        probed: true,
        limits: vec![format!(
            "`{cli}` is not installed on this computer, so this runtime cannot run turns here."
        )],
        ..Default::default()
    }
}

async fn run_capture(binary: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        crate::core::process::command(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

async fn runs_ok(binary: &Path, args: &[&str]) -> bool {
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            crate::core::process::command(binary)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status(),
        )
        .await,
        Ok(Ok(status)) if status.success()
    )
}

pub async fn probe_binary(cli: &str, binary: &Path) -> RuntimeCaps {
    let version = run_capture(binary, &["--version"])
        .await
        .and_then(|t| parse_version(&t));
    let mut help = run_capture(binary, &["--help"]).await.unwrap_or_default();
    if cli == "codex" {
        if let Some(exec) = run_capture(binary, &["exec", "--help"]).await {
            help.push('\n');
            help.push_str(&exec);
        }
    }
    let mut caps = cli_caps(cli, version, parse_help_flags(&help));
    caps.permission_modes = parse_help_choices(&help, "--permission-mode");
    if cli == "codex"
        && caps
            .version
            .as_deref()
            .is_some_and(super::cli_runtime::codex_app_server::version_supports)
        && runs_ok(binary, &["app-server", "--help"]).await
    {
        with_app_server(&mut caps);
    }
    remember(&format!("cli:{cli}"), &caps);
    caps
}

pub async fn probe(runtime: &RuntimeKind) -> RuntimeCaps {
    match runtime {
        RuntimeKind::Native => conservative(runtime),
        RuntimeKind::Cli { cli, .. } => {
            let cli = cli.to_ascii_lowercase();
            match crate::core::dependencies::find_tool(&cli).await {
                Some(path) => probe_binary(&cli, &path).await,
                None => {
                    let caps = missing_cli(&cli);
                    remember(&format!("cli:{cli}"), &caps);
                    caps
                }
            }
        }
        RuntimeKind::Acp { command, .. } => {
            let found = crate::core::dependencies::find_tool(command).await;
            let mut caps = conservative(runtime);
            caps.probed = true;
            if found.is_none() {
                caps = RuntimeCaps {
                    label: command.clone(),
                    missing: true,
                    probed: true,
                    limits: vec![format!("`{command}` is not installed on this computer.")],
                    ..Default::default()
                };
            }
            remember(&cache_key(runtime), &caps);
            caps
        }
    }
}
