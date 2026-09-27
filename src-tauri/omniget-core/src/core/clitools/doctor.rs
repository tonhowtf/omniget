//! Diagnóstico por ferramenta: instalada, versão, dono, login presente,
//! onde ficam config e logs, e formatos deprecados encontrados.
//!
//! Login: roda o comando de status da própria ferramenta (quando existe) ou
//! confere só a **presença** de um arquivo. Nenhum conteúdo é lido.
//!
//! Config/logs: lista mínima própria em `data/tools.json`. Quando o agentkit
//! expuser `targets::target(id)`, a rodada 2 pode trocar esta fonte.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use super::detect::{self, Detection};
use super::exec::{self, run_capture};
use super::latest::{self, Latest};
use super::owner::OwnerMethod;
use super::table::{expand_home, CliTool};

pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Present,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthProbe {
    pub state: AuthState,
    /// `command`, `file` ou `none`.
    pub via: String,
    /// Comando rodado ou arquivos conferidos.
    pub detail: String,
    /// Primeira linha da saída do comando de status (da própria ferramenta).
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathCheck {
    pub path: String,
    pub resolved: Option<PathBuf>,
    pub exists: Option<bool>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub id: String,
    pub name: String,
    pub detection: Detection,
    pub latest: Latest,
    pub auth: AuthProbe,
    pub config_env: Option<(String, Option<String>)>,
    pub config: Vec<PathCheck>,
    pub logs: Vec<PathCheck>,
    pub deprecated: Vec<PathCheck>,
    pub issues: Vec<String>,
}

fn check_path(p: &str, note: Option<String>) -> PathCheck {
    if let Some(bin) = p.strip_prefix("bin:") {
        let found = exec::which(bin);
        return PathCheck {
            path: p.into(),
            exists: Some(found.is_some()),
            resolved: found,
            note,
        };
    }
    if p.starts_with("npm:") || !(p.starts_with("~") || p.starts_with('/')) {
        // Pacote ou arquivo de projeto: informativo, sem projeto aberto.
        return PathCheck {
            path: p.into(),
            resolved: None,
            exists: None,
            note,
        };
    }
    let r = expand_home(p);
    PathCheck {
        path: p.into(),
        exists: Some(r.exists()),
        resolved: Some(r),
        note,
    }
}

async fn probe_auth(tool: &CliTool, det: &Detection) -> AuthProbe {
    if let (Some(args), Some(bin)) = (&tool.auth_status, &det.resolved_path) {
        let mut env = BTreeMap::new();
        env.insert("NO_COLOR".to_string(), "1".to_string());
        env.insert("CI".to_string(), "1".to_string());
        let shown = exec::display_command(&bin.to_string_lossy(), args);
        return match run_capture(bin, args, &env, AUTH_TIMEOUT).await {
            Some(out) => {
                let line: String = out.first_line().chars().take(200).collect();
                let state = if tool.auth_status_exit_means_auth {
                    if out.ok() {
                        AuthState::Present
                    } else {
                        AuthState::Absent
                    }
                } else {
                    AuthState::Unknown
                };
                AuthProbe {
                    state,
                    via: "command".into(),
                    detail: shown,
                    output: Some(line),
                }
            }
            None => AuthProbe {
                state: AuthState::Unknown,
                via: "command".into(),
                detail: format!("{shown} (sem resposta)"),
                output: None,
            },
        };
    }
    if !tool.auth_files.is_empty() {
        let any = tool.auth_files.iter().any(|f| expand_home(f).exists());
        return AuthProbe {
            state: if any {
                AuthState::Present
            } else {
                AuthState::Unknown
            },
            via: "file".into(),
            detail: tool.auth_files.join(", "),
            output: None,
        };
    }
    AuthProbe {
        state: AuthState::Unknown,
        via: "none".into(),
        detail: tool.login_hint.clone().unwrap_or_default(),
        output: None,
    }
}

pub async fn doctor(tool: &CliTool) -> DoctorReport {
    let det = detect::detect_one(tool).await;
    let brew_owner = (det.owner.method == OwnerMethod::Brew && det.owner.proven)
        .then(|| tool.brew.as_ref().map(|b| (b.name.clone(), b.cask)))
        .flatten();
    let latest = latest::for_tool(tool, det.version.clone(), brew_owner, false).await;
    let auth = if det.installed {
        probe_auth(tool, &det).await
    } else {
        AuthProbe {
            state: AuthState::Unknown,
            via: "none".into(),
            detail: String::new(),
            output: None,
        }
    };
    let config: Vec<PathCheck> = tool
        .config_paths
        .iter()
        .map(|p| check_path(p, None))
        .collect();
    let logs: Vec<PathCheck> = tool.log_paths.iter().map(|p| check_path(p, None)).collect();
    let deprecated: Vec<PathCheck> = tool
        .deprecated
        .iter()
        .map(|d| check_path(&d.path, Some(d.note.clone())))
        .collect();

    let mut issues = Vec::new();
    if det.installed && det.version.is_none() && det.binary.is_some() {
        issues.push(format!(
            "`{} {}` não devolveu versão ({})",
            det.binary.clone().unwrap_or_default(),
            tool.version_args.join(" "),
            det.version_raw.clone().unwrap_or_default()
        ));
    }
    if det.installed
        && det.binary.is_some()
        && !det.owner.proven
        && det.owner.method != OwnerMethod::Omniget
    {
        issues.push(format!("Dono não provado: {}", det.owner.evidence));
    }
    if latest.state == latest::VersionState::BehindLatest {
        issues.push(format!(
            "Versão {} atrás da {}",
            latest.installed.clone().unwrap_or_default(),
            latest.latest.clone().unwrap_or_default()
        ));
    }
    for d in &deprecated {
        if d.exists == Some(true) {
            issues.push(format!("Formato deprecado presente: {}", d.path));
        }
    }
    if tool.status != super::table::ToolStatus::Active {
        issues.push(
            tool.status_note
                .clone()
                .unwrap_or_else(|| format!("Status: {:?}", tool.status)),
        );
    }
    let config_env = tool
        .config_env
        .as_ref()
        .map(|k| (k.clone(), std::env::var(k).ok()));

    DoctorReport {
        id: tool.id.clone(),
        name: tool.name.clone(),
        detection: det,
        latest,
        auth,
        config_env,
        config,
        logs,
        deprecated,
        issues,
    }
}
