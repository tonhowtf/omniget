//! Planos de instalar/atualizar: o comando exato, visível antes de rodar.
//!
//! Instalação nova oferece um plano por método disponível (instalador oficial,
//! npm, brew, winget/scoop, uv/pipx), com um recomendado. Atualização só roda
//! o comando do dono **provado**; sem prova o plano sai com `runnable=false`
//! e o comando fica para copiar.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::detect::Detection;
use super::exec;
use super::owner::{self, CommandSpec, OwnerMethod, Probes};
use super::table::CliTool;

pub const PLAN_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Update,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s.trim().to_ascii_lowercase().as_str() {
            "install" => Some(Action::Install),
            "update" | "upgrade" => Some(Action::Update),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub id: String,
    pub tool_id: String,
    pub action: Action,
    /// `installer`, `npm`, `brew`, `winget`, `scoop`, `uv`, `pipx`, `native`,
    /// `bun`, `pnpm`, `pip`, `cargo`.
    pub method: String,
    pub command: CommandSpec,
    pub runnable: bool,
    /// Por que não roda (gerenciador ausente, dono não provado…).
    pub reason: Option<String>,
    pub recommended: bool,
    pub warnings: Vec<String>,
    /// Dono esperado na hora de rodar (update re-prova com detecção fresca).
    pub owner: Option<OwnerMethod>,
    pub prefix: Option<PathBuf>,
}

fn registry() -> &'static Mutex<HashMap<String, (Instant, Plan)>> {
    use std::sync::OnceLock;
    static R: OnceLock<Mutex<HashMap<String, (Instant, Plan)>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember(plan: &Plan) {
    if let Ok(mut r) = registry().lock() {
        r.retain(|_, (at, _)| at.elapsed() < PLAN_TTL);
        r.insert(plan.id.clone(), (Instant::now(), plan.clone()));
    }
}

pub fn lookup(plan_id: &str) -> Option<Plan> {
    let r = registry().lock().ok()?;
    r.get(plan_id)
        .filter(|(at, _)| at.elapsed() < PLAN_TTL)
        .map(|(_, p)| p.clone())
}

fn installer_line(tool: &CliTool) -> Option<String> {
    if cfg!(windows) {
        tool.installer
            .windows
            .as_ref()
            .map(|u| format!("irm '{u}' | iex"))
    } else {
        tool.installer
            .unix
            .as_ref()
            .map(|u| format!("curl -fsSL '{}' | {}", u, tool.installer.unix_shell))
    }
}

/// Instalador oficial como comando de shell.
pub fn installer_spec(tool: &CliTool) -> Option<CommandSpec> {
    installer_line(tool).map(|l| CommandSpec::shell(l, format!("installer:{}", tool.id)))
}

/// Linha para copiar quando não há o que rodar (instalação).
pub fn manual_install_line(tool: &CliTool) -> Option<String> {
    installer_line(tool)
        .or_else(|| {
            tool.npm
                .as_ref()
                .map(|p| format!("npm install -g {p}@latest"))
        })
        .or_else(|| {
            tool.brew.as_ref().map(|b| {
                if b.cask {
                    format!("brew install --cask {}", b.name)
                } else {
                    format!("brew install {}", b.name)
                }
            })
        })
        .or_else(|| tool.pip.as_ref().map(|p| format!("uv tool install {p}")))
}

/// Linha para copiar na atualização quando o dono não foi provado.
pub fn manual_update_line(tool: &CliTool) -> Option<String> {
    tool.npm
        .as_ref()
        .map(|p| format!("npm install -g {p}@latest"))
        .or_else(|| installer_line(tool))
        .or_else(|| tool.pip.as_ref().map(|p| format!("uv tool upgrade {p}")))
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn plan(tool: &CliTool, action: Action, method: &str, command: CommandSpec) -> Plan {
    Plan {
        id: new_id(),
        tool_id: tool.id.clone(),
        action,
        method: method.into(),
        command,
        runnable: true,
        reason: None,
        recommended: false,
        warnings: Vec::new(),
        owner: None,
        prefix: None,
    }
}

fn blocked(mut p: Plan, reason: impl Into<String>) -> Plan {
    p.runnable = false;
    p.reason = Some(reason.into());
    p
}

/// Prefixo npm pedido (testes e instalação isolada): `OMNIGET_CLITOOLS_NPM_PREFIX`.
pub fn npm_prefix_override() -> Option<PathBuf> {
    std::env::var_os("OMNIGET_CLITOOLS_NPM_PREFIX")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Planos de instalação nova, um por método disponível neste sistema.
pub async fn install_plans(tool: &CliTool, probes: &Probes) -> Vec<Plan> {
    let mut out: Vec<Plan> = Vec::new();

    if let Some(spec) = installer_spec(tool) {
        let mut p = plan(tool, Action::Install, "installer", spec);
        if !cfg!(windows) && exec::which("curl").is_none() {
            p = blocked(p, "curl não encontrado");
        }
        p.warnings
            .push("Roda o script oficial do fornecedor (baixado na hora).".into());
        out.push(p);
    }

    if let Some(pkg) = &tool.npm {
        match probes.npm().await {
            Some(npm) => {
                let version = probes.npm_version().await;
                let allow = owner::needs_allow_scripts(version.as_deref(), tool);
                let prefix = npm_prefix_override();
                let spec = owner::npm_install_spec(&npm, prefix.as_deref(), pkg, allow);
                let mut p = plan(tool, Action::Install, "npm", spec);
                p.prefix = prefix;
                if tool.npm_scripts && !allow {
                    p.warnings.push(
                        "O pacote precisa do postinstall; npm < 12 roda scripts por padrão.".into(),
                    );
                }
                out.push(p);
            }
            None => out.push(blocked(
                plan(
                    tool,
                    Action::Install,
                    "npm",
                    CommandSpec::new(
                        "npm",
                        vec!["install".into(), "-g".into(), format!("{pkg}@latest")],
                        "npm-global",
                    ),
                ),
                "npm não encontrado (instale o Node.js)",
            )),
        }
    }

    if let Some(b) = tool.brew.as_ref().filter(|_| !cfg!(windows)) {
        let mut args = vec!["install".to_string()];
        if b.cask {
            args.push("--cask".into());
        }
        args.push(b.name.clone());
        match probes.brew().await {
            Some(brew) => {
                let mut spec =
                    CommandSpec::new(brew.to_string_lossy().into_owned(), args, "homebrew");
                spec.env.insert("HOMEBREW_NO_ENV_HINTS".into(), "1".into());
                out.push(plan(tool, Action::Install, "brew", spec));
            }
            None => out.push(blocked(
                plan(
                    tool,
                    Action::Install,
                    "brew",
                    CommandSpec::new("brew", args, "homebrew"),
                ),
                "Homebrew não encontrado",
            )),
        }
    }

    if cfg!(windows) {
        if let Some(id) = &tool.winget {
            let args = vec![
                "install".to_string(),
                "--id".into(),
                id.clone(),
                "-e".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
            ];
            let p = plan(
                tool,
                Action::Install,
                "winget",
                CommandSpec::new("winget", args, "winget"),
            );
            out.push(if exec::which("winget").is_some() {
                p
            } else {
                blocked(p, "winget não encontrado")
            });
        }
        if let Some(id) = &tool.scoop {
            let p = plan(
                tool,
                Action::Install,
                "scoop",
                CommandSpec::new("scoop", vec!["install".into(), id.clone()], "scoop"),
            );
            out.push(if exec::which("scoop").is_some() {
                p
            } else {
                blocked(p, "scoop não encontrado")
            });
        }
    }

    if let Some(pkg) = &tool.pip {
        match (exec::which("uv"), exec::which("pipx")) {
            (Some(uv), _) => out.push(plan(
                tool,
                Action::Install,
                "uv",
                CommandSpec::new(
                    uv.to_string_lossy().into_owned(),
                    vec!["tool".into(), "install".into(), pkg.clone()],
                    "uv-tool",
                ),
            )),
            (None, Some(pipx)) => out.push(plan(
                tool,
                Action::Install,
                "pipx",
                CommandSpec::new(
                    pipx.to_string_lossy().into_owned(),
                    vec!["install".into(), pkg.clone()],
                    "pipx",
                ),
            )),
            (None, None) => out.push(blocked(
                plan(
                    tool,
                    Action::Install,
                    "uv",
                    CommandSpec::new(
                        "uv",
                        vec!["tool".into(), "install".into(), pkg.clone()],
                        "uv-tool",
                    ),
                ),
                "uv/pipx não encontrados",
            )),
        }
    }

    // Recomendado: o instalador do fornecedor quando ele tem auto-updater
    // (a instalação fica "nativa"); senão o primeiro gerenciador que roda.
    let prefers_vendor = tool.self_update.is_some() || tool.npm.is_none();
    let pick = out
        .iter()
        .position(|p| p.runnable && p.method == "installer" && prefers_vendor)
        .or_else(|| {
            out.iter()
                .position(|p| p.runnable && p.method != "installer")
        })
        .or_else(|| out.iter().position(|p| p.runnable));
    if let Some(i) = pick {
        out[i].recommended = true;
    }
    for p in &out {
        remember(p);
    }
    out
}

/// Plano de atualização a partir da detecção (dono provado ou nada).
pub fn update_plan(tool: &CliTool, det: &Detection) -> Plan {
    let own = &det.owner;
    let method = match own.method {
        OwnerMethod::Native => "native",
        OwnerMethod::Bun => "bun",
        OwnerMethod::Pnpm => "pnpm",
        OwnerMethod::Npm => "npm",
        OwnerMethod::Brew => "brew",
        OwnerMethod::Pipx => "pipx",
        OwnerMethod::Uv => "uv",
        OwnerMethod::Pip => "pip",
        OwnerMethod::Cargo => "cargo",
        OwnerMethod::Omniget => "omniget",
        OwnerMethod::App => "app",
        OwnerMethod::Unknown => "unknown",
    };
    let mut p = match (&own.update, own.proven) {
        (Some(cmd), true) => plan(tool, Action::Update, method, cmd.clone()),
        _ => {
            let line = own
                .manual
                .clone()
                .or_else(|| manual_update_line(tool))
                .unwrap_or_default();
            let reason = if !det.installed {
                "não instalado".to_string()
            } else if own.method == OwnerMethod::App {
                "app de GUI: atualize pelo próprio app".to_string()
            } else if own.method == OwnerMethod::Omniget {
                "instalado pelo registro ACP: reinstale pela seção Agentes ACP".to_string()
            } else {
                format!(
                    "dono não provado ({}): só o comando para copiar",
                    own.evidence
                )
            };
            let mut spec = CommandSpec::shell(line, format!("manual:{}", tool.id));
            spec.via_shell = false;
            blocked(plan(tool, Action::Update, method, spec), reason)
        }
    };
    p.owner = Some(own.method);
    p.prefix = own.prefix.clone();
    p.recommended = p.runnable;
    if tool.auto_updates && p.runnable {
        p.warnings
            .push("A ferramenta também se atualiza sozinha.".into());
    }
    remember(&p);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::clitools::table;

    #[test]
    fn linha_manual_prefere_o_instalador_oficial() {
        let t = table::get("claude").unwrap();
        let line = manual_install_line(t).unwrap();
        if cfg!(windows) {
            assert!(line.contains("install.ps1"));
        } else {
            assert_eq!(line, "curl -fsSL 'https://claude.ai/install.sh' | bash");
        }
        let codex = table::get("codex").unwrap();
        assert_eq!(
            manual_install_line(codex).unwrap(),
            "npm install -g @openai/codex@latest"
        );
    }

    #[test]
    fn update_sem_dono_nao_roda() {
        let t = table::get("codex").unwrap();
        let det = Detection {
            id: "codex".into(),
            installed: true,
            binary: Some("codex".into()),
            resolved_path: Some("/usr/bin/codex".into()),
            real_path: Some("/usr/bin/codex".into()),
            version: Some("0.1.0".into()),
            version_raw: None,
            app_path: None,
            app_version: None,
            owner: owner::Owner::unknown("x", None),
            checked_at: 0,
        };
        let p = update_plan(t, &det);
        assert!(!p.runnable);
        assert_eq!(p.command.display, "npm install -g @openai/codex@latest");
        assert!(lookup(&p.id).is_some());
    }
}
