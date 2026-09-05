//! Sistema: ajustes do Windows, limpeza de caches, analisador de disco,
//! inicialização, desinstalador e (só Windows) debloat, registro e
//! atualizador. A lógica mora em `omniget_core::core::tools`.

use omniget_core::core::tools::{
    disk, startup, sysclean, uninstall, win_apps, win_registry, win_tweaks, win_updater,
};
use serde::Serialize;

use super::{err, progress};

#[tauri::command]
pub async fn tool_win_tweaks_status() -> win_tweaks::TweaksStatus {
    win_tweaks::status().await
}

#[tauri::command]
pub async fn tool_win_tweak_apply(id: String, enable: bool) -> Result<win_tweaks::Rule, String> {
    win_tweaks::apply(&id, enable).await.map_err(err)
}

// ── Limpar caches ──

#[tauri::command]
pub async fn tool_clean_scan(app: tauri::AppHandle) -> Result<Vec<sysclean::CleanRule>, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || sysclean::scan(&p))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn tool_clean_run(
    app: tauri::AppHandle,
    req: sysclean::CleanRequest,
) -> Result<sysclean::CleanResult, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || sysclean::clean(&req, &p))
        .await
        .map_err(err)
}

// ── Analisador de disco ──

#[tauri::command]
pub fn tool_disk_volumes() -> Vec<disk::Volume> {
    disk::volumes()
}

#[tauri::command]
pub async fn tool_disk_scan(
    app: tauri::AppHandle,
    root: String,
    depth: Option<usize>,
    children: Option<usize>,
) -> Result<disk::DiskScan, String> {
    let p = progress(&app);
    tokio::task::spawn_blocking(move || {
        disk::scan(&root, depth.unwrap_or(4), children.unwrap_or(40), &p)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[derive(Serialize)]
pub struct TrashOut {
    pub ok: Vec<String>,
    pub failed: Vec<String>,
}

#[tauri::command]
pub fn tool_disk_trash(paths: Vec<String>) -> TrashOut {
    let (ok, failed) = disk::trash_paths(&paths);
    TrashOut { ok, failed }
}

// ── Inicialização ──

#[tauri::command]
pub async fn tool_startup_list() -> Vec<startup::StartupItem> {
    startup::list().await
}

#[tauri::command]
pub async fn tool_startup_set(
    item: startup::StartupItem,
    enabled: bool,
) -> Result<Vec<startup::StartupItem>, String> {
    startup::set_enabled(&item, enabled).await.map_err(err)?;
    Ok(startup::list().await)
}

// ── Desinstalador ──

#[tauri::command]
pub async fn tool_uninstall_list(app: tauri::AppHandle) -> Vec<uninstall::App> {
    uninstall::list(progress(&app)).await
}

#[tauri::command]
pub async fn tool_uninstall_leftovers(app: uninstall::App) -> Vec<uninstall::Leftover> {
    tokio::task::spawn_blocking(move || uninstall::leftovers(&app))
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn tool_uninstall_run(
    app: uninstall::App,
    leftovers: Vec<String>,
) -> uninstall::UninstallResult {
    uninstall::uninstall(&app, &leftovers).await
}

// ── Windows: debloat, registro, atualizador ──

#[tauri::command]
pub async fn tool_debloat_list() -> Result<Vec<win_apps::AppxPackage>, String> {
    win_apps::list().await.map_err(err)
}

#[tauri::command]
pub async fn tool_debloat_remove(
    app: tauri::AppHandle,
    names: Vec<String>,
    provisioned: Option<bool>,
) -> win_apps::RemoveResult {
    win_apps::remove(&names, provisioned.unwrap_or(false), &progress(&app)).await
}

#[tauri::command]
pub async fn tool_debloat_restore(name: String) -> Result<(), String> {
    win_apps::restore(&name).await.map_err(err)
}

#[tauri::command]
pub async fn tool_registry_scan(app: tauri::AppHandle) -> Vec<win_registry::Orphan> {
    win_registry::scan(&progress(&app)).await
}

#[tauri::command]
pub async fn tool_registry_fix(items: Vec<win_registry::Orphan>) -> win_registry::FixResult {
    win_registry::fix(&items).await
}

#[tauri::command]
pub fn tool_registry_backups_dir() -> Option<String> {
    win_registry::backups_dir().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn tool_updater_status() -> win_updater::UpdaterStatus {
    win_updater::status().await
}

#[tauri::command]
pub async fn tool_updater_upgrade(
    app: tauri::AppHandle,
    items: Vec<win_updater::Outdated>,
) -> win_updater::UpgradeResult {
    win_updater::upgrade(&items, &progress(&app)).await
}
