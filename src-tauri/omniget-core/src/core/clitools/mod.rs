//! Central de agentes: instalação e manutenção dos CLIs (`/llm/tools`).
//!
//! - `table`: um registro por ferramenta (`data/tools.json`).
//! - `detect` + `owner`: caminho resolvido/real, versão e **prova de dono**.
//! - `latest`: versão mais nova (npm/PyPI/GitHub/brew), cache de 1 h.
//! - `plan` + `run`: comando exato antes, execução com lock e saída ≤ 10 mil
//!   caracteres; nunca roda gerenciador contra instalação não provada.
//! - `acp_registry`: registro ACP (npx/uvx/binário com sha256).
//! - `terminal`: login no terminal do sistema (ou embutido, rodada 2).
//! - `doctor`: diagnóstico por ferramenta.
//! - `config_files`: arquivos de config reais por escopo (só caminho e tamanho).
//! - `updates`: monitor de releases (instalada × mais nova + changelog curto).
//!
//! Nada roda em repouso: tudo parte de um comando da tela.

pub mod acp_registry;
pub mod config_files;
pub mod detect;
pub mod doctor;
pub mod exec;
pub mod latest;
pub mod owner;
pub mod plan;
pub mod run;
pub mod table;
pub mod terminal;
pub mod updates;

use serde::Serialize;

pub const ERR_NOT_FOUND: &str = "CLITOOLS_NOT_FOUND";
pub const ERR_NOT_RUNNABLE: &str = "CLITOOLS_NOT_RUNNABLE";
pub const ERR_BUSY: &str = "CLITOOLS_BUSY";
pub const ERR_STALE: &str = "CLITOOLS_STALE";
pub const ERR_FAILED: &str = "CLITOOLS_FAILED";
pub const ERR_ACP: &str = "CLITOOLS_ACP";
pub const ERR_UNVERIFIED: &str = "CLITOOLS_UNVERIFIED";
pub const ERR_LOGIN: &str = "CLITOOLS_LOGIN";

/// Linha da grade: dados da tabela + última detecção (sem rodar nada).
#[derive(Debug, Clone, Serialize)]
pub struct ToolRow {
    pub tool: table::CliTool,
    pub detection: Option<detect::Detection>,
    pub last_run: Option<run::RunResult>,
}

pub fn list() -> Vec<ToolRow> {
    let cached = detect::cached();
    table::all()
        .iter()
        .map(|t| ToolRow {
            tool: t.clone(),
            detection: cached
                .as_ref()
                .and_then(|v| v.iter().find(|d| d.id == t.id).cloned()),
            last_run: run::last_run(&t.id),
        })
        .collect()
}

pub fn tool(id: &str) -> Result<&'static table::CliTool, String> {
    table::get(id).ok_or_else(|| format!("{ERR_NOT_FOUND}: ferramenta {id}"))
}

/// Planos de uma ação (instalar: um por método; atualizar: o do dono).
pub async fn plans(id: &str, action: plan::Action) -> Result<Vec<plan::Plan>, String> {
    let t = tool(id)?;
    match action {
        plan::Action::Install => {
            let probes = owner::Probes::default();
            Ok(plan::install_plans(t, &probes).await)
        }
        plan::Action::Update => {
            let det = detect::detect_one(t).await;
            Ok(vec![plan::update_plan(t, &det)])
        }
    }
}

/// Estado de versão (detecção em cache, ou fresca se não houver).
pub async fn latest(id: &str, force: bool) -> Result<latest::Latest, String> {
    let t = tool(id)?;
    let det = match detect::cached_one(id) {
        Some(d) if !force => d,
        _ => detect::detect_one(t).await,
    };
    let brew = (det.owner.method == owner::OwnerMethod::Brew && det.owner.proven)
        .then(|| t.brew.as_ref().map(|b| (b.name.clone(), b.cask)))
        .flatten();
    Ok(latest::for_tool(t, det.version.clone(), brew, force).await)
}
