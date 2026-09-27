//! Comandos `clitools_*` da Central (`/llm/tools`): tabela de CLIs, detecção
//! com prova de dono, versão mais nova, planos de instalar/atualizar com o
//! comando visível, execução com log ao vivo (`clitools://progress`), registro
//! ACP, login no terminal e diagnóstico.
//!
//! A lógica mora em `omniget_core::core::clitools`; aqui só a costura com o
//! Tauri (eventos e contas).

use std::collections::BTreeMap;
use std::sync::Arc;

use omniget_core::core::clitools::{
    self as ct, acp_registry, detect, doctor, plan, run, table, terminal,
};
use omniget_core::core::llm::cli_runtime::accounts::{
    account_env, create_profile_dir, AccountStore, SCRUB_ENV,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

pub const PROGRESS_EVENT: &str = "clitools://progress";

fn progress_fn(app: AppHandle) -> run::ProgressFn {
    Arc::new(move |p: run::Progress| {
        let _ = app.emit(PROGRESS_EVENT, &p);
    })
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("{}: {e}", ct::ERR_FAILED))
}

/// Tabela + última detecção (não roda nada).
#[tauri::command]
pub async fn clitools_list() -> Result<Value, String> {
    to_value(&json!({
        "tools": ct::list(),
        "platform": acp_registry::platform_key(),
        "detected": detect::cached().is_some(),
    }))
}

/// Varre as ferramentas (cache em memória; `force` refaz).
#[tauri::command]
pub async fn clitools_detect(force: Option<bool>) -> Result<Value, String> {
    // Roda fora do executor: o future de `detect_all` (buffer_unordered com
    // closure) não passa na checagem de `Send` genérica do handler do Tauri.
    let force = force.unwrap_or(false);
    let list = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(detect::detect_all(force))
    })
    .await
    .map_err(|e| e.to_string())?;
    to_value(&*list)
}

/// Versão instalada × mais nova (`unknown|current|behind_latest`).
#[tauri::command]
pub async fn clitools_latest(id: String, force: Option<bool>) -> Result<Value, String> {
    let l = ct::latest(&id, force.unwrap_or(false)).await?;
    to_value(&l)
}

/// Planos de `install` (um por método) ou `update` (o do dono provado).
#[tauri::command]
pub async fn clitools_plan(id: String, action: String) -> Result<Value, String> {
    let action = plan::Action::parse(&action)
        .ok_or_else(|| format!("{}: ação {action}", ct::ERR_NOT_FOUND))?;
    let plans = ct::plans(&id, action).await?;
    to_value(&plans)
}

/// Roda um plano; o log sai em `clitools://progress` e o resultado volta aqui.
#[tauri::command]
pub async fn clitools_run(app: AppHandle, plan_id: String) -> Result<Value, String> {
    let result = run::run_plan(&plan_id, progress_fn(app)).await?;
    to_value(&result)
}

/// Login no terminal do sistema, na conta certa. `surface = "embedded"`
/// devolve só o lançamento (programa, argumentos, ambiente) para o terminal
/// embutido da rodada 2 abrir.
#[tauri::command]
pub async fn clitools_login(
    id: String,
    account: Option<String>,
    surface: Option<String>,
) -> Result<Value, String> {
    let tool = ct::tool(&id)?;
    let surface = terminal::Surface::parse(surface.as_deref());
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut unset: Vec<&str> = Vec::new();
    let mut tag = tool.id.clone();

    if let Some(account_id) = account.filter(|a| !a.trim().is_empty()) {
        let store = AccountStore::default_store()
            .ok_or_else(|| format!("{}: sem pasta de dados", ct::ERR_LOGIN))?;
        let acc = store
            .get(&account_id)
            .ok_or_else(|| format!("{}: conta {account_id} não existe", ct::ERR_LOGIN))?;
        if acc.cli.as_str() != tool.id {
            return Err(format!(
                "{}: a conta {account_id} é do {}, não do {}",
                ct::ERR_LOGIN,
                acc.cli.as_str(),
                tool.id
            ));
        }
        create_profile_dir(&acc.config_dir).map_err(|e| format!("{}: {e}", ct::ERR_LOGIN))?;
        env = account_env(&acc);
        unset = SCRUB_ENV.to_vec();
        tag = format!("{}-{}", tool.id, acc.id);
    }

    let bin = detect::cached_one(&id)
        .and_then(|d| d.resolved_path)
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| tool.binaries.first().cloned())
        .ok_or_else(|| {
            format!(
                "{}: {} não tem CLI; entre pelo app",
                ct::ERR_LOGIN,
                tool.name
            )
        })?;

    let script_dir = omniget_core::core::paths::app_data_dir()
        .ok_or_else(|| format!("{}: sem pasta de dados", ct::ERR_LOGIN))?
        .join("llm")
        .join("launchers");
    let have_wt = cfg!(target_os = "windows")
        && omniget_core::core::dependencies::find_tool("wt")
            .await
            .is_some();
    let launch =
        terminal::login_launch(tool, &bin, env, &unset, &tag, &script_dir, surface, have_wt);
    if surface == terminal::Surface::System {
        let l = launch.clone();
        tokio::task::spawn_blocking(move || terminal::open_system(&l))
            .await
            .map_err(|e| format!("{}: {e}", ct::ERR_LOGIN))?
            .map_err(|e| format!("{}: {e}", ct::ERR_LOGIN))?;
    }
    to_value(&launch)
}

/// Registro ACP com os agentes e o que já foi instalado.
#[tauri::command]
pub async fn clitools_acp_registry(force: Option<bool>) -> Result<Value, String> {
    let v = acp_registry::registry(force.unwrap_or(false)).await?;
    to_value(&v)
}

/// Instala um agente do registro e devolve o comando ACP pronto.
/// `method`: `npx|uvx|binary` (opcional). `allow_unverified`: aceitar binário
/// sem sha256 no registro (a tela pede confirmação antes).
#[tauri::command]
pub async fn clitools_acp_install(
    app: AppHandle,
    agent_id: String,
    method: Option<String>,
    allow_unverified: Option<bool>,
) -> Result<Value, String> {
    let launch = acp_registry::install(
        &agent_id,
        method.as_deref(),
        allow_unverified.unwrap_or(false),
        progress_fn(app),
    )
    .await?;
    to_value(&launch)
}

/// Diagnóstico de uma ferramenta.
#[tauri::command]
pub async fn clitools_doctor(id: String) -> Result<Value, String> {
    let t: &table::CliTool = ct::tool(&id)?;
    let report = doctor::doctor(t).await;
    to_value(&report)
}

// ── Saúde da config e monitor de releases (r3-health) ───────────────────

pub const UPDATES_EVENT: &str = "clitools://updates";

/// Arquivos de config reais de uma ferramenta por escopo (global e, com
/// `projectDir`, projeto/local): só caminho, existência e tamanho. Nomes de
/// credencial (auth.json, credentials*, oauth*, *token*) são pulados.
#[tauri::command]
pub async fn clitools_config_files(
    tool: String,
    project_dir: Option<String>,
) -> Result<Value, String> {
    let files = tauri::async_runtime::spawn_blocking(move || {
        let project = project_dir
            .filter(|p| !p.trim().is_empty())
            .map(std::path::PathBuf::from);
        omniget_core::core::clitools::config_files::config_files(&tool, project.as_deref())
    })
    .await
    .map_err(|e| format!("{}: {e}", ct::ERR_FAILED))??;
    to_value(&files)
}

/// Instalada × mais nova das ferramentas detectadas (cache de 1 h do
/// `latest`) com changelog curto dos releases do GitHub.
///
/// - `auto = true`: respeita o intervalo de 24 h; se ainda não venceu, devolve
///   o último relatório (`from_cache = true`) sem rede e sem evento.
/// - `force = true`: ignora os caches de versão.
///
/// Toda checagem feita de verdade emite `clitools://updates` com o relatório.
#[tauri::command]
pub async fn clitools_updates(
    app: AppHandle,
    auto: Option<bool>,
    force: Option<bool>,
) -> Result<Value, String> {
    use omniget_core::core::clitools::updates;
    let auto = auto.unwrap_or(false);
    let force = force.unwrap_or(false);
    if auto && !force {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if !updates::due(now) {
            if let Some(r) = updates::cached_report() {
                return to_value(&r);
            }
        }
    }
    // Mesmo motivo do `clitools_detect`: o future da detecção não é `Send`.
    let report = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(async move {
            let dets = detect::detect_all(false).await;
            updates::check(&dets, force).await
        })
    })
    .await
    .map_err(|e| format!("{}: {e}", ct::ERR_FAILED))?;
    let _ = app.emit(UPDATES_EVENT, &report);
    to_value(&report)
}

/// Último relatório de atualizações guardado (sem rede).
#[tauri::command]
pub async fn clitools_updates_cached() -> Result<Value, String> {
    to_value(&omniget_core::core::clitools::updates::cached_report())
}
