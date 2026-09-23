//! Comandos `guard` da Central: varredura de segurança/saúde de componentes
//! (agentes, comandos, skills, hooks, MCP, settings, statuslines) e as
//! estatísticas da "Saúde da config". A lógica mora em
//! `omniget_core::core::guard`; aqui só há a ponte com o front.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use omniget_core::core::guard::{
    self, rules::Rule, stats::ConfigStats, ComponentInput, GuardOptions, GuardReport,
};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("GUARD_TASK: {e}"))
}

/// Scan one component given as in-memory text files (`path → content`).
#[tauri::command]
pub async fn guard_scan_component(
    kind: String,
    files: HashMap<String, String>,
    origin_tool: Option<String>,
    entry: Option<String>,
    label: Option<String>,
    options: Option<GuardOptions>,
) -> Result<GuardReport, String> {
    if files.is_empty() {
        return Err("GUARD_EMPTY: no files to scan".into());
    }
    let input = ComponentInput {
        kind,
        files: files
            .into_iter()
            .map(|(k, v)| (k, v.into_bytes()))
            .collect(),
        entry,
        origin_tool,
        label,
    };
    let opts = options.unwrap_or_default();
    blocking(move || guard::scan_component(&input, &opts)).await
}

/// Scan a file or a folder on disk. A folder can hold many components.
#[tauri::command]
pub async fn guard_scan_path(
    path: String,
    kind: Option<String>,
    origin_tool: Option<String>,
    options: Option<GuardOptions>,
) -> Result<Vec<GuardReport>, String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("GUARD_NOT_FOUND: {path}"));
    }
    let opts = options.unwrap_or_default();
    blocking(move || guard::scan_path(&p, kind.as_deref(), origin_tool.as_deref(), &opts)).await
}

/// Scan many files/folders.
#[tauri::command]
pub async fn guard_scan_paths(
    paths: Vec<String>,
    origin_tool: Option<String>,
    options: Option<GuardOptions>,
) -> Result<Vec<GuardReport>, String> {
    let opts = options.unwrap_or_default();
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    blocking(move || guard::scan_paths(&paths, origin_tool.as_deref(), &opts)).await
}

/// Scan what is installed for one target tool; `paths` are the folders/files
/// the agentkit resolved for `(target_id, scope, project_dir)`.
#[tauri::command]
pub async fn guard_scan_installed(
    target_id: String,
    scope: String,
    project_dir: Option<String>,
    paths: Vec<String>,
    options: Option<GuardOptions>,
) -> Result<Vec<GuardReport>, String> {
    let opts = options.unwrap_or_default();
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    blocking(move || {
        let pd = project_dir.map(PathBuf::from);
        guard::scan_installed(&target_id, &scope, pd.as_deref(), &paths, &opts)
    })
    .await
}

/// "Config health": token weight of docs, hooks per event, MCP complexity.
/// Keys: `command`, `rule`, `agent`, `skill`, `memory`, `hook`, `setting`, `mcp`
/// (plural accepted).
#[tauri::command]
pub async fn guard_config_stats(
    paths_by_kind: BTreeMap<String, Vec<String>>,
    project_dir: Option<String>,
) -> Result<ConfigStats, String> {
    blocking(move || {
        let pd = project_dir.map(PathBuf::from);
        guard::stats::config_stats(&paths_by_kind, pd.as_deref())
    })
    .await
}

/// Every rule code with its validator, default severity and description.
#[tauri::command]
pub fn guard_rules() -> Vec<Rule> {
    guard::rules::RULES.to_vec()
}
