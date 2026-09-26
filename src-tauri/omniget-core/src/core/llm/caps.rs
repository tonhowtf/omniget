//! What each runtime really supports, so the UI and the bot's capability
//! manifest never promise more than the executable can do (spec 02,
//! "Contrato de runtime"). Owner: worker W4.
//!
//! [`for_runtime`] is synchronous and conservative: it answers from the last
//! probe of that executable when there is one, else from what the runtime
//! kind guarantees on its own. [`probe`] runs the installed binary
//! (`--version`, `--help`) and caches what it found; flags are read from the
//! help text of the version on this machine, never assumed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::agent::RuntimeKind;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCaps {
    /// Human label of the runtime and the version detected, when known.
    pub label: String,
    pub version: Option<String>,
    /// The provider keeps the session and we can resume it by handle.
    pub native_resume: bool,
    /// We rebuild the session by replaying our own transcript.
    pub history_replay: bool,
    pub streaming: bool,
    /// New input can be sent while a turn runs.
    pub steering: bool,
    pub attachments: bool,
    /// OmniGet's broker executes the tools (native runtime).
    pub managed_tools: bool,
    /// OmniGet's assistant tools reach the runtime through an MCP projection.
    pub mcp_projection: bool,
    /// Permission requests round-trip to the user while the turn waits.
    pub interactive_permissions: bool,
    pub usage_reporting: bool,
    /// The runtime brings its own web search/fetch (e.g. Claude Code).
    pub builtin_web: bool,
    /// Honest notes shown in "advanced details".
    #[serde(default)]
    pub limits: Vec<String>,
    /// Long flags the installed executable lists in its help (probe only).
    #[serde(default)]
    pub flags: Vec<String>,
    /// True when this answer comes from running the executable.
    #[serde(default)]
    pub probed: bool,
    /// The executable was looked for and is not installed.
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

/// The last probe of `cli` (`claude`, `codex`), if any ran in this process.
pub fn cached_cli(cli: &str) -> Option<RuntimeCaps> {
    CACHE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&format!("cli:{}", cli.to_ascii_lowercase())).cloned())
}

/// Capabilities of a runtime kind: the probed answer when there is one, else
/// the conservative one.
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

/// Every `--long-flag` the help text mentions, deduplicated, in order.
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

/// `2.1.282 (Claude Code)` → `2.1.282`; `codex-cli 0.50.0` → `0.50.0`.
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

/// Pure: what a Claude Code / Codex binary of this version with these flags
/// can do.
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

/// A CLI that is not installed: nothing is promised.
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

/// Runs `<binary> --version` and `--help` and caches the result for `cli`.
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
    let caps = cli_caps(cli, version, parse_help_flags(&help));
    remember(&format!("cli:{cli}"), &caps);
    caps
}

/// Probes the executable behind `runtime` (finds it on the PATH first).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines copied from `claude --help` of 2.1.282 (24/09/2026).
    const CLAUDE_HELP: &str = r#"
  --allowedTools, --allowed-tools <tools...>
  --disallowedTools, --disallowed-tools <tools...>
  --include-partial-messages
  --mcp-config <configs...>             Load MCP servers from JSON files or
  --permission-prompts <target>         Who answers permission prompts with
                                        --print: "host" (the SDK host or
                                        --permission-prompt-tool) or "none"
  -r, --resume [value]                  Resume a conversation by session ID, or
  --strict-mcp-config                   Only use MCP servers from --mcp-config,
  --tools <tools...>                    Specify the list of available tools from
"#;

    #[test]
    fn flags_and_version_come_from_the_installed_help() {
        let flags = parse_help_flags(CLAUDE_HELP);
        for f in [
            "--resume",
            "--mcp-config",
            "--tools",
            "--disallowedTools",
            "--permission-prompt-tool",
            "--strict-mcp-config",
        ] {
            assert!(flags.contains(&f.to_string()), "{f} in {flags:?}");
        }
        assert_eq!(
            parse_version("2.1.282 (Claude Code)").as_deref(),
            Some("2.1.282")
        );
        assert_eq!(parse_version("codex-cli 0.50.0").as_deref(), Some("0.50.0"));
        assert_eq!(parse_version("no version here"), None);
        let caps = cli_caps("claude", Some("2.1.282".into()), flags);
        assert!(caps.native_resume && caps.mcp_projection && caps.interactive_permissions);
        assert!(caps.builtin_web && !caps.steering);
    }

    #[test]
    fn an_old_claude_without_resume_says_so() {
        let caps = cli_caps("claude", Some("1.0.0".into()), vec!["--print".into()]);
        assert!(!caps.native_resume);
        assert!(!caps.interactive_permissions);
        assert!(caps.limits.iter().any(|l| l.contains("--resume")));
    }

    #[test]
    fn a_missing_cli_promises_nothing() {
        let caps = missing_cli("codex");
        assert!(caps.missing && !caps.streaming && !caps.native_resume && !caps.history_replay);
        assert!(caps.limits[0].contains("not installed"));
    }

    /// Opt-in: probes the real `claude` on this machine.
    #[tokio::test]
    #[ignore = "runs the installed claude --help"]
    async fn live_probe_of_the_installed_claude() {
        let caps = probe(&RuntimeKind::Cli {
            cli: "claude".into(),
            account: String::new(),
        })
        .await;
        eprintln!("{caps:#?}");
        assert!(caps.version.is_some());
    }
}
