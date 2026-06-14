use std::sync::Arc;

use omniget_plugin_sdk::{PluginHost, PluginManifest, RegistryEntry};
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::plugin_host::PluginHostImpl;
use crate::plugin_loader::{PluginLoadError, PluginManager};

fn emit_plugins_changed(app: &tauri::AppHandle) {
    let _ = app.emit("plugins-changed", ());
}

fn build_plugin_host(app: &tauri::AppHandle, plugins_dir: std::path::PathBuf) -> Arc<dyn PluginHost> {
    Arc::new(PluginHostImpl::new(app.clone(), plugins_dir))
}

#[derive(Debug, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    pub loaded: bool,
    pub icon: Option<String>,
    pub nav: Vec<PluginNavInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_error: Option<PluginLoadError>,
}

#[derive(Debug, Serialize)]
pub struct PluginNavInfo {
    pub route: String,
    pub label: std::collections::HashMap<String, String>,
    pub icon_svg: Option<String>,
    pub group: String,
    pub order: u32,
}

#[tauri::command]
pub fn list_plugins(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
) -> Result<Vec<PluginInfo>, String> {
    let manager = state.blocking_read();
    let installed = manager.installed_plugins();

    let infos: Vec<PluginInfo> = installed
        .iter()
        .map(|entry| {
            let loaded = manager.get(&entry.id);
            match loaded {
                Some(lp) => PluginInfo {
                    id: entry.id.clone(),
                    name: lp.manifest.name.clone(),
                    version: lp.manifest.version.clone(),
                    description: lp.manifest.description.clone(),
                    author: lp.manifest.author.clone(),
                    enabled: entry.enabled,
                    loaded: true,
                    icon: lp.manifest.icon.clone(),
                    nav: lp
                        .manifest
                        .nav
                        .iter()
                        .map(|n| PluginNavInfo {
                            route: n.route.clone(),
                            label: n.label.clone(),
                            icon_svg: n.icon_svg.clone(),
                            group: match n.group {
                                omniget_plugin_sdk::NavGroup::Primary => "primary".into(),
                                omniget_plugin_sdk::NavGroup::Secondary => "secondary".into(),
                            },
                            order: n.order,
                        })
                        .collect(),
                    load_error: None,
                },
                None => {
                    let load_error = manager.load_error(&entry.id).cloned();
                    let manifest_path = manager.plugins_dir().join(&entry.id).join("plugin.json");
                    let manifest: Option<omniget_plugin_sdk::PluginManifest> =
                        std::fs::read_to_string(&manifest_path)
                            .ok()
                            .and_then(|s| serde_json::from_str(&s).ok());
                    match manifest {
                        Some(m) => PluginInfo {
                            id: entry.id.clone(),
                            name: m.name,
                            version: m.version,
                            description: m.description,
                            author: m.author,
                            enabled: entry.enabled,
                            loaded: false,
                            icon: m.icon,
                            nav: m
                                .nav
                                .iter()
                                .map(|n| PluginNavInfo {
                                    route: n.route.clone(),
                                    label: n.label.clone(),
                                    icon_svg: n.icon_svg.clone(),
                                    group: match n.group {
                                        omniget_plugin_sdk::NavGroup::Primary => "primary".into(),
                                        omniget_plugin_sdk::NavGroup::Secondary => {
                                            "secondary".into()
                                        }
                                    },
                                    order: n.order,
                                })
                                .collect(),
                            load_error,
                        },
                        None => PluginInfo {
                            id: entry.id.clone(),
                            name: entry.id.clone(),
                            version: entry.version.clone(),
                            description: String::new(),
                            author: String::new(),
                            enabled: entry.enabled,
                            loaded: false,
                            icon: None,
                            nav: Vec::new(),
                            load_error,
                        },
                    }
                }
            }
        })
        .collect();

    Ok(infos)
}

#[tauri::command]
pub fn get_plugin_frontend_path(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
) -> Result<String, String> {
    let manager = state.blocking_read();
    let frontend_dir = manager.plugins_dir().join(&plugin_id).join("frontend");
    if !frontend_dir.exists() {
        return Err(format!("Plugin '{}' has no frontend", plugin_id));
    }
    Ok(frontend_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub fn set_plugin_enabled(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
    enabled: bool,
) -> Result<(), String> {
    {
        let mut manager = state.blocking_write();
        manager
            .set_enabled(&plugin_id, enabled)
            .map_err(|e| e.to_string())?;
        if enabled && !manager.is_loaded(&plugin_id) {
            let plugins_dir = manager.plugins_dir().to_path_buf();
            let host = build_plugin_host(&app, plugins_dir);
            let _ = manager.load_one(&plugin_id, host);
        }
    }
    emit_plugins_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn uninstall_plugin(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
) -> Result<(), String> {
    {
        let mut manager = state.blocking_write();
        manager.unregister(&plugin_id).map_err(|e| e.to_string())?;
    }
    emit_plugins_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn get_loaded_plugin_manifests(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
) -> Result<Vec<PluginManifest>, String> {
    let manager = state.blocking_read();
    Ok(manager.loaded_manifests().into_iter().cloned().collect())
}

#[tauri::command]
pub fn get_plugin_i18n(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
    locale: String,
) -> Result<serde_json::Value, String> {
    let manager = state.blocking_read();
    let i18n_dir = manager.plugins_dir().join(&plugin_id).join("i18n");
    let locale_file = i18n_dir.join(format!("{}.json", locale));
    if !locale_file.exists() {
        let fallback = i18n_dir.join("en.json");
        if !fallback.exists() {
            return Ok(serde_json::json!({}));
        }
        let content = std::fs::read_to_string(&fallback).map_err(|e| e.to_string())?;
        return serde_json::from_str(&content).map_err(|e| e.to_string());
    }
    let content = std::fs::read_to_string(&locale_file).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugin_command(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
    command: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let manager = state.read().await;
    manager.handle_command(&plugin_id, &command, args).await
}

const REGISTRY_URLS: &[&str] = &[
    "https://raw.githubusercontent.com/tonhowtf/omniget-plugins/main/plugins.json",
    "https://cdn.jsdelivr.net/gh/tonhowtf/omniget-plugins@main/plugins.json",
];

#[derive(Debug, Serialize)]
pub struct MarketplaceEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub repo: String,
    pub homepage: Option<String>,
    pub tags: Vec<String>,
    pub official: bool,
    pub capabilities: Vec<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
}

#[tauri::command]
pub async fn fetch_marketplace_registry(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
) -> Result<Vec<MarketplaceEntry>, String> {
    let client = crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent("OmniGet")
            .timeout(std::time::Duration::from_secs(15)),
    )
    .build()
    .map_err(|e| e.to_string())?;

    #[derive(Deserialize)]
    struct RegistryFile {
        plugins: Vec<RegistryEntry>,
    }

    let mut registry: Option<RegistryFile> = None;
    let mut last_err = String::from("Failed to fetch registry");

    for url in REGISTRY_URLS {
        let body = match client.get(*url).send().await {
            Ok(resp) => match resp.error_for_status() {
                Ok(resp) => match resp.text().await {
                    Ok(body) => body,
                    Err(e) => {
                        last_err = format!("Failed to read registry: {}", e);
                        tracing::warn!("registry fetch via {} failed: {}", url, last_err);
                        continue;
                    }
                },
                Err(e) => {
                    last_err = format!("Registry request failed: {}", e);
                    tracing::warn!("registry fetch via {} failed: {}", url, last_err);
                    continue;
                }
            },
            Err(e) => {
                last_err = format!("Failed to fetch registry: {}", e);
                tracing::warn!("registry fetch via {} failed: {}", url, last_err);
                continue;
            }
        };

        match serde_json::from_str::<RegistryFile>(&body) {
            Ok(parsed) => {
                registry = Some(parsed);
                break;
            }
            Err(e) => {
                last_err = format!("Invalid registry: {}", e);
                tracing::warn!("registry fetch via {} failed: {}", url, last_err);
            }
        }
    }

    let registry = registry.ok_or(last_err)?;

    let installed = {
        let manager = state.read().await;
        manager.installed_plugins().to_vec()
    };

    let entries = registry
        .plugins
        .into_iter()
        .map(|entry| {
            let inst = installed.iter().find(|i| i.id == entry.id);
            MarketplaceEntry {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                author: entry.author,
                repo: entry.repo,
                homepage: entry.homepage,
                tags: entry.tags,
                official: entry.official,
                capabilities: entry.capabilities,
                installed: inst.is_some(),
                installed_version: inst.map(|i| i.version.clone()),
            }
        })
        .collect();

    Ok(entries)
}

fn platform_artifact_suffix() -> &'static str {
    if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-x86_64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "macos-x86_64"
    } else {
        "unknown"
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub async fn install_plugin_zip_from_repo(
    state: &Arc<tokio::sync::RwLock<PluginManager>>,
    plugin_id: String,
    repo: String,
) -> Result<String, String> {
    let suffix = platform_artifact_suffix();
    if suffix == "unknown" {
        return Err("Unsupported platform".to_string());
    }

    let client = crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder().user_agent("OmniGet"),
    )
    .build()
    .map_err(|e| e.to_string())?;

    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let release: GitHubRelease = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| format!("NetworkUnreachable|Failed to fetch release: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Invalid release response: {}", e))?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.contains(suffix) && a.name.ends_with(".zip"))
        .ok_or_else(|| format!("No artifact found for {}", suffix))?;

    let zip_bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("NetworkUnreachable|Failed to download: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("NetworkUnreachable|Failed to read download: {}", e))?;

    let plugin_dir = {
        let manager = state.read().await;
        manager.plugins_dir().join(&plugin_id)
    };
    std::fs::create_dir_all(&plugin_dir).map_err(|e| e.to_string())?;

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid ZIP: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = plugin_dir.join(file.mangled_name());
        if file.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if outpath.exists() {
                let old = outpath.with_extension("old");
                let _ = std::fs::remove_file(&old);
                let _ = std::fs::rename(&outpath, &old);
            }
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }

    let version = release.tag_name.trim_start_matches('v').to_string();

    let mut manager = state.write().await;
    manager
        .register_installed(omniget_plugin_sdk::InstalledPlugin {
            id: plugin_id.clone(),
            version: version.clone(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            enabled: true,
            repo: Some(repo),
            source_release: Some(release.tag_name),
        })
        .map_err(|e| e.to_string())?;

    Ok(version)
}

#[tauri::command]
pub async fn install_plugin_from_registry(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
    repo: String,
) -> Result<String, String> {
    let version =
        install_plugin_zip_from_repo(&state.inner().clone(), plugin_id.clone(), repo).await?;
    {
        let mut manager = state.write().await;
        if !manager.is_loaded(&plugin_id) {
            let plugins_dir = manager.plugins_dir().to_path_buf();
            let host = build_plugin_host(&app, plugins_dir);
            let _ = manager.load_one(&plugin_id, host);
        }
    }
    emit_plugins_changed(&app);
    Ok(version)
}

async fn fetch_registry_entries() -> Result<Vec<RegistryEntry>, String> {
    let client = crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent("OmniGet")
            .timeout(std::time::Duration::from_secs(15)),
    )
    .build()
    .map_err(|e| e.to_string())?;

    #[derive(Deserialize)]
    struct RegistryFile {
        plugins: Vec<RegistryEntry>,
    }

    let mut last_err = String::from("Failed to fetch registry");
    for url in REGISTRY_URLS {
        match client.get(*url).send().await {
            Ok(resp) => match resp.error_for_status() {
                Ok(resp) => match resp.text().await {
                    Ok(body) => match serde_json::from_str::<RegistryFile>(&body) {
                        Ok(parsed) => return Ok(parsed.plugins),
                        Err(e) => last_err = format!("Invalid registry: {}", e),
                    },
                    Err(e) => last_err = format!("Failed to read registry: {}", e),
                },
                Err(e) => last_err = format!("Registry request failed: {}", e),
            },
            Err(e) => last_err = format!("Failed to fetch registry: {}", e),
        }
    }
    Err(last_err)
}

pub async fn ensure_default_plugins(state: Arc<tokio::sync::RwLock<PluginManager>>) {
    let entries = match fetch_registry_entries().await {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("ensure_default_plugins: registry unavailable: {}", e);
            return;
        }
    };

    for entry in entries {
        let skip = {
            let manager = state.read().await;
            manager.installed_plugins().iter().any(|p| p.id == entry.id)
                || manager.is_user_removed(&entry.id)
        };
        if skip {
            continue;
        }
        tracing::info!("installing default plugin '{}' from {}", entry.id, entry.repo);
        match install_plugin_zip_from_repo(&state, entry.id.clone(), entry.repo.clone()).await {
            Ok(v) => tracing::info!("default plugin '{}' installed ({})", entry.id, v),
            Err(e) => tracing::warn!("failed to install default plugin '{}': {}", entry.id, e),
        }
    }
}

pub async fn auto_update_plugins(state: Arc<tokio::sync::RwLock<PluginManager>>) {
    let installed = {
        let manager = state.read().await;
        manager.installed_plugins().to_vec()
    };

    let client = match crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent("OmniGet")
            .timeout(std::time::Duration::from_secs(15)),
    )
    .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("auto_update_plugins: client build failed: {}", e);
            return;
        }
    };

    for plugin in installed {
        let repo = match plugin.repo {
            Some(r) => r,
            None => continue,
        };
        let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let latest = match client.get(&api_url).send().await {
            Ok(resp) => match resp.json::<GitHubRelease>().await {
                Ok(r) => r.tag_name.trim_start_matches('v').to_string(),
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        if latest.is_empty() || latest == plugin.version {
            continue;
        }
        tracing::info!(
            "auto-updating plugin '{}' {} -> {}",
            plugin.id,
            plugin.version,
            latest
        );
        match install_plugin_zip_from_repo(&state, plugin.id.clone(), repo).await {
            Ok(v) => tracing::info!("plugin '{}' updated to {}", plugin.id, v),
            Err(e) => tracing::warn!("auto-update failed for '{}': {}", plugin.id, e),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PluginUpdateInfo {
    pub id: String,
    pub installed_version: String,
    pub latest_version: String,
    pub repo: String,
    pub has_update: bool,
}

#[tauri::command]
pub async fn check_plugin_updates(
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
) -> Result<Vec<PluginUpdateInfo>, String> {
    let installed = {
        let manager = state.read().await;
        manager.installed_plugins().to_vec()
    };

    let client = crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent("OmniGet")
            .timeout(std::time::Duration::from_secs(15)),
    )
    .build()
    .map_err(|e| e.to_string())?;

    let mut updates = Vec::new();

    for plugin in &installed {
        let repo = match &plugin.repo {
            Some(r) => r.clone(),
            None => continue,
        };

        let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let release: GitHubRelease = match client.get(&api_url).send().await {
            Ok(resp) => match resp.json().await {
                Ok(r) => r,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        let latest = release.tag_name.trim_start_matches('v').to_string();
        let has_update = latest != plugin.version;

        updates.push(PluginUpdateInfo {
            id: plugin.id.clone(),
            installed_version: plugin.version.clone(),
            latest_version: latest,
            repo,
            has_update,
        });
    }

    Ok(updates)
}

#[tauri::command]
pub async fn update_plugin(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<tokio::sync::RwLock<PluginManager>>>,
    plugin_id: String,
    repo: String,
) -> Result<String, String> {
    install_plugin_from_registry(app, state, plugin_id, repo).await
}
