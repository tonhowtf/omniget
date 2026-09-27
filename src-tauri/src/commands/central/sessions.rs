//! Comandos `sessions` da Central: analytics das sessões de todos os agentes
//! de código (Claude, Codex, Gemini, Qwen, OpenCode, Kilo, Crush, Goose e o
//! grupo B). A lógica vive em `omniget_core::core::sessions`; aqui só a
//! ponte. Erros no formato `"CODE: mensagem"`.

use omniget_core::core::sessions::analysis::{
    AgentsReport, Heatmap, Retro, SessionAnalysis, TeamGraph, UsageReport,
};
use omniget_core::core::sessions::api::{self, ActiveSession, SessionPage, SourceInfo};
use omniget_core::core::sessions::export::ExportOut;
use omniget_core::core::sessions::import::ImportOutcome;
use omniget_core::core::sessions::index::{ListFilter, ListPage, RefreshStats};
use omniget_core::core::sessions::search::{Hit, SearchQuery, SearchResult};

/// Lista de sessões do índice (filtro por ferramenta/conta/projeto/data/título).
#[tauri::command]
pub async fn sessions_list(filter: Option<ListFilter>) -> Result<ListPage, String> {
    api::list(filter.unwrap_or_default()).await
}

/// Sessão completa; com `page`, paginação reversa (0 = mais recentes).
#[tauri::command]
pub async fn sessions_get(
    tool: String,
    id: String,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<SessionPage, String> {
    api::get(tool, id, page, page_size).await
}

#[tauri::command]
pub async fn sessions_search(query: SearchQuery) -> Result<SearchResult, String> {
    api::search(query).await
}

#[tauri::command]
pub async fn sessions_search_in(
    tool: String,
    id: String,
    query: String,
) -> Result<Vec<Hit>, String> {
    api::search_in(tool, id, query).await
}

#[tauri::command]
pub async fn sessions_analysis(tool: String, id: String) -> Result<SessionAnalysis, String> {
    api::session_analysis(tool, id).await
}

/// `group_by`: `day` | `week` | `month` | `model` | `tool` | `project` |
/// `account` | `hour` | `session`.
#[tauri::command]
pub async fn sessions_usage(
    range: Option<ListFilter>,
    group_by: Option<String>,
) -> Result<UsageReport, String> {
    api::usage(
        range.unwrap_or_default(),
        group_by.unwrap_or_else(|| "day".into()),
    )
    .await
}

#[tauri::command]
pub async fn sessions_heatmap(
    range: Option<ListFilter>,
    tool: Option<String>,
) -> Result<Heatmap, String> {
    api::heatmap(range.unwrap_or_default(), tool).await
}

#[tauri::command]
pub async fn sessions_agents(range: Option<ListFilter>) -> Result<AgentsReport, String> {
    api::agents(range.unwrap_or_default()).await
}

#[tauri::command]
pub async fn sessions_team(tool: String, id: String) -> Result<TeamGraph, String> {
    api::team(tool, id).await
}

#[tauri::command]
pub async fn sessions_retro(
    from: Option<String>,
    to: Option<String>,
    tool: Option<String>,
) -> Result<Retro, String> {
    api::retro(from, to, tool).await
}

/// `format`: `json` | `markdown` | `context` (com `target` = ferramenta onde
/// o contexto vai ser colado).
#[tauri::command]
pub async fn sessions_export(
    tool: String,
    id: String,
    format: String,
    target: Option<String>,
) -> Result<ExportOut, String> {
    api::export(tool, id, format, target).await
}

/// Importa um JSON exportado: `claude`/`codex` gravam uma sessão retomável;
/// outra ferramenta devolve Markdown de contexto.
#[tauri::command]
pub async fn sessions_import(
    path: String,
    target: Option<String>,
) -> Result<ImportOutcome, String> {
    api::import(path, target).await
}

#[tauri::command]
pub async fn sessions_resume_command(tool: String, id: String) -> Result<Option<String>, String> {
    api::resume_command(tool, id).await
}

#[tauri::command]
pub async fn sessions_sources() -> Result<Vec<SourceInfo>, String> {
    api::sources_info().await
}

#[tauri::command]
pub async fn sessions_active(window_secs: Option<u64>) -> Result<Vec<ActiveSession>, String> {
    api::active(window_secs.unwrap_or(600)).await
}

/// Varre agora; `full` apaga o índice e relê tudo.
#[tauri::command]
pub async fn sessions_refresh(full: Option<bool>) -> Result<RefreshStats, String> {
    api::refresh(full.unwrap_or(false)).await
}
