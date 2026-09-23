//! Which ACP agents OmniGet knows how to start, and how each one becomes a
//! [`DriverInstance`] of the `acp` driver (`id = "acp-<agent>"`).
//!
//! Resolution of the launch command, first hit wins:
//! 1. the instance's own `command`/`args` (a custom agent the user added);
//! 2. the agent installed from the ACP registry by clitools
//!    (`<app_data>/agents/acp-installed.json`: npx/uvx command or a verified
//!    binary);
//! 3. the agent's own binary on the PATH with its ACP flag (table below).
//!
//! Auth: nothing is read from disk. When the agent answers `session/new`
//! with "authentication required", the driver tries only the *silent*
//! methods listed here (they reuse a login the CLI already has, or a key the
//! user put in the instance env) and otherwise emits `auth.status` with the
//! agent's login command for the user to run.

use std::collections::BTreeMap;

use crate::core::llm::drivers::{DriverInstance, EnvVar};

/// An auth method the driver may call on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentAuth {
    pub method: &'static str,
    /// Only when this env var is set (in the instance env or the app's).
    pub when_env: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct AgentSpec {
    /// Tool id of the briefing table (`gemini`, `cursor`, …).
    pub id: &'static str,
    pub name: &'static str,
    /// Id in the ACP registry, when listed there.
    pub registry_id: Option<&'static str>,
    /// Binaries to look for, in order.
    pub binaries: &'static [&'static str],
    /// Arguments that start the ACP server on stdio.
    pub args: &'static [&'static str],
    pub env: &'static [(&'static str, &'static str)],
    pub silent_auth: &'static [SilentAuth],
    /// Command that signs the user in (binary + args), when there is one.
    pub login: &'static [&'static str],
    pub login_hint: Option<&'static str>,
}

const fn a(
    id: &'static str,
    name: &'static str,
    registry_id: Option<&'static str>,
    binaries: &'static [&'static str],
    args: &'static [&'static str],
) -> AgentSpec {
    AgentSpec {
        id,
        name,
        registry_id,
        binaries,
        args,
        env: &[],
        silent_auth: &[],
        login: &[],
        login_hint: None,
    }
}

/// Every ACP agent with a documented stdio entry point (study 75 §k, ACP
/// registry 2026-09-22, T3 Code's Cursor/Grok adapters).
pub const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        silent_auth: &[
            SilentAuth { method: "gemini-api-key", when_env: Some("GEMINI_API_KEY") },
            SilentAuth { method: "vertex-ai", when_env: Some("GOOGLE_API_KEY") },
        ],
        login: &["gemini"],
        login_hint: Some("Run `gemini` once and choose \"Login with Google\", or set GEMINI_API_KEY on the instance."),
        ..a("gemini", "Gemini CLI", Some("gemini"), &["gemini"], &["--acp"])
    },
    AgentSpec {
        silent_auth: &[SilentAuth { method: "openai", when_env: Some("OPENAI_API_KEY") }],
        login: &["qwen"],
        login_hint: Some("Run `qwen` and use /auth."),
        ..a("qwen", "Qwen Code", Some("qwen-code"), &["qwen"], &["--acp"])
    },
    AgentSpec {
        silent_auth: &[SilentAuth { method: "cursor_login", when_env: None }],
        login: &["cursor-agent", "login"],
        ..a("cursor", "Cursor Agent", Some("cursor"), &["cursor-agent", "agent"], &["acp"])
    },
    AgentSpec {
        silent_auth: &[
            SilentAuth { method: "xai.api_key", when_env: Some("XAI_API_KEY") },
            SilentAuth { method: "cached_token", when_env: None },
        ],
        login: &["grok", "login"],
        ..a("grok", "Grok Build", Some("grok-build"), &["grok"], &["agent", "stdio"])
    },
    AgentSpec {
        login: &["copilot", "login"],
        ..a("copilot", "GitHub Copilot CLI", Some("github-copilot-cli"), &["copilot"], &["--acp"])
    },
    AgentSpec {
        login: &["cline", "auth"],
        ..a("cline", "Cline", Some("cline"), &["cline"], &["--acp"])
    },
    AgentSpec {
        login: &["goose", "configure"],
        ..a("goose", "Goose", Some("goose"), &["goose"], &["acp"])
    },
    AgentSpec {
        login: &["opencode", "auth", "login"],
        ..a("opencode", "OpenCode", Some("opencode"), &["opencode"], &["acp"])
    },
    AgentSpec {
        login: &["kilo", "auth", "login"],
        ..a("kilo", "Kilo Code", Some("kilo"), &["kilo", "kilocode"], &["acp"])
    },
    AgentSpec {
        login: &["kimi", "login"],
        ..a("kimi", "Kimi Code", Some("kimi"), &["kimi"], &["acp"])
    },
    AgentSpec {
        login_hint: Some("Run `droid` and use /login."),
        ..a("droid", "Factory Droid", Some("factory-droid"), &["droid"], &["exec", "--output-format", "acp"])
    },
    AgentSpec {
        login_hint: Some("Run `junie` and follow the JetBrains sign-in."),
        ..a("junie", "Junie", Some("junie"), &["junie"], &["--acp=true"])
    },
    AgentSpec {
        env: &[("AUGMENT_DISABLE_AUTO_UPDATE", "1")],
        login: &["auggie", "login"],
        ..a("auggie", "Auggie", Some("auggie"), &["auggie"], &["--acp"])
    },
    AgentSpec {
        login: &["vibe", "--setup"],
        ..a("vibe", "Mistral Vibe", Some("mistral-vibe"), &["vibe-acp"], &[])
    },
    AgentSpec {
        login: &["kiro-cli", "login"],
        ..a("kiro", "Kiro CLI", None, &["kiro-cli"], &["acp"])
    },
    AgentSpec {
        login: &["qoder", "login"],
        ..a("qoder", "Qoder CLI", Some("qoder"), &["qoder", "qodercli"], &["--acp"])
    },
    AgentSpec {
        login_hint: Some("Run `devin` and use /login."),
        ..a("devin", "Devin CLI", Some("devin"), &["devin"], &["acp"])
    },
    AgentSpec {
        login: &["openhands", "login"],
        ..a("openhands", "OpenHands", None, &["openhands"], &["acp"])
    },
    AgentSpec {
        login: &["claude", "auth", "login"],
        ..a("claude", "Claude Code (ACP adapter)", Some("claude-acp"), &["claude-agent-acp", "claude-code-acp"], &[])
    },
    AgentSpec {
        login: &["codex", "login"],
        ..a("codex", "Codex (ACP adapter)", Some("codex-acp"), &["codex-acp"], &[])
    },
    AgentSpec {
        login_hint: Some("Run `pi` and use /login."),
        ..a("pi", "Pi (ACP adapter)", Some("pi-acp"), &["pi-acp"], &[])
    },
    AgentSpec {
        login: &["amp", "login"],
        ..a("amp", "Amp (ACP adapter)", Some("amp-acp"), &["amp-acp"], &[])
    },
    a("letta", "Letta Code (ACP adapter)", None, &["letta-acp"], &[]),
    AgentSpec {
        login_hint: Some("Antigravity signs in with Google on first use."),
        ..a("antigravity", "Google Antigravity", Some("antigravity-acp"), &[], &[])
    },
];

pub fn spec(id: &str) -> Option<&'static AgentSpec> {
    AGENTS.iter().find(|s| s.id == id)
}

pub fn spec_by_registry(registry_id: &str) -> Option<&'static AgentSpec> {
    AGENTS.iter().find(|s| s.registry_id == Some(registry_id))
}

/// How to start one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    /// Agent id (table id, or the registry id for agents not in the table).
    pub agent: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Where it came from: `instance`, `registry:<npx|uvx|binary>`, `path`.
    pub source: String,
}

fn registry_launch(registry_id: &str) -> Option<crate::core::clitools::acp_registry::AcpLaunch> {
    crate::core::clitools::acp_registry::installed().remove(registry_id)
}

/// Resolve the launch of a table agent (registry install, then PATH).
pub fn resolve(spec: &AgentSpec) -> Option<Launch> {
    if let Some(reg) = spec.registry_id.and_then(registry_launch) {
        let mut env: Vec<(String, String)> = reg.env.into_iter().collect();
        for (k, v) in spec.env {
            if !env.iter().any(|(ek, _)| ek == k) {
                env.push((k.to_string(), v.to_string()));
            }
        }
        return Some(Launch {
            agent: spec.id.to_string(),
            name: spec.name.to_string(),
            command: reg.command,
            args: reg.args,
            env,
            source: format!("registry:{}", reg.distribution),
        });
    }
    let bin = spec.binaries.iter().find_map(|b| super::proc::which(b))?;
    Some(Launch {
        agent: spec.id.to_string(),
        name: spec.name.to_string(),
        command: bin.to_string_lossy().to_string(),
        args: spec.args.iter().map(|s| s.to_string()).collect(),
        env: spec
            .env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        source: "path".into(),
    })
}

/// Instance id of an agent.
pub fn instance_id(agent: &str) -> String {
    let slug: String = agent
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("acp-{slug}")
}

/// The agent id an instance points at (`acp-gemini` → `gemini`).
pub fn agent_of(instance: &DriverInstance) -> Option<String> {
    instance
        .id
        .strip_prefix("acp-")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn instance_of(launch: &Launch) -> DriverInstance {
    let mut inst = DriverInstance::new(&instance_id(&launch.agent), super::KIND, &launch.name);
    inst.command = Some(launch.command.clone());
    inst.args = launch.args.clone();
    inst.env = launch
        .env
        .iter()
        .map(|(k, v)| EnvVar {
            name: k.clone(),
            value: v.clone(),
            sensitive: false,
            value_redacted: false,
        })
        .collect();
    inst
}

/// Every ACP agent that can start on this machine right now, as an instance
/// of the `acp` driver. Cheap: PATH lookups and one small JSON read.
pub fn instances() -> Vec<DriverInstance> {
    let mut out: Vec<DriverInstance> = AGENTS
        .iter()
        .filter_map(resolve)
        .map(|l| instance_of(&l))
        .collect();
    // Registry installs of agents the table does not know.
    let installed: BTreeMap<_, _> = crate::core::clitools::acp_registry::installed();
    for (rid, reg) in installed {
        if spec_by_registry(&rid).is_some() {
            continue;
        }
        let launch = Launch {
            agent: rid.clone(),
            name: reg.name.clone(),
            command: reg.command.clone(),
            args: reg.args.clone(),
            env: reg.env.clone().into_iter().collect(),
            source: format!("registry:{}", reg.distribution),
        };
        out.push(instance_of(&launch));
    }
    out
}

/// The launch for an instance of the `acp` driver.
pub fn launch_for(instance: &DriverInstance) -> Option<Launch> {
    let env_of = |i: &DriverInstance| -> Vec<(String, String)> {
        i.env
            .iter()
            .map(|e| (e.name.clone(), e.value.clone()))
            .collect()
    };
    if let Some(cmd) = instance.command.clone().filter(|c| !c.trim().is_empty()) {
        return Some(Launch {
            agent: agent_of(instance).unwrap_or_else(|| instance.id.clone()),
            name: instance.label.clone(),
            command: cmd,
            args: instance.args.clone(),
            env: env_of(instance),
            source: "instance".into(),
        });
    }
    let agent = agent_of(instance)?;
    let mut launch = match spec(&agent) {
        Some(s) => resolve(s)?,
        None => {
            let reg = registry_launch(&agent)?;
            Launch {
                agent: agent.clone(),
                name: reg.name,
                command: reg.command,
                args: reg.args,
                env: reg.env.into_iter().collect(),
                source: format!("registry:{}", reg.distribution),
            }
        }
    };
    for (k, v) in env_of(instance) {
        launch.env.retain(|(ek, _)| ek != &k);
        launch.env.push((k, v));
    }
    Some(launch)
}

/// Silent auth methods to try for an agent, given the agent's advertised
/// methods and the env the child will see.
pub fn silent_methods(agent: &str, advertised: &[String], env: &[(String, String)]) -> Vec<String> {
    let Some(s) = spec(agent) else {
        return Vec::new();
    };
    let has_env = |k: &str| {
        env.iter().any(|(ek, v)| ek == k && !v.is_empty())
            || std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false)
    };
    s.silent_auth
        .iter()
        .filter(|m| advertised.iter().any(|a| a == m.method))
        .filter(|m| m.when_env.map(has_env).unwrap_or(true))
        .map(|m| m.method.to_string())
        .collect()
}

/// The login command line to show when the agent needs a sign-in.
pub fn login_command(agent: &str) -> Option<Vec<String>> {
    let s = spec(agent)?;
    if !s.login.is_empty() {
        return Some(s.login.iter().map(|x| x.to_string()).collect());
    }
    // Fall back to the clitools table (same ids).
    let tool = crate::core::clitools::table::get(agent)?;
    let bin = tool.binaries.first()?;
    if tool.login.is_empty() {
        return None;
    }
    let mut v = vec![bin.clone()];
    v.extend(tool.login.iter().cloned());
    Some(v)
}

pub fn login_hint(agent: &str) -> Option<String> {
    spec(agent)
        .and_then(|s| s.login_hint.map(str::to_string))
        .or_else(|| crate::core::clitools::table::get(agent).and_then(|t| t.login_hint.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_ids_are_unique_and_match_the_briefing_ids() {
        let known = [
            "claude",
            "codex",
            "gemini",
            "qwen",
            "opencode",
            "kilo",
            "cursor",
            "copilot",
            "cline",
            "goose",
            "droid",
            "kimi",
            "pi",
            "amp",
            "junie",
            "auggie",
            "grok",
            "kiro",
            "devin",
            "qoder",
            "vibe",
            "letta",
            "antigravity",
            "openhands",
        ];
        let mut seen = std::collections::HashSet::new();
        for s in AGENTS {
            assert!(seen.insert(s.id), "duplicate {}", s.id);
            assert!(known.contains(&s.id), "unknown id {}", s.id);
        }
    }

    #[test]
    fn silent_auth_needs_the_env_and_the_advertised_method() {
        let adv = vec!["oauth-personal".to_string(), "gemini-api-key".to_string()];
        assert!(silent_methods("gemini", &adv, &[])
            .iter()
            .all(|m| m != "oauth-personal"));
        let with_key = silent_methods("gemini", &adv, &[("GEMINI_API_KEY".into(), "x".into())]);
        assert_eq!(with_key, vec!["gemini-api-key".to_string()]);
        let cur = silent_methods("cursor", &["cursor_login".to_string()], &[]);
        assert_eq!(cur, vec!["cursor_login".to_string()]);
    }

    #[test]
    fn instance_ids_round_trip() {
        let mut inst = DriverInstance::new(&instance_id("qwen-code"), "acp", "Qwen");
        assert_eq!(inst.id, "acp-qwen-code");
        assert_eq!(agent_of(&inst).as_deref(), Some("qwen-code"));
        inst.command = Some("/bin/echo".into());
        inst.args = vec!["x".into()];
        let l = launch_for(&inst).unwrap();
        assert_eq!(
            (l.command.as_str(), l.source.as_str()),
            ("/bin/echo", "instance")
        );
    }
}
